// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use serde::{Deserialize, Serialize};

/// A paper size, named as the Print window shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperSize {
    A3,
    A4,
    A5,
    Letter,
    Legal,
    Tabloid,
}

impl PaperSize {
    pub const ALL: [PaperSize; 6] = [
        PaperSize::A3,
        PaperSize::A4,
        PaperSize::A5,
        PaperSize::Letter,
        PaperSize::Legal,
        PaperSize::Tabloid,
    ];

    /// The portrait width and height in PostScript points (1/72 inch).
    pub fn points(self) -> (f32, f32) {
        match self {
            PaperSize::A3 => (841.89, 1190.55),
            PaperSize::A4 => (595.28, 841.89),
            PaperSize::A5 => (419.53, 595.28),
            PaperSize::Letter => (612.0, 792.0),
            PaperSize::Legal => (612.0, 1008.0),
            PaperSize::Tabloid => (792.0, 1224.0),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PaperSize::A3 => "A3",
            PaperSize::A4 => "A4",
            PaperSize::A5 => "A5",
            PaperSize::Letter => "Letter",
            PaperSize::Legal => "Legal",
            PaperSize::Tabloid => "Tabloid",
        }
    }

    /// The default paper for a region code (the system locale's country): Letter where the US
    /// letter size is used, A4 everywhere else.
    pub fn for_region(region: &str) -> PaperSize {
        match region.to_ascii_uppercase().as_str() {
            "US" | "CA" | "MX" | "CL" | "CO" | "VE" | "PH" | "CR" | "GT" | "PR" => {
                PaperSize::Letter
            }
            _ => PaperSize::A4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Portrait,
    Landscape,
}

/// The margin choices of the Print window. Points, so they add to the paper size directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarginPreset {
    /// 10 mm on every side.
    Default,
    None,
    /// 5 mm on every side.
    Minimum,
}

impl MarginPreset {
    pub fn points(self) -> f32 {
        match self {
            MarginPreset::Default => 28.35,
            MarginPreset::None => 0.0,
            MarginPreset::Minimum => 14.17,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f32, expected: f32, tolerance: f32) -> bool {
        (actual - expected).abs() <= tolerance
    }

    #[test]
    fn a4_is_595_by_842_points() {
        let (width, height) = PaperSize::A4.points();
        assert!(close(width, 595.28, 0.5), "width {width}");
        assert!(close(height, 841.89, 0.5), "height {height}");
    }

    #[test]
    fn letter_is_612_by_792() {
        assert_eq!(PaperSize::Letter.points(), (612.0, 792.0));
    }

    #[test]
    fn all_lists_six_sizes_with_their_names() {
        assert_eq!(PaperSize::ALL.len(), 6);
        assert_eq!(PaperSize::A4.name(), "A4");
        assert_eq!(PaperSize::Letter.name(), "Letter");
        assert_eq!(PaperSize::Tabloid.name(), "Tabloid");
    }

    #[test]
    fn region_defaults() {
        for region in ["US", "CA", "MX", "CL", "CO", "VE", "PH", "CR", "GT", "PR"] {
            assert_eq!(PaperSize::for_region(region), PaperSize::Letter, "{region}");
        }
        for region in ["DE", "GB", ""] {
            assert_eq!(PaperSize::for_region(region), PaperSize::A4, "{region:?}");
        }
        assert_eq!(PaperSize::for_region("us"), PaperSize::Letter);
    }

    #[test]
    fn serde_names_are_snake_case() {
        assert_eq!(serde_json::to_string(&PaperSize::A4).unwrap(), "\"a4\"");
        assert_eq!(
            serde_json::to_string(&PaperSize::Letter).unwrap(),
            "\"letter\""
        );
        assert_eq!(
            serde_json::to_string(&Orientation::Landscape).unwrap(),
            "\"landscape\""
        );
        assert_eq!(
            serde_json::to_string(&MarginPreset::Minimum).unwrap(),
            "\"minimum\""
        );
    }
}
