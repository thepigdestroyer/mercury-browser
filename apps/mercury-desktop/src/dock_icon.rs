// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Dock icon follows the theme while the app runs (macOS).
//!
//! The artwork of the bundle's `AppIcon.icns` (`resources/macos/AppIcon.svg`) is recoloured for the
//! palette and scheme in use (`mercury_appearance::AppIconColours`: the Sun in the palette's
//! accent on a dark plate of the palette, see that module for why), rendered at 1024 px
//! (`mercury_iconset`) and handed to `NSApplication.applicationIconImage`. The one entry is
//! [`follow`], called by `SettingsHandle::push_dock_icon` when a look is applied and when the palette,
//! the theme or the system's colour scheme changes.
//!
//! **What this does not change:** the bundle's own icon stays the orange one. A Dock tile of an app
//! that is not running (a pinned one, the Dock before launch), Finder, the application switcher
//! before launch and the first instants after launch show it; the running app's tile changes when
//! the first look is applied. And the image set here does not get the system's icon mask and
//! shadow: it carries its own plate (`AppIcon.svg` draws the macOS grid's 824 px rounded square
//! with the transparent margin), so its edge may differ slightly from a system-masked icon.

use std::cell::RefCell;

use mercury_appearance::{AppIconColours, Palette, Scheme};
use mercury_iconset::{APP_ICON_PIXELS, render_png, tint_app_icon};
use mercury_ui::Look;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::{MainThreadMarker, NSData};

/// The artwork the bundle's icon is made from.
const ARTWORK: &str = include_str!("../resources/macos/AppIcon.svg");

thread_local! {
    /// The icon last set: the palette, the scheme, and the picture (kept so that a repeat is free).
    static CURRENT: RefCell<Option<(&'static str, Scheme, Retained<NSImage>)>> =
        const { RefCell::new(None) };
}

/// The recoloured icon as a 1024 px PNG.
pub(crate) fn icon_png(
    palette: &Palette,
    scheme: Scheme,
) -> Result<Vec<u8>, mercury_iconset::Error> {
    let tinted = tint_app_icon(ARTWORK, &AppIconColours::new(palette, scheme))?;
    render_png(tinted.as_bytes(), APP_ICON_PIXELS)
}

/// Makes the Dock icon the one of the palette `look` shows in the dark or the light scheme.
///
/// Does nothing off the main thread or when the icon in place is already that one, unless `force`
/// (used once the event loop runs, in case AppKit put the bundle's icon back while launching).
/// A failure is logged and leaves the icon as it was.
pub(crate) fn follow(look: &Look, dark: bool, force: bool) {
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let scheme = if dark { Scheme::Dark } else { Scheme::Light };
    let palette = if dark {
        look.palette_dark
    } else {
        look.palette_light
    };
    let image = CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        if let Some((id, shown, image)) = current.as_ref()
            && *id == palette.id
            && *shown == scheme
        {
            return (force).then(|| image.clone());
        }
        let image = match make_image(palette, scheme) {
            Ok(image) => image,
            Err(error) => {
                log::warn!("dock icon: {error}");
                return None;
            }
        };
        *current = Some((palette.id, scheme, image.clone()));
        Some(image)
    });
    if let Some(image) = image {
        let application = NSApplication::sharedApplication(main_thread);
        // SAFETY: `image` is a valid, initialised `NSImage` and we are on the main thread (the
        // marker above); `applicationIconImage` accepts nil or an image.
        unsafe { application.setApplicationIconImage(Some(&image)) };
    }
}

fn make_image(palette: &Palette, scheme: Scheme) -> Result<Retained<NSImage>, String> {
    let png = icon_png(palette, scheme).map_err(|error| error.to_string())?;
    let data = NSData::with_bytes(&png);
    NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or_else(|| "AppKit could not read the rendered icon".to_owned())
}
