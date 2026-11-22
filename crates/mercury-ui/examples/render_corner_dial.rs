// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the optional corner dial (`CornerDial`, SPEC §4.5) to PNG files without a display, through
//! Slint's software renderer, at 1440 x 900 and scale 1 so the pictures can be laid beside the mockups
//! `design/ui-final/dial/dial-{rest,expanded}-{light,dark}.png`: at rest and fanned out in all four
//! corners, with the page load at 65 % and fading out, without a favicon, being dragged between
//! corners, and the 64 group and 3 tab fans; light and dark.
//!
//! The dial and its fan are the real component filled through the real Rust functions of
//! `mercury_ui::corner_dial` (the way a host wires them) and the real `RingBandsData`. Every scene
//! runs with `reduce-motion` (so the fan, the slide and the load ring are in their final state at
//! once) and `animate-spinners: false`, except `fading`, which lets the ring's 400 ms fade run in
//! real time and takes the picture part of the way through it.
//!
//! The software renderer draws no transforms (`SoftwareRenderer::supports_transformations` is
//! false): the curved labels of the fan are drawn upright and the fan's 0.92 to 1 opening scale is
//! not drawn. Both are right in the browser's own renderer (see `lane-ring-bands-report.md`).
//!
//! `cargo run --release -p mercury-ui --features test-support --example render_corner_dial -- <output directory>`
//!
//! `tests/corner_dial_render.rs` includes this file as a module and checks pixels of the same scenes.

use std::cell::RefCell;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use mercury_appearance::resolve::AppearanceSettings;
use mercury_settings::DialCorner;
use mercury_ui::corner_dial::{
    DialFan, DialGeometry, corner_from_slint, corner_to_slint, dial_label, step_focus,
};
use mercury_ui::ring_bands::fixtures::{generated, mockup_groups, mockup_tabs};
use mercury_ui::ring_bands::{RingBandsModels, RingColours, RingOptions, metrics};
use mercury_ui::ring_geometry::Point;
use mercury_ui::{
    ChromePalette, ChromeTheme, CornerDialPreview, DialPlacement, GroupRow, Look, TabRow,
    apply_look,
};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, Image, LogicalPosition, PhysicalSize, Rgba8Pixel, SharedPixelBuffer};

pub type Res<T> = Result<T, Box<dyn Error>>;
pub type Pixels = SharedPixelBuffer<Rgba8Pixel>;

/// The window of the mockups.
pub const WIDTH: u32 = 1440;
pub const HEIGHT: u32 = 900;
/// "Essay on maps" in the group Reading, the hovered tab of the mockup's `dial-expanded`.
pub const ESSAY: i32 = 111;
pub const CORNERS: [DialCorner; 4] = [
    DialCorner::BottomRight,
    DialCorner::BottomLeft,
    DialCorner::TopLeft,
    DialCorner::TopRight,
];

struct HeadlessPlatform;

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

/// Installs the headless software platform (once per process; a second call is ignored).
pub fn init() {
    let _ = slint::platform::set_platform(Box::new(HeadlessPlatform));
}

/// The corner as the file names spell it.
pub fn slug(corner: DialCorner) -> &'static str {
    match corner {
        DialCorner::TopLeft => "top-left",
        DialCorner::TopRight => "top-right",
        DialCorner::BottomLeft => "bottom-left",
        DialCorner::BottomRight => "bottom-right",
    }
}

/// What a scene shows besides its rows.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub dark: bool,
    pub corner: DialCorner,
    pub expanded: bool,
    /// A tab id the pointer is over: it is lifted and the caption is shown.
    pub hovered: Option<i32>,
    pub loading: bool,
    pub progress: f32,
    pub favicon: bool,
    /// Durations of 0, so that a picture is taken in the final state. `false` only for `fading`.
    pub reduce_motion: bool,
    pub width: u32,
    pub height: u32,
}

impl Params {
    pub fn rest(dark: bool, corner: DialCorner) -> Self {
        Self {
            dark,
            corner,
            expanded: false,
            hovered: None,
            loading: false,
            progress: 0.0,
            favicon: true,
            reduce_motion: true,
            width: WIDTH,
            height: HEIGHT,
        }
    }

    pub fn fanned(dark: bool, corner: DialCorner) -> Self {
        Self {
            expanded: true,
            ..Self::rest(dark, corner)
        }
    }
}

pub struct Scene {
    pub window: CornerDialPreview,
    pub fan: Rc<RefCell<DialFan>>,
    pub geometry: DialGeometry,
    pub corner: DialCorner,
    pub width: u32,
    pub height: u32,
    group_name: String,
    _models: RingBandsModels,
}

impl Scene {
    pub fn new(groups: &[GroupRow], tabs: &[TabRow], params: &Params) -> Res<Self> {
        init();
        let first = groups.first().ok_or("a scene needs at least one group")?;
        let window = CornerDialPreview::new()?;
        let (look, _) = Look::from_appearance(&AppearanceSettings::default());
        apply_look(&window, &look);
        window.global::<ChromePalette>().set_theme(if params.dark {
            ChromeTheme::Dark
        } else {
            ChromeTheme::Light
        });
        let opts = RingOptions {
            colours: RingColours::from_palette(&window.global::<ChromePalette>()),
            ..RingOptions::default()
        };
        let geometry = DialGeometry::new(params.width as f32, params.height as f32);
        let fan = Rc::new(RefCell::new(DialFan::new(
            &geometry,
            params.corner,
            groups,
            tabs,
            &opts,
            &*metrics(),
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
        window.set_animate_spinners(false);
        window.set_reduce_motion(params.reduce_motion);
        window.set_active_colour(first.colour);
        window.set_fallback_letter("D".into());
        window.set_corner(corner_to_slint(params.corner));

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
        let data = Rc::clone(&fan);
        window.on_hit_test(move |x, y, hovered| data.borrow().data.hit(x, y, hovered));
        let data = Rc::clone(&fan);
        window.on_drop_target(move |x, y, dragging| data.borrow().data.drop_target(x, y, dragging));

        window.show()?;
        window
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
        window
            .window()
            .set_size(PhysicalSize::new(params.width, params.height));

        let scene = Self {
            window,
            fan,
            geometry,
            corner: params.corner,
            width: params.width,
            height: params.height,
            group_name: first.name.to_string(),
            _models: models,
        };
        if params.favicon {
            scene.window.set_favicon(favicon());
            scene.window.set_has_favicon(true);
        }
        scene.set_load(params.loading, params.progress);
        scene.window.set_expanded(params.expanded);
        if let Some(tab) = params.hovered {
            scene.hover(tab);
        }
        Ok(scene)
    }

    /// Sets the page load and the accessible label that says so.
    pub fn set_load(&self, loading: bool, progress: f32) {
        self.window.set_loading(loading);
        self.window.set_progress(progress);
        self.window.set_dial_label(dial_label(
            Some(self.group_name.as_str()),
            loading,
            progress,
        ));
    }

    /// Lifts the tab and shows its caption, as the host does on `tab-hovered`.
    pub fn hover(&self, tab: i32) {
        self.window.set_hovered_tab(tab);
        if let Some(caption) = self.fan.borrow().caption(tab) {
            self.window.set_caption(caption);
        }
    }

    /// The dial's centre at rest, in window pixels.
    pub fn dial_centre(&self) -> Point {
        self.geometry.centre(self.corner)
    }

    /// Presses the dial and moves the pointer to `to` without letting go (a drag in progress), the
    /// way a user's hand does: in steps of 5 px.
    pub fn drag_to(&self, to: (f32, f32)) {
        let from = self.dial_centre();
        let window = self.window.window();
        let at = |x: f32, y: f32| LogicalPosition::new(x, y);
        window.dispatch_event(WindowEvent::PointerMoved {
            position: at(from.x, from.y),
        });
        window.dispatch_event(WindowEvent::PointerPressed {
            position: at(from.x, from.y),
            button: PointerEventButton::Left,
        });
        let (dx, dy) = (to.0 - from.x, to.1 - from.y);
        let steps = ((dx.hypot(dy) / 5.0).ceil() as usize).max(1);
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            window.dispatch_event(WindowEvent::PointerMoved {
                position: at(from.x + dx * t, from.y + dy * t),
            });
        }
    }

    /// One frame: the animations move to the present time, then the window is drawn.
    pub fn frame(&self) -> Res<Pixels> {
        slint::platform::update_timers_and_animations();
        self.window.window().request_redraw();
        Ok(self.window.window().take_snapshot()?)
    }

    /// The window after the animations have settled (four frames).
    pub fn render(&self) -> Res<Pixels> {
        for _ in 0..3 {
            self.frame()?;
        }
        self.frame()
    }
}

/// A stand-in favicon: a warm red chip with a white "D", 18 px (the mockup's current site).
pub fn favicon() -> Image {
    const SIZE: usize = 18;
    const LETTER: [&str; 7] = [
        "####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####.",
    ];
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(SIZE as u32, SIZE as u32);
    let radius = 4.5_f32;
    let pixels = buffer.make_mut_slice();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = (radius - fx).max(fx - (SIZE as f32 - radius)).max(0.0);
            let dy = (radius - fy).max(fy - (SIZE as f32 - radius)).max(0.0);
            let inside = dx.hypot(dy) <= radius;
            let (gx, gy) = (x as i32 - 4, y as i32 - 2);
            let letter = (0..10).contains(&gx)
                && (0..14).contains(&gy)
                && LETTER[(gy / 2) as usize].as_bytes()[(gx / 2) as usize] == b'#';
            pixels[y * SIZE + x] = match (inside, letter) {
                (false, _) => Rgba8Pixel::new(0, 0, 0, 0),
                (true, true) => Rgba8Pixel::new(255, 255, 255, 255),
                (true, false) => Rgba8Pixel::new(0xA8, 0x58, 0x4E, 255),
            };
        }
    }
    Image::from_rgba8(buffer)
}

pub fn save(pixels: &Pixels, path: &Path) -> Res<()> {
    image::save_buffer_with_format(
        path,
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )?;
    println!("wrote {}", path.display());
    Ok(())
}

/// The pixel at a point of the window (logical pixels, drawn at scale 1).
pub fn at(pixels: &Pixels, x: f32, y: f32) -> Rgba8Pixel {
    let (px, py) = (x.max(0.0) as u32, y.max(0.0) as u32);
    pixels.as_slice()
        [(py.min(pixels.height() - 1) * pixels.width() + px.min(pixels.width() - 1)) as usize]
}

pub fn rgb(pixel: Rgba8Pixel) -> (u8, u8, u8) {
    (pixel.r, pixel.g, pixel.b)
}

fn mockup(params: &Params) -> Res<Scene> {
    Scene::new(
        &mockup_groups(params.dark),
        &mockup_tabs(params.dark),
        params,
    )
}

fn main() -> Res<()> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_corner_dial <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;

    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let file = |scene: &str| output.join(format!("{scene}-{theme}.png"));

        for corner in CORNERS {
            let name = slug(corner);
            save(
                &mockup(&Params::rest(dark, corner))?.render()?,
                &file(&format!("rest-{name}")),
            )?;
            // The mockup's hovered tab and its caption, in the bottom-right corner only.
            let hovered = (corner == DialCorner::BottomRight).then_some(ESSAY);
            let params = Params {
                hovered,
                ..Params::fanned(dark, corner)
            };
            save(
                &mockup(&params)?.render()?,
                &file(&format!("fanned-{name}")),
            )?;
        }

        // The mockup's rest picture is mid-load; this is the same at 65 %.
        let params = Params {
            loading: true,
            progress: 0.65,
            ..Params::rest(dark, DialCorner::BottomRight)
        };
        save(&mockup(&params)?.render()?, &file("loading-65"))?;

        // The load ended a moment ago: the whole ring in `accent`, fading out over 400 ms.
        let params = Params {
            loading: true,
            progress: 0.9,
            reduce_motion: false,
            ..Params::rest(dark, DialCorner::BottomRight)
        };
        let scene = mockup(&params)?;
        scene.render()?;
        std::thread::sleep(Duration::from_millis(200));
        scene.frame()?;
        scene.set_load(false, 0.9);
        scene.frame()?;
        std::thread::sleep(Duration::from_millis(150));
        save(&scene.frame()?, &file("fading"))?;

        let params = Params {
            favicon: false,
            ..Params::rest(dark, DialCorner::BottomRight)
        };
        save(&mockup(&params)?.render()?, &file("no-favicon"))?;

        // Between corners with the fan closed: the dial follows the pointer.
        let scene = mockup(&Params::fanned(dark, DialCorner::BottomRight))?;
        scene.render()?;
        scene.drag_to((1010.0, 590.0));
        save(&scene.render()?, &file("dragging"))?;
    }

    let light = |scene: &str| output.join(format!("{scene}-light.png"));
    let params = Params::fanned(false, DialCorner::BottomRight);
    let (groups, tabs) = generated(64, 64, false);
    save(
        &Scene::new(&groups, &tabs, &params)?.render()?,
        &light("crowded-fan"),
    )?;
    let (groups, tabs) = generated(2, 3, false);
    save(
        &Scene::new(&groups, &tabs, &params)?.render()?,
        &light("three-tabs-fan"),
    )?;
    Ok(())
}
