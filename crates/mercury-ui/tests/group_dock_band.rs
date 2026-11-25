// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The dock's reserved band and its settle rule (SPEC §4.4, brief D5 and D6): the band heights, the
//! page height, the dock height while magnified, the initial state and `DockState`.

use mercury_ui::group_dock::{
    BOTTOM_MARGIN, CAPSULE_HEIGHT, DockState, dock_height, initial_shrunk, page_height,
    reserved_band_height,
};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn the_band_is_72_open_and_40_shrunk() {
    assert_eq!(reserved_band_height(false), 72.0);
    assert_eq!(reserved_band_height(true), 40.0);
}

#[test]
fn the_dock_and_the_capsule_fit_in_their_bands_with_the_bottom_margin() {
    // What the band is for: the resting dock and the capsule, each with the margin under them.
    assert!(BOTTOM_MARGIN + dock_height(1.0) <= reserved_band_height(false));
    assert!(BOTTOM_MARGIN + CAPSULE_HEIGHT <= reserved_band_height(true));
    // ...and nothing else: no more than 8 px of slack above either of them.
    assert!(reserved_band_height(false) - (BOTTOM_MARGIN + dock_height(1.0)) <= 8.0);
    assert!(reserved_band_height(true) - (BOTTOM_MARGIN + CAPSULE_HEIGHT) <= 0.5);
}

#[test]
fn shrinking_gives_the_page_32_px_back_whatever_the_window() {
    for content in [200.0, 600.0, 900.0, 1440.0, 2160.0] {
        let gained = page_height(content, true) - page_height(content, false);
        assert!(
            close(
                gained,
                reserved_band_height(false) - reserved_band_height(true)
            ),
            "{content}: {gained}"
        );
        assert!(close(gained, 32.0), "{content}: {gained}");
    }
}

#[test]
fn the_page_height_is_the_content_minus_the_band() {
    assert_eq!(page_height(900.0, false), 828.0);
    assert_eq!(page_height(900.0, true), 860.0);
    assert_eq!(page_height(72.0, false), 0.0);
    assert_eq!(page_height(50.0, false), 0.0);
    assert_eq!(page_height(50.0, true), 10.0);
    assert_eq!(page_height(0.0, true), 0.0);
}

#[test]
fn the_page_height_survives_bad_input() {
    for bad in [-1.0, -900.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(page_height(bad, false), 0.0, "{bad}");
        assert_eq!(page_height(bad, true), 0.0, "{bad}");
    }
}

#[test]
fn the_dock_grows_with_the_scale() {
    assert_eq!(dock_height(1.0), 52.0);
    assert!(close(dock_height(1.45), 65.5));
    assert!(close(dock_height(1.12), 2.0 * 11.0 + 30.0 * 1.12));
    assert!(dock_height(1.45) > dock_height(1.12));
}

#[test]
fn a_garbage_scale_gives_the_resting_height() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -2.0, 0.0, 0.5] {
        assert_eq!(dock_height(bad), 52.0, "{bad}");
    }
}

#[test]
fn the_magnified_bar_overshoots_the_band_by_5_and_a_half_pixels() {
    // The bar is bottom-anchored 12 px above the band's bottom and grows upward to 65.5 px: the
    // top of the magnified bar is 5.5 px above the band's top edge, onto the page, and only while
    // a pill is magnified.
    let top_of_bar = BOTTOM_MARGIN + dock_height(1.45);
    assert!(close(top_of_bar - reserved_band_height(false), 5.5));
    assert!(top_of_bar - reserved_band_height(false) < 8.0);
    assert!(close(
        BOTTOM_MARGIN + dock_height(1.0) - reserved_band_height(false),
        -8.0
    ));
}

#[test]
fn the_session_value_wins_and_none_follows_the_setting() {
    for starts in [false, true] {
        assert!(initial_shrunk(Some(true), starts));
        assert!(!initial_shrunk(Some(false), starts));
    }
    assert!(initial_shrunk(None, true));
    assert!(!initial_shrunk(None, false));
}

#[test]
fn a_new_dock_is_settled() {
    let open = DockState::new(false);
    assert!(!open.shrunk());
    assert_eq!(open.reserved_band(), 72.0);
    assert!(!open.animating());
    let shrunk = DockState::new(true);
    assert!(shrunk.shrunk());
    assert_eq!(shrunk.reserved_band(), 40.0);
    assert!(!shrunk.animating());
}

#[test]
fn the_web_view_band_changes_once_at_the_end_of_the_animation() {
    let mut dock = DockState::new(false);
    assert!(dock.toggle(1000));
    assert!(dock.shrunk(), "the dock follows the request at once");
    assert!(dock.animating());
    assert_eq!(dock.reserved_band(), 72.0);
    assert_eq!(dock.tick(1000), None);
    assert_eq!(dock.reserved_band(), 72.0);
    assert_eq!(dock.tick(1179), None);
    assert_eq!(dock.reserved_band(), 72.0);
    assert_eq!(dock.tick(1180), Some(40.0));
    assert_eq!(dock.reserved_band(), 40.0);
    assert!(!dock.animating());
    assert_eq!(dock.tick(1180), None);
    assert_eq!(dock.tick(5000), None);
    assert_eq!(dock.reserved_band(), 40.0);
}

#[test]
fn expanding_settles_on_the_open_band() {
    let mut dock = DockState::new(true);
    assert!(!dock.toggle(0));
    assert!(!dock.shrunk());
    assert_eq!(dock.reserved_band(), 40.0);
    assert_eq!(dock.tick(179), None);
    assert_eq!(dock.tick(180), Some(72.0));
    assert_eq!(dock.reserved_band(), 72.0);
}

#[test]
fn a_late_tick_still_reports_once() {
    let mut dock = DockState::new(false);
    dock.toggle(10);
    assert_eq!(dock.tick(60_000), Some(40.0));
    assert_eq!(dock.tick(60_001), None);
}

#[test]
fn toggling_back_during_the_animation_does_not_resize_the_web_view() {
    let mut dock = DockState::new(false);
    assert!(dock.toggle(1000));
    assert!(!dock.toggle(1100));
    assert!(!dock.shrunk());
    assert_eq!(dock.reserved_band(), 72.0);
    assert!(dock.animating());
    assert_eq!(dock.tick(1280), None, "the band is the one it was");
    assert_eq!(dock.reserved_band(), 72.0);
    assert!(!dock.animating());
    assert_eq!(dock.tick(1281), None);
}

#[test]
fn the_reversal_restarts_the_animation() {
    let mut dock = DockState::new(false);
    dock.toggle(1000);
    dock.toggle(1100);
    assert_eq!(dock.tick(1180), None);
    assert!(dock.animating(), "the second toggle runs its own 180 ms");
    assert_eq!(dock.tick(1280), None);
    assert!(!dock.animating());
}

#[test]
fn three_quick_toggles_resize_once_to_the_last_request() {
    let mut dock = DockState::new(false);
    dock.toggle(0);
    dock.toggle(50);
    dock.toggle(100);
    assert!(dock.shrunk());
    assert_eq!(dock.tick(279), None);
    assert_eq!(dock.reserved_band(), 72.0);
    assert_eq!(dock.tick(280), Some(40.0));
    assert_eq!(dock.tick(300), None);
}

#[test]
fn a_toggle_after_settling_starts_a_new_animation() {
    let mut dock = DockState::new(false);
    dock.toggle(0);
    assert_eq!(dock.tick(180), Some(40.0));
    dock.toggle(500);
    assert_eq!(dock.reserved_band(), 40.0);
    assert_eq!(dock.tick(680), Some(72.0));
}

#[test]
fn setting_the_same_value_does_nothing() {
    let mut dock = DockState::new(false);
    dock.set(false, 100);
    assert!(!dock.animating());
    assert_eq!(dock.tick(10_000), None);

    dock.set(true, 200);
    assert!(dock.shrunk());
    assert!(dock.animating());
    dock.set(true, 250);
    assert_eq!(dock.tick(379), None, "the second set did not restart it");
    assert_eq!(dock.tick(380), Some(40.0));
}

#[test]
fn setting_back_during_the_animation_is_like_a_second_toggle() {
    let mut dock = DockState::new(false);
    dock.set(true, 0);
    dock.set(false, 90);
    assert!(!dock.shrunk());
    assert_eq!(dock.tick(1000), None);
    assert_eq!(dock.reserved_band(), 72.0);
}

#[test]
fn time_going_backwards_does_not_panic() {
    let mut dock = DockState::new(false);
    dock.toggle(5000);
    assert_eq!(dock.tick(10), None);
    assert!(dock.animating());
    dock.toggle(0);
    assert_eq!(dock.tick(0), None);
    assert_eq!(dock.tick(179), None);
    assert_eq!(dock.tick(180), None, "toggled twice: the band is unchanged");
    assert_eq!(dock.reserved_band(), 72.0);
}

#[test]
fn a_clock_at_the_end_of_time_does_not_overflow() {
    let mut dock = DockState::new(false);
    dock.toggle(u64::MAX);
    assert!(dock.animating());
    assert_eq!(dock.tick(u64::MAX), Some(40.0));
    dock.toggle(u64::MAX - 1);
    assert_eq!(dock.tick(u64::MAX), Some(72.0));
}
