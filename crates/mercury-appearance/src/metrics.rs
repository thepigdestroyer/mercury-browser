// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The sizes of the browser's own chrome, as a preset.
//!
//! [`Metrics`] is what the interface receives: lengths in logical pixels, one field for each
//! property of `ChromeMetrics` in `crates/mercury-ui/ui/theme/chrome-metrics.slint` (the same names,
//! with underscores). Today there is one preset, [`Metrics::ROOMY`], the sizes the chrome has
//! always had, so that adding the look as data changes no pixel. Compact and Standard presets, a
//! density setting and a chrome text scale come later, next to this table, together with the two
//! stored settings they need (`appearance.density`, `appearance.chrome_text_size`).
//!
//! Hit areas stay at 28 px or more in every preset (R12 §5.2).

/// The sizes of the chrome's rows, buttons, address field and tabs, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// The height of the row that holds the tabs.
    pub tab_row_height: f32,
    /// Space left of the tab strip in the tab row.
    pub tab_row_padding_left: f32,
    /// Space right of the "new tab" button in the tab row.
    pub tab_row_padding_right: f32,
    /// Space above the tabs.
    pub tab_row_padding_top: f32,
    /// Space below the tabs.
    pub tab_row_padding_bottom: f32,
    /// Space between the tab strip and the "new tab" button.
    pub tab_row_spacing: f32,
    /// Width and height of the "new tab" button.
    pub new_tab_button_size: f32,
    /// The height of the toolbar row.
    pub toolbar_height: f32,
    /// Space left and right of the toolbar's items.
    pub toolbar_padding_x: f32,
    /// Space above and below the toolbar's items.
    pub toolbar_padding_y: f32,
    /// Space between the toolbar's items.
    pub toolbar_spacing: f32,
    /// Width of a toolbar button.
    pub button_width: f32,
    /// Height of a toolbar button.
    pub button_height: f32,
    /// Corner radius of a toolbar button.
    pub button_radius: f32,
    /// Size of the glyph on a toolbar button.
    pub button_font_size: f32,
    /// Height of the address field.
    pub address_height: f32,
    /// Corner radius of the address field.
    pub address_radius: f32,
    /// Size of the text typed in the address field.
    pub address_font_size: f32,
    /// Height of the bookmarks bar when it is shown.
    pub bookmarks_bar_height: f32,
    /// Height of the find bar when it is shown.
    pub find_bar_height: f32,
    /// Width of a pinned tab.
    pub tab_pinned_width: f32,
    /// Narrowest an ordinary tab gets before the strip scrolls.
    pub tab_min_width: f32,
    /// Widest an ordinary tab gets.
    pub tab_max_width: f32,
    /// Space between tabs.
    pub tab_spacing: f32,
    /// Width of the "all tabs" button at the end of an overflowing strip.
    pub tab_list_button_width: f32,
    /// Corner radius of a tab.
    pub tab_radius: f32,
    /// Size of a tab's title.
    pub tab_title_font_size: f32,
}

impl Metrics {
    /// The sizes the chrome has always had: the preset every window starts with.
    pub const ROOMY: Self = Self {
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

    /// The layout sizes: a tab row of 46 px and a toolbar of 52 px, pushed by the desktop app.
    pub const LAYOUT: Self = Self {
        tab_row_height: 46.0,
        toolbar_height: 52.0,
        toolbar_padding_y: 7.0,
        ..Self::ROOMY
    };
}

impl Default for Metrics {
    fn default() -> Self {
        Self::ROOMY
    }
}
