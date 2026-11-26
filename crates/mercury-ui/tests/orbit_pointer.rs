// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! What the Orbit view reports for the pointer and the keyboard, through Slint's testing backend:
//! hover and click on a planet, a preview, a chip, the card and the empty stage; Esc and ⌘↑; and
//! the drag and drop wiring (`DragArea` of a planet's hole and of a preview, `DropArea` of the
//! planets, the chips, the "New group" ring and the empty stage) with the payload callbacks the
//! host answers. The window is an `OrbitPreview` filled through the real pipeline (`tests/orbit_scenes/mod.rs`).
//!
//! The tests marked "ring" read what the real `RingBands` does with the pointer (`hit-test`, its own drag).

mod common;

#[allow(dead_code)]
#[path = "orbit_scenes/mod.rs"]
mod scenes;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::a11y;
use mercury_ui::orbit_view::{OrbitPayload, Source};
use mercury_ui::{
    OrbitCard, OrbitChip, OrbitNeighbour, OrbitPlanet, OrbitPreview, OrbitPreviewCard, RingHit,
    RingHitKind,
};
use scenes::{Frame, Scene, scene};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, DataTransfer, LogicalPosition};

type Log = Rc<RefCell<Vec<String>>>;

const SESSION: u64 = 7;

fn payload(source: Source) -> DataTransfer {
    let mut data = DataTransfer::default();
    data.set_user_data(Rc::new(OrbitPayload {
        session: SESSION,
        source,
    }));
    data
}

fn decode(data: &DataTransfer) -> Option<OrbitPayload> {
    data.user_data()?
        .downcast::<OrbitPayload>()
        .ok()
        .map(|payload| *payload)
}

/// The Slint rows of the scene, as the stage shows them.
struct Rows {
    planets: Vec<OrbitPlanet>,
    previews: Vec<OrbitPreviewCard>,
    chips: Vec<OrbitChip>,
    neighbours: Vec<OrbitNeighbour>,
    card: OrbitCard,
}

struct Harness {
    window: OrbitPreview,
    scene: Scene,
    log: Log,
    rows: Rows,
}

impl Harness {
    fn new(frame: Frame) -> Self {
        a11y::init();
        let scene = scene(frame, false);
        let window = scene.window.clone_strong();
        let rows = Rows {
            planets: scene.data.planets.iter().map(|p| p.row.clone()).collect(),
            previews: scene.data.previews.iter().map(|p| p.row.clone()).collect(),
            chips: scene.data.chips.clone(),
            neighbours: scene.data.neighbours.clone(),
            card: scene.data.card.clone(),
        };
        a11y::open_window(&window, 1440.0, 900.0);
        // The pure hit tests of the rings, as the glue answers them: the ring's own data.
        let rings: Rc<Vec<(i32, mercury_ui::ring_bands::RingBandsData)>> = Rc::new(
            scene
                .data
                .planets
                .iter()
                .chain(scene.data.centre.iter())
                .map(|p| (p.row.group, p.ring.clone()))
                .collect(),
        );
        let for_hit = Rc::clone(&rings);
        window.on_ring_hit_test(move |group, x, y, hovered| {
            for_hit
                .iter()
                .find(|(g, _)| *g == group)
                .map_or_else(outside, |(_, ring)| ring.hit(x, y, hovered))
        });
        let for_drop = Rc::clone(&rings);
        window.on_ring_drop_target(move |group, x, y, dragging| {
            for_drop
                .iter()
                .find(|(g, _)| *g == group)
                .map(|(_, ring)| ring.drop_target(x, y, dragging))
                .unwrap_or_default()
        });
        let log: Log = Rc::default();
        let push = |log: &Log| {
            let log = Rc::clone(log);
            move |line: String| log.borrow_mut().push(line)
        };
        let add = push(&log);
        window.on_planet_hovered({
            let add = add.clone();
            move |id| add(format!("planet-hovered {id}"))
        });
        window.on_planet_clicked({
            let add = add.clone();
            move |id| add(format!("planet-clicked {id}"))
        });
        window.on_preview_hovered({
            let add = add.clone();
            move |id| add(format!("preview-hovered {id}"))
        });
        window.on_preview_clicked({
            let add = add.clone();
            move |id| add(format!("preview-clicked {id}"))
        });
        window.on_chip_clicked({
            let add = add.clone();
            move |id| add(format!("chip-clicked {id}"))
        });
        window.on_card_clicked({
            let add = add.clone();
            move || add("card-clicked".into())
        });
        window.on_neighbour_clicked({
            let add = add.clone();
            move |id| add(format!("neighbour-clicked {id}"))
        });
        window.on_backdrop_clicked({
            let add = add.clone();
            move || add("backdrop-clicked".into())
        });
        window.on_new_group_clicked({
            let add = add.clone();
            move || add("new-group-clicked".into())
        });
        window.on_back_clicked({
            let add = add.clone();
            move || add("back-clicked".into())
        });
        window.on_search_edited({
            let add = add.clone();
            move |text| add(format!("search-edited {text}"))
        });
        window.on_key_pressed({
            let add = add.clone();
            move |text, command, shift, alt, control| {
                let key = text.chars().next().map(u32::from).unwrap_or_default();
                add(format!(
                    "key {key:#x} command={command} shift={shift} alt={alt} control={control}"
                ));
                true
            }
        });
        window.on_ring_drag_started({
            let add = add.clone();
            move |group, tab| add(format!("ring-drag-started {group} {tab}"))
        });
        window.on_ring_drag_cancelled({
            let add = add.clone();
            move || add("ring-drag-cancelled".into())
        });
        window.on_payload_for(|kind, id| {
            let source = match kind.as_str() {
                "tab" => Source::Tab(id),
                _ => Source::Group(id),
            };
            payload(source)
        });
        window.on_payload_accepts(|data, kind, _id, _slot| {
            let Some(payload) = decode(&data) else {
                return false;
            };
            !(kind == "new-group" && matches!(payload.source, Source::Group(_)))
        });
        window.on_payload_dropped({
            let add = add.clone();
            move |data, kind, id, slot| {
                add(format!(
                    "dropped {kind} {id} {slot} {:?}",
                    decode(&data).map(|p| p.source)
                ));
                true
            }
        });
        window.on_drag_over({
            let add = add.clone();
            move |kind, id, _x, _y| add(format!("drag-over {kind} {id}"))
        });
        window.on_drag_source_started({
            let add = add.clone();
            move |kind, id| add(format!("drag-source-started {kind} {id}"))
        });
        window.on_drag_source_finished({
            let add = add.clone();
            move |accepted| add(format!("drag-source-finished {accepted}"))
        });
        Self {
            window,
            scene,
            log,
            rows,
        }
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    fn send(&self, event: WindowEvent) {
        self.window.window().dispatch_event(event);
    }

    fn at(x: f32, y: f32) -> LogicalPosition {
        LogicalPosition::new(x, y)
    }

    fn move_to(&self, x: f32, y: f32) {
        self.send(WindowEvent::PointerMoved {
            position: Self::at(x, y),
        });
    }

    fn press(&self, x: f32, y: f32) {
        self.move_to(x, y);
        self.send(WindowEvent::PointerPressed {
            position: Self::at(x, y),
            button: PointerEventButton::Left,
        });
    }

    fn release(&self, x: f32, y: f32) {
        self.send(WindowEvent::PointerReleased {
            position: Self::at(x, y),
            button: PointerEventButton::Left,
        });
    }

    fn click(&self, x: f32, y: f32) {
        self.press(x, y);
        self.release(x, y);
    }

    /// Press at `from`, move in a few steps, release at `to`.
    fn drag(&self, from: (f32, f32), to: (f32, f32)) {
        self.press(from.0, from.1);
        for step in 1..=8 {
            let t = step as f32 / 8.0;
            self.move_to(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        }
        self.release(to.0, to.1);
    }

    fn key(&self, key: Key) {
        self.send(WindowEvent::KeyPressed { text: key.into() });
        self.send(WindowEvent::KeyReleased { text: key.into() });
    }

    fn planet(&self, group: i32) -> (f32, f32) {
        let row = self
            .rows
            .planets
            .iter()
            .find(|p| p.group == group)
            .expect("the planet");
        (row.x, row.y)
    }
}

fn outside() -> RingHit {
    RingHit {
        kind: RingHitKind::Outside,
        id: -1,
    }
}

fn has(log: &[String], line: &str) -> bool {
    log.iter().any(|entry| entry == line)
}

fn has_prefix(log: &[String], prefix: &str) -> bool {
    log.iter().any(|entry| entry.starts_with(prefix))
}

// ---- hover and click

#[test]
fn hovering_a_planet_reports_its_group_and_leaving_reports_none() {
    let h = Harness::new(Frame::Overview);
    let (x, y) = h.planet(3);
    h.move_to(x, y);
    assert!(has(&h.take(), "planet-hovered 3"));
    h.move_to(720.0, 470.0);
    assert!(has(&h.take(), "planet-hovered -1"));
}

#[test]
fn a_click_in_the_hole_zooms_into_the_planet() {
    let h = Harness::new(Frame::Overview);
    for group in [1, 2, 3, 4, 5] {
        let (x, y) = h.planet(group);
        h.click(x, y);
        let log = h.take();
        assert!(
            has(&log, &format!("planet-clicked {group}")),
            "group {group}: {log:?}"
        );
        assert!(!has(&log, "backdrop-clicked"), "group {group}: {log:?}");
    }
}

#[test]
fn a_click_on_the_halo_margin_zooms_into_the_planet_and_outside_the_halo_does_not() {
    let h = Harness::new(Frame::Overview);
    let (x, y) = h.planet(1);
    let halo = h.rows.planets[0].halo_radius;
    h.click(x, y - halo + 3.0);
    let log = h.take();
    assert!(has(&log, "planet-clicked 1"), "{log:?}");
    h.click(x, y - halo - 8.0);
    let log = h.take();
    assert!(!has_prefix(&log, "planet-clicked"), "{log:?}");
    assert!(has(&log, "backdrop-clicked"), "{log:?}");
}

#[test]
fn a_click_on_empty_space_and_on_the_sun_is_a_backdrop_click() {
    let h = Harness::new(Frame::Overview);
    h.click(500.0, 470.0);
    assert_eq!(h.take(), vec!["backdrop-clicked"]);
    h.click(720.0, 470.0);
    assert_eq!(h.take(), vec!["backdrop-clicked"]);
}

#[test]
fn a_click_on_the_title_bar_is_not_a_backdrop_click() {
    let h = Harness::new(Frame::Overview);
    h.click(300.0, 24.0);
    assert!(h.take().is_empty());
}

#[test]
fn the_new_group_ring_reports_a_click() {
    let h = Harness::new(Frame::Overview);
    let ring = h.scene.data.new_group.clone();
    h.click(ring.x, ring.y);
    assert_eq!(h.take(), vec!["new-group-clicked"]);
}

#[test]
fn back_to_the_page_reports_a_click() {
    let h = Harness::new(Frame::Overview);
    h.click(1380.0, 24.0);
    assert_eq!(h.take(), vec!["back-clicked"]);
}

#[test]
fn a_click_on_a_preview_reports_its_tab_and_hover_follows_the_pointer() {
    let h = Harness::new(Frame::Group);
    for card in h.rows.previews.clone() {
        h.move_to(card.x, card.y);
        let log = h.take();
        assert!(
            has(&log, &format!("preview-hovered {}", card.tab)),
            "{}: {log:?}",
            card.title
        );
        h.click(card.x, card.y);
        let log = h.take();
        assert!(
            has(&log, &format!("preview-clicked {}", card.tab)),
            "{}: {log:?}",
            card.title
        );
    }
    h.move_to(720.0, 470.0);
    assert!(has(&h.take(), "preview-hovered -1"));
}

#[test]
fn a_click_on_an_other_group_chip_reports_the_group() {
    let h = Harness::new(Frame::Group);
    for chip in h.rows.chips.clone() {
        h.click(chip.x + chip.width / 2.0, chip.y + chip.height / 2.0);
        assert_eq!(h.take(), vec![format!("chip-clicked {}", chip.group)]);
    }
}

#[test]
fn at_level_three_the_card_the_neighbours_and_the_rest_report_separately() {
    let h = Harness::new(Frame::Tab);
    let card = &h.rows.card;
    h.click(card.x + card.width / 2.0, card.y + card.height / 2.0);
    assert_eq!(h.take(), vec!["card-clicked"]);
    for near in h.rows.neighbours.clone() {
        // The part of a neighbour that is on the stage.
        let x = (near.x.max(0.0) + (near.x + near.width).min(1440.0)) / 2.0;
        h.click(x, near.y + near.height / 2.0);
        assert_eq!(h.take(), vec![format!("neighbour-clicked {}", near.tab)]);
    }
    h.click(720.0, 800.0);
    assert_eq!(h.take(), vec!["backdrop-clicked"]);
}

// ---- keys

#[test]
fn escape_and_command_up_reach_the_host_with_their_modifiers() {
    let h = Harness::new(Frame::Overview);
    h.window.invoke_focus_view();
    h.key(Key::Escape);
    let log = h.take();
    assert_eq!(
        log,
        vec![format!(
            "key {:#x} command=false shift=false alt=false control=false",
            u32::from(char::from(Key::Escape))
        )]
    );
    // Slint calls the Command key `control` on macOS: Key::Control sets it.
    h.send(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    h.send(WindowEvent::KeyPressed {
        text: Key::UpArrow.into(),
    });
    h.send(WindowEvent::KeyReleased {
        text: Key::UpArrow.into(),
    });
    h.send(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    let log = h.take();
    assert!(
        log.contains(&format!(
            "key {:#x} command=true shift=false alt=false control=false",
            u32::from(char::from(Key::UpArrow))
        )),
        "{log:?}"
    );
}

#[test]
fn a_typed_letter_and_the_return_key_reach_the_host() {
    let h = Harness::new(Frame::Group);
    h.window.invoke_focus_view();
    h.send(WindowEvent::KeyPressed { text: "n".into() });
    h.send(WindowEvent::KeyReleased { text: "n".into() });
    h.key(Key::Return);
    let log = h.take();
    assert!(has_prefix(&log, "key 0x6e "), "{log:?}");
    assert!(has_prefix(&log, "key 0xa "), "{log:?}");
}

#[test]
fn typing_in_the_search_field_reports_the_text() {
    let h = Harness::new(Frame::Overview);
    h.window.invoke_focus_search();
    for letter in ["l", "i", "s"] {
        h.send(WindowEvent::KeyPressed {
            text: letter.into(),
        });
        h.send(WindowEvent::KeyReleased {
            text: letter.into(),
        });
    }
    let log = h.take();
    assert!(has(&log, "search-edited lis"), "{log:?}");
    assert_eq!(h.window.get_search_text(), "lis");
}

// ---- drag and drop through DragArea and DropArea

#[test]
fn dragging_a_planet_by_its_hole_onto_another_planet_drops_the_group_there() {
    let h = Harness::new(Frame::Overview);
    let from = h.planet(1);
    let to = h.planet(3);
    h.drag(from, to);
    let log = h.take();
    assert!(has(&log, "drag-source-started group 1"), "{log:?}");
    assert!(has(&log, "drag-over group 3"), "{log:?}");
    assert!(has(&log, "dropped group 3 -1 Some(Group(1))"), "{log:?}");
    assert!(has(&log, "drag-source-finished true"), "{log:?}");
    assert!(!has_prefix(&log, "planet-clicked"), "{log:?}");
}

#[test]
fn the_loose_planet_cannot_be_dragged() {
    // `payload-for` answers an empty transfer for what may not be dragged: the DragArea stays put.
    let h = Harness::new(Frame::Overview);
    h.window.on_payload_for(|_, _| DataTransfer::default());
    h.drag(h.planet(1), h.planet(3));
    let log = h.take();
    assert!(!has_prefix(&log, "drag-source-started"), "{log:?}");
}

#[test]
fn a_drag_over_the_new_group_ring_is_reported_and_a_group_is_refused_there() {
    let h = Harness::new(Frame::Overview);
    let ring = h.scene.data.new_group.clone();
    h.drag(h.planet(1), (ring.x, ring.y));
    let log = h.take();
    assert!(has(&log, "drag-over new-group -1"), "{log:?}");
    // The host's `payload-accepts` says no for a group: nothing is dropped on the ring.
    assert!(!has_prefix(&log, "dropped new-group"), "{log:?}");
    assert!(has(&log, "drag-source-finished false"), "{log:?}");
}

#[test]
fn a_group_dropped_on_empty_space_is_offered_to_the_empty_stage() {
    let h = Harness::new(Frame::Overview);
    h.drag(h.planet(1), (500.0, 470.0));
    let log = h.take();
    assert!(has(&log, "drag-over empty -1"), "{log:?}");
    assert!(has(&log, "dropped empty -1 -1 Some(Group(1))"), "{log:?}");
}

#[test]
fn dragging_a_preview_onto_a_chip_drops_the_tab_on_that_group() {
    let h = Harness::new(Frame::Group);
    let card = h.rows.previews[2].clone();
    let chip = h.rows.chips[0].clone();
    h.drag(
        (card.x, card.y),
        (chip.x + chip.width / 2.0, chip.y + chip.height / 2.0),
    );
    let log = h.take();
    assert!(
        has(&log, &format!("drag-source-started tab {}", card.tab)),
        "{log:?}"
    );
    assert!(
        has(&log, &format!("drag-over group {}", chip.group)),
        "{log:?}"
    );
    assert!(
        has(
            &log,
            &format!("dropped group {} -1 Some(Tab({}))", chip.group, card.tab)
        ),
        "{log:?}"
    );
    assert!(!has_prefix(&log, "preview-clicked"), "{log:?}");
}

#[test]
fn a_foreign_transfer_is_not_accepted_and_nothing_is_dropped() {
    let h = Harness::new(Frame::Group);
    h.window.on_payload_for(|_, _| {
        DataTransfer::from(slint::SharedString::from("https://example.org/"))
    });
    let card = h.rows.previews[0].clone();
    let chip = h.rows.chips[0].clone();
    h.drag(
        (card.x, card.y),
        (chip.x + chip.width / 2.0, chip.y + chip.height / 2.0),
    );
    let log = h.take();
    assert!(!has_prefix(&log, "dropped"), "{log:?}");
}

// ---- ring (needs the finished ring-bands)

#[test]
fn ring_a_press_and_move_on_a_planet_segment_starts_the_rings_own_drag() {
    let h = Harness::new(Frame::Overview);
    let row = h.rows.planets[0].clone();
    let data = h.scene.data.planets[0].ring.clone();
    // The middle of the first segment of Research's tab band, in stage coordinates.
    let segment = data.segments.first().expect("a ring with segments");
    let origin = (row.x - row.ring_size / 2.0, row.y - row.ring_size / 2.0);
    let (sx, sy) = (origin.0 + segment.mid_x, origin.1 + segment.mid_y);
    h.press(sx, sy);
    h.move_to(sx + 12.0, sy + 12.0);
    h.move_to(sx + 30.0, sy + 30.0);
    let log = h.take();
    assert!(
        has(&log, &format!("ring-drag-started 1 {}", segment.tab)),
        "{log:?}"
    );
    assert_ne!(
        data.hit(segment.mid_x, segment.mid_y, -1).kind,
        RingHitKind::Outside
    );
}

#[test]
fn ring_the_centre_ring_takes_the_pointer_at_level_two_and_not_at_level_three() {
    let two = Harness::new(Frame::Group);
    let asked = Rc::new(RefCell::new(0));
    let counter = Rc::clone(&asked);
    two.window.on_ring_hit_test(move |_, _, _, _| {
        *counter.borrow_mut() += 1;
        RingHit {
            kind: RingHitKind::Outside,
            id: -1,
        }
    });
    two.move_to(720.0 + 60.0, 470.0);
    assert!(*asked.borrow() > 0, "the level-2 centre ring was not asked");

    let three = Harness::new(Frame::Tab);
    let asked = Rc::new(RefCell::new(0));
    let counter = Rc::clone(&asked);
    three.window.on_ring_hit_test(move |_, _, _, _| {
        *counter.borrow_mut() += 1;
        RingHit {
            kind: RingHitKind::Outside,
            id: -1,
        }
    });
    three.move_to(100.0, 470.0);
    three.move_to(720.0, 800.0);
    assert_eq!(*asked.borrow(), 0, "the huge ring at level 3 only draws");
}

// ---- what the host reads and drives: text focus, the open animation, the pinch reset

/// Moves Slint's mocked clock on (animations, timers and change handlers run).
fn elapse(milliseconds: u64) {
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(milliseconds));
}

fn close_to(value: f32, expected: f32) -> bool {
    (value - expected).abs() < 0.002
}

#[test]
fn text_field_focused_is_true_only_while_the_search_field_has_the_focus() {
    let h = Harness::new(Frame::Overview);
    h.window.invoke_focus_view();
    elapse(1);
    assert!(!h.window.get_text_field_focused(), "the key scope has it");
    h.window.invoke_focus_search();
    elapse(1);
    assert!(h.window.get_text_field_focused(), "the search field has it");
    h.window.invoke_focus_view();
    elapse(1);
    assert!(!h.window.get_text_field_focused(), "blurred again");
}

#[test]
fn text_field_focused_is_true_while_the_popover_name_field_has_the_focus() {
    let h = Harness::new(Frame::NewGroup);
    elapse(1);
    assert!(
        h.window.get_popover().visible,
        "the new-group frame shows the popover"
    );
    assert!(
        h.window.get_text_field_focused(),
        "the name field takes the focus when the popover opens"
    );
    h.window.invoke_focus_view();
    elapse(1);
    assert!(
        !h.window.get_text_field_focused(),
        "the key scope has it now"
    );
}

#[test]
fn a_popover_that_closes_with_its_field_focused_no_longer_counts() {
    let h = Harness::new(Frame::NewGroup);
    elapse(1);
    assert!(h.window.get_text_field_focused());
    h.window.set_popover(mercury_ui::OrbitPopover::default());
    elapse(1);
    assert!(!h.window.get_text_field_focused(), "the field is gone");
}

#[test]
fn play_open_runs_320_ms_from_zero_to_one_in_three_phases() {
    let h = Harness::new(Frame::Overview);
    h.window.set_reduce_motion(false);
    assert_eq!(
        h.window.get_open_progress(),
        1.0,
        "at rest nothing is lifted"
    );
    assert_eq!(h.window.get_open_scale(), 1.0);
    assert_eq!(h.window.get_open_opacity(), 1.0);

    h.window.invoke_play_open();
    assert_eq!(h.window.get_open_progress(), 0.0, "it starts at zero");
    assert!(close_to(h.window.get_open_scale(), 0.94));
    assert!(close_to(h.window.get_open_opacity(), 0.0));

    // Phase 1, 80 ms: the page lifts into a card.
    elapse(80);
    assert!(
        close_to(h.window.get_open_scale(), 0.98),
        "{}",
        h.window.get_open_scale()
    );
    assert!(
        close_to(h.window.get_open_opacity(), 0.6),
        "{}",
        h.window.get_open_opacity()
    );

    // Phase 2, 160 ms: it shrinks toward its group (the middle of the run).
    elapse(80);
    let middle = h.window.get_open_progress();
    assert!(middle > 0.0 && middle < 1.0, "{middle}");
    elapse(80);
    assert!(
        close_to(h.window.get_open_scale(), 1.02),
        "{}",
        h.window.get_open_scale()
    );
    assert!(
        close_to(h.window.get_open_opacity(), 1.0),
        "{}",
        h.window.get_open_opacity()
    );

    // Phase 3, 80 ms: it settles, and the end is exactly the static frame.
    elapse(80);
    assert_eq!(h.window.get_open_progress(), 1.0);
    assert_eq!(h.window.get_open_scale(), 1.0);
    assert_eq!(h.window.get_open_opacity(), 1.0);
    elapse(1000);
    assert_eq!(h.window.get_open_progress(), 1.0, "it stays there");
}

#[test]
fn play_open_is_strictly_between_zero_and_one_at_160_ms() {
    let h = Harness::new(Frame::Overview);
    h.window.set_reduce_motion(false);
    h.window.invoke_play_open();
    assert_eq!(h.window.get_open_progress(), 0.0);
    elapse(160);
    let progress = h.window.get_open_progress();
    assert!(progress > 0.0 && progress < 1.0, "{progress}");
    elapse(160);
    assert_eq!(h.window.get_open_progress(), 1.0, "exactly 1 at 320 ms");
}

#[test]
fn play_open_again_restarts_from_zero() {
    let h = Harness::new(Frame::Overview);
    h.window.set_reduce_motion(false);
    h.window.invoke_play_open();
    elapse(400);
    assert_eq!(h.window.get_open_progress(), 1.0);
    h.window.invoke_play_open();
    assert_eq!(h.window.get_open_progress(), 0.0);
}

#[test]
fn play_open_with_reduced_motion_is_at_the_end_at_once() {
    let h = Harness::new(Frame::Overview);
    h.window.set_reduce_motion(true);
    h.window.invoke_play_open();
    assert_eq!(h.window.get_open_progress(), 1.0);
    assert_eq!(h.window.get_open_scale(), 1.0);
    assert_eq!(h.window.get_open_opacity(), 1.0);
}

#[test]
fn focus_view_clears_a_pinch_that_never_ended() {
    let h = Harness::new(Frame::Overview);
    h.window.set_reduce_motion(false);
    // A pinch is live and has reached its end; its `ended` is lost (Esc in the middle of it).
    h.window.set_pinch_live(true);
    h.window.set_pinch_progress(1.0);
    assert_eq!(
        h.window.get_pinch_progress(),
        1.0,
        "a live pinch follows the fingers"
    );
    h.window.invoke_focus_view();
    assert!(
        !h.window.get_pinch_live(),
        "no pinch is live after the view takes the focus"
    );
    assert_eq!(
        h.window.get_pinch_progress(),
        0.0,
        "the next Orbit opens unscaled"
    );
    // The next spring-back is the 160 ms one, not a jump.
    h.window.set_pinch_progress(1.0);
    assert_eq!(
        h.window.get_pinch_progress(),
        0.0,
        "it starts from where it was"
    );
    elapse(80);
    let half = h.window.get_pinch_progress();
    assert!(half > 0.0 && half < 1.0, "{half}");
    elapse(80);
    assert_eq!(h.window.get_pinch_progress(), 1.0);
}

// ---- cancelling the ring's own drag (ring)

impl Harness {
    /// Press on the middle of Research's first segment and move 12 px: the ring's own drag is live.
    fn start_ring_drag(&self) -> (f32, f32, i32) {
        let row = self.rows.planets[0].clone();
        let data = &self.scene.data.planets[0].ring;
        let segment = data.segments.first().expect("a ring with segments");
        let (sx, sy) = (
            row.x - row.ring_size / 2.0 + segment.mid_x,
            row.y - row.ring_size / 2.0 + segment.mid_y,
        );
        self.press(sx, sy);
        self.move_to(sx + 12.0, sy + 12.0);
        (sx + 12.0, sy + 12.0, segment.tab)
    }

    /// Slint runs change handlers (where the rings end their drag) before the next event: a move
    /// of the pointer far from every ring is such an event.
    fn pump(&self) {
        self.move_to(500.0, 470.0);
    }
}

#[test]
fn ring_cancel_ring_drag_ends_a_live_drag_once_and_the_ghost_goes() {
    let h = Harness::new(Frame::Overview);
    let (x, y, tab) = h.start_ring_drag();
    let log = h.take();
    assert!(has(&log, &format!("ring-drag-started 1 {tab}")), "{log:?}");
    assert_eq!(h.window.get_ring_drag_source(), tab, "the ring is dragging");

    h.window.invoke_cancel_ring_drag();
    h.pump();
    let log = h.take();
    assert_eq!(
        log.iter().filter(|l| *l == "ring-drag-cancelled").count(),
        1,
        "{log:?}"
    );
    assert_eq!(h.window.get_ring_drag_source(), -1, "no ghost is left");
    // The button is still down: the release afterwards drops nothing and cancels nothing again.
    h.release(x, y);
    let log = h.take();
    assert!(!has_prefix(&log, "ring-dropped"), "{log:?}");
    assert!(!has(&log, "ring-drag-cancelled"), "{log:?}");
}

#[test]
fn ring_cancel_ring_drag_with_no_drag_live_does_nothing() {
    let h = Harness::new(Frame::Overview);
    h.window.invoke_cancel_ring_drag();
    h.pump();
    h.window.invoke_cancel_ring_drag();
    h.pump();
    assert!(h.take().iter().all(|l| l.starts_with("planet-hovered")));
    assert_eq!(h.window.get_ring_drag_source(), -1);
}

#[test]
fn ring_a_level_change_cancels_a_live_ring_drag() {
    let h = Harness::new(Frame::Overview);
    let (_, _, tab) = h.start_ring_drag();
    assert_eq!(h.window.get_ring_drag_source(), tab);
    h.take();
    h.window.set_level(mercury_ui::OrbitLevel::Group);
    h.pump();
    let log = h.take();
    assert_eq!(
        log.iter().filter(|l| *l == "ring-drag-cancelled").count(),
        1,
        "{log:?}"
    );
    assert_eq!(h.window.get_ring_drag_source(), -1);
}
