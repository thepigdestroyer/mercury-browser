// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What a window does for the group dock, written once for the `group_dock_*` tests: a
//! `GroupDockPreview` whose models, frame and hit test are wired to `group_dock::DockData` the way
//! the host of the real dock will wire them (pointer move -> `frame(Some(x))` -> `update`), a log
//! of every callback, and pointer helpers. Not a test itself.
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use mercury_ui::group_dock::{DockData, DockFrame, DockModels, DockOptions};
use mercury_ui::ring_geometry::ApproxMetrics;
use mercury_ui::{ChromePalette, ChromeTheme, GroupDockPreview, GroupRow, TabRow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, Rgba8Pixel, SharedPixelBuffer};

/// The preview window, like the mockups: 1440 x 900 logical pixels.
pub const WIDTH: f32 = 1440.0;
pub const HEIGHT: f32 = 900.0;

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

/// Slint's software renderer without a display, once per test thread (for the pixel tests).
pub fn init_software() {
    let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
}

/// The dock, its data and everything a callback reported.
pub struct Host {
    pub window: GroupDockPreview,
    pub data: Rc<RefCell<DockData>>,
    pub frame: Rc<RefCell<DockFrame>>,
    pub models: Rc<DockModels>,
    pub log: Rc<RefCell<Vec<String>>>,
    pub groups: Vec<GroupRow>,
    pub tabs: Vec<TabRow>,
}

/// Puts a frame (and its tooltip) on the window's properties and the models.
fn apply(
    window: &GroupDockPreview,
    models: &DockModels,
    data: &DockData,
    frame: &DockFrame,
    pointer: Option<(f32, f32)>,
) {
    models.update(frame, &data.capsule());
    window.set_dock_x(frame.dock_x);
    window.set_dock_width(frame.dock_width);
    window.set_dock_height(frame.dock_height);
    window.set_new_tab_x(frame.new_tab_x);
    window.set_shrink_x(frame.shrink_x);
    window.set_hovered_tab(frame.hovered_tab.unwrap_or(-1));
    let tooltip = pointer.and_then(|(x, y)| data.tooltip_for(frame, x, y));
    match tooltip {
        Some((text, x)) => {
            window.set_tooltip(text);
            window.set_tooltip_x(x);
        }
        None => window.set_tooltip("".into()),
    }
}

impl Host {
    /// A shown, light-themed dock for these rows with no motion (positions equal their targets
    /// at once). The caller has installed a platform.
    pub fn new(groups: Vec<GroupRow>, tabs: Vec<TabRow>, options: DockOptions) -> Host {
        let metrics = ApproxMetrics;
        let data = Rc::new(RefCell::new(DockData::new(
            &groups, &tabs, &options, &metrics,
        )));
        let frame = Rc::new(RefCell::new(data.borrow().frame(None)));
        let models = Rc::new(DockModels::new());
        let log = Rc::new(RefCell::new(Vec::<String>::new()));
        let window = GroupDockPreview::new().expect("the preview window can be made");
        window
            .global::<ChromePalette>()
            .set_theme(ChromeTheme::Light);
        window.set_reduce_motion(true);
        window.set_held(options.held);
        window.set_pills(models.pills());
        window.set_slivers(models.slivers());
        {
            let data = data.borrow();
            let capsule = data.capsule();
            window.set_count_text(capsule.count_text.clone());
            window.set_capsule_label(capsule.label.clone());
            apply(&window, &models, &data, &frame.borrow(), None);
        }

        window.on_hit({
            let data = Rc::clone(&data);
            let frame = Rc::clone(&frame);
            move |x, y| data.borrow().hit(&frame.borrow(), x, y)
        });
        window.on_pointer_moved({
            let weak = window.as_weak();
            let data = Rc::clone(&data);
            let frame = Rc::clone(&frame);
            let models = Rc::clone(&models);
            move |x, y| {
                let Some(window) = weak.upgrade() else { return };
                let next = data.borrow().frame(Some(x));
                apply(&window, &models, &data.borrow(), &next, Some((x, y)));
                *frame.borrow_mut() = next;
            }
        });
        window.on_pointer_left({
            let weak = window.as_weak();
            let data = Rc::clone(&data);
            let frame = Rc::clone(&frame);
            let models = Rc::clone(&models);
            move || {
                let Some(window) = weak.upgrade() else { return };
                let next = data.borrow().frame(None);
                apply(&window, &models, &data.borrow(), &next, None);
                *frame.borrow_mut() = next;
            }
        });
        let record = |name: &'static str, log: &Rc<RefCell<Vec<String>>>| {
            let log = Rc::clone(log);
            move |id: i32| log.borrow_mut().push(format!("{name}:{id}"))
        };
        window.on_pill_activated(record("pill", &log));
        window.on_tab_activated(record("tab", &log));
        window.on_tab_close(record("close", &log));
        window.on_new_tab_in(record("new", &log));
        window.on_pill_peek_requested(record("peek", &log));
        window.on_shrunk_changed({
            let log = Rc::clone(&log);
            move |shrunk| log.borrow_mut().push(format!("shrunk:{shrunk}"))
        });
        window.on_shrink_requested({
            let log = Rc::clone(&log);
            move || log.borrow_mut().push("shrink".into())
        });
        window.on_expand_requested({
            let log = Rc::clone(&log);
            move || log.borrow_mut().push("expand".into())
        });
        Host {
            window,
            data,
            frame,
            models,
            log,
            groups,
            tabs,
        }
    }

    /// Shows the window at 1440 x 900 logical pixels.
    pub fn open(&self) {
        self.window
            .window()
            .set_size(slint::LogicalSize::new(WIDTH, HEIGHT));
        self.window.show().expect("the window can be shown");
        self.tick();
    }

    /// Lets timers and animations run.
    pub fn tick(&self) {
        slint::platform::update_timers_and_animations();
    }

    /// The window y of a y measured in `GroupDock`'s box (what `DockData::hit` takes).
    pub fn window_y(local_y: f32) -> f32 {
        HEIGHT - DockData::container_height() + local_y
    }

    /// The window y of the middle of the dock's pills at rest.
    pub fn mid_y() -> f32 {
        Self::window_y(DockData::container_height() - 12.0 - 26.0)
    }

    /// Takes the log, leaving it empty.
    pub fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    pub fn move_to(&self, x: f32, y: f32) {
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(x, y),
            });
        self.tick();
    }

    pub fn leave(&self) {
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerExited);
        self.tick();
    }

    pub fn click_with(&self, x: f32, y: f32, button: PointerEventButton) {
        let position = LogicalPosition::new(x, y);
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
        self.tick();
    }

    pub fn click(&self, x: f32, y: f32) {
        self.click_with(x, y, PointerEventButton::Left);
    }

    /// The centre of pill `index` in the frame as it is now.
    pub fn pill_centre(&self, index: usize) -> f32 {
        let frame = self.frame.borrow();
        let pill = &frame.pills[index];
        pill.x + pill.width / 2.0
    }

    /// Moves the pointer onto pill `index` (twice: the second move sees the magnified layout,
    /// which must keep the pill under the pointer).
    pub fn settle_on_pill(&self, index: usize) {
        let rest_centre = self.pill_centre(index);
        self.move_to(rest_centre, Self::mid_y());
        let centre = self.pill_centre(index);
        self.move_to(centre, Self::mid_y());
    }
}

/// A snapshot of the preview window at scale factor 2, after the window has settled.
pub fn snapshot(window: &GroupDockPreview) -> SharedPixelBuffer<Rgba8Pixel> {
    snapshot_of(window.window(), WIDTH, HEIGHT)
}

/// A snapshot of any window of `width` x `height` logical pixels at scale factor 2.
pub fn snapshot_of(win: &slint::Window, width: f32, height: f32) -> SharedPixelBuffer<Rgba8Pixel> {
    win.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    win.set_size(PhysicalSize::new(
        (width * 2.0) as u32,
        (height * 2.0) as u32,
    ));
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        win.request_redraw();
        let _ = win.take_snapshot();
    }
    win.take_snapshot()
        .expect("the software renderer can take a snapshot")
}

/// The pixel at a logical position of a 2x snapshot.
pub fn pixel_at(buffer: &SharedPixelBuffer<Rgba8Pixel>, x: f32, y: f32) -> (u8, u8, u8) {
    let px = (x * 2.0) as u32;
    let py = (y * 2.0) as u32;
    let p = buffer.as_slice()[(py * buffer.width() + px) as usize];
    (p.r, p.g, p.b)
}

/// Whether two colours are within `tolerance` in every channel.
pub fn near(a: (u8, u8, u8), b: (u8, u8, u8), tolerance: u8) -> bool {
    a.0.abs_diff(b.0) <= tolerance
        && a.1.abs_diff(b.1) <= tolerance
        && a.2.abs_diff(b.2) <= tolerance
}
