// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! OKLab conversions, the helpers and the colour difference.

use mercury_appearance::Colour;
use mercury_appearance::oklab::{Oklab, chroma, delta_e, hue, lightness};

fn near(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: {actual} is not within {tolerance} of {expected}"
    );
}

#[test]
fn white_is_lightness_one_and_colourless() {
    let white = Oklab::from_colour(Colour::WHITE);
    near(white.l, 1.0, 1e-6, "white L");
    near(white.a, 0.0, 1e-6, "white a");
    near(white.b, 0.0, 1e-6, "white b");
}

#[test]
fn black_is_lightness_zero_and_colourless() {
    let black = Oklab::from_colour(Colour::BLACK);
    near(black.l, 0.0, 1e-12, "black L");
    near(black.a, 0.0, 1e-12, "black a");
    near(black.b, 0.0, 1e-12, "black b");
}

#[test]
fn srgb_red_green_and_blue_match_the_published_values() {
    // sRGB red in OKLab is (0.628, 0.225, 0.126), the example in Ottosson's post. The other two
    // are the CSS Color 4 OKLCH values for `lime` and `blue`.
    let red = Oklab::from_colour(Colour::rgb(255, 0, 0));
    near(red.l, 0.628, 0.0005, "red L");
    near(red.a, 0.225, 0.0005, "red a");
    near(red.b, 0.126, 0.0005, "red b");

    let green = Oklab::from_colour(Colour::rgb(0, 255, 0));
    near(green.l, 0.8664, 0.0005, "green L");
    near(green.chroma(), 0.2948, 0.0005, "green C");
    near(green.hue(), 142.5, 0.1, "green hue");

    let blue = Oklab::from_colour(Colour::rgb(0, 0, 255));
    near(blue.l, 0.452, 0.0005, "blue L");
    near(blue.chroma(), 0.3132, 0.0005, "blue C");
    near(blue.hue(), 264.05, 0.1, "blue hue");
}

#[test]
fn a_mid_grey_is_neutral_and_between_black_and_white() {
    let grey = Oklab::from_colour(Colour::rgb(128, 128, 128));
    near(grey.a, 0.0, 1e-6, "grey a");
    near(grey.b, 0.0, 1e-6, "grey b");
    assert!(grey.l > 0.5 && grey.l < 0.7, "{}", grey.l);
}

#[test]
fn every_colour_on_a_lattice_survives_the_round_trip() {
    // 8 bits per channel is the resolution that matters: the brief asks for one 8-bit step, and
    // the maths is exact enough to return the same colour.
    let mut checked = 0_u32;
    let mut worst = 0_i16;
    let levels: Vec<u8> = (0..=255).step_by(3).chain([254, 255]).collect();
    for &r in &levels {
        for &g in &levels {
            for &b in &levels {
                let start = Colour::rgb(r, g, b);
                let back = Oklab::from_colour(start).to_colour();
                let off = [(back.r, r), (back.g, g), (back.b, b)]
                    .iter()
                    .map(|&(x, y)| (i16::from(x) - i16::from(y)).abs())
                    .max()
                    .unwrap_or(0);
                worst = worst.max(off);
                assert!(off <= 1, "{start:?} came back as {back:?}");
                assert_eq!(back.a, 255);
                checked += 1;
            }
        }
    }
    println!("round trip: {checked} colours, worst channel error {worst}");
}

#[test]
fn every_grey_survives_the_round_trip_exactly() {
    for level in 0..=255_u8 {
        let grey = Colour::rgb(level, level, level);
        assert_eq!(Oklab::from_colour(grey).to_colour(), grey, "grey {level}");
    }
}

#[test]
fn the_from_impls_agree_with_the_methods() {
    let colour = Colour::hex(0x2f6fd6);
    let lab: Oklab = colour.into();
    assert_eq!(lab, Oklab::from_colour(colour));
    let back: Colour = lab.into();
    assert_eq!(back, lab.to_colour());
    assert_eq!(back, colour);
}

#[test]
fn alpha_is_ignored_going_in_and_opaque_coming_out() {
    let solid = Colour::rgb(200, 100, 50);
    let faint = Colour::rgba(200, 100, 50, 10);
    assert_eq!(Oklab::from_colour(solid), Oklab::from_colour(faint));
    assert_eq!(delta_e(solid, faint), 0.0);
    assert_eq!(Oklab::from_colour(faint).to_colour().a, 255);
}

#[test]
fn a_colour_outside_the_gamut_is_clamped_not_garbage() {
    // Far more colourful than sRGB can show; the result must still be a valid colour.
    let wild = Oklab {
        l: 0.7,
        a: 0.6,
        b: -0.4,
    };
    let colour = wild.to_colour();
    assert_eq!(colour.a, 255);
    // Lightness out of range clamps to the ends.
    assert_eq!(
        Oklab {
            l: 2.0,
            a: 0.0,
            b: 0.0
        }
        .to_colour(),
        Colour::WHITE
    );
    assert_eq!(
        Oklab {
            l: -1.0,
            a: 0.0,
            b: 0.0
        }
        .to_colour(),
        Colour::BLACK
    );
    // Not-a-number must not panic; what it maps to is unspecified.
    let _ = Oklab {
        l: f64::NAN,
        a: 0.0,
        b: 0.0,
    }
    .to_colour();
}

#[test]
fn delta_e_is_zero_for_the_same_colour_and_symmetric() {
    let colours = [
        Colour::hex(0x2f6fd6),
        Colour::hex(0xe0912a),
        Colour::hex(0x7c8088),
        Colour::WHITE,
        Colour::BLACK,
    ];
    for &a in &colours {
        assert_eq!(delta_e(a, a), 0.0);
        for &b in &colours {
            assert_eq!(delta_e(a, b), delta_e(b, a));
        }
    }
}

#[test]
fn delta_e_between_black_and_white_is_one() {
    near(
        delta_e(Colour::BLACK, Colour::WHITE),
        1.0,
        1e-6,
        "black to white",
    );
}

#[test]
fn delta_e_obeys_the_triangle_inequality_on_a_sample() {
    let sample = [
        Colour::hex(0x2f6fd6),
        Colour::hex(0xe0912a),
        Colour::hex(0x7e5bd0),
        Colour::hex(0xd9486e),
        Colour::hex(0x2e9e62),
        Colour::WHITE,
        Colour::BLACK,
    ];
    for &a in &sample {
        for &b in &sample {
            for &c in &sample {
                assert!(delta_e(a, c) <= delta_e(a, b) + delta_e(b, c) + 1e-12);
            }
        }
    }
}

#[test]
fn delta_e_is_the_euclidean_distance_of_the_coordinates() {
    let (a, b) = (Colour::hex(0x2f6fd6), Colour::hex(0xe0912a));
    let (p, q) = (Oklab::from_colour(a), Oklab::from_colour(b));
    let by_hand = ((p.l - q.l).powi(2) + (p.a - q.a).powi(2) + (p.b - q.b).powi(2)).sqrt();
    near(delta_e(a, b), by_hand, 1e-12, "delta_e");
    near(p.distance(q), by_hand, 1e-12, "distance");
}

#[test]
fn the_helpers_match_the_methods() {
    for colour in [
        Colour::hex(0x2f6fd6),
        Colour::hex(0xe0912a),
        Colour::hex(0x7c8088),
    ] {
        let lab = Oklab::from_colour(colour);
        assert_eq!(lightness(colour), lab.l);
        assert_eq!(chroma(colour), lab.chroma());
        assert_eq!(hue(colour), lab.hue());
    }
}

#[test]
fn greys_have_no_chroma_and_hue_zero() {
    for level in [0_u8, 1, 64, 128, 200, 255] {
        let grey = Colour::rgb(level, level, level);
        assert!(chroma(grey) < 1e-6, "{level}: {}", chroma(grey));
        assert_eq!(hue(grey), 0.0, "{level}");
    }
}

#[test]
fn hue_is_in_zero_to_360_and_orders_the_primaries() {
    for colour in [
        Colour::rgb(255, 0, 0),
        Colour::rgb(255, 255, 0),
        Colour::rgb(0, 255, 0),
        Colour::rgb(0, 255, 255),
        Colour::rgb(0, 0, 255),
        Colour::rgb(255, 0, 255),
        Colour::hex(0xd9486e),
    ] {
        let angle = hue(colour);
        assert!((0.0..360.0).contains(&angle), "{colour:?}: {angle}");
    }
    // Around the wheel: red, yellow, green, cyan, blue, magenta.
    let wheel: Vec<f64> = [
        (255, 0, 0),
        (255, 255, 0),
        (0, 255, 0),
        (0, 255, 255),
        (0, 0, 255),
        (255, 0, 255),
    ]
    .iter()
    .map(|&(r, g, b)| hue(Colour::rgb(r, g, b)))
    .collect();
    assert!(wheel.windows(2).all(|pair| pair[0] < pair[1]), "{wheel:?}");
}

#[test]
fn chroma_grows_with_saturation_at_one_lightness_and_hue() {
    let pale = chroma(Colour::hex(0xb8c4e8));
    let bright = chroma(Colour::hex(0x2f6fd6));
    assert!(bright > pale, "{bright} {pale}");
    assert!(bright < 0.4);
}

#[test]
fn lightness_orders_the_chrome_surfaces() {
    // Paper is lighter than its second surface, which is lighter than the night plate.
    assert!(lightness(Colour::hex(0xf8f4ee)) > lightness(Colour::hex(0xefe8df)));
    assert!(lightness(Colour::hex(0xefe8df)) > lightness(Colour::hex(0x1c1534)));
    assert!(lightness(Colour::hex(0x1c1534)) > lightness(Colour::hex(0x140f27)));
}
