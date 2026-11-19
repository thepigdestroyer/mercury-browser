// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Lets the test binaries find the GStreamer libraries (t05, t07 and t10 use the bridge crate)
//! without `DYLD_FALLBACK_LIBRARY_PATH`: when `scripts/macos/gstreamer.sh env` has been evaluated
//! (`MERCURY_GSTREAMER_PREFIX` is set), its library directory becomes an rpath of the test binaries.

fn main() {
    // libwebrtc.a is a static library whose Objective-C code includes categories
    // (RTCVideoCodecInfo+Private and others). The linker drops an object file that holds only a
    // category unless it is told to keep categories: the first CI run aborted in the codec factory
    // with "-[RTCVideoCodecInfo initWithNativeSdpVideoFormat:]: unrecognized selector". The final
    // Mercury binary needs the same flag. (webrtc-sys passes it to its own targets only.)
    println!("cargo:rustc-link-arg-tests=-ObjC");
    println!("cargo:rustc-link-arg-examples=-ObjC");
    println!("cargo:rerun-if-env-changed=MERCURY_GSTREAMER_PREFIX");
    if let Ok(prefix) = std::env::var("MERCURY_GSTREAMER_PREFIX") {
        println!("cargo:rustc-link-arg-tests=-Wl,-rpath,{prefix}/lib");
    }
}
