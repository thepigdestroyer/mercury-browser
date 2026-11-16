//! Renders the bookmarks UI (the bar with folders and overflow, the star and its popover, a
//! folder popup, the manager) to PNG files without a display, through Slint's software
//! renderer, in the light, dark and system colour schemes.
//!
//! `cargo run --release -p mercury-ui --example render_bookmarks -- <output directory>`

use std::cell::RefCell;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mercury_ui::{
    BookmarkBarItem, BookmarkColors, BookmarkMenuRow, BookmarkRowKind, BookmarksManager,
    BookmarksPreview, MainWindow,
};
use slint::language::ColorScheme;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, Model, ModelRc, PhysicalSize, SharedString, VecModel};

const SCALE: f32 = 2.0;
/// The desktop app's `bookmarks::layout` constants.
const SPACING: f32 = 4.0;
const OVERFLOW_BUTTON: f32 = 28.0;

struct HeadlessPlatform;

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

fn settle(window: &slint::Window, width: u32, height: u32) {
    window.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: SCALE,
    });
    window.set_size(PhysicalSize::new(width, height));
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let _ = window.take_snapshot();
    }
}

fn render(
    window: &slint::Window,
    width: u32,
    height: u32,
    path: &Path,
) -> Result<(), Box<dyn Error>> {
    settle(window, width, height);
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

const BAR: &[(&str, bool)] = &[
    ("Mercury Browser", false),
    ("News", true),
    ("Rust documentation", false),
    ("Work", true),
    ("A page with a rather long title that is cut short", false),
    ("Weather", false),
    ("Recipes", true),
    ("Maps", false),
    ("Music", false),
    ("Photos", false),
];

fn bar_items() -> Rc<VecModel<BookmarkBarItem>> {
    Rc::new(VecModel::from(
        BAR.iter()
            .map(|(title, is_folder)| BookmarkBarItem {
                title: (*title).into(),
                is_folder: *is_folder,
                x: 0.0,
                width: 0.0,
                shown: false,
            })
            .collect::<Vec<_>>(),
    ))
}

/// The desktop app's overflow rule (`bookmarks::layout::bar_layout`), applied to the model.
fn lay_out(items: &VecModel<BookmarkBarItem>, widths: &[f32], available: f32) -> bool {
    let fit = |limit: f32| {
        let mut positions = Vec::new();
        let mut x = 0.0;
        for width in widths {
            if x + width > limit {
                break;
            }
            positions.push(x);
            x += width + SPACING;
        }
        positions
    };
    let mut positions = fit(available);
    let overflow = positions.len() < widths.len();
    if overflow {
        positions = fit(available - OVERFLOW_BUTTON - SPACING);
    }
    for (index, width) in widths.iter().enumerate().take(items.row_count()) {
        let mut item = items.row_data(index).unwrap();
        match positions.get(index) {
            Some(x) => {
                item.x = *x;
                item.width = *width;
                item.shown = true;
            }
            None => item.shown = false,
        }
        items.set_row_data(index, item);
    }
    overflow
}

fn row(title: &str, url: &str, kind: BookmarkRowKind, depth: i32) -> BookmarkMenuRow {
    BookmarkMenuRow {
        title: title.into(),
        url: url.into(),
        kind,
        depth,
    }
}

fn menu_rows() -> ModelRc<BookmarkMenuRow> {
    ModelRc::new(VecModel::from(vec![
        row("News", "", BookmarkRowKind::Back, 0),
        row("Local", "", BookmarkRowKind::Folder, 0),
        row(
            "BBC News",
            "https://www.bbc.co.uk/news",
            BookmarkRowKind::Bookmark,
            0,
        ),
        row(
            "The Guardian",
            "https://www.theguardian.com/",
            BookmarkRowKind::Bookmark,
            0,
        ),
        row("", "", BookmarkRowKind::Separator, 0),
        row(
            "Hacker News",
            "https://news.ycombinator.com/",
            BookmarkRowKind::Bookmark,
            0,
        ),
    ]))
}

fn folder_labels() -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        [
            "Bookmarks Bar",
            "\u{2003}News",
            "\u{2003}\u{2003}Local",
            "\u{2003}Work",
            "Other Bookmarks",
            "\u{2003}Imported 2026-11-16",
        ]
        .into_iter()
        .map(SharedString::from)
        .collect::<Vec<_>>(),
    ))
}

/// Shows the bar, lets the buttons report their widths, and lays them out.
fn measured_bar(
    measured: &Rc<RefCell<Vec<f32>>>,
    items: &VecModel<BookmarkBarItem>,
    available: f32,
) -> bool {
    let widths = measured.borrow().clone();
    lay_out(items, &widths, available)
}

fn preview(scheme: ColorScheme, output: &Path, name: &str) -> Result<(), Box<dyn Error>> {
    let window = BookmarksPreview::new()?;
    window.set_forced_color_scheme(scheme);
    let items = bar_items();
    let measured = Rc::new(RefCell::new(vec![0.0; BAR.len()]));
    let report = Rc::clone(&measured);
    window.on_bar_measured(move |index, width| {
        report.borrow_mut()[index as usize] = width;
    });
    window.set_bar_items(ModelRc::from(Rc::clone(&items)));
    window.set_menu_rows(menu_rows());
    window.set_popover_name("Mercury Browser — a fast, private browser".into());
    window.set_popover_folders(folder_labels());
    window.set_popover_folder_index(4);
    window.show()?;
    settle(window.window(), 1520, 1040);
    let overflow = measured_bar(&measured, &items, window.get_bar_available_width());
    window.set_bar_overflow(overflow);
    render(
        window.window(),
        1520,
        1040,
        &output.join(format!("preview-{name}.png")),
    )?;
    window.hide()?;
    Ok(())
}

fn main_window(scheme: ColorScheme, output: &Path, name: &str) -> Result<(), Box<dyn Error>> {
    let window = MainWindow::new()?;
    window.global::<BookmarkColors>().set_forced(scheme);
    let items = bar_items();
    let measured = Rc::new(RefCell::new(vec![0.0; BAR.len()]));
    let report = Rc::clone(&measured);
    window.on_bookmarks_bar_measured(move |index, width| {
        report.borrow_mut()[index as usize] = width;
    });
    window.set_bookmarks_bar_items(ModelRc::from(Rc::clone(&items)));
    window.set_address("https://mercury.example/".into());
    window.set_page_title("Mercury".into());
    window.set_is_new_tab(false);
    window.set_bookmark_star_enabled(true);
    window.set_bookmark_star_filled(true);
    window.set_bookmark_popover_is_new(false);
    window.set_bookmark_popover_name("Mercury Browser".into());
    window.set_bookmark_popover_folders(folder_labels());
    window.set_bookmark_popover_folder_index(0);
    window.set_bookmarks_menu_rows(menu_rows());
    window.show()?;
    settle(window.window(), 1800, 1000);
    let overflow = measured_bar(
        &measured,
        &items,
        window.get_bookmarks_bar_available_width(),
    );
    window.set_bookmarks_bar_overflow(overflow);
    render(
        window.window(),
        1800,
        1000,
        &output.join(format!("window-{name}.png")),
    )?;

    let folder_x = items.row_data(1).map_or(0.0, |item| item.x);
    window.invoke_show_bookmarks_menu(folder_x);
    render(
        window.window(),
        1800,
        1000,
        &output.join(format!("window-{name}-folder.png")),
    )?;
    window.invoke_close_bookmarks_menu();

    window.invoke_open_bookmark_popover();
    render(
        window.window(),
        1800,
        1000,
        &output.join(format!("window-{name}-popover.png")),
    )?;
    window.invoke_close_bookmark_popover();

    window.hide()?;
    Ok(())
}

fn manager(scheme: ColorScheme, output: &Path, name: &str) -> Result<(), Box<dyn Error>> {
    let window = BookmarksManager::new()?;
    window.set_forced_color_scheme(scheme);
    window.set_folders(ModelRc::new(VecModel::from(vec![
        row("Bookmarks Bar", "", BookmarkRowKind::Folder, 0),
        row("News", "", BookmarkRowKind::Folder, 1),
        row("Local", "", BookmarkRowKind::Folder, 2),
        row("Work", "", BookmarkRowKind::Folder, 1),
        row("Other Bookmarks", "", BookmarkRowKind::Folder, 0),
        row("Imported 2026-11-16", "", BookmarkRowKind::Folder, 1),
    ])));
    window.set_folder_index(1);
    window.set_heading("News".into());
    window.set_rows(ModelRc::new(VecModel::from(vec![
        row("Local", "", BookmarkRowKind::Folder, 0),
        row(
            "BBC News",
            "https://www.bbc.co.uk/news",
            BookmarkRowKind::Bookmark,
            0,
        ),
        row(
            "The Guardian",
            "https://www.theguardian.com/international",
            BookmarkRowKind::Bookmark,
            0,
        ),
        row("", "", BookmarkRowKind::Separator, 0),
        row(
            "Hacker News",
            "https://news.ycombinator.com/",
            BookmarkRowKind::Bookmark,
            0,
        ),
        row(
            "Lobsters",
            "https://lobste.rs/",
            BookmarkRowKind::Bookmark,
            0,
        ),
    ])));
    window.set_row_index(2);
    window.set_editing(true);
    window.set_editing_bookmark(true);
    window.set_edit_name("The Guardian".into());
    window.set_edit_url("https://www.theguardian.com/international".into());
    window.set_move_targets(folder_labels());
    window.set_move_index(1);
    window.set_can_move_up(true);
    window.set_can_move_down(true);
    window.set_can_undo(true);
    window.set_undo_label("Undo delete of “Weather”".into());
    window.set_problem("Enter a full address, such as https://example.com/.".into());
    window.set_status("Imported 1,204 items into “Imported 2026-11-16”.".into());
    window.show()?;
    render(
        window.window(),
        1840,
        1240,
        &output.join(format!("manager-{name}.png")),
    )?;

    window.set_search_text("guardian".into());
    window.set_searching(true);
    window.set_heading("1 result for “guardian”".into());
    window.set_rows(ModelRc::new(VecModel::from(vec![row(
        "The Guardian",
        "https://www.theguardian.com/international",
        BookmarkRowKind::Bookmark,
        0,
    )])));
    window.set_row_index(-1);
    window.set_editing(false);
    window.set_problem(SharedString::default());
    render(
        window.window(),
        1840,
        1240,
        &output.join(format!("manager-{name}-search.png")),
    )?;
    window.hide()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_bookmarks <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;
    slint::platform::set_platform(Box::new(HeadlessPlatform))
        .map_err(|error| format!("{error:?}"))?;
    for (scheme, name) in [
        (ColorScheme::Light, "light"),
        (ColorScheme::Dark, "dark"),
        (ColorScheme::Unknown, "system"),
    ] {
        preview(scheme, &output, name)?;
        main_window(scheme, &output, name)?;
        manager(scheme, &output, name)?;
    }
    Ok(())
}
