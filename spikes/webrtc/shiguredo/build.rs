// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Two small jobs for this crate's own test binaries.
//!
//! * When `scripts/macos/gstreamer.sh env` has been evaluated (`MERCURY_GSTREAMER_PREFIX` is set),
//!   its library directory becomes an rpath of the test binaries, because the bridge crate links
//!   GStreamer dynamically (the same job the bridge's own build script does for its tests).
//! * It prints the deployment target the build uses, so that a log shows 14.0 (this crate's
//!   `.cargo/config.toml`) or whatever an exported variable overrides it with.

fn main() {
    println!("cargo:rerun-if-env-changed=MERCURY_GSTREAMER_PREFIX");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    if let Ok(prefix) = std::env::var("MERCURY_GSTREAMER_PREFIX") {
        println!("cargo:rustc-link-arg-tests=-Wl,-rpath,{prefix}/lib");
    }
    if let Ok(target) = std::env::var("MACOSX_DEPLOYMENT_TARGET") {
        println!("cargo:warning=deployment target {target}");
    }
}
