// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the PDF viewer headlessly with Slint's software renderer and checks how it answers
//! keys, clicks, the wheel and the model's data. Set `MERCURY_UI_RENDER_DIR` to also save each
//! rendering as a PNG.
//!
//! The page tiles here are synthetic: the real ones come from the PDF helper process.

use std::cell::RefCell;
use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{
    ComponentHandle, Image, LogicalPosition, ModelRc, PhysicalSize, Rgba8Pixel, SharedPixelBuffer,
    SharedString, VecModel,
};

slint::slint! {
    import { PdfViewer } from "../ui/pdf/pdf-viewer.slint";
    import {
        PdfFitMode, PdfPageRow, PdfPointer, PdfRectRow, PdfScrollKey, PdfTileRow, PdfViewState,
        PdfViewerModel,
    } from "../ui/pdf/pdf-model.slint";
    import { ChromePalette, ChromeTheme } from "../ui/theme/chrome-palette.slint";

    export {
        ChromePalette, ChromeTheme, PdfFitMode, PdfPageRow, PdfPointer, PdfRectRow, PdfScrollKey,
        PdfTileRow, PdfViewState, PdfViewerModel,
    }

    // A stand-in for the browser window: the viewer fills it.
    export component Harness inherits Window {
        out property <bool> text-input-focused: PdfViewerModel.text-focused;

        public function focus-viewer() {
            viewer.focus-viewer();
        }

        preferred-width: 1000px;
        preferred-height: 640px;
        background: ChromePalette.window-background;

        // In the browser window the viewer is inside the page area, a plain rectangle: its own
        // layout constraints do not reach the window.
        Rectangle {
            viewer := PdfViewer {
                width: parent.width;
                height: parent.height;
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
const WIDTH: u32 = 2000;
const HEIGHT: u32 = 1280;

/// The window is 1000 x 640 logical pixels: a 40 px toolbar, then the page area below it.
const AREA_WIDTH: f32 = 1000.0;
const AREA_HEIGHT: f32 = 600.0;
const TOOLBAR_HEIGHT: f32 = 40.0;

/// The synthetic document: pages of 700 x 900 logical pixels stacked with 8 px gaps in a content
/// area 1000 wide, so each page sits at x = 150. Tiles are 256 logical pixels (512 device pixels
/// at the 2x scale the tiles pretend to have been rendered at).
const PAGE_WIDTH: f32 = 700.0;
const PAGE_HEIGHT: f32 = 900.0;
const PAGE_GAP: f32 = 8.0;
const PAGE_COUNT: i32 = 12;
const TILE: f32 = 256.0;

/// A pixel of the synthetic page: white paper, a heading, lines of "words", a grey figure.
fn page_pixel(page: u32, x: f32, y: f32) -> [u8; 4] {
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const INK: [u8; 4] = [34, 38, 46, 255];
    let in_heading = (60.0..92.0).contains(&y) && (60.0..380.0).contains(&x);
    if in_heading {
        return INK;
    }
    let in_figure = (560.0..760.0).contains(&y) && (400.0..640.0).contains(&x);
    if in_figure {
        return [196, 204, 214, 255];
    }
    // Lines of text every 20 px from y = 120, 8 px tall, with a gap every 70 px or so (a word
    // space); the line's length depends on the page so the pages differ.
    let line = ((y - 120.0) / 20.0).floor();
    let row_y = (y - 120.0) - line * 20.0;
    if (120.0..540.0).contains(&y) && row_y < 8.0 {
        let length = 580.0 - ((line as u32 + page) % 5) as f32 * 40.0;
        let along = x - 60.0;
        if (0.0..length).contains(&along) && (along % 70.0) < 62.0 {
            return INK;
        }
    }
    WHITE
}

/// One tile of the synthetic page, as the image the helper process would have sent.
fn tile_image(page: u32, tile_x: f32, tile_y: f32, width: f32, height: f32) -> Image {
    let device = 2.0;
    let (pixel_width, pixel_height) = ((width * device) as u32, (height * device) as u32);
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(pixel_width, pixel_height);
    let bytes = buffer.make_mut_bytes();
    for row in 0..pixel_height {
        for column in 0..pixel_width {
            let x = tile_x + (column as f32 + 0.5) / device;
            let y = tile_y + (row as f32 + 0.5) / device;
            let offset = ((row * pixel_width + column) * 4) as usize;
            bytes[offset..offset + 4].copy_from_slice(&page_pixel(page, x, y));
        }
    }
    Image::from_rgba8(buffer)
}

fn page_top(page: i32) -> f32 {
    PAGE_GAP + page as f32 * (PAGE_HEIGHT + PAGE_GAP)
}

fn content_height() -> f32 {
    PAGE_GAP + PAGE_COUNT as f32 * (PAGE_HEIGHT + PAGE_GAP)
}

const PAGE_LEFT: f32 = (AREA_WIDTH - PAGE_WIDTH) / 2.0;

/// The pages and tiles that cover `scroll_y .. scroll_y + AREA_HEIGHT` of the content, which is
/// what the view model's plan gives the viewer.
fn visible_rows(scroll_y: f32) -> (Vec<PdfPageRow>, Vec<PdfTileRow>) {
    let mut pages = Vec::new();
    let mut tiles = Vec::new();
    for page in 0..PAGE_COUNT {
        let top = page_top(page);
        if top + PAGE_HEIGHT < scroll_y || top > scroll_y + AREA_HEIGHT {
            continue;
        }
        pages.push(PdfPageRow {
            x: PAGE_LEFT,
            y: top,
            width: PAGE_WIDTH,
            height: PAGE_HEIGHT,
        });
        let mut tile_y = 0.0;
        while tile_y < PAGE_HEIGHT {
            let height = TILE.min(PAGE_HEIGHT - tile_y);
            let visible =
                top + tile_y + height >= scroll_y && top + tile_y <= scroll_y + AREA_HEIGHT;
            if visible {
                let mut tile_x = 0.0;
                while tile_x < PAGE_WIDTH {
                    let width = TILE.min(PAGE_WIDTH - tile_x);
                    tiles.push(PdfTileRow {
                        x: PAGE_LEFT + tile_x,
                        y: top + tile_y,
                        width,
                        height,
                        image: tile_image(page as u32, tile_x, tile_y, width, height),
                    });
                    tile_x += TILE;
                }
            }
            tile_y += TILE;
        }
    }
    (pages, tiles)
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
    calls: Rc<RefCell<Vec<String>>>,
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
        let calls = Rc::new(RefCell::new(Vec::new()));
        let model = harness.global::<PdfViewerModel>();
        let record = |name: &'static str| {
            let calls = Rc::clone(&calls);
            move || calls.borrow_mut().push(name.to_owned())
        };
        model.on_zoom_in(record("zoom in"));
        model.on_zoom_out(record("zoom out"));
        model.on_fit_width(record("fit width"));
        model.on_fit_page(record("fit page"));
        model.on_rotate(record("rotate"));
        model.on_find(record("find"));
        model.on_save_copy(record("save a copy"));
        model.on_open_externally(record("open externally"));
        model.on_retry(record("try again"));
        model.on_copy_selection(record("copy"));
        model.on_escape(record("escape"));
        let calls_for = Rc::clone(&calls);
        model.on_go_to_page(move |text| calls_for.borrow_mut().push(format!("go to {text}")));
        let calls_for = Rc::clone(&calls);
        model.on_submit_password(move |text| {
            calls_for.borrow_mut().push(format!("password {text}"));
        });
        let calls_for = Rc::clone(&calls);
        model.on_scroll_key(move |key| {
            let name = match key {
                PdfScrollKey::LineDown => "line down",
                PdfScrollKey::LineUp => "line up",
                PdfScrollKey::LineLeft => "line left",
                PdfScrollKey::LineRight => "line right",
                PdfScrollKey::PageDown => "page down",
                PdfScrollKey::PageUp => "page up",
                PdfScrollKey::Top => "top",
                PdfScrollKey::Bottom => "bottom",
            };
            calls_for.borrow_mut().push(name.to_owned());
        });
        let calls_for = Rc::clone(&calls);
        model.on_pointer(move |kind, x, y| {
            let kind = match kind {
                PdfPointer::Down => "down",
                PdfPointer::Up => "up",
                PdfPointer::Moved => "moved",
                PdfPointer::Cancel => "cancel",
            };
            calls_for
                .borrow_mut()
                .push(format!("pointer {kind} {x:.0} {y:.0}"));
        });
        let calls_for = Rc::clone(&calls);
        model.on_scrolled(move |dx, dy| {
            calls_for
                .borrow_mut()
                .push(format!("scrolled {dx:.0} {dy:.0}"));
        });
        let calls_for = Rc::clone(&calls);
        model.on_viewport_changed(move |width, height| {
            calls_for
                .borrow_mut()
                .push(format!("viewport {width:.0} {height:.0}"));
        });
        Self {
            window,
            harness,
            calls,
        }
    }

    fn model(&self) -> PdfViewerModel<'_> {
        self.harness.global::<PdfViewerModel>()
    }

    /// The viewer showing the first pages of the synthetic document.
    fn show_pages(&self, scroll_y: f32) {
        let model = self.model();
        let (pages, tiles) = visible_rows(scroll_y);
        model.set_state(PdfViewState::Ready);
        model.set_title("Quarterly report.pdf".into());
        model.set_page(((scroll_y + AREA_HEIGHT / 2.0) / (PAGE_HEIGHT + PAGE_GAP)) as i32 + 1);
        model.set_page_count(PAGE_COUNT);
        model.set_zoom_percent(100);
        model.set_fit(PdfFitMode::Width);
        model.set_content_width(AREA_WIDTH);
        model.set_content_height(content_height());
        model.set_scroll_x(0.0);
        model.set_scroll_y(scroll_y);
        model.set_pages(ModelRc::new(VecModel::from(pages)));
        model.set_tiles(ModelRc::new(VecModel::from(tiles)));
        model.set_selection(ModelRc::new(VecModel::<PdfRectRow>::default()));
        model.set_hits(ModelRc::new(VecModel::<PdfRectRow>::default()));
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

    fn save(&self, name: &str) {
        self.settle();
        let pixels = self.render();
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

    /// The pixel at a logical position of the window.
    fn pixel(&self, x: f32, y: f32) -> PremultipliedRgbaColor {
        let pixels = self.render();
        let column = (x * SCALE) as u32;
        let row = (y * SCALE) as u32;
        pixels[(row * WIDTH + column) as usize]
    }

    fn press(&self, key: impl Into<SharedString>) {
        self.window
            .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    }

    fn release(&self, key: impl Into<SharedString>) {
        self.window
            .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    }

    fn key(&self, key: impl Into<SharedString>) {
        let key = key.into();
        self.press(key.clone());
        self.release(key);
        self.settle();
    }

    fn key_with(&self, modifier: Key, key: impl Into<SharedString>) {
        self.press(modifier);
        self.key(key);
        self.release(modifier);
    }

    fn click(&self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.window.dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        self.window.dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
        self.settle();
    }

    fn scroll(&self, x: f32, y: f32, delta_x: f32, delta_y: f32) {
        self.window.dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(x, y),
            delta_x,
            delta_y,
        });
        self.settle();
    }

    fn take_calls(&self) -> Vec<String> {
        std::mem::take(&mut self.calls.borrow_mut())
    }
}

// One test: Slint's platform is global to the process.
#[test]
fn the_pdf_viewer_renders_and_answers_keys_clicks_and_the_model() {
    let fixture = Fixture::new();
    let harness = &fixture.harness;

    // Loading: only a sentence.
    harness
        .global::<PdfViewerModel>()
        .set_title("Quarterly report.pdf".into());
    fixture.save("pdf-loading-dark");
    assert!(
        fixture.take_calls().is_empty(),
        "no page area exists while the file is opening"
    );

    // Pages: the area reports its size once it exists, and the paper and a tile are drawn.
    fixture.show_pages(0.0);
    fixture.settle();
    assert_eq!(
        fixture.take_calls(),
        vec![format!("viewport {AREA_WIDTH:.0} {AREA_HEIGHT:.0}")]
    );
    let paper = fixture.pixel(PAGE_LEFT + 20.0, TOOLBAR_HEIGHT + PAGE_GAP + 20.0);
    assert_eq!(
        (paper.red, paper.green, paper.blue),
        (255, 255, 255),
        "the paper is white"
    );
    let ink = fixture.pixel(PAGE_LEFT + 100.0, TOOLBAR_HEIGHT + PAGE_GAP + 75.0);
    assert!(luminance(&ink) < 0.05, "the heading's tile shows dark text");
    let backdrop = fixture.pixel(20.0, TOOLBAR_HEIGHT + 300.0);
    assert!(
        luminance(&backdrop) < 0.05,
        "the area beside the page is the dark backdrop"
    );
    fixture.save("pdf-pages-dark");

    // Light chrome around the same white paper.
    harness
        .global::<ChromePalette>()
        .set_theme(ChromeTheme::Light);
    fixture.save("pdf-pages-light");
    let paper_light = fixture.pixel(PAGE_LEFT + 20.0, TOOLBAR_HEIGHT + PAGE_GAP + 20.0);
    assert_eq!(
        (paper_light.red, paper_light.green, paper_light.blue),
        (255, 255, 255),
        "the paper is not inverted in the light theme either"
    );
    let backdrop_light = fixture.pixel(20.0, TOOLBAR_HEIGHT + 300.0);
    assert!(
        luminance(&backdrop_light) > 0.4,
        "the light backdrop is light"
    );
    let _ = fixture.take_calls();

    // The toolbar's buttons answer clicks (positions read off the rendering: the strip is 40
    // px high, so every button's middle is at y = 20).
    for (x, name) in [
        (405.0, "zoom out"),
        (484.0, "zoom in"),
        (549.0, "fit width"),
        (617.0, "fit page"),
        (694.0, "rotate"),
        (748.0, "find"),
        (831.0, "save a copy"),
        (933.0, "open externally"),
    ] {
        fixture.click(x, 20.0);
        assert_eq!(fixture.take_calls(), vec![name], "the button at x = {x}");
    }
    // A disabled button answers nothing.
    fixture.model().set_can_save_copy(false);
    fixture.model().set_can_zoom_in(false);
    fixture.settle();
    fixture.click(831.0, 20.0);
    fixture.click(484.0, 20.0);
    assert_eq!(fixture.take_calls(), Vec::<String>::new());
    fixture.model().set_can_save_copy(true);
    fixture.model().set_can_zoom_in(true);

    // The keys of a document, with the keyboard in the pages.
    harness.invoke_focus_viewer();
    fixture.settle();
    for (key, name) in [
        (SharedString::from(" "), "page down"),
        (Key::PageDown.into(), "page down"),
        (Key::PageUp.into(), "page up"),
        (Key::Home.into(), "top"),
        (Key::End.into(), "bottom"),
        (Key::DownArrow.into(), "line down"),
        (Key::UpArrow.into(), "line up"),
        (Key::LeftArrow.into(), "line left"),
        (Key::RightArrow.into(), "line right"),
        (Key::Escape.into(), "escape"),
    ] {
        fixture.key(key);
        assert_eq!(fixture.take_calls(), vec![name], "the key for {name}");
    }
    fixture.key_with(Key::Shift, " ");
    assert_eq!(fixture.take_calls(), vec!["page up"], "Shift+Space");
    fixture.key_with(Key::Control, "c");
    assert_eq!(
        fixture.take_calls(),
        vec!["copy"],
        "Cmd+C (Ctrl+C elsewhere)"
    );
    // Other shortcuts are left to the browser, and letters mean nothing.
    fixture.key_with(Key::Control, " ");
    fixture.key_with(Key::Control, Key::PageDown);
    fixture.key("x");
    assert_eq!(fixture.take_calls(), Vec::<String>::new());

    // The pointer: a click in the pages reports the press and the release, in the page area's
    // own coordinates (the strip is 40 px high); the wheel reports its delta.
    fixture.click(500.0, 300.0);
    let calls = fixture.take_calls();
    assert!(
        calls.contains(&"pointer down 500 260".to_owned())
            && calls.contains(&"pointer up 500 260".to_owned()),
        "{calls:?}"
    );
    assert!(
        calls.iter().position(|call| call == "pointer down 500 260")
            < calls.iter().position(|call| call == "pointer up 500 260"),
        "{calls:?}"
    );
    fixture.scroll(500.0, 300.0, 4.0, -30.0);
    let calls = fixture.take_calls();
    assert!(
        calls.contains(&"scrolled 4 -30".to_owned()),
        "the wheel reports its delta: {calls:?}"
    );

    // The page field: clicking it takes the keyboard (so browser shortcuts that move the text
    // cursor leave it alone), a typed number goes to the page, and Return gives the keyboard back.
    assert!(!harness.get_text_input_focused());
    fixture.click(309.0, 20.0);
    assert!(harness.get_text_input_focused());
    fixture.key("5");
    fixture.key(Key::Return);
    assert_eq!(fixture.take_calls(), vec!["go to 5"]);
    assert!(
        !harness.get_text_input_focused(),
        "the keyboard is back in the pages"
    );
    // Escape drops what was typed.
    fixture.click(309.0, 20.0);
    fixture.key("9");
    fixture.key(Key::Escape);
    assert_eq!(fixture.take_calls(), Vec::<String>::new());
    assert!(!harness.get_text_input_focused());

    // A selection over two lines of text, and find matches (the third is the current one).
    harness
        .global::<ChromePalette>()
        .set_theme(ChromeTheme::Dark);
    let model = fixture.model();
    model.set_selection(ModelRc::new(VecModel::from(vec![
        rect(PAGE_LEFT + 60.0, page_top(0) + 120.0, 520.0, 8.0, false),
        rect(PAGE_LEFT + 60.0, page_top(0) + 140.0, 310.0, 8.0, false),
    ])));
    model.set_hits(ModelRc::new(VecModel::from(vec![
        rect(PAGE_LEFT + 60.0, page_top(0) + 197.0, 62.0, 16.0, false),
        rect(PAGE_LEFT + 200.0, page_top(0) + 237.0, 62.0, 16.0, false),
        rect(PAGE_LEFT + 340.0, page_top(0) + 277.0, 62.0, 16.0, true),
    ])));
    model.set_find_open(true);
    fixture.save("pdf-highlights-dark");
    let selected = fixture.pixel(PAGE_LEFT + 100.0, TOOLBAR_HEIGHT + page_top(0) + 124.0);
    assert!(
        selected.red > selected.blue.saturating_add(30),
        "selected text is tinted warm over its ink, not left plain: {selected:?}"
    );
    let hit = fixture.pixel(PAGE_LEFT + 80.0, TOOLBAR_HEIGHT + page_top(0) + 210.0);
    assert!(
        hit.red > 200 && hit.green > 150 && hit.blue < 160,
        "a find match is yellow: {hit:?}"
    );
    let current = fixture.pixel(PAGE_LEFT + 380.0, TOOLBAR_HEIGHT + page_top(0) + 290.0);
    assert!(
        current.red > 200 && current.green < hit.green,
        "the current match is the stronger orange: {current:?} against {hit:?}"
    );
    model.set_selection(ModelRc::new(VecModel::<PdfRectRow>::default()));
    model.set_hits(ModelRc::new(VecModel::<PdfRectRow>::default()));
    model.set_find_open(false);

    // The password prompt, then with the error line.
    // The application clears the flag when the state changes (a field that goes away cannot say
    // that it lost the keyboard), and tells the prompt it may take the keyboard.
    model.set_text_focused(false);
    model.set_take_focus(true);
    model.set_state(PdfViewState::Password);
    model.set_message("".into());
    fixture.save("pdf-password-dark");
    assert!(
        harness.get_text_input_focused(),
        "the password field takes the keyboard when the prompt appears"
    );
    // "Open" waits for a password (clicking it elsewhere takes the keyboard from the field, as any
    // click outside a field does); Return or "Open" sends it, and the field is emptied.
    fixture.click(658.0, 311.0);
    assert_eq!(fixture.take_calls(), Vec::<String>::new());
    fixture.click(500.0, 270.0);
    assert!(harness.get_text_input_focused());
    for letter in "secret".chars() {
        fixture.key(letter.to_string());
    }
    fixture.key(Key::Return);
    assert_eq!(fixture.take_calls(), vec!["password secret"]);
    for letter in "again".chars() {
        fixture.key(letter.to_string());
    }
    fixture.click(658.0, 311.0);
    assert_eq!(fixture.take_calls(), vec!["password again"]);
    model.set_message("That password is not right.".into());
    fixture.save("pdf-password-wrong-dark");
    harness
        .global::<ChromePalette>()
        .set_theme(ChromeTheme::Light);
    fixture.save("pdf-password-wrong-light");

    // The error state.
    model.set_text_focused(false);
    model.set_state(PdfViewState::Error);
    model.set_message("The PDF viewer stopped working on this file.".into());
    fixture.save("pdf-error-light");
    for (x, name) in [
        (405.0, "save a copy"),
        (535.0, "open externally"),
        (655.0, "try again"),
    ] {
        fixture.click(x, 283.0);
        assert_eq!(
            fixture.take_calls(),
            vec![name],
            "the error button at x = {x}"
        );
    }
    harness
        .global::<ChromePalette>()
        .set_theme(ChromeTheme::Dark);
    fixture.save("pdf-error-dark");

    // A narrow window gets the toolbar's short words, and pages wider than the window scroll
    // sideways (a thin indicator shows where).
    model.set_text_focused(false);
    fixture.show_pages(0.0);
    fixture.window.set_size(PhysicalSize::new(1400, HEIGHT));
    model.set_content_width(1600.0);
    model.set_scroll_x(300.0);
    fixture.save("pdf-pages-narrow-dark");
    let _ = fixture.take_calls();
}

fn rect(x: f32, y: f32, width: f32, height: f32, current: bool) -> PdfRectRow {
    PdfRectRow {
        x,
        y,
        width,
        height,
        current,
    }
}

/// The viewer's Slint sources: every control is named for assistive technology, and no colour is
/// written out except in the viewer's own palette file.
#[test]
fn the_viewer_sources_name_every_control_and_use_only_palette_colours() {
    let controls = include_str!("../ui/pdf/pdf-controls.slint");
    let toolbar = include_str!("../ui/pdf/pdf-toolbar.slint");
    let pages = include_str!("../ui/pdf/pdf-pages.slint");
    let panels = include_str!("../ui/pdf/pdf-panels.slint");
    let viewer = include_str!("../ui/pdf/pdf-viewer.slint");
    let model = include_str!("../ui/pdf/pdf-model.slint");

    // Both kinds of button: a role, a name, a state and a default action, and the caption inside
    // is hidden from assistive technology (the button says it).
    assert_eq!(controls.matches("accessible-role: button;").count(), 2);
    assert_eq!(controls.matches("accessible-label:").count(), 2);
    assert_eq!(controls.matches("accessible-enabled:").count(), 2);
    assert_eq!(controls.matches("accessible-action-default").count(), 2);
    assert_eq!(
        controls.matches("Text {").count(),
        controls.matches("accessible-role: none;").count(),
        "every caption inside a button is hidden"
    );
    // The toolbar is a navigation landmark with a name; the page field says what it is for.
    assert!(toolbar.contains("accessible-role: AccessibleRole.navigation;"));
    assert!(toolbar.contains("accessible-label: \"PDF toolbar\";"));
    assert!(toolbar.contains("accessible-label: \"Page number\";"));
    // Every button of the toolbar has a spoken name that is not its glyph.
    for name in [
        "Zoom out",
        "Zoom in",
        "Fit width",
        "Fit page",
        "Rotate clockwise",
        "Find in document",
        "Save a copy",
        "Open in Preview",
    ] {
        assert!(
            toolbar.contains(&format!("tooltip: \"{name}\";")),
            "the toolbar names its {name:?} button"
        );
    }
    // The page area says what it is and where it is.
    assert!(pages.contains("accessible-role: AccessibleRole.main;"));
    assert!(pages.contains("accessible-label: \"PDF document, page \""));
    // The password field is a `RequestField` in its password mode (the field that keeps a typed
    // password from assistive technology; `a11y_secrets.rs` allows a password input type nowhere
    // else), labelled "Password". The field gives that word as its label and as its placeholder,
    // and hides the drawn placeholder, so it is not read twice.
    assert!(panels.contains("field := RequestField {"));
    assert!(panels.contains("accessible-name: \"Password\";"));
    assert!(panels.contains("placeholder: \"Password\";"));
    assert!(panels.contains("password: true;"));
    assert!(
        !panels.contains("input-type:"),
        "the panels write no input type of their own"
    );
    assert!(panels.contains("accessible-role: groupbox;"));

    // Colours come from the palettes: no colour literal anywhere else.
    for (name, source) in [
        ("pdf-controls", controls),
        ("pdf-toolbar", toolbar),
        ("pdf-pages", pages),
        ("pdf-panels", panels),
        ("pdf-viewer", viewer),
        ("pdf-model", model),
    ] {
        for (index, line) in source.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            let has_literal = line.split('#').skip(1).any(|rest| {
                let digits = rest
                    .chars()
                    .take_while(|character| character.is_ascii_hexdigit())
                    .count();
                matches!(digits, 3 | 4 | 6 | 8)
            });
            assert!(
                !has_literal,
                "{name}.slint:{} writes a colour out: {line}",
                index + 1
            );
        }
    }
}
