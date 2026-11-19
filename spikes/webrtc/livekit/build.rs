// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Lets the test binaries find the GStreamer libraries (t05, t07 and t10 use the bridge crate)
//! without `DYLD_FALLBACK_LIBRARY_PATH`: when `scripts/macos/gstreamer.sh env` has been evaluated
//! (`MERCURY_GSTREAMER_PREFIX` is set), its library directory becomes an rpath of the test binaries.

fn main() {
    println!("cargo:rerun-if-env-changed=MERCURY_GSTREAMER_PREFIX");
    if let Ok(prefix) = std::env::var("MERCURY_GSTREAMER_PREFIX") {
        println!("cargo:rustc-link-arg-tests=-Wl,-rpath,{prefix}/lib");
    }
}
