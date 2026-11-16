//! Renders the import window's pages to PNG files without a display, through Slint's software
//! renderer, in the light and dark schemes and with the scheme left to the system.
//!
//! `cargo run --release -p mercury-ui --example render_import -- <output directory>`

use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mercury_ui::{ImportPage, ImportProfileRow, ImportSummaryLine, ImportWindow};
use slint::language::ColorScheme;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

const SCALE: f32 = 2.0;

struct HeadlessPlatform;

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

fn render(window: &slint::Window, path: &Path) -> Result<(), Box<dyn Error>> {
    window.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: SCALE,
    });
    window.set_size(PhysicalSize::new(1120, 960));
    // Let layouts and bindings settle.
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let _ = window.take_snapshot();
    }
    let snapshot = window.take_snapshot()?;
    let bytes = snapshot
        .as_slice()
        .iter()
        .flat_map(|pixel| [pixel.r, pixel.g, pixel.b])
        .collect::<Vec<_>>();
    image::save_buffer(
        path,
        &bytes,
        snapshot.width(),
        snapshot.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    println!("wrote {}", path.display());
    Ok(())
}

fn row(browser: &str, name: &str, holds: &str, bookmarks: bool, history: bool) -> ImportProfileRow {
    ImportProfileRow {
        browser: browser.into(),
        name: name.into(),
        holds: holds.into(),
        bookmarks,
        history,
    }
}

fn line(text: &str, problem: bool) -> ImportSummaryLine {
    ImportSummaryLine {
        text: text.into(),
        problem,
    }
}

fn sample(scheme: ColorScheme) -> Result<ImportWindow, Box<dyn Error>> {
    let window = ImportWindow::new()?;
    window.set_forced_color_scheme(scheme);
    window.set_profiles(ModelRc::new(VecModel::from(vec![
        row(
            "Firefox",
            "default-release",
            "Bookmarks and history",
            true,
            true,
        ),
        row("Chrome", "Work", "Bookmarks and history", true, true),
        row("Chrome", "Reading", "Bookmarks", true, false),
        row("Arc", "Default", "History", false, true),
    ])));
    window.set_selected(1);
    window.set_import_bookmarks(true);
    window.set_import_history(true);
    Ok(window)
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_import <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;
    slint::platform::set_platform(Box::new(HeadlessPlatform))
        .map_err(|error| format!("{error:?}"))?;

    for (scheme, name) in [
        (ColorScheme::Light, "light"),
        (ColorScheme::Dark, "dark"),
        (ColorScheme::Unknown, "system"),
    ] {
        let window = sample(scheme)?;
        window.show()?;

        window.set_page(ImportPage::Choose);
        render(
            window.window(),
            &output.join(format!("import-{name}-1-choose.png")),
        )?;

        window.set_unfinished_notice(
            "The import from Firefox — default-release did not finish.".into(),
        );
        window.set_selected(2);
        window.set_import_history(false);
        render(
            window.window(),
            &output.join(format!("import-{name}-2-choose-unfinished.png")),
        )?;

        window.set_page(ImportPage::NothingFound);
        render(
            window.window(),
            &output.join(format!("import-{name}-3-nothing.png")),
        )?;

        window.set_page(ImportPage::Importing);
        window.set_progress_title("Importing history from Firefox — default-release…".into());
        window.set_progress_detail("12,000 of 38,912 visits".into());
        window.set_progress_known(true);
        window.set_progress(12_000.0 / 38_912.0);
        render(
            window.window(),
            &output.join(format!("import-{name}-4-importing.png")),
        )?;

        window.set_page(ImportPage::Summary);
        window.set_summary(ModelRc::new(VecModel::from(vec![
            line(
                "Imported 1,204 bookmarks and 38,912 visits from Firefox — default-release.",
                false,
            ),
            line(
                "12 bookmarks and 1,000 visits were already in Mercury Browser and were left out.",
                false,
            ),
            line(
                "2 visits could not be kept because their addresses are too long.",
                true,
            ),
        ])));
        render(
            window.window(),
            &output.join(format!("import-{name}-5-summary.png")),
        )?;

        window.set_summary_title("Import stopped".into());
        window.set_summary(ModelRc::new(VecModel::from(vec![
            line("Imported 500 visits from Chrome — Work.", false),
            line(
                "Bookmarks could not be imported: Chrome kept writing to its files while they \
                 were being read. Quit Chrome and try again.",
                true,
            ),
            line(
                "The import of history stopped before it finished: 500 of 38,912 had been \
                 imported. Import again to add the rest; nothing will be added twice.",
                true,
            ),
        ])));
        render(
            window.window(),
            &output.join(format!("import-{name}-6-summary-problems.png")),
        )?;
        window.set_summary_title("Import finished".into());
        window.hide()?;
    }
    Ok(())
}
