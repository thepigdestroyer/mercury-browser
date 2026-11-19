// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The pure group model (groups-core lane, item `groups-model`): names, colours, keys, ids, the
//! group book, the status flags of a group and the load-progress estimate. No tabs and no clock:
//! every time is handed in.

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use talaria_core::{
    CaptureAccess, DEFAULT_GROUP_KEY_SEQUENCE, GroupBook, GroupColour, GroupError, GroupId,
    GroupKey, GroupPreset, GroupSummary, MAX_GROUP_ID, MAX_GROUP_NAME_BYTES, MAX_GROUPS,
    RestoredGroup, Rgb, TabStatus, clean_group_name, load_progress_estimate,
};

// ----- helpers -------------------------------------------------------------------------------

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

fn key(letter: char) -> GroupKey {
    GroupKey::from_letter(letter).expect("the test letter is a letter")
}

fn blue() -> GroupColour {
    GroupColour::Preset(GroupPreset::Blue)
}

fn make(book: &mut GroupBook) -> GroupId {
    book.create(None, None, at(100))
        .expect("the book has room for another group")
}

fn make_named(book: &mut GroupBook, name: &str) -> GroupId {
    book.create(Some(name), None, at(100))
        .expect("the book has room for another group")
}

fn book_of(count: usize) -> (GroupBook, Vec<GroupId>) {
    let mut book = GroupBook::new();
    let ids = (0..count).map(|_| make(&mut book)).collect();
    (book, ids)
}

fn names(book: &GroupBook) -> Vec<String> {
    book.as_slice()
        .iter()
        .map(|group| group.name.clone())
        .collect()
}

fn ids(book: &GroupBook) -> Vec<GroupId> {
    book.as_slice().iter().map(|group| group.id).collect()
}

fn key_of(book: &GroupBook, id: GroupId) -> Option<GroupKey> {
    book.get(id).expect("the group exists").key
}

fn colour_of(book: &GroupBook, id: GroupId) -> GroupColour {
    book.get(id).expect("the group exists").colour
}

fn name_of(book: &GroupBook, id: GroupId) -> String {
    book.get(id).expect("the group exists").name.clone()
}

fn preset(book: &GroupBook, id: GroupId) -> GroupPreset {
    match colour_of(book, id) {
        GroupColour::Preset(preset) => preset,
        GroupColour::Custom(rgb) => panic!("expected a preset, found {rgb:?}"),
    }
}

/// The rules every book keeps, whatever was done to it.
fn assert_book_invariants(book: &GroupBook) {
    assert!(book.len() <= MAX_GROUPS, "more than {MAX_GROUPS} groups");
    assert_eq!(book.len(), book.as_slice().len());
    assert_eq!(book.is_empty(), book.as_slice().is_empty());
    let mut seen_ids = HashSet::new();
    let mut seen_keys = HashSet::new();
    for group in book.as_slice() {
        assert!(seen_ids.insert(group.id), "id {:?} repeats", group.id);
        assert!(group.id.value() >= 1, "ids start at 1");
        assert!(group.id.value() <= MAX_GROUP_ID, "id above MAX_GROUP_ID");
        assert!(
            group.id.value() < book.next_id(),
            "id {:?} is not below next_id {}",
            group.id,
            book.next_id()
        );
        if let Some(key) = group.key {
            assert!(seen_keys.insert(key), "key {:?} repeats", key.letter());
        }
        assert!(!group.name.is_empty(), "empty name");
        assert!(group.name.len() <= MAX_GROUP_NAME_BYTES, "name too long");
        assert_eq!(group.name.trim(), group.name, "name not trimmed");
        assert!(
            !group.name.chars().any(char::is_control),
            "control character in a name"
        );
        assert_eq!(book.get(group.id), Some(group));
    }
    assert!(book.next_id() >= 1);
}

// ----- name cleaning -------------------------------------------------------------------------

#[test]
fn a_plain_name_is_kept_as_it_is() {
    assert_eq!(
        clean_group_name("Reading list").as_deref(),
        Some("Reading list")
    );
    assert_eq!(clean_group_name("a  b").as_deref(), Some("a  b"));
    assert_eq!(
        clean_group_name("Zürich ☕ 日本").as_deref(),
        Some("Zürich ☕ 日本")
    );
}

#[test]
fn control_characters_are_removed() {
    assert_eq!(
        clean_group_name("a\u{0}b\nc\td\u{7f}e\u{85}f\r").as_deref(),
        Some("abcdef")
    );
    assert_eq!(
        clean_group_name("Work\u{1b}[31mstuff").as_deref(),
        Some("Work[31mstuff")
    );
}

#[test]
fn a_name_is_trimmed_on_both_sides() {
    assert_eq!(clean_group_name("  Home ").as_deref(), Some("Home"));
    // Unicode white space counts: no-break space and an em space.
    assert_eq!(
        clean_group_name("\u{a0} Reading list \u{2003}").as_deref(),
        Some("Reading list")
    );
    // Control characters go first, so white space they were hiding is trimmed too.
    assert_eq!(clean_group_name("\n  Home \t").as_deref(), Some("Home"));
    assert_eq!(clean_group_name(" \u{0} Home").as_deref(), Some("Home"));
}

#[test]
fn a_name_is_cut_to_64_bytes() {
    let exactly = "x".repeat(MAX_GROUP_NAME_BYTES);
    assert_eq!(
        clean_group_name(&exactly).as_deref(),
        Some(exactly.as_str())
    );
    let longer = "x".repeat(200);
    let cut = clean_group_name(&longer).expect("something is left");
    assert_eq!(cut.len(), MAX_GROUP_NAME_BYTES);
    assert!(cut.chars().all(|letter| letter == 'x'));
}

#[test]
fn the_cut_never_splits_a_character() {
    // 63 bytes, then a two-byte character that would end at byte 65.
    let two_byte = format!("{}é", "a".repeat(63));
    assert_eq!(two_byte.len(), 65);
    assert_eq!(
        clean_group_name(&two_byte).as_deref(),
        Some("a".repeat(63).as_str())
    );
    // A four-byte emoji straddling the limit.
    let emoji = format!("{}😀", "a".repeat(61));
    assert_eq!(emoji.len(), 65);
    assert_eq!(
        clean_group_name(&emoji).as_deref(),
        Some("a".repeat(61).as_str())
    );
    // Only multi-byte characters: 80 bytes of "é" is 40 letters, 64 bytes keep 32.
    let all_wide = "é".repeat(40);
    let cut = clean_group_name(&all_wide).expect("something is left");
    assert_eq!(cut, "é".repeat(32));
    assert_eq!(cut.len(), 64);
    // Every cut point of a mixed string is a valid string no longer than the limit.
    for prefix in 0..30 {
        let mixed = format!("{}{}", "k".repeat(prefix), "日本語é😀".repeat(8));
        let cut = clean_group_name(&mixed).expect("something is left");
        assert!(cut.len() <= MAX_GROUP_NAME_BYTES);
        assert!(mixed.starts_with(&cut));
    }
}

#[test]
fn a_name_cut_at_a_space_is_trimmed_again() {
    let name = format!("{} {}", "a".repeat(63), "bbbb");
    let cut = clean_group_name(&name).expect("something is left");
    assert_eq!(cut, "a".repeat(63));
    assert_eq!(cut.trim(), cut);
    // Several spaces right at the limit.
    let spaced = format!("{}   {}", "a".repeat(62), "tail");
    assert_eq!(clean_group_name(&spaced), Some("a".repeat(62)));
}

#[test]
fn a_name_with_nothing_left_is_none() {
    for raw in [
        "",
        " ",
        "   \t  ",
        "\u{0}",
        "\n\r\n",
        "\u{a0}\u{2003}",
        " \u{7f} \u{85} ",
    ] {
        assert_eq!(clean_group_name(raw), None, "{raw:?}");
    }
}

#[test]
fn cleaning_twice_changes_nothing() {
    for raw in [
        "  Home ",
        "a\nb",
        "é",
        "plain",
        "\u{a0}x\u{a0}",
        &"é".repeat(100),
        &format!("{} {}", "a".repeat(63), "b"),
        &format!("{}\n\n{}", "a".repeat(40), "b".repeat(40)),
    ] {
        let once = clean_group_name(raw);
        let twice = once.as_deref().and_then(clean_group_name);
        assert_eq!(once, twice, "{raw:?}");
    }
}

// ----- default names -------------------------------------------------------------------------

#[test]
fn new_groups_are_called_group_1_group_2_and_so_on() {
    let (book, _) = book_of(3);
    assert_eq!(names(&book), ["Group 1", "Group 2", "Group 3"]);
}

#[test]
fn a_default_name_takes_the_smallest_number_nobody_has() {
    let (mut book, group_ids) = book_of(3);
    book.remove(group_ids[1]).expect("group 2 exists");
    assert_eq!(names(&book), ["Group 1", "Group 3"]);
    let fresh = make(&mut book);
    assert_eq!(name_of(&book, fresh), "Group 2");
    let next = make(&mut book);
    assert_eq!(name_of(&book, next), "Group 4");

    let (mut book, group_ids) = book_of(3);
    book.remove(group_ids[0]).expect("group 1 exists");
    book.remove(group_ids[2]).expect("group 3 exists");
    let first = make(&mut book);
    let second = make(&mut book);
    assert_eq!(name_of(&book, first), "Group 1");
    assert_eq!(name_of(&book, second), "Group 3");
}

#[test]
fn a_renamed_group_gives_its_default_name_back() {
    let (mut book, group_ids) = book_of(1);
    book.rename(group_ids[0], "Home").expect("renamed");
    let fresh = make(&mut book);
    assert_eq!(name_of(&book, fresh), "Group 1");

    // Renamed to another default name: the number it took is taken, the one it left is free.
    let mut book = GroupBook::new();
    let first = make(&mut book);
    book.rename(first, "Group 2").expect("renamed");
    let second = make(&mut book);
    let third = make(&mut book);
    assert_eq!(name_of(&book, second), "Group 1");
    assert_eq!(name_of(&book, third), "Group 3");
}

#[test]
fn only_an_exact_match_counts_as_taken() {
    let mut book = GroupBook::new();
    make_named(&mut book, "group 1");
    make_named(&mut book, "Group  1");
    make_named(&mut book, "Group 01");
    make_named(&mut book, "My Group 1");
    let fresh = make(&mut book);
    assert_eq!(name_of(&book, fresh), "Group 1");
}

#[test]
fn a_name_that_cleans_to_nothing_is_the_default() {
    let mut book = GroupBook::new();
    let first = book.create(Some(" \n\t "), None, at(1)).expect("created");
    let second = book.create(Some(""), None, at(1)).expect("created");
    assert_eq!(name_of(&book, first), "Group 1");
    assert_eq!(name_of(&book, second), "Group 2");
}

#[test]
fn a_given_name_is_cleaned_and_need_not_be_unique() {
    let mut book = GroupBook::new();
    let first = make_named(&mut book, "  Work\n ");
    let second = make_named(&mut book, "Work");
    assert_eq!(name_of(&book, first), "Work");
    assert_eq!(name_of(&book, second), "Work");
    let long = make_named(&mut book, &"é".repeat(100));
    assert_eq!(name_of(&book, long).len(), MAX_GROUP_NAME_BYTES);
}

#[test]
fn rename_cleans_the_name_and_keeps_the_old_one_when_nothing_is_left() {
    let (mut book, group_ids) = book_of(2);
    let group = group_ids[0];
    book.rename(group, "  Travel\t").expect("renamed");
    assert_eq!(name_of(&book, group), "Travel");
    book.rename(group, "Group 2")
        .expect("a duplicate name is allowed");
    assert_eq!(name_of(&book, group), "Group 2");
    for empty in ["", "   ", "\n\t", "\u{0}"] {
        assert_eq!(
            book.rename(group, empty),
            Err(GroupError::EmptyName),
            "{empty:?}"
        );
        assert_eq!(name_of(&book, group), "Group 2");
    }
    assert_eq!(
        book.rename(GroupId::new(99), "Nope"),
        Err(GroupError::NotFound)
    );
    assert_eq!(names(&book), ["Group 2", "Group 2"]);
}

#[test]
fn a_restored_group_without_a_usable_name_gets_the_default() {
    let mut book = GroupBook::new();
    make_named(&mut book, "Group 1");
    for name in [None, Some(String::new()), Some("  \n ".to_owned())] {
        let restored = book
            .restore(
                RestoredGroup {
                    name,
                    ..RestoredGroup::default()
                },
                at(1),
            )
            .expect("restored");
        assert!(name_of(&book, restored).starts_with("Group "));
    }
    assert_eq!(names(&book), ["Group 1", "Group 2", "Group 3", "Group 4"]);
    let named = book
        .restore(
            RestoredGroup {
                name: Some("  Saved\t".to_owned()),
                ..RestoredGroup::default()
            },
            at(1),
        )
        .expect("restored");
    assert_eq!(name_of(&book, named), "Saved");
}

// ----- default colours -----------------------------------------------------------------------

#[test]
fn the_first_fourteen_groups_take_the_presets_in_order() {
    let (book, group_ids) = book_of(14);
    for (index, id) in group_ids.iter().enumerate() {
        assert_eq!(preset(&book, *id), GroupPreset::ALL[index]);
    }
    assert_eq!(GroupPreset::ALL[0], GroupPreset::Blue);
    assert_eq!(GroupPreset::ALL[13], GroupPreset::Brown);
}

#[test]
fn once_all_fourteen_are_used_the_least_used_goes_first() {
    let (book, group_ids) = book_of(28);
    for (index, id) in group_ids.iter().enumerate() {
        assert_eq!(
            preset(&book, *id),
            GroupPreset::ALL[index % 14],
            "group {index}"
        );
    }
    // 14 more still go round in the same order (42 groups, three of each).
    let (book, group_ids) = book_of(42);
    assert_eq!(preset(&book, group_ids[41]), GroupPreset::ALL[41 % 14]);
}

#[test]
fn a_preset_that_was_freed_is_taken_before_any_is_reused() {
    let (mut book, group_ids) = book_of(14);
    // Rose is the fourth preset.
    assert_eq!(preset(&book, group_ids[3]), GroupPreset::Rose);
    book.remove(group_ids[3]).expect("removed");
    let fresh = make(&mut book);
    assert_eq!(preset(&book, fresh), GroupPreset::Rose);
    // Recolouring frees the old colour as well.
    let (mut book, group_ids) = book_of(3);
    book.recolour(group_ids[1], GroupColour::Preset(GroupPreset::Red))
        .expect("recoloured");
    let fresh = make(&mut book);
    assert_eq!(preset(&book, fresh), GroupPreset::Amber);
}

#[test]
fn uneven_use_decides_by_count_then_by_order() {
    let (mut book, group_ids) = book_of(14);
    // Amber goes to Blue: Blue twice, Amber not at all.
    book.recolour(group_ids[1], blue()).expect("recoloured");
    let first = make(&mut book);
    assert_eq!(
        preset(&book, first),
        GroupPreset::Amber,
        "the unused one first"
    );
    // Now all fourteen are used; Blue twice, the rest once: the earliest of the rest.
    let second = make(&mut book);
    assert_eq!(preset(&book, second), GroupPreset::Amber);
    // Amber and Blue twice each: Violet is the earliest with one.
    let third = make(&mut book);
    assert_eq!(preset(&book, third), GroupPreset::Violet);
}

#[test]
fn custom_colours_do_not_count_as_used() {
    let custom = GroupColour::Custom(Rgb {
        red: 0x2f,
        green: 0x6f,
        blue: 0xd6,
    });
    let mut book = GroupBook::new();
    let first = book.create(None, Some(custom), at(1)).expect("created");
    assert_eq!(colour_of(&book, first), custom);
    let second = make(&mut book);
    assert_eq!(preset(&book, second), GroupPreset::Blue);
    // A custom colour equal in look to a preset is still custom: Blue stays free for counting.
    book.recolour(second, custom).expect("recoloured");
    let third = make(&mut book);
    assert_eq!(preset(&book, third), GroupPreset::Blue);
}

#[test]
fn a_given_colour_is_used_as_given() {
    let mut book = GroupBook::new();
    let first = book
        .create(None, Some(GroupColour::Preset(GroupPreset::Brown)), at(1))
        .expect("created");
    assert_eq!(preset(&book, first), GroupPreset::Brown);
    let second = make(&mut book);
    assert_eq!(preset(&book, second), GroupPreset::Blue);
    book.recolour(second, GroupColour::Preset(GroupPreset::Brown))
        .expect("the same colour twice is allowed");
    assert_eq!(preset(&book, second), GroupPreset::Brown);
    assert_eq!(
        book.recolour(GroupId::new(77), blue()),
        Err(GroupError::NotFound)
    );
}

#[test]
fn a_restored_group_without_a_colour_gets_the_default() {
    let mut book = GroupBook::new();
    let first = book
        .restore(RestoredGroup::default(), at(1))
        .expect("restored");
    let second = book
        .restore(RestoredGroup::default(), at(1))
        .expect("restored");
    assert_eq!(preset(&book, first), GroupPreset::Blue);
    assert_eq!(preset(&book, second), GroupPreset::Amber);
    let given = book
        .restore(
            RestoredGroup {
                colour: Some(GroupColour::Preset(GroupPreset::Teal)),
                ..RestoredGroup::default()
            },
            at(1),
        )
        .expect("restored");
    assert_eq!(preset(&book, given), GroupPreset::Teal);
}

// ----- colour names --------------------------------------------------------------------------

#[test]
fn there_are_fourteen_distinct_preset_names() {
    assert_eq!(GroupPreset::ALL.len(), 14);
    let names: HashSet<&str> = GroupPreset::ALL
        .iter()
        .map(|preset| preset.name())
        .collect();
    assert_eq!(names.len(), 14);
    for preset in GroupPreset::ALL {
        let name = preset.name();
        assert!(
            (1..=16).contains(&name.len()) && name.bytes().all(|byte| byte.is_ascii_lowercase()),
            "{name} is not a lower-case ASCII name of 1 to 16 letters"
        );
    }
    let presets: HashSet<GroupPreset> = GroupPreset::ALL.into_iter().collect();
    assert_eq!(presets.len(), 14);
}

#[test]
fn the_names_and_their_order_are_the_contract() {
    let listed: Vec<&str> = GroupPreset::ALL
        .iter()
        .map(|preset| preset.name())
        .collect();
    assert_eq!(
        listed,
        [
            "blue", "amber", "violet", "rose", "green", "teal", "slate", "red", "orange", "lime",
            "cyan", "indigo", "magenta", "brown"
        ]
    );
}

#[test]
fn every_preset_survives_its_stored_name() {
    for preset in GroupPreset::ALL {
        let colour = GroupColour::Preset(preset);
        assert_eq!(colour.to_name(), preset.name());
        assert_eq!(GroupColour::parse(&colour.to_name()), Some(colour));
        assert_eq!(GroupPreset::from_name(preset.name()), Some(preset));
        let upper = preset.name().to_ascii_uppercase();
        assert_eq!(GroupColour::parse(&upper), Some(colour), "{upper}");
        let mut title = preset.name().to_owned();
        title[..1].make_ascii_uppercase();
        assert_eq!(GroupColour::parse(&title), Some(colour), "{title}");
    }
}

#[test]
fn a_custom_colour_is_stored_as_lower_case_hex() {
    let custom = GroupColour::Custom(Rgb {
        red: 0x2f,
        green: 0x6f,
        blue: 0xd6,
    });
    assert_eq!(custom.to_name(), "#2f6fd6");
    assert_eq!(GroupColour::parse("#2f6fd6"), Some(custom));
    assert_eq!(GroupColour::parse("#2F6FD6"), Some(custom));
    assert_eq!(GroupColour::parse("#2f6FD6"), Some(custom));
    let black = GroupColour::Custom(Rgb {
        red: 0,
        green: 0,
        blue: 0,
    });
    assert_eq!(black.to_name(), "#000000");
    let white = GroupColour::Custom(Rgb {
        red: 255,
        green: 255,
        blue: 255,
    });
    assert_eq!(white.to_name(), "#ffffff");
    let padded = GroupColour::Custom(Rgb {
        red: 0x0a,
        green: 0x0b,
        blue: 0x0c,
    });
    assert_eq!(padded.to_name(), "#0a0b0c");
    assert_eq!(GroupColour::parse("#0A0B0C"), Some(padded));
}

#[test]
fn every_value_of_every_channel_round_trips() {
    for value in 0..=255u8 {
        for colour in [
            Rgb {
                red: value,
                green: 0,
                blue: 0,
            },
            Rgb {
                red: 0,
                green: value,
                blue: 0,
            },
            Rgb {
                red: 0,
                green: 0,
                blue: value,
            },
            Rgb {
                red: value,
                green: value.wrapping_mul(7),
                blue: 255 - value,
            },
        ] {
            let colour = GroupColour::Custom(colour);
            let name = colour.to_name();
            assert_eq!(name.len(), 7);
            assert_eq!(name, name.to_ascii_lowercase());
            assert_eq!(GroupColour::parse(&name), Some(colour), "{name}");
            assert_eq!(
                GroupColour::parse(&name.to_ascii_uppercase()),
                Some(colour),
                "{name}"
            );
        }
    }
}

#[test]
fn malformed_colours_are_refused() {
    for text in [
        "",
        "#",
        "#12345",
        "#1234567",
        "#GGGGGG",
        "#12345g",
        "2f6fd6",
        "##2f6fd6",
        "#+12345",
        "#-12345",
        "#12 456",
        "#2f6fd6 ",
        " #2f6fd6",
        "#ｆｆｆｆｆｆ",
        "#é2345",
        "#12345é",
        "0x2f6fd6",
        "rgb(1,2,3)",
    ] {
        assert_eq!(GroupColour::parse(text), None, "{text:?}");
    }
}

#[test]
fn unknown_names_and_long_names_are_refused() {
    for text in [
        "purple",
        "blu",
        "bluee",
        "blue2",
        "blue ",
        " blue",
        "bl ue",
        "blue\u{0}",
        "abcdefghijklmnopq",
        "indigoindigoindigo",
        "BLUEISH",
        "slate-grey",
        "é",
    ] {
        assert_eq!(GroupColour::parse(text), None, "{text:?}");
    }
    // 16 letters is the size limit of a stored name, but only a preset name is accepted.
    assert_eq!("abcdefghijklmnop".len(), 16);
    assert_eq!(GroupColour::parse("abcdefghijklmnop"), None);
}

// ----- keys ----------------------------------------------------------------------------------

#[test]
fn every_letter_round_trips_through_its_code_and_letter() {
    let mut seen = HashSet::new();
    for index in 0..26u8 {
        let key = GroupKey::from_index(index).expect("0 to 25 are keys");
        let letter = (b'A' + index) as char;
        assert_eq!(key.letter(), letter);
        assert_eq!(GroupKey::from_letter(letter), Some(key));
        assert_eq!(
            GroupKey::from_letter(letter.to_ascii_lowercase()),
            Some(key)
        );
        assert_eq!(key.code_name(), format!("Key{letter}"));
        assert_eq!(GroupKey::from_code_name(key.code_name()), Some(key));
        assert!(seen.insert(key), "{letter} repeats");
    }
    assert_eq!(seen.len(), 26);
    assert!(GroupKey::from_index(0) < GroupKey::from_index(1));
    assert!(key('z') > key('a'));
}

#[test]
fn only_the_26_letters_are_keys() {
    assert_eq!(GroupKey::from_index(26), None);
    assert_eq!(GroupKey::from_index(255), None);
    for letter in ['é', 'ß', '1', ' ', '\0', '-', 'ａ', 'Ａ', 'ı', '\u{212a}'] {
        assert_eq!(GroupKey::from_letter(letter), None, "{letter:?}");
    }
}

#[test]
fn only_exact_code_names_are_keys() {
    for name in [
        "KeyAA", "keya", "KEYA", "Key", "", "KeyÀ", "KeyA ", " KeyA", "A", "Digit1", "KeyAKeyB",
        "Key1", "KeyÀB", "Keya",
    ] {
        assert_eq!(GroupKey::from_code_name(name), None, "{name:?}");
    }
    assert_eq!(GroupKey::from_code_name("KeyA"), Some(key('a')));
    assert_eq!(GroupKey::from_code_name("KeyZ"), Some(key('z')));
}

#[test]
fn a_key_sequence_keeps_the_first_of_a_repeated_letter() {
    let letters = |text: &str| -> String {
        GroupKey::parse_sequence(text)
            .iter()
            .map(|key| key.letter())
            .collect()
    };
    assert_eq!(letters("ASDFASD"), "ASDF");
    assert_eq!(letters("SADS"), "SAD");
    assert_eq!(letters("asdf"), "ASDF");
    assert_eq!(letters("aAsS"), "AS");
    assert_eq!(letters(""), "");
    assert_eq!(letters("A S,D-F1é"), "ASDF");
    assert_eq!(letters("123 ,.é日"), "");
    assert_eq!(letters(DEFAULT_GROUP_KEY_SEQUENCE), "ASDFGHJKL");
    assert_eq!(
        letters("ABCDEFGHIJKLMNOPQRSTUVWXYZABC"),
        "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    );
    assert_eq!(GroupKey::parse_sequence("zyx").len(), 3);
    assert_eq!(GroupKey::parse_sequence("zyx")[0], key('z'));
}

#[test]
fn a_new_book_has_the_default_key_sequence() {
    assert_eq!(DEFAULT_GROUP_KEY_SEQUENCE, "ASDFGHJKL");
    let expected = GroupKey::parse_sequence(DEFAULT_GROUP_KEY_SEQUENCE);
    assert_eq!(expected.len(), 9);
    assert_eq!(GroupBook::new().key_sequence(), expected.as_slice());
    assert_eq!(GroupBook::default().key_sequence(), expected.as_slice());
    assert!(GroupBook::new().is_empty());
    assert_eq!(GroupBook::new().next_id(), 1);
}

// ----- keys of groups ------------------------------------------------------------------------

#[test]
fn new_groups_take_the_sequence_in_order_and_then_none() {
    let (book, group_ids) = book_of(11);
    let letters: Vec<Option<char>> = group_ids
        .iter()
        .map(|id| key_of(&book, *id).map(GroupKey::letter))
        .collect();
    assert_eq!(
        letters,
        [
            Some('A'),
            Some('S'),
            Some('D'),
            Some('F'),
            Some('G'),
            Some('H'),
            Some('J'),
            Some('K'),
            Some('L'),
            None,
            None
        ]
    );
    assert_book_invariants(&book);
}

#[test]
fn a_new_group_skips_the_letters_that_are_held() {
    let (mut book, group_ids) = book_of(4);
    // Group 1 gives A up for F's holder... simply: group 1 moves from A to Z.
    book.set_key(group_ids[0], Some(key('z')))
        .expect("Z is free");
    let fifth = make(&mut book);
    assert_eq!(key_of(&book, fifth), Some(key('a')), "A was freed");
    let sixth = make(&mut book);
    assert_eq!(key_of(&book, sixth), Some(key('g')), "A S D F are held");
    assert_book_invariants(&book);

    // A letter taken by hand from the sequence is skipped as well.
    let mut book = GroupBook::new();
    let first = make(&mut book);
    book.set_key(first, Some(key('s'))).expect("S is free");
    let second = make(&mut book);
    assert_eq!(key_of(&book, second), Some(key('a')));
    let third = make(&mut book);
    assert_eq!(key_of(&book, third), Some(key('d')), "A and S are held");
}

#[test]
fn removing_a_group_frees_its_key() {
    let (mut book, group_ids) = book_of(3);
    book.remove(group_ids[1]).expect("removed");
    let fresh = make(&mut book);
    assert_eq!(key_of(&book, fresh), Some(key('s')));
    book.remove(group_ids[0]).expect("removed");
    let next = make(&mut book);
    assert_eq!(key_of(&book, next), Some(key('a')));
    assert_book_invariants(&book);
}

#[test]
fn an_empty_sequence_gives_no_keys() {
    let mut book = GroupBook::new();
    book.set_key_sequence(Vec::new());
    assert!(book.key_sequence().is_empty());
    let first = make(&mut book);
    let second = make(&mut book);
    assert_eq!(key_of(&book, first), None);
    assert_eq!(key_of(&book, second), None);
    // Keys can still be given by hand.
    book.set_key(first, Some(key('q'))).expect("Q is free");
    assert_eq!(key_of(&book, first), Some(key('q')));
}

#[test]
fn a_new_sequence_applies_to_new_groups_only() {
    let (mut book, group_ids) = book_of(2);
    book.set_key_sequence(GroupKey::parse_sequence("zx"));
    assert_eq!(key_of(&book, group_ids[0]), Some(key('a')));
    assert_eq!(key_of(&book, group_ids[1]), Some(key('s')));
    let third = make(&mut book);
    let fourth = make(&mut book);
    let fifth = make(&mut book);
    assert_eq!(key_of(&book, third), Some(key('z')));
    assert_eq!(key_of(&book, fourth), Some(key('x')));
    assert_eq!(key_of(&book, fifth), None);
    assert_eq!(
        book.key_sequence(),
        GroupKey::parse_sequence("zx").as_slice()
    );
}

#[test]
fn restore_keeps_a_free_key_and_drops_a_taken_one() {
    let mut book = GroupBook::new();
    let first = book
        .restore(
            RestoredGroup {
                key: Some(key('q')),
                ..RestoredGroup::default()
            },
            at(1),
        )
        .expect("restored");
    assert_eq!(
        key_of(&book, first),
        Some(key('q')),
        "a key outside the sequence is kept"
    );
    let second = book
        .restore(
            RestoredGroup {
                key: Some(key('q')),
                ..RestoredGroup::default()
            },
            at(1),
        )
        .expect("restored");
    assert_eq!(key_of(&book, second), None, "Q is taken");
    assert_eq!(key_of(&book, first), Some(key('q')), "the holder keeps it");
    assert_book_invariants(&book);
}

#[test]
fn restore_never_makes_up_a_key() {
    // A saved group without a key stays without one, so a save and a restore change nothing.
    let mut book = GroupBook::new();
    let first = book
        .restore(RestoredGroup::default(), at(1))
        .expect("restored");
    assert_eq!(key_of(&book, first), None);
    let taken = book
        .restore(
            RestoredGroup {
                key: Some(key('a')),
                ..RestoredGroup::default()
            },
            at(1),
        )
        .expect("restored");
    assert_eq!(key_of(&book, taken), Some(key('a')));
    // A group made afterwards still takes the first free letter of the sequence.
    let created = make(&mut book);
    assert_eq!(key_of(&book, created), Some(key('s')));
}

#[test]
fn set_key_refuses_a_key_another_group_holds() {
    let (mut book, group_ids) = book_of(2);
    assert_eq!(
        book.set_key(group_ids[1], Some(key('a'))),
        Err(GroupError::KeyTaken {
            holder: group_ids[0]
        })
    );
    assert_eq!(key_of(&book, group_ids[0]), Some(key('a')));
    assert_eq!(
        key_of(&book, group_ids[1]),
        Some(key('s')),
        "nothing changed"
    );
    assert_book_invariants(&book);
}

#[test]
fn set_key_to_the_key_a_group_already_has_is_fine() {
    let (mut book, group_ids) = book_of(2);
    assert_eq!(book.set_key(group_ids[0], Some(key('a'))), Ok(()));
    assert_eq!(key_of(&book, group_ids[0]), Some(key('a')));
}

#[test]
fn set_key_moves_a_group_to_a_free_letter_and_frees_the_old_one() {
    let (mut book, group_ids) = book_of(2);
    book.set_key(group_ids[1], Some(key('m')))
        .expect("M is free");
    assert_eq!(key_of(&book, group_ids[1]), Some(key('m')));
    // S is free again.
    book.set_key(group_ids[0], Some(key('s')))
        .expect("S is free");
    assert_eq!(key_of(&book, group_ids[0]), Some(key('s')));
    assert_book_invariants(&book);
}

#[test]
fn set_key_none_frees_the_key() {
    let (mut book, group_ids) = book_of(2);
    book.set_key(group_ids[0], None).expect("cleared");
    assert_eq!(key_of(&book, group_ids[0]), None);
    book.set_key(group_ids[1], Some(key('a')))
        .expect("A is free now");
    assert_eq!(key_of(&book, group_ids[1]), Some(key('a')));
    assert_eq!(
        book.set_key(group_ids[0], None),
        Ok(()),
        "clearing twice is fine"
    );
}

#[test]
fn set_key_on_an_unknown_group_is_not_found() {
    let (mut book, _) = book_of(1);
    let unknown = GroupId::new(40);
    assert_eq!(book.set_key(unknown, None), Err(GroupError::NotFound));
    assert_eq!(
        book.set_key(unknown, Some(key('q'))),
        Err(GroupError::NotFound)
    );
    // An unknown group does not get a taken-key error either.
    assert_eq!(
        book.set_key(unknown, Some(key('a'))),
        Err(GroupError::NotFound)
    );
}

#[test]
fn swap_key_gives_the_holder_the_old_key() {
    let (mut book, group_ids) = book_of(3);
    assert_eq!(book.swap_key(group_ids[1], key('a')), Ok(()));
    assert_eq!(key_of(&book, group_ids[1]), Some(key('a')));
    assert_eq!(
        key_of(&book, group_ids[0]),
        Some(key('s')),
        "the holder got S"
    );
    assert_eq!(
        key_of(&book, group_ids[2]),
        Some(key('d')),
        "others are untouched"
    );
    assert_book_invariants(&book);
}

#[test]
fn swap_key_gives_the_holder_nothing_when_the_requester_had_no_key() {
    let (mut book, group_ids) = book_of(2);
    book.set_key(group_ids[1], None).expect("cleared");
    assert_eq!(book.swap_key(group_ids[1], key('a')), Ok(()));
    assert_eq!(key_of(&book, group_ids[1]), Some(key('a')));
    assert_eq!(key_of(&book, group_ids[0]), None);
    assert_book_invariants(&book);
}

#[test]
fn swap_key_with_a_free_key_is_set_key() {
    let (mut book, group_ids) = book_of(2);
    assert_eq!(book.swap_key(group_ids[0], key('z')), Ok(()));
    assert_eq!(key_of(&book, group_ids[0]), Some(key('z')));
    assert_eq!(key_of(&book, group_ids[1]), Some(key('s')));
    // A was freed, not handed to anybody.
    let fresh = make(&mut book);
    assert_eq!(key_of(&book, fresh), Some(key('a')));
    assert_book_invariants(&book);
}

#[test]
fn swap_key_with_ones_own_key_changes_nothing() {
    let (mut book, group_ids) = book_of(2);
    assert_eq!(book.swap_key(group_ids[0], key('a')), Ok(()));
    assert_eq!(key_of(&book, group_ids[0]), Some(key('a')));
    assert_eq!(key_of(&book, group_ids[1]), Some(key('s')));
}

#[test]
fn swap_key_on_an_unknown_group_is_not_found_and_changes_nothing() {
    let (mut book, group_ids) = book_of(2);
    assert_eq!(
        book.swap_key(GroupId::new(50), key('a')),
        Err(GroupError::NotFound)
    );
    assert_eq!(key_of(&book, group_ids[0]), Some(key('a')));
    assert_eq!(key_of(&book, group_ids[1]), Some(key('s')));
}

// ----- limits --------------------------------------------------------------------------------

#[test]
fn a_window_holds_at_most_64_groups() {
    assert_eq!(MAX_GROUPS, 64);
    let (mut book, group_ids) = book_of(MAX_GROUPS);
    assert_eq!(book.len(), 64);
    let next_before = book.next_id();
    assert_eq!(
        book.create(Some("one too many"), None, at(1)),
        Err(GroupError::TooManyGroups)
    );
    assert_eq!(
        book.restore(RestoredGroup::default(), at(1)),
        Err(GroupError::TooManyGroups)
    );
    assert_eq!(
        book.restore(
            RestoredGroup {
                id_hint: Some(5000),
                ..RestoredGroup::default()
            },
            at(1)
        ),
        Err(GroupError::TooManyGroups)
    );
    assert_eq!(book.len(), 64, "a refusal changes nothing");
    assert_eq!(book.next_id(), next_before, "and uses no id");
    assert_book_invariants(&book);

    // Taking one out makes room for exactly one.
    book.remove(group_ids[10]).expect("removed");
    let again = make(&mut book);
    assert_eq!(book.len(), 64);
    assert_eq!(
        again.value(),
        next_before,
        "the new group has the next id, not the freed one"
    );
    assert_eq!(make_err(&mut book), GroupError::TooManyGroups);
    assert_book_invariants(&book);
}

fn make_err(book: &mut GroupBook) -> GroupError {
    book.create(None, None, at(1))
        .expect_err("the book is full")
}

#[test]
fn the_default_name_of_the_64th_group_is_group_64() {
    let (book, group_ids) = book_of(MAX_GROUPS);
    assert_eq!(name_of(&book, group_ids[63]), "Group 64");
    assert_eq!(name_of(&book, group_ids[0]), "Group 1");
}

// ----- ids -----------------------------------------------------------------------------------

#[test]
fn ids_count_up_from_one() {
    assert_eq!(GroupId::new(7).value(), 7);
    assert!(GroupId::new(2) > GroupId::new(1));
    let mut book = GroupBook::new();
    assert_eq!(book.next_id(), 1);
    let first = make(&mut book);
    let second = make(&mut book);
    let third = make(&mut book);
    assert_eq!([first.value(), second.value(), third.value()], [1, 2, 3]);
    assert_eq!(book.next_id(), 4);
    assert_eq!(MAX_GROUP_ID, i32::MAX as u64);
}

#[test]
fn an_id_is_never_used_twice() {
    let mut book = GroupBook::new();
    let first = make(&mut book);
    let second = make(&mut book);
    book.remove(second).expect("removed");
    let third = make(&mut book);
    assert_eq!(third.value(), 3);
    book.remove(first).expect("removed");
    book.remove(third).expect("removed");
    assert!(book.is_empty());
    let fourth = make(&mut book);
    assert_eq!(fourth.value(), 4, "an empty book goes on counting");
    assert_eq!(book.next_id(), 5);
}

#[test]
fn restore_honours_a_free_id() {
    let mut book = GroupBook::new();
    let restored = book
        .restore(
            RestoredGroup {
                id_hint: Some(10),
                ..RestoredGroup::default()
            },
            at(1),
        )
        .expect("restored");
    assert_eq!(restored.value(), 10);
    assert_eq!(
        book.next_id(),
        11,
        "ids below the hint are never handed out afterwards"
    );
    let created = make(&mut book);
    assert_eq!(created.value(), 11);
    assert_book_invariants(&book);
}

#[test]
fn restore_honours_an_id_that_was_used_and_freed() {
    // Closing a group and reopening it gives it its old id back.
    let (mut book, group_ids) = book_of(4);
    let closed = group_ids[1];
    book.remove(closed).expect("removed");
    let next_before = book.next_id();
    let reopened = book
        .restore(
            RestoredGroup {
                id_hint: Some(closed.value()),
                ..RestoredGroup::default()
            },
            at(1),
        )
        .expect("restored");
    assert_eq!(reopened, closed);
    assert_eq!(
        book.next_id(),
        next_before,
        "the counter does not go back or forward"
    );
    assert_book_invariants(&book);
}

#[test]
fn restore_ignores_a_taken_id() {
    let (mut book, group_ids) = book_of(2);
    let restored = book
        .restore(
            RestoredGroup {
                id_hint: Some(group_ids[0].value()),
                name: Some("Other".to_owned()),
                ..RestoredGroup::default()
            },
            at(1),
        )
        .expect("restored");
    assert_eq!(restored.value(), 3, "a fresh id");
    assert_ne!(restored, group_ids[0]);
    assert_eq!(
        name_of(&book, group_ids[0]),
        "Group 1",
        "the holder is untouched"
    );
    assert_eq!(name_of(&book, restored), "Other");
    assert_book_invariants(&book);
}

#[test]
fn restore_ignores_an_id_too_large_for_slint() {
    for hint in [MAX_GROUP_ID + 1, MAX_GROUP_ID + 2, u64::MAX, 1 << 40] {
        let mut book = GroupBook::new();
        let restored = book
            .restore(
                RestoredGroup {
                    id_hint: Some(hint),
                    ..RestoredGroup::default()
                },
                at(1),
            )
            .expect("restored");
        assert_eq!(restored.value(), 1, "hint {hint}");
        assert_eq!(book.next_id(), 2, "hint {hint}");
        assert_book_invariants(&book);
    }
}

#[test]
fn restore_ignores_id_zero() {
    let mut book = GroupBook::new();
    let restored = book
        .restore(
            RestoredGroup {
                id_hint: Some(0),
                ..RestoredGroup::default()
            },
            at(1),
        )
        .expect("restored");
    assert_eq!(restored.value(), 1, "ids start at 1");
}

#[test]
fn restore_without_a_hint_takes_the_next_id() {
    let mut book = GroupBook::new();
    book.raise_next_id(30);
    let restored = book
        .restore(RestoredGroup::default(), at(1))
        .expect("restored");
    assert_eq!(restored.value(), 30);
    assert_eq!(book.next_id(), 31);
}

#[test]
fn the_largest_id_is_honoured_and_then_ids_run_out() {
    let mut book = GroupBook::new();
    let last = book
        .restore(
            RestoredGroup {
                id_hint: Some(MAX_GROUP_ID),
                ..RestoredGroup::default()
            },
            at(1),
        )
        .expect("the largest id is allowed");
    assert_eq!(last.value(), MAX_GROUP_ID);
    assert_eq!(
        book.create(None, None, at(1)),
        Err(GroupError::IdsExhausted)
    );
    assert_eq!(
        book.restore(RestoredGroup::default(), at(1)),
        Err(GroupError::IdsExhausted)
    );
    // A hint that is free and in range still works: ids below the top are not all used.
    let low = book
        .restore(
            RestoredGroup {
                id_hint: Some(5),
                ..RestoredGroup::default()
            },
            at(1),
        )
        .expect("a free hint needs no counter");
    assert_eq!(low.value(), 5);
    assert_eq!(book.len(), 2, "the refusals changed nothing");
}

#[test]
fn raise_next_id_only_raises_and_stops_at_the_limit() {
    let mut book = GroupBook::new();
    book.raise_next_id(0);
    assert_eq!(book.next_id(), 1);
    book.raise_next_id(100);
    assert_eq!(book.next_id(), 100);
    book.raise_next_id(50);
    assert_eq!(book.next_id(), 100, "never lowered");
    book.raise_next_id(u64::MAX);
    assert_eq!(book.next_id(), MAX_GROUP_ID, "clamped");
    book.raise_next_id(MAX_GROUP_ID + 1);
    assert_eq!(book.next_id(), MAX_GROUP_ID);
}

#[test]
fn the_last_id_is_usable_and_the_next_create_is_refused() {
    let mut book = GroupBook::new();
    book.raise_next_id(u64::MAX);
    let last = make(&mut book);
    assert_eq!(last.value(), MAX_GROUP_ID);
    assert_eq!(
        book.create(Some("late"), None, at(1)),
        Err(GroupError::IdsExhausted)
    );
    assert_eq!(book.len(), 1, "nothing was added");
    assert_eq!(names(&book), ["Group 1"]);
    // Removing the group does not make its id available to create.
    book.remove(last).expect("removed");
    assert_eq!(make_err_ids(&mut book), GroupError::IdsExhausted);
}

fn make_err_ids(book: &mut GroupBook) -> GroupError {
    book.create(None, None, at(1)).expect_err("ids are used up")
}

// ----- timestamps ----------------------------------------------------------------------------

#[test]
fn create_and_restore_stamp_both_times() {
    let mut book = GroupBook::new();
    let created = book.create(None, None, at(1_000)).expect("created");
    let restored = book
        .restore(RestoredGroup::default(), at(2_000))
        .expect("restored");
    let first = book.get(created).expect("exists");
    assert_eq!((first.created, first.last_active), (at(1_000), at(1_000)));
    let second = book.get(restored).expect("exists");
    assert_eq!((second.created, second.last_active), (at(2_000), at(2_000)));
}

#[test]
fn stamp_activity_stamps_the_touched_groups_and_the_active_one() {
    let (mut book, group_ids) = book_of(4);
    let before: Vec<_> = book.as_slice().to_vec();
    book.touch(group_ids[0]);
    book.touch(group_ids[0]);
    book.touch(group_ids[2]);
    book.stamp_activity(at(200), Some(group_ids[3]));
    for (index, expected) in [(0, 200), (1, 100), (2, 200), (3, 200)] {
        let group = book.get(group_ids[index]).expect("exists");
        assert_eq!(group.last_active, at(expected), "group {index}");
        let old = &before[index];
        assert_eq!(group.created, old.created, "created is never restamped");
        assert_eq!(
            (&group.name, group.colour, group.key, group.id),
            (&old.name, old.colour, old.key, old.id)
        );
    }
    assert_eq!(ids(&book), group_ids, "the order is untouched");
}

#[test]
fn the_touches_are_forgotten_after_a_stamp() {
    let (mut book, group_ids) = book_of(3);
    book.touch(group_ids[1]);
    book.stamp_activity(at(200), None);
    assert_eq!(book.get(group_ids[1]).expect("exists").last_active, at(200));
    book.stamp_activity(at(300), None);
    for id in &group_ids {
        let stamped = book.get(*id).expect("exists").last_active;
        assert_ne!(stamped, at(300), "nothing was touched since");
    }
    assert_eq!(book.get(group_ids[1]).expect("exists").last_active, at(200));
    // Only the active group moves on the next stamp.
    book.stamp_activity(at(400), Some(group_ids[2]));
    assert_eq!(book.get(group_ids[2]).expect("exists").last_active, at(400));
    assert_eq!(book.get(group_ids[1]).expect("exists").last_active, at(200));
    assert_eq!(book.get(group_ids[0]).expect("exists").last_active, at(100));
}

#[test]
fn a_stamp_with_nothing_to_do_changes_nothing() {
    let (mut book, _) = book_of(3);
    let before: Vec<_> = book.as_slice().to_vec();
    book.stamp_activity(at(500), None);
    assert_eq!(book.as_slice(), before.as_slice());
}

#[test]
fn touching_or_stamping_a_group_that_is_gone_is_harmless() {
    let (mut book, group_ids) = book_of(3);
    book.touch(group_ids[1]);
    book.touch(GroupId::new(900));
    book.remove(group_ids[1]).expect("removed");
    book.stamp_activity(at(500), Some(GroupId::new(901)));
    assert_eq!(book.len(), 2);
    for group in book.as_slice() {
        assert_eq!(group.last_active, at(100));
    }
    // A removed group's old touch does not follow a new group that cannot have its id.
    let fresh = make(&mut book);
    book.stamp_activity(at(600), None);
    assert_eq!(book.get(fresh).expect("exists").last_active, at(100));
    assert_book_invariants(&book);
}

// ----- ordering ------------------------------------------------------------------------------

#[test]
fn reorder_puts_the_named_groups_first_in_the_given_order() {
    let (mut book, g) = book_of(4);
    book.reorder_to_match(&[g[2], g[0]]);
    assert_eq!(ids(&book), [g[2], g[0], g[1], g[3]]);
    // Everything named: the whole order is the given one.
    book.reorder_to_match(&[g[3], g[2], g[1], g[0]]);
    assert_eq!(ids(&book), [g[3], g[2], g[1], g[0]]);
    // Names and keys came along with their groups.
    assert_eq!(name_of(&book, g[0]), "Group 1");
    assert_eq!(key_of(&book, g[0]), Some(key('a')));
    assert_book_invariants(&book);
}

#[test]
fn reorder_keeps_the_relative_order_of_the_groups_it_does_not_name() {
    let (mut book, g) = book_of(5);
    book.reorder_to_match(&[g[3]]);
    assert_eq!(ids(&book), [g[3], g[0], g[1], g[2], g[4]]);
    book.reorder_to_match(&[g[1], g[4]]);
    assert_eq!(ids(&book), [g[1], g[4], g[3], g[0], g[2]]);
}

#[test]
fn reorder_ignores_unknown_ids_and_repeats() {
    let (mut book, g) = book_of(4);
    book.reorder_to_match(&[g[3], GroupId::new(99), g[1], GroupId::new(1000)]);
    assert_eq!(ids(&book), [g[3], g[1], g[0], g[2]]);
    book.reorder_to_match(&[g[2], g[2], g[0], g[2]]);
    assert_eq!(ids(&book), [g[2], g[0], g[3], g[1]]);
    assert_eq!(book.len(), 4, "nothing was lost or doubled");
}

#[test]
fn reorder_with_nothing_to_say_changes_nothing() {
    let (mut book, g) = book_of(3);
    book.reorder_to_match(&[]);
    assert_eq!(ids(&book), g);
    book.reorder_to_match(&[GroupId::new(77)]);
    assert_eq!(ids(&book), g);
    let mut empty = GroupBook::new();
    empty.reorder_to_match(&[GroupId::new(1)]);
    assert!(empty.is_empty());
}

#[test]
fn remove_returns_the_group_and_forgets_it() {
    let (mut book, g) = book_of(3);
    let removed = book.remove(g[1]).expect("it was there");
    assert_eq!(removed.id, g[1]);
    assert_eq!(removed.name, "Group 2");
    assert_eq!(removed.key, Some(key('s')));
    assert_eq!(book.get(g[1]), None);
    assert_eq!(ids(&book), [g[0], g[2]], "the others keep their order");
    assert_eq!(book.remove(g[1]), None, "nothing to remove twice");
    assert_eq!(book.remove(GroupId::new(500)), None);
    assert_eq!(book.len(), 2);
}

// ----- a book that is treated roughly ----------------------------------------------------------

/// A fixed-seed xorshift: the same sequence on every run.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

#[test]
fn the_rules_hold_over_thousands_of_random_operations() {
    let mut random = Xorshift(0x9e37_79b9_7f4a_7c15);
    let mut book = GroupBook::new();
    let mut ever_used = HashSet::new();
    let letters = ['a', 's', 'd', 'f', 'q', 'z'];
    let colours = [
        blue(),
        GroupColour::Preset(GroupPreset::Brown),
        GroupColour::Custom(Rgb {
            red: 1,
            green: 2,
            blue: 3,
        }),
    ];
    for step in 0..4_000u64 {
        let now = at(step);
        let pick = |random: &mut Xorshift, book: &GroupBook| -> GroupId {
            if book.is_empty() || random.below(8) == 0 {
                GroupId::new(random.below(80) as u64)
            } else {
                book.as_slice()[random.below(book.len())].id
            }
        };
        match random.below(12) {
            0 | 1 => {
                if let Ok(id) = book.create(None, None, now) {
                    // Only a created group is new; a restored one may bring back an old id.
                    ever_used.insert(id);
                }
            }
            2 => {
                let name = ["", "  ", "x", "Group 3", "é\n"][random.below(5)];
                let _ = book.create(Some(name), Some(colours[random.below(3)]), now);
            }
            3 => {
                let hint = [
                    None,
                    Some(0),
                    Some(3),
                    Some(random.below(100) as u64),
                    Some(u64::MAX),
                ][random.below(5)];
                let spec = RestoredGroup {
                    id_hint: hint,
                    name: Some("saved".to_owned()),
                    colour: None,
                    key: GroupKey::from_letter(letters[random.below(6)]),
                };
                let _ = book.restore(spec, now);
            }
            4 | 5 => {
                let id = pick(&mut random, &book);
                book.remove(id);
            }
            6 => {
                let id = pick(&mut random, &book);
                let _ = book.rename(id, ["", "Home", "  Work "][random.below(3)]);
            }
            7 => {
                let id = pick(&mut random, &book);
                let _ = book.recolour(id, colours[random.below(3)]);
            }
            8 => {
                let id = pick(&mut random, &book);
                let key = if random.below(4) == 0 {
                    None
                } else {
                    GroupKey::from_letter(letters[random.below(6)])
                };
                let _ = book.set_key(id, key);
            }
            9 => {
                let id = pick(&mut random, &book);
                let _ = book.swap_key(id, key(letters[random.below(6)]));
            }
            10 => {
                let order: Vec<GroupId> = (0..random.below(5))
                    .map(|_| pick(&mut random, &book))
                    .collect();
                let before: HashSet<GroupId> = ids(&book).into_iter().collect();
                book.reorder_to_match(&order);
                let after: HashSet<GroupId> = ids(&book).into_iter().collect();
                assert_eq!(before, after, "reordering neither adds nor drops groups");
            }
            _ => {
                let id = pick(&mut random, &book);
                book.touch(id);
                let active = Some(pick(&mut random, &book));
                book.stamp_activity(now, active);
            }
        }
        assert_book_invariants(&book);
    }
    assert!(!ever_used.is_empty());
}

// ----- tab status and group summary ----------------------------------------------------------

fn status() -> TabStatus {
    TabStatus::default()
}

#[test]
fn no_tabs_give_an_empty_summary() {
    let summary = GroupSummary::from_statuses(Vec::<TabStatus>::new());
    assert_eq!(summary, GroupSummary::default());
    assert_eq!(summary.tabs, 0);
    assert!(!summary.any_loading && !summary.any_unread && !summary.any_capture);
}

#[test]
fn quiet_tabs_set_no_flag() {
    let summary = GroupSummary::from_statuses([status(), status(), status()]);
    assert_eq!(
        summary,
        GroupSummary {
            tabs: 3,
            ..GroupSummary::default()
        }
    );
}

#[test]
fn the_summary_counts_and_flags_what_the_tabs_show() {
    let summary = GroupSummary::from_statuses([
        TabStatus {
            loading: true,
            unread: true,
            ..status()
        },
        TabStatus {
            loading: true,
            audible: true,
            ..status()
        },
        TabStatus {
            unread: true,
            crashed: true,
            ..status()
        },
        status(),
    ]);
    assert_eq!(
        summary,
        GroupSummary {
            tabs: 4,
            loading_tabs: 2,
            unread_tabs: 2,
            any_loading: true,
            any_audible: true,
            any_unread: true,
            any_crashed: true,
            any_capture: false,
        }
    );
}

#[test]
fn each_flag_sets_only_its_own_summary_flag() {
    let flag = |tab: TabStatus| GroupSummary::from_statuses([tab, status()]);
    let loading = flag(TabStatus {
        loading: true,
        ..status()
    });
    assert_eq!(
        (loading.tabs, loading.loading_tabs, loading.unread_tabs),
        (2, 1, 0)
    );
    assert!(loading.any_loading && !loading.any_audible && !loading.any_unread);

    let audible = flag(TabStatus {
        audible: true,
        ..status()
    });
    assert!(audible.any_audible && !audible.any_loading && !audible.any_crashed);

    let unread = flag(TabStatus {
        unread: true,
        ..status()
    });
    assert_eq!((unread.unread_tabs, unread.any_unread), (1, true));
    assert!(!unread.any_loading && !unread.any_capture);

    let crashed = flag(TabStatus {
        crashed: true,
        ..status()
    });
    assert!(crashed.any_crashed && !crashed.any_unread);

    // Muted and pinned are shown on the tab, not on the group.
    let quiet = flag(TabStatus {
        muted: true,
        pinned: true,
        ..status()
    });
    assert_eq!(
        quiet,
        GroupSummary {
            tabs: 2,
            ..GroupSummary::default()
        }
    );
}

#[test]
fn camera_or_microphone_sets_the_capture_flag() {
    for capture in [
        CaptureAccess {
            camera: true,
            microphone: false,
        },
        CaptureAccess {
            camera: false,
            microphone: true,
        },
        CaptureAccess {
            camera: true,
            microphone: true,
        },
    ] {
        let summary = GroupSummary::from_statuses([TabStatus {
            capture,
            ..status()
        }]);
        assert!(summary.any_capture, "{capture:?}");
    }
    let none = GroupSummary::from_statuses([TabStatus {
        capture: CaptureAccess::default(),
        ..status()
    }]);
    assert!(!none.any_capture);
}

#[test]
fn the_summary_takes_any_iterator() {
    let from_iterator = GroupSummary::from_statuses((0..5).map(|index| TabStatus {
        loading: index % 2 == 0,
        ..status()
    }));
    assert_eq!((from_iterator.tabs, from_iterator.loading_tabs), (5, 3));
    let from_vec = GroupSummary::from_statuses(vec![status(); 7]);
    assert_eq!(from_vec.tabs, 7);
}

// ----- load progress -------------------------------------------------------------------------

#[test]
fn progress_starts_at_zero() {
    assert_eq!(load_progress_estimate(Duration::ZERO), 0);
    assert_eq!(load_progress_estimate(Duration::from_nanos(1)), 0);
    assert_eq!(load_progress_estimate(Duration::from_millis(1)), 0);
}

#[test]
fn progress_never_goes_backwards_and_never_passes_90() {
    let mut last = 0;
    for step in 0..=6_000u64 {
        let elapsed = Duration::from_millis(step * 10);
        let now = load_progress_estimate(elapsed);
        assert!(now >= last, "went from {last} to {now} at {elapsed:?}");
        assert!(now <= 90, "{now} at {elapsed:?}");
        last = now;
    }
    assert_eq!(last, 90, "a minute is plenty to reach the ceiling");
}

#[test]
fn progress_stays_at_or_below_90_for_any_duration() {
    for elapsed in [
        Duration::from_secs(3_600),
        Duration::from_secs(86_400 * 365),
        Duration::from_secs(u64::MAX),
        Duration::new(u64::MAX, 999_999_999),
        Duration::MAX,
    ] {
        let progress = load_progress_estimate(elapsed);
        assert!(progress <= 90, "{progress} at {elapsed:?}");
        assert!(progress >= 89, "the curve has settled at {elapsed:?}");
    }
    assert_eq!(load_progress_estimate(Duration::MAX), 90);
}

#[test]
fn progress_rises_quickly_at_first_then_slows_down() {
    let at_ms = |millis| load_progress_estimate(Duration::from_millis(millis));
    assert!(at_ms(100) > 0, "something shows within a tenth of a second");
    assert!(at_ms(500) < at_ms(1_500));
    assert!(at_ms(1_500) < at_ms(5_000));
    assert!(at_ms(5_000) <= at_ms(20_000));
    let first_second = at_ms(1_000) - at_ms(0);
    let second_second = at_ms(2_000) - at_ms(1_000);
    let third_second = at_ms(3_000) - at_ms(2_000);
    assert!(first_second > second_second && second_second > third_second);
    assert!((30..=70).contains(&at_ms(1_000)), "{}", at_ms(1_000));
}
