// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Pushes the sizes preset into `ChromeMetrics`.

use mercury_appearance::Metrics;
use slint::{ComponentHandle, Global};

use crate::ChromeMetrics;

/// Sets every property of `ChromeMetrics` from `metrics`.
pub(super) fn push<C>(component: &C, metrics: &Metrics)
where
    C: ComponentHandle,
    for<'a> ChromeMetrics<'a>: Global<'a, C>,
{
    // No `..` in the pattern: a field added to `Metrics` without a setter here does not compile.
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
    let sizes = component.global::<ChromeMetrics>();
    sizes.set_tab_row_height(tab_row_height);
    sizes.set_tab_row_padding_left(tab_row_padding_left);
    sizes.set_tab_row_padding_right(tab_row_padding_right);
    sizes.set_tab_row_padding_top(tab_row_padding_top);
    sizes.set_tab_row_padding_bottom(tab_row_padding_bottom);
    sizes.set_tab_row_spacing(tab_row_spacing);
    sizes.set_new_tab_button_size(new_tab_button_size);
    sizes.set_toolbar_height(toolbar_height);
    sizes.set_toolbar_padding_x(toolbar_padding_x);
    sizes.set_toolbar_padding_y(toolbar_padding_y);
    sizes.set_toolbar_spacing(toolbar_spacing);
    sizes.set_button_width(button_width);
    sizes.set_button_height(button_height);
    sizes.set_button_radius(button_radius);
    sizes.set_button_font_size(button_font_size);
    sizes.set_address_height(address_height);
    sizes.set_address_radius(address_radius);
    sizes.set_address_font_size(address_font_size);
    sizes.set_bookmarks_bar_height(bookmarks_bar_height);
    sizes.set_find_bar_height(find_bar_height);
    sizes.set_tab_pinned_width(tab_pinned_width);
    sizes.set_tab_min_width(tab_min_width);
    sizes.set_tab_max_width(tab_max_width);
    sizes.set_tab_spacing(tab_spacing);
    sizes.set_tab_list_button_width(tab_list_button_width);
    sizes.set_tab_radius(tab_radius);
    sizes.set_tab_title_font_size(tab_title_font_size);
}
