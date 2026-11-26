// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! How a number is written: up to 12 significant digits, scientific form at or beyond 1e15 or below
//! 1e-6, and `,` thousands separators in the shown text only (spec D3).

/// `(shown, copy)` for a finite number: up to 12 significant digits; `shown` has `,` thousands
/// separators. `None` for NaN and infinities.
pub(crate) fn number(value: f64) -> Option<(String, String)> {
    if !value.is_finite() {
        return None;
    }
    let copy = plain(value);
    let shown = if copy.contains('e') {
        copy.clone()
    } else {
        with_separators(&copy)
    };
    Some((shown, copy))
}

fn plain(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude == 0.0 {
        return "0".into();
    }
    if !(1e-6..1e15).contains(&magnitude) {
        return scientific(value);
    }
    let decimals = usize::try_from((11 - decimal_exponent(magnitude)).max(0)).unwrap_or(0);
    trim_zeros(&format!("{value:.decimals$}"))
}

fn decimal_exponent(magnitude: f64) -> i32 {
    format!("{magnitude:.11e}")
        .split_once('e')
        .and_then(|(_, exponent)| exponent.parse().ok())
        .unwrap_or(0)
}

fn scientific(value: f64) -> String {
    let text = format!("{value:.11e}");
    match text.split_once('e') {
        Some((mantissa, exponent)) => {
            let mantissa = trim_zeros(mantissa);
            format!("{mantissa}e{exponent}")
        }
        None => text,
    }
}

fn trim_zeros(text: &str) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text.to_string()
    }
}

fn with_separators(plain: &str) -> String {
    let (sign, rest) = match plain.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", plain),
    };
    let (whole, fraction) = match rest.find('.') {
        Some(dot) => rest.split_at(dot),
        None => (rest, ""),
    };
    let mut grouped = String::with_capacity(plain.len() + whole.len() / 3);
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}{grouped}{fraction}")
}
