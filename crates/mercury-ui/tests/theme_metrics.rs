// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The sizes layer of the chrome: `ChromeMetrics` in Slint and `mercury_appearance::Metrics`
//! list the same 27 lengths, the Slint defaults are the roomy preset, and `apply_look` sets every
//! property from the field of the same name (no setter wired to another property).

use std::rc::Rc;

use mercury_appearance::Metrics;
use mercury_ui::{ChromeMetrics, Look, MainWindow, apply_look};
use slint::ComponentHandle;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

fn init_platform() {
    let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
}

/// What `ChromeMetrics` holds now. A struct literal with every field: a property added to
/// `Metrics` without a line here does not compile.
fn read_back(sizes: &ChromeMetrics<'_>) -> Metrics {
    Metrics {
        tab_row_height: sizes.get_tab_row_height(),
        tab_row_padding_left: sizes.get_tab_row_padding_left(),
        tab_row_padding_right: sizes.get_tab_row_padding_right(),
        tab_row_padding_top: sizes.get_tab_row_padding_top(),
        tab_row_padding_bottom: sizes.get_tab_row_padding_bottom(),
        tab_row_spacing: sizes.get_tab_row_spacing(),
        new_tab_button_size: sizes.get_new_tab_button_size(),
        toolbar_height: sizes.get_toolbar_height(),
        toolbar_padding_x: sizes.get_toolbar_padding_x(),
        toolbar_padding_y: sizes.get_toolbar_padding_y(),
        toolbar_spacing: sizes.get_toolbar_spacing(),
        button_width: sizes.get_button_width(),
        button_height: sizes.get_button_height(),
        button_radius: sizes.get_button_radius(),
        button_font_size: sizes.get_button_font_size(),
        address_height: sizes.get_address_height(),
        address_radius: sizes.get_address_radius(),
        address_font_size: sizes.get_address_font_size(),
        bookmarks_bar_height: sizes.get_bookmarks_bar_height(),
        find_bar_height: sizes.get_find_bar_height(),
        tab_pinned_width: sizes.get_tab_pinned_width(),
        tab_min_width: sizes.get_tab_min_width(),
        tab_max_width: sizes.get_tab_max_width(),
        tab_spacing: sizes.get_tab_spacing(),
        tab_list_button_width: sizes.get_tab_list_button_width(),
        tab_radius: sizes.get_tab_radius(),
        tab_title_font_size: sizes.get_tab_title_font_size(),
    }
}

/// A `Metrics` whose field number `n` (in declaration order) is `100 + n`.
fn distinct() -> Metrics {
    let mut next = 100.0;
    let mut take = || {
        next += 1.0;
        next - 1.0
    };
    Metrics {
        tab_row_height: take(),
        tab_row_padding_left: take(),
        tab_row_padding_right: take(),
        tab_row_padding_top: take(),
        tab_row_padding_bottom: take(),
        tab_row_spacing: take(),
        new_tab_button_size: take(),
        toolbar_height: take(),
        toolbar_padding_x: take(),
        toolbar_padding_y: take(),
        toolbar_spacing: take(),
        button_width: take(),
        button_height: take(),
        button_radius: take(),
        button_font_size: take(),
        address_height: take(),
        address_radius: take(),
        address_font_size: take(),
        bookmarks_bar_height: take(),
        find_bar_height: take(),
        tab_pinned_width: take(),
        tab_min_width: take(),
        tab_max_width: take(),
        tab_spacing: take(),
        tab_list_button_width: take(),
        tab_radius: take(),
        tab_title_font_size: take(),
    }
}

fn look(metrics: Metrics) -> Look {
    Look {
        metrics,
        ..Look::hermes_roomy()
    }
}

#[test]
fn the_slint_defaults_are_the_roomy_preset() {
    init_platform();
    let window = MainWindow::new().expect("a window");
    assert_eq!(read_back(&window.global::<ChromeMetrics>()), Metrics::ROOMY);
}

#[test]
fn every_property_is_set_from_the_field_of_its_name() {
    init_platform();
    let window = MainWindow::new().expect("a window");
    let pushed = distinct();
    apply_look(&window, &look(pushed));
    assert_eq!(read_back(&window.global::<ChromeMetrics>()), pushed);
}

#[test]
fn pushing_roomy_again_restores_the_defaults() {
    init_platform();
    let window = MainWindow::new().expect("a window");
    apply_look(&window, &look(distinct()));
    apply_look(&window, &look(Metrics::ROOMY));
    assert_eq!(read_back(&window.global::<ChromeMetrics>()), Metrics::ROOMY);
}

#[test]
fn another_window_keeps_its_own_sizes() {
    init_platform();
    let first = MainWindow::new().expect("a window");
    let second = MainWindow::new().expect("a window");
    apply_look(&first, &look(distinct()));
    assert_eq!(read_back(&second.global::<ChromeMetrics>()), Metrics::ROOMY);
}
