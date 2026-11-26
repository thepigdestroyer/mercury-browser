// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
mod common;

use mercury_omnibox::{AnswerKind, answer};

const TABLE: &[(&str, &str)] = &[
    ("5 km to mi", "3.10685596119 mi"),
    ("5 KM TO MI", "3.10685596119 mi"),
    ("5 kilometers to miles", "3.10685596119 mi"),
    ("5 km -> mi", "3.10685596119 mi"),
    ("5 km → mi", "3.10685596119 mi"),
    ("5 km into mi", "3.10685596119 mi"),
    ("5 km as mi", "3.10685596119 mi"),
    ("100 f to c", "37.7777777778 °C"),
    ("0 c in f", "32 °F"),
    ("-40 f to c", "-40 °C"),
    ("100 c to k", "373.15 K"),
    ("1 hour to minutes", "60 min"),
    ("1 day to hours", "24 h"),
    ("1 wk to d", "7 d"),
    ("1 yr to d", "365.25 d"),
    ("1 gb to mb", "1,000 MB"),
    ("1 gib to mib", "1,024 MiB"),
    ("1 B to bit", "8 bit"),
    ("60 mph to km/h", "96.56064 km/h"),
    ("10 knots to km/h", "18.52 km/h"),
    ("2 * 3 km to m", "6,000 m"),
    ("1 kg to lb", "2.20462262185 lb"),
    ("1 lb to oz", "16 oz"),
    ("1 mi to km", "1.609344 km"),
    ("1 gal to l", "3.785411784 L"),
    ("1 tsp to ml", "4.92892159375 mL"),
    ("1 ha to m2", "10,000 m²"),
    ("1 ft to in", "12 in"),
    ("1 yd to ft", "3 ft"),
    ("20 in to cm", "50.8 cm"),
    ("20 in in cm", "50.8 cm"),
    ("1 ft in in", "12 in"),
];

const NONE: &[&str] = &[
    "100 usd to eur",
    "5 km to kg",
    "5 km",
    "5 km to",
    "to mi",
    "5 to 6",
    "5 zz to mi",
    "5 km to km2",
    "c to f",
    "1/0 km to m",
];

#[test]
fn conversions_show_and_copy_without_thousands_separators() {
    for &(input, shown) in TABLE {
        let found =
            answer(input, &common::ctx()).unwrap_or_else(|| panic!("an answer for {input:?}"));
        assert_eq!(found.kind, AnswerKind::Conversion, "kind of {input:?}");
        assert_eq!(found.text, shown, "text of {input:?}");
        assert_eq!(found.copy, shown.replace(',', ""), "copy of {input:?}");
        assert_eq!(found.detail, None, "detail of {input:?}");
    }
}

#[test]
fn a_conversion_carries_the_collapsed_query() {
    let found = answer("  5 km   to mi ", &common::ctx()).expect("an answer");
    assert_eq!(found.query, "5 km to mi");
}

#[test]
fn unknown_units_mismatched_dimensions_and_missing_targets_have_no_answer() {
    for &input in NONE {
        assert_eq!(common::text(input), None, "{input:?}");
    }
}
