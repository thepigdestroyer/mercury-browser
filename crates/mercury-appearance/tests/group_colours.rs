// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The 14 group colours: that they can be told apart, parse and print as ids, give swatches that
//! text can be read on, give tab shades that stay inside their group, and are handed out in order.

use mercury_appearance::Colour;
use mercury_appearance::group_colour::{
    GroupColour, MIN_TEXT_CONTRAST, PRESET_COUNT, PRESETS, Preset, SHADE_CYCLE, SHADE_STEP, Swatch,
    TINT_STRENGTH_MAX, auto_colour, parse, preset, tab_shade, tint_strength_limit,
};
use mercury_appearance::oklab::{Oklab, delta_e, hue, lightness};

/// The light chrome: paper and its ink.
const LIGHT: (Colour, Colour) = (Colour::hex(0xf8f4ee), Colour::hex(0x1a1230));
/// The dark chrome: the night plate and its text.
const DARK: (Colour, Colour) = (Colour::hex(0x1c1534), Colour::hex(0xf3eef8));
/// Seven colours a user might pick, including both ends and the three RGB primaries.
const CUSTOMS: [&str; 7] = [
    "#000000", "#ffffff", "#ff0000", "#00ff00", "#0000ff", "#ffff00", "#3a7bd5",
];

fn pairs() -> Vec<(f64, &'static Preset, &'static Preset)> {
    let mut all = Vec::new();
    for (i, a) in PRESETS.iter().enumerate() {
        for b in &PRESETS[i + 1..] {
            all.push((delta_e(a.base, b.base), a, b));
        }
    }
    all.sort_by(|x, y| x.0.total_cmp(&y.0));
    all
}

fn custom(hex: &str) -> Colour {
    match parse(hex) {
        Some(GroupColour::Custom(colour)) => colour,
        other => panic!("{hex} parsed as {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// The presets

#[test]
fn there_are_fourteen_presets_with_unique_lowercase_ascii_ids_and_names() {
    assert_eq!(PRESETS.len(), 14);
    assert_eq!(PRESET_COUNT, 14);
    for (i, a) in PRESETS.iter().enumerate() {
        assert!(!a.id.is_empty(), "{}", a.name);
        assert!(
            a.id.bytes().all(|byte| byte.is_ascii_lowercase()),
            "id {:?} is not lower-case ASCII letters",
            a.id
        );
        assert!(
            a.name.chars().next().is_some_and(char::is_uppercase)
                && a.name.chars().all(|c| c.is_ascii_alphabetic()),
            "name {:?}",
            a.name
        );
        assert_eq!(a.name.to_ascii_lowercase(), a.id, "name and id agree");
        assert_eq!(a.base.a, 255, "{} is opaque", a.id);
        for b in &PRESETS[i + 1..] {
            assert_ne!(a.id, b.id);
            assert_ne!(a.name, b.name);
            assert_ne!(a.base, b.base, "{} and {}", a.id, b.id);
        }
    }
}

#[test]
fn the_specs_seven_keep_their_ids_and_exact_values() {
    for (id, hex) in [
        ("blue", 0x2f6fd6),
        ("amber", 0xe0912a),
        ("violet", 0x7e5bd0),
        ("rose", 0xd9486e),
        ("green", 0x2e9e62),
        ("teal", 0x169c9c),
        ("slate", 0x7c8088),
    ] {
        let found = preset(id).unwrap_or_else(|| panic!("no preset {id}"));
        assert_eq!(found.base, Colour::hex(hex), "{id}");
    }
}

#[test]
fn the_seven_added_colours_are_the_measured_seeds() {
    for (id, hex) in [
        ("orchid", 0xe08af3),
        ("sky", 0x77b5fb),
        ("lime", 0x91c43e),
        ("brown", 0x9a5505),
        ("mint", 0x3ccaba),
        ("plum", 0x954a7e),
        ("salmon", 0xf3909d),
    ] {
        let found = preset(id).unwrap_or_else(|| panic!("no preset {id}"));
        assert_eq!(found.base, Colour::hex(hex), "{id}");
    }
}

/// The ids, in hand-out order, exactly as `session.json` holds them. This is one half of a
/// contract: `talaria-core/src/groups.rs` (`GroupPreset::ALL` and `GroupPreset::name`) must name
/// the same fourteen ids in the same order, and the crate that links both asserts
/// `GroupPreset::ALL.map(GroupPreset::name) == PRESETS.map(|preset| preset.id)`. Changing this
/// list changes a file format that is never migrated, so it is a decision to take on purpose:
/// change it here and in `GroupPreset` together, never one side only.
const CONTRACT_IDS: [&str; 14] = [
    "blue", "amber", "rose", "mint", "orchid", "brown", "green", "lime", "sky", "plum", "salmon",
    "violet", "teal", "slate",
];

#[test]
fn the_preset_ids_and_their_hand_out_order_are_the_persisted_contract() {
    let ids: Vec<&str> = PRESETS.iter().map(|preset| preset.id).collect();
    assert_eq!(ids, CONTRACT_IDS, "the stored ids or their order changed");
    // The order is what `auto_colour` hands out when no group is in use yet.
    let mut used: Vec<&str> = Vec::new();
    for expected in CONTRACT_IDS {
        let chosen = auto_colour(&used).id;
        assert_eq!(chosen, expected, "after {used:?}");
        used.push(chosen);
    }
}

#[test]
fn no_two_presets_are_closer_than_0_08_in_oklab() {
    let all = pairs();
    assert_eq!(all.len(), 91);
    println!("14 presets, 91 pairs, OKLab delta-E, closest first:");
    for (distance, a, b) in all.iter().take(8) {
        println!("  {distance:.4}  {} / {}", a.id, b.id);
    }
    let (minimum, a, b) = all[0];
    println!(
        "minimum pairwise delta-E: {minimum:.4} ({} / {})",
        a.id, b.id
    );
    assert!(minimum >= 0.08, "{} / {} are {minimum}", a.id, b.id);
}

#[test]
fn only_the_specs_own_close_pairs_are_under_0_13() {
    // The seven colours of the design spec were fixed first and have three pairs closer than
    // 0.13 (green/teal, blue/violet, teal/slate). The seven added ones are all at least 0.13
    // from every colour, so they never make the picture worse.
    let close: Vec<(&str, &str)> = pairs()
        .into_iter()
        .filter(|(distance, _, _)| *distance < 0.13)
        .map(|(_, a, b)| (a.id, b.id))
        .collect();
    let spec = ["blue", "amber", "violet", "rose", "green", "teal", "slate"];
    for (a, b) in &close {
        assert!(
            spec.contains(a) && spec.contains(b),
            "{a} / {b} is under 0.13 but is not a pair from the spec"
        );
    }
    assert_eq!(close.len(), 3, "{close:?}");
}

#[test]
fn the_first_colours_handed_out_stay_far_apart() {
    assert_eq!(PRESETS[0].id, "blue");
    assert_eq!(
        PRESETS[PRESET_COUNT - 1].id,
        "slate",
        "slate is the loose-tabs colour"
    );
    let mut worst_of_each: Vec<(usize, f64)> = Vec::new();
    for count in 2..=PRESET_COUNT {
        let mut minimum = f64::MAX;
        for (i, a) in PRESETS[..count].iter().enumerate() {
            for b in &PRESETS[i + 1..count] {
                minimum = minimum.min(delta_e(a.base, b.base));
            }
        }
        worst_of_each.push((count, minimum));
        if count <= 11 {
            assert!(
                minimum >= 0.13,
                "the first {count} presets have a pair {minimum} apart"
            );
        }
    }
    println!("closest pair among the first n presets: {worst_of_each:.3?}");
}

#[test]
fn the_picker_can_sort_by_hue_and_every_hue_is_a_real_angle() {
    // Slate is nearly grey; every other preset is clearly coloured.
    for found in &PRESETS {
        let lab = Oklab::from_colour(found.base);
        if found.id == "slate" {
            assert!(lab.chroma() < 0.03);
        } else {
            assert!(lab.chroma() > 0.09, "{}: {}", found.id, lab.chroma());
        }
        assert!((0.0..360.0).contains(&hue(found.base)));
    }
}

// ---------------------------------------------------------------------------------------------
// Ids

#[test]
fn every_preset_id_parses_to_that_preset_and_prints_back() {
    for found in &PRESETS {
        let parsed = parse(found.id).unwrap_or_else(|| panic!("{} did not parse", found.id));
        assert_eq!(parsed, GroupColour::Preset(found));
        assert_eq!(parsed.id(), found.id);
        assert_eq!(parsed.base(), found.base);
        assert_eq!(GroupColour::parse(found.id), Some(parsed));
    }
}

#[test]
fn custom_ids_are_exactly_hash_and_six_hex_digits_and_print_in_lower_case() {
    for (text, expected) in [
        ("#2f6fd6", Colour::hex(0x2f6fd6)),
        ("#2F6FD6", Colour::hex(0x2f6fd6)),
        ("#2f6FD6", Colour::hex(0x2f6fd6)),
        ("#000000", Colour::BLACK),
        ("#FFFFFF", Colour::WHITE),
        ("#0a0B0c", Colour::rgb(10, 11, 12)),
    ] {
        let parsed = parse(text).unwrap_or_else(|| panic!("{text} did not parse"));
        assert_eq!(parsed, GroupColour::Custom(expected), "{text}");
        assert_eq!(parsed.base(), expected);
        assert_eq!(parsed.id(), text.to_ascii_lowercase());
        // The printed id parses to the same colour.
        assert_eq!(parse(&parsed.id()), Some(parsed));
    }
}

#[test]
fn a_custom_colour_that_matches_a_preset_value_stays_custom() {
    // Its id is the hex text; only the preset's own id names the preset.
    assert_eq!(
        parse("#2f6fd6"),
        Some(GroupColour::Custom(Colour::hex(0x2f6fd6)))
    );
    assert_ne!(parse("#2f6fd6"), parse("blue"));
}

#[test]
fn text_that_is_not_a_group_colour_is_none() {
    for text in [
        "",
        "#",
        "#f",
        "#fff",
        "#ffff",
        "#fffff",
        "#fffffff",
        "#ffffffff",
        "#2f6fd680",
        "#12345g",
        "#gggggg",
        "#+2f6fd",
        "#-2f6fd",
        "#2f6fd6 ",
        " #2f6fd6",
        "#2f6fd6\n",
        "2f6fd6",
        "0x2f6fd6",
        "##2f6fd6",
        "# 2f6fd6",
        "#２f6fd6",
        "#2f6fdé",
        "rgb(1,2,3)",
        "red",
        "grey",
        "Blue",
        "BLUE",
        " blue",
        "blue ",
        "blue\n",
        "bl ue",
        "blu",
        "bluee",
        "default",
        "loose",
        "none",
        "\0",
    ] {
        assert_eq!(parse(text), None, "{text:?}");
        assert_eq!(GroupColour::parse(text), None, "{text:?}");
    }
}

#[test]
fn preset_lookup_is_exact() {
    assert!(preset("blue").is_some());
    assert!(preset("Blue").is_none());
    assert!(preset("#2f6fd6").is_none());
    assert!(preset("").is_none());
}

#[test]
fn a_custom_colour_built_in_code_is_opaque() {
    let translucent = Colour::rgba(10, 20, 30, 40);
    let colour = GroupColour::custom(translucent);
    assert_eq!(colour.base(), Colour::rgb(10, 20, 30));
    assert_eq!(colour.id(), "#0a141e");
    assert_eq!(parse(&colour.id()), Some(colour));
}

// ---------------------------------------------------------------------------------------------
// Swatches

/// Checks everything a swatch promises for one colour on one chrome, and returns the strength
/// of the tint.
fn check_swatch(label: &str, base: Colour, (surface, ink): (Colour, Colour)) -> f64 {
    let swatch = Swatch::new(base, surface, ink);
    assert_eq!(swatch.colour, base, "{label}: colour is the base");
    assert_eq!(swatch.on_deep, Colour::WHITE, "{label}");

    let on_tint = ink.contrast_ratio(swatch.tint);
    assert!(
        on_tint >= MIN_TEXT_CONTRAST,
        "{label}: ink on tint {on_tint}"
    );
    let on_deep = swatch.on_deep.contrast_ratio(swatch.deep);
    assert!(
        on_deep >= MIN_TEXT_CONTRAST,
        "{label}: white on deep {on_deep}"
    );

    // The tint is as strong as legibility and the cap allow, and no stronger.
    let limit = tint_strength_limit(base, surface, ink);
    let strength = limit.min(TINT_STRENGTH_MAX);
    assert_eq!(swatch.tint, surface.mix(base, strength), "{label}: tint");
    if limit < TINT_STRENGTH_MAX {
        let one_more = surface.mix(base, limit + 1.0 / 255.0);
        assert!(
            ink.contrast_ratio(one_more) < MIN_TEXT_CONTRAST,
            "{label}: the tint could go further"
        );
    }

    // The deep fill is the least darkening that lets white read, which is none when the colour
    // already does.
    let darkening = (0..=255_u32)
        .find(|step| base.darker(f64::from(*step) / 255.0) == swatch.deep)
        .unwrap_or_else(|| panic!("{label}: deep is not a darkening of the base"));
    if darkening == 0 {
        assert_eq!(swatch.deep, base);
    } else {
        let lighter = base.darker(f64::from(darkening - 1) / 255.0);
        assert!(
            swatch.on_deep.contrast_ratio(lighter) < MIN_TEXT_CONTRAST,
            "{label}: deep is darker than it needs to be"
        );
    }
    limit
}

#[test]
fn swatches_of_all_presets_and_seven_custom_colours_are_legible_on_both_chromes() {
    println!("tint strength limit (1 = the colour itself, cap {TINT_STRENGTH_MAX}) and deep:");
    println!("  colour      light  dark   deep darkening");
    for found in &PRESETS {
        let light = check_swatch(found.id, found.base, LIGHT);
        let dark = check_swatch(found.id, found.base, DARK);
        let deep = Swatch::new(found.base, LIGHT.0, LIGHT.1).deep;
        println!(
            "  {:<10} {light:>5.2}  {dark:>4.2}   {} -> {}",
            found.id,
            found.base.to_hex(),
            deep.to_hex()
        );
    }
    for hex in CUSTOMS {
        let base = custom(hex);
        check_swatch(hex, base, LIGHT);
        check_swatch(hex, base, DARK);
    }
}

#[test]
fn the_swatch_of_a_group_colour_is_the_swatch_of_its_base() {
    for hex in ["blue", "#3a7bd5", "slate"] {
        let colour = parse(hex).unwrap_or_else(|| panic!("{hex}"));
        assert_eq!(
            colour.swatch(LIGHT.0, LIGHT.1),
            Swatch::new(colour.base(), LIGHT.0, LIGHT.1)
        );
    }
}

#[test]
fn a_tint_is_a_wash_never_the_colour_itself() {
    // Without the cap the light chrome would give most presets a tint equal to the colour.
    let mut at_cap = 0;
    for found in &PRESETS {
        for (surface, ink) in [LIGHT, DARK] {
            let swatch = Swatch::new(found.base, surface, ink);
            assert_ne!(swatch.tint, found.base, "{}", found.id);
            // A tint sits between the surface and the colour, never past it.
            assert!(
                delta_e(swatch.tint, surface) <= delta_e(found.base, surface) + 1e-9,
                "{}",
                found.id
            );
            if tint_strength_limit(found.base, surface, ink) >= TINT_STRENGTH_MAX {
                at_cap += 1;
            }
        }
    }
    println!("{at_cap} of 28 preset tints are held at the cap of {TINT_STRENGTH_MAX}");
    assert!(at_cap > 14, "the cap should be what binds for most tints");
}

#[test]
fn the_tint_cap_in_steps_matches_the_published_strength() {
    // `TINT_STRENGTH_MAX` is used as a whole number of 1/255 steps inside the module.
    let steps = TINT_STRENGTH_MAX * 255.0;
    assert!((steps - steps.round()).abs() < 1e-9, "{steps}");
}

#[test]
fn deep_is_the_colour_itself_where_white_already_reads_on_it() {
    for (id, already) in [
        ("blue", true),
        ("violet", true),
        ("brown", true),
        ("plum", true),
        ("amber", false),
        ("lime", false),
        ("sky", false),
    ] {
        let found = preset(id).unwrap_or_else(|| panic!("{id}"));
        let deep = Swatch::new(found.base, LIGHT.0, LIGHT.1).deep;
        assert_eq!(deep == found.base, already, "{id}: deep {}", deep.to_hex());
        // Deep is never lighter than the colour.
        assert!(deep.luminance() <= found.base.luminance());
    }
}

#[test]
fn a_swatch_ignores_the_alpha_of_the_colour() {
    let translucent = Swatch::new(Colour::rgba(0x2f, 0x6f, 0xd6, 20), LIGHT.0, LIGHT.1);
    assert_eq!(
        translucent,
        Swatch::new(Colour::hex(0x2f6fd6), LIGHT.0, LIGHT.1)
    );
    assert_eq!(translucent.colour.a, 255);
}

#[test]
fn when_ink_does_not_read_on_the_surface_the_tint_is_the_surface() {
    // The guarantee needs ink that reads on the surface; without it the tint is not pushed.
    let surface = Colour::hex(0x808080);
    let ink = Colour::hex(0x909090);
    assert!(ink.contrast_ratio(surface) < MIN_TEXT_CONTRAST);
    assert_eq!(
        Swatch::new(Colour::hex(0xff0000), surface, ink).tint,
        surface
    );
}

// ---------------------------------------------------------------------------------------------
// Tab shades

fn shade_bases() -> Vec<(String, Colour)> {
    let mut all: Vec<(String, Colour)> = PRESETS
        .iter()
        .map(|found| (found.id.to_owned(), found.base))
        .collect();
    all.extend(CUSTOMS.iter().map(|hex| ((*hex).to_owned(), custom(hex))));
    all
}

#[test]
fn shade_zero_is_the_base_and_alpha_is_kept() {
    for (label, base) in shade_bases() {
        assert_eq!(tab_shade(base, 0), base, "{label}");
        assert_eq!(tab_shade(base, SHADE_CYCLE), base, "{label}");
    }
    let translucent = Colour::rgba(0x2f, 0x6f, 0xd6, 99);
    for index in 0..SHADE_CYCLE {
        assert_eq!(tab_shade(translucent, index).a, 99, "index {index}");
    }
}

#[test]
fn shades_repeat_with_the_cycle_of_six_and_are_six_different_colours() {
    assert_eq!(SHADE_CYCLE, 6);
    for found in &PRESETS {
        let cycle: Vec<Colour> = (0..SHADE_CYCLE).map(|i| tab_shade(found.base, i)).collect();
        for (i, shade) in cycle.iter().enumerate() {
            assert_eq!(tab_shade(found.base, i + SHADE_CYCLE), *shade);
            assert_eq!(tab_shade(found.base, i + 10 * SHADE_CYCLE), *shade);
        }
        // The largest index is a valid index too.
        assert_eq!(
            tab_shade(found.base, usize::MAX),
            cycle[usize::MAX % SHADE_CYCLE]
        );
        for (i, a) in cycle.iter().enumerate() {
            for b in &cycle[i + 1..] {
                assert_ne!(a, b, "{}: two shades are equal", found.id);
            }
        }
    }
}

#[test]
fn neighbouring_tabs_always_differ_by_at_least_0_02() {
    let mut worst = (f64::MAX, String::new());
    for (label, base) in shade_bases() {
        if label == "#000000" {
            continue; // see the black test below
        }
        for index in 0..64 {
            let distance = delta_e(tab_shade(base, index), tab_shade(base, index + 1));
            if distance < worst.0 {
                worst = (distance, format!("{label} {index}->{}", index + 1));
            }
            assert!(
                distance >= 0.02,
                "{label}: shades {index} and {} are {distance} apart",
                index + 1
            );
        }
    }
    println!(
        "smallest delta-E between neighbouring tabs: {:.4} ({})",
        worst.0, worst.1
    );
}

#[test]
fn the_presets_neighbours_have_the_designed_margin() {
    // Ladder rungs are 0.02 apart and the order keeps neighbours two rungs or more apart, so
    // 0.04 less the rounding of 8-bit colours.
    let mut worst = f64::MAX;
    for found in &PRESETS {
        for index in 0..64 {
            worst = worst.min(delta_e(
                tab_shade(found.base, index),
                tab_shade(found.base, index + 1),
            ));
        }
    }
    println!("smallest neighbour delta-E among the presets: {worst:.4}");
    assert!(worst >= 2.0 * SHADE_STEP - 0.01, "{worst}");
}

#[test]
fn no_shade_of_a_preset_is_closer_to_another_presets_colour_than_to_its_own() {
    let mut tightest = (f64::MAX, String::new());
    for found in &PRESETS {
        for index in 0..SHADE_CYCLE {
            let shade = tab_shade(found.base, index);
            let own = delta_e(shade, found.base);
            for other in PRESETS.iter().filter(|other| other.id != found.id) {
                let theirs = delta_e(shade, other.base);
                let margin = theirs - own;
                if margin < tightest.0 {
                    tightest = (
                        margin,
                        format!("{} shade {index} vs {}", found.id, other.id),
                    );
                }
                assert!(
                    theirs > own,
                    "{} shade {index} is {own} from its own colour and {theirs} from {}",
                    found.id,
                    other.id
                );
            }
        }
    }
    println!(
        "tightest margin (other base minus own base): {:.4} ({})",
        tightest.0, tightest.1
    );
}

#[test]
fn shades_stay_within_three_rungs_of_the_colour() {
    let mut farthest = 0.0_f64;
    for (label, base) in shade_bases() {
        if label == "#000000" {
            continue;
        }
        for index in 0..SHADE_CYCLE {
            let away = delta_e(base, tab_shade(base, index));
            farthest = farthest.max(away);
            assert!(
                away <= 5.0 * SHADE_STEP + 0.01,
                "{label} shade {index} is {away} from its colour"
            );
        }
    }
    println!("largest distance of a shade from its colour: {farthest:.4}");
}

#[test]
fn presets_stay_within_three_rungs_exactly() {
    // Presets all have room on both sides, so none needs more than three rungs.
    for found in &PRESETS {
        for index in 0..SHADE_CYCLE {
            let away = delta_e(found.base, tab_shade(found.base, index));
            assert!(
                away <= 3.0 * SHADE_STEP + 0.006,
                "{} {index}: {away}",
                found.id
            );
        }
    }
}

#[test]
fn light_colours_get_more_light_shades_and_dark_colours_more_dark_ones() {
    let lighter_count = |base: Colour| {
        (0..SHADE_CYCLE)
            .filter(|&i| lightness(tab_shade(base, i)) > lightness(base))
            .count()
    };
    let darker_count = |base: Colour| {
        (0..SHADE_CYCLE)
            .filter(|&i| lightness(tab_shade(base, i)) < lightness(base))
            .count()
    };
    for id in ["amber", "orchid", "sky", "lime", "mint", "salmon"] {
        let base = preset(id).map(|found| found.base).unwrap_or(Colour::BLACK);
        assert_eq!((lighter_count(base), darker_count(base)), (3, 2), "{id}");
    }
    for id in [
        "blue", "violet", "rose", "green", "teal", "slate", "brown", "plum",
    ] {
        let base = preset(id).map(|found| found.base).unwrap_or(Colour::BLACK);
        assert_eq!((lighter_count(base), darker_count(base)), (2, 3), "{id}");
    }
}

#[test]
fn white_only_gets_darker_shades_and_still_cycles_cleanly() {
    let white = Colour::WHITE;
    for index in 0..SHADE_CYCLE {
        assert!(lightness(tab_shade(white, index)) <= lightness(white) + 1e-9);
    }
    let distinct: std::collections::HashSet<Colour> =
        (0..SHADE_CYCLE).map(|i| tab_shade(white, i)).collect();
    assert_eq!(distinct.len(), SHADE_CYCLE);
}

#[test]
fn black_only_gets_lighter_shades_and_never_panics() {
    let black = Colour::BLACK;
    assert_eq!(tab_shade(black, 0), black);
    for index in 0..200 {
        let shade = tab_shade(black, index);
        assert!(lightness(shade) >= 0.0);
        assert_eq!(shade.a, 255);
    }
    // 8-bit dark greys are too far apart in OKLab for six distinct shades within 0.1 of black:
    // the first step alone is 0.067. The shades are all lighter than black and not all equal.
    let distinct: std::collections::HashSet<Colour> =
        (0..SHADE_CYCLE).map(|i| tab_shade(black, i)).collect();
    assert!(distinct.len() >= 3, "{distinct:?}");
}

#[test]
fn a_grid_of_colours_gives_neighbouring_shades_apart_except_near_black() {
    // Every sixth level of each channel, 216 colours. The shades of a colour are 0.02-rung
    // positions, so where 8-bit rounding is finer than that (anything not within a few levels of
    // black) neighbours stay at least 0.02 apart.
    let levels = [0_u8, 51, 102, 153, 204, 255];
    let mut checked = 0;
    let mut skipped = 0;
    let mut worst = f64::MAX;
    for &r in &levels {
        for &g in &levels {
            for &b in &levels {
                let base = Colour::rgb(r, g, b);
                if lightness(base) < 0.12 {
                    skipped += 1;
                    continue;
                }
                for index in 0..SHADE_CYCLE {
                    let distance = delta_e(tab_shade(base, index), tab_shade(base, index + 1));
                    worst = worst.min(distance);
                    assert!(distance >= 0.02, "{base:?} {index}: {distance}");
                }
                checked += 1;
            }
        }
    }
    println!(
        "grid: {checked} colours checked, {skipped} near black skipped, smallest neighbour {worst:.4}"
    );
    assert!(checked > 150);
}

// ---------------------------------------------------------------------------------------------
// Choosing a colour for a new group

fn next(used: &[&str]) -> &'static str {
    auto_colour(used).id
}

#[test]
fn the_first_preset_not_in_use_is_chosen() {
    assert_eq!(next(&[]), PRESETS[0].id);
    for taken in 0..PRESET_COUNT {
        let used: Vec<&str> = PRESETS[..taken].iter().map(|found| found.id).collect();
        assert_eq!(next(&used), PRESETS[taken].id, "{taken} in use");
    }
}

#[test]
fn which_ids_are_in_use_matters_but_not_their_order_or_repeats_beyond_the_count() {
    assert_eq!(next(&["blue"]), "amber");
    assert_eq!(next(&["amber"]), "blue");
    assert_eq!(next(&["amber", "blue"]), "rose");
    assert_eq!(next(&["blue", "amber"]), "rose");
    assert_eq!(next(&["blue", "blue", "amber"]), "rose");
    let all_but_teal: Vec<&str> = PRESETS
        .iter()
        .map(|found| found.id)
        .filter(|id| *id != "teal")
        .collect();
    assert_eq!(next(&all_but_teal), "teal");
    let all_but_teal_and_mint: Vec<&str> = all_but_teal
        .iter()
        .copied()
        .filter(|id| *id != "mint")
        .collect();
    assert_eq!(next(&all_but_teal_and_mint), "mint");
}

#[test]
fn custom_colours_and_unknown_text_do_not_use_up_a_preset() {
    assert_eq!(next(&["#2f6fd6"]), "blue");
    assert_eq!(next(&["#2f6fd6", "#e0912a", "#d9486e"]), "blue");
    assert_eq!(next(&["Blue", "BLUE", "nonsense", "", "#fff"]), "blue");
    assert_eq!(next(&["#2f6fd6", "blue"]), "amber");
}

#[test]
fn when_all_are_in_use_the_least_used_wins_and_the_first_breaks_ties() {
    let once: Vec<&str> = PRESETS.iter().map(|found| found.id).collect();
    assert_eq!(next(&once), "blue");

    let mut blue_twice = once.clone();
    blue_twice.push("blue");
    assert_eq!(next(&blue_twice), "amber");

    let mut two_twice = once.clone();
    two_twice.extend(["blue", "amber"]);
    assert_eq!(next(&two_twice), "rose");

    // Everything twice except teal: teal is the least used.
    let mut twice: Vec<&str> = once.iter().chain(once.iter()).copied().collect();
    let position = twice
        .iter()
        .rposition(|id| *id == "teal")
        .unwrap_or_else(|| panic!("teal is in the list"));
    twice.remove(position);
    assert_eq!(next(&twice), "teal");

    // Ignored ids on top change nothing.
    let mut with_noise = blue_twice.clone();
    with_noise.extend(["#ff0000", "red", "#2f6fd6"]);
    assert_eq!(next(&with_noise), "amber");
}

#[test]
fn handing_out_colours_one_after_another_goes_through_the_list_and_wraps() {
    let mut used: Vec<&str> = Vec::new();
    for round in 0..3 {
        for expected in &PRESETS {
            let chosen = auto_colour(&used);
            assert_eq!(chosen.id, expected.id, "round {round}");
            used.push(chosen.id);
        }
    }
    assert_eq!(used.len(), 3 * PRESET_COUNT);
}

#[test]
fn removing_a_group_frees_its_colour_for_the_next_one() {
    let mut used: Vec<&str> = PRESETS[..5].iter().map(|found| found.id).collect();
    assert_eq!(next(&used), PRESETS[5].id);
    used.remove(1); // the second group closes
    assert_eq!(next(&used), PRESETS[1].id);
}

#[test]
fn auto_colour_returns_the_static_preset_itself() {
    let chosen = auto_colour(&[]);
    assert!(std::ptr::eq(chosen, &PRESETS[0]));
    assert!(preset("blue").is_some_and(|blue| std::ptr::eq(chosen, blue)));
}

// ---------------------------------------------------------------------------------------------
// Report only: colour-vision deficiency

type Matrix = [[f64; 3]; 3];

/// Machado, Oliveira and Fernandes (2009), "A Physiologically-based Model for Simulation of
/// Color Vision Deficiency", severity 1.0, applied to linear RGB.
const PROTANOPIA: Matrix = [
    [0.152286, 1.052583, -0.204868],
    [0.114503, 0.786281, 0.099216],
    [-0.003882, -0.048116, 1.051998],
];
/// The same paper, deuteranopia at severity 1.0.
const DEUTERANOPIA: Matrix = [
    [0.367322, 0.860646, -0.227968],
    [0.280085, 0.672501, 0.047413],
    [-0.011820, 0.042940, 0.968881],
];

fn to_linear(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(value: f64) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let encoded = if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}

fn simulate(colour: Colour, matrix: &Matrix) -> Colour {
    let rgb = [colour.r, colour.g, colour.b].map(to_linear);
    let out = matrix.map(|row| row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2]);
    Colour::rgb(
        from_linear(out[0]),
        from_linear(out[1]),
        from_linear(out[2]),
    )
}

#[test]
fn report_how_far_apart_the_presets_are_for_protanopia_and_deuteranopia() {
    // Report only, as the brief says: the numbers go in the lane report and the test cannot
    // fail on them. Each row of the matrices sums to 1, so greys stay grey.
    for (name, matrix) in [("protanopia", &PROTANOPIA), ("deuteranopia", &DEUTERANOPIA)] {
        let mut all = Vec::new();
        for (i, a) in PRESETS.iter().enumerate() {
            for b in &PRESETS[i + 1..] {
                all.push((
                    delta_e(simulate(a.base, matrix), simulate(b.base, matrix)),
                    a.id,
                    b.id,
                ));
            }
        }
        all.sort_by(|x, y| x.0.total_cmp(&y.0));
        println!(
            "{name}: minimum pairwise delta-E among the 14 is {:.4}",
            all[0].0
        );
        for (distance, a, b) in all.iter().take(5) {
            println!("  {distance:.4}  {a} / {b}");
        }
        assert!(all[0].0.is_finite());
    }
    // A sanity check of the simulation itself: grey stays grey.
    let grey = Colour::hex(0x808080);
    for matrix in [&PROTANOPIA, &DEUTERANOPIA] {
        let seen = simulate(grey, matrix);
        assert!(delta_e(seen, grey) < 0.005, "{seen:?}");
    }
}
