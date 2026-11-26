// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Text that looks like arithmetic but is not: a phone number, a date, a version or an address.

/// Whether `input` is something other than maths (spec D4).
pub(crate) fn not_maths(input: &str) -> bool {
    let text = input.trim();
    let spaced = text.chars().any(char::is_whitespace);
    let only_digits_and = |joint: char| text.chars().all(|c| c.is_ascii_digit() || c == joint);
    (!spaced && only_digits_and('-') && text.matches('-').count() >= 2)
        || (!spaced && only_digits_and('/') && text.matches('/').count() >= 2)
        || is_digit_group(text)
        || is_plus_number(text)
}

/// `ddd-dddd`, as in `555-1234`.
fn is_digit_group(text: &str) -> bool {
    match text.split_once('-') {
        Some((left, right)) => {
            left.len() == 3 && right.len() == 4 && all_digits(left) && all_digits(right)
        }
        None => false,
    }
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

/// `+` and a number with no operator after it, as in `+1 555 123 4567`.
fn is_plus_number(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('+') else {
        return false;
    };
    if !rest.starts_with(|c: char| c.is_ascii_digit()) {
        return false;
    }
    let tail = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
    let operator = tail.chars().any(|c| "+-*/^%×÷".contains(c))
        || tail
            .split_whitespace()
            .any(|word| matches!(word, "x" | "mod" | "of"));
    !operator
}
