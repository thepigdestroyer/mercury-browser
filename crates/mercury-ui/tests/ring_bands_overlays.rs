// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the ring's flag markers (`ui/groups/flags.slint`, drawn by `RingFlags`) with Slint's
//! software renderer, in the light and dark appearance, through the scaffold's `RingBandsPreview`
//! (`RingFlags` itself is not exported to Rust). Three rows of pins, one of every `FlagKind` each:
//! 40 px (to see the drawing), the real 20 px and the 8 px floor. Set `MERCURY_UI_RENDER_DIR` to a
//! directory to get the PNGs. The placement of the pins is tested in `src/ring_bands/flags.rs`.
#![cfg(feature = "test-support")]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use mercury_ui::{ChromePalette, ChromeTheme, FlagKind, FlagPin, RingBandsPreview};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{
    ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, SharedPixelBuffer, SharedString,
    VecModel,
};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

type Pixels = SharedPixelBuffer<slint::Rgba8Pixel>;

const SCALE: f32 = 2.0;
const WINDOW: u32 = 516;
/// The three rows: pin size and the y of the pin centres, from the top of the window.
const ROWS: [(f32, f32); 3] = [(40.0, 60.0), (20.0, 150.0), (8.0, 230.0)];
const KINDS: [FlagKind; 8] = [
    FlagKind::Spinner,
    FlagKind::Audible,
    FlagKind::Muted,
    FlagKind::Unread,
    FlagKind::Crashed,
    FlagKind::Capture,
    FlagKind::Current,
    FlagKind::Digit,
];

fn save(pixels: &Pixels, name: &str) {
    let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR").map(PathBuf::from) else {
        return;
    };
    image::save_buffer_with_format(
        directory.join(name),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .expect("the rendering can be saved");
}

fn centre_x(kind: usize) -> f32 {
    40.0 + 62.0 * kind as f32
}

fn tab_id(row: usize, kind: usize) -> i32 {
    1000 + (row * 10 + kind) as i32
}

fn pins() -> Vec<FlagPin> {
    let mut pins = Vec::new();
    for (row, &(size, y)) in ROWS.iter().enumerate() {
        for (index, kind) in KINDS.iter().enumerate() {
            pins.push(FlagPin {
                tab: tab_id(row, index),
                kind: *kind,
                x: centre_x(index),
                y,
                size,
                progress: if *kind == FlagKind::Spinner {
                    0.64
                } else {
                    -1.0
                },
                label: if *kind == FlagKind::Digit {
                    SharedString::from("3")
                } else {
                    SharedString::new()
                },
            });
        }
    }
    pins
}

fn render(theme: ChromeTheme, animate: bool) -> (RingBandsPreview, Pixels) {
    let window = RingBandsPreview::new().expect("the preview can be created");
    window.global::<ChromePalette>().set_theme(theme);
    window.set_animate_spinners(animate);
    window.set_flags(ModelRc::new(VecModel::from(pins())));
    window.show().expect("the preview can be shown");
    window
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: SCALE,
        });
    window.window().set_size(PhysicalSize::new(
        (WINDOW as f32 * SCALE) as u32,
        (WINDOW as f32 * SCALE) as u32,
    ));
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.window().request_redraw();
        let _ = window.window().take_snapshot();
    }
    let pixels = window.window().take_snapshot().expect("the window renders");
    (window, pixels)
}

fn at(pixels: &Pixels, x: u32, y: u32) -> slint::Rgba8Pixel {
    pixels.as_slice()[(y * pixels.width() + x) as usize]
}

/// Physical pixel ranges of the logical box of half-size `half` around (`x`, `y`).
fn around(x: f32, y: f32, half: f32) -> ((u32, u32), (u32, u32)) {
    let range = |c: f32| (((c - half) * SCALE) as u32, ((c + half) * SCALE) as u32);
    (range(x), range(y))
}

fn near(
    pixels: &Pixels,
    (x0, x1): (u32, u32),
    (y0, y1): (u32, u32),
    colour: slint::Color,
) -> usize {
    let close = |a: u8, b: u8| a.abs_diff(b) <= 16;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = at(pixels, x, y);
            close(p.r, colour.red()) && close(p.g, colour.green()) && close(p.b, colour.blue())
        })
        .count()
}

/// Pixels of the box that differ from the window background.
fn ink(
    pixels: &Pixels,
    (x0, x1): (u32, u32),
    (y0, y1): (u32, u32),
    background: slint::Color,
) -> usize {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = at(pixels, x, y);
            (p.r, p.g, p.b) != (background.red(), background.green(), background.blue())
        })
        .count()
}

fn click(window: &RingBandsPreview, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}

fn check_theme(theme: ChromeTheme, name: &str) {
    let (window, pixels) = render(theme, false);
    save(&pixels, &format!("ring-flags-{name}.png"));
    let palette = window.global::<ChromePalette>();
    let background = palette.get_window_background();
    let kind_index = |kind: FlagKind| KINDS.iter().position(|k| *k == kind).expect("a kind");

    for (row, &(size, y)) in ROWS.iter().enumerate() {
        for (index, kind) in KINDS.iter().enumerate() {
            let x = centre_x(index);
            let own = around(x, y, size / 2.0 + 1.0);
            assert!(
                ink(&pixels, own.0, own.1, background) > 4,
                "{name}: row {row} {kind:?} draws something"
            );
            let inner = around(x, y, size / 2.0 - 1.0);
            let danger = near(&pixels, inner.0, inner.1, palette.get_danger());
            let accent = near(&pixels, inner.0, inner.1, palette.get_accent());
            match kind {
                FlagKind::Muted => assert!(danger > 0, "{name}: row {row} the slash is danger"),
                FlagKind::Crashed => {
                    assert!(danger > 4, "{name}: row {row} crashed is a danger badge");
                }
                FlagKind::Spinner => {
                    assert!(accent > 0, "{name}: row {row} the arc is accent");
                }
                FlagKind::Unread => {
                    assert!(accent > 4, "{name}: row {row} unread is accent");
                }
                FlagKind::Audible => {
                    assert_eq!(danger, 0, "{name}: row {row} audible is not muted");
                }
                _ => {}
            }
        }
    }

    // The 40 px pins show their parts.
    let (size, y) = ROWS[0];
    let big = |kind: FlagKind| around(centre_x(kind_index(kind)), y, size / 2.0);
    let centre = |kind: FlagKind| {
        let (x, y) = (centre_x(kind_index(kind)), y);
        at(&pixels, (x * SCALE) as u32, (y * SCALE) as u32)
    };
    let same =
        |p: slint::Rgba8Pixel, c: slint::Color| (p.r, p.g, p.b) == (c.red(), c.green(), c.blue());
    assert!(
        same(centre(FlagKind::Unread), palette.get_accent()),
        "{name}: the unread dot"
    );
    assert!(
        same(centre(FlagKind::Current), palette.get_text()),
        "{name}: the current dot"
    );
    let spinner = big(FlagKind::Spinner);
    assert!(
        same(centre(FlagKind::Spinner), palette.get_popup_background()),
        "{name}: the spinner's badge"
    );
    assert!(
        near(&pixels, spinner.0, spinner.1, palette.get_accent()) > 40,
        "{name}: arc"
    );
    let crashed = big(FlagKind::Crashed);
    assert!(
        near(
            &pixels,
            crashed.0,
            crashed.1,
            palette.get_primary_button_text()
        ) > 10,
        "{name}: the exclamation mark"
    );
    let capture = big(FlagKind::Capture);
    assert!(
        near(
            &pixels,
            capture.0,
            capture.1,
            palette.get_capture_indicator()
        ) > 100,
        "{name}"
    );
    assert!(
        near(
            &pixels,
            capture.0,
            capture.1,
            palette.get_capture_indicator_glyph()
        ) > 20,
        "{name}: the camera"
    );
    let digit = big(FlagKind::Digit);
    assert!(
        near(&pixels, digit.0, digit.1, palette.get_text()) > 10,
        "{name}: the digit"
    );
    assert!(
        near(&pixels, digit.0, digit.1, palette.get_popup_background()) > 100,
        "{name}: the chip"
    );

    // A stopped spinner draws the same picture every time.
    let (_again, second) = render(theme, false);
    assert_eq!(
        pixels.as_bytes(),
        second.as_bytes(),
        "{name}: spinner stands still"
    );

    window.hide().expect("the preview can be hidden");
}

/// The speaker and the muted speaker are the only touch targets (20 px at least, also the 8 px
/// one), and they report the tab.
fn check_mute_clicks(theme: ChromeTheme, name: &str) {
    let (window, _pixels) = render(theme, false);
    let kind_index = |kind: FlagKind| KINDS.iter().position(|k| *k == kind).expect("a kind");
    let muted = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&muted);
    window.on_tab_mute(move |id| log.borrow_mut().push(id));
    for &(_, y) in &ROWS {
        for kind in KINDS {
            click(&window, centre_x(kind_index(kind)), y);
        }
    }
    let want: Vec<i32> = (0..3)
        .flat_map(|row| {
            [FlagKind::Audible, FlagKind::Muted].map(|kind| tab_id(row, kind_index(kind)))
        })
        .collect();
    assert_eq!(*muted.borrow(), want, "{name}: only speakers are clickable");
    window.hide().expect("the preview can be hidden");
}

fn use_software_platform() {
    // The platform is set once per thread; a second call on the same thread is an error to ignore.
    let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
}

#[test]
fn every_flag_kind_renders_in_light_and_dark() {
    use_software_platform();
    for (theme, name) in [(ChromeTheme::Light, "light"), (ChromeTheme::Dark, "dark")] {
        check_theme(theme, name);
    }
}

#[test]
fn speaker_pins_report_mute_clicks_and_nothing_else_does() {
    use_software_platform();
    for (theme, name) in [(ChromeTheme::Light, "light"), (ChromeTheme::Dark, "dark")] {
        check_mute_clicks(theme, name);
    }
}
