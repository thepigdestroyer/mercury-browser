// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The Seatbelt profile of the content process and the parameters that go with it.
//!
//! The profile text is `profiles/content.sb` (BSD-3-Clause, translated from Chromium). It reads
//! every path and switch through `(param "NAME")`, so no path is ever pasted into the text:
//! a directory called `a")(allow default)(` is a value, not a rule. This module checks the
//! values (absolute, resolved by the caller, no `..`, no NUL, never `/` as a subpath root) and
//! produces the parameter list for `sandbox_init_with_parameters`.

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// The profile text of the content process (`profiles/content.sb`).
pub const CONTENT_PROFILE: &str = include_str!("../profiles/content.sb");

/// The bundle identifier of Mercury Browser (`Info.plist`), which names its preference domain.
pub const BUNDLE_IDENTIFIER: &str = "io.github.thepigdestroyer.mercury-browser";

/// What the profile needs to know about this run. Every path must already be resolved
/// (`std::fs::canonicalize`): Seatbelt matches the resolved path, so `/var/...` and
/// `/private/var/...` are different things to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileInputs {
    /// The `.app` bundle, or the directory of a development executable; read-only.
    pub bundle_path: PathBuf,
    /// The running executable.
    pub executable: PathBuf,
    /// The user's home directory (fonts and preferences are read from it).
    pub home: PathBuf,
    /// The private per-run directory GLib and GStreamer write to, if there is one.
    pub run_root: Option<PathBuf>,
    /// Rule group AUDIO_OUT: Core Audio services.
    pub audio_output: bool,
    /// Rule group MEDIA_DECODE: VideoToolbox and AudioToolbox.
    pub media_decode: bool,
    /// Rule group CAPTURE: camera and microphone.
    pub capture: bool,
    /// `(debug deny)`: denials are logged by the kernel in more detail.
    pub logging: bool,
}

impl ProfileInputs {
    /// Inputs with every rule group on (until the evidence of a real-Mac run says otherwise)
    /// and no run root.
    pub fn new(bundle_path: PathBuf, executable: PathBuf, home: PathBuf) -> Self {
        Self {
            bundle_path,
            executable,
            home,
            run_root: None,
            audio_output: true,
            media_decode: true,
            capture: true,
            logging: false,
        }
    }
}

/// A profile and its parameters, ready for `sandbox_init_with_parameters`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub text: String,
    /// Name and value, in the order they are handed to the kernel.
    pub parameters: Vec<(String, String)>,
}

impl Profile {
    /// The value of a parameter, if it is set.
    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Why a path was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileError {
    NotAbsolute(&'static str),
    ParentComponent(&'static str),
    Empty(&'static str),
    Nul(&'static str),
    Root(&'static str),
    NotUtf8(&'static str),
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAbsolute(name) => write!(formatter, "{name} is not an absolute path"),
            Self::ParentComponent(name) => write!(formatter, "{name} contains \"..\""),
            Self::Empty(name) => write!(formatter, "{name} is empty"),
            Self::Nul(name) => write!(formatter, "{name} contains a NUL byte"),
            Self::Root(name) => write!(formatter, "{name} is the root directory"),
            Self::NotUtf8(name) => write!(formatter, "{name} is not valid UTF-8"),
        }
    }
}

impl std::error::Error for ProfileError {}

/// The content-process profile with the parameters for `inputs`.
pub fn content_profile(inputs: &ProfileInputs) -> Result<Profile, ProfileError> {
    let mut parameters = vec![
        param_path("BUNDLE_PATH", &inputs.bundle_path, true)?,
        param_path("EXECUTABLE_PATH", &inputs.executable, false)?,
        ("BUNDLE_ID".to_owned(), BUNDLE_IDENTIFIER.to_owned()),
        param_path("HOMEDIR", &inputs.home, true)?,
    ];
    if let Some(run_root) = &inputs.run_root {
        parameters.push(param_path("RUN_ROOT", run_root, true)?);
    }
    parameters.push(param_flag("ALLOW_AUDIO_OUT", inputs.audio_output));
    parameters.push(param_flag("ALLOW_MEDIA_DECODE", inputs.media_decode));
    parameters.push(param_flag("ALLOW_CAPTURE", inputs.capture));
    parameters.push(param_flag("ENABLE_LOGGING", inputs.logging));
    Ok(Profile {
        text: CONTENT_PROFILE.to_owned(),
        parameters,
    })
}

/// Report mode: nothing is confined and the kernel logs every operation the process performs
/// (`log show --predicate 'sender == "Sandbox"'`). A development tool for finishing the profile.
pub fn report_profile() -> Profile {
    Profile {
        text: "(version 1)\n(allow default (with report))\n".to_owned(),
        parameters: Vec::new(),
    }
}

fn param_flag(name: &str, on: bool) -> (String, String) {
    (
        name.to_owned(),
        if on { "TRUE" } else { "FALSE" }.to_owned(),
    )
}

/// A path parameter. `subpath_root` says the profile allows everything below it, so `/` is
/// refused.
fn param_path(
    name: &'static str,
    path: &Path,
    subpath_root: bool,
) -> Result<(String, String), ProfileError> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.is_empty() {
        return Err(ProfileError::Empty(name));
    }
    if bytes.contains(&0) {
        return Err(ProfileError::Nul(name));
    }
    if !path.is_absolute() {
        return Err(ProfileError::NotAbsolute(name));
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(ProfileError::ParentComponent(name));
    }
    if subpath_root && path.components().count() == 1 {
        return Err(ProfileError::Root(name));
    }
    // The kernel takes bytes, but a lossy conversion would name another path: refuse instead.
    let Some(value) = path.to_str() else {
        return Err(ProfileError::NotUtf8(name));
    };
    Ok((name.to_owned(), value.to_owned()))
}
