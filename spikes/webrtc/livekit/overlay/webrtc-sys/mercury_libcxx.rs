// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2025 LiveKit, Inc.
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Adapted from the Linux function `configure_hermetic_libcxx` in the build.rs of webrtc-sys 0.3.54
// (LiveKit rust-sdks, https://github.com/livekit/rust-sdks, Apache-2.0): the flags and the second
// copy of cxx.cc are theirs, the macOS use, the header source and the checks are ours. Kept in its
// own file, under its upstream licence, as AGENTS.md asks (entry for THIRD-PARTY-NOTICES.md: see
// the sp-rtc report).
//
// Mercury's addition to webrtc-sys 0.3.54's build script (copied next to build.rs by
// spikes/webrtc/livekit/prepare.sh; build.rs gets a `mod mercury_libcxx;` line and one call).
//
// Shiguredo's libwebrtc.a holds Chromium's libc++ in the inline namespace std::__Cr, so every
// std type in libwebrtc's headers (std::string, std::unique_ptr, std::vector) has a different
// mangled name and layout from Apple's std::__1. Upstream handles this for Linux only
// (configure_hermetic_libcxx); on macOS it compiles against the Xcode libc++ because LiveKit's own
// macOS build is made that way. Against this archive that would leave unresolved or silently wrong
// std::__1 symbols. So the glue is compiled against the same libc++ headers the archive was built
// with, fetched by fetch-libcxx.sh at the revision named in the archive's VERSIONS file.

use std::env;
use std::path::{Path, PathBuf};

/// The directory fetch-libcxx.sh verified: `MERCURY_LIBCXX_DIR`, or else the path prepare.sh wrote to
/// `mercury-libcxx-dir` next to build.rs. Nothing is downloaded here, and a missing or unverified
/// directory stops the build.
fn libcxx_dir() -> PathBuf {
    println!("cargo:rerun-if-env-changed=MERCURY_LIBCXX_DIR");
    println!("cargo:rerun-if-changed=mercury-libcxx-dir");
    if let Some(dir) = env::var_os("MERCURY_LIBCXX_DIR") {
        return PathBuf::from(dir);
    }
    match std::fs::read_to_string("mercury-libcxx-dir") {
        Ok(text) => PathBuf::from(text.trim()),
        Err(_) => panic!(
            "MERCURY_LIBCXX_DIR is not set and prepare.sh has not run; run ./prepare.sh or eval \"$(./fetch-libcxx.sh env)\""
        ),
    }
}

pub fn configure(builder: &mut cc::Build, webrtc_include: &Path) {
    let dir = libcxx_dir();
    let include = dir.join("include");
    let config = dir.join("config");
    assert!(
        include.join("vector").is_file() && config.join("__config_site").is_file(),
        "{} is not a fetch-libcxx.sh directory (no include/vector or config/__config_site)",
        dir.display()
    );

    builder
        .flag("-nostdinc++")
        .flag(format!("-isystem{}", include.display()))
        .flag(format!(
            "-isystem{}",
            webrtc_include
                .join("third_party/libc++abi/src/include")
                .display()
        ))
        // Holds __config_site (_LIBCPP_ABI_NAMESPACE=__Cr, _LIBCPP_ABI_VERSION=2) and __assertion_handler.
        .include(&config);

    // libc++ and libc++abi are inside libwebrtc.a; do not add Apple's libc++ for this code.
    builder.cpp_link_stdlib(None);

    // The cxx crate builds its runtime (cxx.cc) with the default libc++, so the String <-> std::string
    // conversions it exports are mangled for std::__1 and cannot satisfy the std::__Cr call sites
    // of the generated bridges. Compile a second copy with the flags above.
    let cxx_h = env::var("DEP_CXXBRIDGE1_HEADER").expect("cxx did not export HEADER");
    let cxx_root = Path::new(&cxx_h)
        .parent()
        .and_then(Path::parent)
        .expect("unexpected DEP_CXXBRIDGE1_HEADER layout");
    builder.file(cxx_root.join("src/cxx.cc"));
}
