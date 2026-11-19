// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The "Couldn't save the login for <site>." notice under the toolbar, drawn through Slint's
//! software renderer (to PNG files in `MERCURY_UI_RENDER_DIR`, when that is set) and checked in
//! the light and the dark theme: where its band and its two buttons are at 1000 px and at
//! 420 px, that the text stays readable, what a click and the keyboard do, and that it sits in
//! the main window's toolbar column and is hidden with the rest of the chrome.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use mercury_ui::{ChromePalette as AppPalette, ChromeTheme as AppTheme, MainWindow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, Rgba8Pixel, SharedPixelBuffer};

slint::slint! {
    import { PasswordSaveNotice } from "ui/passwords/password-save-notice.slint";
    import { ChromePalette, ChromeTheme } from "ui/theme/chrome-palette.slint";
    export { ChromePalette, ChromeTheme }

    // The notice under 20 px of toolbar, in a window of its own, so that the rows above and below
    // the band are not the band's colour.
    export component NoticePreview inherits Window {
        in property <string> text;
        out property <int> retries;
        out property <int> dismissals;
        min-width: 0px;
        background: ChromePalette.window-background;

        VerticalLayout {
            alignment: start;
            Rectangle { height: 20px; background: ChromePalette.toolbar-background; }
            PasswordSaveNotice {
                text: root.text;
                retry => { root.retries += 1; }
                dismissed => { root.dismissals += 1; }
            }
        }
    }
}

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

type Rgb = [u8; 3];

/// The notice's own height, and where it starts in the preview window.
const BAND_HEIGHT: usize = 44;
const BAND_Y: usize = 20;
const PREVIEW_HEIGHT: u32 = 100;
/// The buttons are 28 px high, 8 px under the band's top.
const BUTTON_TOP: usize = 8;
const BUTTON_BOTTOM: usize = 36;
/// The band's right padding, and the gap between its parts.
const RIGHT_PADDING: usize = 14;

const SHORT: &str = "Couldn't save the login for example.com.";
const LONG: &str = "Couldn't save the login for the-quick-brown-fox-jumps-over-the-lazy-dog.\
                    long-subdomain.example-hosting-service.co.uk.";

const MINIMUM_TEXT_CONTRAST: f64 = 4.5;
const MINIMUM_RING_CONTRAST: f64 = 3.0;

fn rgb(color: slint::Color) -> Rgb {
    [color.red(), color.green(), color.blue()]
}

/// WCAG 2.x relative luminance.
fn luminance([r, g, b]: Rgb) -> f64 {
    let linear = |channel: u8| {
        let c = f64::from(channel) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// WCAG 2.x contrast ratio, from 1 to 21.
fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn distance(a: Rgb, b: Rgb) -> u32 {
    a.iter().zip(b).map(|(x, y)| u32::from(x.abs_diff(y))).sum()
}

/// The colours of the chrome palette the notice uses, for one window and theme.
struct Colours {
    window: Rgb,
    notice: Rgb,
    border: Rgb,
    text: Rgb,
    primary_background: Rgb,
    primary_text: Rgb,
    hover: Rgb,
    pressed: Rgb,
    focus_ring: Rgb,
}

macro_rules! colours {
    ($palette:expr) => {{
        let palette = $palette;
        Colours {
            window: rgb(palette.get_window_background()),
            notice: rgb(palette.get_notice_background()),
            border: rgb(palette.get_notice_border()),
            text: rgb(palette.get_text()),
            primary_background: rgb(palette.get_primary_button_background()),
            primary_text: rgb(palette.get_primary_button_text()),
            hover: rgb(palette.get_button_hover()),
            pressed: rgb(palette.get_button_pressed()),
            focus_ring: rgb(palette.get_focus_ring()),
        }
    }};
}

/// A rendering of a window.
struct Frame {
    width: usize,
    height: usize,
    pixels: SharedPixelBuffer<Rgba8Pixel>,
}

impl Frame {
    fn of(window: &slint::Window, name: Option<&str>) -> Frame {
        let pixels = window.take_snapshot().expect("the window renders");
        if let (Some(name), Some(directory)) = (name, std::env::var_os("MERCURY_UI_RENDER_DIR")) {
            image::save_buffer(
                std::path::Path::new(&directory).join(format!("{name}.png")),
                pixels.as_bytes(),
                pixels.width(),
                pixels.height(),
                image::ExtendedColorType::Rgba8,
            )
            .expect("the rendering can be saved");
        }
        Frame {
            width: pixels.width() as usize,
            height: pixels.height() as usize,
            pixels,
        }
    }

    fn at(&self, x: usize, y: usize) -> Rgb {
        assert!(x < self.width && y < self.height, "({x}, {y}) is outside");
        let p = self.pixels.as_slice()[y * self.width + x];
        [p.r, p.g, p.b]
    }
}

/// Where the parts of a notice are, as inclusive columns of the frame.
#[derive(Debug)]
struct Band {
    text: (usize, usize),
    retry: (usize, usize),
    dismiss: (usize, usize),
}

impl Band {
    fn middle(run: (usize, usize)) -> f32 {
        (run.0 + run.1 + 1) as f32 / 2.0
    }
}

/// Runs of columns that have a pixel other than `background` in `rows`, with less than
/// `min_gap` blank columns between the columns of one run.
fn ink_runs(
    frame: &Frame,
    rows: Range<usize>,
    background: Rgb,
    min_gap: usize,
) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for x in 0..frame.width {
        if !rows.clone().any(|y| frame.at(x, y) != background) {
            continue;
        }
        match runs.last_mut() {
            Some(run) if x - run.1 <= min_gap => run.1 = x,
            _ => runs.push((x, x)),
        }
    }
    runs
}

/// The band's first row is all padding and its last row is all its border, whatever is drawn
/// in it (a focus ring must not reach them).
fn band_edges(frame: &Frame, band_y: usize, colours: &Colours, context: &str) {
    for x in 0..frame.width {
        assert_eq!(
            frame.at(x, band_y + BAND_HEIGHT - 1),
            colours.border,
            "{context}: the bottom border at x {x}"
        );
        assert_eq!(
            frame.at(x, band_y),
            colours.notice,
            "{context}: top row at x {x}"
        );
    }
}

/// Checks the band that starts at row `band_y` of `frame` and finds its text and its buttons.
fn locate(frame: &Frame, band_y: usize, colours: &Colours, context: &str) -> Band {
    let width = frame.width;
    // The band is exactly 44 px: the notice colour in every row but the last, which is its
    // border, and a different colour just above and just below. Rows are checked at the two
    // edges, where only padding is.
    assert_ne!(frame.at(2, band_y - 1), colours.notice, "{context}: above");
    assert_ne!(
        frame.at(2, band_y + BAND_HEIGHT),
        colours.notice,
        "{context}: below"
    );
    for row in 0..BAND_HEIGHT - 1 {
        for x in [0, 2, width - 3, width - 1] {
            assert_eq!(
                frame.at(x, band_y + row),
                colours.notice,
                "{context}: row {row} of the band at x {x} is not the notice colour"
            );
        }
    }
    band_edges(frame, band_y, colours, context);
    assert_ne!(
        colours.notice, colours.window,
        "{context}: the band must stand out"
    );

    let runs = ink_runs(
        frame,
        band_y + BUTTON_TOP..band_y + BUTTON_BOTTOM,
        colours.notice,
        6,
    );
    assert!(
        runs.len() >= 3,
        "{context}: text, Retry and Dismiss expected, got {runs:?}"
    );
    let band = Band {
        text: runs[runs.len() - 3],
        retry: runs[runs.len() - 2],
        dismiss: runs[runs.len() - 1],
    };
    assert_eq!(
        runs.len(),
        3,
        "{context}: stray marks in the band: {runs:?}"
    );

    // The text starts after the 16 px of padding, and stops at least the 8 px spacing before
    // Retry (a cut text ends in an ellipsis, it never runs into the button).
    assert!(band.text.0 >= 15, "{context}: {band:?}");
    assert!(
        band.text.1 + 7 <= band.retry.0,
        "{context}: text runs into Retry: {band:?}"
    );
    // Both buttons are whole and inside the window, the right one 14 px from the edge.
    assert!(band.retry.1 + 7 <= band.dismiss.0, "{context}: {band:?}");
    assert!(
        band.dismiss.1 + RIGHT_PADDING <= width + 1,
        "{context}: Dismiss reaches into the padding or off the window: {band:?}"
    );
    for (name, run) in [("Retry", band.retry), ("Dismiss", band.dismiss)] {
        let button_width = run.1 - run.0 + 1;
        assert!(
            (40..=120).contains(&button_width),
            "{context}: {name} is {button_width} px wide"
        );
        // 28 px high, fully inside the band (the column through its middle).
        let x = Band::middle(run) as usize;
        let top =
            (band_y + 1..band_y + BAND_HEIGHT - 1).find(|&y| frame.at(x, y) != colours.notice);
        let bottom = (band_y + 1..band_y + BAND_HEIGHT - 1)
            .rev()
            .find(|&y| frame.at(x, y) != colours.notice);
        let (Some(top), Some(bottom)) = (top, bottom) else {
            panic!("{context}: {name} is not drawn in its column {x}");
        };
        assert!(
            top >= band_y + BUTTON_TOP - 1 && bottom < band_y + BUTTON_BOTTOM + 1,
            "{context}: {name} spans rows {top} to {bottom}, the band starts at {band_y}"
        );
        assert!(
            bottom - top + 1 >= 26,
            "{context}: {name} is only {} rows high",
            bottom - top + 1
        );
    }
    band
}

/// Pixels of the text colour (the stems of the letters), in the text's columns.
fn text_pixels(frame: &Frame, band: &Band, band_y: usize, colours: &Colours) -> usize {
    (band.text.0..=band.text.1)
        .flat_map(|x| (band_y + BUTTON_TOP..band_y + BUTTON_BOTTOM).map(move |y| (x, y)))
        .filter(|&(x, y)| distance(frame.at(x, y), colours.text) <= 24)
        .count()
}

/// Pixels of the focus ring's colour around a button: the ring is outside the button, 3 px
/// beyond its edges, and still inside the band.
fn ring_pixels(frame: &Frame, run: (usize, usize), band_y: usize, colours: &Colours) -> usize {
    (run.0 - 4..=run.1 + 4)
        .flat_map(|x| (band_y + BUTTON_TOP - 4..band_y + BUTTON_BOTTOM + 4).map(move |y| (x, y)))
        .filter(|&(x, y)| distance(frame.at(x, y), colours.focus_ring) <= 6)
        .count()
}

fn pointer(window: &slint::Window, x: f32, y: f32) -> LogicalPosition {
    let position = LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    position
}

fn press_pointer(window: &slint::Window, position: LogicalPosition) {
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
}

fn release_pointer(window: &slint::Window, position: LogicalPosition) {
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn click(window: &slint::Window, x: f32, y: f32) {
    let position = pointer(window, x, y);
    press_pointer(window, position);
    release_pointer(window, position);
    window.dispatch_event(WindowEvent::PointerExited);
}

fn press(window: &slint::Window, key: impl Into<slint::SharedString> + Clone) {
    window.dispatch_event(WindowEvent::KeyPressed {
        text: key.clone().into(),
    });
    window.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
}

fn ratio(label: &str, ratio: f64) -> f64 {
    println!("contrast {label}: {ratio:.2}:1");
    ratio
}

fn preview_counts(preview: &NoticePreview) -> (i32, i32) {
    (preview.get_retries(), preview.get_dismissals())
}

/// The notice in a window of its own, in one theme.
fn notice_in_theme(theme: ChromeTheme, name: &str) {
    let preview = NoticePreview::new().expect("the preview can be created");
    preview.global::<ChromePalette>().set_theme(theme);
    let colours = colours!(preview.global::<ChromePalette>());
    let window = preview.window();
    preview.show().expect("the preview can be shown");
    let draw_at = |width: u32, picture: Option<&str>| {
        window.set_size(PhysicalSize::new(width, PREVIEW_HEIGHT));
        Frame::of(window, picture)
    };

    // (a) Where the band, the text and the buttons are at 1000 px and at 420 px.
    preview.set_text(SHORT.into());
    let wide = draw_at(1000, Some(&format!("password-save-notice-{name}-1000")));
    let wide_band = locate(&wide, BAND_Y, &colours, &format!("{name} 1000 px"));
    assert!(
        text_pixels(&wide, &wide_band, BAND_Y, &colours) >= 20,
        "{name}: the text is not drawn in its colour"
    );
    // The short text is whole: a long one is wider than it, not the same.
    let short_width = wide_band.text.1 - wide_band.text.0;
    preview.set_text(LONG.into());
    let wide_long = draw_at(
        1000,
        Some(&format!("password-save-notice-{name}-1000-long")),
    );
    let wide_long_band = locate(
        &wide_long,
        BAND_Y,
        &colours,
        &format!("{name} 1000 px long"),
    );
    assert!(wide_long_band.text.1 - wide_long_band.text.0 > short_width + 100);
    // The same on a narrow window: the text is cut, the buttons stay whole and in place.
    let narrow = draw_at(420, Some(&format!("password-save-notice-{name}-420")));
    let band = locate(&narrow, BAND_Y, &colours, &format!("{name} 420 px"));
    assert!(
        text_pixels(&narrow, &band, BAND_Y, &colours) >= 20,
        "{name}: the cut text is not drawn in its colour"
    );
    let cut = band.text.1 - band.text.0;
    let whole = wide_long_band.text.1 - wide_long_band.text.0;
    assert!(
        cut + 100 < whole && cut > 100,
        "{name}: at 420 px the text should be cut ({cut} px) from {whole} px, not gone"
    );
    // The buttons are as wide as at 1000 px, within a pixel.
    for (narrow_run, wide_run) in [
        (band.retry, wide_long_band.retry),
        (band.dismiss, wide_long_band.dismiss),
    ] {
        let narrow_width = narrow_run.1 - narrow_run.0;
        let wide_width = wide_run.1 - wide_run.0;
        assert!(
            narrow_width.abs_diff(wide_width) <= 1,
            "{name}: a button was squeezed"
        );
    }

    // (b) Clicks, at 420 px with the long text. The buttons are found where they are drawn.
    let middle = (BAND_Y + BUTTON_TOP + BUTTON_BOTTOM) as f32 / 2.0;
    let retry_x = Band::middle(band.retry);
    let dismiss_x = Band::middle(band.dismiss);
    assert_eq!(preview_counts(&preview), (0, 0));
    // Where nothing is: the text, the gap between the buttons, the band's padding above a
    // button and the strip of toolbar above the band.
    click(window, 40.0, middle);
    click(
        window,
        (band.retry.1 + band.dismiss.0 + 1) as f32 / 2.0,
        middle,
    );
    click(window, retry_x, BAND_Y as f32 + 3.0);
    click(window, dismiss_x, BAND_Y as f32 + 41.0);
    click(window, retry_x, 10.0);
    assert_eq!(
        preview_counts(&preview),
        (0, 0),
        "{name}: a click beside the buttons"
    );

    // Retry: hovered, pressed (both stay readable), then released.
    let position = pointer(window, retry_x, middle);
    let hover = Frame::of(window, None);
    press_pointer(window, position);
    let pressed = Frame::of(
        window,
        Some(&format!("password-save-notice-{name}-retry-pressed")),
    );
    release_pointer(window, position);
    window.dispatch_event(WindowEvent::PointerExited);
    assert_eq!(
        preview_counts(&preview),
        (1, 0),
        "{name}: one click on Retry"
    );
    let rest = ratio(
        &format!("{name} Retry label on its fill"),
        contrast(
            narrow.at(band.retry.0 + 4, middle as usize),
            colours.primary_text,
        ),
    );
    let hovered = ratio(
        &format!("{name} Retry label on its hovered fill"),
        contrast(
            hover.at(band.retry.0 + 4, middle as usize),
            colours.primary_text,
        ),
    );
    let held = ratio(
        &format!("{name} Retry label on its pressed fill"),
        contrast(
            pressed.at(band.retry.0 + 4, middle as usize),
            colours.primary_text,
        ),
    );
    for (state, value) in [("at rest", rest), ("hovered", hovered), ("pressed", held)] {
        assert!(
            value >= MINIMUM_TEXT_CONTRAST,
            "{name}: Retry's label on its fill {state} is {value:.2}:1"
        );
    }
    assert_ne!(
        narrow.at(band.retry.0 + 4, middle as usize),
        hover.at(band.retry.0 + 4, middle as usize),
        "{name}: hovering Retry changes nothing"
    );

    // Dismiss: hovered and pressed (the label stays readable on the button's fill).
    let position = pointer(window, dismiss_x, middle);
    let hover = Frame::of(window, None);
    press_pointer(window, position);
    let pressed = Frame::of(window, None);
    release_pointer(window, position);
    window.dispatch_event(WindowEvent::PointerExited);
    assert_eq!(
        preview_counts(&preview),
        (1, 1),
        "{name}: one click on Dismiss"
    );
    let fill = band.dismiss.0 + 4;
    assert_eq!(hover.at(fill, middle as usize), colours.hover, "{name}");
    assert_eq!(pressed.at(fill, middle as usize), colours.pressed, "{name}");
    for (state, background) in [("hovered", colours.hover), ("pressed", colours.pressed)] {
        let value = ratio(
            &format!("{name} Dismiss label on its {state} fill"),
            contrast(colours.text, background),
        );
        assert!(
            value >= MINIMUM_TEXT_CONTRAST,
            "{name}: Dismiss {state} is {value:.2}:1"
        );
    }
    // A click does not take focus (typing elsewhere is not interrupted): no ring yet.
    let after_clicks = Frame::of(window, None);
    assert_eq!(
        ring_pixels(&after_clicks, band.retry, BAND_Y, &colours),
        0,
        "{name}"
    );
    assert_eq!(
        ring_pixels(&after_clicks, band.dismiss, BAND_Y, &colours),
        0,
        "{name}"
    );

    // (c) The keyboard, at 1000 px. Tab reaches Retry, then Dismiss; Return and Space press the
    // focused button; another key does nothing.
    preview.set_text(SHORT.into());
    window.set_size(PhysicalSize::new(1000, PREVIEW_HEIGHT));
    let before = preview_counts(&preview);
    press(window, Key::Tab);
    let on_retry = Frame::of(
        window,
        Some(&format!("password-save-notice-{name}-focus-retry")),
    );
    assert!(
        ring_pixels(&on_retry, wide_band.retry, BAND_Y, &colours) >= 150,
        "{name}: Tab does not show a focus ring on Retry"
    );
    band_edges(
        &on_retry,
        BAND_Y,
        &colours,
        &format!("{name} Retry focused"),
    );
    assert_eq!(
        ring_pixels(&on_retry, wide_band.dismiss, BAND_Y, &colours),
        0,
        "{name}"
    );
    press(window, Key::Return);
    assert_eq!(
        preview_counts(&preview),
        (before.0 + 1, before.1),
        "{name}: Return on Retry"
    );
    press(window, Key::Space);
    assert_eq!(
        preview_counts(&preview),
        (before.0 + 2, before.1),
        "{name}: Space on Retry"
    );
    press(window, "x");
    assert_eq!(
        preview_counts(&preview),
        (before.0 + 2, before.1),
        "{name}: another key"
    );

    press(window, Key::Tab);
    let on_dismiss = Frame::of(
        window,
        Some(&format!("password-save-notice-{name}-focus-dismiss")),
    );
    assert!(
        ring_pixels(&on_dismiss, wide_band.dismiss, BAND_Y, &colours) >= 150,
        "{name}: a second Tab does not show a focus ring on Dismiss"
    );
    band_edges(
        &on_dismiss,
        BAND_Y,
        &colours,
        &format!("{name} Dismiss focused"),
    );
    assert_eq!(
        ring_pixels(&on_dismiss, wide_band.retry, BAND_Y, &colours),
        0,
        "{name}"
    );
    press(window, Key::Return);
    assert_eq!(
        preview_counts(&preview),
        (before.0 + 2, before.1 + 1),
        "{name}: Return on Dismiss"
    );
    press(window, Key::Space);
    assert_eq!(
        preview_counts(&preview),
        (before.0 + 2, before.1 + 2),
        "{name}: Space on Dismiss"
    );
    press(window, "x");
    assert_eq!(
        preview_counts(&preview),
        (before.0 + 2, before.1 + 2),
        "{name}: another key"
    );

    // The text and the ring are readable on the band; the label of the pressed button too.
    let text_on_band = ratio(
        &format!("{name} text on the notice background"),
        contrast(colours.text, colours.notice),
    );
    assert!(
        text_on_band >= MINIMUM_TEXT_CONTRAST,
        "{name}: text on the notice is {text_on_band:.2}:1"
    );
    let ring_on_band = ratio(
        &format!("{name} focus ring on the notice background"),
        contrast(colours.focus_ring, colours.notice),
    );
    assert!(
        ring_on_band >= MINIMUM_RING_CONTRAST,
        "{name}: focus ring is {ring_on_band:.2}:1"
    );
    // For the record: the palette's own pair for primary buttons, which the notice does not use
    // as it is where it would fall short.
    ratio(
        &format!("{name} palette primary-button-text on primary-button-background"),
        contrast(colours.primary_text, colours.primary_background),
    );
    preview.hide().expect("the preview can be hidden");
}

/// Where the page area starts and how high it is, as the application reads them to place popups
/// and size the page. Read straight after a property change, `controls-page-y` still has its old
/// value (measured: the notice was not yet in the layout); a draw brings the layout up to date.
fn page_area(window: &MainWindow) -> (f32, f32) {
    let _ = Frame::of(window.window(), None);
    (window.get_controls_page_y(), window.get_web_view_height())
}

/// The notice where the application puts it: in the main window's toolbar column.
fn notice_in_main_window(theme: AppTheme, name: &str) {
    let window = MainWindow::new().expect("the main window can be created");
    window.global::<AppPalette>().set_theme(theme);
    let colours = colours!(window.global::<AppPalette>());
    // The notice then sits directly above the page, with nothing else between.
    window.set_bookmarks_bar_shown(false);
    window.window().set_size(PhysicalSize::new(1000, 700));
    window.show().expect("the main window can be shown");

    let retries = Rc::new(Cell::new(0));
    let dismissals = Rc::new(Cell::new(0));
    let counted = Rc::clone(&retries);
    window.on_retry_password_save(move || counted.set(counted.get() + 1));
    let counted = Rc::clone(&dismissals);
    window.on_dismiss_password_save_problem(move || counted.set(counted.get() + 1));

    // No problem: the page area starts under the toolbar.
    let (page_top, page_height) = page_area(&window);
    assert!(
        page_top > 0.0 && page_height > 100.0,
        "{name}: {page_top} {page_height}"
    );

    // A problem: the page area moves down by the notice's height and gets that much shorter.
    window.set_password_save_problem(SHORT.into());
    assert_eq!(
        page_area(&window),
        (
            page_top + BAND_HEIGHT as f32,
            page_height - BAND_HEIGHT as f32
        ),
        "{name}"
    );
    let frame = Frame::of(
        window.window(),
        Some(&format!("main-window-save-notice-{name}")),
    );
    let band_y = page_top as usize;
    let band = locate(
        &frame,
        band_y,
        &colours,
        &format!("{name} main window 1000 px"),
    );

    // The buttons are wired to the window's callbacks.
    let middle = (band_y + (BUTTON_TOP + BUTTON_BOTTOM) / 2) as f32;
    click(window.window(), Band::middle(band.retry), middle);
    assert_eq!(
        (retries.get(), dismissals.get()),
        (1, 0),
        "{name}: Retry in the main window"
    );
    click(window.window(), Band::middle(band.dismiss), middle);
    assert_eq!(
        (retries.get(), dismissals.get()),
        (1, 1),
        "{name}: Dismiss in the main window"
    );
    // The window does not clear the problem itself; the application does.
    assert_eq!(window.get_password_save_problem(), SHORT);

    // On a narrow window, with a long site name, the buttons are still whole.
    window.set_password_save_problem(LONG.into());
    window.window().set_size(PhysicalSize::new(420, 700));
    let narrow = Frame::of(
        window.window(),
        Some(&format!("main-window-save-notice-{name}-420")),
    );
    let narrow_band = locate(
        &narrow,
        band_y,
        &colours,
        &format!("{name} main window 420 px"),
    );
    click(window.window(), Band::middle(narrow_band.retry), middle);
    assert_eq!(
        (retries.get(), dismissals.get()),
        (2, 1),
        "{name}: Retry at 420 px"
    );
    click(window.window(), Band::middle(narrow_band.dismiss), middle);
    assert_eq!(
        (retries.get(), dismissals.get()),
        (2, 2),
        "{name}: Dismiss at 420 px"
    );
    window.window().set_size(PhysicalSize::new(1000, 700));

    // Full-screen pages hide the chrome, the notice with it, and it comes back with the chrome.
    window.set_chrome_hidden(true);
    let (hidden_top, hidden_height) = page_area(&window);
    assert_eq!(hidden_top, 0.0, "{name}: the page fills the window");
    window.set_password_save_problem("".into());
    assert_eq!(
        page_area(&window),
        (hidden_top, hidden_height),
        "{name}: no notice, chrome hidden"
    );
    window.set_password_save_problem(SHORT.into());
    assert_eq!(
        page_area(&window),
        (hidden_top, hidden_height),
        "{name}: the notice shows in a full-screen page"
    );
    window.set_chrome_hidden(false);
    assert_eq!(
        page_area(&window),
        (
            page_top + BAND_HEIGHT as f32,
            page_height - BAND_HEIGHT as f32
        ),
        "{name}"
    );

    // Cleared, it is gone and the page area is back where it was.
    window.set_password_save_problem("".into());
    assert_eq!(page_area(&window), (page_top, page_height), "{name}");
    window.hide().expect("the main window can be hidden");
}

// One test: Slint's platform can be set once per process.
#[test]
fn the_password_save_notice_renders_and_takes_clicks_and_keys_in_both_themes() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).expect("no platform is set yet");
    for (theme, name) in [(ChromeTheme::Light, "light"), (ChromeTheme::Dark, "dark")] {
        notice_in_theme(theme, name);
    }
    for (theme, name) in [(AppTheme::Light, "light"), (AppTheme::Dark, "dark")] {
        notice_in_main_window(theme, name);
    }
}
