// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The hub ring's keys and pointer through Slint's testing backend (SPEC §5 "Ring popover",
//! brief §7 item 4): `RingPopoverPreview` with the 22-tab mockup scene, `HubRing::install` and a
//! host that records its calls. Key events go in as `WindowEvent::KeyPressed`, the way the
//! platform backend sends them (a modifier is a key press of its own, then the key).
//!
//! The focus ring is drawn only after a key moved the focus, so `focused-tab` is -1 until then
//! while the controller's cursor is already on the active tab.
//!
//! The testing backend reports a modifier key as the key it is (Slint's winit backend swaps ⌘ and ⌃
//! on macOS, so ⌘ is `Key::Control` there): [`command_key`] and [`control_key`] give the key to
//! send for each, per platform. ⌥ with a letter or digit is not sent as a key event: the window
//! hook calls `HubRing::option_key`, as on a real keyboard where the event carries "ß" or "¡".
#![cfg(feature = "test-support")]

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use mercury_ui::hub_ring::{HubRing, HubRingHost, HubRingInput, fixtures};
use mercury_ui::ring_bands::RingColours;
use mercury_ui::{
    ChromePalette, ChromeTheme, HubRingBridge, RingHit, RingHitKind, RingPopoverPreview, ring_bands,
};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, SharedString};

/// Remembers every call the ring makes on the browser, as text ("activate 101").
#[derive(Default)]
struct Recorder {
    calls: RefCell<Vec<String>>,
}

impl Recorder {
    fn note(&self, call: String) {
        self.calls.borrow_mut().push(call);
    }

    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }

    fn clear(&self) {
        self.calls.borrow_mut().clear();
    }
}

impl HubRingHost for Recorder {
    fn activate_tab(&self, tab: i32) {
        self.note(format!("activate {tab}"));
    }
    fn close_tab(&self, tab: i32) {
        self.note(format!("close {tab}"));
    }
    fn toggle_mute(&self, tab: i32) {
        self.note(format!("mute {tab}"));
    }
    fn toggle_pause(&self, tab: i32) {
        self.note(format!("pause {tab}"));
    }
    fn move_tab(&self, tab: i32, group: i32, index: usize) {
        self.note(format!("move {tab} {group} {index}"));
    }
    fn open_orbit(&self) {
        self.note("orbit".to_owned());
    }
    fn search_web(&self, query: &str) {
        self.note(format!("web {query}"));
    }
}

struct Rig {
    window: RingPopoverPreview,
    ring: HubRing,
    host: Rc<Recorder>,
    input: HubRingInput,
}

impl Rig {
    fn new() -> Self {
        common::a11y::init();
        let window = RingPopoverPreview::new().expect("the preview window can be created");
        common::a11y::open_window(&window, 516.0, 516.0);
        let host = Rc::new(Recorder::default());
        let ring = HubRing::install(&window, host.clone(), ring_bands::metrics());
        let input = fixtures::mockup_input(false);
        ring.set_input(input.clone());
        ring.opened();
        let rig = Self {
            window,
            ring,
            host,
            input,
        };
        // What a popup does when it opens; the window's `changed` handlers run: the search field
        // reports that it has the focus.
        rig.window.invoke_focus_keys();
        rig.elapse(1);
        rig
    }

    /// Closes and opens the popover again (the preview has no popup that would report `closed`).
    fn reopen(&self) {
        self.bridge().invoke_closed();
        self.ring.opened();
        self.window.invoke_focus_keys();
        self.elapse(1);
    }

    /// The pointer comes over a tab, as RingBands reports it.
    fn hover(&self, tab: i32) {
        self.bridge().set_hovered_tab(tab);
        self.bridge().invoke_tab_hovered(tab);
        // The centre may change mode (an audio tab hides the field): its `changed` handler runs.
        self.elapse(1);
    }

    fn bridge(&self) -> HubRingBridge<'_> {
        self.window.global::<HubRingBridge>()
    }

    fn send(&self, event: WindowEvent) {
        self.window.window().dispatch_event(event);
        // A real event loop runs the `changed` handlers (the popover moves the keyboard focus when
        // the centre's mode changes) before the next event arrives.
        i_slint_backend_testing::mock_elapsed_time(Duration::ZERO);
    }

    /// One key, pressed and released.
    fn key(&self, text: &str) {
        let text = SharedString::from(text);
        self.send(WindowEvent::KeyPressed { text: text.clone() });
        self.send(WindowEvent::KeyReleased { text });
    }

    fn named(&self, key: Key) {
        self.key(&char::from(key).to_string());
    }

    fn down(&self, key: Key) {
        self.send(WindowEvent::KeyPressed {
            text: char::from(key).to_string().into(),
        });
    }

    fn up(&self, key: Key) {
        self.send(WindowEvent::KeyReleased {
            text: char::from(key).to_string().into(),
        });
    }

    /// `key` with `modifier` held: the modifier goes down, then the key, then the modifier up.
    fn chord(&self, modifier: Key, key: &str) {
        self.down(modifier);
        self.key(key);
        self.up(modifier);
    }

    fn type_text(&self, text: &str) {
        for character in text.chars() {
            self.key(&character.to_string());
        }
    }

    fn elapse(&self, millis: u64) {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(millis));
        slint::platform::update_timers_and_animations();
    }

    fn focused_tab(&self) -> i32 {
        self.bridge().get_focused_tab()
    }

    fn title(&self) -> String {
        self.bridge().get_title().to_string()
    }

    fn close_requests(&self) -> i32 {
        self.bridge().get_close_request()
    }

    fn row(&self, id: i32) -> &mercury_ui::TabRow {
        self.input
            .tabs
            .iter()
            .find(|row| row.id == id)
            .expect("the scene has this tab")
    }

    fn first_tab_of(&self, group: i32) -> i32 {
        self.input
            .tabs
            .iter()
            .find(|row| row.group == group && !row.pinned && row.index_in_group == 0)
            .map(|row| row.id)
            .expect("the group has a first tab")
    }

    /// Moves the focus with → (through the beads too) until it is on the segment of `id`.
    fn focus(&self, id: i32) {
        for _ in 0..=self.input.tabs.len() {
            if self.focused_tab() == id {
                return;
            }
            self.named(Key::RightArrow);
        }
        assert_eq!(
            self.focused_tab(),
            id,
            "→ walks the whole order and reaches the tab"
        );
    }
}

const ESSAY: i32 = 111;
const TRAM: i32 = 108;
const AUDIO: i32 = 115;
const LISBON: i32 = 2;

/// The key the testing backend must be sent for ⌘: Slint reports ⌘ as `control` on macOS.
fn command_key() -> Key {
    if cfg!(target_os = "macos") {
        Key::Control
    } else {
        Key::Meta
    }
}

/// The key sent for ⌃: the other one.
fn control_key() -> Key {
    if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    }
}

fn backspace() -> String {
    char::from(Key::Backspace).to_string()
}

// The spike of the lane: a pure callback on the bridge global, set from Rust, answers a call
// through the global. (That `RingBands` reaches it from inside `RingPopoverContent` is proved by
// the pointer tests below once the finished ring-bands is in; here it is compile-checked: the
// popover forwards `hit-test` to the global.)
#[test]
fn a_pure_callback_set_on_the_bridge_global_from_rust_answers() {
    common::a11y::init();
    let window = RingPopoverPreview::new().expect("window");
    common::a11y::open_window(&window, 516.0, 516.0);
    let bridge = window.global::<HubRingBridge>();
    bridge.on_hit_test(|x, y, hovered| RingHit {
        kind: RingHitKind::Tab,
        id: x as i32 + y as i32 + hovered,
    });
    let hit = bridge.invoke_hit_test(10.0, 20.0, 5);
    assert_eq!(hit.kind, RingHitKind::Tab);
    assert_eq!(hit.id, 35);
}

#[test]
fn install_fills_the_bridge_for_the_scene() {
    let rig = Rig::new();
    let bridge = rig.bridge();
    assert_eq!(
        rig.focused_tab(),
        -1,
        "no focus ring before a key moves the focus"
    );
    assert_eq!(
        rig.title(),
        "Servo layout notes",
        "the centre shows the active tab"
    );
    assert!(!bridge.get_query_active());
    assert!(!bridge.get_held());
    assert_eq!(bridge.get_field_placeholder().as_str(), "Search 25 tabs");
    assert_eq!(rig.close_requests(), 0);
}

#[test]
fn left_and_right_move_the_focus_and_the_centre_follows() {
    let rig = Rig::new();
    rig.named(Key::RightArrow);
    assert_eq!(rig.focused_tab(), 102);
    assert_eq!(rig.title(), "WebRender overview");
    rig.named(Key::LeftArrow);
    assert_eq!(rig.focused_tab(), 101);
    assert_eq!(rig.title(), "Servo layout notes");
    // Past the last segment the focus walks the pinned beads, then wraps to the first segment.
    rig.focus(122);
    rig.named(Key::RightArrow);
    assert_eq!(rig.bridge().get_focused_bead(), 200);
    assert_eq!(rig.focused_tab(), -1);
    assert_eq!(rig.title(), "Mail");
    rig.named(Key::LeftArrow);
    assert_eq!(rig.focused_tab(), 122);
    assert_eq!(rig.bridge().get_focused_bead(), -1);
}

#[test]
fn up_and_down_jump_to_the_first_tab_of_the_next_or_previous_group() {
    let rig = Rig::new();
    rig.named(Key::DownArrow);
    assert_eq!(rig.focused_tab(), rig.first_tab_of(2));
    rig.named(Key::UpArrow);
    assert_eq!(rig.focused_tab(), rig.first_tab_of(1));
    rig.named(Key::UpArrow);
    assert_eq!(
        rig.focused_tab(),
        rig.first_tab_of(6),
        "up from the first group wraps to the last"
    );
}

#[test]
fn enter_shows_the_focused_tab_and_closes_the_popover() {
    let rig = Rig::new();
    rig.focus(ESSAY);
    rig.named(Key::Return);
    assert_eq!(rig.host.calls(), [format!("activate {ESSAY}")]);
    assert_eq!(rig.close_requests(), 1);
}

// Review finding 1. The old code read ⌘ from `meta`; Slint reports ⌘ as `control` on macOS, so
// (on macOS) this test fails on it: ⌘⌫ was a plain Backspace and nothing was closed.
#[test]
fn command_backspace_closes_the_focused_tab_and_the_focus_moves_on() {
    let rig = Rig::new();
    rig.focus(ESSAY);
    rig.chord(command_key(), &backspace());
    assert_eq!(rig.host.calls(), [format!("close {ESSAY}")]);
    assert_eq!(rig.focused_tab(), ESSAY + 1);
    // The host removes the tab and sends the new rows: the focus stays where it moved.
    let mut input = rig.input.clone();
    input.tabs.retain(|row| row.id != ESSAY);
    rig.ring.set_input(input);
    assert_eq!(rig.focused_tab(), ESSAY + 1);
    assert_eq!(
        rig.close_requests(),
        0,
        "closing a tab does not close the popover"
    );
}

// Review finding 1, the other half: on macOS the testing backend's `Key::Meta` is ⌃, which closed a
// tab in the old code (it read ⌘ from `meta`). It must close nothing and edit nothing.
#[test]
fn control_backspace_does_not_close_a_tab() {
    let rig = Rig::new();
    rig.focus(ESSAY);
    rig.chord(control_key(), &backspace());
    assert!(rig.host.calls().is_empty(), "{:?}", rig.host.calls());
}

// Review finding 1: with a query typed, ⌘⌫ closes the tab and leaves the query alone (the old code
// deleted a character of the query instead, on macOS).
#[test]
fn command_backspace_does_not_edit_the_query() {
    let rig = Rig::new();
    rig.type_text("tram");
    rig.chord(command_key(), &backspace());
    assert_eq!(rig.bridge().get_query().as_str(), "tram");
    assert_eq!(rig.host.calls().len(), 1, "{:?}", rig.host.calls());
    assert!(rig.host.calls()[0].starts_with("close "));
}

#[test]
fn backspace_alone_edits_the_query_and_closes_nothing() {
    let rig = Rig::new();
    rig.type_text("tram");
    rig.named(Key::Backspace);
    assert_eq!(rig.bridge().get_query().as_str(), "tra");
    assert!(rig.host.calls().is_empty());
}

#[test]
fn option_left_moves_the_tab_within_its_group_and_does_nothing_at_the_end() {
    let rig = Rig::new();
    rig.chord(Key::Alt, &char::from(Key::LeftArrow).to_string());
    assert!(
        rig.host.calls().is_empty(),
        "the first tab of a group cannot move left: {:?}",
        rig.host.calls()
    );
    rig.named(Key::RightArrow);
    rig.chord(Key::Alt, &char::from(Key::LeftArrow).to_string());
    assert_eq!(rig.host.calls(), ["move 102 1 0"]);
}

#[test]
fn m_mutes_a_tab_with_sound_and_a_silent_tab_gets_no_mute() {
    let rig = Rig::new();
    rig.focus(AUDIO);
    rig.key("m");
    assert_eq!(rig.host.calls(), [format!("mute {AUDIO}")]);
    assert_eq!(
        rig.bridge().get_query().as_str(),
        "",
        "the M was a command, not text"
    );

    rig.host.clear();
    rig.focus(ESSAY);
    rig.key("m");
    assert!(rig.host.calls().is_empty(), "{:?}", rig.host.calls());
}

#[test]
fn typing_searches_and_selects_the_first_result() {
    let rig = Rig::new();
    rig.type_text("tram");
    let bridge = rig.bridge();
    assert_eq!(bridge.get_query().as_str(), "tram");
    assert!(bridge.get_query_active());
    let rows: Vec<_> = bridge.get_results().iter().collect();
    assert_eq!(rows.len(), 2, "one match and the web row: {rows:?}");
    assert_eq!(rows[0].tab, TRAM);
    assert!(rows[0].title.contains("Tram"));
    assert!(rows[0].selected);
    assert_eq!(rows[1].tab, -1);
    assert!(rows[1].title.contains("tram"), "{}", rows[1].title);
    assert!(!rows[1].selected);
}

// The faded segments are rows of RingBands' own models.
#[test]
fn typing_fades_every_tab_but_the_match() {
    let rig = Rig::new();
    rig.type_text("tram");
    let tabs = rig.bridge().get_tabs();
    let lit: Vec<i32> = tabs
        .iter()
        .filter(|t| t.matches_query)
        .map(|t| t.id)
        .collect();
    assert_eq!(lit, [TRAM]);
}

#[test]
fn backspace_edits_the_query_and_the_field_text_stays_when_it_is_empty() {
    let rig = Rig::new();
    rig.type_text("tram");
    rig.named(Key::Backspace);
    assert_eq!(rig.bridge().get_query().as_str(), "tra");
    for _ in 0..3 {
        rig.named(Key::Backspace);
    }
    assert_eq!(rig.bridge().get_query().as_str(), "");
    assert!(!rig.bridge().get_query_active());
}

#[test]
fn down_and_enter_open_the_second_result_and_the_web_row_searches_the_web() {
    let rig = Rig::new();
    rig.type_text("servo");
    let rows: Vec<_> = rig.bridge().get_results().iter().collect();
    assert_eq!(rows.len(), 3, "two matches and the web row: {rows:?}");
    rig.named(Key::DownArrow);
    let selected: Vec<bool> = rig
        .bridge()
        .get_results()
        .iter()
        .map(|r| r.selected)
        .collect();
    assert_eq!(selected, [false, true, false]);
    rig.named(Key::Return);
    assert_eq!(rig.host.calls(), [format!("activate {}", rows[1].tab)]);
    assert_eq!(rig.close_requests(), 1);

    // The popover reports `closed`; the next time it opens the query is empty again.
    rig.host.clear();
    rig.reopen();
    rig.type_text("servo");
    rig.named(Key::DownArrow);
    rig.named(Key::DownArrow);
    rig.named(Key::Return);
    assert_eq!(rig.host.calls(), ["web servo"]);
    assert_eq!(rig.close_requests(), 2);
}

#[test]
fn up_wraps_from_the_first_result_to_the_web_row() {
    let rig = Rig::new();
    rig.type_text("servo");
    rig.named(Key::UpArrow);
    let selected: Vec<bool> = rig
        .bridge()
        .get_results()
        .iter()
        .map(|r| r.selected)
        .collect();
    assert_eq!(selected, [false, false, true]);
}

// Review finding 9: the first Esc only ends the drag; the query and the popover are left alone. The
// old code also cleared the query (and, without one, closed the popover) in the same press.
#[test]
fn escape_ends_a_drag_in_progress_as_a_release_outside_would() {
    let rig = Rig::new();
    rig.type_text("tram");
    let cancelled = Rc::new(Cell::new(0));
    let count = Rc::clone(&cancelled);
    rig.bridge()
        .on_drag_cancelled(move || count.set(count.get() + 1));
    // Press on a segment of Reading (straight below the centre) and move 14 px: a drag begins.
    let start = LogicalPosition::new(258.0, 258.0 + 180.0);
    rig.send(WindowEvent::PointerMoved { position: start });
    rig.send(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    rig.send(WindowEvent::PointerMoved {
        position: LogicalPosition::new(start.x + 14.0, start.y),
    });
    assert_eq!(cancelled.get(), 0, "the drag is still going");
    rig.named(Key::Escape);
    assert_eq!(cancelled.get(), 1, "Esc cancels the drag, no drop");
    assert!(
        rig.host.calls().is_empty(),
        "a cancelled drag moves nothing: {:?}",
        rig.host.calls()
    );
    assert_eq!(rig.close_requests(), 0, "the first Esc does not close");
    assert_eq!(
        rig.bridge().get_query().as_str(),
        "tram",
        "the first Esc does not clear the query"
    );
    rig.send(WindowEvent::PointerReleased {
        position: start,
        button: PointerEventButton::Left,
    });
    rig.named(Key::Escape);
    assert_eq!(rig.bridge().get_query().as_str(), "", "the next Esc clears");
    assert_eq!(rig.close_requests(), 0);
    rig.named(Key::Escape);
    assert_eq!(rig.close_requests(), 1, "the next one closes");
}

#[test]
fn escape_clears_the_query_first_and_closes_the_popover_second() {
    let rig = Rig::new();
    rig.type_text("tram");
    rig.named(Key::Escape);
    assert_eq!(rig.bridge().get_query().as_str(), "");
    assert!(!rig.bridge().get_query_active());
    assert_eq!(rig.close_requests(), 0);
    rig.named(Key::Escape);
    assert_eq!(rig.close_requests(), 1);
}

#[test]
fn holding_option_for_the_hold_time_turns_the_key_look_on_and_releasing_early_does_not() {
    let rig = Rig::new();
    rig.down(Key::Alt);
    assert!(!rig.bridge().get_held());
    rig.elapse(399);
    assert!(!rig.bridge().get_held(), "399 ms is not yet 400");
    rig.elapse(1);
    assert!(rig.bridge().get_held());
    rig.up(Key::Alt);
    assert!(!rig.bridge().get_held(), "releasing ends the look");
    assert!(
        rig.host.calls().is_empty(),
        "no group was chosen: nothing is shown"
    );

    rig.down(Key::Alt);
    rig.elapse(200);
    rig.up(Key::Alt);
    rig.elapse(500);
    assert!(
        !rig.bridge().get_held(),
        "released before the hold time: the timer is stopped"
    );
}

// Review finding 2: ⌥ with a letter or digit arrives composed on macOS, so the window hook calls
// `option_key` with the physical key. Before this round there was no such entry point (the popover
// only understood "s" and "2" with the alt flag, which a Mac keyboard never sends).
#[test]
fn option_key_s_then_2_shows_lisbon_trips_second_tab() {
    let rig = Rig::new();
    rig.ring.option_key('S');
    assert_eq!(rig.bridge().get_selected_group(), LISBON);
    assert!(
        rig.bridge().get_held(),
        "choosing a group turns the look on"
    );
    rig.ring.option_key('2');
    assert_eq!(rig.host.calls(), ["activate 107"]);
    assert_eq!(rig.close_requests(), 1);
}

#[test]
fn option_key_s_then_releasing_option_commits_the_group() {
    let rig = Rig::new();
    rig.ring.option_key('S');
    rig.up(Key::Alt);
    assert_eq!(
        rig.host.calls(),
        [format!("activate {}", rig.first_tab_of(LISBON))]
    );
    assert!(!rig.bridge().get_held());
}

#[test]
fn composed_option_characters_and_other_characters_are_ignored() {
    let rig = Rig::new();
    rig.down(Key::Alt);
    rig.key("ß");
    rig.key("¡");
    rig.key("s");
    assert_eq!(
        rig.bridge().get_selected_group(),
        -1,
        "no group is chosen by a composed character"
    );
    for ignored in ['s', '0', '!', 'ß', '¡', ' '] {
        rig.ring.option_key(ignored);
    }
    assert_eq!(rig.bridge().get_selected_group(), -1);
    assert!(rig.host.calls().is_empty(), "{:?}", rig.host.calls());
}

#[test]
fn option_s_chooses_lisbon_trip_and_digit_2_shows_its_second_tab() {
    let rig = Rig::new();
    rig.down(Key::Alt);
    rig.elapse(400);
    rig.ring.option_key('S');
    let bridge = rig.bridge();
    assert_eq!(bridge.get_selected_group(), LISBON);
    assert_eq!(bridge.get_held_name().as_str(), "Lisbon trip");
    assert!(
        bridge.get_held_help().contains("1–4"),
        "{}",
        bridge.get_held_help()
    );
    rig.ring.option_key('2');
    assert_eq!(rig.host.calls(), ["activate 107"]);
    assert_eq!(rig.close_requests(), 1);
}

#[test]
fn a_digit_beyond_the_group_does_nothing_and_a_letter_without_a_group_too() {
    let rig = Rig::new();
    rig.down(Key::Alt);
    rig.elapse(400);
    rig.ring.option_key('Q');
    assert_eq!(rig.bridge().get_selected_group(), -1);
    rig.ring.option_key('S');
    rig.ring.option_key('9');
    assert!(rig.host.calls().is_empty(), "{:?}", rig.host.calls());
    assert_eq!(rig.bridge().get_selected_group(), LISBON);
}

#[test]
fn releasing_option_after_a_group_letter_shows_that_groups_last_active_tab() {
    let mut rig = Rig::new();
    rig.input.last_active = vec![(LISBON, 108)];
    rig.ring.set_input(rig.input.clone());
    rig.down(Key::Alt);
    rig.ring.option_key('S');
    assert_eq!(rig.bridge().get_selected_group(), LISBON);
    rig.up(Key::Alt);
    assert_eq!(rig.host.calls(), ["activate 108"]);
    assert_eq!(rig.close_requests(), 1);
    assert!(!rig.bridge().get_held());
}

#[test]
fn releasing_option_without_a_last_active_tab_shows_the_groups_first_tab() {
    let rig = Rig::new();
    rig.down(Key::Alt);
    rig.ring.option_key('S');
    rig.up(Key::Alt);
    assert_eq!(
        rig.host.calls(),
        [format!("activate {}", rig.first_tab_of(LISBON))]
    );
}

#[test]
fn set_held_from_outside_shows_the_look_at_once_and_releasing_commits() {
    let rig = Rig::new();
    rig.ring.set_held(true);
    assert!(rig.bridge().get_held());
    rig.ring.option_key('S');
    assert_eq!(rig.bridge().get_selected_group(), LISBON);
    rig.ring.set_held(false);
    assert_eq!(
        rig.host.calls(),
        [format!("activate {}", rig.first_tab_of(LISBON))]
    );
    assert!(!rig.bridge().get_held());
}

#[test]
fn clicking_a_bead_shows_its_tab() {
    let rig = Rig::new();
    let beads: Vec<_> = rig.bridge().get_beads().iter().collect();
    assert_eq!(
        beads.len(),
        3,
        "the three pinned tabs of the scene each get a bead"
    );
    let second = &beads[1];
    // The row's x and y are the bead's top-left corner: click its middle.
    let position = LogicalPosition::new(second.x + second.size / 2.0, second.y + second.size / 2.0);
    rig.send(WindowEvent::PointerMoved { position });
    rig.send(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    rig.send(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert_eq!(rig.host.calls(), [format!("activate {}", second.tab)]);
    assert_eq!(rig.close_requests(), 1);
}

// The pointer reaches a segment only through RingBands' hit-test, which it forwards to the bridge
// global: this is the proof that the hop works at run time.
#[test]
fn the_pointer_over_a_segment_names_it_in_the_centre_and_a_click_shows_it() {
    let rig = Rig::new();
    // Straight below the centre on the middle of the band (radius 180): a tab of Reading, the
    // violet group at the bottom of the ring.
    let position = LogicalPosition::new(258.0, 258.0 + 180.0);
    rig.send(WindowEvent::PointerMoved { position });
    assert_ne!(
        rig.bridge().get_hovered_tab(),
        -1,
        "the pointer is over a segment"
    );
    let hovered = rig.bridge().get_hovered_tab();
    assert_eq!(rig.title(), rig.row(hovered).title.as_str());
    rig.send(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    rig.send(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert_eq!(rig.host.calls(), [format!("activate {hovered}")]);
}

// The hit-test the pointer uses is `RingBandsData::hit`.
#[test]
fn the_hit_test_through_the_bridge_is_the_rings_own() {
    let rig = Rig::new();
    let hit = rig.bridge().invoke_hit_test(258.0, 258.0, -1);
    assert_eq!(hit.kind, RingHitKind::Centre);
    let outside = rig.bridge().invoke_hit_test(-30.0, -30.0, -1);
    assert_eq!(outside.kind, RingHitKind::Outside);
}

#[test]
fn set_input_with_fewer_tabs_keeps_a_surviving_focus_and_moves_off_a_removed_one() {
    let rig = Rig::new();
    rig.focus(103);

    let mut fewer = rig.input.clone();
    fewer.tabs.retain(|row| row.id != 120 && row.id != 121);
    rig.ring.set_input(fewer);
    assert_eq!(rig.focused_tab(), 103, "the focused tab survived");

    let mut fewer = rig.input.clone();
    fewer.tabs.retain(|row| row.id != 103);
    rig.ring.set_input(fewer.clone());
    let now = rig.focused_tab();
    assert_ne!(now, 103);
    assert!(
        fewer.tabs.iter().any(|row| row.id == now && !row.pinned),
        "the focus is on a tab that still exists: {now}"
    );
    assert_eq!(rig.title(), rig.row(now).title.as_str());
}

#[test]
fn set_input_keeps_the_query_and_runs_the_search_again() {
    let rig = Rig::new();
    rig.type_text("tram");
    let mut input = rig.input.clone();
    input.tabs.retain(|row| row.id != TRAM);
    rig.ring.set_input(input);
    assert_eq!(rig.bridge().get_query().as_str(), "tram");
    let rows: Vec<_> = rig.bridge().get_results().iter().collect();
    assert_eq!(rows.len(), 1, "only the web row is left: {rows:?}");
    assert_eq!(rows[0].tab, -1);
}

#[test]
fn opened_resets_the_query_the_focus_and_the_look() {
    let rig = Rig::new();
    rig.type_text("tram");
    rig.focus(AUDIO);
    rig.ring.opened();
    assert_eq!(rig.bridge().get_query().as_str(), "");
    assert_eq!(rig.focused_tab(), -1, "the focus ring is hidden again");
    rig.named(Key::RightArrow);
    assert_eq!(
        rig.focused_tab(),
        102,
        "the focus restarted on the active tab, 101"
    );
    assert!(rig.bridge().get_hub_open());
    assert!(!rig.bridge().get_held());
}

#[test]
fn the_centre_buttons_act_on_the_centre_tab() {
    let rig = Rig::new();
    rig.focus(AUDIO);
    rig.bridge().invoke_pause_clicked();
    rig.bridge().invoke_mute_clicked();
    assert_eq!(
        rig.host.calls(),
        [format!("pause {AUDIO}"), format!("mute {AUDIO}")]
    );
    rig.host.clear();
    rig.bridge().invoke_see_everything();
    assert_eq!(rig.host.calls(), ["orbit"]);
}

#[test]
fn a_result_row_activates_through_the_bridge_too() {
    let rig = Rig::new();
    rig.type_text("tram");
    rig.bridge().invoke_result_activated(0);
    assert_eq!(rig.host.calls(), [format!("activate {TRAM}")]);
    rig.host.clear();
    rig.reopen();
    rig.type_text("tram");
    rig.bridge().invoke_result_activated(1);
    assert_eq!(rig.host.calls(), ["web tram"]);
}

// Review finding 8. The old code kept `hovered` over the keyboard cursor: with the pointer resting on
// a tab, → moved the focus ring but the centre kept naming the hovered tab, and ⏎ opened the other
// one.
#[test]
fn the_pointer_and_the_keyboard_name_the_same_tab() {
    let rig = Rig::new();
    rig.hover(103);
    assert_eq!(rig.title(), rig.row(103).title.as_str());
    rig.named(Key::RightArrow);
    assert_eq!(
        rig.focused_tab(),
        104,
        "the focus followed the pointer, then moved on"
    );
    assert_eq!(
        rig.title(),
        rig.row(104).title.as_str(),
        "the centre follows the key"
    );
    rig.named(Key::Return);
    assert_eq!(rig.host.calls(), ["activate 104"]);
}

// Review finding 8: M is gated on the tab the centre shows and mutes that same tab. The old code
// gated on the hovered tab and muted the keyboard cursor's, so M did nothing (cursor on a silent tab).
#[test]
fn m_acts_on_the_tab_the_centre_shows() {
    let rig = Rig::new();
    rig.hover(AUDIO);
    rig.key("m");
    assert_eq!(rig.host.calls(), [format!("mute {AUDIO}")]);

    rig.host.clear();
    rig.hover(103);
    rig.key("m");
    assert!(rig.host.calls().is_empty(), "{:?}", rig.host.calls());
}

// Review finding 11: the search field has the keyboard focus when the popover opens, so an input
// method can begin with the first key. The field reports it, Rust leaves it the printable keys, and
// the field's `edited` drives the query.
#[test]
fn the_field_has_the_focus_at_open_and_takes_the_typing() {
    let rig = Rig::new();
    assert!(rig.bridge().get_field_focused(), "the field has the focus");
    // A leading space: the field keeps it, Rust's own typing would drop it. So this went
    // through the field.
    rig.type_text(" tram");
    assert_eq!(rig.bridge().get_query().as_str(), " tram");
    assert!(rig.bridge().get_query_active());
    let rows: Vec<_> = rig.bridge().get_results().iter().collect();
    assert_eq!(rows[0].tab, TRAM, "{rows:?}");
    // A key that is not text still reaches Rust first.
    rig.named(Key::DownArrow);
    let selected: Vec<bool> = rig
        .bridge()
        .get_results()
        .iter()
        .map(|r| r.selected)
        .collect();
    assert_eq!(selected, [false, true]);
}

#[test]
fn keys_that_are_not_text_reach_the_ring_while_the_field_has_the_focus() {
    let rig = Rig::new();
    assert!(rig.bridge().get_field_focused());
    rig.named(Key::RightArrow);
    assert_eq!(rig.focused_tab(), 102);
    rig.named(Key::Return);
    assert_eq!(rig.host.calls(), ["activate 102"]);
}

#[test]
fn when_the_field_has_no_focus_the_controller_types() {
    let rig = Rig::new();
    rig.bridge().set_field_focused(false);
    // Rust's typing drops a leading space.
    rig.type_text(" tram");
    assert_eq!(rig.bridge().get_query().as_str(), "tram");
    assert_eq!(
        rig.bridge().get_results().iter().next().map(|r| r.tab),
        Some(TRAM)
    );
}

// Review finding 16: Space pauses or plays an audio tab with an empty query (the field is hidden in
// audio mode); without media it is a plain space for the field.
#[test]
fn space_pauses_the_audio_tab_the_centre_shows() {
    let rig = Rig::new();
    rig.hover(AUDIO);
    rig.key(" ");
    assert_eq!(rig.host.calls(), [format!("pause {AUDIO}")]);
    rig.host.clear();
    rig.hover(103);
    rig.key(" ");
    assert!(rig.host.calls().is_empty(), "{:?}", rig.host.calls());
}

// Review finding 14: Esc while ⌥ is down or the key look is on cancels the look (nothing is shown,
// the popover stays); the old code closed the popover and ⌥ up then still committed the group.
#[test]
fn escape_while_the_key_look_is_on_cancels_it_and_the_option_release_commits_nothing() {
    let rig = Rig::new();
    rig.down(Key::Alt);
    rig.elapse(400);
    rig.ring.option_key('S');
    assert_eq!(rig.bridge().get_selected_group(), LISBON);
    rig.named(Key::Escape);
    assert!(!rig.bridge().get_held());
    assert_eq!(rig.bridge().get_selected_group(), -1);
    assert_eq!(
        rig.close_requests(),
        0,
        "Esc ended the look, it did not close"
    );
    rig.up(Key::Alt);
    assert!(rig.host.calls().is_empty(), "{:?}", rig.host.calls());
    // ⌥ down and Esc before the hold time: the timer is stopped too.
    rig.down(Key::Alt);
    rig.named(Key::Escape);
    rig.elapse(500);
    assert!(!rig.bridge().get_held());
    assert_eq!(rig.close_requests(), 0);
}

// Review finding 14: after a close was asked for, nothing acts until the popover reports `closed`
// (the 120 ms close animation): ⏎ then ⌘⌫ closed a tab, and Esc then ⌥-release committed a group.
#[test]
fn nothing_acts_while_the_popover_is_closing() {
    let rig = Rig::new();
    rig.named(Key::Return);
    assert_eq!(rig.host.calls(), ["activate 101"]);
    assert_eq!(rig.close_requests(), 1);
    rig.chord(command_key(), &backspace());
    rig.named(Key::Return);
    rig.named(Key::RightArrow);
    rig.ring.option_key('S');
    rig.bridge().invoke_tab_activated(103);
    rig.bridge().invoke_bead_activated(200);
    rig.bridge().invoke_tab_close(103);
    assert_eq!(rig.host.calls(), ["activate 101"], "{:?}", rig.host.calls());
    assert_eq!(rig.close_requests(), 1);
    // It acts again once the popover has closed and opened again.
    rig.reopen();
    rig.named(Key::Return);
    assert_eq!(rig.host.calls(), ["activate 101", "activate 101"]);
}

// Review finding 5: the ring's key letters and counts take the window's live palette colours, read
// at install and on every `set_input`; `input.colours` overrides them and there is no literal.
#[test]
fn the_label_colours_are_the_live_palette_unless_the_input_overrides_them() {
    let rig = Rig::new();
    let accent = |rig: &Rig| rig.window.global::<ChromePalette>().get_accent();
    let key_colours = |rig: &Rig| -> Vec<slint::Color> {
        rig.bridge()
            .get_label_glyphs()
            .iter()
            .map(|g| g.colour)
            .collect()
    };
    assert!(
        key_colours(&rig).contains(&accent(&rig)),
        "the labels' ⌥ and key letters use the light palette's accent"
    );

    rig.window
        .global::<ChromePalette>()
        .set_theme(ChromeTheme::Dark);
    let dark_accent = accent(&rig);
    assert_ne!(dark_accent, accent_of_light());
    rig.ring.set_input(rig.input.clone());
    assert!(
        key_colours(&rig).contains(&dark_accent),
        "after the theme changed, set_input reads the dark palette"
    );

    let mut input = rig.input.clone();
    let red = slint::Color::from_rgb_u8(200, 10, 10);
    input.colours = Some(RingColours {
        key: red,
        ..RingColours::default()
    });
    rig.ring.set_input(input);
    assert!(key_colours(&rig).contains(&red), "the input's colours win");
}

fn accent_of_light() -> slint::Color {
    let window = RingPopoverPreview::new().expect("window");
    window
        .global::<ChromePalette>()
        .set_theme(ChromeTheme::Light);
    window.global::<ChromePalette>().get_accent()
}

#[test]
fn the_field_edit_runs_the_same_search() {
    let rig = Rig::new();
    rig.bridge().set_query("lisbon".into());
    rig.bridge().invoke_query_edited("lisbon".into());
    assert!(rig.bridge().get_query_active());
    let rows: Vec<_> = rig.bridge().get_results().iter().collect();
    assert_eq!(
        rows.first().map(|r| r.tab),
        Some(101 + 5),
        "Flights to Lisbon: {rows:?}"
    );
}

#[test]
fn a_drop_moves_the_tab_that_the_ring_was_dragging() {
    let rig = Rig::new();
    // RingBands asks the drop target for the dragged tab while it drags.
    let _ = rig.bridge().invoke_drop_target(300.0, 300.0, ESSAY);
    rig.bridge().invoke_drop(LISBON, 1);
    assert_eq!(rig.host.calls(), [format!("move {ESSAY} {LISBON} 1")]);
    rig.host.clear();
    rig.bridge().invoke_drop(LISBON, 1);
    assert!(rig.host.calls().is_empty(), "one drag moves one tab");
}
