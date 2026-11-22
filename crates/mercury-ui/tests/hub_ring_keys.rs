// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The hub ring's keys and pointer through Slint's testing backend (SPEC §5 "Ring popover",
//! brief §7 item 4): `RingPopoverPreview` with the 22-tab mockup scene, `HubRing::install` and a
//! host that records its calls. Key events go in as `WindowEvent::KeyPressed`, the way the
//! platform backend sends them (a modifier is a key press of its own, then the key).
//!
//! The focus ring is drawn only after a key moved the focus, so `focused-tab` is -1 until then
//! while the controller's cursor is already on the active tab.
#![cfg(feature = "test-support")]

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use mercury_ui::hub_ring::{HubRing, HubRingHost, HubRingInput, fixtures};
use mercury_ui::{HubRingBridge, RingHit, RingHitKind, RingPopoverPreview, ring_bands};
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
        Self {
            window,
            ring,
            host,
            input,
        }
    }

    fn bridge(&self) -> HubRingBridge<'_> {
        self.window.global::<HubRingBridge>()
    }

    fn send(&self, event: WindowEvent) {
        self.window.window().dispatch_event(event);
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

#[test]
fn command_backspace_closes_the_focused_tab_and_the_focus_moves_on() {
    let rig = Rig::new();
    rig.focus(ESSAY);
    rig.chord(Key::Meta, &char::from(Key::Backspace).to_string());
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

    rig.host.clear();
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

#[test]
fn escape_ends_a_drag_in_progress_as_a_release_outside_would() {
    let rig = Rig::new();
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

#[test]
fn option_s_chooses_lisbon_trip_and_digit_2_shows_its_second_tab() {
    let rig = Rig::new();
    rig.down(Key::Alt);
    rig.elapse(400);
    rig.key("s");
    let bridge = rig.bridge();
    assert_eq!(bridge.get_selected_group(), LISBON);
    assert_eq!(bridge.get_held_name().as_str(), "Lisbon trip");
    assert!(
        bridge.get_held_help().contains("1–4"),
        "{}",
        bridge.get_held_help()
    );
    rig.key("2");
    assert_eq!(rig.host.calls(), ["activate 107"]);
    assert_eq!(rig.close_requests(), 1);
}

#[test]
fn a_digit_beyond_the_group_does_nothing_and_a_letter_without_a_group_too() {
    let rig = Rig::new();
    rig.down(Key::Alt);
    rig.elapse(400);
    rig.key("q");
    assert_eq!(rig.bridge().get_selected_group(), -1);
    rig.key("s");
    rig.key("9");
    assert!(rig.host.calls().is_empty(), "{:?}", rig.host.calls());
    assert_eq!(rig.bridge().get_selected_group(), LISBON);
}

#[test]
fn releasing_option_after_a_group_letter_shows_that_groups_last_active_tab() {
    let mut rig = Rig::new();
    rig.input.last_active = vec![(LISBON, 108)];
    rig.ring.set_input(rig.input.clone());
    rig.down(Key::Alt);
    rig.key("s");
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
    rig.key("s");
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
    rig.key("s");
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
    rig.bridge().invoke_result_activated(1);
    assert_eq!(rig.host.calls(), ["web tram"]);
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
