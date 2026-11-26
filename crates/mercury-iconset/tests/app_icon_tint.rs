// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Dock icon follows the theme: the app icon artwork recoloured for a palette and rasterised at
//! 1024 px. Checks that every brand colour is replaced, that the picture is square and 1024 px, and
//! that the colours of the palette are really in the pixels (the M, the sun and the plate).

use mercury_appearance::palette::Scheme;
use mercury_appearance::{AppIconColours, Colour, palettes};
use mercury_iconset::{APP_ICON_PIXELS, Error, render_png, tint_app_icon};
use resvg::tiny_skia::Pixmap;

fn artwork() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/mercury-desktop/resources/macos/AppIcon.svg");
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn pixel(pixmap: &Pixmap, x: u32, y: u32) -> Colour {
    let p = pixmap.pixel(x, y).expect("inside").demultiply();
    Colour::rgba(p.red(), p.green(), p.blue(), p.alpha())
}

fn near(actual: Colour, expected: Colour, tolerance: u8) -> bool {
    actual.a == 255
        && actual.r.abs_diff(expected.r) <= tolerance
        && actual.g.abs_diff(expected.g) <= tolerance
        && actual.b.abs_diff(expected.b) <= tolerance
}

const ORIGINAL: [&str; 8] = [
    "#fff3c4", "#ffc857", "#ff8a2f", "#ec5b27", "#ff9a3c", "#2a1f4f", "#0c0819", "#1e1233",
];

#[test]
fn every_brand_colour_of_the_artwork_is_replaced_for_every_selectable_palette() {
    let svg = artwork();
    for palette in palettes::selectable() {
        for scheme in Scheme::ALL {
            let colours = AppIconColours::new(palette, scheme);
            let tinted = tint_app_icon(&svg, &colours)
                .unwrap_or_else(|error| panic!("{} {}: {error}", palette.id, scheme.name()));
            let lower = tinted.to_ascii_lowercase();
            let drawn: Vec<String> = colours
                .sun
                .iter()
                .chain([&colours.halo, &colours.plate_bottom, &colours.ink])
                .map(|colour| colour.to_hex())
                .collect();
            for old in ORIGINAL {
                if drawn.iter().any(|hex| hex == old) {
                    continue;
                }
                assert!(
                    !lower.contains(old),
                    "{} {}: {old} is left",
                    palette.id,
                    scheme.name()
                );
            }
            assert!(lower.contains(&colours.sun[2].to_hex()));
            assert!(lower.contains(&colours.ink.to_hex()));
            assert!(lower.contains(&colours.plate_bottom.to_hex()));
            assert!(lower.contains(&colours.halo.to_hex()));
        }
    }
}

#[test]
fn the_rasterised_icon_is_1024_square_and_carries_the_palettes_colours() {
    let svg = artwork();
    for palette in palettes::selectable() {
        for scheme in Scheme::ALL {
            let name = format!("{} {}", palette.id, scheme.name());
            let colours = AppIconColours::new(palette, scheme);
            let tinted = tint_app_icon(&svg, &colours).expect("tints");
            let png = render_png(tinted.as_bytes(), APP_ICON_PIXELS).expect("renders");
            // `MERCURY_ICON_DIR=<dir>` saves the picture, for looking at.
            if let Some(directory) = std::env::var_os("MERCURY_ICON_DIR") {
                let file = format!("dock-icon-{}-{}.png", palette.id, scheme.name());
                std::fs::write(std::path::Path::new(&directory).join(file), &png)
                    .expect("the icon can be saved");
            }
            let pixmap = Pixmap::decode_png(&png).expect("a valid PNG");
            assert_eq!((pixmap.width(), pixmap.height()), (1024, 1024), "{name}");

            // Outside the plate (the 100 px margin) it is transparent.
            assert_eq!(pixel(&pixmap, 20, 20).a, 0, "{name}: the margin");
            // The left leg of the M, mid-stroke (x 405.4, y 500).
            assert_eq!(pixel(&pixmap, 405, 500), colours.ink, "{name}: the M");
            // The sun, to the right of the M: between its second and third stops.
            let sun = pixel(&pixmap, 662, 512);
            let blend = colours.sun[1].mix(colours.sun[2], 0.4875);
            assert!(
                near(sun, blend, 6),
                "{name}: the sun is {sun:?}, expected {blend:?}"
            );
            // The plate low on the icon (y 800 of the 100..924 gradient), left of the sun.
            let plate = pixel(&pixmap, 200, 800);
            let t = (800.0 - 100.0) / (924.0 - 100.0);
            let expected = colours.plate_top.mix(colours.plate_bottom, t);
            assert!(
                near(plate, expected, 6),
                "{name}: the plate is {plate:?}, expected {expected:?}"
            );
            // None of the artwork's orange is left in the sun.
            assert!(
                !near(sun, Colour::hex(0xff8a2f), 20),
                "{name}: the sun is still orange"
            );
        }
    }
}

#[test]
fn the_derived_colours_read_on_each_other() {
    for palette in palettes::selectable() {
        for scheme in Scheme::ALL {
            let colours = AppIconColours::new(palette, scheme);
            assert!(
                colours.ink_contrast() >= mercury_appearance::app_icon::INK_ON_SUN_MINIMUM,
                "{} {}: the M on the sun is {:.2}",
                palette.id,
                scheme.name(),
                colours.ink_contrast()
            );
            assert!(
                colours.sun_on_plate_contrast()
                    >= mercury_appearance::app_icon::SUN_ON_PLATE_MINIMUM,
                "{} {}: the sun on the plate is {:.2}",
                palette.id,
                scheme.name(),
                colours.sun_on_plate_contrast()
            );
        }
    }
}

#[test]
fn different_palettes_give_different_suns_and_the_scheme_is_followed() {
    let signal = palettes::find("signal").expect("signal");
    let forest = palettes::find("forest").expect("forest");
    assert_ne!(
        AppIconColours::new(signal, Scheme::Dark).sun,
        AppIconColours::new(forest, Scheme::Dark).sun
    );
    // Signal's light fill accent is bright enough on its black plate, so the scheme shows.
    assert_ne!(
        AppIconColours::new(signal, Scheme::Light).sun,
        AppIconColours::new(signal, Scheme::Dark).sun
    );
}

#[test]
fn artwork_that_lost_a_brand_colour_is_an_error_not_a_half_tinted_icon() {
    let svg = artwork().replace("#1E1233", "#000000");
    let colours = AppIconColours::new(palettes::default_palette(), Scheme::Dark);
    assert!(matches!(
        tint_app_icon(&svg, &colours),
        Err(Error::Template(_))
    ));
}
