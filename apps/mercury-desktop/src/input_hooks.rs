// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The seams between the window-level input hooks in `keyboard.rs` and what they act on
//! (lane input-hooks, brief `lane-input-hooks-brief.md` section 5).
//!
//! The hooks (the ⌥ group keys, the Orbit keys and pinch, the ⌃⇥ hold, F6) decide *when*; the
//! hosts below decide *what exists and how to act*. `layout-switch` implements and registers the
//! four hosts when the group interface is on. **With nothing registered every hook passes the
//! event on and does nothing** (brief D5, D22), so this lane changes no behaviour until a host
//! exists.
//!
//! The registry is a thread-local (the pattern of `option_keys.rs` and `ime.rs`): the winit hook,
//! the Slint callbacks and the timers all run on the main thread.

#![allow(dead_code)] // wired by the input-hooks `wire` item and by layout-switch

use std::cell::RefCell;
use std::rc::Rc;

use mercury_option_keys::{CardChange, CardView};
use mercury_ui::MainWindow;
use mercury_ui::orbit_view;
use talaria_core::BrowserController;

use crate::settings::SettingsHandle;

/// Implemented by layout-switch: the "Switch group" card is a Slint component in `MainWindow`.
pub(crate) trait CardHost {
    /// Called after the machine returned (never inside `option_keys::with_machine`). `view` is
    /// what the card shows now.
    fn card_changed(&self, change: CardChange, view: CardView);
}

/// Implemented by layout-switch over `OrbitController` and the `OrbitView` in `MainWindow`.
pub(crate) trait OrbitHost {
    /// Orbit exists in the current layout and `tab_groups.orbit` is on.
    fn available(&self) -> bool;
    /// Orbit is up (its overlay covers the page).
    fn is_open(&self) -> bool;
    /// A key while Orbit is open (`OrbitController::handle_key`). `true` = Orbit acted, so the key
    /// is consumed.
    fn handle_key(
        &self,
        key: orbit_view::Key,
        mods: orbit_view::Mods,
        context: orbit_view::KeyContext,
    ) -> bool;
    /// Page to Orbit pinch (SPEC §6.1): the page lifts toward the ring while the gesture runs.
    /// 0.0 is rest, 1.0 the committed distance.
    fn pinch_open_progress(&self, progress: f32);
    /// The gesture ended: `true` opens Orbit (level 1, from the page), `false` springs back over
    /// 160 ms.
    fn pinch_open_end(&self, commit: bool);
}

/// Implemented by layout-switch over the hub-ring controller (`HubRing`). The ⌃⇥ hold acts on it
/// (brief D18).
pub(crate) trait RingHost {
    /// The ring exists in this window (the group interface is on and the layout has a hub) and
    /// nothing (Orbit, a sheet) prevents opening it now.
    fn available(&self) -> bool;
    /// The ring popover is up. The hook asks at every key event, so that a ring closed by Esc, a
    /// click or a tab choice ends the hold.
    fn is_open(&self) -> bool;
    /// Open the ring with the focus on the active tab (`HubRing::opened` semantics, the "held"
    /// footer rather than the search field).
    fn open_held(&self);
    /// Move the ring's focus to the next (`backwards == false`) or previous tab in ring order
    /// (group by group, strip order inside a group). Never activates.
    fn cycle(&self, backwards: bool);
    /// ⌃ was released: activate the ring's focused tab if it is not the active one, then close
    /// the ring.
    fn commit_and_close(&self);
    /// The hold was abandoned (⌘ or ⌥ pressed, focus lost): close the ring without activating
    /// anything.
    fn close(&self);
    /// A physical key pressed with ⌥ alone while the ring is open (`HubRing::option_key`): an
    /// upper-case letter `'A'..='Z'` or a digit `'1'..='9'`, taken from the physical key because
    /// macOS composes ⌥S into "ß". The ring sets "⌥ down" itself; the ⌥ release that follows
    /// commits.
    fn option_key(&self, key: char);
}

/// The F6 targets, in cycle order (the order the variants are declared in).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Region {
    /// The address field.
    Address,
    /// The toolbar (its first focusable button; the hub button in the Mercury layout).
    Toolbar,
    /// The tab strip (Classic) or nothing.
    Tabs,
    /// The group dock, or its capsule while shrunk.
    Dock,
    /// The web view.
    Page,
}

/// Implemented by layout-switch, which owns `main-window.slint` and knows what is on screen and
/// how to focus it.
pub(crate) trait FocusHost {
    /// The regions that exist and can take focus now, in the order of [`Region`]. Never empty
    /// while the window is shown (the address field always exists).
    fn regions(&self) -> Vec<Region>;
    /// The region that has the keyboard focus now (`None`: a popup, a sheet, Settings, anything
    /// not listed).
    fn current(&self) -> Option<Region>;
    /// Move the focus to `region`. For [`Region::Page`] this hands the keyboard to the web view
    /// (`web_view_focused` becomes true through the existing Slint focus callback).
    fn focus(&self, region: Region);
}

#[derive(Default)]
struct Hosts {
    card: Option<Rc<dyn CardHost>>,
    orbit: Option<Rc<dyn OrbitHost>>,
    ring: Option<Rc<dyn RingHost>>,
    focus: Option<Rc<dyn FocusHost>>,
}

thread_local! {
    static HOSTS: RefCell<Hosts> = RefCell::new(Hosts::default());
}

pub(crate) fn register_card_host(host: Rc<dyn CardHost>) {
    HOSTS.with(|hosts| hosts.borrow_mut().card = Some(host));
}

pub(crate) fn register_orbit_host(host: Rc<dyn OrbitHost>) {
    HOSTS.with(|hosts| hosts.borrow_mut().orbit = Some(host));
}

pub(crate) fn register_ring_host(host: Rc<dyn RingHost>) {
    HOSTS.with(|hosts| hosts.borrow_mut().ring = Some(host));
}

pub(crate) fn register_focus_host(host: Rc<dyn FocusHost>) {
    HOSTS.with(|hosts| hosts.borrow_mut().focus = Some(host));
}

/// Forgets every host (a window going away, tests).
pub(crate) fn clear_hosts() {
    HOSTS.with(|hosts| *hosts.borrow_mut() = Hosts::default());
}

pub(crate) fn card_host() -> Option<Rc<dyn CardHost>> {
    HOSTS.with(|hosts| hosts.borrow().card.clone())
}

pub(crate) fn orbit_host() -> Option<Rc<dyn OrbitHost>> {
    HOSTS.with(|hosts| hosts.borrow().orbit.clone())
}

pub(crate) fn ring_host() -> Option<Rc<dyn RingHost>> {
    HOSTS.with(|hosts| hosts.borrow().ring.clone())
}

pub(crate) fn focus_host() -> Option<Rc<dyn FocusHost>> {
    HOSTS.with(|hosts| hosts.borrow().focus.clone())
}

// ---------------------------------------------------------------------------------------------
// What the card's Slint callbacks call (layout-switch wires them). STUBS in the scaffold; the
// `option-hook` item gives them bodies.
// ---------------------------------------------------------------------------------------------

/// A tile of the card was clicked: `group_row_id` is the row's `GroupRow.id`.
pub(crate) fn card_tile_clicked(
    _controller: &Rc<RefCell<BrowserController>>,
    _window: &MainWindow,
    _group_row_id: i32,
) {
}

/// A tab chip of the card was clicked: `digit_1_based` is the chip's `index` (1 to 9).
pub(crate) fn card_chip_clicked(
    _controller: &Rc<RefCell<BrowserController>>,
    _window: &MainWindow,
    _digit_1_based: i32,
) {
}

/// The card's backdrop was pressed (`KeySwitcher.dismissed()`).
pub(crate) fn card_backdrop_pressed(
    _controller: &Rc<RefCell<BrowserController>>,
    _window: &MainWindow,
) {
}

/// "Change keys…" was pressed (`KeySwitcher.change-keys()`): the card closes with no effect and
/// the host then opens Settings.
pub(crate) fn card_change_keys(_controller: &Rc<RefCell<BrowserController>>, _window: &MainWindow) {
}

/// Reads the key and Orbit settings once and follows every change. Called once from
/// `connect_native_keyboard_input`.
pub(crate) fn follow_settings(_settings: &Rc<SettingsHandle>) {}
