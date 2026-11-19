// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The open and save panels for importing and exporting logins, attached to the settings window
//! as sheets. Each calls back once, on the main thread, with the chosen path or `None`.

use std::cell::Cell;
use std::path::PathBuf;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSModalResponse, NSModalResponseOK, NSOpenPanel, NSSavePanel, NSView, NSWindow,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSString};
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(crate) type Chosen = Box<dyn FnOnce(Option<PathBuf>)>;

fn ns_window(window: &slint::Window) -> Option<Retained<NSWindow>> {
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

/// Asks for a CSV file to import.
pub(crate) fn choose_import_file(window: &slint::Window, done: Chosen) {
    let (Some(main_thread), Some(ns_window)) = (MainThreadMarker::new(), ns_window(window)) else {
        done(None);
        return;
    };
    let panel = NSOpenPanel::openPanel(main_thread);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setMessage(Some(&NSString::from_str(
        "Choose a CSV file of logins exported from Chrome, Edge, Brave, Firefox or Safari",
    )));
    // See `controls::file_panel` for why the older file-type list is used.
    #[expect(deprecated)]
    panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[NSString::from_str(
        "csv",
    )])));
    let chosen_panel = panel.clone();
    let done = Cell::new(Some(done));
    let handler = RcBlock::new(move |response: NSModalResponse| {
        let path = (response == NSModalResponseOK)
            .then(|| {
                chosen_panel
                    .URLs()
                    .iter()
                    .find_map(|url| url.to_file_path())
            })
            .flatten();
        if let Some(done) = done.take() {
            done(path);
        }
    });
    panel.beginSheetModalForWindow_completionHandler(&ns_window, &handler);
}

/// Asks where to write the exported logins.
pub(crate) fn choose_export_file(window: &slint::Window, done: Chosen) {
    let (Some(main_thread), Some(ns_window)) = (MainThreadMarker::new(), ns_window(window)) else {
        done(None);
        return;
    };
    let panel = NSSavePanel::savePanel(main_thread);
    panel.setNameFieldStringValue(&NSString::from_str("Mercury Browser logins.csv"));
    panel.setMessage(Some(&NSString::from_str(
        "The file will hold every saved password in clear text.",
    )));
    let chosen_panel = panel.clone();
    let done = Cell::new(Some(done));
    let handler = RcBlock::new(move |response: NSModalResponse| {
        let path = (response == NSModalResponseOK)
            .then(|| chosen_panel.URL().and_then(|url| url.to_file_path()))
            .flatten();
        if let Some(done) = done.take() {
            done(path);
        }
    });
    panel.beginSheetModalForWindow_completionHandler(&ns_window, &handler);
}
