// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The group dock driven with Slint's own pointer and key events (no display): the pill under the
//! pointer grows and the others follow, leaving resets, clicks on slivers, chips, pills, the "+"
//! and the x do what SPEC §4.4 says, the x shrinks the dock, the capsule restores it, and with
//! reduce-motion off the pills really animate. The host is wired the way the real window will be
//! (`group_dock_common::Host`).

mod common;
mod group_dock_common;

use std::time::Duration;

use common::a11y;
use group_dock_common::Host;
use i_slint_backend_testing::{AccessibleRole, mock_elapsed_time};
use mercury_ui::group_dock::DockOptions;
use mercury_ui::ring_bands::fixtures::{mockup_groups, mockup_tabs};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Model};

fn host() -> Host {
    a11y::init();
    let host = Host::new(
        mockup_groups(false),
        mockup_tabs(false),
        DockOptions::default(),
    );
    host.open();
    host
}

fn scales(host: &Host) -> Vec<f32> {
    host.frame.borrow().scales.clone()
}

#[test]
fn the_pill_under_the_pointer_grows_and_leaving_resets_it() {
    let host = host();
    assert!(
        scales(&host)
            .iter()
            .all(|scale| (*scale - 1.0).abs() < 1e-4)
    );
    host.settle_on_pill(2);
    let scales = scales(&host);
    assert_eq!(scales.len(), 6);
    assert!((scales[2] - 1.45).abs() < 1e-3, "{scales:?}");
    assert!((scales[1] - 1.12).abs() < 1e-3, "{scales:?}");
    assert!((scales[3] - 1.12).abs() < 1e-3, "{scales:?}");
    for index in [0, 4, 5] {
        assert!((scales[index] - 1.0).abs() < 1e-3, "{scales:?}");
    }
    // The row the window draws says the same.
    let row = host.window.get_pills().row_data(2).expect("pill 2");
    assert!((row.scale - 1.45).abs() < 1e-3);
    assert!(
        host.window.get_dock_height() > 52.0 + 13.0,
        "the bar grew upward: {}",
        host.window.get_dock_height()
    );

    host.leave();
    assert!(
        scales_of(&host)
            .iter()
            .all(|scale| (*scale - 1.0).abs() < 1e-4),
        "{:?}",
        scales_of(&host)
    );
    assert!((host.window.get_dock_height() - 52.0).abs() < 1e-3);
}

fn scales_of(host: &Host) -> Vec<f32> {
    scales(host)
}

#[test]
fn moving_along_the_dock_magnifies_one_pill_at_a_time() {
    let host = host();
    let mut last: Option<usize> = None;
    let start = host.frame.borrow().dock_x;
    let end = start + host.frame.borrow().dock_width;
    let mut x = start + 20.0;
    while x < end - 80.0 {
        host.move_to(x, Host::mid_y());
        let scales = scales(&host);
        let big: Vec<usize> = scales
            .iter()
            .enumerate()
            .filter(|(_, scale)| **scale > 1.4)
            .map(|(index, _)| index)
            .collect();
        assert!(big.len() <= 1, "at most one pill at 1.45: {scales:?}");
        if let (Some(previous), Some(&now)) = (last, big.first()) {
            assert!(
                now >= previous,
                "moving right never goes back: {previous} -> {now}"
            );
            assert!(
                now <= previous + 1,
                "one pill at a time: {previous} -> {now}"
            );
        }
        if let Some(&now) = big.first() {
            last = Some(now);
        }
        x += 7.0;
    }
    assert!(last.is_some());
}

#[test]
fn clicks_on_slivers_chips_pills_and_the_plus_do_what_the_spec_says() {
    let host = host();

    // A sliver of a folded pill activates its tab; a middle click closes it. The bar is anchored on
    // the pointer, so a sliver comes towards a pointer that moves to it: chase it to where it
    // settles (each move changes the frame, the sliver moves by less than the pointer did).
    host.settle_on_pill(1);
    let sliver_at = |host: &Host| {
        let frame = host.frame.borrow();
        let pill = &frame.pills[1];
        let sliver = pill.slivers.row_data(0).expect("a sliver");
        (pill.x + sliver.x + sliver.width / 2.0, sliver.tab)
    };
    let (mut sliver_x, tab_id) = sliver_at(&host);
    for _ in 0..12 {
        host.move_to(sliver_x, Host::mid_y());
        sliver_x = sliver_at(&host).0;
    }
    host.take_log();
    host.click(sliver_x, Host::mid_y());
    assert_eq!(host.take_log(), [format!("tab:{tab_id}")]);
    host.click_with(sliver_x, Host::mid_y(), PointerEventButton::Middle);
    assert_eq!(host.take_log(), [format!("close:{tab_id}")]);

    // A chip of the current group activates its tab.
    host.leave();
    host.settle_on_pill(0);
    let chip_at = |host: &Host| {
        let frame = host.frame.borrow();
        let pill = &frame.pills[0];
        assert!(
            pill.expanded,
            "the first pill of the fixture is the current group"
        );
        let chip = pill.chips.row_data(1).expect("a chip");
        (pill.x + chip.x + chip.width / 2.0, chip.tab)
    };
    let (mut chip_x, chip_tab) = chip_at(&host);
    for _ in 0..12 {
        host.move_to(chip_x, Host::mid_y());
        chip_x = chip_at(&host).0;
    }
    host.take_log();
    host.click(chip_x, Host::mid_y());
    assert_eq!(host.take_log(), [format!("tab:{chip_tab}")]);

    // The "+" at the right end of the current pill is drawn and hit: a new tab in that group.
    host.leave();
    let pill_plus_x = {
        let frame = host.frame.borrow();
        let pill = &frame.pills[0];
        pill.x + pill.width - (4.0 + 13.0) * pill.scale
    };
    host.take_log();
    host.click(pill_plus_x, Host::mid_y());
    assert_eq!(
        host.take_log(),
        [format!(
            "new:{}",
            host.data.borrow().current_group().expect("a current group")
        )]
    );

    // The x at a chip's right end closes its tab.
    let close_x = {
        let frame = host.frame.borrow();
        let pill = &frame.pills[0];
        let chip = pill.chips.row_data(1).expect("a chip");
        pill.x + chip.x + chip.width - (8.0 + 8.0) * pill.scale
    };
    host.take_log();
    host.click(close_x, Host::mid_y());
    assert_eq!(host.take_log(), [format!("close:{chip_tab}")]);

    // The trailing "+" targets the current group.
    host.leave();
    let (plus_x, current) = {
        let frame = host.frame.borrow();
        (
            frame.new_tab_x + 13.0,
            host.data.borrow().current_group().expect("a current group"),
        )
    };
    host.take_log();
    host.click(plus_x, Host::mid_y());
    assert_eq!(host.take_log(), [format!("new:{current}")]);
}

#[test]
fn the_x_shrinks_the_dock_and_the_capsule_restores_it() {
    let host = host();
    let shrink_x = host.frame.borrow().shrink_x + 13.0;
    host.take_log();
    host.click(shrink_x, Host::mid_y());
    assert_eq!(host.take_log(), ["shrunk:true", "shrink"]);
    assert!(host.window.get_shrunk());
    assert!((host.window.get_target_band() - 40.0).abs() < 1e-3);

    // The dock takes no click while the capsule is shown: a click where the x was does nothing.
    host.click(shrink_x, Host::mid_y());
    assert!(host.take_log().is_empty(), "the hidden dock must not react");

    // A click on the capsule (centred, 28 px high, 12 px above the window's bottom) restores it.
    host.click(1440.0 / 2.0, 900.0 - 12.0 - 14.0);
    assert_eq!(host.take_log(), ["shrunk:false", "expand"]);
    assert!(!host.window.get_shrunk());
    assert!((host.window.get_target_band() - 72.0).abs() < 1e-3);
}

#[test]
fn tab_then_space_restores_a_shrunk_dock_from_the_keyboard() {
    let host = host();
    host.window.set_shrunk(true);
    host.tick();
    host.take_log();
    let window = host.window.window();
    window.dispatch_event(WindowEvent::KeyPressed { text: "\t".into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: "\t".into() });
    window.dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
    host.tick();
    assert_eq!(host.take_log(), ["shrunk:false", "expand"]);
    assert!(!host.window.get_shrunk());
}

#[test]
fn arrows_move_between_pills_and_return_activates_the_focused_one() {
    let host = host();
    // Focus the dock with a click on a pill's padding, then walk with the keys.
    host.settle_on_pill(3);
    let centre = host.pill_centre(3);
    host.click(centre, Host::mid_y());
    host.take_log();
    let window = host.window.window();
    let press = |text: slint::SharedString| {
        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text });
    };
    press(Key::RightArrow.into());
    press(Key::Return.into());
    host.tick();
    let log = host.take_log();
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(log[0].starts_with("pill:"), "{log:?}");
}

/// The x of the Work pill (the fifth) as the accessibility tree reports it: where it is drawn now.
fn work_pill_x(host: &Host) -> f32 {
    a11y::reading_order(&host.window)
        .iter()
        .find(|node| node.role == AccessibleRole::Tab && node.label.starts_with("Work,"))
        .expect("the Work pill")
        .rect()
        .x
}

#[test]
fn the_pills_slide_to_their_new_place_over_120_ms() {
    let host = host();
    host.window.set_reduce_motion(false);
    host.tick();
    let before = work_pill_x(&host);
    host.settle_on_pill(2);
    let target = host.window.get_pills().row_data(4).expect("pill 4").x;
    assert!((target - before).abs() > 5.0, "{before} -> {target}");

    // No time has passed: the pill is still where it was.
    assert!((work_pill_x(&host) - before).abs() < 1.0);
    // Part of the way: between the old place and the new one.
    mock_elapsed_time(Duration::from_millis(60));
    host.tick();
    let middle = work_pill_x(&host);
    let (low, high) = (before.min(target), before.max(target));
    assert!(
        middle > low + 0.5 && middle < high - 0.5,
        "after 60 ms the pill is at {middle}, between {before} and {target}"
    );
    // After the animation: at the new place.
    mock_elapsed_time(Duration::from_millis(300));
    host.tick();
    assert!(
        (work_pill_x(&host) - target).abs() < 0.5,
        "{} against {target}",
        work_pill_x(&host)
    );
}

#[test]
fn with_reduce_motion_the_pills_are_at_their_new_place_at_once() {
    let host = host();
    assert!(host.window.get_reduce_motion());
    let before = work_pill_x(&host);
    host.settle_on_pill(2);
    let target = host.window.get_pills().row_data(4).expect("pill 4").x;
    assert!((target - before).abs() > 5.0, "{before} -> {target}");
    assert!(
        (work_pill_x(&host) - target).abs() < 0.01,
        "{} against {target} with no time passed",
        work_pill_x(&host)
    );
}

#[test]
fn the_keys_move_between_pills_activate_peek_and_close() {
    let host = host();
    host.settle_on_pill(3);
    let centre = host.pill_centre(3);
    host.click(centre, Host::mid_y());
    let window = host.window.window();
    let press = |key: slint::SharedString| {
        window.dispatch_event(WindowEvent::KeyPressed { text: key.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text: key });
    };
    let keys = |presses: &[Key]| {
        for key in presses {
            press((*key).into());
        }
        host.tick();
    };
    host.leave();
    host.take_log();

    // ← from nowhere goes to the first pill (the current group, expanded): ⏎ activates it, ↓ does
    // nothing on an expanded pill.
    keys(&[Key::LeftArrow, Key::DownArrow, Key::Return]);
    assert_eq!(host.take_log(), ["pill:1"]);
    // → → to the third pill (a folded one): Space activates, ↓ asks for the peek, ← goes back one.
    keys(&[Key::RightArrow, Key::RightArrow, Key::Space]);
    assert_eq!(host.take_log(), ["pill:3"]);
    keys(&[Key::DownArrow]);
    assert_eq!(host.take_log(), ["peek:3"]);
    keys(&[Key::LeftArrow, Key::Return]);
    assert_eq!(host.take_log(), ["pill:2"]);

    // ⌘⌫ (Slint's `control` modifier on macOS) closes the active chip of the focused expanded
    // pill; a plain ⌫ does nothing.
    keys(&[Key::LeftArrow]);
    keys(&[Key::Backspace]);
    assert!(
        host.take_log().is_empty(),
        "a plain backspace closes nothing"
    );
    window.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    keys(&[Key::Backspace]);
    window.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    host.tick();
    let active = mockup_tabs(false)
        .iter()
        .find(|tab| tab.active)
        .map(|tab| tab.id)
        .expect("an active tab");
    assert_eq!(host.take_log(), [format!("close:{active}")]);

    // The "+" and the x are reachable too: → past the last pill.
    for _ in 0..6 {
        keys(&[Key::RightArrow]);
    }
    keys(&[Key::Return]);
    let current = host.data.borrow().current_group().expect("a current group");
    assert_eq!(host.take_log(), [format!("new:{current}")]);
    keys(&[Key::RightArrow, Key::Space]);
    assert_eq!(host.take_log(), ["shrunk:true", "shrink"]);
}

#[test]
fn the_dock_hits_where_it_draws_in_a_box_of_any_height() {
    // The box of `GroupDock` is `box-height` (77.5 px) in the preview by default, but a host
    // gives it the 72 px band (or less while the band settles): the bar stays anchored 12 px above
    // the box's bottom and `hit` is asked in the same box either way.
    for height in [77.5, 72.0, 64.0] {
        let host = host();
        host.window.set_bar_height(height);
        host.tick();
        host.settle_on_pill(1);
        let sliver_at = |host: &Host| {
            let frame = host.frame.borrow();
            let pill = &frame.pills[1];
            let sliver = pill.slivers.row_data(0).expect("a sliver");
            (pill.x + sliver.x + sliver.width / 2.0, sliver.tab)
        };
        let (mut x, tab) = sliver_at(&host);
        for _ in 0..12 {
            host.move_to(x, Host::mid_y());
            x = sliver_at(&host).0;
        }
        host.take_log();
        host.click(x, Host::mid_y());
        assert_eq!(host.take_log(), [format!("tab:{tab}")], "box {height}");
        // The box's top 2 px (inside the bar) hit it, and 2 px under the bar's bottom edge do not.
        let top = 900.0 - height + 2.0;
        let below = 900.0 - 12.0 + 2.0;
        host.take_log();
        host.click(x, top);
        assert_eq!(
            host.take_log(),
            [format!("tab:{tab}")],
            "box {height}, near the top"
        );
        host.click(x, below);
        assert!(host.take_log().is_empty(), "box {height}, under the bar");
        // And the x shrinks the dock.
        host.leave();
        let shrink_x = host.frame.borrow().shrink_x + 13.0;
        host.click(shrink_x, Host::mid_y());
        assert_eq!(host.take_log(), ["shrunk:true", "shrink"], "box {height}");
    }
}

#[test]
fn a_handler_less_hit_says_there_is_nothing() {
    a11y::init();
    let window = mercury_ui::GroupDockPreview::new().expect("the preview");
    let hit = window.invoke_hit(10.0, 10.0);
    assert_eq!(hit.kind, mercury_ui::DockHitKind::None);
    assert_eq!((hit.group, hit.tab), (-1, -1));
}
