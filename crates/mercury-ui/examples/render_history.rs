//! Draws the main window headless through Slint's software renderer, with the history lane's
//! UI open on sample data, and saves PNGs in both colour sets: the suggestion list, the history
//! window and the "Clear browsing data" dialog.
//!
//! `cargo run --release -p mercury-ui --example render_history -- <output directory>`

use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mercury_ui::{BrowserTab, HistoryRow, MainWindow, SuggestionRow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, Rgb8Pixel, SharedPixelBuffer, VecModel};

const WIDTH: u32 = 1180;
const HEIGHT: u32 = 760;
const SCALE: f32 = 2.0;

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

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

fn day(label: &str) -> HistoryRow {
    HistoryRow {
        is_day: true,
        label: label.into(),
        ..HistoryRow::default()
    }
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

fn sample(window: &MainWindow) {
    window.set_tabs(ModelRc::new(VecModel::from(vec![
        BrowserTab {
            title: "Example Domain".into(),
            active: true,
            ..BrowserTab::default()
        },
        BrowserTab {
            title: "The Rust Programming Language".into(),
            ..BrowserTab::default()
        },
    ])));
    window.set_is_new_tab(false);
    window.set_content_message("".into());
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
        suggestion(
            false,
            false,
            ["Rust by ", "Exa", "mple"],
            ["doc.rust-lang.org/rust-by-", "exa", "mple/"],
        ),
    ])));
    window.set_suggestion_selected(1);
    window.set_history_rows(ModelRc::new(VecModel::from(vec![
        day("Today"),
        visit(0, "14:05", "Example Domain", "example.com", "example.com"),
        visit(
            1,
            "13:52",
            "Rust by Example",
            "doc.rust-lang.org/rust-by-example/",
            "doc.rust-lang.org",
        ),
        visit(
            2,
            "09:17",
            "Unstyled page",
            "file:///Users/me/tests/pages/unstyled.html",
            "",
        ),
        day("Yesterday"),
        visit(
            3,
            "22:40",
            "The Rust Programming Language",
            "doc.rust-lang.org/book/",
            "doc.rust-lang.org",
        ),
        visit(
            4,
            "18:03",
            "Servo, the embeddable, independent, memory-safe web engine",
            "servo.org",
            "servo.org",
        ),
        day("Thursday 12 November 2026"),
        visit(
            5,
            "11:30",
            "Mercury Browser: release notes",
            "mercury.example/releases/0.2",
            "mercury.example",
        ),
        HistoryRow {
            is_more: true,
            ..HistoryRow::default()
        },
    ])));
}

fn render(
    platform_window: &MinimalSoftwareWindow,
    window: &MainWindow,
    path: &Path,
) -> Result<(), Box<dyn Error>> {
    window.show()?;
    let width = (WIDTH as f32 * SCALE) as u32;
    let height = (HEIGHT as f32 * SCALE) as u32;
    let mut buffer = SharedPixelBuffer::<Rgb8Pixel>::new(width, height);
    platform_window.request_redraw();
    platform_window.draw_if_needed(|renderer| {
        renderer.render(buffer.make_mut_slice(), width as usize);
    });
    image::save_buffer(
        path,
        buffer.as_bytes(),
        width,
        height,
        image::ExtendedColorType::Rgb8,
    )?;
    println!("wrote {}", path.display());
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_history <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;
    let platform_window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    platform_window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
        scale_factor: SCALE,
    });
    platform_window.set_size(PhysicalSize::new(
        (WIDTH as f32 * SCALE) as u32,
        (HEIGHT as f32 * SCALE) as u32,
    ));
    slint::platform::set_platform(Box::new(Headless {
        window: platform_window.clone(),
    }))
    .map_err(|error| format!("{error:?}"))?;

    let window = MainWindow::new()?;
    sample(&window);
    for (theme, dark) in [("dark", true), ("light", false)] {
        window.set_history_dark(dark);

        window.set_history_open(false);
        window.set_clear_dialog_open(false);
        window.set_address("exa".into());
        window.set_suggestions_open(true);
        window.invoke_focus_address_bar();
        window.invoke_complete_address("example.com".into(), 3, 11);
        render(
            &platform_window,
            &window,
            &output.join(format!("suggestions-{theme}.png")),
        )?;

        window.set_suggestions_open(false);
        window.set_address("https://example.com/".into());
        window.set_history_open(true);
        render(
            &platform_window,
            &window,
            &output.join(format!("history-{theme}.png")),
        )?;

        window.set_clear_dialog_open(true);
        window.set_clear_period(1);
        window.set_clear_cookies(true);
        render(
            &platform_window,
            &window,
            &output.join(format!("clear-{theme}.png")),
        )?;
    }
    Ok(())
}
