// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Colours: the hex forms Slint takes, compositing, mixing, and the WCAG 2.2 relative luminance
//! and contrast ratio.
//!
//! This is the one place the rest of the crate gets its colour arithmetic from. It works in 8-bit
//! sRGB. Contrast is defined for opaque colours only: put a translucent colour over its
//! background with [`Colour::over`] first.

/// An sRGB colour with alpha, 8 bits per channel (255 is opaque).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Colour {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha; 255 is opaque.
    pub a: u8,
}

impl Colour {
    /// Opaque black.
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    /// Opaque white.
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// A colour with alpha.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// An opaque colour from `0xRRGGBB`, for tables of literals: `Colour::hex(0xf8f4ee)`.
    pub const fn hex(rgb: u32) -> Self {
        Self::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    }

    /// A colour from `0xRRGGBBAA`.
    pub const fn hex_alpha(rgba: u32) -> Self {
        Self::rgba(
            (rgba >> 24) as u8,
            (rgba >> 16) as u8,
            (rgba >> 8) as u8,
            rgba as u8,
        )
    }

    /// Parses `#rgb`, `#rrggbb` or `#rrggbbaa` (any letter case), the forms Slint accepts.
    /// Anything else, including a missing `#`, is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(digits, 16).ok()?;
        match digits.len() {
            3 => {
                let [r, g, b] = [(value >> 8) & 0xf, (value >> 4) & 0xf, value & 0xf]
                    .map(|nibble| (nibble * 17) as u8);
                Some(Self::rgb(r, g, b))
            }
            6 => Some(Self::hex(value)),
            8 => Some(Self::hex_alpha(value)),
            _ => None,
        }
    }

    /// The Slint literal for this colour: `#rrggbb` when opaque, else `#rrggbbaa`, lower case.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// The WCAG 2.2 relative luminance (0.0 black to 1.0 white); alpha is ignored.
    pub fn luminance(self) -> f64 {
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }

    /// The WCAG 2.2 contrast ratio between two colours, from 1.0 to 21.0, in either order.
    /// Alpha is ignored: composite with [`over`](Self::over) first.
    pub fn contrast_ratio(self, other: Self) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// This colour drawn over `background` (source-over, straight alpha). Over an opaque
    /// background the result is opaque.
    pub fn over(self, background: Self) -> Self {
        let (fa, ba) = (f64::from(self.a) / 255.0, f64::from(background.a) / 255.0);
        let out_a = fa + ba * (1.0 - fa);
        if out_a <= 0.0 {
            return Self::rgba(0, 0, 0, 0);
        }
        let blend = |front: u8, back: u8| {
            let value = (f64::from(front) * fa + f64::from(back) * ba * (1.0 - fa)) / out_a;
            value.round().clamp(0.0, 255.0) as u8
        };
        Self::rgba(
            blend(self.r, background.r),
            blend(self.g, background.g),
            blend(self.b, background.b),
            (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
        )
    }

    /// The straight sRGB interpolation of the colour channels from `self` (`t` = 0) to `other`
    /// (`t` = 1), the way design tools mix. `t` is clamped to 0..=1; the alpha of `self` is kept.
    pub fn mix(self, other: Self, t: f64) -> Self {
        let t = t.clamp(0.0, 1.0);
        let lerp = |from: u8, to: u8| {
            (f64::from(from) + (f64::from(to) - f64::from(from)) * t)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Self::rgba(
            lerp(self.r, other.r),
            lerp(self.g, other.g),
            lerp(self.b, other.b),
            self.a,
        )
    }

    /// `self` mixed `t` of the way toward black.
    pub fn darker(self, t: f64) -> Self {
        self.mix(Self::BLACK, t)
    }

    /// `self` mixed `t` of the way toward white.
    pub fn lighter(self, t: f64) -> Self {
        self.mix(Self::WHITE, t)
    }
}

/// One sRGB channel as linear light (WCAG 2.2 relative luminance, with its 0.04045 threshold).
fn linear(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_forms_parse_and_print() {
        assert_eq!(
            Colour::parse("#f8f4ee"),
            Some(Colour::rgb(0xf8, 0xf4, 0xee))
        );
        assert_eq!(Colour::parse("#F8F4EE"), Some(Colour::hex(0xf8f4ee)));
        assert_eq!(Colour::parse("#fa0"), Some(Colour::rgb(0xff, 0xaa, 0x00)));
        assert_eq!(
            Colour::parse("#00000099"),
            Some(Colour::rgba(0, 0, 0, 0x99))
        );
        assert_eq!(Colour::hex(0xc2491a).to_hex(), "#c2491a");
        assert_eq!(Colour::rgba(0, 0, 0, 0x4d).to_hex(), "#0000004d");
    }

    #[test]
    fn text_that_is_not_a_slint_colour_is_refused() {
        for text in [
            "",
            "#",
            "f8f4ee",
            "#ff",
            "#fffff",
            "#fffffff",
            "#ggg",
            "#12345g",
            "red",
            "#+12345",
            " #fff",
            "#fff ",
            "#ffffffffff",
        ] {
            assert_eq!(Colour::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn black_and_white_are_21_to_1() {
        assert!((Colour::BLACK.contrast_ratio(Colour::WHITE) - 21.0).abs() < 1e-9);
        assert!((Colour::WHITE.contrast_ratio(Colour::BLACK) - 21.0).abs() < 1e-9);
        assert!((Colour::WHITE.contrast_ratio(Colour::WHITE) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_spec_accent_on_paper_is_just_under_4_5() {
        // SPEC §1 names #C2491A on #F8F4EE as the pair that must pass 4.5:1 as text. Measured
        // with the WCAG formula it is 4.497: it does not. The palette work keeps #C2491A for
        // fills (white text on it is 4.93) and marks (they need 3:1), and gives accent *text* a
        // slightly deeper tone. This test records the number so nobody rediscovers it.
        let accent = Colour::hex(0xc2491a);
        let paper = Colour::hex(0xf8f4ee);
        let ratio = accent.contrast_ratio(paper);
        assert!((4.49..4.50).contains(&ratio), "{ratio}");
        let on_accent = Colour::WHITE.contrast_ratio(accent);
        assert!(on_accent >= 4.5, "{on_accent}");
    }

    #[test]
    fn an_opaque_colour_over_anything_is_itself() {
        let ink = Colour::hex(0x1a1230);
        assert_eq!(ink.over(Colour::WHITE), ink);
    }

    #[test]
    fn a_translucent_colour_is_composited_over_its_background() {
        let scrim = Colour::rgba(0, 0, 0, 0x80);
        let over_white = scrim.over(Colour::WHITE);
        assert_eq!(over_white.a, 255);
        assert!((i16::from(over_white.r) - 127).abs() <= 1, "{over_white:?}");
        assert_eq!(Colour::rgba(9, 9, 9, 0).over(Colour::rgba(1, 2, 3, 0)).a, 0);
    }

    #[test]
    fn mixing_moves_toward_the_target_and_keeps_alpha() {
        let blue = Colour::hex(0x2f6fd6);
        assert_eq!(blue.mix(Colour::WHITE, 0.0), blue);
        assert_eq!(blue.mix(Colour::WHITE, 1.0), Colour::WHITE);
        assert_eq!(blue.mix(Colour::WHITE, 7.0), Colour::WHITE, "t is clamped");
        assert!(blue.lighter(0.3).luminance() > blue.luminance());
        assert!(blue.darker(0.3).luminance() < blue.luminance());
        assert_eq!(Colour::rgba(10, 20, 30, 7).darker(0.5).a, 7);
        assert_eq!(
            Colour::rgb(100, 100, 100).mix(Colour::BLACK, 0.5),
            Colour::rgb(50, 50, 50)
        );
    }
}
