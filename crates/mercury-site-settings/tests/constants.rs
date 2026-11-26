// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The zoom bounds match the settings crate, and the crate stays free of the engine and the toolkit.

use mercury_site_settings::{MAX_ZOOM_PERCENT, MIN_ZOOM_PERCENT};

#[test]
fn zoom_bounds_equal_the_settings_crates() {
    assert_eq!(MIN_ZOOM_PERCENT, mercury_settings::MIN_ZOOM_PERCENT);
    assert_eq!(MAX_ZOOM_PERCENT, mercury_settings::MAX_ZOOM_PERCENT);
}

#[test]
fn no_public_item_mentions_servo_or_slint() {
    let manifest = include_str!("../Cargo.toml");
    let dependencies: Vec<&str> = manifest
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .collect();
    for banned in ["servo", "slint", "talaria"] {
        let mentioned = dependencies
            .iter()
            .any(|line| line.to_ascii_lowercase().contains(banned));
        assert!(
            !mentioned,
            "Cargo.toml names `{banned}` outside a comment: {dependencies:?}"
        );
    }
}
