// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Images per site, in the window: the sites whose images are off are read from
//! `site-images.json` once at start-up, pushed to the engine's guard, and changed from the
//! menus and the site popover.
//!
//! The set lives here (on the UI thread, like every other setting): `init` loads it,
//! `attach_engine` hands a copy to `talaria_servo::SiteImagesHandle` (which the engine asks at
//! every image load), and `set_active_site_images` changes all three (the set, the file, the
//! engine) and reloads the tab, because an image already drawn would stay (brief decision
//! O10). The menus and the popover only ask `can_set_site_images` and
//! `active_site_blocks_images`.
//!
//! Without a profile directory (a temporary profile, a test) the set is kept for the run and
//! never written.

use std::cell::RefCell;
use std::path::Path;

use mercury_settings::{Loaded, SiteImagesFile, SiteImagesStore};
use mercury_ui::MainWindow;
use slint::ComponentHandle as _;
use talaria_core::{
    BrowserCommand, BrowserController, SiteImages, SiteImagesError, image_site_key,
};
use talaria_servo::{ServoRuntime, SiteImagesHandle};
use url::Url;

/// What the module knows, once per process (the UI thread only).
#[derive(Default)]
struct State {
    /// The sites whose images are off.
    images: SiteImages,
    /// The file; `None` without a profile directory.
    store: Option<SiteImagesStore>,
    /// False once the file turned out to be from a newer version: it is never saved over.
    saving: bool,
    /// The engine's guard, once the engine exists.
    engine: Option<SiteImagesHandle>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Loads `site-images.json` from the profile once at start-up. A damaged file was moved aside
/// by the store and counts as empty; a file from a newer version counts as empty and is never
/// saved over; neither is told to the user (the log has a line).
pub(crate) fn init(profile_dir: &Path) {
    let store = SiteImagesStore::new(profile_dir);
    let (images, saving) = load(&store);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.images = images;
        state.store = Some(store);
        state.saving = saving;
    });
}

/// What the file holds as a set, and whether saving may go on.
fn load(store: &SiteImagesStore) -> (SiteImages, bool) {
    match store.load() {
        Ok(Loaded::Missing(file)) => (from_file(&file), true),
        Ok(Loaded::Ok { value, repairs }) => {
            let images = from_file(&value);
            if !repairs.is_empty() {
                // Entries were dropped or reordered: write the repaired list once.
                save(store, &images);
            }
            (images, true)
        }
        Ok(Loaded::Damaged { backup, .. }) => {
            log::warn!(
                "site-images.json could not be read; kept at {}; images are on everywhere",
                backup.display()
            );
            (SiteImages::default(), true)
        }
        Ok(Loaded::Newer { found_version, .. }) => {
            log::warn!(
                "site-images.json is from a newer Mercury (format {found_version}); images are \
                 on everywhere and the file is left alone"
            );
            (SiteImages::default(), false)
        }
        Err(error) => {
            log::warn!("site-images.json could not be loaded: {error}; images are on everywhere");
            (SiteImages::default(), true)
        }
    }
}

fn from_file(file: &SiteImagesFile) -> SiteImages {
    SiteImages::from_hosts(file.blocked_hosts.iter().cloned())
}

fn save(store: &SiteImagesStore, images: &SiteImages) {
    let file = SiteImagesFile::new(images.hosts().map(str::to_owned));
    if let Err(error) = store.save(&file) {
        log::warn!("site-images.json was not saved: {error}");
    }
}

/// Hands the set to the engine's guard. Called once, when the engine exists.
pub(crate) fn attach_engine(runtime: &ServoRuntime) {
    let handle = runtime.site_images();
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        handle.replace(state.images.clone());
        state.engine = Some(handle);
    });
}

/// The page the active tab shows, if it is an `http(s)` page that has a site.
fn active_web_page(controller: &BrowserController) -> Option<Url> {
    let page = Url::parse(controller.active_state().url()).ok()?;
    image_site_key(&page).map(|_| page)
}

/// Whether the active tab's site has images off.
pub(crate) fn active_site_blocks_images(controller: &BrowserController) -> bool {
    active_web_page(controller)
        .is_some_and(|page| STATE.with(|state| state.borrow().images.blocks(&page)))
}

/// Whether images can be turned off or on for the active tab: it shows an `http(s)` page.
pub(crate) fn can_set_site_images(controller: &BrowserController) -> bool {
    active_web_page(controller).is_some()
}

/// Turns images off (`true`) or on (`false`) for the active tab's site: updates the set, saves
/// the file, pushes the set to the engine, reloads the tab (a plain reload, decision O10) and
/// refreshes the site popover. Nothing happens, apart from the refresh, when the tab shows no
/// web page, the site is already so, or the thousand sites of the limit are all in use.
pub(crate) fn set_active_site_images(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
    blocked: bool,
) {
    let page = active_web_page(&controller.borrow());
    let changed = page.is_some_and(|page| change(&page, blocked));
    if changed {
        controller
            .borrow_mut()
            .queue_command(BrowserCommand::Reload);
        window.window().request_redraw();
    }
    sync_popover(window, &controller.borrow());
}

/// Applies the change to the set, the file and the engine; whether the set changed.
fn change(page: &Url, blocked: bool) -> bool {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        match state.images.set_blocked(page, blocked) {
            Ok(false) => false,
            Ok(true) => {
                if let Some(store) = state.store.as_ref().filter(|_| state.saving) {
                    save(store, &state.images);
                }
                if let Some(engine) = &state.engine {
                    engine.replace(state.images.clone());
                }
                true
            }
            Err(SiteImagesError::TooMany) => {
                log::warn!(
                    "images are off on as many sites as can be kept ({}); not adding another",
                    talaria_core::MAX_IMAGE_BLOCKED_SITES
                );
                false
            }
            Err(SiteImagesError::NotAWebPage) => false,
        }
    })
}

/// The two properties the site popover's "Images" row reads. `refresh_site_info` in
/// `site_security.rs` calls this whenever it fills the popover.
pub(crate) fn sync_popover(window: &MainWindow, controller: &BrowserController) {
    window.set_site_images_available(can_set_site_images(controller));
    window.set_site_images_blocked(active_site_blocks_images(controller));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a valid URL")
    }

    fn blocked_now(page: &str) -> bool {
        STATE.with(|state| state.borrow().images.blocks(&url(page)))
    }

    #[test]
    fn a_change_updates_the_set_and_the_file() {
        let profile = tempfile::tempdir().expect("a profile");
        init(profile.path());
        // The engine's handle is a plain holder here (its guard is tested in `talaria-servo`).
        STATE.with(|state| state.borrow_mut().engine = Some(SiteImagesHandle::default()));

        assert!(change(&url("https://example.com/a"), true));
        assert!(blocked_now("http://example.com:8080/"));
        // Nothing to do the second time.
        assert!(!change(&url("https://example.com/b"), true));

        let Ok(Loaded::Ok { value, .. }) = SiteImagesStore::new(profile.path()).load() else {
            panic!("the file should hold the change");
        };
        assert_eq!(value.blocked_hosts, ["example.com"]);

        assert!(change(&url("https://example.com/"), false));
        assert!(!blocked_now("https://example.com/"));
        let Ok(Loaded::Ok { value, .. }) = SiteImagesStore::new(profile.path()).load() else {
            panic!("the file should hold the change");
        };
        assert!(value.blocked_hosts.is_empty());
    }

    #[test]
    fn a_new_run_starts_with_what_the_last_one_saved() {
        let profile = tempfile::tempdir().expect("a profile");
        init(profile.path());
        assert!(change(&url("https://a.test/"), true));
        assert!(change(&url("https://b.test/"), true));

        // Another thread has its own state, like a new run.
        let path = profile.path().to_owned();
        std::thread::spawn(move || {
            init(&path);
            assert!(blocked_now("https://a.test/x"));
            assert!(blocked_now("https://b.test/y"));
            assert!(!blocked_now("https://c.test/"));
        })
        .join()
        .expect("the second run");
    }

    #[test]
    fn a_file_from_a_newer_version_is_never_saved_over() {
        let profile = tempfile::tempdir().expect("a profile");
        let path = profile.path().join("site-images.json");
        let original = br#"{"version": 2, "blocked_hosts": [{"host": "a.test"}]}"#;
        std::fs::write(&path, original).expect("a file");
        init(profile.path());

        assert!(!blocked_now("https://a.test/"), "none are read from it");
        assert!(
            change(&url("https://b.test/"), true),
            "the session still works"
        );
        assert!(blocked_now("https://b.test/"));

        assert_eq!(std::fs::read(&path).expect("untouched"), original);
    }

    #[test]
    fn a_damaged_file_starts_empty_and_is_kept_aside() {
        let profile = tempfile::tempdir().expect("a profile");
        std::fs::write(profile.path().join("site-images.json"), b"{ nonsense").expect("a file");

        init(profile.path());

        assert!(!blocked_now("https://a.test/"));
        let kept = std::fs::read(profile.path().join("site-images.json.damaged-1"));
        assert_eq!(kept.expect("kept aside"), b"{ nonsense");
        assert!(change(&url("https://a.test/"), true));
        assert!(profile.path().join("site-images.json").is_file());
    }

    #[test]
    fn without_a_profile_the_set_is_kept_for_the_run_only() {
        // `init` was never called on this thread.
        assert!(change(&url("https://a.test/"), true));
        assert!(blocked_now("https://a.test/"));
        assert!(!change(&url("about:blank"), true));
    }

    #[test]
    fn the_thousand_and_first_site_changes_nothing() {
        for number in 0..talaria_core::MAX_IMAGE_BLOCKED_SITES {
            assert!(change(&url(&format!("https://site{number}.test/")), true));
        }
        assert!(!change(&url("https://one-too-many.test/"), true));
        assert!(!blocked_now("https://one-too-many.test/"));
        assert!(change(&url("https://site0.test/"), false));
        assert!(change(&url("https://one-too-many.test/"), true));
    }
}
