// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The chrome palette's accent colours: readable (WCAG 2.2 AA, 4.5:1 for text and 3:1 for
//! interface parts such as focus rings, underlines and button shapes) on every surface they are
//! drawn on, in Light and in Dark, and drawn from the Mercury brand (Sun orange) rather than the
//! blue inherited from Hermes. Run with `--nocapture` to print every ratio.

use std::rc::Rc;

use mercury_ui::{AboutWindow, ChromePalette, ChromeTheme};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{Color, ComponentHandle};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

const TEXT: f64 = 4.5;
const UI: f64 = 3.0;

fn channel(value: u8) -> f64 {
    let c = f64::from(value) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(color: Color) -> f64 {
    0.2126 * channel(color.red()) + 0.7152 * channel(color.green()) + 0.0722 * channel(color.blue())
}

fn contrast(a: Color, b: Color) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Hue in degrees (0..360) of an opaque colour.
fn hue(color: Color) -> f64 {
    let [r, g, b] = [color.red(), color.green(), color.blue()].map(|v| f64::from(v) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let delta = max - min;
    if delta == 0.0 {
        return 0.0;
    }
    let h = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    h * 60.0
}

fn hex(color: Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        color.red(),
        color.green(),
        color.blue()
    )
}

#[test]
fn accents_are_readable_and_on_brand_in_both_appearances() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).expect("no platform is set yet");
    let window = AboutWindow::new().expect("a window to read the palette from");
    let palette = window.global::<ChromePalette>();
    let mut failures = Vec::new();

    for (theme, name) in [(ChromeTheme::Light, "light"), (ChromeTheme::Dark, "dark")] {
        palette.set_theme(theme);
        let p = &palette;
        let surfaces = [
            ("window-background", p.get_window_background()),
            ("toolbar-background", p.get_toolbar_background()),
            ("content-background", p.get_content_background()),
            ("tab-active", p.get_tab_active()),
            ("field-background", p.get_field_background()),
            ("popup-background", p.get_popup_background()),
            ("sheet-background", p.get_sheet_background()),
        ];
        // (foreground role, colour, background role, colour, minimum)
        let mut pairs: Vec<(&str, Color, &str, Color, f64)> = Vec::new();
        for (surface, background) in surfaces {
            // Links (About), checked boxes, the active-tab underline, the loading bar and
            // focused field borders.
            pairs.push(("accent", p.get_accent(), surface, background, TEXT));
            pairs.push(("focus-ring", p.get_focus_ring(), surface, background, UI));
            pairs.push((
                "button-label-hover",
                p.get_button_label_hover(),
                surface,
                background,
                TEXT,
            ));
            pairs.push((
                "primary-button-background",
                p.get_primary_button_background(),
                surface,
                background,
                UI,
            ));
        }
        pairs.push((
            "match-highlight",
            p.get_match_highlight(),
            "popup-background",
            p.get_popup_background(),
            TEXT,
        ));
        pairs.push((
            "match-highlight",
            p.get_match_highlight(),
            "field-background",
            p.get_field_background(),
            TEXT,
        ));
        pairs.push((
            "item-selected",
            p.get_item_selected(),
            "popup-background",
            p.get_popup_background(),
            UI,
        ));
        pairs.push((
            "item-selected-text",
            p.get_item_selected_text(),
            "item-selected",
            p.get_item_selected(),
            TEXT,
        ));
        pairs.push((
            "item-selected",
            p.get_item_selected(),
            "page-popup-background",
            p.get_page_popup_background(),
            UI,
        ));
        for (role, background) in [
            (
                "primary-button-background",
                p.get_primary_button_background(),
            ),
            ("primary-button-hover", p.get_primary_button_hover()),
            ("primary-button-pressed", p.get_primary_button_pressed()),
            ("accent", p.get_accent()),
        ] {
            pairs.push((
                "primary-button-text",
                p.get_primary_button_text(),
                role,
                background,
                TEXT,
            ));
        }
        pairs.push((
            "info-text",
            p.get_info_text(),
            "info-background",
            p.get_info_background(),
            TEXT,
        ));
        pairs.push((
            "text",
            p.get_text(),
            "info-background",
            p.get_info_background(),
            TEXT,
        ));

        for (fg_role, fg, bg_role, bg, minimum) in pairs {
            let ratio = contrast(fg, bg);
            println!(
                "{name:5} {fg_role:26} {} on {bg_role:26} {}: {ratio:5.2} (min {minimum})",
                hex(fg),
                hex(bg)
            );
            if ratio < minimum {
                failures.push(format!(
                    "{name}: {fg_role} {} on {bg_role} {} is {ratio:.2}, needs {minimum}",
                    hex(fg),
                    hex(bg)
                ));
            }
        }

        // Hermes' blues sit at hues 195–215°; the brand accent is the Sun's orange (15–45°).
        for (role, color) in [
            ("accent", p.get_accent()),
            ("button-label-hover", p.get_button_label_hover()),
            (
                "primary-button-background",
                p.get_primary_button_background(),
            ),
            ("primary-button-hover", p.get_primary_button_hover()),
            ("primary-button-pressed", p.get_primary_button_pressed()),
            ("focus-ring", p.get_focus_ring()),
            ("match-highlight", p.get_match_highlight()),
            ("item-selected", p.get_item_selected()),
        ] {
            let h = hue(color);
            if !(15.0..=45.0).contains(&h) {
                failures.push(format!(
                    "{name}: {role} {} has hue {h:.0}°, not the brand orange",
                    hex(color)
                ));
            }
        }
        for (role, color) in [
            ("info-background", p.get_info_background()),
            ("info-border", p.get_info_border()),
        ] {
            let h = hue(color);
            if (180.0..=240.0).contains(&h) {
                failures.push(format!(
                    "{name}: {role} {} is still blue (hue {h:.0}°)",
                    hex(color)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
