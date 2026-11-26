// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
mod common;

use std::time::{Duration, Instant};

#[test]
fn every_ascii_byte_alone_and_doubled_gives_no_answer() {
    for byte in 0u8..=127 {
        let single = char::from(byte).to_string();
        let doubled = format!("{single}{single}");
        assert_eq!(common::text(&single), None, "{single:?}");
        assert_eq!(common::text(&doubled), None, "{doubled:?}");
    }
}

#[test]
fn hostile_input_is_refused_quickly() {
    let started = Instant::now();
    let inputs = [
        "(".repeat(1_000_000),
        "(".repeat(10_000) + "1" + &")".repeat(10_000),
        "9^9^9^9".to_string(),
        "1".repeat(300),
        "\u{0}".to_string(),
        "\u{202E}2+2".to_string(),
        "２+２".to_string(),
    ];
    for input in &inputs {
        assert_eq!(common::text(input), None, "{:.40?}", input);
    }
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn nesting_past_32_levels_is_refused() {
    let within = format!("{}1+1{}", "(".repeat(10), ")".repeat(10));
    let past = format!("{}1+1{}", "(".repeat(33), ")".repeat(33));
    assert_eq!(common::text(&within).as_deref(), Some("2"));
    assert_eq!(common::text(&past), None);
}
