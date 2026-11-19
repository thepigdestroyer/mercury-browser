// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The part of LiveKit's `webrtc-sys-build` 0.3.21 that `webrtc-sys` 0.3.54's `build.rs` needs, and
//! nothing else: where the pinned libwebrtc is (`LK_CUSTOM_WEBRTC`, exported by
//! `scripts/macos/webrtc.sh env`) and the preprocessor defines to compile against it.
//!
//! What is gone: the downloader (`reqwest`, `zip`, `scratch`, `fs2`), the version checks, the
//! Android helpers. The functions `build.rs` still names for other targets exist as stubs that
//! stop with a message, so the file compiles unchanged. The crate refuses every target except
//! `aarch64-apple-darwin`, because the pinned archive is arm64 macOS only.

use std::env;
use std::path::PathBuf;

const DEFINES: &str = include_str!("../macos-arm64.defines");

const ONLY_TARGET: &str = "this build of webrtc-sys-build exists for the Mercury WebRTC spike and \
supports only aarch64-apple-darwin (the pinned libwebrtc is arm64 macOS); \
scripts/macos/webrtc.sh explains why there is no other archive";

fn require_target() {
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    assert!(
        os == "macos" && arch == "aarch64",
        "{ONLY_TARGET} (target was {arch}-{os})"
    );
    // The archive is built for macOS 14 (its DEPS file). Linking it with a lower minimum would let
    // the app start on a macOS that lacks APIs the archive uses. The value in force is checked here
    // because the repository root's .cargo/config.toml says 13.0 and this crate's cannot force 14.
    let minos = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_default();
    let major: u32 = minos
        .split('.')
        .next()
        .and_then(|m| m.parse().ok())
        .unwrap_or(0);
    assert!(
        major >= 14,
        "MACOSX_DEPLOYMENT_TARGET is '{minos}'; the pinned libwebrtc needs 14.0 or later"
    );
}

/// `"mac"` for the only supported target (the names are the ones upstream uses).
pub fn target_os() -> String {
    require_target();
    "mac".to_owned()
}

/// `"arm64"` for the only supported target.
pub fn target_arch() -> String {
    require_target();
    "arm64".to_owned()
}

/// The unpacked archive: `LK_CUSTOM_WEBRTC`. The spike's `.cargo/config.toml` defaults it to an
/// empty directory, so a missing export stops in [`webrtc_defines`] (no `lib/libwebrtc.a`).
pub fn webrtc_dir() -> PathBuf {
    match env::var_os("LK_CUSTOM_WEBRTC") {
        Some(dir) => PathBuf::from(dir),
        None => panic!(
            "LK_CUSTOM_WEBRTC is not set; run: eval \"$(scripts/macos/webrtc.sh env libwebrtc)\""
        ),
    }
}

/// The defines from `macos-arm64.defines`, as `(name, value)` pairs.
pub fn webrtc_defines() -> Vec<(String, Option<String>)> {
    require_target();
    let dir = webrtc_dir();
    assert!(
        dir.join("lib/libwebrtc.a").is_file() && dir.join(".mercury-webrtc-verified").is_file(),
        "{} is not a verified libwebrtc directory (no lib/libwebrtc.a or no .mercury-webrtc-verified \
         marker); run: eval \"$(scripts/macos/webrtc.sh env libwebrtc)\"",
        dir.display()
    );
    DEFINES
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| match line.split_once('=') {
            Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
            None => (line.to_owned(), None),
        })
        .collect()
}

/// Upstream downloads LiveKit's prebuilt here. This build never does (decision D3 and D8).
pub fn download_webrtc() -> Result<(), String> {
    Err(
        "downloading a libwebrtc is not supported: obtain the pinned archive with \
         scripts/macos/webrtc.sh fetch unpack and export it with webrtc.sh env"
            .to_owned(),
    )
}

/// Android only upstream; never reached on the supported target.
pub fn configure_jni_symbols() -> Result<(), String> {
    Err(ONLY_TARGET.to_owned())
}

/// Android only upstream; never reached on the supported target.
pub fn android_ndk_toolchain() -> Result<PathBuf, String> {
    Err(ONLY_TARGET.to_owned())
}
