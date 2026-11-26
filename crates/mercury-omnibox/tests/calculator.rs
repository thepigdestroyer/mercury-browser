// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
mod common;

use mercury_omnibox::{AnswerKind, answer};

const TABLE: &[(&str, &str, &str)] = &[
    ("2+2", "4", "4"),
    ("2*(3+4)", "14", "14"),
    ("2^10", "1,024", "1024"),
    ("10/4", "2.5", "2.5"),
    ("1/3", "0.333333333333", "0.333333333333"),
    ("0.1+0.2", "0.3", "0.3"),
    ("sqrt(16)", "4", "4"),
    ("100 * 15%", "15", "15"),
    ("200 + 10%", "220", "220"),
    ("200 - 10%", "180", "180"),
    ("15% of 200", "30", "30"),
    ("-3^2", "-9", "-9"),
    ("2^3^2", "512", "512"),
    ("1e3 + 1", "1,001", "1001"),
    ("pi * 2", "6.28318530718", "6.28318530718"),
    ("7 mod 3", "1", "1"),
    ("2 x 3", "6", "6"),
    ("6 ÷ 3", "2", "2"),
    ("2 × 3", "6", "6"),
    ("2 ** 3", "8", "8"),
    ("1_000 * 3", "3,000", "3000"),
    ("-2+5", "3", "3"),
    ("2*-3", "-6", "-6"),
    ("(1+2)", "3", "3"),
    ("abs(-5)", "5", "5"),
    ("round(2.5)", "3", "3"),
    ("floor(-1.5)", "-2", "-2"),
    ("log(1000)", "3", "3"),
    ("log2(8)", "3", "3"),
    ("ln(e)", "1", "1"),
    ("sin(0)", "0", "0"),
    ("cos(0)", "1", "1"),
    ("cbrt(27)", "3", "3"),
    ("atan(1)*4", "3.14159265359", "3.14159265359"),
    ("sin(pi/2)", "1", "1"),
    ("0*-1", "0", "0"),
    (
        "999999999999999+0",
        "999,999,999,999,999",
        "999999999999999",
    ),
    ("10^15", "1e15", "1e15"),
    ("10^20", "1e20", "1e20"),
    ("1.5 * 10^21", "1.5e21", "1.5e21"),
    ("1 / 10^7", "1e-7", "1e-7"),
    ("  2 +   2 ", "4", "4"),
    ("1234.5678 * 1", "1,234.5678", "1234.5678"),
];

const NONE: &[&str] = &[
    "1/0",
    "ln(0)",
    "sqrt(-1)",
    "2^2000",
    "1e300*1e300",
    "(1+2",
    "2+",
    "+",
    "()",
    "2 3",
    "1,5",
    "3",
    "pi",
    "e",
    "-5",
    "(5)",
    "1e3",
    "foo(2)",
    "sqrt 16",
    "2++",
    "5 %% 2",
];

#[test]
fn calculations_show_and_copy() {
    for &(input, shown, copied) in TABLE {
        assert_eq!(
            common::text(input).as_deref(),
            Some(shown),
            "text of {input:?}"
        );
        assert_eq!(
            common::copy(input).as_deref(),
            Some(copied),
            "copy of {input:?}"
        );
    }
}

#[test]
fn an_answer_carries_the_collapsed_query_and_kind() {
    let found = answer("  2 +   2 ", &common::ctx()).expect("an answer");
    assert_eq!(found.query, "2 + 2");
    assert_eq!(found.kind, AnswerKind::Calculation);
    assert_eq!(found.detail, None);
}

#[test]
fn bare_numbers_constants_and_broken_expressions_have_no_answer() {
    for &input in NONE {
        assert_eq!(common::text(input), None, "{input:?}");
    }
}
