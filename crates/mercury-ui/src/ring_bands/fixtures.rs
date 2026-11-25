// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The owner-approved mockup's rows (`design/ui-final/ring/ring-open-*.svg`): 6 groups and 22
//! tabs, with the colours of the seven group colours' light and dark `tint` and `deep` and the
//! per-tab shades read from the SVG. Every lane that renders a ring, a dock, a dial or Orbit
//! starts from these so that its pictures can be compared with the mockup. Behind
//! `cfg(any(test, feature = "test-support"))`; frozen (an addition is fine, a change is not).
//!
//! Ids: group ids are 1 to 6 in ring order; tab ids are 101 to 122 in ring order (so "tab 101" is
//! the first Research tab); a lane that adds rows (pinned beads, a loose tab) uses ids from 200.
//!
//! What the mockup shows, kept here: Research's first tab is the current one; its second tab is
//! loading at 64 percent; Lisbon trip's second tab is unread; Work's first tab is unread; Reading's
//! last tab ("Interview: Servo") plays sound. No tab is pinned, muted, crashed or capturing.

use slint::{Color, Image, SharedString};

use crate::{GroupRow, TabRow};

/// One of the seven group colours of SPEC §1, with its light and dark label colours.
#[derive(Clone, Copy, Debug)]
pub struct GroupPalette {
    pub colour: u32,
    pub light_tint: u32,
    pub light_deep: u32,
    pub dark_tint: u32,
    pub dark_deep: u32,
}

/// Blue, amber, violet, rose, green, teal, slate (SPEC §1; teal's label colours are derived in the
/// same way as the other six, the mockup does not use it).
pub const GROUP_PALETTES: [GroupPalette; 7] = [
    GroupPalette {
        colour: 0x2F6FD6,
        light_tint: 0xDAE5F8,
        light_deep: 0x204B92,
        dark_tint: 0x223065,
        dark_deep: 0x86ABE7,
    },
    GroupPalette {
        colour: 0xE0912A,
        light_tint: 0xF9EBD9,
        light_deep: 0x98631D,
        dark_tint: 0x573A31,
        dark_deep: 0xEDBF83,
    },
    GroupPalette {
        colour: 0x7E5BD0,
        light_tint: 0xE8E1F7,
        light_deep: 0x563E8D,
        dark_tint: 0x392A63,
        dark_deep: 0xB4A0E4,
    },
    GroupPalette {
        colour: 0xD9486E,
        light_tint: 0xF8DEE5,
        light_deep: 0x94314B,
        dark_tint: 0x552445,
        dark_deep: 0xE995AB,
    },
    GroupPalette {
        colour: 0x2E9E62,
        light_tint: 0xD9EEE3,
        light_deep: 0x1F6B43,
        dark_tint: 0x213E42,
        dark_deep: 0x86C7A4,
    },
    GroupPalette {
        colour: 0x169C9C,
        light_tint: 0xD5EEEE,
        light_deep: 0x0F6B6B,
        dark_tint: 0x1F3F46,
        dark_deep: 0x7FCACA,
    },
    GroupPalette {
        colour: 0x7C8088,
        light_tint: 0xE7E8EA,
        light_deep: 0x54575C,
        dark_tint: 0x39354D,
        dark_deep: 0xB3B5BA,
    },
];

/// Index into [`GROUP_PALETTES`] for each mockup group: blue, amber, violet, rose, green, slate.
const MOCKUP_PALETTE: [usize; 6] = [0, 1, 2, 3, 4, 6];

fn rgb(hex: u32) -> Color {
    Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// One mockup tab: title, host, shade (the same in light and dark) and its states.
struct MockupTab {
    title: &'static str,
    host: &'static str,
    shade: u32,
    /// Percent of a load in progress, 0 for none.
    loading: u8,
    audible: bool,
    unread: bool,
}

const fn tab(title: &'static str, host: &'static str, shade: u32) -> MockupTab {
    MockupTab {
        title,
        host,
        shade,
        loading: 0,
        audible: false,
        unread: false,
    }
}

/// The tabs of each mockup group, in ring order.
fn mockup_tab_table() -> [Vec<MockupTab>; 6] {
    [
        vec![
            tab("Servo layout notes", "docs.example.org", 0x2655A0),
            MockupTab {
                loading: 64,
                ..tab("WebRender overview", "servo.example.org", 0x2B62BB)
            },
            tab("Slint rendering", "slint.example.dev", 0x2F6FD6),
            tab("AccessKit and VoiceOver", "accesskit.example.dev", 0x598CDE),
            tab("Forum: tab groups", "forum.example.net", 0x82A9E6),
        ],
        vec![
            tab("Flights to Lisbon", "flights.example.com", 0xA76E23),
            MockupTab {
                unread: true,
                ..tab("Pasteis de Belem", "maps.example.com", 0xCD8528)
            },
            tab("Tram 28 timetable", "transit.example.pt", 0xE4A046),
            tab("Hotel in Alfama", "stays.example.com", 0xECBD7F),
        ],
        vec![
            tab("The ring road, a history", "longform.example.net", 0x60469B),
            tab("Essay on maps", "essays.example.net", 0x6C4FB0),
            tab("Notes on typography", "notes.example.net", 0x7857C5),
            tab("A field guide to moss", "field.example.org", 0x8868D4),
            tab("Short stories, week 12", "stories.example.net", 0x9D82DB),
            MockupTab {
                audible: true,
                ..tab("Interview: Servo", "podcasts.example.org", 0xB29DE3)
            },
        ],
        vec![
            tab("Running shoes", "shop.example.com", 0xA23954),
            tab("Desk lamp", "lights.example.com", 0xD9486E),
            tab("Bookshelf", "furniture.example.com", 0xE891A8),
        ],
        vec![
            MockupTab {
                unread: true,
                ..tab("Sprint board", "work.example.com", 0x26774B)
            },
            tab("Design review", "docs.example.com", 0x82C5A1),
        ],
        vec![
            tab("Weather", "weather.example.org", 0x5E6167),
            tab("Recipes", "recipes.example.net", 0xB0B3B8),
        ],
    ]
}

const MOCKUP_GROUPS: [(&str, &str, &str); 6] = [
    ("Research", "A", "now"),
    ("Lisbon trip", "S", "2 min ago"),
    ("Reading", "D", "1 hour ago"),
    ("Shopping", "F", "yesterday"),
    ("Work", "G", "yesterday"),
    ("Loose", "H", "3 days ago"),
];

/// The mockup's six groups (Research, Lisbon trip, Reading, Shopping, Work, Loose) with the keys
/// A, S, D, F, G, H; Research is the active group. The loading, audible and unread flags of the
/// groups follow from the tabs of [`mockup_tabs`].
pub fn mockup_groups(dark: bool) -> Vec<GroupRow> {
    let table = mockup_tab_table();
    MOCKUP_GROUPS
        .iter()
        .enumerate()
        .map(|(index, &(name, key, last_used))| {
            let palette = GROUP_PALETTES[MOCKUP_PALETTE[index]];
            let tabs = &table[index];
            group_row(
                index as i32 + 1,
                name,
                key,
                last_used,
                palette,
                dark,
                index == 0,
                tabs.len(),
                tabs.iter().any(|t| t.loading > 0),
                tabs.iter().any(|t| t.audible),
                tabs.iter().any(|t| t.unread),
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn group_row(
    id: i32,
    name: &str,
    key: &str,
    last_used: &str,
    palette: GroupPalette,
    dark: bool,
    active: bool,
    count: usize,
    any_loading: bool,
    any_audible: bool,
    any_unread: bool,
) -> GroupRow {
    GroupRow {
        id,
        name: name.into(),
        colour: rgb(palette.colour),
        tint: rgb(if dark {
            palette.dark_tint
        } else {
            palette.light_tint
        }),
        deep: rgb(if dark {
            palette.dark_deep
        } else {
            palette.light_deep
        }),
        key_label: key.into(),
        count: count as i32,
        last_used: last_used.into(),
        active,
        any_loading,
        any_audible,
        any_unread,
    }
}

/// The mockup's 22 tabs in ring order (ids 101 to 122; `group` is 1 to 6). The shades are the same
/// in light and dark (the mockup draws them so); `dark` is accepted so that callers read alike.
pub fn mockup_tabs(_dark: bool) -> Vec<TabRow> {
    let table = mockup_tab_table();
    let mut rows = Vec::with_capacity(22);
    let mut id = 101;
    for (group, tabs) in table.iter().enumerate() {
        for (index, t) in tabs.iter().enumerate() {
            rows.push(TabRow {
                id,
                group: group as i32 + 1,
                index_in_group: index as i32,
                title: t.title.into(),
                host: t.host.into(),
                shade: rgb(t.shade),
                favicon: Image::default(),
                active: group == 0 && index == 0,
                pinned: false,
                loading: t.loading > 0,
                progress: f32::from(t.loading) / 100.0,
                audible: t.audible,
                muted: false,
                unread: t.unread,
                crashed: false,
                capture: false,
                matches_query: true,
            });
            id += 1;
        }
    }
    rows
}

/// `n_groups` groups (ids 1 to `n_groups`, colours cycling through the seven, names "Group N",
/// keys A S D F G H J K L then none) holding `n_tabs` tabs shared as evenly as possible (the first
/// groups get the extra ones; tab ids from 1000), for the crowded-ring scenes (60 tabs, 64 groups).
/// A tab's shade runs from darker to lighter across its group, as in SPEC §1. No flags are set; the
/// first tab is active.
pub fn generated(n_groups: usize, n_tabs: usize, dark: bool) -> (Vec<GroupRow>, Vec<TabRow>) {
    const KEYS: [&str; 9] = ["A", "S", "D", "F", "G", "H", "J", "K", "L"];
    let n_groups = n_groups.max(1);
    let base = n_tabs / n_groups;
    let extra = n_tabs % n_groups;
    let mut groups = Vec::with_capacity(n_groups);
    let mut tabs = Vec::with_capacity(n_tabs);
    let mut id = 1000;
    for g in 0..n_groups {
        let size = base + usize::from(g < extra);
        let palette = GROUP_PALETTES[g % GROUP_PALETTES.len()];
        groups.push(group_row(
            g as i32 + 1,
            &format!("Group {}", g + 1),
            KEYS.get(g).copied().unwrap_or(""),
            "now",
            palette,
            dark,
            g == 0,
            size,
            false,
            false,
            false,
        ));
        for index in 0..size {
            let t = if size > 1 {
                index as f32 / (size - 1) as f32
            } else {
                0.5
            };
            tabs.push(TabRow {
                id,
                group: g as i32 + 1,
                index_in_group: index as i32,
                title: SharedString::from(format!("Tab {} of group {}", index + 1, g + 1)),
                host: format!("site{}.example.org", id).into(),
                shade: shade(palette.colour, t),
                favicon: Image::default(),
                active: id == 1000,
                pinned: false,
                loading: false,
                progress: 0.0,
                audible: false,
                muted: false,
                unread: false,
                crashed: false,
                capture: false,
                matches_query: true,
            });
            id += 1;
        }
    }
    (groups, tabs)
}

/// The group colour mixed toward black (t = 0, by 22 percent) or white (t = 1, by 45 percent).
fn shade(colour: u32, t: f32) -> Color {
    let channel = |shift: u32| f32::from(((colour >> shift) & 0xFF) as u8);
    let mix = |c: f32| {
        let v = if t < 0.5 {
            c * (1.0 - 0.22 * (1.0 - t * 2.0))
        } else {
            c + (255.0 - c) * 0.45 * ((t - 0.5) * 2.0)
        };
        v.round().clamp(0.0, 255.0) as u8
    };
    Color::from_rgb_u8(mix(channel(16)), mix(channel(8)), mix(channel(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mockup_has_six_groups_and_twenty_two_tabs_that_agree() {
        for dark in [false, true] {
            let groups = mockup_groups(dark);
            let tabs = mockup_tabs(dark);
            assert_eq!(groups.len(), 6);
            assert_eq!(tabs.len(), 22);
            for group in &groups {
                let own: Vec<_> = tabs.iter().filter(|t| t.group == group.id).collect();
                assert_eq!(own.len() as i32, group.count, "{}", group.name);
                for (i, t) in own.iter().enumerate() {
                    assert_eq!(t.index_in_group, i as i32);
                }
            }
            let mut ids: Vec<_> = tabs.iter().map(|t| t.id).collect();
            ids.dedup();
            assert_eq!(ids.len(), 22);
            assert_eq!(tabs.iter().filter(|t| t.active).count(), 1);
        }
    }

    #[test]
    fn dark_changes_the_label_colours_and_not_the_shades() {
        let (light, dark) = (mockup_groups(false), mockup_groups(true));
        assert_ne!(light[0].tint, dark[0].tint);
        assert_ne!(light[0].deep, dark[0].deep);
        assert_eq!(light[0].colour, dark[0].colour);
        assert_eq!(mockup_tabs(false)[3].shade, mockup_tabs(true)[3].shade);
    }

    #[test]
    fn the_mockup_flags_are_where_the_mockup_has_them() {
        let tabs = mockup_tabs(false);
        assert!(tabs[1].loading && (tabs[1].progress - 0.64).abs() < 1e-6);
        assert!(tabs[6].unread && tabs[18].unread);
        assert_eq!(tabs.iter().filter(|t| t.unread).count(), 2);
        assert!(tabs[14].audible && tabs[14].group == 3);
        assert_eq!(tabs.iter().filter(|t| t.audible).count(), 1);
        let groups = mockup_groups(false);
        assert!(groups[0].any_loading && groups[1].any_unread && groups[2].any_audible);
    }

    #[test]
    fn generated_rings_share_the_tabs_out() {
        let (groups, tabs) = generated(5, 60, false);
        assert_eq!((groups.len(), tabs.len()), (5, 60));
        assert!(groups.iter().all(|g| g.count == 12));
        let (groups, tabs) = generated(64, 64, true);
        assert_eq!((groups.len(), tabs.len()), (64, 64));
        assert!(groups.iter().all(|g| g.count == 1));
        let (_, tabs) = generated(3, 8, false);
        assert_eq!(tabs.iter().filter(|t| t.group == 1).count(), 3);
        assert_eq!(tabs.iter().filter(|t| t.group == 3).count(), 2);
    }
}
