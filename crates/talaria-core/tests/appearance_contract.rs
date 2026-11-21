// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The contract between this crate's tab groups and `mercury-appearance`, which draws them.
//!
//! `talaria-core` stores a group's colour and key as text in `session.json`; `mercury-appearance`
//! reads the same text to pick the colours and the keys. Neither crate depends on the other, so
//! each pins its side as a literal list (`GroupPreset` here, `tests/group_colours.rs` there), and
//! this file, which links both, asserts that the two agree: the 14 ids, their hand-out order, the
//! way a stored colour is read, the colour a new group gets, and the default key sequence.
//! A change to one side that is not made to the other fails here.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mercury_appearance::group_colour::{self, PRESETS};
use mercury_appearance::keymap::parse_sequence;
use mercury_appearance::{GroupColour as Drawn, auto_colour};
use talaria_core::{
    DEFAULT_GROUP_KEY_SEQUENCE, GroupBook, GroupColour, GroupId, GroupKey, GroupPreset, Rgb,
};

/// The ids in hand-out order, written out a second time on purpose: if this list, `GroupPreset`
/// and `mercury_appearance::PRESETS` are not all equal, one of the tests below names which one
/// moved.
const CONTRACT_IDS: [&str; 14] = [
    "blue", "amber", "orchid", "mint", "rose", "lime", "plum", "teal", "salmon", "sky", "brown",
    "violet", "green", "slate",
];

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

fn core_ids() -> Vec<&'static str> {
    GroupPreset::ALL
        .iter()
        .map(|preset| preset.name())
        .collect()
}

fn drawn_ids() -> Vec<&'static str> {
    PRESETS.iter().map(|preset| preset.id).collect()
}

/// The stored ids of the colours the book's groups have, in strip order.
fn stored_colours(book: &GroupBook) -> Vec<String> {
    book.as_slice()
        .iter()
        .map(|group| group.colour.to_name())
        .collect()
}

/// The preset the drawing crate would hand a new group, given the colours in use.
fn drawn_default(book: &GroupBook) -> &'static str {
    let used = stored_colours(book);
    let used: Vec<&str> = used.iter().map(String::as_str).collect();
    auto_colour(&used).id
}

#[test]
fn the_core_ids_are_the_contract_ids_in_the_contract_order() {
    assert_eq!(core_ids(), CONTRACT_IDS, "GroupPreset::ALL moved");
}

#[test]
fn the_drawn_ids_are_the_contract_ids_in_the_contract_order() {
    assert_eq!(
        drawn_ids(),
        CONTRACT_IDS,
        "mercury_appearance::PRESETS moved"
    );
}

#[test]
fn both_crates_have_the_same_fourteen_ids_in_the_same_order() {
    assert_eq!(GroupPreset::ALL.len(), PRESETS.len());
    assert_eq!(core_ids(), drawn_ids());
    // The same thing, as one array comparison.
    assert_eq!(
        GroupPreset::ALL.map(GroupPreset::name),
        PRESETS.map(|p| p.id)
    );
}

#[test]
fn every_core_preset_is_drawn_and_every_drawn_preset_is_a_core_preset() {
    for preset in GroupPreset::ALL {
        let id = preset.name();
        assert!(group_colour::preset(id).is_some(), "{id} is not drawn");
        let drawn = Drawn::parse(id).unwrap_or_else(|| panic!("{id} is not read by appearance"));
        assert_eq!(drawn.id(), id);
    }
    for preset in &PRESETS {
        let core = GroupPreset::from_name(preset.id)
            .unwrap_or_else(|| panic!("{} is not a core preset", preset.id));
        assert_eq!(core.name(), preset.id);
    }
}

#[test]
fn both_crates_read_a_stored_colour_the_same_way() {
    let texts = [
        // presets, exactly as stored
        "blue",
        "slate",
        "salmon",
        // names of an earlier draft of the core list: unknown to both
        "red",
        "orange",
        "cyan",
        "indigo",
        "magenta",
        // capitals are no preset on either side
        "Blue",
        "BLUE",
        "bLue",
        "Slate",
        // custom colours: six hex digits in either case, nothing else
        "#2f6fd6",
        "#2F6FD6",
        "#2f6FD6",
        "#000000",
        "#ffffff",
        "#fff",
        "#12345",
        "#1234567",
        "#12345g",
        "#+12345",
        "#12 456",
        "##2f6fd6",
        "2f6fd6",
        "#ｆｆｆｆｆｆ",
        // shape errors
        "",
        "#",
        " blue",
        "blue ",
        "blue\n",
        "bluee",
        "blu",
        "rgb(1,2,3)",
        "é",
    ];
    for text in texts {
        let core = GroupColour::parse(text).map(GroupColour::to_name);
        let drawn = Drawn::parse(text).map(|colour| colour.id());
        assert_eq!(core, drawn, "{text:?} is read differently");
    }
}

#[test]
fn a_custom_colour_is_stored_and_read_back_the_same_by_both() {
    for (red, green, blue) in [
        (0x2f, 0x6f, 0xd6),
        (0, 0, 0),
        (255, 255, 255),
        (0x0a, 0x0b, 0x0c),
    ] {
        let stored = GroupColour::Custom(Rgb { red, green, blue }).to_name();
        let drawn = Drawn::parse(&stored).expect("appearance reads what core stores");
        assert_eq!(drawn.id(), stored);
        let base = drawn.base();
        assert_eq!((base.r, base.g, base.b), (red, green, blue));
    }
}

#[test]
fn the_default_colour_of_a_new_group_is_the_one_appearance_would_pick() {
    // The first 45 groups: every preset in order, then the least used, three rounds over.
    let mut book = GroupBook::new();
    for number in 0..45u64 {
        let expected = drawn_default(&book);
        let id = book
            .create(None, None, at(number))
            .expect("a group is made");
        assert_eq!(
            book.get(id).expect("the group exists").colour.to_name(),
            expected,
            "group {number}"
        );
    }
}

#[test]
fn the_default_colour_agrees_after_removals_recolours_and_custom_colours() {
    let custom = GroupColour::Custom(Rgb {
        red: 0x2f,
        green: 0x6f,
        blue: 0xd6,
    });
    let mut book = GroupBook::new();
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move |bound: u64| {
        // xorshift64: a fixed pseudo-random walk, the same on every run.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let mut made = 0;
    for step in 0..600u64 {
        match next(5) {
            // Make a group with the default colour and compare it.
            0..=2 => {
                if book.len() == talaria_core::MAX_GROUPS {
                    continue;
                }
                let expected = drawn_default(&book);
                let id = book.create(None, None, at(step)).expect("a group is made");
                made += 1;
                assert_eq!(
                    book.get(id).expect("the group exists").colour.to_name(),
                    expected,
                    "step {step}"
                );
            }
            // Remove one.
            3 => {
                if !book.is_empty() {
                    let id = book.as_slice()[next(book.len() as u64) as usize].id;
                    book.remove(id).expect("it exists");
                }
            }
            // Recolour one, to a preset or to a custom colour.
            _ => {
                if !book.is_empty() {
                    let id: GroupId = book.as_slice()[next(book.len() as u64) as usize].id;
                    let colour = match next(4) {
                        0 => custom,
                        pick => GroupColour::Preset(GroupPreset::ALL[(pick * 4) as usize]),
                    };
                    book.recolour(id, colour).expect("it exists");
                }
            }
        }
    }
    assert!(made > 100, "the walk made only {made} groups");
}

#[test]
fn the_default_key_sequence_is_the_same_in_both_crates() {
    assert_eq!(
        DEFAULT_GROUP_KEY_SEQUENCE,
        mercury_settings::DEFAULT_KEY_SEQUENCE
    );
    let core: Vec<char> = GroupKey::parse_sequence(DEFAULT_GROUP_KEY_SEQUENCE)
        .into_iter()
        .map(GroupKey::letter)
        .collect();
    let drawn: Vec<char> = parse_sequence(mercury_settings::DEFAULT_KEY_SEQUENCE)
        .into_iter()
        .map(|key| key.letter())
        .collect();
    assert_eq!(core, drawn);
    assert_eq!(core.len(), 9);
    // The way a stored sequence is read agrees on sloppier input as well.
    for text in [
        "asdfghjkl",
        "ASDFGHJKLA",
        "a s-d_f1",
        "",
        "QWERTYUIOPASDFGHJKLZXCVBNM",
        "éa",
    ] {
        let core: Vec<char> = GroupKey::parse_sequence(text)
            .into_iter()
            .map(GroupKey::letter)
            .collect();
        let drawn: Vec<char> = parse_sequence(text)
            .into_iter()
            .map(|key| key.letter())
            .collect();
        assert_eq!(core, drawn, "{text:?}");
    }
}
