// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The sizes the dock's Slint and Rust halves must agree on (brief D7): the `DockGeometry` global
//! of `dock-models.slint` against the constants of `group_dock`, and what Slint draws against what
//! Rust's frame says (a pixel probe of two slivers).

mod common;
mod group_dock_common;

use common::a11y;
use group_dock_common::{Host, init_software, near, pixel_at, snapshot};
use mercury_ui::group_dock::{
    BAND_OPEN, BAND_SHRUNK, BOTTOM_MARGIN, CAPSULE_HEIGHT, CAPSULE_WIDTH, CHEVRON_X, COUNT_WIDTH,
    COUNT_X, DOCK_HEIGHT, DockData, DockOptions, PAD_Y, PILL_HEIGHT, SLIVER_LEFT, SLIVER_RIGHT,
    SLIVER_WIDTH,
};
use mercury_ui::ring_bands::fixtures::{mockup_groups, mockup_tabs};
use mercury_ui::{DockGeometry, GroupDockPreview};
use slint::{ComponentHandle, Model};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn the_slint_sizes_equal_the_rust_constants() {
    a11y::init();
    let window = GroupDockPreview::new().expect("the preview window");
    let geometry = window.global::<DockGeometry>();
    assert!(close(geometry.get_pill_height(), PILL_HEIGHT));
    assert!(close(geometry.get_pad_y(), PAD_Y));
    assert!(close(geometry.get_bottom_margin(), BOTTOM_MARGIN));
    assert!(close(geometry.get_band_open(), BAND_OPEN));
    assert!(close(geometry.get_band_shrunk(), BAND_SHRUNK));
    assert!(close(geometry.get_capsule_width(), CAPSULE_WIDTH));
    assert!(close(geometry.get_capsule_height(), CAPSULE_HEIGHT));
    assert!(close(geometry.get_sliver(), SLIVER_WIDTH));
    assert!(close(geometry.get_capsule_sliver_left(), SLIVER_LEFT));
    assert!(close(geometry.get_capsule_sliver_right(), SLIVER_RIGHT));
    assert!(close(geometry.get_capsule_count_x(), COUNT_X));
    assert!(close(geometry.get_capsule_count_width(), COUNT_WIDTH));
    assert!(close(geometry.get_capsule_chevron_x(), CHEVRON_X));
    // 2 * pad-y + pill-height = the dock at rest; the box is the band plus the overshoot.
    assert!(close(
        2.0 * geometry.get_pad_y() + geometry.get_pill_height(),
        DOCK_HEIGHT
    ));
    assert!(close(
        geometry.get_box_height(),
        DockData::container_height()
    ));
    assert!(close(geometry.get_box_height(), 77.5));
    assert!(close(geometry.get_radius(), 24.0));
    assert!(
        geometry.get_button() >= 24.0,
        "a touch target of at least 24 px"
    );
}

#[test]
fn what_slint_draws_for_a_sliver_is_where_rust_says_it_is() {
    init_software();
    let host = Host::new(
        mockup_groups(false),
        mockup_tabs(false),
        DockOptions::default(),
    );
    host.open();
    let picture = snapshot(&host.window);
    let frame = host.frame.borrow();
    let y = Host::mid_y();
    let mut probed = 0;
    for pill in frame.pills.iter().filter(|pill| !pill.expanded).take(2) {
        let sliver = pill.slivers.row_data(1).expect("a folded pill has slivers");
        let x = pill.x + sliver.x + sliver.width / 2.0;
        let shade = sliver.shade;
        let got = pixel_at(&picture, x, y);
        assert!(
            near(got, (shade.red(), shade.green(), shade.blue()), 2),
            "sliver of tab {} at ({x}, {y}): drawn {got:?}, row says {:?}",
            sliver.tab,
            (shade.red(), shade.green(), shade.blue())
        );
        probed += 1;
    }
    assert_eq!(probed, 2);
}
