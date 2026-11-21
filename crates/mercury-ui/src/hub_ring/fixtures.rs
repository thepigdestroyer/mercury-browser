// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The hub ring's test scenes: [`ring_bands::fixtures`](crate::ring_bands::fixtures)'s 22 mockup
//! tabs in 6 groups, plus the three pinned tabs of the `ring-open` mockup (ids 200 to 202, initials
//! M, M and C) and the media state of the `ring-status` frame (the audio tab plays at 12:40 of
//! 48:00). Behind `cfg(any(test, feature = "test-support"))`; an addition is fine, a change is not.

use slint::{Color, Image, SharedString};

use super::{HubRingInput, LOOSE_GROUP_ID, TabMedia};
use crate::TabRow;
use crate::ring_bands::fixtures;

/// Ids of the three pinned tabs.
pub const PINNED_IDS: [i32; 3] = [200, 201, 202];

/// The three pinned tabs of the mockup (M, M, C): colours `#76629A`, `#A35F64`, `#5B7083`.
pub fn pinned_tabs() -> Vec<TabRow> {
    [
        ("Mail", "mail.example.org", 0x76629A_u32),
        ("Maps", "maps.example.org", 0xA35F64),
        ("Calendar", "calendar.example.org", 0x5B7083),
    ]
    .iter()
    .zip(PINNED_IDS)
    .map(|(&(title, host, shade), id)| TabRow {
        id,
        group: LOOSE_GROUP_ID,
        index_in_group: 0,
        title: SharedString::from(title),
        host: SharedString::from(host),
        shade: Color::from_rgb_u8((shade >> 16) as u8, (shade >> 8) as u8, shade as u8),
        favicon: Image::default(),
        pinned: true,
        matches_query: true,
        ..TabRow::default()
    })
    .collect()
}

/// The mockup scene: 6 groups, 22 tabs, 3 pinned tabs, the audio tab's media state, no previews.
pub fn mockup_input(dark: bool) -> HubRingInput {
    let groups = fixtures::mockup_groups(dark);
    let mut tabs = fixtures::mockup_tabs(dark);
    let media = tabs
        .iter()
        .filter(|tab| tab.audible)
        .map(|tab| TabMedia {
            tab: tab.id,
            playing: true,
            can_pause: true,
            position: Some((760, 2880)),
            ..TabMedia::default()
        })
        .collect();
    tabs.extend(pinned_tabs());
    HubRingInput {
        groups,
        tabs,
        media,
        dark,
        hold_ms: 400,
        ..HubRingInput::default()
    }
}
