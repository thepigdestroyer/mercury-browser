// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The six palettes designed on 22 Nov 2026 (the four the owner chose to ship, Signal, Forest,
//! Abyss and Glacier, and the two that stay in the code, Graphite and Midnight) held to the
//! crate's own contrast rules in both schemes, so the two that cannot be chosen stay as good as
//! the four that can. Run with `--nocapture` for the report (lowest ratios, the exempt pairs).

use mercury_appearance::palette::{Role, Scheme};
use mercury_appearance::palettes;
use mercury_appearance::{Colour, EXEMPT, Minimum, Palette, check, measure, oklab};

/// The ids of the six, and which of them a user can choose.
const SIX: [(&str, bool); 6] = [
    ("signal", true),
    ("forest", true),
    ("abyss", true),
    ("glacier", true),
    ("graphite", false),
    ("midnight", false),
];

fn six() -> Vec<&'static Palette> {
    SIX.iter()
        .map(|&(id, _)| palettes::find(id).unwrap_or_else(|| panic!("{id} is in the code")))
        .collect()
}

fn ratio(palette: &Palette, scheme: Scheme, foreground: Role, background: Role) -> f64 {
    let fg = palette.colour(scheme, foreground);
    let bg = palette.colour(scheme, background);
    fg.over(bg).contrast_ratio(bg)
}

fn hue(colour: Colour) -> f64 {
    let (r, g, b) = (
        f64::from(colour.r) / 255.0,
        f64::from(colour.g) / 255.0,
        f64::from(colour.b) / 255.0,
    );
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let delta = max - min;
    if delta == 0.0 {
        return 0.0;
    }
    let sector = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    sector * 60.0
}

#[test]
fn the_six_have_every_role_and_unique_ids_and_four_can_be_chosen() {
    let all = six();
    assert_eq!(all.len(), 6);
    let mut ids: Vec<&str> = all.iter().map(|palette| palette.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 6);
    for (palette, &(id, selectable)) in all.into_iter().zip(&SIX) {
        assert_eq!(palette.id, id);
        assert_eq!(palette.selectable, selectable, "{}", palette.id);
        for scheme in Scheme::ALL {
            assert_eq!(palette.colours(scheme).count(), Role::COUNT);
            for (role, colour) in palette.colours(scheme) {
                let veil = matches!(role, Role::Scrim | Role::Shadow);
                assert_eq!(
                    colour.a != 255,
                    veil,
                    "{} {}",
                    palette.id,
                    role.slint_name()
                );
            }
        }
    }
}

#[test]
fn every_candidate_passes_every_rule_in_both_schemes() {
    let mut failures = Vec::new();
    for palette in six() {
        for scheme in Scheme::ALL {
            let measurements = measure(palette, scheme);
            let lowest_text = measurements
                .iter()
                .filter(|m| m.minimum == Minimum::Text)
                .map(|m| m.ratio)
                .fold(f64::MAX, f64::min);
            let lowest_ui = measurements
                .iter()
                .filter(|m| m.minimum == Minimum::Ui)
                .map(|m| m.ratio)
                .fold(f64::MAX, f64::min);
            let text_pairs = measurements
                .iter()
                .filter(|m| m.minimum == Minimum::Text)
                .count();
            let ui_pairs = measurements.len() - text_pairs;
            println!(
                "CONTRAST {:9} {:5} {} pairs ({text_pairs} text, {ui_pairs} ui): lowest text {lowest_text:.2}, lowest ui {lowest_ui:.2}",
                palette.id,
                scheme.name(),
                measurements.len(),
            );
            for violation in check(palette, scheme) {
                failures.push(format!("{} {}: {violation}", palette.id, scheme.name()));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn report_the_exempt_pairs() {
    // Each exempt role against the surface it is drawn on or beside, so the owner sees how
    // visible the hairlines are.
    let pairs = [
        (Role::ToolbarDivider, Role::ToolbarBackground),
        (Role::FieldBorder, Role::FieldBackground),
        (Role::SearchFieldBorder, Role::SearchFieldBackground),
        (Role::StatusBorder, Role::StatusBackground),
        (Role::NoticeBorder, Role::NoticeBackground),
        (Role::InfoBorder, Role::InfoBackground),
        (Role::PopupBorder, Role::PopupBackground),
        (Role::PagePopupBorder, Role::PagePopupBackground),
        (Role::Separator, Role::PopupBackground),
        (Role::DisabledText, Role::PopupBackground),
    ];
    for (role, _) in EXEMPT {
        if !matches!(role, Role::Scrim | Role::Shadow) {
            assert!(
                pairs.iter().any(|(r, _)| r == role),
                "{role:?} is not reported"
            );
        }
    }
    for palette in six() {
        for scheme in Scheme::ALL {
            let cells: Vec<String> = pairs
                .iter()
                .map(|&(fg, bg)| {
                    format!("{} {:.2}", fg.slint_name(), ratio(palette, scheme, fg, bg))
                })
                .collect();
            println!(
                "EXEMPT {:9} {:5} {}",
                palette.id,
                scheme.name(),
                cells.join(" | ")
            );
        }
    }
}

#[test]
fn the_house_rules_that_are_not_in_the_contrast_rules_hold() {
    for palette in six() {
        for scheme in Scheme::ALL {
            // The information bar is never blue (Role docs); the capture mark is a recording red.
            for role in [Role::InfoBackground, Role::InfoBorder] {
                let degrees = hue(palette.colour(scheme, role));
                assert!(
                    !(180.0..=240.0).contains(&degrees),
                    "{} {} {} is blue",
                    palette.id,
                    scheme.name(),
                    role.slint_name()
                );
            }
            let capture = palette.colour(scheme, Role::CaptureIndicator);
            let degrees = hue(capture);
            assert!(
                degrees.min(360.0 - degrees) <= 12.0,
                "{} capture hue",
                palette.id
            );
            assert!(
                capture.r > capture.g.saturating_add(100)
                    && capture.r > capture.b.saturating_add(100)
            );
            let danger = hue(palette.colour(scheme, Role::Danger));
            assert!(
                danger.min(360.0 - danger) <= 12.0,
                "{} danger hue",
                palette.id
            );
            // The text tiers step down in strength on the toolbar.
            let on = |role| ratio(palette, scheme, role, Role::ToolbarBackground);
            assert!(on(Role::Text) > on(Role::TextSecondary), "{}", palette.id);
            assert!(
                on(Role::TextSecondary) >= on(Role::TextMuted),
                "{}",
                palette.id
            );
        }
    }
}

#[test]
fn report_how_far_apart_the_palettes_are() {
    let all: Vec<&Palette> = palettes::all().iter().collect();
    for scheme in Scheme::ALL {
        for (index, a) in all.iter().enumerate() {
            for b in &all[index + 1..] {
                let frame = oklab::delta_e(
                    a.colour(scheme, Role::WindowBackground),
                    b.colour(scheme, Role::WindowBackground),
                );
                let accent = oklab::delta_e(
                    a.colour(scheme, Role::PrimaryButtonBackground),
                    b.colour(scheme, Role::PrimaryButtonBackground),
                );
                println!(
                    "DISTANCE {:5} {:9} {:9} frame {frame:.3} accent {accent:.3}",
                    scheme.name(),
                    a.id,
                    b.id
                );
            }
        }
    }
}
