// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! View source (sh-pages): ⌥⌘U (Ctrl+U elsewhere), View ▸ "View Source" and the page context
//! menu's "View page source" open the source of the active tab's page in a new tab.
//!
//! The new tab is opened right after the active tab and selected, as Chrome does, at
//! `view-source:<page address>` (`talaria_core::view_source_address`). The `view-source:` page
//! itself is made by the engine's protocol handler (`talaria-servo`'s `view_source.rs`), which
//! fetches the page again; this module only decides *whether* a tab has a source to show and
//! opens the tab, the way "Duplicate Tab" does (`session::duplicate_tab`).
//!
//! - **Which pages.** An `http(s)` page that is not one of Mercury's own pages: not an error
//!   page, not the HTTPS-only interstitial, not a failed load, not a `view-source:` page
//!   (`source_address`). Anything else is disabled in the menus and does nothing from the key.
//! - **History.** The tab is opened as a restored tab, whose page is not a new visit, and
//!   `visits.rs` records only `http`, `https` and `file` pages in any case, so a source tab
//!   never reaches the history (`a_source_tab_is_never_recorded_in_the_history`). Session
//!   restore drops the scheme too.
//! - **Keys.** `shortcut` is the one rule for the key; `shortcuts.rs` calls it.

use std::cell::RefCell;

use mercury_ui::MainWindow;
use talaria_core::{BrowserController, LoadState, RestoredTab, TabId, view_source_address};

use crate::shortcuts::{Platform, ShortcutAction};
use crate::ui_sync::sync_ui;

/// ⌥⌘U on macOS (as Safari and Chrome), Ctrl+U elsewhere (as Chrome and Firefox).
///
/// `character` is the lower-case key, `primary` is Cmd on macOS and Ctrl elsewhere (the same
/// arguments as `shortcuts::bookmark_shortcut`). Alt is not allowed with Ctrl off macOS: on
/// Windows and many Linux layouts Ctrl+Alt is AltGr, which types characters.
pub(crate) fn shortcut(
    character: Option<&str>,
    primary: bool,
    shift: bool,
    alt: bool,
    platform: Platform,
) -> Option<ShortcutAction> {
    if !primary || shift || character != Some("u") {
        return None;
    }
    match (platform, alt) {
        (Platform::MacOs, true) | (Platform::Other, false) => Some(ShortcutAction::ViewSource),
        _ => None,
    }
}

/// What a tab shows besides its address, as far as view source is concerned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PageKind {
    /// One of Mercury's own error pages (no connection, certificate).
    error_page: bool,
    /// The HTTPS-only interstitial, shown while the tab's address is still the `http` one.
    interstitial: bool,
    /// The page crashed or failed to load.
    load_failed: bool,
}

/// The `view-source:` address for a tab showing `url` of this kind, if the page has a source:
/// an `http(s)` page, credentials and fragment dropped, that is not one of Mercury's own pages.
fn source_address(url: &str, kind: PageKind) -> Option<String> {
    if kind.error_page || kind.interstitial || kind.load_failed {
        return None;
    }
    view_source_address(url)
}

/// The `view-source:` address of the active tab's page, if it has a source to show.
fn source_address_of_active_tab(controller: &BrowserController) -> Option<String> {
    let tab_id = controller.active_tab_id();
    let state = controller.active_state();
    source_address(
        state.url(),
        PageKind {
            error_page: controller.error_page(tab_id).is_some(),
            interstitial: controller.https_only_interstitial(tab_id).is_some(),
            load_failed: matches!(state.load_state(), LoadState::Failed { .. }),
        },
    )
}

/// Whether the active tab shows a page whose source can be viewed (the View menu's row, the
/// page context menu's row, and the key).
pub(crate) fn can_view_source(controller: &BrowserController) -> bool {
    source_address_of_active_tab(controller).is_some()
}

/// Opens the source of the active tab's page in a new tab right after it, selected. `None`, and
/// no tab, when the page has no source (`can_view_source`) or the address does not resolve.
pub(crate) fn open_source_tab(controller: &mut BrowserController) -> Option<TabId> {
    let address = source_address_of_active_tab(controller)?;
    let active_tab_id = controller.active_tab_id();
    let index = controller
        .tabs()
        .position(|tab| tab.id() == active_tab_id)?
        + 1;
    let tab = RestoredTab {
        url: Some(address),
        // The page names itself ("Source of host/path") once it has loaded.
        title: String::new(),
        zoom_percent: controller.default_zoom_percent(),
        pinned: false,
    };
    match controller.open_restored_tab(index, &tab) {
        Ok(tab_id) => {
            controller.select_tab(tab_id);
            Some(tab_id)
        }
        Err(error) => {
            log::warn!("could not open the source of the page: {error}");
            None
        }
    }
}

/// "View Source": the active tab's page source in a new tab next to it. Does nothing for a page
/// that has none.
pub(crate) fn open_source_of_active_tab(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
) {
    let opened = open_source_tab(&mut controller.borrow_mut());
    if opened.is_some() {
        sync_ui(window, &controller.borrow());
    }
}

#[cfg(test)]
mod tests {
    use talaria_core::{BrowserEvent, ErrorPageKind, TabActivation, VisitUpdate};

    use super::*;

    const MAC: Platform = Platform::MacOs;
    const OTHER: Platform = Platform::Other;

    fn key(
        platform: Platform,
        character: &str,
        primary: bool,
        shift: bool,
        alt: bool,
    ) -> Option<ShortcutAction> {
        shortcut(Some(character), primary, shift, alt, platform)
    }

    #[test]
    fn the_key_is_option_command_u_on_macos_and_control_u_elsewhere() {
        // primary, shift, alt
        assert_eq!(
            key(MAC, "u", true, false, true),
            Some(ShortcutAction::ViewSource)
        );
        assert_eq!(
            key(OTHER, "u", true, false, false),
            Some(ShortcutAction::ViewSource)
        );
    }

    #[test]
    fn no_other_combination_is_view_source() {
        // Cmd+U alone is "underline" in editors and belongs to the page on macOS.
        assert_eq!(key(MAC, "u", true, false, false), None);
        assert_eq!(key(MAC, "u", true, true, true), None);
        assert_eq!(
            key(MAC, "u", false, false, true),
            None,
            "Option+U types a mark"
        );
        assert_eq!(key(MAC, "u", false, false, false), None);
        assert_eq!(key(MAC, "i", true, false, true), None);
        // Ctrl+Alt is AltGr on many layouts; Ctrl+Shift+U enters a Unicode character on Linux.
        assert_eq!(key(OTHER, "u", true, false, true), None);
        assert_eq!(key(OTHER, "u", true, true, false), None);
        assert_eq!(key(OTHER, "u", false, false, false), None);
        assert_eq!(key(OTHER, "i", true, false, false), None);
        assert_eq!(shortcut(None, true, false, true, MAC), None);
    }

    #[test]
    fn view_source_does_not_repeat() {
        assert!(!ShortcutAction::ViewSource.repeats());
    }

    #[test]
    fn only_a_web_page_has_a_source_address() {
        let none = PageKind::default();
        assert_eq!(
            source_address("https://example.com/a?b=1", none).as_deref(),
            Some("view-source:https://example.com/a?b=1")
        );
        assert_eq!(
            source_address("http://example.com:8080/", none).as_deref(),
            Some("view-source:http://example.com:8080/")
        );
        // Credentials and the fragment are not part of what is fetched.
        assert_eq!(
            source_address("https://user:secret@example.com/a#top", none).as_deref(),
            Some("view-source:https://example.com/a")
        );
        for url in [
            "",
            "about:blank",
            "about:newtab",
            "file:///Users/me/index.html",
            "data:text/html,hi",
            "blob:https://example.com/0b1c",
            "view-source:https://example.com/",
            "mailto:me@example.com",
            "not an address",
        ] {
            assert_eq!(source_address(url, none), None, "{url}");
        }
    }

    #[test]
    fn mercurys_own_pages_have_no_source() {
        let url = "https://example.com/";
        for kind in [
            PageKind {
                error_page: true,
                ..PageKind::default()
            },
            PageKind {
                interstitial: true,
                ..PageKind::default()
            },
            PageKind {
                load_failed: true,
                ..PageKind::default()
            },
        ] {
            assert_eq!(source_address(url, kind), None, "{kind:?}");
        }
    }

    /// A controller with a blank first tab, then `urls` in order, each its own tab; the first of
    /// them is selected. Returns the controller and the ids of the tabs for `urls`.
    fn controller_with(urls: &[&str]) -> (BrowserController, Vec<TabId>) {
        let mut controller = BrowserController::default();
        let ids = urls
            .iter()
            .map(|url| {
                controller
                    .open_tab_with(Some(url), TabActivation::Background)
                    .expect("the address resolves")
            })
            .collect::<Vec<_>>();
        controller.select_tab(ids[0]);
        (controller, ids)
    }

    fn addresses(controller: &BrowserController) -> Vec<String> {
        controller
            .tabs()
            .map(|tab| tab.state().url().to_owned())
            .collect()
    }

    #[test]
    fn the_active_tabs_page_decides_whether_view_source_is_available() {
        let (mut controller, ids) =
            controller_with(&["https://a.test/page#x", "https://b.test/", "about:blank"]);
        assert!(can_view_source(&controller));

        controller.select_tab(ids[2]);
        assert!(!can_view_source(&controller), "about:blank");
        controller.select_tab(ids[1]);
        assert!(can_view_source(&controller));

        // The first tab of a new controller is the blank start tab.
        let blank = controller.tabs().next().expect("the start tab").id();
        controller.select_tab(blank);
        assert!(!can_view_source(&controller), "the start tab");
    }

    #[test]
    fn the_source_opens_in_a_new_tab_right_after_the_active_one_and_is_selected() {
        let (mut controller, ids) =
            controller_with(&["https://a.test/page?q=1#frag", "https://b.test/"]);
        let before = controller.tabs().len();
        controller.set_default_zoom_percent(125);

        let source = open_source_tab(&mut controller).expect("a page with a source");

        assert_eq!(controller.tabs().len(), before + 1);
        assert_eq!(controller.active_tab_id(), source, "selected");
        let order = controller.tabs().map(|tab| tab.id()).collect::<Vec<_>>();
        let position = order
            .iter()
            .position(|id| *id == ids[0])
            .expect("the page's tab is still there");
        assert_eq!(order[position + 1], source, "right after the page's tab");
        assert_eq!(order[position + 2], ids[1], "the next tab moved along");
        assert_eq!(
            addresses(&controller)[position + 1],
            "view-source:https://a.test/page?q=1"
        );
        let tab = controller
            .tabs()
            .find(|tab| tab.id() == source)
            .expect("the source tab");
        assert_eq!(tab.state().zoom_percent(), 125, "the default zoom");
        assert!(!tab.is_pinned());
    }

    #[test]
    fn a_pinned_page_gets_an_unpinned_source_tab() {
        let (mut controller, ids) = controller_with(&["https://a.test/", "https://b.test/"]);
        assert!(controller.set_tab_pinned(ids[0], true));

        let source = open_source_tab(&mut controller).expect("a page with a source");

        let tab = controller
            .tabs()
            .find(|tab| tab.id() == source)
            .expect("the source tab");
        assert!(!tab.is_pinned());
        assert_eq!(controller.active_tab_id(), source);
    }

    #[test]
    fn a_source_page_has_no_source_to_view() {
        let (mut controller, _) = controller_with(&["https://a.test/"]);
        let source = open_source_tab(&mut controller).expect("a page with a source");
        assert_eq!(controller.active_tab_id(), source);
        let before = addresses(&controller);

        assert!(!can_view_source(&controller));
        assert_eq!(open_source_tab(&mut controller), None);
        assert_eq!(addresses(&controller), before, "no tab was opened");
        assert_eq!(controller.active_tab_id(), source);
    }

    #[test]
    fn an_error_page_an_interstitial_and_a_failed_load_have_no_source() {
        let (mut controller, ids) = controller_with(&["https://a.test/", "http://b.test/"]);
        let tab = ids[0];
        assert!(can_view_source(&controller));
        let count = controller.tabs().len();

        controller.handle_event(
            tab,
            BrowserEvent::ErrorPageShown {
                kind: ErrorPageKind::Network,
                url: "https://a.test/".to_owned(),
            },
        );
        assert!(!can_view_source(&controller), "an error page");
        assert_eq!(open_source_tab(&mut controller), None);
        assert_eq!(controller.tabs().len(), count);

        // A new load starts: the error page is gone with it.
        controller.handle_event(tab, BrowserEvent::PageLoadStarted);
        assert!(can_view_source(&controller));

        controller.handle_event(
            tab,
            BrowserEvent::LoadStateChanged {
                state: LoadState::Failed {
                    message: "the page crashed".to_owned(),
                },
            },
        );
        assert!(!can_view_source(&controller), "a failed load");
        assert_eq!(open_source_tab(&mut controller), None);

        // The interstitial is shown over the second tab's `http` address.
        controller.select_tab(ids[1]);
        assert!(can_view_source(&controller));
        controller.handle_event(
            ids[1],
            BrowserEvent::HttpsOnlyBlocked {
                url: "http://b.test/".to_owned(),
            },
        );
        assert!(!can_view_source(&controller), "the HTTPS-only interstitial");
        assert_eq!(open_source_tab(&mut controller), None);
        assert_eq!(controller.tabs().len(), count);
    }

    #[test]
    fn a_source_tab_is_never_recorded_in_the_history() {
        let (mut controller, _) = controller_with(&["https://a.test/page"]);
        // The page's own visit is not what is asked about.
        drop(controller.take_visit_updates());

        let source = open_source_tab(&mut controller).expect("a page with a source");
        let address = "view-source:https://a.test/page";
        for event in [
            BrowserEvent::PageCommitted {
                url: address.to_owned(),
                history_length: 1,
                history_index: 0,
            },
            BrowserEvent::PageTitleChanged {
                title: Some("Source of a.test/page".to_owned()),
            },
            BrowserEvent::PageTitleAt {
                url: address.to_owned(),
                title: Some("Source of a.test/page".to_owned()),
            },
            // A click on a link inside the source page: another `view-source:` page.
            BrowserEvent::PageCommitted {
                url: "view-source:https://a.test/other".to_owned(),
                history_length: 2,
                history_index: 1,
            },
        ] {
            controller.handle_event(source, event);
        }

        let updates = controller.take_visit_updates();
        for update in &updates {
            let url = match update {
                VisitUpdate::Visit { url, .. } | VisitUpdate::Title { url, .. } => url,
                VisitUpdate::Forget { .. } => continue,
            };
            assert!(!url.starts_with("view-source:"), "{updates:?}");
        }
        assert!(
            updates
                .iter()
                .all(|update| !matches!(update, VisitUpdate::Visit { .. })),
            "{updates:?}"
        );
    }
}
