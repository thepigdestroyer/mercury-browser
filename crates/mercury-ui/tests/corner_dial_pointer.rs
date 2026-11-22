// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The corner dial's pointer, keyboard and motion behaviour, through Slint's testing backend:
//! a click toggles the fan, a short move is still a click, a longer one drags the dial and a
//! release snaps it to the corner `DialGeometry::snap` names, Esc and the arrows, the scrim, the
//! motion (fan 180 ms in and 140 ms out, the slide to a corner, the load ring's fade) and whether
//! `transform-scale` keeps pointer input right.
//!
//! The callbacks the component calls are wired to the real Rust functions of
//! `mercury_ui::corner_dial`, as a host does. Tests marked `[RingBands]` need the finished
//! `RingBands` body (and the data of `RingBandsData::new`); until ring-bands lands they fail
//! because the fan draws and hit-tests nothing.
//!
//! Animations are stepped by hand: `Rig::frame` reads the accessibility tree (which evaluates what
//! a frame would), `Rig::advance` moves Slint's mock clock and updates the animations.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::a11y;
use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use mercury_settings::DialCorner;
use mercury_ui::corner_dial::{
    CAPTION_MAX_WIDTH, DialFan, DialGeometry, corner_from_slint, corner_to_slint, step_focus,
};
use mercury_ui::ring_bands::{RingBandsModels, RingOptions, fixtures};
use mercury_ui::ring_geometry::{ApproxMetrics, Point};
use mercury_ui::{CornerDialPreview, DialCaption, DialPlacement, RingHitKind};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};

const AREA: (f32, f32) = (1440.0, 900.0);
const LABEL: &str = "Tab dial, Research";
/// The dial's centre at rest in the bottom-right corner of the 1440 x 900 area (the mockup's).
const BOTTOM_RIGHT: (f32, f32) = (1366.0, 826.0);
/// The top-left corner of the fan's box when the dial is in the bottom-right corner.
const BOX_ORIGIN: (f32, f32) = (1366.0 - 384.0, 826.0 - 384.0);
const ALL_CORNERS: [DialCorner; 4] = [
    DialCorner::TopLeft,
    DialCorner::TopRight,
    DialCorner::BottomLeft,
    DialCorner::BottomRight,
];

/// A `CornerDialPreview` wired the way a host wires it.
struct Rig {
    window: CornerDialPreview,
    fan: Rc<RefCell<DialFan>>,
    /// One line per callback the host heard, in order.
    log: Rc<RefCell<Vec<String>>>,
    /// The ring-local points `hit-test` was asked about.
    hit_points: Rc<RefCell<Vec<(f32, f32)>>>,
    _models: RingBandsModels,
}

impl Rig {
    fn new(corner: DialCorner) -> Self {
        a11y::init();
        let window = CornerDialPreview::new().unwrap();
        let geometry = DialGeometry::new(AREA.0, AREA.1);
        let groups = fixtures::mockup_groups(false);
        let tabs = fixtures::mockup_tabs(false);
        let fan = Rc::new(RefCell::new(DialFan::new(
            &geometry,
            corner,
            &groups,
            &tabs,
            &RingOptions::default(),
            &ApproxMetrics,
        )));
        let models = RingBandsModels::new();
        models.update(&fan.borrow().data);
        window.set_groups(models.groups());
        window.set_tabs(models.tabs());
        window.set_segments(models.segments());
        window.set_lifted_segments(models.lifted_segments());
        window.set_segment_labels(models.segment_labels());
        window.set_label_bands(models.label_bands());
        window.set_label_glyphs(models.label_glyphs());
        window.set_flags(models.flags());
        window.set_active_colour(groups[0].colour);
        window.set_dial_label(LABEL.into());
        window.set_fallback_letter("D".into());
        window.set_corner(corner_to_slint(corner));

        window.on_place(|corner, width, height| {
            let centre = DialGeometry::new(width, height).centre(corner_from_slint(corner));
            DialPlacement {
                x: centre.x,
                y: centre.y,
            }
        });
        window.on_clamp_drag(|x, y, width, height| {
            let centre = DialGeometry::new(width, height).clamp_drag(Point::new(x, y));
            DialPlacement {
                x: centre.x,
                y: centre.y,
            }
        });
        window.on_snap_corner(|x, y, width, height, current| {
            corner_to_slint(
                DialGeometry::new(width, height).snap(Point::new(x, y), corner_from_slint(current)),
            )
        });
        let ids: Vec<i32> = fan.borrow().data.tabs.iter().map(|tab| tab.id).collect();
        window.on_step_focus(move |focused, step| step_focus(&ids, focused, step));
        // The host keeps `focused-tab` in step with the arrow keys (the component cannot assign it).
        let weak = window.as_weak();
        window.on_focus_moved(move |id| {
            if let Some(window) = weak.upgrade() {
                window.set_focused_tab(id);
            }
        });
        let data = Rc::clone(&fan);
        let points = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&points);
        window.on_hit_test(move |x, y, hovered| {
            recorded.borrow_mut().push((x, y));
            data.borrow().data.hit(x, y, hovered)
        });
        let data = Rc::clone(&fan);
        window.on_drop_target(move |x, y, dragging| data.borrow().data.drop_target(x, y, dragging));

        let log = Rc::new(RefCell::new(Vec::new()));
        let push = |log: &Rc<RefCell<Vec<String>>>, line: String| log.borrow_mut().push(line);
        let heard = Rc::clone(&log);
        window.on_toggled(move |open| push(&heard, format!("toggled({open})")));
        let heard = Rc::clone(&log);
        window.on_corner_changed(move |kind| push(&heard, format!("corner-changed({kind:?})")));
        let heard = Rc::clone(&log);
        window.on_close_requested(move || push(&heard, "close-requested".to_string()));
        let heard = Rc::clone(&log);
        window.on_tab_activated(move |id| push(&heard, format!("tab-activated({id})")));
        let heard = Rc::clone(&log);
        window.on_tab_hovered(move |id| push(&heard, format!("tab-hovered({id})")));

        a11y::open_window(&window, AREA.0, AREA.1);
        let rig = Self {
            window,
            fan,
            log,
            hit_points: points,
            _models: models,
        };
        rig.frame();
        rig
    }

    fn events(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    /// What a frame would evaluate: reading the accessibility tree reads every `visible`, and the
    /// position of every element that has a role, so animations start at the current time.
    fn frame(&self) {
        let _ = a11y::reading_order(&self.window);
    }

    fn advance(&self, milliseconds: u64) {
        self.frame();
        mock_elapsed_time(Duration::from_millis(milliseconds));
        slint::platform::update_timers_and_animations();
        self.frame();
    }

    fn dial(&self) -> ElementHandle {
        a11y::control(&self.window, AccessibleRole::Button, LABEL)
    }

    /// The dial's centre as the accessibility tree reports it.
    fn dial_centre(&self) -> (f32, f32) {
        let dial = self.dial();
        let position = dial.absolute_position();
        let size = dial.size();
        (
            position.x + size.width / 2.0,
            position.y + size.height / 2.0,
        )
    }

    /// The opacity of the fan (0 when it is not drawn).
    fn fan_opacity(&self) -> f32 {
        ElementHandle::find_by_element_id(&self.window, "CornerDial::ring")
            .next()
            .map_or(0.0, |ring| ring.computed_opacity())
    }

    fn element(&self, id: &str) -> Option<ElementHandle> {
        ElementHandle::find_by_element_id(&self.window, id).next()
    }

    fn pointer(&self, event: WindowEvent) {
        self.window.window().dispatch_event(event);
    }

    fn move_to(&self, x: f32, y: f32) {
        self.pointer(WindowEvent::PointerMoved {
            position: LogicalPosition::new(x, y),
        });
    }

    fn press(&self, x: f32, y: f32) {
        self.move_to(x, y);
        self.pointer(WindowEvent::PointerPressed {
            position: LogicalPosition::new(x, y),
            button: PointerEventButton::Left,
        });
    }

    fn release(&self, x: f32, y: f32) {
        self.pointer(WindowEvent::PointerReleased {
            position: LogicalPosition::new(x, y),
            button: PointerEventButton::Left,
        });
        self.frame();
    }

    fn click(&self, x: f32, y: f32) {
        self.press(x, y);
        self.release(x, y);
    }

    /// Moves the pointer to (`x`, `y`) in steps of at most 5 px, 16 ms apart.
    fn drag_to(&self, from: (f32, f32), to: (f32, f32)) {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let steps = ((dx.hypot(dy) / 5.0).ceil() as usize).max(1);
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            mock_elapsed_time(Duration::from_millis(16));
            self.move_to(from.0 + dx * t, from.1 + dy * t);
        }
    }

    fn key(&self, key: impl Into<SharedString>) {
        let text = key.into();
        self.pointer(WindowEvent::KeyPressed { text: text.clone() });
        self.pointer(WindowEvent::KeyReleased { text });
        self.frame();
    }

    fn open(&self) {
        let (x, y) = self.dial_centre();
        self.click(x, y);
        self.advance(200);
        assert!(self.window.get_expanded(), "the click did not open the fan");
        self.events();
    }
}

fn corner_event(corner: DialCorner) -> String {
    format!("corner-changed({:?})", corner_to_slint(corner))
}

#[test]
fn a_click_on_the_dial_opens_the_fan_and_a_second_click_closes_it() {
    let rig = Rig::new(DialCorner::BottomRight);
    let (x, y) = rig.dial_centre();
    assert_eq!((x, y), BOTTOM_RIGHT, "the dial rests 74 px from the edges");
    assert!(!rig.window.get_expanded());

    rig.click(x, y);
    assert!(rig.window.get_expanded());
    assert_eq!(rig.events(), ["toggled(true)"]);
    rig.advance(200);
    assert_eq!(rig.fan_opacity(), 1.0);

    rig.click(x, y);
    assert!(!rig.window.get_expanded());
    assert_eq!(rig.events(), ["toggled(false)"]);
    rig.advance(200);
    assert_eq!(rig.fan_opacity(), 0.0);
    assert_eq!(rig.dial_centre(), (x, y), "toggling does not move the dial");
}

#[test]
fn a_press_and_a_five_pixel_move_is_still_a_click() {
    let rig = Rig::new(DialCorner::BottomRight);
    let (x, y) = rig.dial_centre();
    rig.press(x, y);
    rig.drag_to((x, y), (x + 3.0, y + 4.0));
    rig.release(x + 3.0, y + 4.0);
    assert_eq!(rig.events(), ["toggled(true)"]);
    assert!(rig.window.get_expanded());
    assert_eq!(rig.dial_centre(), (x, y), "a click does not drag the dial");
}

#[test]
fn a_press_and_a_forty_pixel_move_drags_the_dial_and_closes_the_fan() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.open();
    let (x, y) = rig.dial_centre();
    let geometry = DialGeometry::new(AREA.0, AREA.1);

    rig.press(x, y);
    rig.drag_to((x, y), (x - 40.0, y - 30.0));
    assert!(!rig.window.get_expanded(), "dragging collapses the fan");
    assert_eq!(rig.events(), ["toggled(false)"]);
    let wanted = geometry.clamp_drag(Point::new(x - 40.0, y - 30.0));
    assert_eq!(rig.dial_centre(), (wanted.x, wanted.y));
    assert_eq!(rig.dial_centre(), (x - 40.0, y - 30.0));
    rig.advance(200);
    assert_eq!(rig.fan_opacity(), 0.0);

    // Far outside the area the dial stays wholly inside it (clamp-drag: 36 px from the edges).
    rig.drag_to((x - 40.0, y - 30.0), (-50.0, -50.0));
    let wanted = geometry.clamp_drag(Point::new(-50.0, -50.0));
    assert_eq!(rig.dial_centre(), (wanted.x, wanted.y));
    assert_eq!(rig.dial_centre(), (36.0, 36.0));

    rig.release(-50.0, -50.0);
    assert_eq!(rig.events(), [corner_event(DialCorner::TopLeft)]);
    assert_eq!(
        corner_from_slint(rig.window.get_corner()),
        DialCorner::TopLeft
    );
    assert!(
        !rig.window.get_expanded(),
        "a drag is not a click: it does not reopen the fan"
    );
    rig.advance(200);
    assert_eq!(rig.dial_centre(), (74.0, 74.0), "it slides to the corner");
}

#[test]
fn releasing_a_drag_in_each_quadrant_snaps_to_what_snap_says() {
    let geometry = DialGeometry::new(AREA.0, AREA.1);
    // One release point inside each quadrant, away from the axes.
    let releases = [
        ((300.0, 200.0), DialCorner::TopLeft),
        ((1100.0, 200.0), DialCorner::TopRight),
        ((300.0, 700.0), DialCorner::BottomLeft),
        ((1100.0, 700.0), DialCorner::BottomRight),
    ];
    for start in ALL_CORNERS {
        for (release, quadrant) in releases {
            let rig = Rig::new(start);
            let from = rig.dial_centre();
            let rest = geometry.centre(start);
            assert_eq!(from, (rest.x, rest.y), "{start:?} rests where centre says");

            rig.press(from.0, from.1);
            rig.drag_to(from, release);
            rig.release(release.0, release.1);

            let dragged_to = geometry.clamp_drag(Point::new(release.0, release.1));
            let expected = geometry.snap(dragged_to, start);
            assert_eq!(
                expected, quadrant,
                "the oracle for {release:?} from {start:?}"
            );
            let wanted_events = if expected == start {
                Vec::new()
            } else {
                vec![corner_event(expected)]
            };
            assert_eq!(
                rig.events(),
                wanted_events,
                "release at {release:?} from {start:?}"
            );
            assert_eq!(corner_from_slint(rig.window.get_corner()), expected);
            assert!(!rig.window.get_expanded());

            rig.advance(200);
            let settled = geometry.centre(expected);
            assert_eq!(
                rig.dial_centre(),
                (settled.x, settled.y),
                "the dial rests in {expected:?} after {release:?} from {start:?}"
            );
        }
    }
}

#[test]
fn a_drag_that_ends_where_it_started_changes_nothing() {
    let rig = Rig::new(DialCorner::BottomRight);
    let (x, y) = rig.dial_centre();
    rig.press(x, y);
    rig.drag_to((x, y), (x - 120.0, y - 90.0));
    rig.drag_to((x - 120.0, y - 90.0), (x, y));
    rig.release(x, y);
    assert!(rig.events().is_empty(), "no corner change and no toggle");
    assert!(!rig.window.get_expanded());
    rig.advance(200);
    assert_eq!(rig.dial_centre(), (x, y));
}

#[test]
fn a_press_that_grabs_the_dial_off_centre_does_not_make_it_jump() {
    let rig = Rig::new(DialCorner::BottomRight);
    let (x, y) = rig.dial_centre();
    let grab = (x + 20.0, y - 10.0);
    rig.press(grab.0, grab.1);
    rig.drag_to(grab, (grab.0 - 100.0, grab.1));
    assert_eq!(
        rig.dial_centre(),
        (x - 100.0, y),
        "the dial keeps the grab offset"
    );
    rig.release(grab.0 - 100.0, grab.1);
}

#[test]
fn escape_closes_the_fan_and_says_so() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.open();
    rig.key(Key::Escape);
    assert!(!rig.window.get_expanded());
    assert_eq!(rig.events(), ["toggled(false)", "close-requested"]);
    rig.advance(200);
    assert_eq!(rig.fan_opacity(), 0.0);

    // With the fan closed Escape is not the dial's.
    rig.key(Key::Escape);
    assert!(rig.events().is_empty());
}

#[test]
fn space_and_enter_toggle_the_fan_and_a_held_key_does_not_repeat_it() {
    let rig = Rig::new(DialCorner::BottomRight);
    // A click gives the dial the keyboard focus; Escape puts the fan away again.
    rig.open();
    rig.key(Key::Escape);
    rig.events();

    rig.key(" ");
    assert!(rig.window.get_expanded());
    assert_eq!(rig.events(), ["toggled(true)"]);
    rig.key(Key::Return);
    assert!(!rig.window.get_expanded());
    assert_eq!(rig.events(), ["toggled(false)"]);

    // A key held down sends the press once and then repeats of it.
    rig.pointer(WindowEvent::KeyPressed { text: " ".into() });
    rig.pointer(WindowEvent::KeyPressRepeated { text: " ".into() });
    rig.pointer(WindowEvent::KeyPressRepeated { text: " ".into() });
    rig.pointer(WindowEvent::KeyReleased { text: " ".into() });
    assert_eq!(
        rig.events(),
        ["toggled(true)"],
        "the repeats are not more toggles"
    );
}

#[test]
fn keys_with_a_command_modifier_are_not_the_dials() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.open();
    rig.key(Key::Escape);
    rig.events();
    rig.pointer(WindowEvent::KeyPressed {
        text: Key::Meta.into(),
    });
    rig.key(" ");
    rig.pointer(WindowEvent::KeyReleased {
        text: Key::Meta.into(),
    });
    assert!(
        rig.events().is_empty(),
        "⌘ Space belongs to the system, not to the dial"
    );
}

#[test]
fn arrow_keys_move_focused_tab_through_step_focus_while_the_fan_is_open() {
    let rig = Rig::new(DialCorner::BottomRight);
    let ids: Vec<i32> = rig.fan.borrow().data.tabs.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), 22, "the mockup fixture has 22 tabs");

    // Closed: the arrows do nothing. (Focus the dial with a click and put the fan away.)
    rig.open();
    rig.key(Key::Escape);
    rig.events();
    rig.key(Key::RightArrow);
    assert_eq!(rig.window.get_focused_tab(), -1);

    rig.key(" ");
    rig.advance(200);
    assert!(rig.window.get_expanded());
    rig.events();

    // Right and Down step forward, Left and Up back, through the same oracle the host uses.
    let steps = [
        (Key::RightArrow, 1),
        (Key::DownArrow, 1),
        (Key::LeftArrow, -1),
        (Key::UpArrow, -1),
        (Key::UpArrow, -1),
        (Key::LeftArrow, -1),
        (Key::RightArrow, 1),
    ];
    let mut focused = -1;
    for (key, step) in steps {
        rig.key(key);
        focused = step_focus(&ids, focused, step);
        assert_eq!(rig.window.get_focused_tab(), focused, "after {step:+} step");
    }
    // The first step from "none" is the first tab in ring order, the one before it the last.
    rig.window.set_focused_tab(-1);
    rig.advance(1);
    rig.key(Key::RightArrow);
    assert_eq!(rig.window.get_focused_tab(), 101);
    rig.key(Key::LeftArrow);
    assert_eq!(rig.window.get_focused_tab(), 122, "wraps to the last tab");

    // Enter on a focused tab activates it instead of closing the fan.
    rig.events();
    rig.key(Key::Return);
    assert_eq!(rig.events(), ["tab-activated(122)"]);
    assert!(rig.window.get_expanded());
}

#[test]
fn suppressing_the_dial_while_the_fan_is_open_collapses_it_and_stops_the_pointer() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.open();
    rig.window.set_suppressed(true);
    rig.advance(10);
    assert!(
        !rig.window.get_expanded(),
        "the component collapses the fan itself"
    );
    assert_eq!(rig.fan_opacity(), 0.0, "the fan is not drawn at once");
    assert!(
        a11y::reading_order(&rig.window).is_empty(),
        "nothing is left to read"
    );

    let (x, y) = BOTTOM_RIGHT;
    rig.click(x, y);
    rig.click(400.0, 300.0);
    assert!(rig.events().is_empty(), "a click does nothing");
    assert!(!rig.window.get_expanded());

    // Asking for the fan while suppressed draws nothing either.
    rig.window.set_expanded(true);
    rig.advance(200);
    assert_eq!(rig.fan_opacity(), 0.0);

    rig.window.set_suppressed(false);
    rig.window.set_expanded(false);
    rig.advance(200);
    assert_eq!(rig.dial_centre(), (x, y), "the dial is back where it was");
    rig.click(x, y);
    assert_eq!(rig.events(), ["toggled(true)"]);
}

#[test]
fn a_press_outside_the_fan_box_closes_it_through_the_scrim() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.open();
    // Left of and above the box (982, 442): only the scrim is there.
    rig.click(400.0, 300.0);
    assert!(!rig.window.get_expanded());
    assert_eq!(rig.events(), ["toggled(false)", "close-requested"]);
    rig.advance(200);

    // The scrim is gone with the fan: a press there is the page's.
    rig.click(400.0, 300.0);
    assert!(rig.events().is_empty());
}

#[test]
fn the_closed_fan_takes_no_pointer_input() {
    let rig = Rig::new(DialCorner::BottomRight);
    let segment = rig.fan.borrow().data.segments.first().cloned();
    // [RingBands] needs the real data for a point on a segment; without it the corner of the box does.
    let (x, y) = segment.map_or((BOX_ORIGIN.0 + 20.0, BOX_ORIGIN.1 + 20.0), |s| {
        (BOX_ORIGIN.0 + s.mid_x, BOX_ORIGIN.1 + s.mid_y)
    });
    rig.click(x, y);
    rig.move_to(x + 3.0, y + 3.0);
    assert!(rig.events().is_empty());
    assert!(rig.hit_points.borrow().is_empty(), "the ring was not asked");
    assert!(!rig.window.get_expanded());
}

/// [RingBands]
#[test]
fn with_the_fan_open_a_move_over_a_segment_hovers_it_and_a_click_activates_it() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.open();
    let (tabs, segments) = {
        let fan = rig.fan.borrow();
        (fan.data.tabs.clone(), fan.data.segments.clone())
    };
    assert_eq!(segments.len(), 22);
    for index in [0, 7, 21] {
        let segment = &segments[index];
        let at = (BOX_ORIGIN.0 + segment.mid_x, BOX_ORIGIN.1 + segment.mid_y);
        let id = tabs[index].id;
        rig.move_to(at.0, at.1);
        assert_eq!(
            rig.window.get_hovered_tab(),
            id,
            "hover over segment {index}"
        );
        assert!(rig.events().contains(&format!("tab-hovered({id})")));
        rig.events();
        rig.click(at.0, at.1);
        assert_eq!(rig.events(), [format!("tab-activated({id})")]);
    }

    // Empty page: nothing hovered.
    rig.move_to(500.0, 300.0);
    assert_eq!(rig.window.get_hovered_tab(), -1);
}

/// [RingBands] The caption shows while a tab is hovered, wherever the host puts it.
#[test]
fn the_caption_follows_the_hovered_tab() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.open();
    rig.window.set_caption(DialCaption {
        text: "Reading · Essay on maps".into(),
        x: 500.0,
        y: 400.0,
        max_width: 260.0,
        ..Default::default()
    });
    // No tab hovered: no caption.
    assert!(rig.element("CornerDial::caption-box").is_none());
    let segment = rig.fan.borrow().data.segments[9].clone();
    rig.move_to(BOX_ORIGIN.0 + segment.mid_x, BOX_ORIGIN.1 + segment.mid_y);
    let caption = rig
        .element("CornerDial::caption-box")
        .expect("the caption is drawn while a tab is hovered");
    let position = caption.absolute_position();
    assert_eq!(
        (position.x, position.y),
        (BOX_ORIGIN.0 + 500.0, BOX_ORIGIN.1 + 400.0),
        "its position is fan-box-local"
    );
    assert_eq!(caption.size().height, 30.0);
    rig.move_to(500.0, 300.0);
    assert!(rig.element("CornerDial::caption-box").is_none());
}

/// [RingBands] R1: a press on empty space inside the ring's box closes the fan.
#[test]
fn a_press_on_empty_space_inside_the_box_closes_the_fan() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.open();
    // The box's top-left corner is 517 px from the dial: far outside the 380 px fan.
    let at = (BOX_ORIGIN.0 + 18.0, BOX_ORIGIN.1 + 18.0);
    assert_eq!(
        rig.fan.borrow().data.hit(18.0, 18.0, -1).kind,
        RingHitKind::Outside
    );
    rig.click(at.0, at.1);
    assert!(
        !rig.window.get_expanded(),
        "R1: RingBands.background-pressed reaches close-requested"
    );
    assert_eq!(rig.events(), ["toggled(false)", "close-requested"]);
}

/// [RingBands] D6: `transform-scale` on the fan keeps pointer input right: while it opens, a
/// point of the window reaches the ring as the point of the unscaled box that is drawn there.
#[test]
fn pointer_input_follows_the_scale_of_the_opening_fan() {
    let rig = Rig::new(DialCorner::BottomRight);
    let (x, y) = rig.dial_centre();
    rig.click(x, y);
    rig.advance(60);
    let opacity = rig.fan_opacity();
    assert!(
        opacity > 0.0 && opacity < 1.0,
        "still opening, opacity {opacity}"
    );

    let (dx, dy) = (-200.0, -100.0);
    rig.hit_points.borrow_mut().clear();
    rig.move_to(x + dx, y + dy);
    let (hx, hy) = *rig
        .hit_points
        .borrow()
        .last()
        .expect("the opening fan is asked about the pointer");
    let (ratio_x, ratio_y) = ((hx - 384.0) / dx, (hy - 384.0) / dy);
    // The box is scaled by s in 0.92..1 about its centre, so a window offset d is the ring offset d / s.
    assert!(
        ratio_x > 1.0001 && ratio_x < 1.0 / 0.92 + 0.0001,
        "x ratio {ratio_x}"
    );
    assert!(
        (ratio_x - ratio_y).abs() < 0.0005,
        "x ratio {ratio_x}, y ratio {ratio_y}"
    );

    rig.advance(200);
    rig.hit_points.borrow_mut().clear();
    rig.move_to(x + dx, y + dy + 1.0);
    let (hx, hy) = *rig.hit_points.borrow().last().unwrap();
    assert!((hx - (384.0 + dx)).abs() < 0.01 && (hy - (384.0 + dy + 1.0)).abs() < 0.01);
}

slint::slint! {
    export component ScaleProbe inherits Window {
        width: 200px;
        height: 200px;
        in property <float> factor: 0.5;
        callback hit();
        Rectangle {
            x: 50px;
            y: 50px;
            width: 100px;
            height: 100px;
            transform-scale: root.factor;
            TouchArea {
                clicked => {
                    root.hit();
                }
            }
        }
    }
}

/// D6 on its own, with no ring: Slint maps the pointer through `transform-scale` (the testing
/// backend, like the femtovg renderer the browser uses, supports transformations; the software
/// renderer does not draw them and then does not map them either).
#[test]
fn transform_scale_maps_the_pointer_into_the_scaled_element() {
    a11y::init();
    let probe = ScaleProbe::new().unwrap();
    a11y::open_window(&probe, 200.0, 200.0);
    let hits = Rc::new(RefCell::new(0));
    let counted = Rc::clone(&hits);
    probe.on_hit(move || *counted.borrow_mut() += 1);
    let click = |x: f32, y: f32| {
        let position = LogicalPosition::new(x, y);
        let window = probe.window();
        window.dispatch_event(WindowEvent::PointerMoved { position });
        window.dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        window.dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    };
    // Scaled by 0.5 about its centre (100, 100), the 50..150 box is drawn at 75..125.
    click(55.0, 100.0);
    assert_eq!(
        *hits.borrow(),
        0,
        "inside the unscaled box, outside the drawn one"
    );
    click(80.0, 100.0);
    assert_eq!(*hits.borrow(), 1, "inside the drawn box");
    probe.set_factor(1.0);
    click(55.0, 100.0);
    assert_eq!(*hits.borrow(), 2, "unscaled again");
}

#[test]
fn the_fan_opens_in_180_ms_and_closes_in_140_ms() {
    let rig = Rig::new(DialCorner::BottomRight);
    let (x, y) = rig.dial_centre();
    assert_eq!(rig.fan_opacity(), 0.0);

    rig.click(x, y);
    assert_eq!(rig.fan_opacity(), 0.0, "nothing has elapsed yet");
    rig.advance(90);
    let half = rig.fan_opacity();
    assert!(half > 0.3 && half < 1.0, "half way, opacity {half}");
    rig.advance(80);
    assert!(rig.fan_opacity() < 1.0, "170 ms: not there yet");
    rig.advance(20);
    assert_eq!(rig.fan_opacity(), 1.0, "190 ms: open");

    rig.click(x, y);
    rig.advance(70);
    let half = rig.fan_opacity();
    assert!(half > 0.0 && half < 0.8, "half way down, opacity {half}");
    rig.advance(60);
    assert!(rig.fan_opacity() > 0.0, "130 ms: still closing");
    rig.advance(20);
    assert_eq!(rig.fan_opacity(), 0.0, "150 ms: closed");
}

#[test]
fn with_reduce_motion_the_fan_and_the_slide_take_no_time() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.window.set_reduce_motion(true);
    rig.frame();
    let (x, y) = rig.dial_centre();
    rig.click(x, y);
    assert_eq!(rig.fan_opacity(), 1.0, "open at once");
    rig.click(x, y);
    assert_eq!(rig.fan_opacity(), 0.0, "closed at once");

    rig.press(x, y);
    rig.drag_to((x, y), (300.0, 200.0));
    rig.release(300.0, 200.0);
    assert_eq!(rig.dial_centre(), (74.0, 74.0), "in the corner at once");
}

#[test]
fn the_dial_slides_to_its_corner_in_180_ms() {
    let rig = Rig::new(DialCorner::BottomRight);
    let (x, y) = rig.dial_centre();
    rig.press(x, y);
    rig.drag_to((x, y), (300.0, 200.0));
    assert_eq!(
        rig.dial_centre(),
        (300.0, 200.0),
        "a drag follows the pointer without delay"
    );
    rig.release(300.0, 200.0);
    rig.advance(90);
    let (mx, my) = rig.dial_centre();
    assert!(
        mx > 74.0 && mx < 300.0 && my > 74.0 && my < 200.0,
        "half way: ({mx}, {my})"
    );
    rig.advance(100);
    assert_eq!(rig.dial_centre(), (74.0, 74.0));
}

#[test]
fn the_host_can_move_the_dial_by_setting_the_corner() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.window
        .set_corner(corner_to_slint(DialCorner::BottomLeft));
    rig.advance(200);
    assert_eq!(rig.dial_centre(), (74.0, 826.0));
    assert!(
        rig.events().is_empty(),
        "the host's own change is not reported back"
    );
}

#[test]
fn the_load_ring_shows_the_progress_and_ends_with_a_fade() {
    let rig = Rig::new(DialCorner::BottomRight);
    let arc = || rig.element("CornerDial::arc");
    let full = || {
        rig.element("CornerDial::full-ring")
            .map(|ring| ring.computed_opacity())
    };
    assert!(
        arc().is_none() && full().is_none(),
        "at rest: the track only"
    );

    rig.window.set_loading(true);
    rig.window.set_progress(0.0);
    rig.advance(200);
    rig.window.set_progress(0.65);
    rig.advance(200);
    assert!(arc().is_some(), "a 65 % arc");
    assert!(full().is_none());

    // Unknown progress: an arc too (the quarter that turns).
    rig.window.set_progress(-1.0);
    rig.advance(200);
    assert!(arc().is_some());
    rig.window.set_progress(1.0);
    rig.advance(200);
    assert!(arc().is_none(), "at 100 % the ring is whole");
    assert_eq!(full(), Some(1.0));

    // The load ends: the whole ring is there at once and is gone 400 ms later.
    rig.window.set_loading(false);
    rig.frame();
    assert_eq!(full(), Some(1.0), "fully drawn when the load ends");
    rig.advance(200);
    let half = full().expect("still fading at 200 ms");
    assert!(half > 0.2 && half < 0.8, "half way, opacity {half}");
    rig.advance(220);
    assert_eq!(full(), None, "gone after 400 ms");
    assert!(arc().is_none());

    // The next load starts from the beginning, with no leftover of the last one.
    rig.window.set_loading(true);
    rig.window.set_progress(0.2);
    rig.advance(200);
    assert!(arc().is_some());
    assert!(full().is_none());
}

#[test]
fn with_reduce_motion_the_load_ring_does_not_fade() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.window.set_reduce_motion(true);
    rig.window.set_loading(true);
    rig.window.set_progress(1.0);
    rig.advance(10);
    rig.window.set_loading(false);
    rig.frame();
    assert!(rig.element("CornerDial::full-ring").is_none(), "no fade");
}

/// A fan the host opened (⌘⇧E sets `expanded`) takes the keyboard focus, so Escape and the arrows
/// work without a click.
#[test]
fn a_fan_opened_by_the_host_takes_the_keyboard_focus() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.window.set_expanded(true);
    rig.advance(200);
    assert!(
        rig.events().is_empty(),
        "the host's own change is not reported back"
    );

    rig.key(Key::RightArrow);
    assert_eq!(
        rig.window.get_focused_tab(),
        101,
        "the first arrow reaches the dial"
    );
    rig.key(Key::Escape);
    assert!(!rig.window.get_expanded());
    assert_eq!(rig.events(), ["toggled(false)", "close-requested"]);
}

/// The caption also shows the tab the keys have focused (the host sets it on `focus-moved`).
#[test]
fn the_caption_shows_for_the_keyboard_focused_tab_too() {
    let rig = Rig::new(DialCorner::BottomRight);
    rig.window.set_expanded(true);
    rig.advance(200);
    let caption = rig.fan.borrow().caption(101).expect("caption");
    rig.window.set_caption(caption);
    assert_eq!(rig.window.get_hovered_tab(), -1);
    assert!(
        rig.element("CornerDial::caption-box").is_none(),
        "neither hovered nor focused: no caption"
    );
    rig.key(Key::RightArrow);
    assert_eq!(rig.window.get_focused_tab(), 101);
    assert!(rig.element("CornerDial::caption-box").is_some());
}

/// A title of any length: the box is at most `CAPTION_MAX_WIDTH` wide, its text is elided, and it lies
/// inside the window and clear of the dial, for every tab of the fan in all four corners.
#[test]
fn a_long_title_never_overflows_the_window_or_covers_the_dial() {
    let title = "A page title that is far too long for any caption, ".repeat(3);
    let mut tabs = fixtures::mockup_tabs(false);
    for tab in &mut tabs {
        tab.title = title.as_str().into();
    }
    for corner in ALL_CORNERS {
        let rig = Rig::new(corner);
        *rig.fan.borrow_mut() = DialFan::new(
            &DialGeometry::new(AREA.0, AREA.1),
            corner,
            &fixtures::mockup_groups(false),
            &tabs,
            &RingOptions::default(),
            &ApproxMetrics,
        );
        rig.window.set_expanded(true);
        rig.advance(200);
        let (dx, dy) = rig.dial_centre();
        let ids: Vec<i32> = rig.fan.borrow().data.tabs.iter().map(|t| t.id).collect();
        for id in ids {
            let caption = rig.fan.borrow().caption(id).expect("caption");
            rig.window.set_caption(caption);
            rig.window.set_hovered_tab(id);
            rig.advance(1);
            let handle = rig
                .element("CornerDial::caption-box")
                .expect("the caption is drawn");
            let (position, size) = (handle.absolute_position(), handle.size());
            let what = format!("{corner:?} tab {id}: at {position:?} size {size:?}");
            assert!(size.width <= CAPTION_MAX_WIDTH + 0.01, "{what}");
            assert!(
                size.width > CAPTION_MAX_WIDTH - 0.01,
                "a long title fills the widest box: {what}"
            );
            assert!(position.x >= 0.0 && position.y >= 0.0, "{what}");
            assert!(position.x + size.width <= AREA.0, "{what}");
            assert!(position.y + size.height <= AREA.1, "{what}");
            let over_x = position.x < dx + 38.5 && position.x + size.width > dx - 38.5;
            let over_y = position.y < dy + 38.5 && position.y + size.height > dy - 38.5;
            assert!(!(over_x && over_y), "covers the dial: {what}");
        }
    }
}

/// A short title gives a narrow box, against the near edge of the widest box on either side.
#[test]
fn a_short_title_gives_a_narrow_box_against_the_side_nearest_the_dial() {
    for corner in ALL_CORNERS {
        let rig = Rig::new(corner);
        rig.window.set_expanded(true);
        rig.advance(200);
        let ids: Vec<i32> = rig.fan.borrow().data.tabs.iter().map(|t| t.id).collect();
        for id in ids {
            let caption = rig.fan.borrow().caption(id).expect("caption");
            let (x, right) = (caption.x, caption.right);
            let origin = rig.fan.borrow().origin;
            rig.window.set_caption(caption);
            rig.window.set_hovered_tab(id);
            rig.advance(1);
            let handle = rig.element("CornerDial::caption-box").expect("drawn");
            let (position, size) = (handle.absolute_position(), handle.size());
            assert!(
                size.width < CAPTION_MAX_WIDTH,
                "{corner:?} tab {id}: {size:?}"
            );
            let widest_left = origin.x + x;
            let edge = if right {
                position.x + size.width - (widest_left + CAPTION_MAX_WIDTH)
            } else {
                position.x - widest_left
            };
            assert!(edge.abs() < 0.01, "{corner:?} tab {id}: {edge}");
        }
    }
}
