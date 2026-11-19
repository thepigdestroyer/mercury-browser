// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Lets this crate's own tests and examples find the GStreamer libraries without
//! `DYLD_FALLBACK_LIBRARY_PATH`: when `scripts/macos/gstreamer.sh env` has been evaluated
//! (`MERCURY_GSTREAMER_PREFIX` is set), the library directory becomes an rpath of the test and
//! example binaries. Crates that depend on this one need the same for their own binaries.

fn main() {
    println!("cargo:rerun-if-env-changed=MERCURY_GSTREAMER_PREFIX");
    if let Ok(prefix) = std::env::var("MERCURY_GSTREAMER_PREFIX") {
        for kind in ["tests", "examples", "bins"] {
            println!("cargo:rustc-link-arg-{kind}=-Wl,-rpath,{prefix}/lib");
        }
        // The library's own unit tests are linked as a test target of this package too.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{prefix}/lib");
    }
}
