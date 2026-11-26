// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Page setup through the crate's public API only: what the desktop and the capture driver see.

use mercury_print::paper::{MarginPreset, Orientation, PaperSize};
use mercury_print::setup::{
    MAX_PAGES, MAX_SCREENSHOT_DEVICE_PX, MAX_SCREENSHOT_PIXELS, MAX_TILE_DEVICE_PX,
    PRINT_DEVICE_SCALE, PageSetup,
};

fn default_a4() -> PageSetup {
    PageSetup {
        paper: PaperSize::A4,
        orientation: Orientation::Portrait,
        margins: MarginPreset::Default,
        scale_percent: 100,
    }
}

#[test]
fn print_constants_match_the_spec() {
    assert_eq!(PRINT_DEVICE_SCALE, 2.0);
    assert_eq!(MAX_TILE_DEVICE_PX, 8192);
    assert_eq!(MAX_PAGES, 200);
    assert_eq!(MAX_SCREENSHOT_DEVICE_PX, 32_768);
    assert_eq!(MAX_SCREENSHOT_PIXELS, 400_000_000);
}

#[test]
fn a_print_at_the_default_setup_lays_out_718_css_px() {
    assert_eq!(default_a4().content_width_css_px(), 718);
}

#[test]
fn an_out_of_range_scale_is_clamped_before_use() {
    let page = PageSetup {
        scale_percent: 400,
        ..default_a4()
    }
    .sanitized();
    assert_eq!(page.scale_percent, 200);
    assert_eq!(page.content_width_css_px(), 359);
}

#[test]
fn a_zero_size_paper_request_is_still_a_usable_layout() {
    let page = PageSetup {
        paper: PaperSize::A5,
        orientation: Orientation::Landscape,
        margins: MarginPreset::Default,
        scale_percent: 200,
    };
    assert!(page.content_width_css_px() >= 200);
}
