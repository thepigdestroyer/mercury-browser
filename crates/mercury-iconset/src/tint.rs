// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Recolours the app icon artwork (`AppIcon.svg`) for a palette, so the Dock icon can follow the
//! theme while the app runs. The colours come from `mercury_appearance::AppIconColours`; the
//! artwork is the one the bundle's `AppIcon.icns` is made from, and its brand colours are the
//! literals below, so editing the artwork's colours without editing this list is an error
//! (`Error::Template`), caught by the tests, never a half-tinted icon.

use std::collections::HashMap;

use mercury_appearance::AppIconColours;

use crate::Error;

/// The size the Dock icon is rendered at, in pixels.
pub const APP_ICON_PIXELS: u32 = 1024;

/// The four stops of the sun in the artwork, pale core to rim.
const SUN: [&str; 4] = ["#FFF3C4", "#FFC857", "#FF8A2F", "#EC5B27"];
/// The halo around the sun.
const HALO: &str = "#FF9A3C";
/// The top and the bottom of the plate.
const PLATE: [&str; 2] = ["#2A1F4F", "#0C0819"];
/// The M.
const INK: &str = "#1E1233";

/// `svg` (the text of `AppIcon.svg`) with every brand colour replaced by the palette's.
///
/// All other colours (the sheen, the plate's rim light) are white at low opacity and stay as they
/// are. Fails when the artwork does not contain one of the brand colours.
pub fn tint_app_icon(svg: &str, colours: &AppIconColours) -> Result<String, Error> {
    let mut replacements: HashMap<String, String> = HashMap::new();
    for (old, new) in SUN.iter().zip(colours.sun) {
        replacements.insert(old.to_ascii_lowercase(), new.to_hex());
    }
    replacements.insert(HALO.to_ascii_lowercase(), colours.halo.to_hex());
    replacements.insert(PLATE[0].to_ascii_lowercase(), colours.plate_top.to_hex());
    replacements.insert(PLATE[1].to_ascii_lowercase(), colours.plate_bottom.to_hex());
    replacements.insert(INK.to_ascii_lowercase(), colours.ink.to_hex());

    // One pass over the text, so a new colour that happens to equal an old literal is never
    // replaced a second time.
    let mut out = String::with_capacity(svg.len());
    let mut seen: Vec<String> = Vec::new();
    let mut rest = svg;
    while let Some(at) = rest.find('#') {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        let literal = after.get(..7).filter(|text| {
            text.as_bytes()[1..]
                .iter()
                .all(|byte| byte.is_ascii_hexdigit())
        });
        match literal.and_then(|text| {
            replacements
                .get(&text.to_ascii_lowercase())
                .map(|n| (text, n))
        }) {
            Some((text, new)) => {
                out.push_str(new);
                seen.push(text.to_ascii_lowercase());
                rest = &after[7..];
            }
            None => {
                out.push('#');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);

    for wanted in replacements.keys() {
        if !seen.contains(wanted) {
            return Err(Error::Template(format!("colour {wanted}")));
        }
    }
    Ok(out)
}
