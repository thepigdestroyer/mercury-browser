// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! OKLab: a colour space in which the straight-line distance between two colours tracks how
//! different they look, which sRGB distances do not. It is used to check that the tab-group
//! colours can be told apart and to size the tab shades.
//!
//! The space and its two 3x3 matrices are Bjorn Ottosson's, from the post "A perceptual color
//! space for image processing" (2020). Only those published constants are used; the code here
//! is written from the formulas, so nothing is copied and no third-party notice applies.
//!
//! Lightness `l` runs from 0 (black) to 1 (white). `a` is green (negative) to red (positive) and
//! `b` is blue (negative) to yellow (positive). Alpha is ignored throughout: composite with
//! [`Colour::over`] first when a colour is translucent.

use crate::colour::Colour;

/// A colour in OKLab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklab {
    /// Perceptual lightness: 0 is black, 1 is white.
    pub l: f64,
    /// Green (negative) to red (positive).
    pub a: f64,
    /// Blue (negative) to yellow (positive).
    pub b: f64,
}

/// Below this chroma a colour is treated as grey and has no meaningful hue. Neutral 8-bit greys
/// measure under 1e-7; the least saturated group colour (slate) measures 0.013.
const ACHROMATIC: f64 = 1e-4;

impl Oklab {
    /// The OKLab coordinates of an sRGB colour (alpha ignored).
    pub fn from_colour(colour: Colour) -> Self {
        let (r, g, b) = (
            to_linear(colour.r),
            to_linear(colour.g),
            to_linear(colour.b),
        );
        // Linear sRGB to the cone-like LMS space, then a cube root.
        let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
        let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
        let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
        Self {
            l: 0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
            a: 1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
            b: 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
        }
    }

    /// The nearest opaque sRGB colour. A colour outside the sRGB gamut is clamped channel by
    /// channel in linear light, which is crude but is only meant for round trips and for shades
    /// of colours that started in gamut.
    pub fn to_colour(self) -> Colour {
        let l = (self.l + 0.396_337_777_4 * self.a + 0.215_803_757_3 * self.b).powi(3);
        let m = (self.l - 0.105_561_345_8 * self.a - 0.063_854_172_8 * self.b).powi(3);
        let s = (self.l - 0.089_484_177_5 * self.a - 1.291_485_548_0 * self.b).powi(3);
        Colour::rgb(
            from_linear(4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s),
            from_linear(-1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s),
            from_linear(-0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s),
        )
    }

    /// The distance from the origin in the `a`/`b` plane: how colourful the colour is. 0 is a
    /// grey; the most saturated sRGB colours are about 0.3.
    pub fn chroma(self) -> f64 {
        self.a.hypot(self.b)
    }

    /// The hue angle in degrees, from 0 up to (not including) 360: about 29 for red, 143 for
    /// green, 264 for blue. A grey (chroma under 1e-4) has no hue and returns 0.
    pub fn hue(self) -> f64 {
        if self.chroma() < ACHROMATIC {
            return 0.0;
        }
        self.b.atan2(self.a).to_degrees().rem_euclid(360.0)
    }

    /// The Euclidean distance to `other`, the colour difference called delta-E in OKLab. About
    /// 0.02 is the smallest difference a person notices between two neighbouring patches, and
    /// black to white is 1.
    pub fn distance(self, other: Self) -> f64 {
        let (dl, da, db) = (self.l - other.l, self.a - other.a, self.b - other.b);
        (dl * dl + da * da + db * db).sqrt()
    }
}

impl From<Colour> for Oklab {
    fn from(colour: Colour) -> Self {
        Self::from_colour(colour)
    }
}

impl From<Oklab> for Colour {
    fn from(lab: Oklab) -> Self {
        lab.to_colour()
    }
}

/// The OKLab distance between two sRGB colours (alpha ignored), in either order.
pub fn delta_e(a: Colour, b: Colour) -> f64 {
    Oklab::from_colour(a).distance(Oklab::from_colour(b))
}

/// The OKLab lightness of an sRGB colour: 0 for black, 1 for white.
pub fn lightness(colour: Colour) -> f64 {
    Oklab::from_colour(colour).l
}

/// The OKLab chroma of an sRGB colour; see [`Oklab::chroma`].
pub fn chroma(colour: Colour) -> f64 {
    Oklab::from_colour(colour).chroma()
}

/// The OKLab hue of an sRGB colour in degrees; see [`Oklab::hue`].
pub fn hue(colour: Colour) -> f64 {
    Oklab::from_colour(colour).hue()
}

/// One sRGB channel as linear light (the exact sRGB curve, 0.04045 threshold).
fn to_linear(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light to one 8-bit sRGB channel, clamped to the gamut.
fn from_linear(value: f64) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let encoded = if value <= 0.003_130_8 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}
