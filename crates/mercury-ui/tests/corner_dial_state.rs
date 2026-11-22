// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The pure state helpers of the corner dial: corner names (pinned against the settings crate's serde
//! output), the mapping to the Slint enum, the session override, the accessible label and the
//! keyboard stepping.

use mercury_settings::DialCorner;
use mercury_ui::DialCornerKind;
use mercury_ui::corner_dial::{
    corner_from_name, corner_from_slint, corner_name, corner_to_slint, dial_label,
    effective_corner, step_focus,
};

/// Every corner. The `match` is exhaustive on purpose: a fifth variant fails to compile here, so the
/// list below cannot silently fall behind the enum.
fn all_corners() -> [DialCorner; 4] {
    fn index(corner: DialCorner) -> usize {
        match corner {
            DialCorner::TopLeft => 0,
            DialCorner::TopRight => 1,
            DialCorner::BottomLeft => 2,
            DialCorner::BottomRight => 3,
        }
    }
    let all = [
        DialCorner::TopLeft,
        DialCorner::TopRight,
        DialCorner::BottomLeft,
        DialCorner::BottomRight,
    ];
    for (i, corner) in all.into_iter().enumerate() {
        assert_eq!(index(corner), i);
    }
    all
}

fn all_kinds() -> [DialCornerKind; 4] {
    fn index(kind: DialCornerKind) -> usize {
        match kind {
            DialCornerKind::TopLeft => 0,
            DialCornerKind::TopRight => 1,
            DialCornerKind::BottomLeft => 2,
            DialCornerKind::BottomRight => 3,
        }
    }
    let all = [
        DialCornerKind::TopLeft,
        DialCornerKind::TopRight,
        DialCornerKind::BottomLeft,
        DialCornerKind::BottomRight,
    ];
    for (i, kind) in all.into_iter().enumerate() {
        assert_eq!(index(kind), i);
    }
    all
}

#[test]
fn corner_name_equals_the_serde_name() {
    for corner in all_corners() {
        let serde = serde_json::to_value(corner).unwrap();
        assert_eq!(serde.as_str(), Some(corner_name(corner)), "{corner:?}");
    }
    assert_eq!(corner_name(DialCorner::TopLeft), "top_left");
    assert_eq!(corner_name(DialCorner::TopRight), "top_right");
    assert_eq!(corner_name(DialCorner::BottomLeft), "bottom_left");
    assert_eq!(corner_name(DialCorner::BottomRight), "bottom_right");
}

#[test]
fn corner_from_name_reads_every_name_back() {
    for corner in all_corners() {
        assert_eq!(corner_from_name(corner_name(corner)), Some(corner));
        let serde = serde_json::to_value(corner).unwrap();
        assert_eq!(corner_from_name(serde.as_str().unwrap()), Some(corner));
    }
}

#[test]
fn corner_from_name_rejects_other_text() {
    for bad in [
        "middle",
        "",
        "TOP_LEFT",
        "Top_Left",
        "topleft",
        "top-left",
        " top_left",
        "top_left ",
        "top_left\n",
    ] {
        assert_eq!(corner_from_name(bad), None, "{bad:?}");
    }
}

#[test]
fn slint_mapping_round_trips_all_four() {
    for corner in all_corners() {
        assert_eq!(corner_from_slint(corner_to_slint(corner)), corner);
    }
    for kind in all_kinds() {
        assert_eq!(corner_to_slint(corner_from_slint(kind)), kind);
    }
}

#[test]
fn slint_mapping_pairs_the_same_named_corners() {
    assert_eq!(
        corner_to_slint(DialCorner::TopLeft),
        DialCornerKind::TopLeft
    );
    assert_eq!(
        corner_to_slint(DialCorner::TopRight),
        DialCornerKind::TopRight
    );
    assert_eq!(
        corner_to_slint(DialCorner::BottomLeft),
        DialCornerKind::BottomLeft
    );
    assert_eq!(
        corner_to_slint(DialCorner::BottomRight),
        DialCornerKind::BottomRight
    );
    // The four images are distinct.
    for (i, a) in all_corners().into_iter().enumerate() {
        for b in all_corners().into_iter().skip(i + 1) {
            assert_ne!(corner_to_slint(a), corner_to_slint(b));
        }
    }
}

#[test]
fn effective_corner_override_wins() {
    for setting in all_corners() {
        for wanted in all_corners() {
            assert_eq!(
                effective_corner(Some(corner_name(wanted)), setting),
                wanted,
                "setting {setting:?}"
            );
        }
    }
}

#[test]
fn effective_corner_falls_back_to_the_setting() {
    for setting in all_corners() {
        assert_eq!(effective_corner(None, setting), setting);
        assert_eq!(effective_corner(Some("middle"), setting), setting);
        assert_eq!(effective_corner(Some(""), setting), setting);
        assert_eq!(effective_corner(Some("TOP_LEFT"), setting), setting);
    }
}

#[test]
fn label_without_a_group_is_just_tab_dial() {
    assert_eq!(dial_label(None, false, 0.0), "Tab dial");
    assert_eq!(dial_label(Some(""), false, 0.0), "Tab dial");
    assert_eq!(dial_label(Some("   "), false, 0.0), "Tab dial");
}

#[test]
fn label_names_the_group() {
    assert_eq!(
        dial_label(Some("Research"), false, 0.0),
        "Tab dial, Research"
    );
}

#[test]
fn label_appends_nothing_when_not_loading() {
    for progress in [0.0, 0.65, 1.0, -1.0, f32::NAN] {
        assert_eq!(
            dial_label(Some("Research"), false, progress),
            "Tab dial, Research",
            "{progress}"
        );
        assert_eq!(dial_label(None, false, progress), "Tab dial");
    }
}

#[test]
fn label_gives_the_loading_percent() {
    let cases = [
        (0.0, "0"),
        (0.65, "65"),
        (1.0, "100"),
        (0.004, "0"),
        (0.006, "1"),
        (0.994, "99"),
        (0.996, "100"),
        (0.5, "50"),
    ];
    for (progress, percent) in cases {
        assert_eq!(
            dial_label(Some("Research"), true, progress),
            format!("Tab dial, Research, page loading {percent} percent"),
            "{progress}"
        );
    }
    assert_eq!(
        dial_label(None, true, 0.65),
        "Tab dial, page loading 65 percent"
    );
}

#[test]
fn label_clamps_progress_above_one() {
    assert_eq!(
        dial_label(None, true, 1.5),
        "Tab dial, page loading 100 percent"
    );
    assert_eq!(
        dial_label(None, true, f32::INFINITY),
        "Tab dial, page loading 100 percent"
    );
}

#[test]
fn label_with_unknown_progress_says_only_page_loading() {
    for progress in [-1.0, -0.001, f32::NEG_INFINITY, f32::NAN] {
        let label = dial_label(Some("Research"), true, progress);
        assert_eq!(label, "Tab dial, Research, page loading", "{progress}");
        assert!(!label.contains("NaN") && !label.contains("percent"));
    }
    assert_eq!(dial_label(None, true, -1.0), "Tab dial, page loading");
}

#[test]
fn step_focus_moves_and_wraps_both_ways() {
    let ids = [10, 20, 30, 40];
    assert_eq!(step_focus(&ids, 10, 1), 20);
    assert_eq!(step_focus(&ids, 20, 1), 30);
    assert_eq!(step_focus(&ids, 30, 1), 40);
    assert_eq!(step_focus(&ids, 40, 1), 10);
    assert_eq!(step_focus(&ids, 40, -1), 30);
    assert_eq!(step_focus(&ids, 20, -1), 10);
    assert_eq!(step_focus(&ids, 10, -1), 40);
}

#[test]
fn step_focus_unknown_focus_gives_the_ends() {
    let ids = [10, 20, 30];
    for unknown in [-1, 0, 11, 999] {
        assert_eq!(step_focus(&ids, unknown, 1), 10, "{unknown}");
        assert_eq!(step_focus(&ids, unknown, -1), 30, "{unknown}");
    }
}

#[test]
fn step_focus_empty_list_gives_minus_one() {
    for focused in [-1, 0, 7] {
        for step in [-2, -1, 0, 1, 2] {
            assert_eq!(step_focus(&[], focused, step), -1);
        }
    }
}

#[test]
fn step_focus_one_tab_stays_on_it() {
    assert_eq!(step_focus(&[5], 5, 1), 5);
    assert_eq!(step_focus(&[5], 5, -1), 5);
    assert_eq!(step_focus(&[5], -1, 1), 5);
    assert_eq!(step_focus(&[5], -1, -1), 5);
    assert_eq!(step_focus(&[5], 5, 0), 5);
    assert_eq!(step_focus(&[0], -1, 1), 0);
}

#[test]
fn step_focus_uses_the_sign_of_the_step() {
    let ids = [1, 2, 3, 4, 5];
    for step in [1, 2, 7, i32::MAX] {
        assert_eq!(step_focus(&ids, 2, step), 3, "{step}");
        assert_eq!(step_focus(&ids, 5, step), 1, "{step}");
    }
    for step in [-1, -2, -7, i32::MIN] {
        assert_eq!(step_focus(&ids, 2, step), 1, "{step}");
        assert_eq!(step_focus(&ids, 1, step), 5, "{step}");
    }
}

#[test]
fn step_focus_zero_keeps_a_present_focus_else_minus_one() {
    let ids = [10, 20, 30];
    assert_eq!(step_focus(&ids, 20, 0), 20);
    assert_eq!(step_focus(&ids, 10, 0), 10);
    assert_eq!(step_focus(&ids, 99, 0), -1);
    assert_eq!(step_focus(&ids, -1, 0), -1);
}

#[test]
fn step_focus_duplicates_use_the_first_match() {
    let ids = [7, 8, 7, 9];
    // The first 7 is at index 0, so forward gives 8 (not 9) and backward wraps to 9 (not 8).
    assert_eq!(step_focus(&ids, 7, 1), 8);
    assert_eq!(step_focus(&ids, 7, -1), 9);
    // A duplicate at the end of the list: the first match is at index 0.
    let ids = [3, 4, 3];
    assert_eq!(step_focus(&ids, 3, 1), 4);
    assert_eq!(step_focus(&ids, 3, -1), 3);
}
