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

use mercury_option_keys::{CardChange, CardView, Event};
use mercury_ui::MainWindow;
use mercury_ui::orbit_view;
use talaria_core::BrowserController;

use crate::option_hook;
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
// What the card's Slint callbacks call (layout-switch wires them). Each sends the machine the
// event the click means (`option_hook::card_event`), so the answer is applied exactly as a key's
// is. The conversions of the callbacks' arguments happen here, once (brief D9).
// ---------------------------------------------------------------------------------------------

/// The machine's event for a chip: `digit_1_based` is the chip's `index` (1 to 9), the machine's
/// `index` is 0-based. A digit outside 1 to 9 is no chip.
fn chip_clicked_event(digit_1_based: i32) -> Option<Event> {
    if !(1..=9).contains(&digit_1_based) {
        return None;
    }
    let index = usize::try_from(digit_1_based - 1).ok()?;
    Some(Event::ChipClicked { index })
}

/// The machine's event for a tile: the group is the row's id, which is never negative for a group
/// (`-1` is "none").
fn tile_clicked_event(group_row_id: i32) -> Option<Event> {
    let group = u64::try_from(group_row_id).ok()?;
    Some(Event::TileClicked { group })
}

/// A tile of the card was clicked: `group_row_id` is the row's `GroupRow.id`.
pub(crate) fn card_tile_clicked(
    controller: &Rc<RefCell<BrowserController>>,
    window: &MainWindow,
    group_row_id: i32,
) {
    if let Some(event) = tile_clicked_event(group_row_id) {
        option_hook::card_event(window, controller, event);
    }
}

/// A tab chip of the card was clicked: `digit_1_based` is the chip's `index` (1 to 9).
pub(crate) fn card_chip_clicked(
    controller: &Rc<RefCell<BrowserController>>,
    window: &MainWindow,
    digit_1_based: i32,
) {
    if let Some(event) = chip_clicked_event(digit_1_based) {
        option_hook::card_event(window, controller, event);
    }
}

/// The card's backdrop was pressed (`KeySwitcher.dismissed()`). The hook does not watch the
/// pointer itself (brief D16): a winit press would also fire for a tile before Slint delivers
/// its click.
pub(crate) fn card_backdrop_pressed(
    controller: &Rc<RefCell<BrowserController>>,
    window: &MainWindow,
) {
    option_hook::card_event(window, controller, Event::PointerDown);
}

/// "Change keys…" was pressed (`KeySwitcher.change-keys()`): the card closes with no effect and
/// the host then opens Settings.
pub(crate) fn card_change_keys(controller: &Rc<RefCell<BrowserController>>, window: &MainWindow) {
    option_hook::card_event(window, controller, Event::Dismiss);
}

/// Reads the key and Orbit settings once and follows every change. Called once from
/// `connect_native_keyboard_input`.
pub(crate) fn follow_settings(settings: &Rc<SettingsHandle>) {
    option_hook::set_settings(option_hook::HookSettings::from_settings(
        &settings.settings(),
    ));
    // Weak: the handle owns its listeners, and the listener must not keep the handle alive.
    let handle = Rc::downgrade(settings);
    settings.subscribe_while(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let Some(settings) = handle.upgrade() else {
            return false;
        };
        option_hook::set_settings(option_hook::HookSettings::from_settings(
            &settings.settings(),
        ));
        true
    });
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use mercury_option_keys::{Config, KeyCapture};
    use slint::winit_030::winit::keyboard::ModifiersState;
    use talaria_core::{GroupId, GroupKey, TabActivation, TabId};

    use super::*;
    use crate::option_hook::{HookSettings, test_guard};

    // ---- D9: the card's arguments, converted once ------------------------------------------

    #[test]
    fn a_chip_digit_becomes_a_zero_based_index() {
        for digit in 1..=9 {
            assert_eq!(
                chip_clicked_event(digit),
                Some(Event::ChipClicked {
                    index: usize::try_from(digit - 1).expect("a small index")
                }),
                "digit {digit}"
            );
        }
        assert_eq!(chip_clicked_event(1), Some(Event::ChipClicked { index: 0 }));
    }

    #[test]
    fn a_digit_outside_one_to_nine_is_no_chip() {
        for digit in [0, 10, -1, i32::MIN, i32::MAX] {
            assert_eq!(chip_clicked_event(digit), None, "digit {digit}");
        }
    }

    #[test]
    fn a_tile_id_becomes_the_group_and_a_negative_one_is_ignored() {
        assert_eq!(tile_clicked_event(7), Some(Event::TileClicked { group: 7 }));
        assert_eq!(
            tile_clicked_event(i32::MAX),
            Some(Event::TileClicked {
                group: u64::from(i32::MAX.unsigned_abs())
            })
        );
        assert_eq!(tile_clicked_event(-1), None);
        assert_eq!(tile_clicked_event(i32::MIN), None);
    }

    // ---- the callbacks, through a real window, controller and registered card host ---------

    #[derive(Default)]
    struct FakeCard {
        changes: RefCell<Vec<CardChange>>,
    }

    impl CardHost for FakeCard {
        fn card_changed(&self, change: CardChange, _view: CardView) {
            self.changes.borrow_mut().push(change);
        }
    }

    struct Rig {
        window: MainWindow,
        controller: Rc<RefCell<BrowserController>>,
        card: Rc<FakeCard>,
        group: GroupId,
        loose: TabId,
        members: Vec<TabId>,
    }

    /// A window, a controller with the loose tab `a` showing and the group `Work` (key S) of the
    /// tabs `b` and `c`, a registered card host, and a 250 ms hold.
    fn rig() -> Rig {
        crate::settings::tests::init_slint();
        let window = MainWindow::new().expect("the main window can be created");
        let mut controller = BrowserController::default();
        let loose = controller.active_tab_id();
        let mut open = || {
            controller
                .open_tab_with(None, TabActivation::Background)
                .expect("a blank tab opens")
        };
        let members = vec![open(), open()];
        let group = controller
            .create_group(&members, Some("Work"), None, SystemTime::now())
            .expect("the group is made");
        controller
            .set_group_key(group, None)
            .expect("the key is cleared");
        controller
            .set_group_key(group, GroupKey::from_letter('S'))
            .expect("S is free");
        let card = Rc::new(FakeCard::default());
        register_card_host(card.clone());
        crate::option_hook::set_settings(HookSettings {
            config: Config::new(true, KeyCapture::default(), Duration::from_millis(250)),
            ..HookSettings::default()
        });
        Rig {
            window,
            controller: Rc::new(RefCell::new(controller)),
            card,
            group,
            loose,
            members,
        }
    }

    impl Rig {
        /// ⌥ held past the hold: the card is up.
        fn open_card(&self) {
            let alt = ModifiersState::ALT;
            crate::option_hook::modifiers(&self.window, &self.controller, true, alt);
            std::thread::sleep(Duration::from_millis(300));
            crate::option_hook::modifiers(&self.window, &self.controller, true, alt);
            assert_eq!(*self.card.changes.borrow(), vec![CardChange::Show]);
        }

        fn active(&self) -> TabId {
            self.controller.borrow().active_tab_id()
        }

        fn row_id(&self) -> i32 {
            i32::try_from(self.group.value()).expect("a small id")
        }
    }

    #[test]
    fn a_tile_click_on_the_card_opens_the_group() {
        let _guard = test_guard();
        let rig = rig();
        rig.open_card();

        card_tile_clicked(&rig.controller, &rig.window, rig.row_id());

        assert!(
            rig.members.contains(&rig.active()),
            "a tab of the group shows"
        );
        assert_eq!(
            *rig.card.changes.borrow(),
            vec![CardChange::Show, CardChange::Hide]
        );
    }

    #[test]
    fn a_tile_click_with_a_negative_id_does_nothing() {
        let _guard = test_guard();
        let rig = rig();
        rig.open_card();

        card_tile_clicked(&rig.controller, &rig.window, -1);

        assert_eq!(rig.active(), rig.loose);
        assert_eq!(*rig.card.changes.borrow(), vec![CardChange::Show]);
    }

    #[test]
    fn a_chip_click_with_no_group_chosen_does_nothing() {
        let _guard = test_guard();
        let rig = rig();
        rig.open_card();

        for digit in [0, 1, 10, -1] {
            card_chip_clicked(&rig.controller, &rig.window, digit);
        }

        assert_eq!(rig.active(), rig.loose);
        assert_eq!(*rig.card.changes.borrow(), vec![CardChange::Show]);
    }

    #[test]
    fn a_press_on_the_backdrop_closes_the_card_with_no_effect() {
        let _guard = test_guard();
        let rig = rig();
        rig.open_card();

        card_backdrop_pressed(&rig.controller, &rig.window);

        assert_eq!(rig.active(), rig.loose);
        assert_eq!(
            *rig.card.changes.borrow(),
            vec![CardChange::Show, CardChange::Hide]
        );
    }

    #[test]
    fn change_keys_closes_the_card_with_no_effect() {
        let _guard = test_guard();
        let rig = rig();
        rig.open_card();

        card_change_keys(&rig.controller, &rig.window);

        assert_eq!(rig.active(), rig.loose);
        assert_eq!(
            *rig.card.changes.borrow(),
            vec![CardChange::Show, CardChange::Hide]
        );
    }

    // ---- follow_settings -------------------------------------------------------------------

    #[test]
    fn the_settings_are_read_once_and_then_followed() {
        let _guard = test_guard();
        crate::settings::tests::init_slint();
        let handle = SettingsHandle::start(None);
        handle.update(|settings| settings.appearance.tab_groups.orbit_pinch = false);

        follow_settings(&handle);

        assert!(!option_hook::settings().orbit_pinch, "read at the start");
        assert!(option_hook::settings().orbit);

        handle.update(|settings| {
            settings.appearance.tab_groups.keys.hold_ms = 600;
            settings.appearance.tab_groups.orbit = false;
        });

        let followed = option_hook::settings();
        assert_eq!(followed.config.hold, Duration::from_millis(600));
        assert!(!followed.orbit);
        assert!(!followed.orbit_pinch);
    }
}
