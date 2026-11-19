// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The "About Mercury Browser" window. It holds the `AboutSlint` widget that the Slint
//! Royalty-free License 2.0 requires (§2(a)), and is reached from the application menu and
//! from the browser menu.

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

use mercury_ui::{AboutLink, AboutWindow};
use slint::ComponentHandle;

use crate::menus::{self, Document, MenuCommand};

/// The line `Info.plist` carries as `NSHumanReadableCopyright` (checked by a test).
pub(crate) const COPYRIGHT_LINE: &str = "Copyright © 2026 Mrinal Joshi and the Mercury Browser contributors. Portions © 2026 Kayk Caputo (Hermes Browser).";
/// The licence terms in one line, as `LICENSE` and `NOTICE` give them.
pub(crate) const LICENCE_LINE: &str = "Source-available: free for non-commercial use \
    (PolyForm Noncommercial 1.0.0; MPL-2.0 for parts from Hermes and Servo).";
/// Mercury's origin, as `NOTICE` records it.
pub(crate) const CREDIT_LINE: &str = "Began as a fork of Hermes Browser by Kayk Caputo.";
/// Servo is pinned to exactly this version in the workspace `Cargo.toml` (checked by a test).
pub(crate) const ENGINE_LINE: &str = "Servo 0.8.0 web engine";
/// Where the packaging puts the page of third-party notices (`mercury-credits`), below the
/// directory that holds the application's resources (see `credits_candidates`).
pub(crate) const CREDITS_RELATIVE_PATH: &str = "credits/credits.html";
/// What "Third-party notices" opens when there is no bundled page: the notices file on GitHub.
pub(crate) const THIRD_PARTY_NOTICES_ONLINE_URL: &str =
    "https://github.com/thepigdestroyer/mercury-browser/blob/master/THIRD-PARTY-NOTICES.md";

thread_local! {
    static ABOUT: RefCell<Option<AboutWindow>> = const { RefCell::new(None) };
}

pub(crate) fn version_line() -> String {
    format!("Version {}", env!("CARGO_PKG_VERSION"))
}

/// Shown only when the build recorded the GStreamer version (`MERCURY_GSTREAMER_VERSION`).
pub(crate) fn media_line(gstreamer_version: Option<&str>) -> String {
    gstreamer_version
        .filter(|version| !version.is_empty())
        .map(|version| format!("GStreamer {version} media framework"))
        .unwrap_or_default()
}

/// Where the bundled third-party notices page may be, for the executable at `executable`, in
/// order: the macOS bundle's `Contents/Resources/credits/credits.html`; `credits/credits.html`
/// beside the executable (the Linux archive, and `target/<profile>/` after a build);
/// and `../share/mercury-browser/credits/credits.html` (a Linux installation under a prefix).
/// The same order as `filter_lists::bundled_dir_candidates`.
pub(crate) fn credits_candidates(executable: &Path) -> Vec<PathBuf> {
    let Some(directory) = executable.parent() else {
        return Vec::new();
    };
    vec![
        directory.join("../Resources").join(CREDITS_RELATIVE_PATH),
        directory.join(CREDITS_RELATIVE_PATH),
        directory
            .join("../share/mercury-browser")
            .join(CREDITS_RELATIVE_PATH),
    ]
}

/// The `file://` address of the first candidate for which `exists` says yes, or None. The
/// address is written by `url::Url::from_file_path`, so that the space in `Mercury Browser.app`
/// becomes `%20`; a candidate with `..` in it is first made clean (see `clean_path`).
pub(crate) fn credits_url(executable: &Path, exists: impl Fn(&Path) -> bool) -> Option<String> {
    credits_candidates(executable)
        .into_iter()
        .find(|candidate| exists(candidate))
        .and_then(|candidate| {
            url::Url::from_file_path(clean_path(&candidate))
                .ok()
                .map(String::from)
        })
}

/// What "Third-party notices" opens for the executable at `executable`: the bundled page when
/// `exists` finds it, otherwise the notices file on GitHub. A build that was not packaged (such
/// as `cargo run`) has no page, and is sent to GitHub.
pub(crate) fn third_party_notices_address(
    executable: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Cow<'static, str> {
    credits_url(executable, exists)
        .map_or(Cow::Borrowed(THIRD_PARTY_NOTICES_ONLINE_URL), Cow::Owned)
}

/// `third_party_notices_address` for the running program: the page that was packaged with it,
/// otherwise the notices file on GitHub.
pub(crate) fn third_party_notices_url() -> Cow<'static, str> {
    match std::env::current_exe() {
        Ok(executable) => third_party_notices_address(&executable, Path::is_file),
        Err(error) => {
            log::warn!("could not find the running program, so no bundled notices: {error}");
            Cow::Borrowed(THIRD_PARTY_NOTICES_ONLINE_URL)
        }
    }
}

/// `path` without `..`, so that the address made from it is clean: the file system's own answer
/// when the path exists (the path may go through a link), else the same by the names alone.
fn clean_path(path: &Path) -> PathBuf {
    if !path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return path.to_path_buf();
    }
    std::fs::canonicalize(path).unwrap_or_else(|_| lexically_clean(path))
}

fn lexically_clean(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match clean.components().next_back() {
                Some(Component::Normal(_)) => {
                    clean.pop();
                }
                // `/..` is `/`.
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => clean.push(".."),
            },
            other => clean.push(other.as_os_str()),
        }
    }
    clean
}

/// Shows the About window, creating it the first time.
pub(crate) fn show() {
    let existing = ABOUT.with(|about| about.borrow().as_ref().map(ComponentHandle::clone_strong));
    let about = match existing {
        Some(about) => about,
        None => match create() {
            Ok(about) => {
                ABOUT.with(|stored| stored.replace(Some(about.clone_strong())));
                about
            }
            Err(error) => {
                log::warn!("could not open the About window: {error}");
                return;
            }
        },
    };
    if let Err(error) = about.show() {
        log::warn!("could not show the About window: {error}");
    }
}

pub(crate) fn hide() {
    let about = ABOUT.with(|about| about.borrow().as_ref().map(ComponentHandle::clone_strong));
    if let Some(about) = about
        && let Err(error) = about.hide()
    {
        log::warn!("could not close the About window: {error}");
    }
}

/// Renders the open About window into a PNG through the app's own renderer (menu diagnostics).
#[cfg(all(target_os = "macos", feature = "menu-diagnostics"))]
pub(crate) fn save_snapshot(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let about = ABOUT
        .with(|about| about.borrow().as_ref().map(ComponentHandle::clone_strong))
        .ok_or("the About window is not open")?;
    let pixels = about.window().take_snapshot()?;
    image::save_buffer_with_format(
        path,
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )?;
    Ok(())
}

/// Drops the window before the engine and the browser window go away. Called from the end of
/// the run (`post_run.rs`), which may run inside AppKit's terminate callback: it only drops.
pub(crate) fn release() {
    ABOUT.with(|about| about.take());
}

fn create() -> Result<AboutWindow, slint::PlatformError> {
    let about = AboutWindow::new()?;
    // Its own copy of `ChromePalette`, following the appearance setting.
    crate::settings::apply_chrome_theme(&about);
    about.set_version_line(version_line().into());
    about.set_engine_line(ENGINE_LINE.into());
    about.set_media_line(media_line(option_env!("MERCURY_GSTREAMER_VERSION")).into());
    about.set_copyright_line(COPYRIGHT_LINE.into());
    about.set_licence_line(LICENCE_LINE.into());
    about.set_credit_line(CREDIT_LINE.into());
    about.on_open_link(|link| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let document = match link {
            AboutLink::Licence => Document::Licence,
            AboutLink::ThirdPartyNotices => Document::ThirdPartyNotices,
            AboutLink::SourceCode => Document::SourceCode,
        };
        menus::dispatch(MenuCommand::OpenDocument(document));
    });
    about.on_close_requested(hide);
    #[cfg(target_os = "macos")]
    menus::reinstall_when_key(about.window(), "the About window became key");
    Ok(about)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copyright_line_is_the_app_bundles() {
        let template = include_str!("../resources/macos/Info.plist.in");
        assert!(template.contains(&format!(
            "<key>NSHumanReadableCopyright</key>\n\t<string>{COPYRIGHT_LINE}</string>"
        )));
    }

    #[test]
    fn the_copyright_and_credit_lines_match_the_notice_file() {
        let notice = include_str!("../../../NOTICE");
        let holder = COPYRIGHT_LINE
            .split_inclusive(". ")
            .next()
            .expect("the copyright line has a first sentence")
            .trim_end();
        assert_eq!(
            holder,
            "Copyright © 2026 Mrinal Joshi and the Mercury Browser contributors."
        );
        assert!(notice.contains(holder));
        assert!(notice.contains("Mercury began as a fork of Hermes Browser"));
        assert!(notice.contains("Kayk Caputo"));
        assert!(CREDIT_LINE.contains("Hermes Browser by Kayk Caputo"));
        assert!(COPYRIGHT_LINE.ends_with("Portions © 2026 Kayk Caputo (Hermes Browser)."));
        assert!(notice.contains("PolyForm Noncommercial License 1.0.0"));
        assert!(LICENCE_LINE.contains("PolyForm Noncommercial 1.0.0"));
        assert!(LICENCE_LINE.contains("MPL-2.0"));
        assert!(!COPYRIGHT_LINE.contains("Licensed under"));
    }

    #[test]
    fn the_engine_line_names_the_pinned_servo() {
        let workspace = include_str!("../../../Cargo.toml");
        assert!(workspace.contains("servo = { version = \"=0.8.0\""));
        assert!(ENGINE_LINE.contains("Servo 0.8.0"));
    }

    #[test]
    fn the_version_is_the_workspace_version() {
        let workspace = include_str!("../../../Cargo.toml");
        let package = workspace
            .split("[workspace.package]")
            .nth(1)
            .and_then(|section| section.split("\n[").next())
            .expect("the workspace has a [workspace.package] section");
        let version = package
            .lines()
            .find_map(|line| line.strip_prefix("version = "))
            .expect("[workspace.package] has a version");
        assert_eq!(version, format!("\"{}\"", env!("CARGO_PKG_VERSION")));
        assert_eq!(
            version_line(),
            format!("Version {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn the_media_line_appears_only_when_the_version_is_known() {
        assert_eq!(media_line(None), "");
        assert_eq!(media_line(Some("")), "");
        assert_eq!(
            media_line(Some("1.28.9")),
            "GStreamer 1.28.9 media framework"
        );
    }

    /// The disk image's mount point: a path with spaces and a dot, which no machine has.
    const MOUNTED_EXECUTABLE: &str =
        "/Volumes/Mercury 0.2.0/Mercury Browser.app/Contents/MacOS/mercury-browser";

    #[test]
    fn the_credits_path_is_the_one_the_packaging_writes() {
        // `mercury-credits generate` writes `credits.html` into the `credits` directory that
        // `bundle.sh` puts in `Contents/Resources` and the Linux archive puts beside the program.
        assert_eq!(CREDITS_RELATIVE_PATH, "credits/credits.html");
    }

    #[test]
    fn the_credits_page_is_looked_for_in_the_bundle_then_beside_the_executable() {
        assert_eq!(
            credits_candidates(Path::new(MOUNTED_EXECUTABLE)),
            [
                PathBuf::from(
                    "/Volumes/Mercury 0.2.0/Mercury Browser.app/Contents/MacOS/../Resources/credits/credits.html"
                ),
                PathBuf::from(
                    "/Volumes/Mercury 0.2.0/Mercury Browser.app/Contents/MacOS/credits/credits.html"
                ),
                PathBuf::from(
                    "/Volumes/Mercury 0.2.0/Mercury Browser.app/Contents/MacOS/../share/mercury-browser/credits/credits.html"
                ),
            ]
        );
        assert_eq!(
            credits_candidates(Path::new("/opt/mercury/bin/mercury-browser"))[2],
            Path::new("/opt/mercury/bin/../share/mercury-browser/credits/credits.html")
        );
        // A path with no directory has nothing beside it.
        assert!(credits_candidates(Path::new("/")).is_empty());
    }

    #[test]
    fn the_bundled_page_opens_by_a_clean_file_address() {
        let address = credits_url(Path::new(MOUNTED_EXECUTABLE), |_| true)
            .expect("an address for a page that exists");
        // The spaces are percent-encoded and the `..` of the candidate is gone.
        assert_eq!(
            address,
            "file:///Volumes/Mercury%200.2.0/Mercury%20Browser.app/Contents/Resources/credits/credits.html"
        );
        let parsed = url::Url::parse(&address).expect("the address parses");
        assert_eq!(parsed.scheme(), "file");
        assert_eq!(
            parsed.to_file_path().expect("a local path"),
            Path::new(
                "/Volumes/Mercury 0.2.0/Mercury Browser.app/Contents/Resources/credits/credits.html"
            )
        );
    }

    #[test]
    fn the_first_candidate_that_exists_wins() {
        let beside = Path::new("/opt/mercury/bin/credits/credits.html");
        let shared = Path::new("/opt/mercury/bin/../share/mercury-browser/credits/credits.html");
        let executable = Path::new("/opt/mercury/bin/mercury-browser");
        assert_eq!(
            credits_url(executable, |path| path == beside || path == shared).as_deref(),
            Some("file:///opt/mercury/bin/credits/credits.html")
        );
        assert_eq!(
            credits_url(executable, |path| path == shared).as_deref(),
            Some("file:///opt/mercury/share/mercury-browser/credits/credits.html")
        );
        // The bundle's own directory comes before the others.
        let mac = credits_url(Path::new(MOUNTED_EXECUTABLE), |_| true).expect("an address");
        assert!(
            mac.contains("/Contents/Resources/credits/credits.html"),
            "{mac}"
        );
    }

    #[test]
    fn without_a_bundled_page_the_notices_open_on_github() {
        let executable = Path::new("/Users/me/mercury-browser/target/release/mercury-browser");
        assert_eq!(credits_url(executable, |_| false), None);
        assert_eq!(
            third_party_notices_address(executable, |_| false),
            THIRD_PARTY_NOTICES_ONLINE_URL
        );
        assert_eq!(
            THIRD_PARTY_NOTICES_ONLINE_URL,
            "https://github.com/thepigdestroyer/mercury-browser/blob/master/THIRD-PARTY-NOTICES.md"
        );
        // A path that is not absolute cannot be an address, so GitHub it is.
        assert_eq!(
            third_party_notices_address(Path::new("bin/mercury-browser"), |_| true),
            THIRD_PARTY_NOTICES_ONLINE_URL
        );
        assert_eq!(
            third_party_notices_address(Path::new("/"), |_| true),
            THIRD_PARTY_NOTICES_ONLINE_URL
        );
    }

    #[test]
    fn a_page_in_a_bundle_on_disk_is_found_and_its_address_is_clean() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let contents = directory.path().join("Mercury Browser.app/Contents");
        std::fs::create_dir_all(contents.join("MacOS")).expect("the MacOS directory");
        std::fs::create_dir_all(contents.join("Resources/credits")).expect("the credits directory");
        std::fs::write(
            contents.join("Resources/credits/credits.html"),
            "<html></html>",
        )
        .expect("the page");
        let executable = contents.join("MacOS/mercury-browser");

        let address = third_party_notices_address(&executable, Path::is_file);
        let parsed = url::Url::parse(&address).expect("the address parses");
        assert_eq!(parsed.scheme(), "file");
        assert!(!address.contains(".."), "{address}");
        assert!(
            address.ends_with("/Mercury%20Browser.app/Contents/Resources/credits/credits.html"),
            "{address}"
        );
        assert!(parsed.to_file_path().expect("a local path").is_file());

        // Without the page, the same bundle sends the reader to GitHub.
        std::fs::remove_file(contents.join("Resources/credits/credits.html")).expect("remove");
        assert_eq!(
            third_party_notices_address(&executable, Path::is_file),
            THIRD_PARTY_NOTICES_ONLINE_URL
        );
    }

    #[test]
    fn dots_in_a_path_are_removed_by_the_names_when_the_file_system_cannot() {
        assert_eq!(clean_path(Path::new("/a b/c")), Path::new("/a b/c"));
        assert_eq!(
            lexically_clean(Path::new("/a/b/../c/./d")),
            Path::new("/a/c/d")
        );
        assert_eq!(lexically_clean(Path::new("/../a")), Path::new("/a"));
        assert_eq!(lexically_clean(Path::new("a/../../b")), Path::new("../b"));
    }
}
