// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the tab strip's sound indicator with Slint's software renderer and clicks it, on
//! pinned and other tabs, in the light and dark appearance. Set `MERCURY_UI_RENDER_DIR` to a
//! directory to get the PNGs.
//!
//! Geometry (from `tab_strip.rs`): the strip starts at x = 10; a pinned tab is 40 px wide on a
//! 46 px stride; with room to spare the other tabs are 184 px wide on a 190 px stride. Tabs are
//! 38 px high, from y = 7.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use mercury_ui::{BrowserTab, ChromePalette, ChromeTheme, MainWindow, TabAudioState};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, SharedPixelBuffer, VecModel};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

type Pixels = SharedPixelBuffer<slint::Rgba8Pixel>;

const STRIP_X: u32 = 10;
const PINNED_STRIDE: u32 = 46;
const STRIDE: u32 = 190;
const PINNED_COUNT: u32 = 3;
/// The middle of the tabs, from the top of the window.
const TAB_Y: f32 = 26.0;
/// The speaker of a 184 px tab spans these rows (22 px, centred on `TAB_Y`).
const SPEAKER_Y: (u32, u32) = (15, 37);
/// A pinned tab's badge: 16 px, 3 px above the bottom of the tab (7 to 45).
const BADGE_Y: (u32, u32) = (26, 42);
/// The loading dot of a pinned tab: 6 px, from 7 px below the top of the tab (7 to 45).
const DOT_Y: (u32, u32) = (14, 20);

/// Writes the PNG only when `MERCURY_UI_RENDER_DIR` names a directory.
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

/// A tab whose id is its index plus one, as in the application's strip rows.
fn tab(index: i32, title: &str, active: bool, pinned: bool, audio: TabAudioState) -> BrowserTab {
    BrowserTab {
        title: title.into(),
        active,
        id: index + 1,
        pinned,
        audio,
        url: format!("https://site{}.test/", index + 1).into(),
        ..Default::default()
    }
}

/// Three pinned tabs (playing, muted, silent), then three others (silent, playing and active,
/// muted).
fn sample_tabs() -> Vec<BrowserTab> {
    vec![
        tab(0, "Pinned song", false, true, TabAudioState::Playing),
        tab(1, "Pinned muted", false, true, TabAudioState::Muted),
        tab(2, "Pinned quiet", false, true, TabAudioState::Silent),
        tab(3, "Quiet page", false, false, TabAudioState::Silent),
        tab(4, "Song page", true, false, TabAudioState::Playing),
        tab(5, "Muted page", false, false, TabAudioState::Muted),
    ]
}

fn pinned_left(index: u32) -> u32 {
    STRIP_X + index * PINNED_STRIDE
}

/// The left edge of the `index`-th tab after the pinned ones.
fn tab_left(index: u32) -> u32 {
    STRIP_X + PINNED_COUNT * PINNED_STRIDE + index * STRIDE
}

/// Where the speaker sits in a 184 px tab: 8 px of padding, the 22 px close button, 8 px of
/// spacing, then the 22 px speaker.
fn speaker_box(left: u32) -> (u32, u32) {
    (left + 124, left + 146)
}

/// Where the pinned tab's badge sits: 16 px, 2 px from the right of the 40 px tab.
fn badge_box(left: u32) -> (u32, u32) {
    (left + 22, left + 38)
}

fn at(pixels: &Pixels, x: u32, y: u32) -> slint::Rgba8Pixel {
    pixels.as_slice()[(y * pixels.width() + x) as usize]
}

/// Pixels in a box that differ from the box's top-left pixel (the tab's background): the ink.
fn ink(pixels: &Pixels, (x0, x1): (u32, u32), (y0, y1): (u32, u32)) -> usize {
    let background = at(pixels, x0, y0);
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| at(pixels, *x, *y) != background)
        .count()
}

/// Pixels in a box that are close to `colour`.
fn near(
    pixels: &Pixels,
    (x0, x1): (u32, u32),
    (y0, y1): (u32, u32),
    colour: slint::Color,
) -> usize {
    let close = |a: u8, b: u8| a.abs_diff(b) <= 24;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let pixel = at(pixels, *x, *y);
            close(pixel.r, colour.red())
                && close(pixel.g, colour.green())
                && close(pixel.b, colour.blue())
        })
        .count()
}

/// Pixels of a box (given from the tab's left edge) that differ between two tabs, whose left
/// edges are `from` and `other`: what one tab shows that the other does not.
fn differing(
    pixels: &Pixels,
    from: u32,
    other: u32,
    (x0, x1): (u32, u32),
    (y0, y1): (u32, u32),
) -> usize {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| at(pixels, from + x, *y) != at(pixels, other + x, *y))
        .count()
}

fn click(window: &MainWindow, x: f32, y: f32) {
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

fn middle(range: (u32, u32)) -> f32 {
    (range.0 + range.1) as f32 / 2.0
}

fn render_and_click(theme: ChromeTheme, name: &str) {
    let window = MainWindow::new().expect("the main window can be created");
    window.global::<ChromePalette>().set_theme(theme);
    window.window().set_size(LogicalSize::new(1000.0, 120.0));
    window.set_tabs(ModelRc::new(VecModel::from(sample_tabs())));
    window.set_pinned_tab_count(PINNED_COUNT as i32);
    window.set_active_tab_index(4);
    window.set_is_new_tab(false);
    let toggled = Rc::new(RefCell::new(Vec::new()));
    let selected = Rc::new(RefCell::new(Vec::new()));
    let toggled_log = Rc::clone(&toggled);
    window.on_toggle_tab_muted(move |id| toggled_log.borrow_mut().push(id));
    let selected_log = Rc::clone(&selected);
    window.on_select_tab(move |index| selected_log.borrow_mut().push(index));
    window.show().expect("the main window can be shown");

    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("tab-audio-{name}.png"));
    let palette = window.global::<ChromePalette>();

    // Other tabs: nothing on a silent one, the speaker on a playing one (the active tab's text
    // colour), the muted speaker (the danger colour) on a muted one.
    assert_eq!(
        ink(&pixels, speaker_box(tab_left(0)), SPEAKER_Y),
        0,
        "{name}: a silent tab has no speaker"
    );
    let playing = speaker_box(tab_left(1));
    assert!(
        ink(&pixels, playing, SPEAKER_Y) > 20,
        "{name}: a playing tab shows the speaker"
    );
    assert!(
        near(&pixels, playing, SPEAKER_Y, palette.get_text()) > 4,
        "{name}: the speaker of the active tab uses the text colour"
    );
    let muted = speaker_box(tab_left(2));
    assert!(
        ink(&pixels, muted, SPEAKER_Y) > 20,
        "{name}: a muted tab shows the muted speaker"
    );
    assert!(
        near(&pixels, muted, SPEAKER_Y, palette.get_danger()) > 4,
        "{name}: the muted speaker uses the danger colour"
    );
    assert_eq!(
        near(&pixels, playing, SPEAKER_Y, palette.get_danger()),
        0,
        "{name}: a playing speaker is not drawn as muted"
    );

    // Pinned tabs: the badge is what differs from the silent pinned tab (the icon is under it).
    for (index, label) in [(0, "playing"), (1, "muted")] {
        let changed = differing(
            &pixels,
            pinned_left(index),
            pinned_left(2),
            badge_box(0),
            BADGE_Y,
        );
        assert!(
            changed > 20,
            "{name}: a {label} pinned tab shows a badge ({changed})"
        );
    }
    assert!(
        near(
            &pixels,
            badge_box(pinned_left(1)),
            BADGE_Y,
            palette.get_danger()
        ) > 2,
        "{name}: the muted badge uses the danger colour"
    );
    assert!(BADGE_Y.0 >= DOT_Y.1, "the badge sits below the loading dot");

    // Clicks toggle the mute of the tab, by id, and select nothing: pinned playing (1), pinned
    // muted (2), the active tab (5) and the muted tab (6).
    click(&window, middle(badge_box(pinned_left(0))), middle(BADGE_Y));
    click(&window, middle(badge_box(pinned_left(1))), middle(BADGE_Y));
    click(&window, middle(playing), TAB_Y);
    click(&window, middle(muted), TAB_Y);
    assert_eq!(*toggled.borrow(), [1, 2, 5, 6], "{name}");
    assert!(
        selected.borrow().is_empty(),
        "{name}: {:?}",
        selected.borrow()
    );

    // Clicking a tab elsewhere still selects it (by index).
    click(&window, (tab_left(0) + 60) as f32, TAB_Y);
    assert_eq!(*selected.borrow(), [3], "{name}");
    assert_eq!(toggled.borrow().len(), 4, "{name}");
    window.hide().expect("the main window can be hidden");
}

#[test]
fn the_sound_indicator_renders_and_toggles_mute_without_selecting_the_tab() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).expect("no platform is set yet");
    for (theme, name) in [(ChromeTheme::Light, "light"), (ChromeTheme::Dark, "dark")] {
        render_and_click(theme, name);
    }
}
