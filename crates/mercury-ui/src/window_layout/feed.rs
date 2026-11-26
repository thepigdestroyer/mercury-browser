// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The feed (task 4 of the layout-switch plan): the dock and the dial, fed from the rows.

use std::cell::RefCell;
use std::rc::Rc;

use mercury_settings::DialCorner;
use slint::{Color, ComponentHandle, Image, SharedString};

use super::ChromePlan;
use crate::corner_dial::{
    DialFan, DialGeometry, corner_from_slint, corner_to_slint, dial_label, step_focus,
};
use crate::group_dock::{DockData, DockFrame, DockModels, DockOptions};
use crate::ring_bands::{RingBandsData, RingBandsModels, RingColours, RingOptions};
use crate::ring_geometry::{GlyphMetrics, Point};
use crate::{
    DialBridge, DialPlacement, DockBridge, DockHit, GroupRow, MainWindow, RingDropTarget, RingHit,
    TabRow,
};

/// The dial's rest face: the active tab's page and group.
#[derive(Clone, Debug, Default)]
pub struct DialFace {
    pub active_colour: Color,
    pub favicon: Option<Image>,
    pub fallback_letter: SharedString,
    pub loading: bool,
    /// 0..=1; -1 = unknown.
    pub progress: f32,
    pub label: SharedString,
}

/// Everything the overlays are built from.
#[derive(Clone, Debug, Default)]
pub struct WindowRows {
    pub groups: Vec<GroupRow>,
    pub tabs: Vec<TabRow>,
    pub dark: bool,
    /// The window's content width (the dock is centred in it).
    pub container_width: f32,
    /// The page area (the dial's overlay), logical px.
    pub area: (f32, f32),
    pub held: bool,
    pub show_keys: bool,
    pub reduce_motion: bool,
    pub dial: DialFace,
}

/// What the cheap key compares: every input that changes what the overlays show.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    expanded: bool,
    groups: Vec<GroupRow>,
    tabs: Vec<TabRow>,
    dark: bool,
    container_width: f32,
    area: (f32, f32),
    held: bool,
    show_keys: bool,
    reduce_motion: bool,
    dial_label: SharedString,
    dial_loading: bool,
    dial_progress: f32,
}

impl Key {
    fn of(rows: &WindowRows, expanded: bool) -> Key {
        Key {
            expanded,
            groups: rows.groups.clone(),
            tabs: rows.tabs.clone(),
            dark: rows.dark,
            container_width: rows.container_width,
            area: rows.area,
            held: rows.held,
            show_keys: rows.show_keys,
            reduce_motion: rows.reduce_motion,
            dial_label: rows.dial.label.clone(),
            dial_loading: rows.dial.loading,
            dial_progress: rows.dial.progress,
        }
    }
}

/// The state the bridge handlers read: the rows last set, the pointer, the built models.
struct State {
    rows: WindowRows,
    key: Option<Key>,
    pointer: Option<f32>,
    dock: Option<DockData>,
    dock_frame: DockFrame,
    ring: Option<RingBandsData>,
    corner: DialCorner,
    expanded: bool,
    area: (f32, f32),
}

/// Fills the dock and dial bridges once and only on change.
pub struct WindowFeed {
    metrics: Box<dyn GlyphMetrics>,
    dock_models: Rc<DockModels>,
    ring_models: Rc<RingBandsModels>,
    state: Rc<RefCell<State>>,
}

impl WindowFeed {
    /// Sets every model and handler of the bridges once.
    pub fn install(window: &MainWindow, metrics: Box<dyn GlyphMetrics>) -> WindowFeed {
        let dock_models = Rc::new(DockModels::default());
        let ring_models = Rc::new(RingBandsModels::default());
        let state = Rc::new(RefCell::new(State {
            rows: WindowRows::default(),
            key: None,
            pointer: None,
            dock: None,
            dock_frame: DockFrame::default(),
            ring: None,
            corner: DialCorner::default(),
            expanded: false,
            area: (0.0, 0.0),
        }));

        let dock = window.global::<DockBridge>();
        dock.set_pills(dock_models.pills());
        dock.set_slivers(dock_models.slivers());

        let dial = window.global::<DialBridge>();
        dial.set_groups(ring_models.groups());
        dial.set_tabs(ring_models.tabs());
        dial.set_segments(ring_models.segments());
        dial.set_lifted_segments(ring_models.lifted_segments());
        dial.set_segment_labels(ring_models.segment_labels());
        dial.set_label_bands(ring_models.label_bands());
        dial.set_label_glyphs(ring_models.label_glyphs());
        dial.set_flags(ring_models.flags());

        let feed = WindowFeed {
            metrics,
            dock_models,
            ring_models,
            state,
        };
        feed.wire(window);
        feed
    }

    fn wire(&self, window: &MainWindow) {
        let dock = window.global::<DockBridge>();
        let state = Rc::clone(&self.state);
        dock.on_hit(move |x, y| {
            let st = state.borrow();
            match st.dock.as_ref() {
                Some(data) => data.hit(&st.dock_frame, x, y),
                None => DockHit::default(),
            }
        });
        let state = Rc::clone(&self.state);
        let models = Rc::clone(&self.dock_models);
        dock.on_pointer_moved(move |x, _| {
            let mut st = state.borrow_mut();
            st.pointer = Some(x);
            if let Some(data) = st.dock.as_ref() {
                let frame = data.frame(Some(x));
                let capsule = data.capsule();
                models.update(&frame, &capsule);
                st.dock_frame = frame;
            }
        });
        let state = Rc::clone(&self.state);
        let models = Rc::clone(&self.dock_models);
        dock.on_pointer_left(move || {
            let mut st = state.borrow_mut();
            st.pointer = None;
            if let Some(data) = st.dock.as_ref() {
                let frame = data.frame(None);
                let capsule = data.capsule();
                models.update(&frame, &capsule);
                st.dock_frame = frame;
            }
        });

        let dial = window.global::<DialBridge>();
        let state = Rc::clone(&self.state);
        dial.on_hit_test(move |x, y, hovered| match state.borrow().ring.as_ref() {
            Some(data) => data.hit(x, y, hovered),
            None => RingHit::default(),
        });
        let state = Rc::clone(&self.state);
        dial.on_drop_target(move |x, y, dragging| match state.borrow().ring.as_ref() {
            Some(data) => data.drop_target(x, y, dragging),
            None => RingDropTarget::default(),
        });
        dial.on_place(|corner, width, height| {
            let centre = DialGeometry::new(width, height).centre(corner_from_slint(corner));
            DialPlacement {
                x: centre.x,
                y: centre.y,
            }
        });
        dial.on_clamp_drag(|x, y, width, height| {
            let centre = DialGeometry::new(width, height).clamp_drag(Point::new(x, y));
            DialPlacement {
                x: centre.x,
                y: centre.y,
            }
        });
        dial.on_snap_corner(|x, y, width, height, current| {
            let snapped =
                DialGeometry::new(width, height).snap(Point::new(x, y), corner_from_slint(current));
            corner_to_slint(snapped)
        });
        let state = Rc::clone(&self.state);
        dial.on_step_focus(move |focused, step| {
            let ids: Vec<i32> = match state.borrow().ring.as_ref() {
                Some(data) => data.tabs.iter().map(|tab| tab.id).collect(),
                None => Vec::new(),
            };
            step_focus(&ids, focused, step)
        });
    }

    /// Applies a plan (the dial's corner, the dock's first state).
    pub fn set_plan(&self, window: &MainWindow, plan: &ChromePlan) {
        let area = {
            let mut st = self.state.borrow_mut();
            st.corner = plan.dial_corner;
            st.area
        };
        let dial = window.global::<DialBridge>();
        dial.set_corner(corner_to_slint(plan.dial_corner));
        let fits = DialGeometry::new(area.0, area.1).fan_fits(plan.dial_corner);
        dial.set_suppressed(!(plan.show_dial && fits));
    }

    /// Rebuilds what the rows changed; writes nothing when nothing changed.
    pub fn set_rows(&self, window: &MainWindow, rows: &WindowRows) {
        let expanded = window.global::<DialBridge>().get_expanded();
        let key = Key::of(rows, expanded);
        {
            let mut st = self.state.borrow_mut();
            if st.key.as_ref() == Some(&key) {
                return;
            }
            st.key = Some(key);
            st.rows = rows.clone();
            st.area = rows.area;
            st.expanded = expanded;
        }

        let options = DockOptions {
            container_width: rows.container_width,
            held: rows.held,
            show_keys: rows.show_keys,
            current_group: None,
        };
        let data = DockData::new(&rows.groups, &rows.tabs, &options, &*self.metrics);
        let frame = data.frame(self.state.borrow().pointer);
        let capsule = data.capsule();
        self.dock_models.update(&frame, &capsule);

        let dock = window.global::<DockBridge>();
        dock.set_dock_x(frame.dock_x);
        dock.set_dock_width(frame.dock_width);
        dock.set_dock_height(frame.dock_height);
        dock.set_new_tab_x(frame.new_tab_x);
        dock.set_shrink_x(frame.shrink_x);
        dock.set_held(rows.held);
        dock.set_reduce_motion(rows.reduce_motion);
        dock.set_count_text(capsule.count_text.clone());
        dock.set_capsule_label(capsule.label.clone());

        {
            let mut st = self.state.borrow_mut();
            st.dock = Some(data);
            st.dock_frame = frame;
        }

        self.set_dial(window, rows);
    }

    /// The dial's rest face always; its fan only while it is open and shown.
    fn set_dial(&self, window: &MainWindow, rows: &WindowRows) {
        let dial = window.global::<DialBridge>();
        let (corner, expanded) = {
            let st = self.state.borrow();
            (st.corner, st.expanded)
        };
        dial.set_active_colour(rows.dial.active_colour);
        dial.set_favicon(rows.dial.favicon.clone().unwrap_or_default());
        dial.set_has_favicon(rows.dial.favicon.is_some());
        dial.set_fallback_letter(rows.dial.fallback_letter.clone());
        dial.set_loading(rows.dial.loading);
        dial.set_progress(rows.dial.progress);
        dial.set_dial_label(if rows.dial.label.is_empty() {
            dial_label(None, rows.dial.loading, rows.dial.progress)
        } else {
            rows.dial.label.clone()
        });
        dial.set_reduce_motion(rows.reduce_motion);
        dial.set_animate_spinners(!rows.reduce_motion);

        if !expanded {
            self.state.borrow_mut().ring = None;
            return;
        }
        let geometry = DialGeometry::new(rows.area.0, rows.area.1);
        let colours = RingColours::default();
        let opts = RingOptions {
            held: rows.held,
            selected_group: None,
            flag_size: 20.0,
            colours,
        };
        let fan = DialFan::new(
            &geometry,
            corner,
            &rows.groups,
            &rows.tabs,
            &opts,
            &*self.metrics,
        );
        self.ring_models.update(&fan.data);
        dial.set_caption(fan.caption(-1).unwrap_or_default());
        self.state.borrow_mut().ring = Some(fan.data);
    }

    /// The dock was shrunk or expanded (by the user or the desktop).
    pub fn dock_state_changed(&self, window: &MainWindow, shrunk: bool) {
        window.global::<DockBridge>().set_shrunk(shrunk);
    }
}
