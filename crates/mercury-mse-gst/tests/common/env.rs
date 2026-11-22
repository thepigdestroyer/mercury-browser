// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A private GStreamer for the tests (decision D12): nothing is scanned, so no plugin that would touch the
//! GPU or a window is ever loaded (the framework's directory holds `libgstvulkan`, whose scan hung on
//! this Mac); the plugins are loaded by file name the way Servo's `init_with_plugins` does, from the list
//! Mercury ships, plus the few test-only ones (`videotestsrc` and the encoders for T05, `dav1d`). The
//! software decoders are pinned by rank. `libgstapplemedia` (VideoToolbox) is not in the list.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gstreamer as gst;
use gstreamer::prelude::*;

/// The plugin files a macOS build of Mercury loads (Servo's lists minus `ogg`, minus `applemedia`, whose
/// VideoToolbox decoders the tests must not use), then the test-only ones.
const PLUGIN_FILES: &[&str] = &[
    "libgstcoreelements.dylib",
    "libgstapp.dylib",
    "libgstaudioconvert.dylib",
    "libgstaudioresample.dylib",
    "libgstopus.dylib",
    "libgstplayback.dylib",
    "libgsttypefindfunctions.dylib",
    "libgstvideoconvertscale.dylib",
    "libgstaudioparsers.dylib",
    "libgstisomp4.dylib",
    "libgstmatroska.dylib",
    "libgstvpx.dylib",
    "libgstvideoparsersbad.dylib",
    "libgstlibav.dylib",
    // Not in Mercury's list yet (EA0 adds dav1d); the AV1 tests need it.
    "libgstdav1d.dylib",
    // Test only.
    "libgstvideotestsrc.dylib",
    "libgstaudiotestsrc.dylib",
    "libgstopenh264.dylib",
];

/// Which decoders the tests use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The software decoders, pinned by rank (decision D12). The default.
    Software,
    /// Mercury's plugin list as shipped, VideoToolbox included, ranks untouched (the `#[ignore]`d
    /// default-route tests of `default_route.rs`, run by the owner).
    DefaultRoute,
}

static MODE: OnceLock<Mode> = OnceLock::new();

/// Called first thing in a test binary that wants the default decoder route.
pub fn use_default_route() {
    let _ = MODE.set(Mode::DefaultRoute);
}

/// What `init` found.
pub struct Env {
    /// Plugin files that did not load, with the reason (a missing one only matters to the tests that
    /// need an element of it, which then skip).
    pub failed: Vec<(String, String)>,
}

static ENV: OnceLock<Env> = OnceLock::new();

/// Sets up GStreamer once per test binary: environment, `gst::init`, plugins, `register()`, ranks.
pub fn init() -> &'static Env {
    ENV.get_or_init(|| {
        let mode = MODE.get().copied().unwrap_or(Mode::Software);
        let Some(prefix) = std::env::var_os("MERCURY_GSTREAMER_PREFIX").map(PathBuf::from) else {
            // No bundled GStreamer (Linux CI, or a run outside test.sh): the system one as it is. Tests
            // whose elements are missing print `SKIP: <element> missing` and return.
            eprintln!("mercury-mse-gst tests: MERCURY_GSTREAMER_PREFIX is not set, using the system GStreamer");
            gst::init().expect("gstreamer::init");
            mercury_mse_gst::register().expect("register MseSrc");
            if mode == Mode::Software {
                pin_software_decoders();
            }
            return Env { failed: Vec::new() };
        };
        let scratch = scratch_dir();
        for sub in ["", "gio-modules", "xdg/cache", "xdg/data", "xdg/config", "xdg/runtime"] {
            std::fs::create_dir_all(scratch.join(sub)).expect("scratch directory");
        }
        // SAFETY: this runs inside a `OnceLock` before anything of ours uses GStreamer or reads the
        // environment, and every test passes through it first.
        unsafe {
            for name in [
                "GST_REGISTRY",
                "GST_PLUGIN_PATH",
                "GST_PLUGIN_SYSTEM_PATH",
                "GST_PLUGIN_SCANNER",
                "GST_PLUGIN_SCANNER_1_0",
                "GIO_EXTRA_MODULES",
            ] {
                std::env::remove_var(name);
            }
            std::env::set_var(
                "GST_REGISTRY_1_0",
                scratch.join(format!("registry-{}.bin", std::env::consts::ARCH)),
            );
            // Empty, not unset: unset means the user's plugins and the default directory.
            std::env::set_var("GST_PLUGIN_PATH_1_0", "");
            std::env::set_var("GST_PLUGIN_SYSTEM_PATH_1_0", "");
            std::env::set_var("GST_REGISTRY_FORK", "no");
            std::env::set_var("GIO_MODULE_DIR", scratch.join("gio-modules"));
            std::env::set_var("XDG_CACHE_HOME", scratch.join("xdg/cache"));
            std::env::set_var("XDG_DATA_HOME", scratch.join("xdg/data"));
            std::env::set_var("XDG_CONFIG_HOME", scratch.join("xdg/config"));
            std::env::set_var("XDG_RUNTIME_DIR", scratch.join("xdg/runtime"));
        }
        gst::init().expect("gstreamer::init");

        let dir = prefix.join("lib").join("gstreamer-1.0");
        let registry = gst::Registry::get();
        let mut failed = Vec::new();
        let extra: &[&str] = if mode == Mode::DefaultRoute {
            &["libgstapplemedia.dylib"]
        } else {
            &[]
        };
        for name in PLUGIN_FILES.iter().chain(extra) {
            match gst::Plugin::load_file(dir.join(name)) {
                Ok(plugin) => {
                    if registry.add_plugin(&plugin).is_err() {
                        failed.push(((*name).to_owned(), "Registry::add_plugin failed".to_owned()));
                    }
                }
                Err(error) => failed.push(((*name).to_owned(), error.to_string())),
            }
        }
        mercury_mse_gst::register().expect("register MseSrc");
        if mode == Mode::Software {
            pin_software_decoders();
        }
        Env { failed }
    })
}

/// One scratch directory per test binary, under the target directory.
fn scratch_dir() -> PathBuf {
    let exe = std::env::current_exe().ok();
    let stem = exe
        .as_deref()
        .and_then(Path::file_stem)
        .and_then(|stem| stem.to_str())
        .unwrap_or("tests")
        .to_owned();
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("mse-gst-{stem}"))
}

/// D12: the software decoders win by rank, so that no hardware session is ever created.
fn pin_software_decoders() {
    for name in [
        "avdec_h264",
        "avdec_aac",
        "vp9dec",
        "vp8dec",
        "opusdec",
        "dav1ddec",
    ] {
        if let Some(factory) = gst::ElementFactory::find(name) {
            factory.set_rank(gst::Rank::PRIMARY + 100);
        }
    }
}

/// `true` when every named element is in the registry; prints `SKIP: <element> missing` for the first
/// that is not.
pub fn have(elements: &[&str]) -> bool {
    init();
    for name in elements {
        if gst::ElementFactory::find(name).is_none() {
            skip_or_fail(name);
            return false;
        }
    }
    true
}

/// Reports a missing element. Without `MERCURY_GSTREAMER_PREFIX` (Linux CI, a system GStreamer) the test
/// skips: `SKIP: <element> missing`. With it set the bundled GStreamer is meant to have every element
/// the tests use, so a missing one is a failure: a playback suite that skips everything must not pass.
pub fn skip_or_fail(element: &str) {
    assert!(
        std::env::var_os("MERCURY_GSTREAMER_PREFIX").is_none(),
        "{element} is missing from the bundled GStreamer (MERCURY_GSTREAMER_PREFIX is set, so this is a failure, not a skip)"
    );
    eprintln!("SKIP: {element} missing");
}
