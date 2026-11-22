// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Whether the screen is locked. A window run needs an unlocked screen: on a locked one the
//! browser's GL context can hang, and two central runs were lost to that. `ioreg -n Root -d1`
//! lists the console session with `"CGSSessionScreenIsLocked"=Yes` while the screen is locked.

use std::ffi::OsStr;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockState {
    /// `CGSSessionScreenIsLocked` is `Yes`.
    Locked,
    /// A console session exists and is not locked.
    Unlocked,
    /// No answer: `ioreg` failed, or lists no console session (nobody is logged in at the
    /// screen). A window run refuses to start.
    Unknown,
}

/// Reads the output of `ioreg -n Root -d1`.
pub fn parse_ioreg(output: &str) -> LockState {
    if let Some(at) = output.find("\"CGSSessionScreenIsLocked\"") {
        let after = output[at + "\"CGSSessionScreenIsLocked\"".len()..].trim_start();
        let after = after.strip_prefix('=').unwrap_or(after).trim_start();
        if after.starts_with("Yes") || after.starts_with("true") || after.starts_with("1") {
            return LockState::Locked;
        }
        return LockState::Unlocked;
    }
    // The key is absent while the screen has never been locked in this session.
    if output.contains("\"IOConsoleUsers\"") || output.contains("kCGSSessionOnConsoleKey") {
        LockState::Unlocked
    } else {
        LockState::Unknown
    }
}

/// Runs `ioreg -n Root -d1` (the program may be replaced: a fake in the tests, `--ioreg` on the
/// command line) and reads the answer. Gives up after ten seconds.
pub fn check(ioreg: &OsStr) -> LockState {
    let Ok(mut child) = Command::new(ioreg)
        .args(["-n", "Root", "-d1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return LockState::Unknown;
    };
    let Some(mut stdout) = child.stdout.take() else {
        return LockState::Unknown;
    };
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        text.push_str(&String::from_utf8_lossy(&bytes));
        text
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                return reader
                    .join()
                    .map_or(LockState::Unknown, |text| parse_ioreg(&text));
            }
            Ok(Some(_)) | Err(_) => return LockState::Unknown,
            Ok(None) if started.elapsed() > Duration::from_secs(10) => {
                let _ = child.kill();
                let _ = child.wait();
                return LockState::Unknown;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    const LOCKED: &str = r#"+-o Root  <class IORegistryEntry, id 0x100000100, retain 31>
  {
    "IOConsoleUsers" = ({"kCGSSessionOnConsoleKey"=Yes,"CGSSessionScreenLockedTime"=1795345691,"CGSSessionScreenIsLocked"=Yes,"kCGSSessionUserIDKey"=501})
  }
"#;

    const UNLOCKED_FLAG: &str = r#"    "IOConsoleUsers" = ({"kCGSSessionOnConsoleKey"=Yes,"CGSSessionScreenIsLocked"=No,"kCGSSessionUserIDKey"=501})"#;

    const NEVER_LOCKED: &str =
        r#"    "IOConsoleUsers" = ({"kCGSSessionOnConsoleKey"=Yes,"kCGSSessionUserIDKey"=501})"#;

    #[test]
    fn the_output_of_ioreg_is_read() {
        assert_eq!(parse_ioreg(LOCKED), LockState::Locked);
        assert_eq!(parse_ioreg(UNLOCKED_FLAG), LockState::Unlocked);
        assert_eq!(parse_ioreg(NEVER_LOCKED), LockState::Unlocked);
        assert_eq!(parse_ioreg(""), LockState::Unknown);
        assert_eq!(
            parse_ioreg("+-o Root <class IORegistryEntry>"),
            LockState::Unknown
        );
    }

    fn fake_ioreg(dir: &Path, body: &str, exit: i32) -> std::path::PathBuf {
        let path = dir.join("ioreg");
        fs::write(
            &path,
            format!("#!/bin/sh\n# arguments: $*\ncat <<'EOF'\n{body}\nEOF\nexit {exit}\n"),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn a_fake_ioreg_answers_locked_unlocked_and_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let locked = fake_ioreg(dir.path(), LOCKED, 0);
        assert_eq!(check(locked.as_os_str()), LockState::Locked);
        let unlocked = fake_ioreg(dir.path(), UNLOCKED_FLAG, 0);
        assert_eq!(check(unlocked.as_os_str()), LockState::Unlocked);
        let failing = fake_ioreg(dir.path(), LOCKED, 1);
        assert_eq!(
            check(failing.as_os_str()),
            LockState::Unknown,
            "a failed ioreg says nothing"
        );
        assert_eq!(
            check(dir.path().join("missing").as_os_str()),
            LockState::Unknown
        );
    }

    #[test]
    fn ioreg_is_asked_for_the_root_node() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ioreg");
        fs::write(
            &path,
            "#!/bin/sh\n[ \"$*\" = \"-n Root -d1\" ] || exit 2\necho '\"IOConsoleUsers\" = ({\"CGSSessionScreenIsLocked\"=Yes})'\n",
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(check(path.as_os_str()), LockState::Locked);
    }
}
