// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
mod common;

const NOT_MATHS: &[&str] = &[
    "1-800-555-1234",
    "2026-11-22",
    "555-1234",
    "123-4567",
    "100-50-25",
    "11/22/2026",
    "+1 555 123 4567",
    "1.2.3",
    "10.0.0.1",
    "sin city",
    "hello world",
    "rust 1.75",
    "iphone 15 pro",
    "a+b",
    "https://example.com/1+1",
];

const MATHS: &[(&str, &str)] = &[
    ("10-5", "5"),
    ("1/2", "0.5"),
    ("5-3", "2"),
    ("1-2", "-1"),
    ("100 - 50 - 25", "25"),
    ("12/4", "3"),
    ("2 + 2", "4"),
];

#[test]
fn phone_numbers_dates_versions_and_addresses_are_not_maths() {
    for &input in NOT_MATHS {
        assert_eq!(common::text(input), None, "{input:?}");
    }
}

#[test]
fn short_subtraction_and_division_stay_maths() {
    for &(input, shown) in MATHS {
        assert_eq!(common::text(input).as_deref(), Some(shown), "{input:?}");
    }
}
