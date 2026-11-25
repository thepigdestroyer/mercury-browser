// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The pointer behaviour of `RingBands` through Slint's testing backend: hover, click, middle
//! click, leaving, the drag with its 6 px threshold, and `background-pressed` (lead request R1).
//!
//! The geometry callbacks are wired to `ring_geometry` directly (`RingModel` and `hit_test`), so
//! that these tests do not depend on `ring_bands::data`.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::a11y;
use mercury_ui::ring_bands::fixtures::{mockup_groups, mockup_tabs};
use mercury_ui::ring_geometry::slint_models::RingModel;
use mercury_ui::ring_geometry::{Hit, Point, RingSpec, hit_test, point_on_circle};
use mercury_ui::{ArcSeg, RingBandsPreview, RingDropTarget, RingHit, RingHitKind};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

const CENTRE: Point = Point { x: 258.0, y: 258.0 };

type Log = Rc<RefCell<Vec<String>>>;

struct Scene {
    window: RingBandsPreview,
    model: Rc<RingModel>,
    log: Log,
}

fn hit_of(model: &RingModel, x: f32, y: f32, hovered: i32) -> RingHit {
    let lifted = (0..model.layout.tabs.len()).find(|&flat| model.tab_id(flat) == Some(hovered));
    match hit_test(&model.layout, Point::new(x, y), lifted) {
        Hit::Tab(flat) => RingHit {
            kind: RingHitKind::Tab,
            id: model.tab_id(flat).unwrap_or(-1),
        },
        Hit::Group(group) => RingHit {
            kind: RingHitKind::Group,
            id: model.group_id(group).unwrap_or(-1),
        },
        Hit::Centre => RingHit {
            kind: RingHitKind::Centre,
            id: -1,
        },
        Hit::Reserve => RingHit {
            kind: RingHitKind::Reserve,
            id: -1,
        },
        _ => RingHit {
            kind: RingHitKind::Outside,
            id: -1,
        },
    }
}

/// A drop target for these tests: over a tab it is that tab's slot, anywhere else it is invalid.
fn drop_of(model: &RingModel, tabs: &[mercury_ui::TabRow], x: f32, y: f32) -> RingDropTarget {
    match hit_test(&model.layout, Point::new(x, y), None) {
        Hit::Tab(flat) => {
            let row = &tabs[model.tab_rows[flat]];
            RingDropTarget {
                valid: true,
                group: row.group,
                index: row.index_in_group,
                indicator: ArcSeg::default(),
            }
        }
        _ => RingDropTarget {
            valid: false,
            group: -1,
            index: -1,
            indicator: ArcSeg::default(),
        },
    }
}

fn scene(with_hit_test: bool) -> Scene {
    a11y::init();
    let groups = mockup_groups(false);
    let tabs = mockup_tabs(false);
    let model = Rc::new(RingModel::new(RingSpec::hub(CENTRE), &groups, &tabs));
    let window = RingBandsPreview::new().unwrap();
    window.set_ring_size(516.0);
    window.set_groups(ModelRc::new(VecModel::from(groups)));
    window.set_tabs(ModelRc::new(VecModel::from(
        model
            .tab_rows
            .iter()
            .map(|&row| tabs[row].clone())
            .collect::<Vec<_>>(),
    )));
    window.set_segments(ModelRc::new(VecModel::from(model.segments_for_slint())));
    window.set_label_bands(ModelRc::new(VecModel::from(model.label_bands_for_slint())));

    if with_hit_test {
        let for_hit = Rc::clone(&model);
        window.on_hit_test(move |x, y, hovered| hit_of(&for_hit, x, y, hovered));
        let for_drop = Rc::clone(&model);
        let rows = tabs.clone();
        window.on_drop_target(move |x, y, _| drop_of(&for_drop, &rows, x, y));
    }

    let log: Log = Rc::default();
    let sink = |log: &Log, name: &'static str| {
        let log = Rc::clone(log);
        move |text: String| log.borrow_mut().push(format!("{name}:{text}"))
    };
    let f = sink(&log, "activated");
    window.on_tab_activated(move |id| f(id.to_string()));
    let f = sink(&log, "hovered");
    window.on_tab_hovered(move |id| f(id.to_string()));
    let f = sink(&log, "group");
    window.on_group_activated(move |id| f(id.to_string()));
    let f = sink(&log, "close");
    window.on_tab_close(move |id| f(id.to_string()));
    let f = sink(&log, "drag-started");
    window.on_drag_started(move |id| f(id.to_string()));
    let f = sink(&log, "drag-moved");
    window.on_drag_moved(move |x, y| f(format!("{x:.0},{y:.0}")));
    let f = sink(&log, "drop");
    window.on_drop(move |group, index| f(format!("{group},{index}")));
    let f = sink(&log, "drag-cancelled");
    window.on_drag_cancelled(move || f(String::new()));
    let f = sink(&log, "background");
    window.on_background_pressed(move || f(String::new()));

    a11y::open_window(&window, 516.0, 516.0);
    Scene { window, model, log }
}

impl Scene {
    fn tab_point(&self, flat: usize) -> (f32, f32) {
        let seg = &self.model.segments_for_slint()[flat];
        (seg.mid_x, seg.mid_y)
    }

    fn tab_id(&self, flat: usize) -> i32 {
        self.model.tab_id(flat).unwrap()
    }

    fn polar(&self, radius: f32, deg: f32) -> (f32, f32) {
        let p = point_on_circle(CENTRE, radius, deg);
        (p.x, p.y)
    }

    fn move_to(&self, (x, y): (f32, f32)) {
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(x, y),
            });
    }

    fn press_at(&self, (x, y): (f32, f32), button: PointerEventButton) {
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position: LogicalPosition::new(x, y),
                button,
            });
    }

    fn release_at(&self, (x, y): (f32, f32), button: PointerEventButton) {
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position: LogicalPosition::new(x, y),
                button,
            });
    }

    fn click(&self, at: (f32, f32)) {
        self.move_to(at);
        self.press_at(at, PointerEventButton::Left);
        self.release_at(at, PointerEventButton::Left);
    }

    fn leave(&self) {
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerExited);
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }
}

fn starts(log: &[String], prefix: &str) -> usize {
    log.iter().filter(|entry| entry.starts_with(prefix)).count()
}

#[test]
fn hovering_a_segment_sets_hovered_tab_and_fires_once() {
    let s = scene(true);
    let at = s.tab_point(5);
    s.move_to(at);
    assert_eq!(s.window.get_hovered_tab(), s.tab_id(5));
    assert_eq!(s.take(), [format!("hovered:{}", s.tab_id(5))]);

    s.move_to((at.0 + 1.0, at.1));
    assert!(s.take().is_empty(), "moving inside the same tab is silent");

    let next = s.tab_point(6);
    s.move_to(next);
    assert_eq!(s.window.get_hovered_tab(), s.tab_id(6));
    assert_eq!(s.take(), [format!("hovered:{}", s.tab_id(6))]);
}

#[test]
fn moving_to_the_centre_or_leaving_resets_the_hover() {
    let s = scene(true);
    s.move_to(s.tab_point(3));
    s.take();
    s.move_to((258.0, 258.0));
    assert_eq!(s.window.get_hovered_tab(), -1);
    assert_eq!(s.take(), ["hovered:-1"]);

    s.move_to(s.tab_point(3));
    s.take();
    s.leave();
    assert_eq!(s.window.get_hovered_tab(), -1);
    assert_eq!(s.take(), ["hovered:-1"]);
}

#[test]
fn a_click_on_a_segment_activates_it() {
    let s = scene(true);
    s.click(s.tab_point(8));
    let log = s.take();
    assert!(
        log.contains(&format!("activated:{}", s.tab_id(8))),
        "{log:?}"
    );
    assert_eq!(starts(&log, "activated"), 1);
    assert_eq!(starts(&log, "background"), 0);
    assert_eq!(starts(&log, "drag"), 0);
}

#[test]
fn a_click_on_a_label_band_activates_the_group() {
    let s = scene(true);
    let group = &s.model.layout.groups[1];
    let mid = (group.label_start_deg + group.label_end_deg) / 2.0;
    let at = s.polar(RingSpec::hub(CENTRE).label_radius(), mid);
    s.click(at);
    let log = s.take();
    assert!(
        log.contains(&format!("group:{}", s.model.group_id(1).unwrap())),
        "{log:?}"
    );
    assert_eq!(starts(&log, "activated"), 0);
    assert_eq!(starts(&log, "background"), 0);
}

#[test]
fn a_press_on_a_label_band_that_moves_away_is_no_click_and_no_drag() {
    let s = scene(true);
    let group = &s.model.layout.groups[1];
    let mid = (group.label_start_deg + group.label_end_deg) / 2.0;
    let at = s.polar(RingSpec::hub(CENTRE).label_radius(), mid);
    s.press_at(at, PointerEventButton::Left);
    s.move_to((at.0 + 12.0, at.1));
    s.release_at((at.0 + 12.0, at.1), PointerEventButton::Left);
    let log = s.take();
    assert_eq!(starts(&log, "group"), 0, "{log:?}");
    assert_eq!(starts(&log, "drag"), 0, "{log:?}");
}

#[test]
fn a_middle_click_on_a_segment_closes_it_and_nothing_else() {
    let s = scene(true);
    let at = s.tab_point(2);
    s.move_to(at);
    s.take();
    s.press_at(at, PointerEventButton::Middle);
    s.release_at(at, PointerEventButton::Middle);
    assert_eq!(s.take(), [format!("close:{}", s.tab_id(2))]);
}

#[test]
fn a_press_and_12_px_start_a_drag_that_drops_on_a_valid_slot() {
    let s = scene(true);
    let from = s.tab_point(0);
    let over = s.tab_point(7);
    s.move_to(from);
    s.take();
    s.press_at(from, PointerEventButton::Left);
    s.move_to((from.0 + 3.0, from.1));
    assert!(s.take().is_empty(), "3 px is under the threshold");
    s.move_to((from.0 + 12.0, from.1));
    let log = s.take();
    // The lift of the pressed tab ends when the drag starts (its slot is drawn as the dashed hole).
    assert_eq!(log[0], "hovered:-1", "{log:?}");
    assert_eq!(log[1], format!("drag-started:{}", s.tab_id(0)), "{log:?}");
    assert_eq!(starts(&log, "drag-moved"), 1);

    s.move_to(over);
    s.release_at(over, PointerEventButton::Left);
    let log = s.take();
    let row = &mockup_tabs(false)[7];
    assert_eq!(starts(&log, "drag-moved"), 1);
    let dropped = format!("drop:{},{}", row.group, row.index_in_group);
    assert_eq!(starts(&log, "drop"), 1, "{log:?}");
    assert!(log.contains(&dropped), "{log:?}");
    assert_eq!(starts(&log, "activated"), 0);
    assert_eq!(starts(&log, "drag-cancelled"), 0);
    // After the drop the pointer is over a tab again, so it is hovered again (after the drop).
    let drop_at = log.iter().position(|entry| entry == &dropped).unwrap();
    assert!(
        log[..drop_at]
            .iter()
            .all(|entry| entry.starts_with("drag-moved"))
    );
    assert_eq!(s.window.get_hovered_tab(), s.tab_id(7));
}

#[test]
fn a_drag_released_over_the_centre_is_cancelled() {
    let s = scene(true);
    let from = s.tab_point(0);
    s.move_to(from);
    s.press_at(from, PointerEventButton::Left);
    s.move_to((from.0 + 12.0, from.1));
    s.take();
    s.move_to((258.0, 258.0));
    s.release_at((258.0, 258.0), PointerEventButton::Left);
    let log = s.take();
    assert_eq!(log.last().unwrap(), "drag-cancelled:");
    assert_eq!(starts(&log, "drop"), 0);
    assert_eq!(starts(&log, "activated"), 0);
}

#[test]
fn a_drag_keeps_reporting_moves_after_the_pointer_left_the_box() {
    let s = scene(true);
    let from = s.tab_point(0);
    s.move_to(from);
    s.press_at(from, PointerEventButton::Left);
    s.move_to((from.0 + 12.0, from.1));
    s.take();
    s.window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(600.0, 100.0),
    });
    assert_eq!(s.take(), ["drag-moved:600,100"]);
    s.release_at((600.0, 100.0), PointerEventButton::Left);
    assert_eq!(s.take(), ["drag-cancelled:"]);
}

#[test]
fn a_non_interactive_ring_does_nothing() {
    let s = scene(true);
    s.window.set_interactive(false);
    let at = s.tab_point(4);
    s.move_to(at);
    s.press_at(at, PointerEventButton::Left);
    s.move_to((at.0 + 12.0, at.1));
    s.release_at(at, PointerEventButton::Left);
    s.press_at(at, PointerEventButton::Middle);
    s.release_at(at, PointerEventButton::Middle);
    let corner = (5.0, 5.0);
    s.click(corner);
    assert!(s.take().is_empty());
    assert_eq!(s.window.get_hovered_tab(), -1);
}

mod background {
    use super::*;

    const CORNER: (f32, f32) = (5.0, 5.0);

    fn group_gap(s: &Scene, radius: f32) -> (f32, f32) {
        let groups = &s.model.layout.groups;
        let deg = (groups[0].end_deg + groups[1].start_deg) / 2.0;
        s.polar(radius, deg)
    }

    #[test]
    fn a_press_in_an_empty_corner_fires_it_once_and_nothing_else() {
        let s = scene(true);
        s.move_to(CORNER);
        s.press_at(CORNER, PointerEventButton::Left);
        s.release_at(CORNER, PointerEventButton::Left);
        assert_eq!(s.take(), ["background:"]);
        assert_eq!(s.window.get_hovered_tab(), -1);
    }

    #[test]
    fn a_press_in_a_group_gap_fires_it() {
        let s = scene(true);
        let spec = RingSpec::hub(CENTRE);
        for radius in [spec.radius, spec.label_radius()] {
            let at = group_gap(&s, radius);
            s.click(at);
            assert_eq!(s.take(), ["background:"], "radius {radius}");
        }
        assert_eq!(s.window.get_hovered_tab(), -1);
    }

    #[test]
    fn a_press_on_a_segment_a_label_band_the_centre_or_the_reserve_does_not() {
        let s = scene(true);
        let spec = RingSpec::hub(CENTRE);
        let group = &s.model.layout.groups[2];
        let band = s.polar(
            spec.label_radius(),
            (group.label_start_deg + group.label_end_deg) / 2.0,
        );
        let points = [
            s.tab_point(10),
            band,
            (258.0, 258.0),
            s.polar(60.0, 100.0),
            s.polar(spec.radius, 0.0),
        ];
        for at in points {
            s.click(at);
            let log = s.take();
            assert_eq!(starts(&log, "background"), 0, "{at:?}: {log:?}");
        }
    }

    #[test]
    fn the_reserve_really_is_the_reserve() {
        let s = scene(true);
        let at = s.polar(RingSpec::hub(CENTRE).radius, 0.0);
        let hit = hit_of(&s.model, at.0, at.1, -1);
        assert_eq!(hit.kind, RingHitKind::Reserve);
    }

    #[test]
    fn movement_after_a_background_press_starts_no_drag() {
        let s = scene(true);
        s.move_to(CORNER);
        s.press_at(CORNER, PointerEventButton::Left);
        s.move_to((CORNER.0 + 12.0, CORNER.1));
        s.release_at((CORNER.0 + 12.0, CORNER.1), PointerEventButton::Left);
        assert_eq!(s.take(), ["background:"]);

        let gap = group_gap(&s, 180.0);
        let over = s.tab_point(1);
        s.press_at(gap, PointerEventButton::Left);
        s.move_to(over);
        s.release_at(over, PointerEventButton::Left);
        let log = s.take();
        assert_eq!(log[0], "background:");
        assert_eq!(starts(&log, "drag"), 0, "{log:?}");
        assert_eq!(starts(&log, "activated"), 0, "{log:?}");
        assert_eq!(starts(&log, "drop"), 0, "{log:?}");
    }

    #[test]
    fn a_middle_press_on_empty_space_does_not_fire_it() {
        let s = scene(true);
        s.press_at(CORNER, PointerEventButton::Middle);
        s.release_at(CORNER, PointerEventButton::Middle);
        s.press_at(CORNER, PointerEventButton::Right);
        s.release_at(CORNER, PointerEventButton::Right);
        assert!(s.take().is_empty());
    }

    #[test]
    fn a_non_interactive_ring_does_not_fire_it() {
        let s = scene(true);
        s.window.set_interactive(false);
        s.click(CORNER);
        assert!(s.take().is_empty());
    }

    #[test]
    fn with_no_hit_test_handler_every_press_fires_it() {
        let s = scene(false);
        for at in [CORNER, s.tab_point(3), (258.0, 258.0)] {
            s.click(at);
            assert_eq!(s.take(), ["background:"], "{at:?}");
        }
        assert_eq!(s.window.get_hovered_tab(), -1);
    }
}

mod drag_state {
    use super::*;

    fn start_drag(s: &Scene) -> (f32, f32) {
        let from = s.tab_point(0);
        s.move_to(from);
        s.press_at(from, PointerEventButton::Left);
        s.move_to((from.0 + 12.0, from.1));
        from
    }

    #[test]
    fn drag_source_names_the_tab_the_component_is_dragging() {
        let s = scene(true);
        assert_eq!(s.window.get_drag_source(), -1);
        let from = s.tab_point(0);
        s.move_to(from);
        s.press_at(from, PointerEventButton::Left);
        assert_eq!(s.window.get_drag_source(), -1, "a press is no drag yet");
        s.move_to((from.0 + 3.0, from.1));
        assert_eq!(
            s.window.get_drag_source(),
            -1,
            "3 px is under the threshold"
        );
        s.move_to((from.0 + 12.0, from.1));
        assert_eq!(s.window.get_drag_source(), s.tab_id(0));
        assert_eq!(
            s.window.get_dragging_tab(),
            -1,
            "dragging-tab stays the host's input"
        );

        let over = s.tab_point(7);
        s.move_to(over);
        s.release_at(over, PointerEventButton::Left);
        assert_eq!(s.window.get_drag_source(), -1, "a drop ends it");

        start_drag(&s);
        assert_eq!(s.window.get_drag_source(), s.tab_id(0));
        s.move_to((258.0, 258.0));
        s.release_at((258.0, 258.0), PointerEventButton::Left);
        assert_eq!(
            s.window.get_drag_source(),
            -1,
            "a cancelled release ends it"
        );
    }

    #[test]
    fn cancel_drag_ends_a_drag_without_a_drop() {
        let s = scene(true);
        start_drag(&s);
        s.take();
        assert_eq!(s.window.get_drag_source(), s.tab_id(0));

        s.window.invoke_cancel_drag();
        let log = s.take();
        assert_eq!(log, ["drag-cancelled:"], "{log:?}");
        assert_eq!(s.window.get_drag_source(), -1);

        // The release that follows is nothing: no drop, no second cancel, no click.
        let over = s.tab_point(7);
        s.move_to(over);
        s.release_at(over, PointerEventButton::Left);
        let log = s.take();
        assert_eq!(starts(&log, "drop"), 0, "{log:?}");
        assert_eq!(starts(&log, "drag-cancelled"), 0, "{log:?}");
        assert_eq!(starts(&log, "activated"), 0, "{log:?}");
        assert_eq!(starts(&log, "drag-started"), 0, "{log:?}");
    }

    #[test]
    fn cancel_drag_with_no_drag_is_silent_and_drops_a_pending_press() {
        let s = scene(true);
        s.window.invoke_cancel_drag();
        assert!(s.take().is_empty());

        let at = s.tab_point(4);
        s.move_to(at);
        s.press_at(at, PointerEventButton::Left);
        s.take();
        s.window.invoke_cancel_drag();
        s.release_at(at, PointerEventButton::Left);
        let log = s.take();
        assert_eq!(
            starts(&log, "activated"),
            0,
            "a cancelled press is no click: {log:?}"
        );
        assert_eq!(starts(&log, "drag-cancelled"), 0, "{log:?}");
    }

    #[test]
    fn making_the_ring_inert_cancels_a_live_drag() {
        let s = scene(true);
        start_drag(&s);
        s.take();
        s.window.set_interactive(false);
        // Change handlers run after the property is set, on the next pass of the event loop.
        slint::platform::update_timers_and_animations();
        s.leave();
        assert_eq!(s.take(), ["drag-cancelled:"]);
        assert_eq!(s.window.get_drag_source(), -1);
    }
}
