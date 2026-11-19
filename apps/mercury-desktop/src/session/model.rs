// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The pure part of session saving and restoring: what is saved from the controller, when the
//! session counts as changed, which closed tab comes back, and how a saved session is put back.

use mercury_settings::{ClosedTab, SessionSnapshot, TabSnapshot, WindowBounds, WindowSnapshot};
use talaria_core::{BrowserController, RestoredTab, Tab, TabId};

/// The address saved for a tab showing the new-tab page. The engine shows that page at
/// `about:blank` and the address bar shows nothing; neither survives a session file as such.
pub(crate) const NEW_TAB_PAGE_URL: &str = "about:newtab";

/// Saved addresses that mean "a new-tab page" (the same list `mercury-settings` uses to decide
/// that a session is not worth recovering).
const NEW_TAB_PAGE_URLS: [&str; 3] = ["about:blank", "about:newtab", "about:home"];

/// The only window there is, for `ClosedTab::window`.
const WINDOW_INDEX: usize = 0;

/// Whether a tab belongs in the session file. Every tab does today; a private tab, when there
/// is one, returns false here and is then neither saved nor offered by "Reopen closed tab".
pub(crate) fn is_saved(_tab: &Tab) -> bool {
    true
}

/// What is saved of one tab.
pub(crate) fn tab_snapshot(tab: &Tab) -> TabSnapshot {
    let state = tab.state();
    TabSnapshot {
        url: if state.url().is_empty() {
            NEW_TAB_PAGE_URL.to_owned()
        } else {
            state.url().to_owned()
        },
        title: state.page_title().to_owned(),
        zoom_percent: state.zoom_percent(),
        pinned: tab.is_pinned(),
    }
}

/// Whether a saved address is a new-tab page.
pub(crate) fn is_new_tab_page(url: &str) -> bool {
    NEW_TAB_PAGE_URLS.contains(&url)
}

/// Whether a session holds nothing but new-tab pages (or nothing at all).
pub(crate) fn only_new_tab_pages(snapshot: &SessionSnapshot) -> bool {
    snapshot
        .windows
        .iter()
        .flat_map(|window| &window.tabs)
        .all(|tab| is_new_tab_page(&tab.url))
}

/// The tab to open for a saved one: a new-tab page opens blank.
///
/// Muting a tab is a choice for this run only, so it is not saved: `TabSnapshot` has no audio
/// field, nor has `RestoredTab`. A restored or reopened tab, a pinned one too, opens unmuted.
pub(crate) fn restored_tab(tab: &TabSnapshot) -> RestoredTab {
    RestoredTab {
        url: (!is_new_tab_page(&tab.url)).then(|| tab.url.clone()),
        title: tab.title.clone(),
        zoom_percent: tab.zoom_percent,
        pinned: tab.pinned,
    }
}

/// The window's place and state, as last seen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WindowGeometry {
    /// The last position and size of the window while it was neither maximised, full screen
    /// nor minimised: what it returns to. The position is the origin `saved_origin` names (the
    /// content's on macOS); the size is the content's.
    pub bounds: Option<WindowBounds>,
    pub maximized: bool,
    pub fullscreen: bool,
}

impl WindowGeometry {
    /// Takes in what the window reports now. The bounds are only taken while the window is in
    /// its normal state; `fullscreen` counts only full screen the user chose, not a page's.
    pub(crate) fn update(
        &mut self,
        bounds: WindowBounds,
        maximized: bool,
        user_fullscreen: bool,
        page_fullscreen: bool,
        minimized: bool,
    ) {
        if minimized {
            return;
        }
        if page_fullscreen {
            // A page's full screen ends with the page; it says nothing about the window.
            return;
        }
        self.maximized = maximized;
        self.fullscreen = user_fullscreen;
        if !maximized && !user_fullscreen {
            self.bounds = Some(bounds);
        }
    }
}

/// Which corner of the window is saved as its position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SavedOrigin {
    /// The content's top-left, below the title bar.
    Content,
    /// The window's outer top-left, decorations included.
    Outer,
}

/// The origin to save is the one that a position set before the window exists means, so that
/// a restored window opens where it was instead of moving by its decorations at every start.
/// winit 0.30.13 `WindowAttributes::with_position` (`src/window.rs:229-246`): on macOS it is
/// the content's top-left; on X11 and Windows the outer top-left; Wayland ignores it.
pub(crate) const fn saved_origin(platform: crate::shortcuts::Platform) -> SavedOrigin {
    match platform {
        crate::shortcuts::Platform::MacOs => SavedOrigin::Content,
        crate::shortcuts::Platform::Other => SavedOrigin::Outer,
    }
}

/// The bounds to take in: `position` when the platform tells it (Wayland does not), otherwise
/// the last one known (or the origin, which is never applied where positions are unknown).
pub(crate) fn observed_bounds(
    position: Option<(i32, i32)>,
    size: (u32, u32),
    previous: Option<WindowBounds>,
) -> WindowBounds {
    let (x, y) = position
        .or(previous.map(|previous| (previous.x, previous.y)))
        .unwrap_or((0, 0));
    WindowBounds {
        x,
        y,
        width: size.0,
        height: size.1,
    }
}

/// The window as it would be saved now.
pub(crate) fn window_snapshot(
    controller: &BrowserController,
    geometry: &WindowGeometry,
) -> WindowSnapshot {
    let active_tab_id = controller.active_tab_id();
    let mut tabs = Vec::with_capacity(controller.tabs().len());
    let mut active_tab = 0;
    for tab in controller.tabs() {
        if tab.id() == active_tab_id {
            // When the active tab is not saved, the next saved one is selected instead.
            active_tab = tabs.len();
        }
        if is_saved(tab) {
            tabs.push(tab_snapshot(tab));
        }
    }
    WindowSnapshot {
        active_tab: active_tab.min(tabs.len().saturating_sub(1)),
        tabs,
        bounds: geometry.bounds,
        maximized: geometry.maximized,
        fullscreen: geometry.fullscreen,
    }
}

/// Follows the browser's tabs and window and keeps what the session file needs: whether
/// anything changed since the last look, and the tabs that were closed.
#[derive(Debug, Default)]
pub(crate) struct SessionTracker {
    /// The saved tabs as last seen, in strip order.
    seen: Vec<(TabId, TabSnapshot)>,
    active_tab: Option<TabId>,
    geometry: WindowGeometry,
    /// Only `closed_tabs` is used.
    closed: SessionSnapshot,
}

impl SessionTracker {
    /// Looks at the tabs. Returns whether anything that is saved changed: a tab opened,
    /// closed, moved, navigated, retitled or zoomed, or another tab selected. Tabs that are
    /// gone are added to the closed list (new-tab pages excepted), newest first; several gone
    /// at once go in so that the lowest index comes back first.
    pub(crate) fn observe_tabs(&mut self, controller: &BrowserController, now_ms: i64) -> bool {
        let current: Vec<(TabId, TabSnapshot)> = controller
            .tabs()
            .filter(|tab| is_saved(tab))
            .map(|tab| (tab.id(), tab_snapshot(tab)))
            .collect();
        let active_tab = Some(controller.active_tab_id());
        if current == self.seen && active_tab == self.active_tab {
            return false;
        }

        // Highest index first, so that the lowest is reopened first and each reopened tab
        // lands at its old place.
        for (index, (tab_id, tab)) in self.seen.iter().enumerate().rev() {
            let still_open = current.iter().any(|(open_id, _)| open_id == tab_id);
            if !still_open && !is_new_tab_page(&tab.url) {
                self.closed.push_closed(ClosedTab {
                    tab: tab.clone(),
                    window: WINDOW_INDEX,
                    index,
                    closed_at_ms: now_ms,
                });
            }
        }
        self.seen = current;
        self.active_tab = active_tab;
        true
    }

    /// Takes in the window's place and state; returns whether what is saved changed.
    pub(crate) fn observe_geometry(&mut self, geometry: WindowGeometry) -> bool {
        let changed = geometry != self.geometry;
        self.geometry = geometry;
        changed
    }

    pub(crate) fn geometry(&self) -> WindowGeometry {
        self.geometry
    }

    /// The session to save, from the controller as it is now.
    pub(crate) fn snapshot(&self, controller: &BrowserController, now_ms: i64) -> SessionSnapshot {
        SessionSnapshot {
            windows: vec![window_snapshot(controller, &self.geometry)],
            closed_tabs: self.closed.closed_tabs.clone(),
            saved_at_ms: now_ms,
        }
    }

    pub(crate) fn has_closed_tabs(&self) -> bool {
        !self.closed.closed_tabs.is_empty()
    }

    /// The most recently closed tab, taken off the list.
    pub(crate) fn pop_closed(&mut self) -> Option<ClosedTab> {
        self.closed.pop_closed()
    }

    /// Adds the closed tabs of a restored session after this run's own.
    pub(crate) fn append_closed(&mut self, closed_tabs: &[ClosedTab]) {
        for closed in closed_tabs {
            if self.closed.closed_tabs.len() >= mercury_settings::MAX_CLOSED_TABS {
                break;
            }
            self.closed.closed_tabs.push(ClosedTab {
                window: WINDOW_INDEX,
                ..closed.clone()
            });
        }
    }
}

/// Where a reopened tab goes: back at its old index, or at the end if the strip is shorter now.
pub(crate) fn reopen_index(closed: &ClosedTab, tab_count: usize) -> usize {
    closed.index.min(tab_count)
}

/// "Duplicate tab": a copy of the active tab, to open just after it.
///
/// The copy opens unmuted, even when the active tab is muted (and pinned or not): it is a new
/// page, and the mute choice belongs to the tab the user muted, for this run (see `restored_tab`).
pub(crate) fn duplicate_of_active(controller: &BrowserController) -> (usize, RestoredTab) {
    let active_tab_id = controller.active_tab_id();
    let (index, tab) = controller
        .tabs()
        .enumerate()
        .find(|(_, tab)| tab.id() == active_tab_id)
        .expect("the controller always owns an active tab");
    let state = tab.state();
    (
        index + 1,
        RestoredTab {
            url: (!state.url().is_empty()).then(|| state.url().to_owned()),
            title: state.page_title().to_owned(),
            zoom_percent: state.zoom_percent(),
            // tabs-plus: a copy of a pinned tab opens pinned, right after it
            pinned: tab.is_pinned(),
        },
    )
}

/// How a saved session is put back into the window.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RestorePlan {
    /// Opened in this order at the end of the strip.
    pub tabs: Vec<RestoredTab>,
    /// The index in `tabs` of the tab to select, if one is to be selected.
    pub select: Option<usize>,
    /// The blank start tab that the restored tabs replace, closed once they are open.
    pub replace: Option<TabId>,
    /// The closed tabs the session remembered.
    pub closed_tabs: Vec<ClosedTab>,
}

/// Plans putting `snapshot` back. Every window's tabs go into this one window, the first
/// window's first. A single blank tab (the one a start opens) is replaced. The session's
/// active tab is selected when `activate` is set or when the blank tab is replaced; otherwise
/// the tab the user is on stays selected.
pub(crate) fn plan_restore(
    snapshot: &SessionSnapshot,
    controller: &BrowserController,
    activate: bool,
) -> RestorePlan {
    let tabs: Vec<RestoredTab> = snapshot
        .windows
        .iter()
        .flat_map(|window| &window.tabs)
        .map(restored_tab)
        .collect();
    let replace = (!tabs.is_empty())
        .then(|| single_blank_tab(controller))
        .flatten();
    let select = snapshot
        .windows
        .first()
        .map(|window| window.active_tab.min(window.tabs.len().saturating_sub(1)))
        .filter(|_| (activate || replace.is_some()) && !tabs.is_empty());
    RestorePlan {
        tabs,
        select,
        replace,
        closed_tabs: snapshot.closed_tabs.clone(),
    }
}

/// The only tab, if it is still a blank new-tab page.
fn single_blank_tab(controller: &BrowserController) -> Option<TabId> {
    let mut tabs = controller.tabs();
    let tab = tabs.next()?;
    (tabs.next().is_none() && tab.state().url().is_empty()).then(|| tab.id())
}

/// A screen's usable area, in logical pixels, with the origin at the top left of the main
/// screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScreenArea {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where to put a restored window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowPlacement {
    /// `None` when no screen is known: the system then chooses.
    pub position: Option<(i32, i32)>,
    pub size: (u32, u32),
}

/// The height of a standard macOS title bar, in points. Saved bounds are the window's
/// content (`inner_position` and the inner size), so the title bar is this far above them.
const TITLE_BAR_HEIGHT: f64 = 28.0;
/// How much of the title bar must be on a screen for the window to be reachable: this wide
/// (or the whole window, if narrower) and half its height.
const REACHABLE_WIDTH: f64 = 100.0;
const REACHABLE_HEIGHT: f64 = TITLE_BAR_HEIGHT / 2.0;

/// Puts saved content bounds back where the user can reach them. A window whose title bar is
/// on a screen keeps its place; one off every screen (a screen was unplugged) is centred on
/// the main screen (`screens[0]`), title bar included, shrunk to fit it.
pub(crate) fn place_window(bounds: WindowBounds, screens: &[ScreenArea]) -> WindowPlacement {
    let size = (bounds.width, bounds.height);
    let Some(main) = screens.first() else {
        return WindowPlacement {
            position: None,
            size,
        };
    };
    let (x, y) = (f64::from(bounds.x), f64::from(bounds.y));
    let (width, height) = (f64::from(bounds.width), f64::from(bounds.height));
    let reachable = screens.iter().any(|screen| {
        let across = overlap(x, x + width, screen.x, screen.x + screen.width);
        let down = overlap(y - TITLE_BAR_HEIGHT, y, screen.y, screen.y + screen.height);
        across >= width.min(REACHABLE_WIDTH) && down >= REACHABLE_HEIGHT
    });
    if reachable {
        return WindowPlacement {
            position: Some((bounds.x, bounds.y)),
            size,
        };
    }

    let room = (main.height - TITLE_BAR_HEIGHT).max(1.0);
    let fitted_width = width.min(main.width).max(1.0);
    let fitted_height = height.min(room).max(1.0);
    WindowPlacement {
        position: Some((
            (main.x + (main.width - fitted_width) / 2.0).round() as i32,
            (main.y + TITLE_BAR_HEIGHT + (room - fitted_height) / 2.0).round() as i32,
        )),
        size: (fitted_width as u32, fitted_height as u32),
    }
}

fn overlap(start: f64, end: f64, other_start: f64, other_end: f64) -> f64 {
    (end.min(other_end) - start.max(other_start)).max(0.0)
}

/// How a session in the recovery list is named in the chooser: its first tab and the count.
pub(crate) fn recovery_label(snapshot: &SessionSnapshot) -> String {
    let tabs = snapshot.windows.iter().flat_map(|window| &window.tabs);
    let count = tabs.clone().count();
    let first = tabs
        .clone()
        .find(|tab| !is_new_tab_page(&tab.url))
        .or_else(|| tabs.clone().next());
    let name = first.map_or("", |tab| {
        if tab.title.trim().is_empty() {
            tab.url.as_str()
        } else {
            tab.title.as_str()
        }
    });
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    match count {
        1 => format!("{name} (1 tab)"),
        count => format!("{name} ({count} tabs)"),
    }
}

#[cfg(test)]
mod tests {
    use mercury_ui::TabAudioState;
    use talaria_core::{BrowserEvent, CloseTabOutcome, TabActivation, TabAudio};

    use super::*;
    use crate::shortcuts::Platform;

    const NOW: i64 = 1_700_000_000_000;

    fn page(controller: &mut BrowserController, url: &str, title: &str) -> TabId {
        let tab_id = controller
            .open_tab_with(Some(url), TabActivation::Background)
            .unwrap();
        controller.handle_event(
            tab_id,
            BrowserEvent::PageTitleChanged {
                title: Some(title.to_owned()),
            },
        );
        tab_id
    }

    fn saved_tab(url: &str, title: &str) -> TabSnapshot {
        TabSnapshot {
            url: url.to_owned(),
            title: title.to_owned(),
            zoom_percent: 100,
            pinned: false,
        }
    }

    fn window(tabs: Vec<TabSnapshot>, active_tab: usize) -> WindowSnapshot {
        WindowSnapshot {
            tabs,
            active_tab,
            ..WindowSnapshot::default()
        }
    }

    fn session(windows: Vec<WindowSnapshot>) -> SessionSnapshot {
        SessionSnapshot {
            windows,
            ..SessionSnapshot::default()
        }
    }

    // --- snapshot building ---

    #[test]
    fn the_snapshot_holds_every_tab_in_strip_order_with_the_active_one_marked() {
        let mut controller = BrowserController::default();
        let blank = controller.active_tab_id();
        let a = page(&mut controller, "https://a.test/", "A");
        controller.select_tab(a);
        controller.zoom_in();
        page(&mut controller, "https://b.test/", "B");
        let geometry = WindowGeometry {
            bounds: Some(WindowBounds {
                x: 10,
                y: 20,
                width: 800,
                height: 600,
            }),
            maximized: true,
            fullscreen: false,
        };

        let snapshot = window_snapshot(&controller, &geometry);

        assert_eq!(
            snapshot.tabs,
            vec![
                TabSnapshot {
                    url: NEW_TAB_PAGE_URL.to_owned(),
                    title: "New Tab".to_owned(),
                    zoom_percent: 100,
                    pinned: false,
                },
                TabSnapshot {
                    zoom_percent: 110,
                    pinned: false,
                    ..saved_tab("https://a.test/", "A")
                },
                saved_tab("https://b.test/", "B"),
            ]
        );
        assert_eq!(snapshot.active_tab, 1);
        assert_eq!(snapshot.bounds, geometry.bounds);
        assert!(snapshot.maximized);
        assert!(!snapshot.fullscreen);
        assert_ne!(blank, a);
    }

    #[test]
    fn a_new_tab_page_is_saved_as_an_about_url_that_survives_sanitizing() {
        let controller = BrowserController::default();
        let snapshot = session(vec![window_snapshot(
            &controller,
            &WindowGeometry::default(),
        )])
        .sanitized();

        assert_eq!(snapshot.windows[0].tabs[0].url, NEW_TAB_PAGE_URL);
        assert!(is_new_tab_page(NEW_TAB_PAGE_URL));
    }

    // --- dirty marking ---

    #[test]
    fn the_first_look_counts_as_a_change_and_an_unchanged_look_does_not() {
        let controller = BrowserController::default();
        let mut tracker = SessionTracker::default();

        assert!(tracker.observe_tabs(&controller, NOW));
        assert!(!tracker.observe_tabs(&controller, NOW));
    }

    #[test]
    fn every_change_that_is_saved_marks_the_session_changed() {
        let mut controller = BrowserController::default();
        let mut tracker = SessionTracker::default();
        let first = controller.active_tab_id();
        tracker.observe_tabs(&controller, NOW);

        let second = controller.open_tab();
        assert!(tracker.observe_tabs(&controller, NOW), "tab opened");

        controller.select_tab(first);
        assert!(tracker.observe_tabs(&controller, NOW), "active tab changed");

        controller.queue_navigation("https://a.test/").unwrap();
        assert!(tracker.observe_tabs(&controller, NOW), "navigated");

        controller.handle_event(
            first,
            BrowserEvent::PageTitleChanged {
                title: Some("A".to_owned()),
            },
        );
        assert!(tracker.observe_tabs(&controller, NOW), "retitled");

        controller.zoom_in();
        assert!(tracker.observe_tabs(&controller, NOW), "zoomed");

        assert_eq!(controller.close_tab(second), CloseTabOutcome::Closed);
        assert!(tracker.observe_tabs(&controller, NOW), "tab closed");
    }

    #[test]
    fn what_is_not_saved_does_not_mark_the_session_changed() {
        let mut controller = BrowserController::default();
        let mut tracker = SessionTracker::default();
        let tab_id = controller.active_tab_id();
        tracker.observe_tabs(&controller, NOW);

        controller.handle_event(
            tab_id,
            BrowserEvent::StatusTextChanged {
                text: Some("hover".to_owned()),
            },
        );
        controller.handle_event(
            tab_id,
            BrowserEvent::HistoryChanged {
                can_go_back: true,
                can_go_forward: false,
            },
        );

        assert!(!tracker.observe_tabs(&controller, NOW));
    }

    #[test]
    fn moving_or_resizing_the_window_marks_the_session_changed() {
        let mut tracker = SessionTracker::default();
        let mut geometry = WindowGeometry::default();
        let bounds = WindowBounds {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };

        geometry.update(bounds, false, false, false, false);
        assert!(tracker.observe_geometry(geometry));
        assert!(!tracker.observe_geometry(geometry));

        geometry.update(WindowBounds { x: 50, ..bounds }, false, false, false, false);
        assert!(tracker.observe_geometry(geometry), "moved");

        geometry.update(
            WindowBounds {
                width: 900,
                ..bounds
            },
            false,
            false,
            false,
            false,
        );
        assert!(tracker.observe_geometry(geometry), "resized");
    }

    #[test]
    fn the_saved_origin_is_the_one_a_position_set_before_showing_means() {
        assert_eq!(saved_origin(Platform::MacOs), SavedOrigin::Content);
        assert_eq!(saved_origin(Platform::Other), SavedOrigin::Outer);
    }

    #[test]
    fn an_unknown_position_keeps_the_last_one_and_still_takes_the_size() {
        let previous = WindowBounds {
            x: 40,
            y: 60,
            width: 800,
            height: 600,
        };

        assert_eq!(
            observed_bounds(None, (900, 700), Some(previous)),
            WindowBounds {
                x: 40,
                y: 60,
                width: 900,
                height: 700,
            }
        );
        assert_eq!(
            observed_bounds(None, (900, 700), None),
            WindowBounds {
                x: 0,
                y: 0,
                width: 900,
                height: 700,
            }
        );
        assert_eq!(
            observed_bounds(Some((5, 6)), (900, 700), Some(previous)),
            WindowBounds {
                x: 5,
                y: 6,
                width: 900,
                height: 700,
            }
        );
    }

    #[test]
    fn a_maximised_or_full_screen_window_keeps_its_normal_bounds() {
        let normal = WindowBounds {
            x: 10,
            y: 10,
            width: 800,
            height: 600,
        };
        let screen = WindowBounds {
            x: 0,
            y: 0,
            width: 1512,
            height: 982,
        };
        let mut geometry = WindowGeometry::default();
        geometry.update(normal, false, false, false, false);

        geometry.update(screen, true, false, false, false);
        assert_eq!((geometry.bounds, geometry.maximized), (Some(normal), true));

        geometry.update(screen, false, true, false, false);
        assert_eq!((geometry.bounds, geometry.fullscreen), (Some(normal), true));
    }

    #[test]
    fn a_pages_full_screen_and_a_minimised_window_change_nothing() {
        let normal = WindowBounds {
            x: 10,
            y: 10,
            width: 800,
            height: 600,
        };
        let mut geometry = WindowGeometry::default();
        geometry.update(normal, false, false, false, false);
        let before = geometry;

        geometry.update(normal, false, true, true, false);
        assert_eq!(geometry, before);
        geometry.update(
            WindowBounds {
                x: -32000,
                ..normal
            },
            false,
            false,
            false,
            true,
        );
        assert_eq!(geometry, before);
    }

    // --- closed tabs ---

    #[test]
    fn closed_tabs_are_remembered_newest_first_with_their_index() {
        let mut controller = BrowserController::default();
        let mut tracker = SessionTracker::default();
        let a = page(&mut controller, "https://a.test/", "A");
        let b = page(&mut controller, "https://b.test/", "B");
        tracker.observe_tabs(&controller, NOW);

        assert_eq!(controller.close_tab(a), CloseTabOutcome::Closed);
        tracker.observe_tabs(&controller, NOW + 1);
        assert_eq!(controller.close_tab(b), CloseTabOutcome::Closed);
        tracker.observe_tabs(&controller, NOW + 2);

        let first = tracker.pop_closed().unwrap();
        assert_eq!(
            (first.tab.url.as_str(), first.index, first.closed_at_ms),
            ("https://b.test/", 1, NOW + 2)
        );
        let second = tracker.pop_closed().unwrap();
        assert_eq!(
            (second.tab.url.as_str(), second.index, second.closed_at_ms),
            ("https://a.test/", 1, NOW + 1)
        );
        assert_eq!(tracker.pop_closed(), None);
    }

    #[test]
    fn several_tabs_closed_at_once_reopen_at_their_old_places() {
        let mut controller = BrowserController::default();
        let mut tracker = SessionTracker::default();
        let a = page(&mut controller, "https://a.test/", "A");
        let b = page(&mut controller, "https://b.test/", "B");
        let c = page(&mut controller, "https://c.test/", "C");
        tracker.observe_tabs(&controller, NOW);

        for tab_id in [b, a, c] {
            assert_eq!(controller.close_tab(tab_id), CloseTabOutcome::Closed);
        }
        tracker.observe_tabs(&controller, NOW);
        while let Some(closed) = tracker.pop_closed() {
            let index = reopen_index(&closed, controller.tabs().len());
            controller
                .open_restored_tab(index, &restored_tab(&closed.tab))
                .unwrap();
        }

        let urls = controller
            .tabs()
            .map(|tab| tab.state().url().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec!["", "https://a.test/", "https://b.test/", "https://c.test/"]
        );
    }

    #[test]
    fn a_session_of_new_tab_pages_only_is_recognised() {
        let blank = session(vec![window(
            vec![
                saved_tab(NEW_TAB_PAGE_URL, "New Tab"),
                saved_tab("about:blank", ""),
            ],
            0,
        )]);
        let real = session(vec![window(
            vec![
                saved_tab(NEW_TAB_PAGE_URL, "New Tab"),
                saved_tab("https://a.test/", "A"),
            ],
            0,
        )]);

        assert!(only_new_tab_pages(&blank));
        assert!(only_new_tab_pages(&SessionSnapshot::default()));
        assert!(!only_new_tab_pages(&real));
    }

    #[test]
    fn closing_a_new_tab_page_is_not_remembered() {
        let mut controller = BrowserController::default();
        let mut tracker = SessionTracker::default();
        let blank = controller.active_tab_id();
        page(&mut controller, "https://a.test/", "A");
        tracker.observe_tabs(&controller, NOW);

        assert_eq!(controller.close_tab(blank), CloseTabOutcome::Closed);
        tracker.observe_tabs(&controller, NOW);

        assert!(!tracker.has_closed_tabs());
    }

    #[test]
    fn closed_tabs_are_saved_with_the_session() {
        let mut controller = BrowserController::default();
        let mut tracker = SessionTracker::default();
        let a = page(&mut controller, "https://a.test/", "A");
        tracker.observe_tabs(&controller, NOW);
        assert_eq!(controller.close_tab(a), CloseTabOutcome::Closed);
        tracker.observe_tabs(&controller, NOW);

        let snapshot = tracker.snapshot(&controller, NOW);

        assert_eq!(snapshot.closed_tabs.len(), 1);
        assert_eq!(snapshot.closed_tabs[0].tab.url, "https://a.test/");
        assert_eq!(snapshot.saved_at_ms, NOW);
    }

    #[test]
    fn a_reopened_tab_goes_back_to_its_index_clamped_to_the_strip() {
        let closed = ClosedTab {
            index: 4,
            ..ClosedTab::default()
        };

        assert_eq!(reopen_index(&closed, 9), 4);
        assert_eq!(reopen_index(&closed, 2), 2);
    }

    #[test]
    fn a_restored_sessions_closed_tabs_follow_this_runs_own() {
        let mut controller = BrowserController::default();
        let mut tracker = SessionTracker::default();
        let a = page(&mut controller, "https://a.test/", "A");
        tracker.observe_tabs(&controller, NOW);
        assert_eq!(controller.close_tab(a), CloseTabOutcome::Closed);
        tracker.observe_tabs(&controller, NOW);

        tracker.append_closed(&[ClosedTab {
            tab: saved_tab("https://old.test/", "Old"),
            window: 3,
            index: 0,
            closed_at_ms: 1,
        }]);

        assert_eq!(tracker.pop_closed().unwrap().tab.url, "https://a.test/");
        let old = tracker.pop_closed().unwrap();
        assert_eq!((old.tab.url.as_str(), old.window), ("https://old.test/", 0));
    }

    #[test]
    fn a_duplicate_opens_just_after_the_active_tab_at_the_same_address() {
        let mut controller = BrowserController::default();
        page(&mut controller, "https://a.test/", "A");
        let b = page(&mut controller, "https://b.test/", "B");
        controller.select_tab(b);
        controller.zoom_in();
        page(&mut controller, "https://c.test/", "C");

        assert_eq!(
            duplicate_of_active(&controller),
            (
                3,
                RestoredTab {
                    url: Some("https://b.test/".to_owned()),
                    title: "B".to_owned(),
                    zoom_percent: 110,
                    pinned: false,
                }
            )
        );
    }

    // tabs-plus
    #[test]
    fn pinned_tabs_are_saved_restored_and_duplicated_pinned() {
        let mut controller = BrowserController::default();
        let a = page(&mut controller, "https://a.test/", "A");
        page(&mut controller, "https://b.test/", "B");
        assert!(controller.set_tab_pinned(a, true));
        controller.select_tab(a);
        let saved = controller
            .tabs()
            .map(tab_snapshot)
            .map(|tab| tab.pinned)
            .collect::<Vec<_>>();
        assert_eq!(saved, [true, false, false]);

        let tab = TabSnapshot {
            pinned: true,
            ..saved_tab("https://a.test/", "A")
        };
        assert!(restored_tab(&tab).pinned);

        // The copy of a pinned tab opens right after it, pinned.
        let (index, copy) = duplicate_of_active(&controller);
        assert_eq!(index, 1);
        assert!(copy.pinned);
    }

    // media-session lane x tabs-plus: muting is a choice for this run, not part of the session.

    fn tab_of(controller: &BrowserController, tab_id: TabId) -> &Tab {
        controller
            .tabs()
            .find(|tab| tab.id() == tab_id)
            .expect("the tab is open")
    }

    fn is_silent(controller: &BrowserController, tab_id: TabId) -> bool {
        let state = tab_of(controller, tab_id).state();
        !state.is_audio_muted() && state.audio_indicator() == TabAudio::Silent
    }

    #[test]
    fn a_muted_tab_is_saved_exactly_as_the_same_tab_unmuted() {
        let mut controller = BrowserController::default();
        let mut tracker = SessionTracker::default();
        let a = page(&mut controller, "https://a.test/", "A");
        tracker.observe_tabs(&controller, NOW);

        for pinned in [false, true] {
            if pinned {
                assert!(controller.set_tab_pinned(a, true));
            }
            let unmuted = tab_snapshot(tab_of(&controller, a));
            assert!(controller.toggle_audio_muted(a));
            let muted = tab_snapshot(tab_of(&controller, a));
            assert!(controller.toggle_audio_muted(a));

            // Every field is named, so a field for the mute cannot be added without a decision
            // here: there is none today.
            assert_eq!(
                muted,
                TabSnapshot {
                    url: "https://a.test/".to_owned(),
                    title: "A".to_owned(),
                    zoom_percent: 100,
                    pinned,
                }
            );
            assert_eq!(muted, unmuted);
        }

        // Muting changes nothing that is saved: the session does not look changed, so it does
        // not cause a write either.
        tracker.observe_tabs(&controller, NOW);
        let geometry = WindowGeometry::default();
        let unmuted_window = window_snapshot(&controller, &geometry);
        assert!(controller.toggle_audio_muted(a));
        assert!(!tracker.observe_tabs(&controller, NOW), "muted");
        assert_eq!(
            window_snapshot(&controller, &geometry).tabs,
            unmuted_window.tabs
        );
        assert!(controller.toggle_audio_muted(a));
        assert!(!tracker.observe_tabs(&controller, NOW), "unmuted");
    }

    #[test]
    fn restoring_reopening_and_duplicating_open_unmuted_tabs_a_pinned_muted_one_too() {
        let mut controller = BrowserController::default();
        let mut tracker = SessionTracker::default();
        let a = page(&mut controller, "https://a.test/", "A");
        assert!(controller.set_tab_pinned(a, true));
        assert!(controller.select_tab(a));
        assert!(controller.toggle_audio_muted(a));
        tracker.observe_tabs(&controller, NOW);
        let saved = tracker.snapshot(&controller, NOW);
        assert!(saved.windows[0].tabs[0].pinned);

        // Duplicate Tab: the copy of the pinned muted tab opens right after it, pinned, unmuted.
        let (index, duplicate) = duplicate_of_active(&controller);
        let copy = controller
            .open_restored_tab(index, &duplicate)
            .expect("the address resolves");
        assert!(tab_of(&controller, copy).is_pinned());
        assert!(is_silent(&controller, copy), "the copy is unmuted");
        assert!(
            tab_of(&controller, a).state().is_audio_muted(),
            "the tab it was copied from stays muted"
        );

        // Reopen Closed Tab: the muted pinned tab is closed and comes back pinned, unmuted.
        assert_eq!(controller.close_tab(a), CloseTabOutcome::Closed);
        tracker.observe_tabs(&controller, NOW);
        let closed = tracker.pop_closed().expect("the closed tab is remembered");
        assert!(closed.tab.pinned);
        let index = reopen_index(&closed, controller.tabs().len());
        let reopened = controller
            .open_restored_tab(index, &restored_tab(&closed.tab))
            .expect("the address resolves");
        assert!(tab_of(&controller, reopened).is_pinned());
        assert!(
            is_silent(&controller, reopened),
            "the reopened tab is unmuted"
        );

        // Restoring a session: every tab the plan opens, in a run of its own, is unmuted.
        let mut next_run = BrowserController::default();
        let plan = plan_restore(&saved, &next_run, true);
        assert!(plan.tabs.iter().any(|tab| tab.pinned));
        for tab in &plan.tabs {
            let index = next_run.tabs().len();
            next_run
                .open_restored_tab(index, tab)
                .expect("the address resolves");
        }
        assert!(next_run.tabs().any(|tab| tab.is_pinned()));
        assert!(
            next_run
                .tabs()
                .all(|tab| tab.state().audio_indicator() != TabAudio::Muted),
            "a restored tab is unmuted"
        );
        assert!(
            crate::tab_strip::strip_rows(&next_run)
                .iter()
                .all(|row| row.audio != TabAudioState::Muted)
        );
    }

    #[test]
    fn a_muted_tab_stays_muted_when_pinned_unpinned_or_moved() {
        let mut controller = BrowserController::default();
        let a = page(&mut controller, "https://a.test/", "A");
        let b = page(&mut controller, "https://b.test/", "B");
        page(&mut controller, "https://c.test/", "C");
        assert!(controller.toggle_audio_muted(b));
        // The rows the strip shows: the muted tab says so, wherever it is, and no other does.
        let check = |controller: &BrowserController, step: &str| {
            let rows = crate::tab_strip::strip_rows(controller);
            assert_eq!(rows.len(), 4, "{step}");
            for row in rows {
                let want = if row.id == crate::tab_strip::id_for_ui(b) {
                    TabAudioState::Muted
                } else {
                    TabAudioState::Silent
                };
                assert_eq!(row.audio, want, "{step}: tab {}", row.id);
            }
        };
        check(&controller, "muted");

        assert!(controller.set_tab_pinned(b, true));
        check(&controller, "pinned");
        assert!(controller.set_tab_pinned(a, true));
        check(&controller, "another tab pinned after it");
        assert!(controller.move_tab_by(b, 1));
        check(&controller, "moved within the pinned tabs");
        assert!(controller.move_tab(b, 3, false));
        check(&controller, "dragged out of the pinned tabs");
        assert!(controller.move_tab(b, 0, true));
        check(&controller, "dragged back in");
        assert!(controller.set_tab_pinned(b, false));
        check(&controller, "unpinned");
    }

    // --- the restore plan ---

    #[test]
    fn restoring_replaces_a_single_blank_start_tab_and_selects_the_sessions_active_tab() {
        let controller = BrowserController::default();
        let blank = controller.active_tab_id();
        let snapshot = session(vec![window(
            vec![
                saved_tab("https://a.test/", "A"),
                saved_tab("https://b.test/", "B"),
                saved_tab("https://c.test/", "C"),
            ],
            1,
        )]);

        let plan = plan_restore(&snapshot, &controller, false);

        assert_eq!(plan.replace, Some(blank));
        assert_eq!(plan.select, Some(1));
        assert_eq!(
            plan.tabs
                .iter()
                .map(|tab| tab.url.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("https://a.test/"),
                Some("https://b.test/"),
                Some("https://c.test/"),
            ]
        );
    }

    #[test]
    fn at_start_up_a_tab_from_the_command_line_stays_selected_and_is_kept() {
        let mut controller = BrowserController::default();
        let first = controller.active_tab_id();
        controller
            .set_initial_address(first, "https://typed.test/")
            .unwrap();
        let snapshot = session(vec![window(vec![saved_tab("https://a.test/", "A")], 0)]);

        let plan = plan_restore(&snapshot, &controller, false);

        assert_eq!(plan.replace, None);
        assert_eq!(plan.select, None);
    }

    #[test]
    fn restoring_on_request_beside_open_pages_keeps_them_and_selects_the_sessions_tab() {
        let mut controller = BrowserController::default();
        page(&mut controller, "https://open.test/", "Open");
        let snapshot = session(vec![window(
            vec![
                saved_tab("https://a.test/", "A"),
                saved_tab("https://b.test/", "B"),
            ],
            1,
        )]);

        let plan = plan_restore(&snapshot, &controller, true);

        assert_eq!(plan.replace, None);
        assert_eq!(plan.select, Some(1));
    }

    #[test]
    fn every_windows_tabs_are_restored_into_the_one_window_first_window_first() {
        let controller = BrowserController::default();
        let snapshot = session(vec![
            window(vec![saved_tab("https://a.test/", "A")], 0),
            window(
                vec![
                    saved_tab("https://b.test/", "B"),
                    saved_tab(NEW_TAB_PAGE_URL, "New Tab"),
                ],
                1,
            ),
        ]);

        let plan = plan_restore(&snapshot, &controller, true);

        assert_eq!(
            plan.tabs
                .iter()
                .map(|tab| tab.url.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("https://a.test/"), Some("https://b.test/"), None]
        );
        assert_eq!(plan.select, Some(0));
    }

    #[test]
    fn an_empty_session_replaces_nothing() {
        let controller = BrowserController::default();

        let plan = plan_restore(&SessionSnapshot::default(), &controller, true);

        assert_eq!(
            plan,
            RestorePlan {
                tabs: Vec::new(),
                select: None,
                replace: None,
                closed_tabs: Vec::new(),
            }
        );
    }

    #[test]
    fn restored_tabs_keep_their_title_and_zoom_and_new_tab_pages_open_blank() {
        let tab = TabSnapshot {
            zoom_percent: 150,
            ..saved_tab("https://a.test/", "A")
        };
        assert_eq!(
            restored_tab(&tab),
            RestoredTab {
                url: Some("https://a.test/".to_owned()),
                title: "A".to_owned(),
                zoom_percent: 150,
                pinned: false,
            }
        );
        for url in ["about:blank", "about:newtab", "about:home"] {
            assert_eq!(restored_tab(&saved_tab(url, "")).url, None, "{url}");
        }
    }

    // --- window placement ---

    const MAIN: ScreenArea = ScreenArea {
        x: 0.0,
        y: 25.0,
        width: 1512.0,
        height: 920.0,
    };
    const RIGHT: ScreenArea = ScreenArea {
        x: 1512.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };

    #[test]
    fn a_window_on_a_screen_keeps_its_place() {
        let bounds = WindowBounds {
            x: 1600,
            y: 100,
            width: 1000,
            height: 700,
        };

        assert_eq!(
            place_window(bounds, &[MAIN, RIGHT]),
            WindowPlacement {
                position: Some((1600, 100)),
                size: (1000, 700),
            }
        );
    }

    #[test]
    fn a_window_whose_screen_is_gone_is_centred_on_the_main_screen() {
        let bounds = WindowBounds {
            x: 1600,
            y: 100,
            width: 1000,
            height: 700,
        };

        assert_eq!(
            place_window(bounds, &[MAIN]),
            WindowPlacement {
                position: Some((256, 149)),
                size: (1000, 700),
            }
        );
    }

    #[test]
    fn a_window_too_big_for_the_main_screen_is_shrunk_to_fit_when_moved() {
        let bounds = WindowBounds {
            x: 5000,
            y: 5000,
            width: 3000,
            height: 2000,
        };

        assert_eq!(
            place_window(bounds, &[MAIN]),
            WindowPlacement {
                position: Some((0, 53)),
                size: (1512, 892),
            }
        );
    }

    #[test]
    fn a_window_with_only_its_bottom_on_a_screen_is_moved() {
        // The title bar is above the top of every screen, out of reach.
        let bounds = WindowBounds {
            x: 100,
            y: -500,
            width: 800,
            height: 700,
        };

        assert_ne!(place_window(bounds, &[MAIN]).position, Some((100, -500)));
    }

    #[test]
    fn a_window_whose_title_bar_is_on_screen_keeps_its_place() {
        // The content starts one title bar below the top of the visible area.
        let bounds = WindowBounds {
            x: 100,
            y: 25 + 28,
            width: 800,
            height: 600,
        };

        assert_eq!(place_window(bounds, &[MAIN]).position, Some((100, 53)));
    }

    #[test]
    fn a_window_whose_title_bar_would_be_under_the_menu_bar_is_moved() {
        // Content at the very top of the visible area: the title bar above it is out of reach.
        let bounds = WindowBounds {
            x: 100,
            y: 25,
            width: 800,
            height: 600,
        };

        assert_ne!(place_window(bounds, &[MAIN]).position, Some((100, 25)));
    }

    #[test]
    fn a_window_mostly_off_the_side_but_with_a_grip_on_screen_stays() {
        let bounds = WindowBounds {
            x: -700,
            y: 200,
            width: 800,
            height: 600,
        };

        assert_eq!(place_window(bounds, &[MAIN]).position, Some((-700, 200)));
    }

    #[test]
    fn without_known_screens_only_the_size_is_restored() {
        let bounds = WindowBounds {
            x: 10,
            y: 10,
            width: 800,
            height: 600,
        };

        assert_eq!(
            place_window(bounds, &[]),
            WindowPlacement {
                position: None,
                size: (800, 600),
            }
        );
    }

    // --- recovery list ---

    #[test]
    fn a_recovered_session_is_named_by_its_first_page_and_tab_count() {
        let snapshot = session(vec![
            window(
                vec![
                    saved_tab(NEW_TAB_PAGE_URL, "New Tab"),
                    saved_tab("https://a.test/", "  Page\nA "),
                ],
                0,
            ),
            window(vec![saved_tab("https://b.test/", "B")], 0),
        ]);
        assert_eq!(recovery_label(&snapshot), "Page A (3 tabs)");

        let untitled = session(vec![window(vec![saved_tab("https://u.test/", "")], 0)]);
        assert_eq!(recovery_label(&untitled), "https://u.test/ (1 tab)");
    }
}
