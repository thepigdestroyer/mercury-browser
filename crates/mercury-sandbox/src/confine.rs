// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Confining this process, and checking that it worked.
//!
//! Fail closed: any problem is an [`EnterError`], never an unconfined continue. The caller ends
//! the process with [`exit_unconfined`]. After the profile is applied a canary tries two things a
//! confined content process must not be able to do (read `/private/etc/hosts`, connect to
//! `127.0.0.1:9`); a profile that applied but confined nothing ends the process too.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::profile::{Profile, report_profile};

/// The exit code of a process that could not be confined.
pub const EXIT_CODE_UNCONFINED: i32 = 70;

/// Why a process could not be confined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnterError {
    /// This platform has no content sandbox.
    Unsupported,
    /// A path or switch was missing, relative or malformed.
    InvalidParameter(String),
    /// The system refused the profile (a syntax error, or the process is already confined).
    Refused(String),
    /// [`confine`] was already called in this process.
    AlreadyConfined,
    /// The profile applied, but the canary shows the process is not confined.
    CanaryFailed(&'static str),
}

impl fmt::Display for EnterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(formatter, "no content sandbox on this platform"),
            Self::InvalidParameter(detail) => write!(formatter, "invalid parameter: {detail}"),
            Self::Refused(detail) => write!(formatter, "the system refused the profile: {detail}"),
            Self::AlreadyConfined => write!(formatter, "this process is already confined"),
            Self::CanaryFailed(what) => write!(formatter, "not confined after entering: {what}"),
        }
    }
}

impl std::error::Error for EnterError {}

/// What happened when the process was confined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Confinement {
    /// Report mode: nothing is confined; every operation is logged by the kernel.
    pub report_mode: bool,
}

static CONFINED: AtomicBool = AtomicBool::new(false);

/// Applies `profile` to this process and checks that it took effect. Once per process.
pub fn confine(profile: &Profile) -> Result<Confinement, EnterError> {
    if CONFINED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(EnterError::AlreadyConfined);
    }
    if let Err(error) = apply(profile) {
        // Nothing was applied, so a later attempt may use another profile.
        CONFINED.store(false, Ordering::SeqCst);
        return Err(error);
    }
    let report_mode = *profile == report_profile();
    if !report_mode {
        canary()?;
    }
    Ok(Confinement { report_mode })
}

#[cfg(target_os = "macos")]
fn apply(profile: &Profile) -> Result<(), EnterError> {
    crate::seatbelt::apply(profile)
}

#[cfg(not(target_os = "macos"))]
fn apply(_profile: &Profile) -> Result<(), EnterError> {
    Err(EnterError::Unsupported)
}

#[cfg(not(target_os = "macos"))]
fn canary() -> Result<(), EnterError> {
    Ok(())
}

/// The file the canary tries to read: not in any allow rule of the profile.
#[cfg(target_os = "macos")]
const CANARY_FILE: &str = "/private/etc/hosts";

/// Two things a confined content process cannot do. `EPERM` is the sandbox's answer; any other
/// outcome (success, "connection refused") means the sandbox is not there.
#[cfg(target_os = "macos")]
fn canary() -> Result<(), EnterError> {
    match std::fs::File::open(CANARY_FILE) {
        Ok(_) => return Err(EnterError::CanaryFailed("read a file outside the profile")),
        Err(error) if error.raw_os_error() == Some(libc::EPERM) => {}
        Err(_) => {
            return Err(EnterError::CanaryFailed(
                "file open failed, but not by the sandbox",
            ));
        }
    }
    match connect_loopback_discard() {
        Some(libc::EPERM) => Ok(()),
        _ => Err(EnterError::CanaryFailed(
            "opened or connected a network socket",
        )),
    }
}

/// The `errno` of trying to connect a TCP socket to `127.0.0.1:9`, or `None` if it connected.
#[cfg(target_os = "macos")]
fn connect_loopback_discard() -> Option<i32> {
    // SAFETY: plain system calls on a socket this function creates and closes; `address` is a
    // fully initialised `sockaddr_in` and its length is passed.
    unsafe {
        let socket = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if socket < 0 {
            return Some(*libc::__error());
        }
        let mut address: libc::sockaddr_in = std::mem::zeroed();
        address.sin_family = libc::AF_INET as libc::sa_family_t;
        address.sin_port = 9_u16.to_be();
        address.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
        address.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
        let status = libc::connect(
            socket,
            std::ptr::addr_of!(address).cast::<libc::sockaddr>(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        );
        let code = if status == 0 {
            None
        } else {
            Some(*libc::__error())
        };
        libc::close(socket);
        code
    }
}

/// Confines this process as a content process: the profile of `profiles/content.sb` with the
/// paths of this run, or, when `MERCURY_SANDBOX_REPORT=1`, report mode (a development switch:
/// nothing is confined, which is said loudly).
#[cfg(target_os = "macos")]
pub fn confine_this_process() -> Result<Confinement, EnterError> {
    use crate::layout::{bundle_root, run_root_from_env};
    use crate::profile::{ProfileInputs, content_profile};

    if std::env::var_os("MERCURY_SANDBOX_REPORT").is_some_and(|value| value == "1") {
        log::error!("content sandbox in REPORT mode: this process is NOT confined");
        return confine(&report_profile());
    }
    let resolve = |what: &str, path: std::io::Result<std::path::PathBuf>| {
        path.and_then(std::fs::canonicalize)
            .map_err(|error| EnterError::InvalidParameter(format!("{what}: {error}")))
    };
    let executable = resolve("the executable", std::env::current_exe())?;
    let bundle = resolve("the bundle", Ok(bundle_root(&executable)))?;
    let home = resolve("the home directory", home_directory())?;
    let run_root = run_root_from_env(&|name| std::env::var_os(name))
        .and_then(|root| std::fs::canonicalize(root).ok());
    let mut inputs = ProfileInputs::new(bundle, executable, home);
    inputs.run_root = run_root;
    let profile = content_profile(&inputs)
        .map_err(|error| EnterError::InvalidParameter(error.to_string()))?;
    confine(&profile)
}

/// Confines this process as a content process (macOS only).
#[cfg(not(target_os = "macos"))]
pub fn confine_this_process() -> Result<Confinement, EnterError> {
    Err(EnterError::Unsupported)
}

/// The user's home directory from the password database, not from `$HOME`.
#[cfg(target_os = "macos")]
fn home_directory() -> std::io::Result<std::path::PathBuf> {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;

    // SAFETY: `getpwuid_r` writes into `passwd` and `buffer`, which live until the end of the
    // function; the result's `pw_dir` points into `buffer`.
    unsafe {
        let mut passwd: libc::passwd = std::mem::zeroed();
        let mut buffer = vec![0_u8; 16 * 1024];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let status = libc::getpwuid_r(
            libc::getuid(),
            &mut passwd,
            buffer.as_mut_ptr().cast::<libc::c_char>(),
            buffer.len(),
            &mut result,
        );
        if status != 0 || result.is_null() || passwd.pw_dir.is_null() {
            return Err(std::io::Error::other(
                "no home directory in the user database",
            ));
        }
        let directory = CStr::from_ptr(passwd.pw_dir);
        Ok(std::path::PathBuf::from(OsStr::from_bytes(
            directory.to_bytes(),
        )))
    }
}

/// Ends the process at once with [`EXIT_CODE_UNCONFINED`]: for a content process that could
/// not be confined. No destructors run and nothing else is read or written.
pub fn exit_unconfined() -> ! {
    // SAFETY: `_exit` ends the process immediately and touches no Rust state.
    unsafe { libc::_exit(EXIT_CODE_UNCONFINED) }
}
