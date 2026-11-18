// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The file chooser for `<input type=file>`: the system open panel, attached to the browser
//! window as a sheet. Only the files the user chose are passed on.
//!
//! The sheet cannot cheaply ignore input for its first half second (AppKit runs it), so instead
//! of a delay it is only opened after a recent gesture in the tab (`super::admit`), and never
//! while another sheet is attached.
//!
//! The bookmarks lane's open and save panels for bookmark files are here too (at the end):
//! sheets on the bookmarks manager, with a flag of their own, so that they never hold back a
//! page's file chooser.

use std::cell::Cell;
use std::path::PathBuf;

use mercury_ui::MainWindow;

/// Called once, on the main thread, with the chosen files (empty if the user cancelled or the
/// sheet was ended).
pub(crate) type FilesChosen = Box<dyn FnOnce(Vec<PathBuf>)>;

thread_local! {
    static SHEET_ATTACHED: Cell<bool> = const { Cell::new(false) };
}

/// Whether a file sheet is attached to the window now.
pub(crate) fn sheet_attached() -> bool {
    SHEET_ATTACHED.with(Cell::get)
}

/// The panel's message. Servo does not say which frame asked, so it names the tab's page
/// without claiming that page will receive the files.
fn panel_message(multiple: bool, origin: &str) -> String {
    let what = if multiple { "files" } else { "a file" };
    format!("Choose {what} to upload to a page on {origin}")
}

/// An open sheet, so it can be taken down when its request goes away.
#[cfg(target_os = "macos")]
pub(crate) struct FileSheet {
    panel: objc2::rc::Retained<objc2_app_kit::NSOpenPanel>,
    window: objc2::rc::Retained<objc2_app_kit::NSWindow>,
}

#[cfg(target_os = "macos")]
impl FileSheet {
    /// Closes the sheet if it is still attached; its completion handler then reports no files.
    /// After the user chose or cancelled, the handler has already run and cleared the flag, so
    /// a panel that is no longer attached is left alone.
    pub(crate) fn end(&self) {
        if sheet_attached() {
            self.window.endSheet(&self.panel);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) struct FileSheet;

#[cfg(not(target_os = "macos"))]
impl FileSheet {
    pub(crate) fn end(&self) {}
}

/// The AppKit window of a Slint window, to attach a sheet to.
#[cfg(target_os = "macos")]
fn ns_window(window: &slint::Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2_app_kit::NSView;
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    window
        .with_winit_window(|winit_window| {
            let handle = winit_window.window_handle().ok()?;
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return None;
            };
            // SAFETY: winit's AppKit handle points at the window's content view, which lives as
            // long as the window; it is only used here, on the main thread, to find its window.
            let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
            view.window()
        })
        .flatten()
}

#[cfg(target_os = "macos")]
pub(crate) fn choose_files(
    window: &MainWindow,
    multiple: bool,
    extensions: &[String],
    origin: &str,
    done: FilesChosen,
) -> Option<FileSheet> {
    use block2::RcBlock;
    use objc2_app_kit::{NSModalResponse, NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{MainThreadMarker, NSArray, NSString};
    use slint::ComponentHandle;

    let Some(main_thread) = MainThreadMarker::new() else {
        log::error!("the file panel can only be opened on the main thread");
        done(Vec::new());
        return None;
    };
    let Some(ns_window) = ns_window(window.window()) else {
        log::error!("the browser window has no AppKit window to attach the file panel to");
        done(Vec::new());
        return None;
    };

    let panel = NSOpenPanel::openPanel(main_thread);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(multiple);
    panel.setMessage(Some(&NSString::from_str(&panel_message(multiple, origin))));
    if !extensions.is_empty() {
        let types = extensions
            .iter()
            .map(|extension| NSString::from_str(extension))
            .collect::<Vec<_>>();
        // `allowedContentTypes` needs `UTType` from `objc2-uniform-type-identifiers` 0.3, a
        // package not otherwise in the build; the older file-type list still works on current
        // macOS releases.
        #[expect(deprecated)]
        panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&types)));
    }

    let done = Cell::new(Some(done));
    let chosen_panel = panel.clone();
    let handler = RcBlock::new(move |response: NSModalResponse| {
        SHEET_ATTACHED.with(|attached| attached.set(false));
        let paths = if response == NSModalResponseOK {
            chosen_panel
                .URLs()
                .iter()
                .filter_map(|url| url.to_file_path())
                .collect()
        } else {
            Vec::new()
        };
        if let Some(done) = done.take() {
            done(paths);
        }
    });
    SHEET_ATTACHED.with(|attached| attached.set(true));
    // Returns at once; AppKit calls the handler on the main thread when the sheet closes.
    panel.beginSheetModalForWindow_completionHandler(&ns_window, &handler);
    Some(FileSheet {
        panel,
        window: ns_window,
    })
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn choose_files(
    _window: &MainWindow,
    multiple: bool,
    _extensions: &[String],
    origin: &str,
    done: FilesChosen,
) -> Option<FileSheet> {
    log::warn!(
        "no file chooser on this platform yet; answering \"no files\" ({})",
        panel_message(multiple, origin)
    );
    done(Vec::new());
    None
}

// bookmarks lane: the open and save panels for bookmark files.

/// Called once, on the main thread, with the chosen file (`None` if the user cancelled).
pub(crate) type FileChosen = Box<dyn FnOnce(Option<PathBuf>)>;

/// Whether this platform has the bookmark file panels (the manager says so otherwise).
pub(crate) const BOOKMARK_PANELS_AVAILABLE: bool = cfg!(target_os = "macos");

#[cfg(target_os = "macos")]
thread_local! {
    /// A bookmark file sheet is attached to the bookmarks manager.
    static BOOKMARK_SHEET_ATTACHED: Cell<bool> = const { Cell::new(false) };
}

/// Attaches `panel` to `window` as a sheet; `done` gets the panel's file. One at a time.
#[cfg(target_os = "macos")]
fn run_bookmark_sheet(
    window: &slint::Window,
    panel: objc2::rc::Retained<objc2_app_kit::NSSavePanel>,
    done: FileChosen,
) {
    use block2::RcBlock;
    use objc2_app_kit::{NSModalResponse, NSModalResponseOK};

    if BOOKMARK_SHEET_ATTACHED.with(Cell::get) {
        done(None);
        return;
    }
    let Some(ns_window) = ns_window(window) else {
        log::error!("the bookmarks manager has no AppKit window to attach a panel to");
        done(None);
        return;
    };
    let done = Cell::new(Some(done));
    let chosen_panel = panel.clone();
    let handler = RcBlock::new(move |response: NSModalResponse| {
        BOOKMARK_SHEET_ATTACHED.with(|attached| attached.set(false));
        let path = (response == NSModalResponseOK)
            .then(|| chosen_panel.URL())
            .flatten()
            .and_then(|url| url.to_file_path());
        if let Some(done) = done.take() {
            done(path);
        }
    });
    BOOKMARK_SHEET_ATTACHED.with(|attached| attached.set(true));
    // Returns at once; AppKit calls the handler on the main thread when the sheet closes.
    panel.beginSheetModalForWindow_completionHandler(&ns_window, &handler);
}

#[cfg(target_os = "macos")]
fn html_file_types() -> objc2::rc::Retained<objc2_foundation::NSArray<objc2_foundation::NSString>> {
    use objc2_foundation::{NSArray, NSString};
    NSArray::from_retained_slice(&[NSString::from_str("html"), NSString::from_str("htm")])
}

/// Asks for a bookmarks file to read, in a sheet on `window`.
#[cfg(target_os = "macos")]
pub(crate) fn choose_bookmarks_file(window: &slint::Window, done: FileChosen) {
    use objc2_app_kit::NSOpenPanel;
    use objc2_foundation::{MainThreadMarker, NSString};

    let Some(main_thread) = MainThreadMarker::new() else {
        done(None);
        return;
    };
    let panel = NSOpenPanel::openPanel(main_thread);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setMessage(Some(&NSString::from_str(
        "Choose a bookmarks file (HTML) exported from a browser",
    )));
    // As above: the older file-type list, without `objc2-uniform-type-identifiers`.
    #[expect(deprecated)]
    panel.setAllowedFileTypes(Some(&html_file_types()));
    run_bookmark_sheet(window, objc2::rc::Retained::into_super(panel), done);
}

/// Asks where to write a bookmarks file, in a sheet on `window`.
#[cfg(target_os = "macos")]
pub(crate) fn choose_bookmarks_export(
    window: &slint::Window,
    suggested_name: &str,
    done: FileChosen,
) {
    use objc2_app_kit::NSSavePanel;
    use objc2_foundation::{MainThreadMarker, NSString};

    let Some(main_thread) = MainThreadMarker::new() else {
        done(None);
        return;
    };
    let panel = NSSavePanel::savePanel(main_thread);
    panel.setNameFieldStringValue(&NSString::from_str(suggested_name));
    panel.setMessage(Some(&NSString::from_str(
        "Save your bookmarks as a file other browsers can import",
    )));
    #[expect(deprecated)]
    panel.setAllowedFileTypes(Some(&html_file_types()));
    run_bookmark_sheet(window, panel, done);
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn choose_bookmarks_file(_window: &slint::Window, done: FileChosen) {
    log::warn!("no file panel on this platform yet");
    done(None);
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn choose_bookmarks_export(
    _window: &slint::Window,
    _suggested_name: &str,
    done: FileChosen,
) {
    log::warn!("no file panel on this platform yet");
    done(None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_names_the_page_without_claiming_it_receives_the_files() {
        assert_eq!(
            panel_message(false, "https://upload.example"),
            "Choose a file to upload to a page on https://upload.example"
        );
        assert_eq!(
            panel_message(true, "https://upload.example"),
            "Choose files to upload to a page on https://upload.example"
        );
    }

    #[test]
    fn no_sheet_is_attached_at_first() {
        assert!(!sheet_attached());
    }
}
