// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The scenes of the ring-bands lane, shared by `examples/render_ring_bands.rs` (which writes the
//! PNGs) and `tests/ring_bands_render.rs` (which checks pixels): a `RingBandsPreview` filled through
//! the real `RingBandsData` and `RingBandsModels`, with the B2 palette pushed, drawn by Slint's
//! software renderer with system fonts. Included with `#[path]` (a directory module under `tests/`
//! is not a test target).
#![allow(dead_code)]

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use mercury_appearance::resolve::AppearanceSettings;
use mercury_ui::ring_bands::{RingBandsData, RingBandsModels, RingColours, RingOptions, metrics};
use mercury_ui::ring_geometry::{Point, RingSpec};
use mercury_ui::{
    ChromePalette, ChromeTheme, GroupRow, Look, RingBandsPreview, TabRow, apply_look,
};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize, Rgba8Pixel, SharedPixelBuffer};

pub const SCALE: f32 = 2.0;
pub const BOX: f32 = 516.0;
pub const CENTRE: Point = Point::new(258.0, 258.0);

struct HeadlessPlatform;

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

/// Installs the headless software platform (once per thread; a second call is ignored).
pub fn init() {
    let _ = slint::platform::set_platform(Box::new(HeadlessPlatform));
}

pub type Pixels = SharedPixelBuffer<Rgba8Pixel>;

/// What a scene shows besides its rows.
#[derive(Clone, Copy)]
pub struct Params {
    pub dark: bool,
    pub spec: RingSpec,
    pub held: bool,
    pub selected_group: Option<i32>,
    pub flag_size: f32,
}

impl Params {
    pub fn hub(dark: bool) -> Self {
        Self {
            dark,
            spec: RingSpec::hub(CENTRE),
            held: false,
            selected_group: None,
            flag_size: 20.0,
        }
    }
}

pub struct Scene {
    pub window: RingBandsPreview,
    pub data: Rc<RefCell<RingBandsData>>,
    pub models: RingBandsModels,
}

impl Scene {
    pub fn new(groups: &[GroupRow], tabs: &[TabRow], params: Params) -> Self {
        init();
        let window = RingBandsPreview::new().expect("the preview can be created");
        let (look, _) = Look::from_appearance(&AppearanceSettings::default());
        apply_look(&window, &look);
        window.global::<ChromePalette>().set_theme(if params.dark {
            ChromeTheme::Dark
        } else {
            ChromeTheme::Light
        });
        let colours = RingColours::from_palette(&window.global::<ChromePalette>());
        let opts = RingOptions {
            held: params.held,
            selected_group: params.selected_group,
            flag_size: params.flag_size,
            colours,
        };
        let metrics = metrics();
        let data = Rc::new(RefCell::new(RingBandsData::new(
            params.spec,
            groups,
            tabs,
            &opts,
            &*metrics,
        )));
        let models = RingBandsModels::new();
        models.update(&data.borrow());
        window.set_ring_size(BOX);
        window.set_groups(models.groups());
        window.set_tabs(models.tabs());
        window.set_segments(models.segments());
        window.set_lifted_segments(models.lifted_segments());
        window.set_segment_labels(models.segment_labels());
        window.set_label_bands(models.label_bands());
        window.set_label_glyphs(models.label_glyphs());
        window.set_flags(models.flags());
        window.set_animate_spinners(false);
        window.set_held(params.held);
        window.set_selected_group(params.selected_group.unwrap_or(-1));
        let for_hit = Rc::clone(&data);
        window.on_hit_test(move |x, y, hovered| for_hit.borrow().hit(x, y, hovered));
        let for_drop = Rc::clone(&data);
        window.on_drop_target(move |x, y, dragging| for_drop.borrow().drop_target(x, y, dragging));
        window.show().expect("the preview can be shown");
        window
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: SCALE,
            });
        window.window().set_size(PhysicalSize::new(
            (BOX * SCALE) as u32,
            (BOX * SCALE) as u32,
        ));
        Self {
            window,
            data,
            models,
        }
    }

    /// The centre of the shape of the tab with this id, ring-local.
    pub fn mid_of(&self, tab_id: i32) -> (f32, f32) {
        let data = self.data.borrow();
        let seg = data
            .segments
            .iter()
            .find(|seg| seg.tab == tab_id)
            .expect("a segment of that tab");
        (seg.mid_x, seg.mid_y)
    }

    /// Draws the window at scale 2 after letting animations settle.
    pub fn render(&self) -> Pixels {
        self.window
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: SCALE,
            });
        self.window.window().set_size(PhysicalSize::new(
            (BOX * SCALE) as u32,
            (BOX * SCALE) as u32,
        ));
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            self.window.window().request_redraw();
            let _ = self.window.window().take_snapshot();
        }
        self.window
            .window()
            .take_snapshot()
            .expect("the window renders")
    }
}

pub fn save(pixels: &Pixels, path: &Path) {
    image::save_buffer_with_format(
        path,
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .expect("the rendering can be saved");
    println!("wrote {}", path.display());
}

pub fn at(pixels: &Pixels, x: f32, y: f32) -> Rgba8Pixel {
    let (px, py) = ((x * SCALE) as u32, (y * SCALE) as u32);
    pixels.as_slice()[(py * pixels.width() + px) as usize]
}

pub fn rgb(pixel: Rgba8Pixel) -> (u8, u8, u8) {
    (pixel.r, pixel.g, pixel.b)
}
