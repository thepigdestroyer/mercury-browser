// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Screen-geometry rounding (R14c MH49, spec §5.7).
//!
//! Pages read the screen size, the available area and the window size. [`round_geometry`] rounds them to coarse
//! steps so that a page sees the same values on many machines.

use crate::level::Setting;

/// A width and a height in device-independent pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
}

/// A position on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    /// The horizontal position.
    pub x: i32,
    /// The vertical position.
    pub y: i32,
}

/// The screen, the available area below the menu bar, and the window, as a page would read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// The screen size.
    pub screen: Size,
    /// Where the available area starts.
    pub available_origin: Point,
    /// The size of the available area.
    pub available: Size,
    /// Where the window starts.
    pub window_origin: Point,
    /// The window size.
    pub window: Size,
}

/// Screen sizes that real displays commonly have. [`Setting::Strict`] reports one of these.
pub const COMMON_SCREEN_SIZES: [Size; 14] = [
    Size {
        width: 1024,
        height: 768,
    },
    Size {
        width: 1280,
        height: 720,
    },
    Size {
        width: 1280,
        height: 800,
    },
    Size {
        width: 1366,
        height: 768,
    },
    Size {
        width: 1440,
        height: 900,
    },
    Size {
        width: 1536,
        height: 864,
    },
    Size {
        width: 1600,
        height: 900,
    },
    Size {
        width: 1680,
        height: 1050,
    },
    Size {
        width: 1920,
        height: 1080,
    },
    Size {
        width: 1920,
        height: 1200,
    },
    Size {
        width: 2560,
        height: 1440,
    },
    Size {
        width: 2560,
        height: 1600,
    },
    Size {
        width: 3440,
        height: 1440,
    },
    Size {
        width: 3840,
        height: 2160,
    },
];

const ORIGIN: Point = Point { x: 0, y: 0 };
const SCREEN_STEP: Size = Size {
    width: 100,
    height: 100,
};
const WINDOW_STEP: Size = Size {
    width: 200,
    height: 100,
};

/// Rounds the geometry a page reads, according to `setting`.
///
/// `Off` returns the input. `Standard` rounds the screen down to multiples of 100 × 100 (at least 100 × 100), makes
/// the available area the screen, rounds the window down to multiples of 200 × 100 (at least 200 × 100) and keeps it
/// within the screen. `Strict` reports the largest entry of [`COMMON_SCREEN_SIZES`] that fits the input screen, or the
/// `Standard` result when none fits. Both origins are (0, 0) for every setting but `Off`.
///
/// The "never larger" property holds for inputs of at least 100 × 100; smaller screens are raised to the minimum.
pub fn round_geometry(g: Geometry, setting: Setting) -> Geometry {
    match setting {
        Setting::Off => g,
        Setting::Standard => standard(g),
        Setting::Strict => match largest_common_fit(g.screen) {
            Some(screen) => placed(screen, window_within(g.window, screen)),
            None => standard(g),
        },
    }
}

fn standard(g: Geometry) -> Geometry {
    let screen = screen_step(g.screen);
    placed(screen, window_within(g.window, screen))
}

fn placed(screen: Size, window: Size) -> Geometry {
    Geometry {
        screen,
        available_origin: ORIGIN,
        available: screen,
        window_origin: ORIGIN,
        window,
    }
}

fn screen_step(screen: Size) -> Size {
    Size {
        width: round_down(screen.width, SCREEN_STEP.width).max(SCREEN_STEP.width),
        height: round_down(screen.height, SCREEN_STEP.height).max(SCREEN_STEP.height),
    }
}

/// The window clamped to `screen` first, then rounded down to whole steps, raised to one step and capped at `screen`
/// again. Clamping first keeps the result stable when it is rounded a second time.
fn window_within(window: Size, screen: Size) -> Size {
    let fit =
        |value: u32, limit: u32, step: u32| round_down(value.min(limit), step).max(step).min(limit);
    Size {
        width: fit(window.width, screen.width, WINDOW_STEP.width),
        height: fit(window.height, screen.height, WINDOW_STEP.height),
    }
}

fn largest_common_fit(screen: Size) -> Option<Size> {
    COMMON_SCREEN_SIZES
        .into_iter()
        .filter(|size| size.width <= screen.width && size.height <= screen.height)
        .max_by_key(|size| u64::from(size.width) * u64::from(size.height))
}

fn round_down(value: u32, step: u32) -> u32 {
    value / step * step
}
