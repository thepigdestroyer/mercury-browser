// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Private GStreamer set-up (item 1). Mirrors `apps/mercury-desktop/src/media_isolation.rs` and
//! servo-media-gstreamer's `init_with_plugins`: never touch another GStreamer's registry, never
//! scan system directories, and load the plugins either from the framework directory or one by one
//! by file name, the way Servo does.

use std::path::PathBuf;

/// How the plugins get into the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginMode {
    /// `GST_PLUGIN_SYSTEM_PATH_1_0` = `<prefix>/lib/gstreamer-1.0`, scanned in process
    /// (`GST_REGISTRY_FORK=no`), into a private registry file under `InitOptions::scratch_dir`.
    SystemPath,
    /// Servo's way (`GStreamerBackend::init_with_plugins`): `Plugin::load_file(<dir>/<file>)` for each
    /// name, nothing scanned. `names` are file names such as `libgstmse.dylib`; `dir` defaults to the
    /// framework's `lib/gstreamer-1.0`.
    ServoStyle {
        dir: Option<PathBuf>,
        names: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct InitOptions {
    /// The unpacked framework (`$MERCURY_GSTREAMER_PREFIX`, from `scripts/macos/gstreamer.sh env`);
    /// `None` reads that variable.
    pub prefix: Option<PathBuf>,
    pub plugin_mode: PluginMode,
    /// A private directory for the registry cache and `XDG_*` directories. Created if missing.
    pub scratch_dir: PathBuf,
    /// Run a GLib main loop on a background thread (older Servo needed one; GstMSE might). The point
    /// of the option is to find out whether anything depends on it: scenarios run both ways.
    pub glib_main_loop: bool,
}

/// What `init` found out; copied into the report.
#[derive(Debug, Clone)]
pub struct GstInfo {
    /// For example `1.28.9`.
    pub version: String,
    /// `x86_64` or `aarch64`.
    pub arch: &'static str,
    pub plugin_mode: String,
    /// Whether `msesrc` is available (the `mse` plugin loaded).
    pub msesrc_available: bool,
    /// Plugin files that failed to load in `ServoStyle` mode, with the reason.
    pub failed_plugins: Vec<(String, String)>,
}

/// Sets the environment, calls `gstreamer::init`, loads the plugins. Call once, first thing in
/// `main`, before any other thread exists (it sets environment variables).
pub fn init(_options: &InitOptions) -> Result<GstInfo, crate::RigError> {
    todo!("item 1")
}
