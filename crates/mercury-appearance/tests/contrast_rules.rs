// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The contrast rules, run over the palettes.
//!
//! - Every selectable palette passes every rule in both schemes (run with `--nocapture` to print
//!   every ratio).
//! - A completeness guard: every role is a foreground or background of a rule, or is exempt with
//!   a reason, so adding a role forces a decision.
//! - The accent: `#C2491A` is the fill (decision D1), the roles drawn as text are the same hue,
//!   deepened until they read.
//! - Every pair of the interface's own `palette_contrast.rs` is covered by a rule.
//! - The brand hue and the non-blue information bar, as that test asserts today.

use std::collections::HashSet;

use mercury_appearance::Colour;
use mercury_appearance::contrast::{EXEMPT, Minimum, RULES, check, measure};
use mercury_appearance::palette::{Palette, Role, Scheme};
use mercury_appearance::palettes::{self, hermes, mercury_b2};

/// The least margin the shipped palette keeps over 4.5:1 on text, so that an anti-aliased
/// rendering or a small tweak does not tip a pair over the line.
const TEXT_MARGIN: f64 = 4.55;

fn ratio(palette: &Palette, scheme: Scheme, foreground: Role, background: Role) -> f64 {
    palette
        .colour(scheme, foreground)
        .contrast_ratio(palette.colour(scheme, background))
}

/// Hue in degrees (0..360) of an opaque colour.
fn hue(colour: Colour) -> f64 {
    let [r, g, b] = [colour.r, colour.g, colour.b].map(|value| f64::from(value) / 255.0);
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
fn every_selectable_palette_passes_every_rule_in_both_schemes() {
    let mut checked = 0;
    let mut failures = Vec::new();
    for palette in palettes::selectable() {
        for scheme in Scheme::ALL {
            let measurements = measure(palette, scheme);
            assert!(!measurements.is_empty());
            let mut lowest = f64::MAX;
            for measurement in &measurements {
                println!("{:10} {:5} {measurement}", palette.id, scheme.name());
                lowest = lowest.min(measurement.ratio / measurement.minimum.ratio());
                checked += 1;
            }
            println!(
                "{:10} {:5} {} pairs; the closest to its minimum is at {:.3} times it",
                palette.id,
                scheme.name(),
                measurements.len(),
                lowest
            );
            for violation in check(palette, scheme) {
                failures.push(format!("{} {}: {violation}", palette.id, scheme.name()));
            }
        }
    }
    assert!(checked > 0, "no selectable palette was checked");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn the_shipped_palette_keeps_a_margin_on_text() {
    let b2 = &mercury_b2::PALETTE;
    let mut thin = Vec::new();
    for scheme in Scheme::ALL {
        for measurement in measure(b2, scheme) {
            if measurement.minimum == Minimum::Text && measurement.ratio < TEXT_MARGIN {
                thin.push(format!("{}: {measurement}", scheme.name()));
            }
        }
    }
    assert!(
        thin.is_empty(),
        "text pairs under {TEXT_MARGIN}:1:\n{}",
        thin.join("\n")
    );
}

#[test]
fn every_role_is_in_a_rule_or_exempt_with_a_reason() {
    let mut ruled: HashSet<Role> = HashSet::new();
    for rule in RULES {
        ruled.insert(rule.foreground);
        ruled.extend(rule.backgrounds.iter().copied());
    }
    let mut exempt: HashSet<Role> = HashSet::new();
    for &(role, reason) in EXEMPT {
        assert!(exempt.insert(role), "{} is exempt twice", role.slint_name());
        assert!(
            reason.trim().len() > 10,
            "{} needs a reason for its exemption",
            role.slint_name()
        );
        assert!(
            !ruled.contains(&role),
            "{} is exempt and also in a rule: pick one",
            role.slint_name()
        );
    }
    let forgotten: Vec<&str> = Role::ALL
        .into_iter()
        .filter(|role| !ruled.contains(role) && !exempt.contains(role))
        .map(Role::slint_name)
        .collect();
    assert!(
        forgotten.is_empty(),
        "roles that are in no rule and not exempt (add a rule, or exempt them with a reason): {forgotten:?}"
    );
}

#[test]
fn the_rules_are_well_formed() {
    let mut pairs = HashSet::new();
    for rule in RULES {
        let name = rule.foreground.slint_name();
        assert!(!rule.backgrounds.is_empty(), "{name} has no background");
        assert!(rule.why.trim().len() > 10, "{name} needs a reason");
        assert!(
            !rule.backgrounds.contains(&rule.foreground),
            "{name} is its own background"
        );
        for &background in rule.backgrounds {
            assert!(
                pairs.insert((rule.foreground, background)),
                "{name} on {} is in two rules",
                background.slint_name()
            );
        }
    }
    // WCAG 2.2: 4.5:1 for text (1.4.3) and 3:1 for interface parts (1.4.11).
    assert!((Minimum::Text.ratio() - 4.5).abs() < f64::EPSILON);
    assert!((Minimum::Ui.ratio() - 3.0).abs() < f64::EPSILON);
}

#[test]
fn rules_only_name_roles_that_are_opaque() {
    // A ratio is exact only for opaque colours; the veils (scrim, shadow) are exempt for that
    // reason, and no rule may name a role that turns translucent in some palette.
    for palette in palettes::all() {
        for scheme in Scheme::ALL {
            for rule in RULES {
                for role in std::iter::once(rule.foreground).chain(rule.backgrounds.iter().copied())
                {
                    assert_eq!(
                        palette.colour(scheme, role).a,
                        255,
                        "{} {} {}",
                        palette.id,
                        scheme.name(),
                        role.slint_name()
                    );
                }
            }
        }
    }
}

#[test]
fn the_owners_accent_is_the_fill_and_the_text_accent_is_its_deepened_form() {
    // D1. #C2491A on #F8F4EE measures 4.497:1, 0.003 under what text needs, and 4.05:1 on
    // #EFE8DF; so it is the fill and mark accent, and the text roles are the same hue, deeper.
    let fill = Colour::hex(0xc2491a);
    let toolbar = Colour::hex(0xf8f4ee);
    let window = Colour::hex(0xefe8df);
    let b2 = &mercury_b2::PALETTE;
    let light = Scheme::Light;
    assert_eq!(b2.colour(light, Role::ToolbarBackground), toolbar);
    assert_eq!(b2.colour(light, Role::WindowBackground), window);

    let as_text_on_toolbar = fill.contrast_ratio(toolbar);
    let as_text_on_window = fill.contrast_ratio(window);
    println!(
        "#C2491A as text on #F8F4EE: {as_text_on_toolbar:.3}, on #EFE8DF: {as_text_on_window:.3}"
    );
    assert!(
        (4.49..4.5).contains(&as_text_on_toolbar),
        "the measurement D1 rests on has changed: {as_text_on_toolbar}"
    );
    assert!(as_text_on_window < 4.5);

    // As a fill: the primary button and the selected row are exactly the owner's colour; white
    // text on it reads (4.93:1), and as a mark it clears 3:1 on both chrome surfaces.
    for role in [Role::PrimaryButtonBackground, Role::ItemSelected] {
        assert_eq!(b2.colour(light, role), fill, "{}", role.slint_name());
    }
    let white_on_fill = Colour::WHITE.contrast_ratio(fill);
    println!("white on #C2491A: {white_on_fill:.3}");
    assert!(white_on_fill >= 4.9, "{white_on_fill}");
    assert_eq!(b2.colour(light, Role::PrimaryButtonText), Colour::WHITE);
    assert_eq!(b2.colour(light, Role::ItemSelectedText), Colour::WHITE);
    for (name, surface) in [("#F8F4EE", toolbar), ("#EFE8DF", window)] {
        let mark = fill.contrast_ratio(surface);
        println!("#C2491A as a mark on {name}: {mark:.3}");
        assert!(mark >= Minimum::Ui.ratio(), "{name}: {mark}");
    }

    // As text: the same hue, deeper, with a margin, on both chrome surfaces.
    for role in [Role::Accent, Role::ButtonLabelHover, Role::MatchHighlight] {
        let text = b2.colour(light, role);
        for (name, surface) in [("#F8F4EE", toolbar), ("#EFE8DF", window)] {
            let measured = text.contrast_ratio(surface);
            println!(
                "{} {} on {name}: {measured:.3}",
                role.slint_name(),
                text.to_hex()
            );
            assert!(
                measured >= TEXT_MARGIN,
                "{} {} on {name} is {measured:.3}",
                role.slint_name(),
                text.to_hex()
            );
        }
        assert!(
            text.luminance() < fill.luminance(),
            "{} is not deeper",
            role.slint_name()
        );
        assert!(
            (hue(text) - hue(fill)).abs() <= 2.0,
            "{} {} is not the fill's hue ({:.1} against {:.1})",
            role.slint_name(),
            text.to_hex(),
            hue(text),
            hue(fill)
        );
    }

    // Dark: the amber is readable as text and as a fill on both dark chrome surfaces.
    let amber = b2.colour(Scheme::Dark, Role::Accent);
    for surface in [Role::ToolbarBackground, Role::WindowBackground] {
        let measured = amber.contrast_ratio(b2.colour(Scheme::Dark, surface));
        println!(
            "dark accent {} on {}: {measured:.3}",
            amber.to_hex(),
            surface.slint_name()
        );
        assert!(measured >= TEXT_MARGIN, "{measured}");
    }
    assert_eq!(
        b2.colour(Scheme::Dark, Role::PrimaryButtonBackground),
        amber
    );
}

#[test]
fn the_orange_roles_stay_in_the_brand_hue_and_the_information_bar_is_not_blue() {
    // As crates/mercury-ui/tests/palette_contrast.rs asserts for today's palette: Sun orange
    // (15 to 45 degrees), never the blue Hermes inherited (195 to 215), and a non-blue
    // information bar.
    let b2 = &mercury_b2::PALETTE;
    let orange = [
        Role::Accent,
        Role::ButtonLabelHover,
        Role::PrimaryButtonBackground,
        Role::PrimaryButtonHover,
        Role::PrimaryButtonPressed,
        Role::FocusRing,
        Role::MatchHighlight,
        Role::ItemSelected,
    ];
    for scheme in Scheme::ALL {
        for role in orange {
            let colour = b2.colour(scheme, role);
            let degrees = hue(colour);
            assert!(
                (15.0..=45.0).contains(&degrees),
                "{} {} {} has hue {degrees:.0}, not the brand orange",
                scheme.name(),
                role.slint_name(),
                colour.to_hex()
            );
        }
        for role in [Role::InfoBackground, Role::InfoBorder] {
            let colour = b2.colour(scheme, role);
            let degrees = hue(colour);
            assert!(
                !(180.0..=240.0).contains(&degrees),
                "{} {} {} is blue (hue {degrees:.0})",
                scheme.name(),
                role.slint_name(),
                colour.to_hex()
            );
        }
    }
}

#[test]
fn the_capture_indicator_is_a_recording_red() {
    let b2 = &mercury_b2::PALETTE;
    for scheme in Scheme::ALL {
        let colour = b2.colour(scheme, Role::CaptureIndicator);
        let degrees = hue(colour);
        assert!(
            degrees.min(360.0 - degrees) <= 12.0,
            "{} {} has hue {degrees:.0}, not red",
            scheme.name(),
            colour.to_hex()
        );
        // Strongly red: the red channel dominates.
        assert!(colour.r > colour.g.saturating_add(100) && colour.r > colour.b.saturating_add(100));
    }
    // The glyph is white on it in both schemes (it is a plain `#ffffff` in the Slint file).
    for scheme in Scheme::ALL {
        assert_eq!(
            b2.colour(scheme, Role::CaptureIndicatorGlyph),
            Colour::WHITE
        );
    }
}

/// The pairs `crates/mercury-ui/tests/palette_contrast.rs` checks today, as (foreground,
/// background, minimum).
fn legacy_pairs() -> Vec<(Role, Role, Minimum)> {
    let surfaces = [
        Role::WindowBackground,
        Role::ToolbarBackground,
        Role::ContentBackground,
        Role::TabActive,
        Role::FieldBackground,
        Role::PopupBackground,
        Role::SheetBackground,
    ];
    let mut pairs = Vec::new();
    for surface in surfaces {
        pairs.push((Role::Accent, surface, Minimum::Text));
        pairs.push((Role::FocusRing, surface, Minimum::Ui));
        pairs.push((Role::ButtonLabelHover, surface, Minimum::Text));
        pairs.push((Role::PrimaryButtonBackground, surface, Minimum::Ui));
    }
    pairs.push((Role::MatchHighlight, Role::PopupBackground, Minimum::Text));
    pairs.push((Role::MatchHighlight, Role::FieldBackground, Minimum::Text));
    pairs.push((Role::ItemSelected, Role::PopupBackground, Minimum::Ui));
    pairs.push((Role::ItemSelectedText, Role::ItemSelected, Minimum::Text));
    pairs.push((Role::ItemSelected, Role::PagePopupBackground, Minimum::Ui));
    for fill in [
        Role::PrimaryButtonBackground,
        Role::PrimaryButtonHover,
        Role::PrimaryButtonPressed,
        Role::Accent,
    ] {
        pairs.push((Role::PrimaryButtonText, fill, Minimum::Text));
    }
    pairs.push((Role::InfoText, Role::InfoBackground, Minimum::Text));
    pairs.push((Role::Text, Role::InfoBackground, Minimum::Text));
    for surface in [
        Role::PopupBackground,
        Role::ContentBackground,
        Role::ItemHover,
    ] {
        pairs.push((Role::Text, surface, Minimum::Text));
        pairs.push((Role::TextSecondary, surface, Minimum::Text));
    }
    for surface in [Role::ContentBackground, Role::ItemHover] {
        pairs.push((Role::Danger, surface, Minimum::Text));
    }
    pairs.push((Role::Accent, Role::ItemHover, Minimum::Text));
    pairs
}

#[test]
fn every_pair_of_the_interface_contrast_test_is_covered_by_a_rule() {
    let legacy = legacy_pairs();
    assert!(legacy.len() > 40, "{}", legacy.len());
    let mut uncovered = Vec::new();
    for (foreground, background, minimum) in legacy {
        let covered = RULES.iter().any(|rule| {
            rule.foreground == foreground
                && rule.backgrounds.contains(&background)
                && rule.minimum.ratio() >= minimum.ratio()
        });
        if !covered {
            uncovered.push(format!(
                "{} on {} ({:?})",
                foreground.slint_name(),
                background.slint_name(),
                minimum
            ));
        }
    }
    assert!(
        uncovered.is_empty(),
        "no rule covers:\n{}",
        uncovered.join("\n")
    );
}

#[test]
fn the_legacy_pairs_hold_for_hermes_as_the_interface_test_says() {
    // The port is faithful if today's palette, which that test passes, passes these pairs here.
    let legacy = hermes::PALETTE;
    let mut failures = Vec::new();
    for scheme in Scheme::ALL {
        for (foreground, background, minimum) in legacy_pairs() {
            let measured = ratio(&legacy, scheme, foreground, background);
            if measured < minimum.ratio() {
                failures.push(format!(
                    "{} {} on {}: {measured:.2}",
                    scheme.name(),
                    foreground.slint_name(),
                    background.slint_name()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Every role the same mid grey: nothing reads on anything.
fn flat(_: Role) -> Colour {
    Colour::hex(0x808080)
}

/// `mercury-b2`'s light table with a `text` that is nearly the window colour.
fn faint_text(role: Role) -> Colour {
    if role == Role::Text {
        Colour::hex(0xe9e2d8)
    } else {
        (mercury_b2::PALETTE.light)(role)
    }
}

#[test]
fn check_reports_every_failing_pair_with_the_ratio_and_both_colours() {
    let palette = Palette {
        id: "flat",
        name: "Flat",
        selectable: false,
        light: flat,
        dark: flat,
    };
    let all = measure(&palette, Scheme::Light);
    let violations = check(&palette, Scheme::Light);
    let pairs: usize = RULES.iter().map(|rule| rule.backgrounds.len()).sum();
    assert_eq!(all.len(), pairs);
    assert_eq!(violations.len(), pairs, "grey on grey fails every pair");
    for violation in &violations {
        assert!((violation.ratio - 1.0).abs() < 1e-9);
        assert_eq!(violation.foreground_colour, Colour::hex(0x808080));
        assert_eq!(violation.background_colour, Colour::hex(0x808080));
        assert!(!violation.passes());
        let message = violation.to_string();
        assert!(
            message.contains(violation.foreground.slint_name()),
            "{message}"
        );
        assert!(
            message.contains(violation.background.slint_name()),
            "{message}"
        );
        assert!(
            message.contains("#808080") && message.contains("1.00"),
            "{message}"
        );
    }

    let palette = Palette {
        id: "faint",
        name: "Faint",
        selectable: false,
        light: faint_text,
        dark: flat,
    };
    let violations = check(&palette, Scheme::Light);
    assert!(!violations.is_empty());
    assert!(
        violations
            .iter()
            .all(|violation| violation.foreground == Role::Text),
        "only the text role was changed: {violations:?}"
    );
    assert!(
        violations
            .iter()
            .any(|violation| violation.background == Role::WindowBackground)
    );
    // A pair that still passes is not reported.
    assert!(violations.len() < measure(&palette, Scheme::Light).len());
}

#[test]
fn a_translucent_foreground_is_measured_over_its_background() {
    // The rules only name opaque roles today, but check() composites a veil first rather than
    // pretending its colour channels are the colour that reaches the eye.
    fn veil(role: Role) -> Colour {
        match role {
            Role::Text => Colour::rgba(0, 0, 0, 0x10),
            _ => Colour::WHITE,
        }
    }
    let palette = Palette {
        id: "veil",
        name: "Veil",
        selectable: false,
        light: veil,
        dark: veil,
    };
    let measurement = measure(&palette, Scheme::Light)
        .into_iter()
        .find(|measurement| measurement.foreground == Role::Text)
        .expect("text has a rule");
    // Black at 6% over white is a very light grey: far less than black's 21:1.
    assert!(measurement.ratio < 1.3, "{}", measurement.ratio);
}
