//! The system open and save panels for bookmark files, as sheets on the bookmarks manager.
//!
//! The controls lane is adding an `NSOpenPanel` module for `<input type=file>` that is not
//! merged into this branch; this is the smallest version for this lane's two panels (the same
//! `objc2-app-kit` features, no new package), to be folded into that module at the merge.

use std::path::PathBuf;

use mercury_ui::BookmarksManager;

/// Called once, on the main thread, with the chosen file (`None` if the user cancelled).
pub(crate) type FileChosen = Box<dyn FnOnce(Option<PathBuf>)>;

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::Cell;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSModalResponse, NSModalResponseOK, NSOpenPanel, NSSavePanel, NSView, NSWindow,
    };
    use objc2_foundation::{MainThreadMarker, NSArray, NSString};
    use slint::ComponentHandle;
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::FileChosen;
    use mercury_ui::BookmarksManager;

    thread_local! {
        static SHEET_ATTACHED: Cell<bool> = const { Cell::new(false) };
    }

    fn ns_window(window: &BookmarksManager) -> Option<Retained<NSWindow>> {
        window
            .window()
            .with_winit_window(|winit_window| {
                let handle = winit_window.window_handle().ok()?;
                let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                    return None;
                };
                // SAFETY: winit's AppKit handle points at the window's content view, which
                // lives as long as the window; it is only used here, on the main thread, to
                // find its window.
                let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
                view.window()
            })
            .flatten()
    }

    /// Attaches `panel` to the manager as a sheet; `done` gets the panel's file.
    fn run_sheet(window: &BookmarksManager, panel: Retained<NSSavePanel>, done: FileChosen) {
        if SHEET_ATTACHED.with(Cell::get) {
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
            SHEET_ATTACHED.with(|attached| attached.set(false));
            let path = (response == NSModalResponseOK)
                .then(|| chosen_panel.URL())
                .flatten()
                .and_then(|url| url.to_file_path());
            if let Some(done) = done.take() {
                done(path);
            }
        });
        SHEET_ATTACHED.with(|attached| attached.set(true));
        // Returns at once; AppKit calls the handler on the main thread when the sheet closes.
        panel.beginSheetModalForWindow_completionHandler(&ns_window, &handler);
    }

    fn html_types() -> Retained<NSArray<NSString>> {
        NSArray::from_retained_slice(&[NSString::from_str("html"), NSString::from_str("htm")])
    }

    pub(super) fn choose_file_to_open(window: &BookmarksManager, done: FileChosen) {
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
        // `allowedContentTypes` needs `objc2-uniform-type-identifiers`, a package not in the
        // build; the older file-type list still works on current macOS releases.
        #[expect(deprecated)]
        panel.setAllowedFileTypes(Some(&html_types()));
        run_sheet(window, Retained::into_super(panel), done);
    }

    pub(super) fn choose_file_to_save(
        window: &BookmarksManager,
        suggested_name: &str,
        done: FileChosen,
    ) {
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
        panel.setAllowedFileTypes(Some(&html_types()));
        run_sheet(window, panel, done);
    }
}

/// Asks for a bookmarks file to read.
pub(crate) fn choose_file_to_open(window: &BookmarksManager, done: FileChosen) {
    #[cfg(target_os = "macos")]
    macos::choose_file_to_open(window, done);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        log::warn!("no file panel on this platform yet");
        done(None);
    }
}

/// Asks where to write a bookmarks file.
pub(crate) fn choose_file_to_save(
    window: &BookmarksManager,
    suggested_name: &str,
    done: FileChosen,
) {
    #[cfg(target_os = "macos")]
    macos::choose_file_to_save(window, suggested_name, done);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, suggested_name);
        log::warn!("no file panel on this platform yet");
        done(None);
    }
}

/// Whether this platform has the panels (the manager says so instead of doing nothing).
pub(crate) const AVAILABLE: bool = cfg!(target_os = "macos");
