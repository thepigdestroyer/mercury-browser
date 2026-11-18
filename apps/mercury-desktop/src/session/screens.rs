// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The screens a restored window can be placed on, read before the window exists.

use super::model::ScreenArea;

/// Each screen's area outside the menu bar and Dock, in logical pixels with the origin at the
/// top left of the main screen (winit's and Slint's window coordinates), the main screen first.
#[cfg(target_os = "macos")]
pub(super) fn screen_areas() -> Vec<ScreenArea> {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;

    // AppKit screens may only be used on the main thread; this is checked, not assumed.
    let Some(main_thread) = MainThreadMarker::new() else {
        return Vec::new();
    };
    let screens = NSScreen::screens(main_thread);
    let Some(main_height) = screens.iter().next().map(|main| main.frame().size.height) else {
        return Vec::new();
    };
    screens
        .iter()
        .map(|screen| {
            let visible = screen.visibleFrame();
            to_top_left(
                main_height,
                visible.origin.x,
                visible.origin.y,
                visible.size.width,
                visible.size.height,
            )
        })
        .collect()
}

/// Screens are not known before the window exists here: the window keeps its saved size and
/// the system places it.
#[cfg(not(target_os = "macos"))]
pub(super) fn screen_areas() -> Vec<ScreenArea> {
    Vec::new()
}

/// Converts an AppKit rectangle (points, origin at the bottom left of the main screen, y
/// growing upwards) to top-left coordinates.
#[cfg(any(target_os = "macos", test))]
fn to_top_left(main_height: f64, x: f64, y: f64, width: f64, height: f64) -> ScreenArea {
    ScreenArea {
        x,
        y: main_height - (y + height),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_main_screens_visible_area_starts_below_the_menu_bar() {
        // A 1512 x 982 point screen with a 33 point menu bar and no Dock at the bottom.
        assert_eq!(
            to_top_left(982.0, 0.0, 0.0, 1512.0, 949.0),
            ScreenArea {
                x: 0.0,
                y: 33.0,
                width: 1512.0,
                height: 949.0,
            }
        );
    }

    #[test]
    fn a_screen_above_the_main_one_has_a_negative_top() {
        assert_eq!(to_top_left(982.0, 0.0, 982.0, 1920.0, 1080.0).y, -1080.0);
    }
}
