// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Orbit mockup's rows (`design/ui-final/orbit/orbit-1-overview-*.svg`): 5 groups and 40 tabs
//! ("Personal", "Window 1 · 40 tabs"), the ten Research tabs of level 2 with their titles, and a
//! skeleton picture like the mockup's page drawing. Behind `cfg(any(test, feature = "test-support"))`;
//! frozen (an addition is fine, a change is not).
//!
//! Ids: groups 1 to 5 (Research, Lisbon trip, Reading, Shopping, Work; keys A S D F G); tabs 101
//! onward in ring order (Research 101 to 110, Lisbon trip 111 to 118, Reading 119 to 127,
//! Shopping 128 to 134, Work 135 to 140). Research's first tab is the active one ("Servo layout
//! notes"); the fourth is "AccessKit guide" (level 3: "tab 4 of 10"); WebRender overview (102) is
//! loading at 64 percent; Reading plays sound (tab 127); Lisbon trip and Work have an unread tab.

use slint::{Color, Image, Rgba8Pixel, SharedPixelBuffer, SharedString};

use super::OrbitInput;
use crate::ring_bands::fixtures::GROUP_PALETTES;
use crate::{GroupRow, TabRow};

/// Palette index (blue, amber, violet, rose, green) of each of the five groups.
const PALETTE: [usize; 5] = [0, 1, 2, 3, 4];

struct Spec {
    name: &'static str,
    key: &'static str,
    last_used: &'static str,
    /// The darkest and the lightest tab shade of the group (read from the SVG).
    from: u32,
    to: u32,
    tabs: &'static [(&'static str, &'static str)],
}

const SPECS: [Spec; 5] = [
    Spec {
        name: "Research",
        key: "A",
        last_used: "2 min ago",
        from: 0x2655A0,
        to: 0x82A9E6,
        tabs: &[
            ("Servo layout notes", "docs.example.org"),
            ("WebRender overview", "servo.example.org"),
            ("Slint reference", "slint.example.dev"),
            ("AccessKit guide", "a11y.example.net"),
            ("Forum: window dragging", "forum.example.net"),
            ("Rust book: lifetimes", "doc.example.org"),
            ("GPU notes", "gpu.example.dev"),
            ("Issue tracker", "issues.example.org"),
            ("Benchmarks", "bench.example.dev"),
            ("Style guide", "style.example.org"),
        ],
    },
    Spec {
        name: "Lisbon trip",
        key: "S",
        last_used: "1 h ago",
        from: 0xA76E23,
        to: 0xECBD7F,
        tabs: &[
            ("Flights to Lisbon", "flights.example.com"),
            ("Pasteis de Belem", "maps.example.com"),
            ("Tram 28 timetable", "transit.example.pt"),
            ("Hotel in Alfama", "stays.example.com"),
            ("Fado houses", "music.example.pt"),
            ("Time Out Market", "food.example.pt"),
            ("Sintra day trip", "trips.example.pt"),
            ("Metro map", "transit.example.pt"),
        ],
    },
    Spec {
        name: "Reading",
        key: "D",
        last_used: "now",
        from: 0x60469B,
        to: 0xB29DE3,
        tabs: &[
            ("The ring road, a history", "longform.example.net"),
            ("Essay on maps", "essays.example.net"),
            ("Notes on typography", "notes.example.net"),
            ("A field guide to moss", "field.example.org"),
            ("Short stories, week 12", "stories.example.net"),
            ("On slow software", "blog.example.dev"),
            ("The history of the sun", "science.example.org"),
            ("Letters from a cartographer", "letters.example.net"),
            ("Interview: Servo", "podcasts.example.org"),
        ],
    },
    Spec {
        name: "Shopping",
        key: "F",
        last_used: "yesterday",
        from: 0xA23954,
        to: 0xE891A8,
        tabs: &[
            ("Running shoes", "shop.example.com"),
            ("Desk lamp", "lights.example.com"),
            ("Bookshelf", "furniture.example.com"),
            ("Coffee grinder", "kitchen.example.com"),
            ("Backpack", "outdoor.example.com"),
            ("Headphones", "audio.example.com"),
            ("Plant pots", "garden.example.com"),
        ],
    },
    Spec {
        name: "Work",
        key: "G",
        last_used: "3 days ago",
        from: 0x26774B,
        to: 0x82C5A1,
        tabs: &[
            ("Sprint board", "work.example.com"),
            ("Design review", "docs.example.com"),
            ("Release checklist", "wiki.example.com"),
            ("Team calendar", "calendar.example.com"),
            ("Metrics dashboard", "metrics.example.com"),
            ("Inbox", "mail.example.com"),
        ],
    },
];

fn rgb(hex: u32) -> Color {
    Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn blend(from: u32, to: u32, t: f32) -> Color {
    let channel = |hex: u32, shift: u32| f32::from(((hex >> shift) & 0xFF) as u8);
    let mix = |shift: u32| {
        (channel(from, shift) + (channel(to, shift) - channel(from, shift)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::from_rgb_u8(mix(16), mix(8), mix(0))
}

/// The tab id (101 onward) of tab `index` of group `group` (0-based) in the fixture.
pub fn tab_id(group: usize, index: usize) -> i32 {
    let before: usize = SPECS[..group].iter().map(|s| s.tabs.len()).sum();
    101 + (before + index) as i32
}

/// The five mockup groups, Research first and active. The flags follow [`orbit_tabs`].
pub fn orbit_groups(dark: bool) -> Vec<GroupRow> {
    SPECS
        .iter()
        .enumerate()
        .map(|(g, spec)| {
            let palette = GROUP_PALETTES[PALETTE[g]];
            GroupRow {
                id: g as i32 + 1,
                name: spec.name.into(),
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
                key_label: spec.key.into(),
                count: spec.tabs.len() as i32,
                last_used: spec.last_used.into(),
                active: g == 0,
                any_loading: g == 0,
                any_audible: g == 2,
                any_unread: g == 1 || g == 4,
            }
        })
        .collect()
}

/// The 40 tabs in ring order (ids 101 to 140).
pub fn orbit_tabs() -> Vec<TabRow> {
    let mut rows = Vec::with_capacity(40);
    for (g, spec) in SPECS.iter().enumerate() {
        let n = spec.tabs.len();
        for (index, &(title, host)) in spec.tabs.iter().enumerate() {
            let t = if n > 1 {
                index as f32 / (n - 1) as f32
            } else {
                0.5
            };
            let id = tab_id(g, index);
            let loading = g == 0 && index == 1;
            rows.push(TabRow {
                id,
                group: g as i32 + 1,
                index_in_group: index as i32,
                title: title.into(),
                host: host.into(),
                shade: blend(spec.from, spec.to, t),
                favicon: Image::default(),
                active: g == 0 && index == 0,
                pinned: false,
                loading,
                progress: if loading { 0.64 } else { 0.0 },
                audible: g == 2 && index == n - 1,
                muted: false,
                unread: (g == 1 && index == 1) || (g == 4 && index == 0),
                crashed: false,
                capture: false,
                matches_query: true,
            });
        }
    }
    rows
}

/// The mockup's whole input: profile "Personal", "Window 1", Research's first tab active.
pub fn orbit_input(dark: bool) -> OrbitInput {
    OrbitInput::new(
        &orbit_groups(dark),
        &orbit_tabs(),
        "Personal",
        "Window 1",
        101,
        dark,
    )
}

/// `n_groups` groups of `tabs_each` tabs from `ring_bands::fixtures::generated`, as an input (for
/// the crowded cases: 12, 24 and 64 groups, a 20-tab group).
pub fn generated_input(n_groups: usize, n_tabs: usize, dark: bool) -> OrbitInput {
    let (groups, tabs) = crate::ring_bands::fixtures::generated(n_groups, n_tabs, dark);
    OrbitInput::new(&groups, &tabs, "Personal", "Window 1", 1000, dark)
}

/// A 480 x 300 picture drawn like the mockup's skeleton page (header bar, two title bars, a picture
/// with a mountain, a side card, text lines), light or dark. Stands for a tab snapshot.
pub fn skeleton_picture(dark: bool) -> Image {
    const W: usize = 480;
    const H: usize = 300;
    let (page, block, hill, line) = if dark {
        (
            [0x0F, 0x0B, 0x1E],
            [0x2A, 0x24, 0x4C],
            [0x35, 0x2D, 0x5E],
            [0x24, 0x1E, 0x42],
        )
    } else {
        (
            [0xFF, 0xFF, 0xFF],
            [0xEF, 0xE8, 0xDF],
            [0xDD, 0xD3, 0xC6],
            [0xE8, 0xE1, 0xD8],
        )
    };
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(W as u32, H as u32);
    let pixels = buffer.make_mut_slice();
    let mut fill = |x0: usize, y0: usize, w: usize, h: usize, colour: [u8; 3]| {
        for y in y0..(y0 + h).min(H) {
            for x in x0..(x0 + w).min(W) {
                pixels[y * W + x] = Rgba8Pixel::new(colour[0], colour[1], colour[2], 255);
            }
        }
    };
    fill(0, 0, W, H, page);
    fill(24, 18, 16, 16, block);
    fill(46, 22, 60, 8, block);
    for k in 0..4 {
        fill(320 + k * 36, 24, 28, 6, block);
    }
    fill(24, 56, 250, 22, block);
    fill(24, 86, 150, 22, block);
    fill(24, 120, 70, 6, line);
    fill(24, 138, 250, 100, block);
    for row in 0..100usize {
        let half = row * 2 / 3;
        fill(150usize.saturating_sub(half), 138 + row, half * 2, 1, hill);
    }
    fill(300, 56, 150, 90, line);
    fill(300, 160, 150, 70, line);
    for k in 0..5 {
        fill(24, 252 + k * 9, 240 - (k % 3) * 30, 4, line);
    }
    Image::from_rgba8(buffer)
}

/// A tab's title as the favicon chip's letter ("S" for "Servo layout notes").
pub fn letter(title: &str) -> SharedString {
    title
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mockup_has_five_groups_and_forty_tabs_that_agree() {
        for dark in [false, true] {
            let input = orbit_input(dark);
            assert_eq!(input.groups.len(), 5);
            assert_eq!(input.tabs.len(), 40);
            assert!(input.pinned.is_empty());
            for group in &input.groups {
                let own = input.tabs_of(group.id);
                assert_eq!(own.len() as i32, group.count, "{}", group.name);
                for (i, t) in own.iter().enumerate() {
                    assert_eq!(t.index_in_group, i as i32);
                }
            }
            assert_eq!(
                input.tab(104).map(|t| t.title.as_str()),
                Some("AccessKit guide")
            );
            assert_eq!(input.tabs.iter().filter(|t| t.active).count(), 1);
        }
    }

    #[test]
    fn loose_and_pinned_tabs_become_the_pseudo_planet() {
        let mut tabs = orbit_tabs();
        tabs[0].pinned = true;
        tabs.push(TabRow {
            id: 900,
            group: -1,
            ..tabs[1].clone()
        });
        let input = OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 101, false);
        let loose = input
            .group(super::super::LOOSE_GROUP_ID)
            .expect("pseudo planet");
        assert_eq!(loose.count, 2);
        assert_eq!(input.pinned, vec![101]);
        assert!(input.tabs.iter().all(|t| !t.pinned));
        assert_eq!(input.group(1).map(|g| g.count), Some(9));
        assert_eq!(
            input.groups.last().map(|g| g.id),
            Some(super::super::LOOSE_GROUP_ID)
        );
    }

    #[test]
    fn the_skeleton_picture_is_480_by_300() {
        let size = skeleton_picture(false).size();
        assert_eq!((size.width, size.height), (480, 300));
    }
}
