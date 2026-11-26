// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Appearance section's pure half (brief decisions D4 to D8): what each control shows for the
//! stored settings, and what a switch or a choice does to them. Every value a control can reach
//! must survive `Settings::validated()` untouched.

use mercury_appearance::panels::layout_panel::{
    Choice, Control, LayoutPanel, layout_panel, set_choice, set_switch, snapshot_cap_choices,
};
use mercury_settings::{
    AppearanceSettings, DEFAULT_SNAPSHOT_CAP_MB, DialCorner, DockStart, LAYOUT_CLASSIC,
    LAYOUT_MERCURY, Settings,
};

/// The note of D4, word for word.
use mercury_appearance::panels::INTERFACE_NOTE as NOTE;

const MERCURY_DOCK_NOTE: &str =
    "Bottom of the window. × shrinks it to a capsule; it never hides by itself.";
const CLASSIC_DOCK_NOTE: &str =
    "Optional in Classic: a dock under the tab strip. Off unless you turn it on.";

/// Settings that chose Classic and nothing else.
fn classic() -> AppearanceSettings {
    let mut settings = AppearanceSettings::default();
    assert!(set_choice(&mut settings, Choice::Layout, 1));
    settings
}

/// What `Settings::validated()` makes of `appearance`: it must report nothing and change nothing.
fn assert_survives_validation(appearance: &AppearanceSettings, what: &str) {
    let settings = Settings {
        appearance: appearance.clone(),
        ..Settings::default()
    };
    let (validated, repairs) = settings.validated();
    assert!(repairs.is_empty(), "{what}: repaired {repairs:?}");
    assert_eq!(&validated.appearance, appearance, "{what}");
}

fn caps(panel: &LayoutPanel) -> Vec<&str> {
    panel
        .snapshot_cap_choices
        .iter()
        .map(String::as_str)
        .collect()
}

// --- What the defaults show ---

#[test]
fn the_defaults_show_mercury_with_every_control_on_and_the_corner_bottom_right() {
    let panel = layout_panel(&AppearanceSettings::default(), true);

    assert_eq!(panel.layout_index, 0);
    assert!(panel.dock);
    assert_eq!(panel.dock_note, MERCURY_DOCK_NOTE);
    assert_eq!(panel.dock_starts_index, 0, "the dock starts open");
    assert!(panel.ring);
    assert!(panel.orbit);
    assert!(panel.orbit_pinch);
    assert!(panel.orbit_pinch_enabled);
    assert!(panel.keys);
    assert!(panel.dial, "the owner chose the corner dial on");
    assert_eq!(panel.dial_corner_index, 3, "bottom right");
    assert!(panel.dial_corner_enabled);
    assert_eq!(
        caps(&panel),
        ["32 MB", "64 MB", "128 MB", "256 MB", "512 MB", "1024 MB"]
    );
    assert_eq!(panel.snapshot_cap_index, 2, "128 MB is chosen");
    assert_eq!(
        snapshot_cap_choices(DEFAULT_SNAPSHOT_CAP_MB)[panel.snapshot_cap_index],
        DEFAULT_SNAPSHOT_CAP_MB
    );
}

#[test]
fn the_note_is_the_one_of_d4_and_empty_exactly_when_the_interface_is_live() {
    let appearance = AppearanceSettings::default();
    assert_eq!(layout_panel(&appearance, false).note, NOTE);
    assert_eq!(layout_panel(&appearance, true).note, "");

    // Whatever the settings are, nothing else decides it.
    let mut other = classic();
    set_switch(&mut other, Control::Orbit, false);
    assert_eq!(layout_panel(&other, false).note, NOTE);
    assert_eq!(layout_panel(&other, true).note, "");
}

#[test]
fn the_live_flag_changes_the_note_and_nothing_else() {
    let appearance = classic();
    let mut live = layout_panel(&appearance, true);
    let not_live = layout_panel(&appearance, false);
    assert_ne!(live, not_live);
    live.note.clone_from(&not_live.note);
    assert_eq!(live, not_live);
}

// --- The dock, per layout (D5) ---

#[test]
fn under_classic_the_dock_switch_shows_its_own_field_off_by_default_with_the_classic_note() {
    let panel = layout_panel(&classic(), true);

    assert_eq!(panel.layout_index, 1);
    assert!(!panel.dock, "dock_in_classic is off by default");
    assert_eq!(panel.dock_note, CLASSIC_DOCK_NOTE);
    // Everything but the dock reads as it does under Mercury.
    let mercury = layout_panel(&AppearanceSettings::default(), true);
    assert_eq!(
        LayoutPanel {
            layout_index: 0,
            dock: true,
            dock_note: MERCURY_DOCK_NOTE.to_owned(),
            ..panel
        },
        mercury
    );
}

#[test]
fn the_dock_switch_writes_only_the_field_of_the_layout_that_is_chosen() {
    // Mercury: `dock`.
    let mut settings = AppearanceSettings::default();
    set_switch(&mut settings, Control::Dock, false);
    assert!(!settings.tab_groups.dock);
    assert!(
        !settings.tab_groups.dock_in_classic,
        "Classic's field is untouched"
    );
    let mut expected = AppearanceSettings::default();
    expected.tab_groups.dock = false;
    assert_eq!(settings, expected, "nothing else changed");

    // Classic: `dock_in_classic`.
    let mut settings = classic();
    set_switch(&mut settings, Control::Dock, true);
    assert!(settings.tab_groups.dock_in_classic);
    assert!(settings.tab_groups.dock, "Mercury's field is untouched");
    let mut expected = classic();
    expected.tab_groups.dock_in_classic = true;
    assert_eq!(settings, expected, "nothing else changed");
}

#[test]
fn each_layout_remembers_its_own_dock_choice_when_the_layouts_are_switched() {
    let mut settings = AppearanceSettings::default();
    set_switch(&mut settings, Control::Dock, false); // Mercury: off
    assert!(set_choice(&mut settings, Choice::Layout, 1));
    assert!(
        !layout_panel(&settings, true).dock,
        "Classic shows its own (off)"
    );
    set_switch(&mut settings, Control::Dock, true); // Classic: on
    assert!(layout_panel(&settings, true).dock);

    assert!(set_choice(&mut settings, Choice::Layout, 0));
    let panel = layout_panel(&settings, true);
    assert!(
        !panel.dock,
        "back in Mercury, its choice (off) is unchanged"
    );
    assert_eq!(panel.dock_note, MERCURY_DOCK_NOTE);

    assert!(set_choice(&mut settings, Choice::Layout, 1));
    assert!(layout_panel(&settings, true).dock, "and Classic's (on) too");
}

#[test]
fn an_unknown_stored_layout_reads_and_writes_as_mercury() {
    // Settings that did not come through `validated()` (a later version's layout).
    let mut settings = AppearanceSettings {
        layout: "future".to_owned(),
        ..AppearanceSettings::default()
    };
    let panel = layout_panel(&settings, true);
    assert_eq!(panel.layout_index, 0);
    assert!(panel.dock);
    assert_eq!(panel.dock_note, MERCURY_DOCK_NOTE);

    set_switch(&mut settings, Control::Dock, false);
    assert!(!settings.tab_groups.dock);
    assert!(!settings.tab_groups.dock_in_classic);
    assert_eq!(
        settings.layout, "future",
        "a switch never rewrites the layout"
    );
}

// --- Names ---

#[test]
fn control_names_round_trip_and_are_the_strings_the_slint_side_sends() {
    let names: Vec<&str> = Control::ALL.iter().map(|control| control.name()).collect();
    assert_eq!(
        names,
        ["dock", "ring", "orbit", "orbit-pinch", "keys", "dial"]
    );
    for control in Control::ALL {
        assert_eq!(Control::from_name(control.name()), Some(control));
    }
}

#[test]
fn choice_names_round_trip_and_are_the_strings_the_slint_side_sends() {
    let names: Vec<&str> = Choice::ALL.iter().map(|choice| choice.name()).collect();
    assert_eq!(
        names,
        ["layout", "dock-starts", "dial-corner", "snapshot-cap"]
    );
    for choice in Choice::ALL {
        assert_eq!(Choice::from_name(choice.name()), Some(choice));
    }
}

#[test]
fn unknown_names_are_none_and_a_name_of_the_other_kind_is_not_taken() {
    for name in [
        "",
        " ",
        "Dock",
        "DOCK",
        " dock",
        "dock ",
        "orbit_pinch",
        "orbitpinch",
        "show-home-button",
        "layouts",
        "dock\0",
        "doc",
        "ring ",
    ] {
        assert_eq!(Control::from_name(name), None, "{name:?}");
        assert_eq!(Choice::from_name(name), None, "{name:?}");
    }
    // A switch's name is not a choice's and the other way round.
    for control in Control::ALL {
        assert_eq!(Choice::from_name(control.name()), None);
    }
    for choice in Choice::ALL {
        assert_eq!(Control::from_name(choice.name()), None);
    }
}

// --- Switches ---

#[test]
fn each_switch_writes_its_own_field_and_nothing_else() {
    type Read = fn(&AppearanceSettings) -> bool;
    let fields: [(Control, Read); 5] = [
        (Control::Ring, |s| s.tab_groups.ring),
        (Control::Orbit, |s| s.tab_groups.orbit),
        (Control::OrbitPinch, |s| s.tab_groups.orbit_pinch),
        (Control::Keys, |s| s.tab_groups.keys.enabled),
        (Control::Dial, |s| s.tab_groups.dial),
    ];
    for (control, read) in fields {
        let before = AppearanceSettings::default();
        assert!(read(&before), "{control:?} starts on");

        let mut off = before.clone();
        set_switch(&mut off, control, false);
        assert!(!read(&off), "{control:?} turned off");
        assert_ne!(off, before);

        // Putting it back gives exactly the starting settings: no other field moved.
        set_switch(&mut off, control, true);
        assert_eq!(off, before, "{control:?} on again");

        // Turning on what is on changes nothing.
        let mut same = before.clone();
        set_switch(&mut same, control, true);
        assert_eq!(same, before, "{control:?}");
    }
}

#[test]
fn a_switch_never_touches_the_page_zoom_theme_or_palettes() {
    let mut settings = AppearanceSettings {
        default_zoom_percent: 125,
        show_home_button: true,
        ..AppearanceSettings::default()
    };
    let before = settings.clone();
    for control in Control::ALL {
        set_switch(&mut settings, control, false);
        set_switch(&mut settings, control, true);
    }
    assert_eq!(settings.default_zoom_percent, before.default_zoom_percent);
    assert_eq!(settings.show_home_button, before.show_home_button);
    assert_eq!(settings.theme, before.theme);
    assert_eq!(settings.palette_light, before.palette_light);
    assert_eq!(settings.palette_dark, before.palette_dark);
    assert_eq!(
        settings.tab_groups.keys.hold_ms,
        before.tab_groups.keys.hold_ms
    );
}

// --- Dependent controls (D6) ---

#[test]
fn orbit_off_disables_the_pinch_switch_and_keeps_its_value() {
    let mut settings = AppearanceSettings::default();
    set_switch(&mut settings, Control::OrbitPinch, false);
    set_switch(&mut settings, Control::Orbit, false);

    let panel = layout_panel(&settings, true);
    assert!(!panel.orbit);
    assert!(!panel.orbit_pinch_enabled);
    assert!(!panel.orbit_pinch, "the stored value, off");

    // The other way round: pinch on, Orbit off. The stored value still shows.
    let mut settings = AppearanceSettings::default();
    set_switch(&mut settings, Control::Orbit, false);
    let panel = layout_panel(&settings, true);
    assert!(!panel.orbit_pinch_enabled);
    assert!(panel.orbit_pinch, "kept while disabled");
    assert!(settings.tab_groups.orbit_pinch);

    // Switching Orbit back on restores the pinch switch exactly as it was.
    set_switch(&mut settings, Control::Orbit, true);
    let panel = layout_panel(&settings, true);
    assert!(panel.orbit_pinch_enabled);
    assert!(panel.orbit_pinch);
    assert_eq!(settings, AppearanceSettings::default());
}

#[test]
fn the_dial_off_disables_the_corner_and_keeps_it() {
    let mut settings = AppearanceSettings::default();
    assert!(set_choice(&mut settings, Choice::DialCorner, 0)); // top left
    set_switch(&mut settings, Control::Dial, false);

    let panel = layout_panel(&settings, true);
    assert!(!panel.dial);
    assert!(!panel.dial_corner_enabled);
    assert_eq!(panel.dial_corner_index, 0, "the stored corner still shows");
    assert_eq!(settings.tab_groups.dial_corner, DialCorner::TopLeft);

    set_switch(&mut settings, Control::Dial, true);
    let panel = layout_panel(&settings, true);
    assert!(panel.dial_corner_enabled);
    assert_eq!(panel.dial_corner_index, 0, "restored as it was");
}

#[test]
fn the_pinch_switch_can_still_be_set_while_orbit_is_off_the_window_just_does_not_offer_it() {
    // The model refuses nothing here: the Slint side greys the control and refuses its own
    // action. A late report must not corrupt anything.
    let mut settings = AppearanceSettings::default();
    set_switch(&mut settings, Control::Orbit, false);
    set_switch(&mut settings, Control::OrbitPinch, false);
    assert!(!settings.tab_groups.orbit_pinch);
    assert_survives_validation(&settings, "pinch set while Orbit is off");
}

// --- Choices ---

#[test]
fn the_layout_choice_stores_the_layout_id() {
    let mut settings = AppearanceSettings::default();
    assert!(set_choice(&mut settings, Choice::Layout, 1));
    assert_eq!(settings.layout, LAYOUT_CLASSIC);
    assert_eq!(layout_panel(&settings, true).layout_index, 1);
    assert!(set_choice(&mut settings, Choice::Layout, 0));
    assert_eq!(settings.layout, LAYOUT_MERCURY);
    assert_eq!(settings, AppearanceSettings::default());
}

#[test]
fn the_dock_starts_choice_is_open_then_shrunk() {
    let mut settings = AppearanceSettings::default();
    assert!(set_choice(&mut settings, Choice::DockStarts, 1));
    assert_eq!(settings.tab_groups.dock_starts, DockStart::Shrunk);
    assert_eq!(layout_panel(&settings, true).dock_starts_index, 1);
    assert!(set_choice(&mut settings, Choice::DockStarts, 0));
    assert_eq!(settings.tab_groups.dock_starts, DockStart::Open);
    assert_eq!(layout_panel(&settings, true).dock_starts_index, 0);
}

#[test]
fn the_dial_corner_choice_is_top_left_top_right_bottom_left_bottom_right() {
    let corners = [
        DialCorner::TopLeft,
        DialCorner::TopRight,
        DialCorner::BottomLeft,
        DialCorner::BottomRight,
    ];
    for (index, corner) in corners.into_iter().enumerate() {
        let mut settings = AppearanceSettings::default();
        assert!(set_choice(&mut settings, Choice::DialCorner, index));
        assert_eq!(settings.tab_groups.dial_corner, corner, "index {index}");
        assert_eq!(layout_panel(&settings, true).dial_corner_index, index);
    }
}

#[test]
fn the_memory_choice_stores_the_megabytes_of_the_chosen_item() {
    for (index, expected) in [32_u16, 64, 128, 256, 512, 1024].into_iter().enumerate() {
        let mut settings = AppearanceSettings::default();
        assert!(set_choice(&mut settings, Choice::SnapshotCap, index));
        assert_eq!(settings.tab_groups.snapshot_cap_mb, expected);
        let panel = layout_panel(&settings, true);
        assert_eq!(panel.snapshot_cap_index, index);
        assert_eq!(panel.snapshot_cap_choices[index], format!("{expected} MB"));
    }
}

#[test]
fn an_index_outside_the_list_returns_false_and_changes_nothing() {
    let starts = [AppearanceSettings::default(), classic(), {
        let mut odd = AppearanceSettings::default();
        odd.tab_groups.snapshot_cap_mb = 200;
        odd
    }];
    for start in starts {
        for (choice, len) in [
            (Choice::Layout, 2),
            (Choice::DockStarts, 2),
            (Choice::DialCorner, 4),
            (
                Choice::SnapshotCap,
                snapshot_cap_choices(start.tab_groups.snapshot_cap_mb).len(),
            ),
        ] {
            for index in [len, len + 1, 100, usize::MAX] {
                let mut settings = start.clone();
                assert!(
                    !set_choice(&mut settings, choice, index),
                    "{choice:?} {index}"
                );
                assert_eq!(settings, start, "{choice:?} {index} changed something");
            }
        }
    }
}

// --- Memory for Orbit previews, with an odd stored value (D7) ---

#[test]
fn the_memory_choices_are_the_six_sizes_when_the_stored_one_is_among_them() {
    for size in [32_u16, 64, 128, 256, 512, 1024] {
        assert_eq!(
            snapshot_cap_choices(size),
            [32, 64, 128, 256, 512, 1024],
            "{size}"
        );
    }
}

#[test]
fn a_hand_edited_memory_value_is_offered_in_its_place_in_the_order() {
    assert_eq!(
        snapshot_cap_choices(200),
        [32, 64, 128, 200, 256, 512, 1024]
    );
    assert_eq!(snapshot_cap_choices(33), [32, 33, 64, 128, 256, 512, 1024]);
    assert_eq!(
        snapshot_cap_choices(1000),
        [32, 64, 128, 256, 512, 1000, 1024]
    );
    // Values the validation would repair are still listed, so the panel never panics on them.
    assert_eq!(snapshot_cap_choices(0), [0, 32, 64, 128, 256, 512, 1024]);
    assert_eq!(
        snapshot_cap_choices(u16::MAX),
        [32, 64, 128, 256, 512, 1024, u16::MAX]
    );
    for current in [0_u16, 1, 31, 33, 100, 200, 300, 1000, 1025, u16::MAX] {
        let choices = snapshot_cap_choices(current);
        assert!(
            choices.windows(2).all(|pair| pair[0] < pair[1]),
            "{current}"
        );
        assert!(choices.contains(&current), "{current}");
        assert_eq!(choices.len(), 7, "{current}");
    }
}

#[test]
fn the_panel_selects_the_odd_stored_memory_value_and_choosing_another_drops_it() {
    let mut settings = AppearanceSettings::default();
    settings.tab_groups.snapshot_cap_mb = 200;
    assert_survives_validation(&settings, "200 MB is a valid stored value");

    let panel = layout_panel(&settings, true);
    assert_eq!(
        caps(&panel),
        [
            "32 MB", "64 MB", "128 MB", "200 MB", "256 MB", "512 MB", "1024 MB"
        ]
    );
    assert_eq!(panel.snapshot_cap_index, 3);

    // Choosing the odd item again keeps it.
    assert!(set_choice(&mut settings, Choice::SnapshotCap, 3));
    assert_eq!(settings.tab_groups.snapshot_cap_mb, 200);

    // The index counts in the list with the odd value in it.
    assert!(set_choice(&mut settings, Choice::SnapshotCap, 4));
    assert_eq!(settings.tab_groups.snapshot_cap_mb, 256);
    let panel = layout_panel(&settings, true);
    assert_eq!(panel.snapshot_cap_choices.len(), 6, "the odd value is gone");
    assert_eq!(panel.snapshot_cap_index, 3);
}

#[test]
fn a_stored_memory_value_outside_the_valid_range_still_shows_without_panicking() {
    for value in [0_u16, 5, 2000, u16::MAX] {
        let mut settings = AppearanceSettings::default();
        settings.tab_groups.snapshot_cap_mb = value;
        let panel = layout_panel(&settings, true);
        assert_eq!(
            panel.snapshot_cap_choices[panel.snapshot_cap_index],
            format!("{value} MB")
        );
        // Choosing a real size repairs it, and the result is valid.
        assert!(set_choice(&mut settings, Choice::SnapshotCap, 2));
        assert_survives_validation(&settings, "a hand-edited memory value, replaced");
    }
}

// --- Every reachable value is valid ---

/// Settings to start from: the defaults, Classic, all off, an odd memory value, a shrunk dock.
fn starting_points() -> Vec<AppearanceSettings> {
    let mut all_off = AppearanceSettings::default();
    for control in Control::ALL {
        set_switch(&mut all_off, control, false);
    }
    let mut odd = classic();
    odd.tab_groups.snapshot_cap_mb = 200;
    odd.default_zoom_percent = 125;
    let mut shrunk = AppearanceSettings::default();
    assert!(set_choice(&mut shrunk, Choice::DockStarts, 1));
    assert!(set_choice(&mut shrunk, Choice::DialCorner, 0));
    vec![
        AppearanceSettings::default(),
        classic(),
        all_off,
        odd,
        shrunk,
    ]
}

#[test]
fn every_switch_and_choice_from_every_starting_point_passes_validation_with_no_repair() {
    for start in starting_points() {
        assert_survives_validation(&start, "the starting point");
        for control in Control::ALL {
            for on in [false, true] {
                let mut settings = start.clone();
                set_switch(&mut settings, control, on);
                assert_survives_validation(&settings, &format!("{control:?} {on}"));
            }
        }
        for choice in Choice::ALL {
            for index in 0..8 {
                let mut settings = start.clone();
                let taken = set_choice(&mut settings, choice, index);
                assert_survives_validation(&settings, &format!("{choice:?} {index} ({taken})"));
            }
        }
    }
}

/// A small deterministic generator, so the walk below is the same on every run.
struct Walk(u64);

impl Walk {
    fn next(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % u64::try_from(bound).expect("a small bound")).expect("fits")
    }
}

#[test]
fn a_long_random_walk_over_the_controls_never_leaves_a_value_validation_would_repair() {
    let mut walk = Walk(0x9E37_79B9_7F4A_7C15);
    let mut settings = AppearanceSettings::default();
    for step in 0..3000 {
        let before = settings.clone();
        if walk.next(2) == 0 {
            let control = Control::ALL[walk.next(Control::ALL.len())];
            set_switch(&mut settings, control, walk.next(2) == 0);
        } else {
            let choice = Choice::ALL[walk.next(Choice::ALL.len())];
            let taken = set_choice(&mut settings, choice, walk.next(8));
            if !taken {
                assert_eq!(
                    settings, before,
                    "step {step}: a refused choice changed things"
                );
            }
        }
        assert_survives_validation(&settings, &format!("step {step}"));

        // The panel always agrees with the settings, in both modes.
        let panel = layout_panel(&settings, walk.next(2) == 0);
        assert_eq!(panel.orbit, settings.tab_groups.orbit, "step {step}");
        assert_eq!(
            panel.orbit_pinch, settings.tab_groups.orbit_pinch,
            "step {step}"
        );
        assert_eq!(
            panel.orbit_pinch_enabled, settings.tab_groups.orbit,
            "step {step}"
        );
        assert_eq!(
            panel.dial_corner_enabled, settings.tab_groups.dial,
            "step {step}"
        );
        let dock = if panel.layout_index == 0 {
            settings.tab_groups.dock
        } else {
            settings.tab_groups.dock_in_classic
        };
        assert_eq!(panel.dock, dock, "step {step}");
        assert!(panel.snapshot_cap_index < panel.snapshot_cap_choices.len());
    }
}
