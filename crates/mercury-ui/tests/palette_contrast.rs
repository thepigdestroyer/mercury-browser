// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The chrome palette's accent colours: readable (WCAG 2.2 AA, 4.5:1 for text and 3:1 for
//! interface parts such as focus rings, underlines and button shapes) on every surface they are
//! drawn on, in Light and in Dark, and drawn from the Mercury brand (Sun orange) rather than the
//! blue inherited from Hermes. Run with `--nocapture` to print every ratio.
//!
//! The second test draws the rows that are filled with `item-selected` (the selected address-bar
//! suggestion, the selected folder and row of the bookmarks manager, a pressed history button)
//! and measures what is drawn on that fill: the ordinary text, secondary and accent colours do
//! not read on it, so everything on it must take `item-selected-text`. Set `MERCURY_UI_RENDER_DIR`
//! to also save each rendering as a PNG.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use mercury_ui::{
    AboutWindow, BookmarkMenuRow, BookmarkRowKind, BookmarksManager, ChromePalette, ChromeTheme,
    HistoryRow, MainWindow, SuggestionRow,
};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{Color, ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, VecModel};

thread_local! {
    /// The windows the platform has made on this thread, oldest first, so that a test can reach
    /// the software window behind a component it has built.
    static WINDOWS: RefCell<Vec<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(Vec::new()) };

    /// The platform's clock. Slint times its animations (the rows fade) by `duration_since_start`;
    /// with the wall clock a picture taken a few milliseconds earlier or later shows a different
    /// frame, so each drawing step moves this clock by a fixed amount instead.
    static CLOCK: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        WINDOWS.with(|windows| windows.borrow_mut().push(window.clone()));
        Ok(window)
    }

    fn duration_since_start(&self) -> Duration {
        CLOCK.with(Cell::get)
    }
}

/// How far the clock moves before each drawing step.
const STEP: Duration = Duration::from_millis(100);

fn advance_clock() {
    CLOCK.with(|clock| clock.set(clock.get() + STEP));
}

/// Sets the platform for this thread. Slint keeps it per thread, so each test (which runs on its
/// own thread) sets it once; when the harness runs the tests one after another on a single
/// thread, the second call finds the first one's platform, which is the same.
fn install_platform() {
    let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
}

const TEXT: f64 = 4.5;
const UI: f64 = 3.0;

fn channel(value: u8) -> f64 {
    let c = f64::from(value) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(color: Color) -> f64 {
    0.2126 * channel(color.red()) + 0.7152 * channel(color.green()) + 0.0722 * channel(color.blue())
}

fn contrast(a: Color, b: Color) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Hue in degrees (0..360) of an opaque colour.
fn hue(color: Color) -> f64 {
    let [r, g, b] = [color.red(), color.green(), color.blue()].map(|v| f64::from(v) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let delta = max - min;
    if delta == 0.0 {
        return 0.0;
    }
    let h = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    h * 60.0
}

fn hex(color: Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        color.red(),
        color.green(),
        color.blue()
    )
}

#[test]
fn accents_are_readable_and_on_brand_in_both_appearances() {
    install_platform();
    let window = AboutWindow::new().expect("a window to read the palette from");
    let palette = window.global::<ChromePalette>();
    let mut failures = Vec::new();

    for (theme, name) in [(ChromeTheme::Light, "light"), (ChromeTheme::Dark, "dark")] {
        palette.set_theme(theme);
        let p = &palette;
        let surfaces = [
            ("window-background", p.get_window_background()),
            ("toolbar-background", p.get_toolbar_background()),
            ("content-background", p.get_content_background()),
            ("tab-active", p.get_tab_active()),
            ("field-background", p.get_field_background()),
            ("popup-background", p.get_popup_background()),
            ("sheet-background", p.get_sheet_background()),
        ];
        // (foreground role, colour, background role, colour, minimum)
        let mut pairs: Vec<(&str, Color, &str, Color, f64)> = Vec::new();
        for (surface, background) in surfaces {
            // Links (About), checked boxes, the active-tab underline, the loading bar and
            // focused field borders.
            pairs.push(("accent", p.get_accent(), surface, background, TEXT));
            pairs.push(("focus-ring", p.get_focus_ring(), surface, background, UI));
            pairs.push((
                "button-label-hover",
                p.get_button_label_hover(),
                surface,
                background,
                TEXT,
            ));
            pairs.push((
                "primary-button-background",
                p.get_primary_button_background(),
                surface,
                background,
                UI,
            ));
        }
        pairs.push((
            "match-highlight",
            p.get_match_highlight(),
            "popup-background",
            p.get_popup_background(),
            TEXT,
        ));
        pairs.push((
            "match-highlight",
            p.get_match_highlight(),
            "field-background",
            p.get_field_background(),
            TEXT,
        ));
        pairs.push((
            "item-selected",
            p.get_item_selected(),
            "popup-background",
            p.get_popup_background(),
            UI,
        ));
        pairs.push((
            "item-selected-text",
            p.get_item_selected_text(),
            "item-selected",
            p.get_item_selected(),
            TEXT,
        ));
        pairs.push((
            "item-selected",
            p.get_item_selected(),
            "page-popup-background",
            p.get_page_popup_background(),
            UI,
        ));
        for (role, background) in [
            (
                "primary-button-background",
                p.get_primary_button_background(),
            ),
            ("primary-button-hover", p.get_primary_button_hover()),
            ("primary-button-pressed", p.get_primary_button_pressed()),
            ("accent", p.get_accent()),
        ] {
            pairs.push((
                "primary-button-text",
                p.get_primary_button_text(),
                role,
                background,
                TEXT,
            ));
        }
        pairs.push((
            "info-text",
            p.get_info_text(),
            "info-background",
            p.get_info_background(),
            TEXT,
        ));
        pairs.push((
            "text",
            p.get_text(),
            "info-background",
            p.get_info_background(),
            TEXT,
        ));

        // The rows of the suggestion list, the bookmarks manager and the history panel. On their
        // own fills (the surface, or the hover tint) the title and the secondary line take `text`
        // and `text-secondary`, a bookmark's star and a plain history button's label take
        // `accent` or `danger`.
        for (surface, background) in [
            ("popup-background", p.get_popup_background()),
            ("content-background", p.get_content_background()),
            ("item-hover", p.get_item_hover()),
        ] {
            pairs.push(("text", p.get_text(), surface, background, TEXT));
            pairs.push((
                "text-secondary",
                p.get_text_secondary(),
                surface,
                background,
                TEXT,
            ));
        }
        for (surface, background) in [
            ("content-background", p.get_content_background()),
            ("item-hover", p.get_item_hover()),
        ] {
            pairs.push(("danger", p.get_danger(), surface, background, TEXT));
        }
        pairs.push((
            "accent",
            p.get_accent(),
            "item-hover",
            p.get_item_hover(),
            TEXT,
        ));
        // A selected row, a selected folder and a pressed plain history button are filled with
        // `item-selected`, where none of those colours reads (text 3.09, secondary lines and the
        // star about 1 in Light): everything drawn on that fill, the title, the secondary line,
        // the icons, a separator and a label, takes `item-selected-text`, so it needs 4.5:1 as
        // text (which also covers the 3:1 an icon needs). The pair "item-selected-text on
        // item-selected" is checked above; the second test draws the rows to check that they use
        // it.

        for (fg_role, fg, bg_role, bg, minimum) in pairs {
            let ratio = contrast(fg, bg);
            println!(
                "{name:5} {fg_role:26} {} on {bg_role:26} {}: {ratio:5.2} (min {minimum})",
                hex(fg),
                hex(bg)
            );
            if ratio < minimum {
                failures.push(format!(
                    "{name}: {fg_role} {} on {bg_role} {} is {ratio:.2}, needs {minimum}",
                    hex(fg),
                    hex(bg)
                ));
            }
        }

        // Hermes' blues sit at hues 195–215°; the brand accent is the Sun's orange (15–45°).
        for (role, color) in [
            ("accent", p.get_accent()),
            ("button-label-hover", p.get_button_label_hover()),
            (
                "primary-button-background",
                p.get_primary_button_background(),
            ),
            ("primary-button-hover", p.get_primary_button_hover()),
            ("primary-button-pressed", p.get_primary_button_pressed()),
            ("focus-ring", p.get_focus_ring()),
            ("match-highlight", p.get_match_highlight()),
            ("item-selected", p.get_item_selected()),
        ] {
            let h = hue(color);
            if !(15.0..=45.0).contains(&h) {
                failures.push(format!(
                    "{name}: {role} {} has hue {h:.0}°, not the brand orange",
                    hex(color)
                ));
            }
        }
        for (role, color) in [
            ("info-background", p.get_info_background()),
            ("info-border", p.get_info_border()),
        ] {
            let h = hue(color);
            if (180.0..=240.0).contains(&h) {
                failures.push(format!(
                    "{name}: {role} {} is still blue (hue {h:.0}°)",
                    hex(color)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

const SCALE: f32 = 2.0;

/// Physical pixels for a length in logical pixels.
fn px(logical: f32) -> usize {
    (logical * SCALE).round() as usize
}

/// A rendering, in physical pixels.
struct Shot {
    width: usize,
    pixels: Vec<PremultipliedRgbaColor>,
}

impl Shot {
    fn pixel(&self, x: usize, y: usize) -> Color {
        let pixel = &self.pixels[y * self.width + x];
        Color::from_rgb_u8(pixel.red, pixel.green, pixel.blue)
    }

    /// Saves the rendering as a PNG when `MERCURY_UI_RENDER_DIR` is set.
    fn save(&self, name: &str) {
        let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR") else {
            return;
        };
        let bytes = self
            .pixels
            .iter()
            .flat_map(|pixel| [pixel.red, pixel.green, pixel.blue, 255])
            .collect::<Vec<_>>();
        let height = self.pixels.len() / self.width;
        image::save_buffer(
            std::path::Path::new(&directory).join(format!("{name}.png")),
            &bytes,
            self.width as u32,
            height as u32,
            image::ExtendedColorType::Rgba8,
        )
        .expect("the PNG is written");
    }
}

/// A rectangle of physical pixels; `right` and `bottom` are one past the last pixel.
#[derive(Clone, Copy, Debug)]
struct Region {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}

impl Region {
    fn height(self) -> usize {
        self.bottom - self.top
    }

    /// This region with `horizontal` and `vertical` logical pixels taken off each side, to keep
    /// the antialiased corners of a rounded fill out of it.
    fn inset(self, horizontal: f32, vertical: f32) -> Self {
        Self {
            left: self.left + px(horizontal),
            top: self.top + px(vertical),
            right: self.right - px(horizontal),
            bottom: self.bottom - px(vertical),
        }
    }
}

/// The window behind the newest component, and the means to draw it and to press its pointer.
struct Screen {
    window: Rc<MinimalSoftwareWindow>,
    width: usize,
    height: usize,
}

impl Screen {
    /// `width` and `height` are in logical pixels; call it after showing the component.
    fn of_newest_window(width: u32, height: u32) -> Self {
        let window = WINDOWS
            .with(|windows| windows.borrow().last().cloned())
            .expect("the platform made a window");
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: SCALE,
        });
        let size = PhysicalSize::new(
            (width as f32 * SCALE) as u32,
            (height as f32 * SCALE) as u32,
        );
        window.set_size(size);
        Self {
            window,
            width: size.width as usize,
            height: size.height as usize,
        }
    }

    fn whole(&self) -> Region {
        Region {
            left: 0,
            top: 0,
            right: self.width,
            bottom: self.height,
        }
    }

    fn render(&self) -> Shot {
        let mut pixels = vec![PremultipliedRgbaColor::default(); self.width * self.height];
        // Twice: the first drawing settles the layout.
        for _ in 0..2 {
            advance_clock();
            slint::platform::update_timers_and_animations();
            self.window.request_redraw();
            self.window.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, self.width);
            });
        }
        Shot {
            width: self.width,
            pixels,
        }
    }

    /// Presses the left button at a position in logical pixels and keeps it down.
    fn press(&self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.window.dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
    }

    /// Lets go of the left button at a position in logical pixels.
    fn release(&self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.window.dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    }
}

/// The rectangle of the row or button that `fill` belongs to: the smallest one holding every
/// pixel of `within` that is exactly `fill`, in the pixel rows that have at least `at_least` such
/// pixels. (In Light, `accent` is the colour of `item-selected` too, so a star, an icon or a
/// border drawn in it elsewhere in `within` must not stretch the rectangle: it is narrower.)
/// Without the antialiased pixels of the rounded corners.
fn bounds_of_fill(shot: &Shot, fill: Color, within: Region, at_least: usize) -> Option<Region> {
    let mut bounds: Option<Region> = None;
    for y in within.top..within.bottom {
        let columns = (within.left..within.right)
            .filter(|&x| shot.pixel(x, y) == fill)
            .collect::<Vec<_>>();
        let (Some(&first), Some(&last)) = (columns.first(), columns.last()) else {
            continue;
        };
        if columns.len() < at_least {
            continue;
        }
        bounds = Some(match bounds {
            None => Region {
                left: first,
                top: y,
                right: last + 1,
                bottom: y + 1,
            },
            Some(region) => Region {
                left: region.left.min(first),
                top: region.top,
                right: region.right.max(last + 1),
                bottom: y + 1,
            },
        });
    }
    bounds
}

/// The best contrast with `fill` that any pixel of `region` reaches: what the most legible part
/// of whatever is drawn there achieves, and 1.0 when nothing is drawn.
fn strongest_contrast(shot: &Shot, region: Region, fill: Color) -> f64 {
    let mut best = 1.0_f64;
    for y in region.top..region.bottom {
        for x in region.left..region.right {
            best = best.max(contrast(shot.pixel(x, y), fill));
        }
    }
    best
}

/// What the rendering tests measure, and what failed.
#[derive(Default)]
struct Checks {
    failures: Vec<String>,
}

impl Checks {
    /// Records that `what`, drawn in `region` of `shot` on `fill`, reaches `minimum`.
    fn expect(&mut self, what: &str, shot: &Shot, region: Region, fill: Color, minimum: f64) {
        let ratio = strongest_contrast(shot, region, fill);
        println!("{what:52} {ratio:5.2} (min {minimum})");
        if ratio < minimum {
            self.failures
                .push(format!("{what}: contrast {ratio:.2}, needs {minimum}"));
        }
    }

    /// Records that `what` was not found where the test looks for it, so its geometry is stale.
    fn missing(&mut self, what: &str) {
        self.failures
            .push(format!("{what}: not drawn where expected"));
    }
}

const THEMES: [(ChromeTheme, &str); 2] =
    [(ChromeTheme::Light, "light"), (ChromeTheme::Dark, "dark")];

fn suggestion(
    is_search: bool,
    is_bookmark: bool,
    title: [&str; 3],
    url: [&str; 3],
) -> SuggestionRow {
    SuggestionRow {
        is_search,
        is_bookmark,
        title_before: title[0].into(),
        title_match: title[1].into(),
        title_after: title[2].into(),
        url_before: url[0].into(),
        url_match: url[1].into(),
        url_after: url[2].into(),
    }
}

/// Each selected row of the address bar's suggestion list: its icon (a magnifier, a star or a
/// clock), its title and, below it, the address.
fn selected_suggestions(checks: &mut Checks) {
    let window = MainWindow::new().expect("the main window builds");
    window.show().expect("the main window shows");
    let screen = Screen::of_newest_window(1180, 760);
    window.set_suggestions(ModelRc::new(VecModel::from(vec![
        suggestion(
            true,
            false,
            ["Search DuckDuckGo for “exa”", "", ""],
            ["", "", ""],
        ),
        suggestion(
            false,
            true,
            ["", "Exa", "mple Domain"],
            ["", "exa", "mple.com"],
        ),
        suggestion(
            false,
            false,
            ["", "Exa", "mple docs: getting started"],
            ["", "exa", "mple.org/docs/getting-started"],
        ),
    ])));
    window.set_address("exa".into());
    window.set_suggestions_open(true);
    window.invoke_focus_address_bar();
    assert!(
        window.get_address_focused(),
        "the suggestion list only shows while the address field has focus"
    );

    // The list hangs under the address field (which ends at y 97); only it is searched, since in
    // Light the address field's focused border and the new-tab page's button have the colour of
    // `item-selected` too.
    let list_area = Region {
        top: px(100.0),
        bottom: px(300.0),
        ..screen.whole()
    };

    for (theme, name) in THEMES {
        let palette = window.global::<ChromePalette>();
        palette.set_theme(theme);
        let fill = palette.get_item_selected();
        for (selected, kind) in ["search", "bookmark", "history"].into_iter().enumerate() {
            window.set_suggestion_selected(selected as i32);
            let shot = screen.render();
            shot.save(&format!("suggestions-{name}-{kind}"));
            let what = format!("{name} suggestion ({kind})");
            let Some(row) = bounds_of_fill(&shot, fill, list_area, px(40.0)) else {
                checks.missing(&what);
                continue;
            };
            // Rows are 46 px high.
            if row.height().abs_diff(px(46.0)) > 2 {
                checks.missing(&format!("{what} row of 46 px"));
                continue;
            }
            let middle = row.top + row.height() / 2;
            // The icon, 18 px wide after 12 px of padding; then the text, 12 px further on.
            let icon = Region {
                left: row.left + px(12.0),
                top: row.top,
                right: row.left + px(30.0),
                bottom: row.bottom,
            };
            let text = Region {
                left: row.left + px(42.0),
                right: row.right - px(8.0),
                ..icon
            };
            checks.expect(&format!("{what} icon"), &shot, icon, fill, UI);
            checks.expect(
                &format!("{what} title"),
                &shot,
                Region {
                    bottom: middle,
                    ..text
                },
                fill,
                TEXT,
            );
            // The search row has no address line.
            if selected > 0 {
                checks.expect(
                    &format!("{what} address"),
                    &shot,
                    Region {
                        top: middle,
                        ..text
                    },
                    fill,
                    TEXT,
                );
            }
        }
    }
    window.hide().expect("the main window hides");
}

fn bookmark_row(title: &str, url: &str, kind: BookmarkRowKind, depth: i32) -> BookmarkMenuRow {
    BookmarkMenuRow {
        title: title.into(),
        url: url.into(),
        kind,
        depth,
    }
}

/// The selected folder of the bookmarks manager's tree and the selected row of its list.
fn selected_bookmarks(checks: &mut Checks) {
    let window = BookmarksManager::new().expect("the bookmarks manager builds");
    window.show().expect("the bookmarks manager shows");
    let screen = Screen::of_newest_window(920, 620);
    window.set_folders(ModelRc::new(VecModel::from(vec![
        bookmark_row("Bookmarks Bar", "", BookmarkRowKind::Folder, 0),
        bookmark_row("News", "", BookmarkRowKind::Folder, 1),
        bookmark_row("Local", "", BookmarkRowKind::Folder, 2),
    ])));
    window.set_rows(ModelRc::new(VecModel::from(vec![
        bookmark_row("Local", "", BookmarkRowKind::Folder, 0),
        bookmark_row(
            "The Guardian",
            "https://www.theguardian.com/international",
            BookmarkRowKind::Bookmark,
            0,
        ),
        bookmark_row("", "", BookmarkRowKind::Separator, 0),
    ])));
    window.set_heading("News".into());

    // The tree is the left 252 px of the window (12 px of padding and 240 px); the list is on
    // its right.
    let divide = px(257.0);
    let tree_area = Region {
        right: divide,
        ..screen.whole()
    };
    let list_area = Region {
        left: divide,
        ..screen.whole()
    };

    for (theme, name) in THEMES {
        let palette = window.global::<ChromePalette>();
        palette.set_theme(theme);
        let fill = palette.get_item_selected();

        // A folder: 8 px of padding, 16 px for each level of depth, the 16 px icon, 8 px, then
        // the name. The rows are 28 px high.
        for depth in 0..3 {
            window.set_folder_index(depth);
            let shot = screen.render();
            shot.save(&format!("bookmarks-{name}-folder-{depth}"));
            let what = format!("{name} bookmarks folder (depth {depth})");
            let Some(row) = bounds_of_fill(&shot, fill, tree_area, px(40.0)) else {
                checks.missing(&what);
                continue;
            };
            if row.height().abs_diff(px(28.0)) > 2 {
                checks.missing(&format!("{what} row of 28 px"));
                continue;
            }
            let icon_left = row.left + px(8.0 + 16.0 * depth as f32);
            let icon = Region {
                left: icon_left,
                right: icon_left + px(16.0),
                ..row.inset(0.0, 3.0)
            };
            let title = Region {
                left: icon_left + px(24.0),
                right: row.right - px(8.0),
                ..icon
            };
            checks.expect(&format!("{what} icon"), &shot, icon, fill, UI);
            checks.expect(&format!("{what} name"), &shot, title, fill, TEXT);
        }

        // A list row: 10 px of padding, the 16 px icon, 10 px, the title and the address in the
        // ratio 2 to 3, 10 px apart. The rows are 30 px high.
        window.set_folder_index(-1);
        for (index, kind) in ["folder", "bookmark", "separator"].into_iter().enumerate() {
            window.set_row_index(index as i32);
            let shot = screen.render();
            shot.save(&format!("bookmarks-{name}-row-{kind}"));
            let what = format!("{name} bookmarks row ({kind})");
            let Some(row) = bounds_of_fill(&shot, fill, list_area, px(40.0)) else {
                checks.missing(&what);
                continue;
            };
            if row.height().abs_diff(px(30.0)) > 2 {
                checks.missing(&format!("{what} row of 30 px"));
                continue;
            }
            if kind == "separator" {
                // The line across the row, between 12 px margins.
                checks.expect(
                    &format!("{what} line"),
                    &shot,
                    row.inset(12.0, 0.0),
                    fill,
                    UI,
                );
                continue;
            }
            let icon_left = row.left + px(10.0);
            let icon = Region {
                left: icon_left,
                right: icon_left + px(16.0),
                ..row.inset(0.0, 3.0)
            };
            let width = (row.right - row.left) as f32 / SCALE;
            let title_width = (width - 56.0) * 2.0 / 5.0;
            let title = Region {
                left: icon_left + px(26.0),
                right: icon_left + px(26.0 + title_width),
                ..icon
            };
            let address = Region {
                left: title.right + px(10.0),
                right: row.right - px(8.0),
                ..icon
            };
            checks.expect(&format!("{what} icon"), &shot, icon, fill, UI);
            checks.expect(&format!("{what} title"), &shot, title, fill, TEXT);
            if kind == "bookmark" {
                checks.expect(&format!("{what} address"), &shot, address, fill, TEXT);
            }
        }
    }
    window.hide().expect("the bookmarks manager hides");
}

fn visit(entry: i32, time: &str, title: &str, url: &str, host: &str) -> HistoryRow {
    HistoryRow {
        time: time.into(),
        title: title.into(),
        url: url.into(),
        host: host.into(),
        entry,
        ..HistoryRow::default()
    }
}

/// Plain buttons of the history panel while the pointer is down on them: filled with
/// `item-selected`, with the accent label ("Close") or the danger one ("Delete").
fn pressed_history_buttons(checks: &mut Checks) {
    let window = MainWindow::new().expect("the main window builds");
    window.show().expect("the main window shows");
    let screen = Screen::of_newest_window(1180, 760);
    window.set_history_rows(ModelRc::new(VecModel::from(vec![
        HistoryRow {
            is_day: true,
            label: "Today".into(),
            ..HistoryRow::default()
        },
        visit(0, "14:05", "Example Domain", "example.com", "example.com"),
    ])));
    window.set_history_open(true);

    // The centres of the buttons in logical pixels, as the window lays them out at 1180 x 760:
    // "Close" in the header, then "Delete" on the visit. Each is 28 px high, so the pressed fill is
    // searched for within 20 px above and below the press.
    let buttons = [("Close", 990.0, 179.0), ("Delete", 1115.0, 273.0)];
    for (theme, name) in THEMES {
        let palette = window.global::<ChromePalette>();
        palette.set_theme(theme);
        let fill = palette.get_item_selected();
        for (label, x, y) in buttons {
            let what = format!("{name} history button ({label}) pressed");
            screen.press(x, y);
            let shot = screen.render();
            shot.save(&format!("history-{name}-{}-pressed", label.to_lowercase()));
            let nearby = Region {
                left: px(x - 70.0),
                top: px(y - 20.0),
                right: px(x + 70.0),
                bottom: px(y + 20.0),
            };
            // Let go away from the button, so that nothing is clicked.
            screen.release(600.0, 600.0);
            let Some(button) = bounds_of_fill(&shot, fill, nearby, px(30.0)) else {
                checks.missing(&what);
                continue;
            };
            if button.height().abs_diff(px(28.0)) > 2 {
                checks.missing(&format!("{what} button of 28 px"));
                continue;
            }
            // The label, without the button's rounded corners.
            checks.expect(
                &format!("{what} label"),
                &shot,
                button.inset(6.0, 3.0),
                fill,
                TEXT,
            );
        }
    }
    window.hide().expect("the main window hides");
}

#[test]
fn text_and_icons_on_selected_rows_are_readable_as_drawn() {
    install_platform();
    let mut checks = Checks::default();
    selected_suggestions(&mut checks);
    selected_bookmarks(&mut checks);
    pressed_history_buttons(&mut checks);
    assert!(
        checks.failures.is_empty(),
        "\n{}",
        checks.failures.join("\n")
    );
}
