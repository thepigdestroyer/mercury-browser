// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Private GStreamer set-up (item 1). Mirrors `apps/mercury-desktop/src/media_isolation.rs` and
//! servo-media-gstreamer's `init_with_plugins`: never touch another GStreamer's registry, never
//! scan system directories, and load the plugins either from the framework directory or one by one
//! by file name, the way Servo does.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::RigError;

/// How the plugins get into the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginMode {
    /// `GST_PLUGIN_SYSTEM_PATH_1_0` = `<scratch>/plugins`, a directory of symlinks to the framework's
    /// plugin files (all of `<prefix>/lib/gstreamer-1.0` except [`SCAN_DENYLIST`]), scanned in process
    /// (`GST_REGISTRY_FORK=no`) into a private registry file under `InitOptions::scratch_dir`. Scanning
    /// the framework directory itself hangs on this Mac: see [`SCAN_DENYLIST`].
    SystemPath,
    /// Servo's way (`GStreamerBackend::init_with_plugins`): `Plugin::load_file(<dir>/<file>)` for each
    /// name, nothing scanned. `names` are file names such as `libgstmse.dylib`; `dir` defaults to the
    /// framework's `lib/gstreamer-1.0`. An empty `names` means "the default list" ([`servo_style_names`]).
    ServoStyle {
        dir: Option<PathBuf>,
        names: Vec<String>,
    },
}

impl PluginMode {
    /// `ServoStyle` with the default directory and the default file list.
    pub fn servo_default() -> Self {
        PluginMode::ServoStyle {
            dir: None,
            names: Vec::new(),
        }
    }

    /// The name used in reports: `system-path` or `servo-style`.
    pub fn label(&self) -> &'static str {
        match self {
            PluginMode::SystemPath => "system-path",
            PluginMode::ServoStyle { .. } => "servo-style",
        }
    }
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

// ----------------------------------------------------------------------------------------------
// The files Mercury ships today.
// ----------------------------------------------------------------------------------------------

/// The plugin files a macOS build of Mercury loads today, in Servo's order: the lists in the
/// `servo` 0.8 crate's `gstreamer_plugin_lists/{common,macos}.rs.in` minus the one exclusion
/// `scripts/macos/gstreamer-plugins.sh` makes (`ogg`). [`servo_style_names`] reads the lists
/// themselves when `MERCURY_SERVO_PLUGIN_LISTS` names a directory, and a test (run when that
/// variable is set) checks that this constant still equals them.
pub const MERCURY_PLUGIN_FILES: &[&str] = &[
    // common.rs.in
    "libgstcoreelements.dylib",
    "libgstnice.dylib",
    "libgstapp.dylib",
    "libgstaudioconvert.dylib",
    "libgstaudioresample.dylib",
    "libgstgio.dylib",
    "libgstopengl.dylib",
    "libgstopus.dylib",
    "libgstplayback.dylib",
    "libgsttheora.dylib",
    "libgsttypefindfunctions.dylib",
    "libgstvideoconvertscale.dylib",
    "libgstvolume.dylib",
    "libgstvorbis.dylib",
    "libgstaudiofx.dylib",
    "libgstaudioparsers.dylib",
    "libgstautodetect.dylib",
    "libgstdeinterlace.dylib",
    "libgstid3demux.dylib",
    "libgstinterleave.dylib",
    "libgstisomp4.dylib",
    "libgstmatroska.dylib",
    "libgstrtp.dylib",
    "libgstrtpmanager.dylib",
    "libgstvideofilter.dylib",
    "libgstvpx.dylib",
    "libgstwavparse.dylib",
    "libgstaudiobuffersplit.dylib",
    "libgstdtls.dylib",
    "libgstid3tag.dylib",
    "libgstproxy.dylib",
    "libgstvideoparsersbad.dylib",
    "libgstwebrtc.dylib",
    "libgstlibav.dylib",
    // macos.rs.in
    "libgstosxaudio.dylib",
    "libgstosxvideo.dylib",
    "libgstapplemedia.dylib",
];

/// What the MSE experiment adds to Mercury's list: the plugin that holds `msesrc` (EA0).
pub const MSE_PLUGIN_FILE: &str = "libgstmse.dylib";
/// Optional: the AV1 decoder (not in Servo's list; EA0 decides).
pub const DAV1D_PLUGIN_FILE: &str = "libgstdav1d.dylib";

/// The plugin names (`gstfoo`, no `lib` prefix and no extension) in the text of one of Servo's
/// `*.rs.in` lists: every quoted string on a line that is not a `//` comment.
pub fn parse_plugin_list(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let mut rest = line;
        while let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('"') else { break };
            names.push(after[..end].to_owned());
            rest = &after[end + 1..];
        }
    }
    names
}

/// The `servo`-style file list: Mercury's files (from `$MERCURY_SERVO_PLUGIN_LISTS/{common,macos}.rs.in`
/// minus `gstogg` when that variable is set, else [`MERCURY_PLUGIN_FILES`]) plus `libgstmse.dylib`,
/// plus `libgstdav1d.dylib` when `with_dav1d` is true.
pub fn servo_style_names(with_dav1d: bool) -> Vec<String> {
    let pinned = || -> Vec<String> {
        MERCURY_PLUGIN_FILES
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    };
    let mut names = match std::env::var_os("MERCURY_SERVO_PLUGIN_LISTS") {
        Some(dir) => read_plugin_lists(Path::new(&dir)).unwrap_or_else(pinned),
        None => pinned(),
    };
    names.push(MSE_PLUGIN_FILE.to_owned());
    if with_dav1d {
        names.push(DAV1D_PLUGIN_FILE.to_owned());
    }
    names
}

/// Reads Servo's two lists from `dir` and maps them to file names, dropping `gstogg` (Mercury's
/// exclusion). `None` when a list is missing or empty.
fn read_plugin_lists(dir: &Path) -> Option<Vec<String>> {
    let mut files = Vec::new();
    for list in ["common.rs.in", "macos.rs.in"] {
        let text = std::fs::read_to_string(dir.join(list)).ok()?;
        for name in parse_plugin_list(&text) {
            if name != "gstogg" {
                files.push(format!("lib{name}.dylib"));
            }
        }
    }
    (!files.is_empty()).then_some(files)
}

// ----------------------------------------------------------------------------------------------
// The environment.
// ----------------------------------------------------------------------------------------------

/// One change to the process environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvChange {
    Set(String, OsString),
    Remove(String),
}

/// `GST_*` variables that are kept when inherited: debugging aids that cannot make GStreamer load
/// anything from outside the private set-up.
const KEPT_GST_VARIABLES: &[&str] = &[
    "GST_DEBUG",
    "GST_DEBUG_NO_COLOR",
    "GST_DEBUG_FILE",
    "GST_DEBUG_DUMP_DOT_DIR",
];

/// Plugin files the in-process scan leaves out, with the reason:
///
/// * `libgstvulkan.dylib`: a fresh scan hung twice (the scan process stuck in `plugin_init` of the
///   Vulkan plugin: MoltenVK `vkCreateInstance`, then the AMD Radeon driver's `IOConnectCallMethod`; seen
///   on the Intel i7-9750H MacBook Pro with a discrete Radeon). A persistent registry hides it, a fresh
///   one per run does not. Mercury never loads it (Servo loads its list by file name).
/// * `libgstpython.dylib`: needs a `Python3.framework` the bundle does not have; its load only warns.
pub const SCAN_DENYLIST: &[&str] = &["libgstvulkan.dylib", "libgstpython.dylib"];

/// Which of the file names in the framework's plugin directory the scan gets: `lib*.dylib`, minus the
/// [`SCAN_DENYLIST`] (the directory also holds the static `.a` copies). Sorted.
pub fn scan_plugin_names(listing: &[String]) -> Vec<String> {
    let mut names: Vec<String> = listing
        .iter()
        .filter(|name| name.starts_with("libgst") && name.ends_with(".dylib"))
        .filter(|name| !SCAN_DENYLIST.contains(&name.as_str()))
        .cloned()
        .collect();
    names.sort_unstable();
    names
}

/// Makes `<scratch>/plugins` hold a symlink to each plugin file [`scan_plugin_names`] selects from
/// `<prefix>/lib/gstreamer-1.0` (existing links are kept). Returns how many there are.
fn prepare_scan_directory(prefix: &Path, scratch: &Path) -> Result<usize, RigError> {
    let source = prefix.join("lib").join("gstreamer-1.0");
    let target = scratch.join("plugins");
    std::fs::create_dir_all(&target)
        .map_err(|error| RigError::Setup(format!("cannot create {}: {error}", target.display())))?;
    let listing: Vec<String> = std::fs::read_dir(&source)
        .map_err(|error| RigError::Setup(format!("cannot read {}: {error}", source.display())))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    let names = scan_plugin_names(&listing);
    for name in &names {
        let link = target.join(name);
        if link.symlink_metadata().is_ok() {
            continue;
        }
        std::os::unix::fs::symlink(source.join(name), &link)
            .map_err(|error| RigError::Setup(format!("cannot link {}: {error}", link.display())))?;
    }
    Ok(names.len())
}

/// The registry file inside the scratch directory (named by architecture: a copied scratch from the
/// other architecture would not be used).
pub fn registry_file(scratch: &Path) -> PathBuf {
    scratch.join(format!("registry-{}.bin", std::env::consts::ARCH))
}

/// The environment that isolates GStreamer, as Mercury does (`media_isolation.rs`), computed from
/// the options and the names of the variables the process inherited. Pure: nothing is applied here.
///
/// * `GST_REGISTRY_1_0` = a file in the scratch directory; `GST_REGISTRY_FORK=no`, so a registry
///   update scans in process and never starts `gst-plugin-scanner`.
/// * `GST_PLUGIN_PATH_1_0` = empty. `GST_PLUGIN_SYSTEM_PATH_1_0` = `<scratch>/plugins` (see [`prepare_scan_directory`]) in
///   [`PluginMode::SystemPath`] and empty in [`PluginMode::ServoStyle`] (an unset value would mean
///   the user's home plugins and the default directory).
/// * `GIO_MODULE_DIR` = an empty directory in the scratch directory; `GIO_EXTRA_MODULES` removed.
/// * `XDG_CACHE_HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_RUNTIME_DIR` inside the scratch directory.
/// * Every other inherited `GST_*` variable is removed, except the debugging aids in
///   `KEPT_GST_VARIABLES` (`GST_DEBUG` and friends).
///
/// Removals come first in the result, so a variable that is both inherited and set ends up set.
pub fn isolation_changes(
    scratch: &Path,
    mode: &PluginMode,
    inherited: &[String],
) -> Vec<EnvChange> {
    let system_path = match mode {
        PluginMode::SystemPath => scratch.join("plugins").into_os_string(),
        PluginMode::ServoStyle { .. } => OsString::new(),
    };
    let xdg = scratch.join("xdg");
    let sets: Vec<(&str, OsString)> = vec![
        ("GST_REGISTRY_1_0", registry_file(scratch).into_os_string()),
        ("GST_PLUGIN_PATH_1_0", OsString::new()),
        ("GST_PLUGIN_SYSTEM_PATH_1_0", system_path),
        ("GST_REGISTRY_FORK", OsString::from("no")),
        (
            "GIO_MODULE_DIR",
            scratch.join("gio-modules").into_os_string(),
        ),
        ("XDG_CACHE_HOME", xdg.join("cache").into_os_string()),
        ("XDG_DATA_HOME", xdg.join("data").into_os_string()),
        ("XDG_CONFIG_HOME", xdg.join("config").into_os_string()),
        ("XDG_RUNTIME_DIR", xdg.join("runtime").into_os_string()),
    ];

    let mut removals: Vec<String> = [
        "GST_REGISTRY",
        "GST_PLUGIN_PATH",
        "GST_PLUGIN_SYSTEM_PATH",
        "GST_PLUGIN_SCANNER",
        "GST_PLUGIN_SCANNER_1_0",
        "GIO_EXTRA_MODULES",
    ]
    .iter()
    .map(|name| (*name).to_owned())
    .collect();
    for name in inherited {
        let is_gst = name.starts_with("GST_");
        let kept = KEPT_GST_VARIABLES.contains(&name.as_str());
        let overwritten = sets.iter().any(|(set, _)| set == name);
        if is_gst && !kept && !overwritten && !removals.contains(name) {
            removals.push(name.clone());
        }
    }

    removals
        .into_iter()
        .map(EnvChange::Remove)
        .chain(
            sets.into_iter()
                .map(|(name, value)| EnvChange::Set(name.to_owned(), value)),
        )
        .collect()
}

/// Applies environment changes to this process.
///
/// # Safety
///
/// `std::env::set_var` and `remove_var` are unsafe in edition 2024 because another thread may read
/// the environment (`getenv`) at the same time. The caller must make sure none does: [`init`]
/// calls this once, as its first act, at the top of `main` (the scenario binaries start no thread
/// before it) or, in tests, from inside a `OnceLock` that every test thread passes before it touches
/// GStreamer or the environment.
unsafe fn apply(changes: &[EnvChange]) {
    for change in changes {
        // SAFETY: the caller guarantees that no other thread reads or writes the environment.
        unsafe {
            match change {
                EnvChange::Set(name, value) => std::env::set_var(name, value),
                EnvChange::Remove(name) => std::env::remove_var(name),
            }
        }
    }
}

// ----------------------------------------------------------------------------------------------
// init
// ----------------------------------------------------------------------------------------------

static INIT: OnceLock<Result<GstInfo, String>> = OnceLock::new();

/// Sets the environment, calls `gstreamer::init`, loads the plugins. Call once, first thing in
/// `main`, before any other thread exists (it sets environment variables). A second call in the same
/// process returns the first call's result without doing anything (GStreamer can only be initialised
/// once and its environment is read once), whatever options it passes.
pub fn init(options: &InitOptions) -> Result<GstInfo, RigError> {
    INIT.get_or_init(|| init_once(options).map_err(|error| error.to_string()))
        .clone()
        .map_err(RigError::Setup)
}

fn framework_prefix(options: &InitOptions) -> Result<PathBuf, RigError> {
    let prefix = match &options.prefix {
        Some(prefix) => prefix.clone(),
        None => std::env::var_os("MERCURY_GSTREAMER_PREFIX")
            .map(PathBuf::from)
            .ok_or_else(|| {
                RigError::Setup(
                    "MERCURY_GSTREAMER_PREFIX is not set; run through spikes/mse/run.sh \
                     (it needs `make gstreamer-macos` once)"
                        .to_owned(),
                )
            })?,
    };
    if !prefix.join("lib").join("gstreamer-1.0").is_dir() {
        return Err(RigError::Setup(format!(
            "{} has no lib/gstreamer-1.0 (is the framework unpacked? run: make gstreamer-macos)",
            prefix.display()
        )));
    }
    Ok(prefix)
}

fn init_once(options: &InitOptions) -> Result<GstInfo, RigError> {
    let prefix = framework_prefix(options)?;
    let scratch = &options.scratch_dir;
    for directory in [
        scratch.clone(),
        scratch.join("gio-modules"),
        scratch.join("xdg").join("cache"),
        scratch.join("xdg").join("data"),
        scratch.join("xdg").join("config"),
        scratch.join("xdg").join("runtime"),
    ] {
        std::fs::create_dir_all(&directory).map_err(|error| {
            RigError::Setup(format!("cannot create {}: {error}", directory.display()))
        })?;
    }

    let inherited: Vec<String> = std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    if options.plugin_mode == PluginMode::SystemPath {
        prepare_scan_directory(&prefix, scratch)?;
    }
    let changes = isolation_changes(scratch, &options.plugin_mode, &inherited);
    // SAFETY: see `apply`: this is the first thing the process does with GStreamer or the
    // environment, before any thread of ours exists, and `INIT` serialises every other caller.
    unsafe { apply(&changes) };

    gstreamer::init().map_err(|error| RigError::Setup(format!("gstreamer::init: {error}")))?;

    if options.glib_main_loop {
        std::thread::Builder::new()
            .name("mse-rig-glib-main".to_owned())
            .spawn(|| glib::MainLoop::new(None, false).run())
            .map_err(|error| {
                RigError::Setup(format!("cannot start the GLib main loop: {error}"))
            })?;
    }

    let mut failed_plugins = Vec::new();
    if let PluginMode::ServoStyle { dir, names } = &options.plugin_mode {
        let dir = dir
            .clone()
            .unwrap_or_else(|| prefix.join("lib").join("gstreamer-1.0"));
        let names = if names.is_empty() {
            servo_style_names(false)
        } else {
            names.clone()
        };
        let registry = gstreamer::Registry::get();
        for name in &names {
            let path = dir.join(name);
            match gstreamer::Plugin::load_file(&path) {
                Ok(plugin) => {
                    if registry.add_plugin(&plugin).is_err() {
                        failed_plugins
                            .push((name.clone(), "Registry::add_plugin failed".to_owned()));
                    }
                }
                Err(error) => failed_plugins.push((name.clone(), error.to_string())),
            }
        }
    }

    let (major, minor, micro, _nano) = gstreamer::version();
    Ok(GstInfo {
        version: format!("{major}.{minor}.{micro}"),
        arch: std::env::consts::ARCH,
        plugin_mode: options.plugin_mode.label().to_owned(),
        msesrc_available: gstreamer::ElementFactory::find("msesrc").is_some(),
        failed_plugins,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(mode: &PluginMode, inherited: &[&str]) -> Vec<EnvChange> {
        let inherited: Vec<String> = inherited.iter().map(|s| (*s).to_owned()).collect();
        isolation_changes(Path::new("/scratch"), mode, &inherited)
    }

    fn value_of(changes: &[EnvChange], name: &str) -> Option<OsString> {
        changes.iter().find_map(|change| match change {
            EnvChange::Set(key, value) if key == name => Some(value.clone()),
            _ => None,
        })
    }

    fn removes(changes: &[EnvChange], name: &str) -> bool {
        changes.contains(&EnvChange::Remove(name.to_owned()))
    }

    #[test]
    fn the_registry_is_a_private_file_in_the_scratch_directory() {
        let changes = changes(&PluginMode::SystemPath, &[]);
        let registry = value_of(&changes, "GST_REGISTRY_1_0").unwrap();
        assert_eq!(
            PathBuf::from(registry),
            Path::new("/scratch").join(format!("registry-{}.bin", std::env::consts::ARCH))
        );
        assert!(removes(&changes, "GST_REGISTRY"));
        assert_eq!(value_of(&changes, "GST_REGISTRY_FORK"), Some("no".into()));
        assert!(removes(&changes, "GST_PLUGIN_SCANNER"));
        assert!(removes(&changes, "GST_PLUGIN_SCANNER_1_0"));
    }

    #[test]
    fn system_path_mode_scans_only_the_scratch_symlink_directory() {
        let changes = changes(&PluginMode::SystemPath, &[]);
        assert_eq!(
            value_of(&changes, "GST_PLUGIN_SYSTEM_PATH_1_0"),
            Some("/scratch/plugins".into())
        );
        assert_eq!(
            value_of(&changes, "GST_PLUGIN_PATH_1_0"),
            Some(OsString::new())
        );
        assert!(removes(&changes, "GST_PLUGIN_PATH"));
        assert!(removes(&changes, "GST_PLUGIN_SYSTEM_PATH"));
    }

    #[test]
    fn servo_style_mode_scans_nothing() {
        // Empty, not unset: an unset system path means "the user's home plugins and the default directory".
        let changes = changes(&PluginMode::servo_default(), &[]);
        assert_eq!(
            value_of(&changes, "GST_PLUGIN_SYSTEM_PATH_1_0"),
            Some(OsString::new())
        );
        assert_eq!(
            value_of(&changes, "GST_PLUGIN_PATH_1_0"),
            Some(OsString::new())
        );
    }

    #[test]
    fn inherited_gst_variables_are_removed_except_debugging_aids() {
        let changes = changes(
            &PluginMode::SystemPath,
            &[
                "GST_DEBUG",
                "GST_DEBUG_FILE",
                "GST_PLUGIN_FEATURE_RANK",
                "GST_REGISTRY_UPDATE",
                "GST_REGISTRY_1_0",
                "HOME",
            ],
        );
        assert!(removes(&changes, "GST_PLUGIN_FEATURE_RANK"));
        assert!(removes(&changes, "GST_REGISTRY_UPDATE"));
        assert!(!removes(&changes, "GST_DEBUG"));
        assert!(!removes(&changes, "GST_DEBUG_FILE"));
        assert!(!removes(&changes, "HOME"));
        // An inherited variable the plan also sets is set, not removed.
        assert!(!removes(&changes, "GST_REGISTRY_1_0"));
        assert!(value_of(&changes, "GST_REGISTRY_1_0").is_some());
    }

    #[test]
    fn gio_modules_and_user_directories_are_private() {
        let changes = changes(&PluginMode::SystemPath, &["GIO_EXTRA_MODULES"]);
        assert!(removes(&changes, "GIO_EXTRA_MODULES"));
        assert_eq!(
            value_of(&changes, "GIO_MODULE_DIR"),
            Some("/scratch/gio-modules".into())
        );
        for (name, child) in [
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            assert_eq!(
                value_of(&changes, name),
                Some(Path::new("/scratch/xdg").join(child).into_os_string()),
                "{name}"
            );
        }
    }

    #[test]
    fn every_variable_is_named_once_and_removals_come_first() {
        let changes = changes(
            &PluginMode::SystemPath,
            &["GST_FOO", "GST_REGISTRY", "GST_FOO"],
        );
        let names: Vec<&str> = changes
            .iter()
            .map(|change| match change {
                EnvChange::Set(name, _) | EnvChange::Remove(name) => name.as_str(),
            })
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "{names:?}");
        let first_set = changes
            .iter()
            .position(|change| matches!(change, EnvChange::Set(..)))
            .unwrap();
        assert!(
            changes[first_set..]
                .iter()
                .all(|change| matches!(change, EnvChange::Set(..)))
        );
    }

    #[test]
    fn plugin_lists_parse_like_the_script_does() {
        let text = "// comment \"not-a-plugin\"\n[\n// gstreamer\n\"gstcoreelements\",\n\"gstnice\", \"gstapp\",\n]";
        assert_eq!(
            parse_plugin_list(text),
            vec!["gstcoreelements", "gstnice", "gstapp"]
        );
    }

    #[test]
    fn the_default_servo_style_list_is_mercurys_files_plus_mse() {
        let names = servo_style_names(false);
        assert!(names.contains(&"libgstcoreelements.dylib".to_owned()));
        assert!(names.contains(&"libgstplayback.dylib".to_owned()));
        assert!(names.contains(&"libgstisomp4.dylib".to_owned()));
        assert!(names.contains(&"libgstapplemedia.dylib".to_owned()));
        assert!(
            !names.contains(&"libgstogg.dylib".to_owned()),
            "Mercury excludes ogg"
        );
        assert_eq!(names.last().map(String::as_str), Some(MSE_PLUGIN_FILE));
        assert!(!names.contains(&DAV1D_PLUGIN_FILE.to_owned()));
        assert_eq!(
            servo_style_names(true).last().map(String::as_str),
            Some(DAV1D_PLUGIN_FILE)
        );
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
    }

    /// Runs only when `MERCURY_SERVO_PLUGIN_LISTS` names Servo's `gstreamer_plugin_lists` directory
    /// (for example `~/.cargo/registry/src/*/servo-0.8.0/gstreamer_plugin_lists`).
    #[test]
    fn the_pinned_list_equals_servos_lists_when_they_are_at_hand() {
        let Some(dir) = std::env::var_os("MERCURY_SERVO_PLUGIN_LISTS") else {
            return;
        };
        let from_lists = read_plugin_lists(Path::new(&dir)).expect("Servo's plugin lists");
        let pinned: Vec<String> = MERCURY_PLUGIN_FILES
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        assert_eq!(pinned, from_lists);
    }

    #[test]
    fn the_scan_leaves_out_vulkan_python_and_static_libraries() {
        let listing: Vec<String> = [
            "libgstapp.dylib",
            "libgstapp.a",
            "libgstvulkan.dylib",
            "libgstpython.dylib",
            "libgstmse.dylib",
            "libother.dylib",
            "include",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(
            scan_plugin_names(&listing),
            vec!["libgstapp.dylib", "libgstmse.dylib"]
        );
    }

    #[test]
    fn the_scan_directory_is_made_of_symlinks_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!("mse-rig-scan-test-{}", std::process::id()));
        let prefix = root.join("prefix");
        let plugins = prefix.join("lib").join("gstreamer-1.0");
        std::fs::create_dir_all(&plugins).unwrap();
        for name in [
            "libgstapp.dylib",
            "libgstvulkan.dylib",
            "libgstmse.dylib",
            "libgstmse.a",
        ] {
            std::fs::write(plugins.join(name), b"x").unwrap();
        }
        let scratch = root.join("scratch");
        assert_eq!(prepare_scan_directory(&prefix, &scratch).unwrap(), 2);
        assert_eq!(prepare_scan_directory(&prefix, &scratch).unwrap(), 2);
        let link = scratch.join("plugins").join("libgstmse.dylib");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            plugins.join("libgstmse.dylib")
        );
        assert!(!scratch.join("plugins").join("libgstvulkan.dylib").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn labels_are_stable() {
        assert_eq!(PluginMode::SystemPath.label(), "system-path");
        assert_eq!(PluginMode::servo_default().label(), "servo-style");
    }
}
