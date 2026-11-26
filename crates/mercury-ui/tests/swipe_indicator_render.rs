// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the swipe indicator headlessly with Slint's software renderer and checks where the
//! chip lands for each direction and progress. Set `MERCURY_UI_RENDER_DIR` to also save each
//! rendering as a PNG.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize};

slint::slint! {
    import { SwipeIndicator } from "../ui/gestures/swipe-indicator.slint";
    import { ChromePalette, ChromeTheme } from "../ui/theme/chrome-palette.slint";

    export { ChromePalette, ChromeTheme }

    // A stand-in for the page area: the indicator over a white page with text under it.
    export component Harness inherits Window {
        in property <bool> shown: true;
        in property <bool> forward: false;
        in property <float> progress: 0;
        in property <bool> armed: false;
        in property <bool> reduce-motion: true;
        out property <int> clicks: 0;

        width: 900px;
        height: 400px;
        background: ChromePalette.window-background;

        Rectangle {
            background: #ffffff;

            Text {
                x: 24px;
                y: 16px;
                text: "Page content under the indicator";
                color: #000000;
                font-size: 22px;
            }

            TouchArea {
                width: parent.width;
                height: parent.height;
                clicked => { root.clicks += 1; }
            }

            indicator := SwipeIndicator {
                width: parent.width;
                height: parent.height;
                shown: root.shown;
                forward: root.forward;
                progress: root.progress;
                armed: root.armed;
                reduce-motion: root.reduce-motion;
            }
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

const SCALE: f32 = 2.0;
const WIDTH: u32 = 1800;
const HEIGHT: u32 = 800;

fn same(a: &PremultipliedRgbaColor, b: &PremultipliedRgbaColor) -> bool {
    a.red == b.red && a.green == b.green && a.blue == b.blue && a.alpha == b.alpha
}

fn same_frame(a: &[PremultipliedRgbaColor], b: &[PremultipliedRgbaColor]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
}

/// WCAG relative luminance of a pixel.
fn luminance(pixel: &PremultipliedRgbaColor) -> f64 {
    let channel = |value: u8| {
        let value = f64::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(pixel.red) + 0.7152 * channel(pixel.green) + 0.0722 * channel(pixel.blue)
}

struct Fixture {
    window: Rc<MinimalSoftwareWindow>,
    harness: Harness,
}

impl Fixture {
    fn new() -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform(window.clone())))
            .expect("the platform is set once");
        let harness = Harness::new().expect("the harness builds");
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: SCALE,
        });
        window.set_size(PhysicalSize::new(WIDTH, HEIGHT));
        harness.show().expect("the harness shows");
        Self { window, harness }
    }

    fn settle(&self) {
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            let _ = self.render();
        }
    }

    fn render(&self) -> Vec<PremultipliedRgbaColor> {
        self.window.request_redraw();
        let mut buffer = vec![PremultipliedRgbaColor::default(); (WIDTH * HEIGHT) as usize];
        self.window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, WIDTH as usize);
        });
        buffer
    }

    fn set(&self, shown: bool, forward: bool, progress: f32, armed: bool) {
        self.harness.set_shown(shown);
        self.harness.set_forward(forward);
        self.harness.set_progress(progress);
        self.harness.set_armed(armed);
    }

    fn save(&self, name: &str, pixels: &[PremultipliedRgbaColor]) {
        if let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR") {
            let bytes = pixels
                .iter()
                .flat_map(|pixel| [pixel.red, pixel.green, pixel.blue, 255])
                .collect::<Vec<_>>();
            let path = std::path::Path::new(&directory).join(format!("{name}.png"));
            image::save_buffer(
                &path,
                &bytes,
                WIDTH,
                HEIGHT,
                image::ExtendedColorType::Rgba8,
            )
            .expect("the PNG is written");
        }
    }
}

/// The bounding box of the pixels that differ from the page with no chip, as (left, top,
/// right, bottom) in image pixels.
fn chip_box(
    pixels: &[PremultipliedRgbaColor],
    baseline: &[PremultipliedRgbaColor],
) -> Option<(u32, u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let i = (y * WIDTH + x) as usize;
            if same(&pixels[i], &baseline[i]) {
                continue;
            }
            found = Some(match found {
                None => (x, y, x, y),
                Some((l, t, r, b)) => (l.min(x), t.min(y), r.max(x), b.max(y)),
            });
        }
    }
    found
}

/// The pixel colour at the chip centre, read where the chip sits for the given box.
fn centre(
    pixels: &[PremultipliedRgbaColor],
    (l, t, r, b): (u32, u32, u32, u32),
) -> PremultipliedRgbaColor {
    let x = (l + r) / 2;
    let y = (t + b) / 2;
    pixels[(y * WIDTH + x) as usize]
}

#[test]
fn nothing_is_drawn_while_not_shown() {
    let f = Fixture::new();
    f.set(false, false, 0.0, false);
    f.settle();
    let hidden = f.render();
    f.set(true, false, 0.0, false);
    f.set(false, false, 0.0, false);
    f.settle();
    let also_hidden = f.render();
    assert!(
        same_frame(&hidden, &also_hidden),
        "a hidden indicator must draw nothing"
    );
}

#[test]
fn the_back_chip_hugs_the_left_edge_and_the_forward_chip_the_right() {
    let f = Fixture::new();
    f.set(false, false, 0.0, false);
    f.settle();
    let baseline = f.render();

    f.set(true, false, 1.0, false);
    f.settle();
    let back = f.render();
    let (l, _, r, _) = chip_box(&back, &baseline).expect("the back chip is drawn");
    assert!(
        l < 64 * SCALE as u32 * 2 / 2 + 4 && r < 64 * 2,
        "the back chip is at the left edge, box {l}..{r}"
    );
    assert!(l < WIDTH / 4, "the back chip is in the left quarter");

    f.set(true, true, 1.0, false);
    f.settle();
    let forward = f.render();
    let (fl, _, fr, _) = chip_box(&forward, &baseline).expect("the forward chip is drawn");
    assert!(
        fr > WIDTH - 64 * 2,
        "the forward chip is at the right edge, box {fl}..{fr}"
    );
    assert!(
        fl > WIDTH * 3 / 4,
        "the forward chip is in the right quarter"
    );
}

#[test]
fn the_chip_slides_in_with_progress() {
    let f = Fixture::new();
    f.set(false, false, 0.0, false);
    f.settle();
    let baseline = f.render();
    // The back chip starts off the left edge, so its visible right edge is what grows.
    let mut previous = 0u32;
    for progress in [0.15_f32, 0.5, 1.0] {
        f.set(true, false, progress, false);
        f.settle();
        let frame = f.render();
        let (_, _, r, _) = chip_box(&frame, &baseline).expect("the chip is drawn");
        assert!(
            r > previous,
            "the right edge grows: {previous} then {r} at {progress}"
        );
        previous = r;
    }
}

#[test]
fn an_armed_chip_is_filled_with_the_accent() {
    let f = Fixture::new();
    f.set(false, false, 0.0, false);
    f.settle();
    let baseline = f.render();

    f.set(true, false, 1.0, false);
    f.settle();
    let unarmed = f.render();
    let unarmed_box = chip_box(&unarmed, &baseline).expect("the chip is drawn");

    f.set(true, false, 1.0, true);
    f.settle();
    let armed = f.render();
    let armed_box = chip_box(&armed, &baseline).expect("the armed chip is drawn");

    let before = centre(&unarmed, unarmed_box);
    let after = centre(&armed, armed_box);
    assert!(!same(&before, &after), "an armed chip is drawn differently");
    assert!(
        luminance(&after) < luminance(&before),
        "the accent fill is darker than the neutral one in light mode"
    );
}

#[test]
fn the_arrow_points_away_from_the_edge_it_came_from() {
    let f = Fixture::new();
    f.set(false, false, 0.0, false);
    f.settle();
    let baseline = f.render();

    f.set(true, false, 1.0, false);
    f.settle();
    let back = f.render();
    let (bl, _, br, _) = chip_box(&back, &baseline).expect("the back chip is drawn");
    let back_centre_x = (bl + br) / 2;
    let back_arrow = arrow_centroid_x(&back, &baseline, bl, br);
    assert!(
        back_arrow < f64::from(back_centre_x),
        "the back arrow points left"
    );

    f.set(true, true, 1.0, false);
    f.settle();
    let forward = f.render();
    let (fl, _, fr, _) = chip_box(&forward, &baseline).expect("the forward chip is drawn");
    let forward_centre_x = (fl + fr) / 2;
    let forward_arrow = arrow_centroid_x(&forward, &baseline, fl, fr);
    assert!(
        forward_arrow > f64::from(forward_centre_x),
        "the forward arrow points right"
    );
}

/// The mean x of the pixels inside the chip that differ from the neutral chip fill: the arrow.
fn arrow_centroid_x(
    pixels: &[PremultipliedRgbaColor],
    baseline: &[PremultipliedRgbaColor],
    left: u32,
    right: u32,
) -> f64 {
    let (top, bottom) = chip_box(pixels, baseline)
        .map(|(_, t, _, b)| (t, b))
        .expect("the chip is drawn");
    // Inside the disc above the arrow: the neutral or accent fill.
    let fill = pixels[((top + 8) * WIDTH + (left + right) / 2) as usize];
    let mut sum = 0.0_f64;
    let mut count = 0u32;
    for y in top..=bottom {
        for x in left..=right {
            let i = (y * WIDTH + x) as usize;
            if !same(&pixels[i], &baseline[i]) && !same(&pixels[i], &fill) {
                sum += f64::from(x);
                count += 1;
            }
        }
    }
    sum / f64::from(count.max(1))
}

#[test]
fn the_chip_passes_pointer_events() {
    let f = Fixture::new();
    f.set(true, false, 1.0, false);
    f.settle();
    let frame = f.render();
    let baseline = {
        f.set(false, false, 0.0, false);
        f.settle();
        f.render()
    };
    let (l, t, r, b) = chip_box(&frame, &baseline).expect("the chip is drawn");
    let at = LogicalPosition::new(((l + r) / 2) as f32 / SCALE, ((t + b) / 2) as f32 / SCALE);
    f.set(false, false, 0.0, false);
    f.settle();
    let bare = LogicalPosition::new(450.0, 300.0);
    f.window
        .dispatch_event(WindowEvent::PointerMoved { position: bare });
    f.window.dispatch_event(WindowEvent::PointerPressed {
        position: bare,
        button: PointerEventButton::Left,
    });
    f.window.dispatch_event(WindowEvent::PointerReleased {
        position: bare,
        button: PointerEventButton::Left,
    });
    assert_eq!(
        f.harness.get_clicks(),
        1,
        "control: the harness page takes clicks with no chip"
    );
    f.set(true, false, 1.0, false);
    f.settle();
    f.window
        .dispatch_event(WindowEvent::PointerMoved { position: at });
    f.window.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button: PointerEventButton::Left,
    });
    f.window.dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button: PointerEventButton::Left,
    });
    assert_eq!(
        f.harness.get_clicks(),
        2,
        "the click reaches the page under the chip"
    );
}

#[test]
fn renders_in_light_and_dark() {
    let f = Fixture::new();
    f.set(false, false, 0.0, false);
    f.settle();
    let baseline = f.render();
    let cases = [
        ("back", false, 0.15_f32, false),
        ("back", false, 0.5, false),
        ("back", false, 1.0, true),
        ("forward", true, 0.15, false),
        ("forward", true, 0.5, false),
        ("forward", true, 1.0, true),
    ];
    for (theme, name) in [(ChromeTheme::Light, "light"), (ChromeTheme::Dark, "dark")] {
        f.harness.global::<ChromePalette>().set_theme(theme);
        for (direction, forward, progress, armed) in cases {
            f.set(true, forward, progress, armed);
            f.settle();
            let frame = f.render();
            assert!(
                chip_box(&frame, &baseline).is_some(),
                "{direction} {progress} is drawn"
            );
            let label = if armed {
                format!("swipe-{direction}-armed-{name}")
            } else {
                format!(
                    "swipe-{direction}-p{}-{name}",
                    (progress * 100.0).round() as u32
                )
            };
            f.save(&label, &frame);
        }
    }
}
