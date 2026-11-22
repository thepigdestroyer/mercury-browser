// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The sizes preset: sane values, hit areas of at least 28 px (R12 §5.2), and the roomy preset
//! spelled out, so that a change of a number is a change of this file too.

use mercury_appearance::Metrics;

/// The fields as `(name, value)`, for the checks that apply to all of them. A field added to
/// `Metrics` without a line here does not compile.
fn all(metrics: &Metrics) -> Vec<(&'static str, f32)> {
    let Metrics {
        tab_row_height,
        tab_row_padding_left,
        tab_row_padding_right,
        tab_row_padding_top,
        tab_row_padding_bottom,
        tab_row_spacing,
        new_tab_button_size,
        toolbar_height,
        toolbar_padding_x,
        toolbar_padding_y,
        toolbar_spacing,
        button_width,
        button_height,
        button_radius,
        button_font_size,
        address_height,
        address_radius,
        address_font_size,
        bookmarks_bar_height,
        find_bar_height,
        tab_pinned_width,
        tab_min_width,
        tab_max_width,
        tab_spacing,
        tab_list_button_width,
        tab_radius,
        tab_title_font_size,
    } = *metrics;
    vec![
        ("tab_row_height", tab_row_height),
        ("tab_row_padding_left", tab_row_padding_left),
        ("tab_row_padding_right", tab_row_padding_right),
        ("tab_row_padding_top", tab_row_padding_top),
        ("tab_row_padding_bottom", tab_row_padding_bottom),
        ("tab_row_spacing", tab_row_spacing),
        ("new_tab_button_size", new_tab_button_size),
        ("toolbar_height", toolbar_height),
        ("toolbar_padding_x", toolbar_padding_x),
        ("toolbar_padding_y", toolbar_padding_y),
        ("toolbar_spacing", toolbar_spacing),
        ("button_width", button_width),
        ("button_height", button_height),
        ("button_radius", button_radius),
        ("button_font_size", button_font_size),
        ("address_height", address_height),
        ("address_radius", address_radius),
        ("address_font_size", address_font_size),
        ("bookmarks_bar_height", bookmarks_bar_height),
        ("find_bar_height", find_bar_height),
        ("tab_pinned_width", tab_pinned_width),
        ("tab_min_width", tab_min_width),
        ("tab_max_width", tab_max_width),
        ("tab_spacing", tab_spacing),
        ("tab_list_button_width", tab_list_button_width),
        ("tab_radius", tab_radius),
        ("tab_title_font_size", tab_title_font_size),
    ]
}

#[test]
fn every_size_is_positive_and_finite() {
    let sizes = all(&Metrics::ROOMY);
    assert_eq!(sizes.len(), 27);
    for (name, value) in sizes {
        assert!(value.is_finite() && value > 0.0, "{name} is {value}");
    }
}

#[test]
fn hit_areas_are_at_least_28_px() {
    let metrics = Metrics::ROOMY;
    for (name, value) in [
        ("button_width", metrics.button_width),
        ("button_height", metrics.button_height),
        ("new_tab_button_size", metrics.new_tab_button_size),
        ("tab_list_button_width", metrics.tab_list_button_width),
    ] {
        assert!(value >= 28.0, "{name} is {value}, a hit area needs 28 px");
    }
}

#[test]
fn the_tab_widths_are_in_order() {
    let metrics = Metrics::ROOMY;
    assert!(metrics.tab_min_width <= metrics.tab_max_width);
    assert!(metrics.tab_pinned_width < metrics.tab_min_width);
}

#[test]
fn roomy_is_the_default_and_the_table_of_the_brief() {
    let roomy = Metrics {
        tab_row_height: 48.0,
        tab_row_padding_left: 10.0,
        tab_row_padding_right: 14.0,
        tab_row_padding_top: 7.0,
        tab_row_padding_bottom: 3.0,
        tab_row_spacing: 8.0,
        new_tab_button_size: 34.0,
        toolbar_height: 60.0,
        toolbar_padding_x: 14.0,
        toolbar_padding_y: 11.0,
        toolbar_spacing: 8.0,
        button_width: 36.0,
        button_height: 34.0,
        button_radius: 7.0,
        button_font_size: 17.0,
        address_height: 38.0,
        address_radius: 9.0,
        address_font_size: 14.0,
        bookmarks_bar_height: 34.0,
        find_bar_height: 42.0,
        tab_pinned_width: 40.0,
        tab_min_width: 112.0,
        tab_max_width: 184.0,
        tab_spacing: 6.0,
        tab_list_button_width: 34.0,
        tab_radius: 8.0,
        tab_title_font_size: 13.0,
    };
    assert_eq!(Metrics::ROOMY, roomy);
    assert_eq!(Metrics::default(), roomy);
}
