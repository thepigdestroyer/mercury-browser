// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The one-letter ⌥ group keys: the letters, the option-character table, restore of a stored
//! list, assigning, swapping, adding and removing groups, ring order, and the dead-key note.

use std::collections::HashSet;

use mercury_appearance::keymap::{
    Assign, DeadKey, DeadKeyTable, KeyIssue, KeyMap, LetterKey, OptionChar, dead_key_note,
    parse_sequence,
};
use mercury_settings::{DEFAULT_KEY_SEQUENCE, Settings};

/// A key from its capital letter.
fn key(letter: char) -> LetterKey {
    LetterKey::from_char(letter).expect("a letter")
}

/// A sequence from capitals, with no checking of its own.
fn seq(letters: &str) -> Vec<LetterKey> {
    letters.chars().map(key).collect()
}

/// A map of the given groups with the given keys, built through the public restore.
fn map_of(entries: &[(u64, Option<char>)]) -> KeyMap {
    let names: Vec<(u64, Option<String>)> = entries
        .iter()
        .map(|&(group, letter)| (group, letter.map(|l| key(l).code_name().to_owned())))
        .collect();
    let (map, issues) = KeyMap::restore(names.iter().map(|(g, name)| (*g, name.as_deref())));
    assert!(issues.is_empty(), "test map must be clean: {issues:?}");
    map
}

/// The keys of a map in ring order, as letters (`-` for none), for short assertions.
fn shape(map: &KeyMap) -> Vec<(u64, char)> {
    map.entries()
        .iter()
        .map(|&(group, key)| (group, key.map_or('-', LetterKey::letter)))
        .collect()
}

// ---- LetterKey ----------------------------------------------------------------------------

#[test]
fn all_is_the_26_letters_in_order() {
    assert_eq!(LetterKey::ALL.len(), 26);
    let letters: String = LetterKey::ALL.iter().map(|k| k.letter()).collect();
    assert_eq!(letters, "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    for (index, letter) in LetterKey::ALL.iter().enumerate() {
        assert_eq!(letter.index(), index);
        assert_eq!(LetterKey::from_index(index), Some(*letter));
    }
    assert_eq!(LetterKey::from_index(26), None);
    assert_eq!(LetterKey::from_index(usize::MAX), None);
    assert!(LetterKey::A < LetterKey::B && LetterKey::Y < LetterKey::Z);
    assert_eq!(LetterKey::Q.to_string(), "Q");
}

#[test]
fn from_char_takes_the_ascii_letters_in_either_case() {
    for (index, upper) in ('A'..='Z').enumerate() {
        let lower = upper.to_ascii_lowercase();
        assert_eq!(LetterKey::from_char(upper), LetterKey::from_index(index));
        assert_eq!(LetterKey::from_char(lower), LetterKey::from_index(index));
    }
    for other in [
        '0', '9', ' ', '`', '@', '[', '{', '-', '\u{e9}', '\u{df}', '\u{ff21}', '\u{3a9}', '\0',
    ] {
        assert_eq!(LetterKey::from_char(other), None, "{other:?}");
    }
}

#[test]
fn from_code_name_takes_exactly_key_a_to_key_z() {
    let accepted = [
        ("KeyA", Some(LetterKey::A)),
        ("KeyM", Some(LetterKey::M)),
        ("KeyZ", Some(LetterKey::Z)),
    ];
    for (name, expected) in accepted {
        assert_eq!(LetterKey::from_code_name(name), expected, "{name}");
    }
    let refused = [
        "",
        "Key",
        "KeyAA",
        "KeyA ",
        " KeyA",
        "keya",
        "KEYA",
        "Keya",
        "KeyDigit1",
        "Digit1",
        "A",
        "a",
        "KeyÄ",
        "Key1",
        "KeyAltLeft",
        "Space",
        "ShiftLeft",
        "KeyA\n",
        "Key\u{ff21}",
    ];
    for name in refused {
        assert_eq!(LetterKey::from_code_name(name), None, "{name:?}");
    }
}

#[test]
fn all_26_letters_round_trip_through_the_code_name() {
    let mut names = HashSet::new();
    for letter in LetterKey::ALL {
        let name = letter.code_name();
        assert_eq!(name, format!("Key{}", letter.letter()));
        assert_eq!(LetterKey::from_code_name(name), Some(letter));
        assert_eq!(LetterKey::from_char(letter.letter()), Some(letter));
        assert!(names.insert(name), "{name} appears twice");
    }
    assert_eq!(names.len(), 26);
}

// ---- The ⌥ character table ----------------------------------------------------------------

#[test]
fn the_option_character_table_has_26_entries_and_four_dead_keys() {
    let entries: Vec<(LetterKey, OptionChar)> = LetterKey::ALL
        .iter()
        .map(|&letter| (letter, letter.us_option_char()))
        .collect();
    assert_eq!(entries.len(), 26);

    let dead: Vec<char> = entries
        .iter()
        .filter(|(_, option)| option.is_dead())
        .map(|(letter, _)| letter.letter())
        .collect();
    assert_eq!(dead, vec!['E', 'I', 'N', 'U']);

    // No two keys give the same glyph.
    let glyphs: HashSet<char> = entries.iter().map(|(_, option)| option.glyph()).collect();
    assert_eq!(glyphs.len(), 26);
}

#[test]
fn the_option_character_table_is_the_us_layout() {
    let table = [
        ('A', OptionChar::Char('å')),
        ('B', OptionChar::Char('∫')),
        ('C', OptionChar::Char('ç')),
        ('D', OptionChar::Char('∂')),
        ('E', OptionChar::Dead('´')),
        ('F', OptionChar::Char('ƒ')),
        ('G', OptionChar::Char('©')),
        ('H', OptionChar::Char('˙')),
        ('I', OptionChar::Dead('ˆ')),
        ('J', OptionChar::Char('∆')),
        ('K', OptionChar::Char('˚')),
        ('L', OptionChar::Char('¬')),
        ('M', OptionChar::Char('µ')),
        ('N', OptionChar::Dead('˜')),
        ('O', OptionChar::Char('ø')),
        ('P', OptionChar::Char('π')),
        ('Q', OptionChar::Char('œ')),
        ('R', OptionChar::Char('®')),
        ('S', OptionChar::Char('ß')),
        ('T', OptionChar::Char('†')),
        ('U', OptionChar::Dead('¨')),
        ('V', OptionChar::Char('√')),
        ('W', OptionChar::Char('∑')),
        ('X', OptionChar::Char('≈')),
        ('Y', OptionChar::Char('¥')),
        ('Z', OptionChar::Char('Ω')),
    ];
    assert_eq!(table.len(), 26);
    for (letter, expected) in table {
        assert_eq!(key(letter).us_option_char(), expected, "Option+{letter}");
    }
}

#[test]
fn look_alike_option_characters_are_the_ones_macos_types() {
    // The pairs that are easy to confuse: increment sign and Greek delta, micro sign and Greek
    // mu, Ohm sign and Greek omega. macOS types the first of each pair on Option+J, M and Z
    // (the last pair is the capital omega U+03A9).
    assert_eq!(key('J').us_option_char().glyph() as u32, 0x2206);
    assert_eq!(key('M').us_option_char().glyph() as u32, 0x00b5);
    assert_eq!(key('Z').us_option_char().glyph() as u32, 0x03a9);
    assert_eq!(key('E').us_option_char().glyph() as u32, 0x00b4);
    assert_eq!(key('I').us_option_char().glyph() as u32, 0x02c6);
    assert_eq!(key('N').us_option_char().glyph() as u32, 0x02dc);
    assert_eq!(key('U').us_option_char().glyph() as u32, 0x00a8);
}

#[test]
fn option_char_reports_its_glyph_and_whether_it_is_dead() {
    assert_eq!(OptionChar::Char('ß').glyph(), 'ß');
    assert!(!OptionChar::Char('ß').is_dead());
    assert_eq!(OptionChar::Dead('´').glyph(), '´');
    assert!(OptionChar::Dead('´').is_dead());
}

// ---- parse_sequence -----------------------------------------------------------------------

#[test]
fn parse_sequence_keeps_order_drops_repeats_and_skips_non_letters() {
    let cases: [(&str, &str); 12] = [
        ("ASDFGHJKL", "ASDFGHJKL"),
        ("", ""),
        ("asdf", "ASDF"),
        ("AaSs", "AS"),
        ("QWERTY", "QWERTY"),
        ("A S D", "ASD"),
        ("A1S2D3", "ASD"),
        ("ÄASÖ", "AS"),
        ("a,b;c", "ABC"),
        ("1234", ""),
        ("ZZZZ", "Z"),
        (
            "ABCDEFGHIJKLMNOPQRSTUVWXYZABC",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(parse_sequence(text), seq(expected), "{text:?}");
    }
}

#[test]
fn the_default_sequence_parses_to_the_home_row() {
    let parsed = parse_sequence(DEFAULT_KEY_SEQUENCE);
    assert_eq!(parsed, seq("ASDFGHJKL"));
    assert_eq!(parsed.len(), 9);
}

#[test]
fn parse_sequence_agrees_with_what_settings_repairs_a_sequence_to() {
    for text in [
        "ASDFGHJKL",
        "asdfghjkl",
        "SDAF",
        "A S D",
        "AASSDD",
        "A1B2",
        "xyz!",
        "ÄÖA",
    ] {
        let mut settings = Settings::default();
        settings.appearance.tab_groups.keys.default_sequence = text.to_owned();
        let (repaired, _) = settings.validated();
        let stored = repaired.appearance.tab_groups.keys.default_sequence;
        let spelled: String = parse_sequence(text).iter().map(|k| k.letter()).collect();
        assert_eq!(stored, spelled, "{text:?}");
        // A repaired sequence parses to exactly what it spells, so reading it again is stable.
        assert_eq!(parse_sequence(&stored), parse_sequence(text), "{text:?}");
    }
}

// ---- Dead keys ----------------------------------------------------------------------------

#[test]
fn the_us_dead_key_table_is_e_i_n_u() {
    let table = DeadKeyTable::us();
    let keys: Vec<char> = table.keys().map(LetterKey::letter).collect();
    assert_eq!(keys, vec!['E', 'I', 'N', 'U']);
    assert_eq!(table.accent(key('E')), Some('´'));
    assert_eq!(table.accent(key('I')), Some('ˆ'));
    assert_eq!(table.accent(key('N')), Some('˜'));
    assert_eq!(table.accent(key('U')), Some('¨'));
    assert_eq!(table.accent(key('A')), None);
    assert_eq!(DeadKeyTable::default(), table);
    assert_eq!(DeadKeyTable::empty().keys().count(), 0);
}

#[test]
fn a_dead_key_is_allowed_with_a_note() {
    let table = DeadKeyTable::us();
    for letter in ['E', 'I', 'N', 'U'] {
        let note = dead_key_note(key(letter), &table);
        assert_eq!(
            note,
            Some(DeadKey {
                accent: key(letter).us_option_char().glyph()
            }),
            "{letter}"
        );
    }
    for letter in ['A', 'S', 'D', 'F', 'G', 'H', 'J', 'K', 'L', 'Q', 'Z'] {
        assert_eq!(dead_key_note(key(letter), &table), None, "{letter}");
    }

    // Allowed: a group can hold a dead key like any other.
    let mut map = map_of(&[(1, None)]);
    assert_eq!(map.assign(1, Some(key('E'))), Assign::Assigned);
    assert_eq!(map.key_of(1), Some(key('E')));
}

#[test]
fn what_the_app_learns_changes_the_dead_keys() {
    let learnt = DeadKeyTable::us().with_learnt([
        (key('Q'), OptionChar::Dead('`')), // a layout where Option+Q is dead
        (key('E'), OptionChar::Char('€')), // a layout where Option+E types a character
        (key('U'), OptionChar::Dead('\u{2dd}')), // a different accent replaces the old one
        (key('A'), OptionChar::Char('å')), // already not dead: nothing changes
    ]);
    let keys: Vec<char> = learnt.keys().map(LetterKey::letter).collect();
    assert_eq!(keys, vec!['I', 'N', 'Q', 'U']);
    assert_eq!(learnt.accent(key('Q')), Some('`'));
    assert_eq!(learnt.accent(key('E')), None);
    assert_eq!(learnt.accent(key('U')), Some('\u{2dd}'));
    assert_eq!(learnt.accent(key('I')), Some('ˆ'));

    assert_eq!(
        dead_key_note(key('Q'), &learnt),
        Some(DeadKey { accent: '`' })
    );
    assert_eq!(dead_key_note(key('E'), &learnt), None);

    // Learning nothing changes nothing, and learning is applied in order.
    assert_eq!(
        DeadKeyTable::us().with_learnt(std::iter::empty()),
        DeadKeyTable::us()
    );
    let twice = DeadKeyTable::empty().with_learnt([
        (key('B'), OptionChar::Dead('^')),
        (key('B'), OptionChar::Char('b')),
    ]);
    assert_eq!(twice, DeadKeyTable::empty());
}

// ---- KeyMap::restore ----------------------------------------------------------------------

#[test]
fn restore_of_a_clean_list_keeps_everything() {
    let (map, issues) = KeyMap::restore([
        (10, Some("KeyA")),
        (20, None),
        (30, Some("KeyS")),
        (40, Some("KeyZ")),
    ]);
    assert!(issues.is_empty());
    assert_eq!(
        shape(&map),
        vec![(10, 'A'), (20, '-'), (30, 'S'), (40, 'Z')]
    );
    assert_eq!(map.len(), 4);
    assert!(!map.is_empty());
    assert!(map.validate().is_empty());

    let (empty, issues) = KeyMap::restore(std::iter::empty());
    assert!(empty.is_empty());
    assert!(issues.is_empty());
    assert_eq!(empty, KeyMap::new());
}

#[test]
fn restore_drops_a_key_that_is_not_a_physical_letter() {
    let hostile = ["keya", "KeyAA", "", "Digit1", "A", " KeyA", "KeyÄ", "Key"];
    for text in hostile {
        let (map, issues) = KeyMap::restore([(7, Some(text))]);
        assert_eq!(shape(&map), vec![(7, '-')], "{text:?}");
        assert_eq!(
            issues,
            vec![KeyIssue::Invalid {
                group: 7,
                text: text.to_owned()
            }],
            "{text:?}"
        );
    }
}

#[test]
fn restore_keeps_a_contested_letter_with_the_earlier_group_in_ring_order() {
    // Group 5 comes first in ring order even though its id is larger.
    let (map, issues) = KeyMap::restore([
        (5, Some("KeyA")),
        (2, Some("KeyA")),
        (9, Some("KeyB")),
        (1, Some("KeyA")),
        (3, Some("KeyB")),
    ]);
    assert_eq!(
        shape(&map),
        vec![(5, 'A'), (2, '-'), (9, 'B'), (1, '-'), (3, '-')]
    );
    assert_eq!(
        issues,
        vec![
            KeyIssue::Duplicate {
                key: LetterKey::A,
                kept_by: 5,
                lost_by: 2
            },
            KeyIssue::Duplicate {
                key: LetterKey::A,
                kept_by: 5,
                lost_by: 1
            },
            KeyIssue::Duplicate {
                key: LetterKey::B,
                kept_by: 9,
                lost_by: 3
            },
        ]
    );
    assert!(map.validate().is_empty());
}

#[test]
fn restore_drops_a_repeated_group_id_with_its_key() {
    let (map, issues) = KeyMap::restore([
        (1, Some("KeyA")),
        (2, None),
        (1, Some("KeyB")),
        (1, Some("not a key")),
        (3, Some("KeyB")),
    ]);
    // The second and third entries for group 1 are dropped whole: KeyB stays free for group 3
    // and the bad name is not reported.
    assert_eq!(shape(&map), vec![(1, 'A'), (2, '-'), (3, 'B')]);
    assert_eq!(
        issues,
        vec![
            KeyIssue::DuplicateGroup { group: 1 },
            KeyIssue::DuplicateGroup { group: 1 },
        ]
    );
}

#[test]
fn restore_survives_64_groups_that_all_claim_key_a() {
    let (map, issues) = KeyMap::restore((0..64_u64).map(|group| (group, Some("KeyA"))));
    assert_eq!(map.len(), 64);
    assert_eq!(map.owner_of(LetterKey::A), Some(0));
    assert_eq!(map.key_of(0), Some(LetterKey::A));
    assert!((1..64).all(|group| map.key_of(group).is_none() && map.contains(group)));
    assert_eq!(issues.len(), 63);
    for (offset, issue) in issues.iter().enumerate() {
        assert_eq!(
            *issue,
            KeyIssue::Duplicate {
                key: LetterKey::A,
                kept_by: 0,
                lost_by: offset as u64 + 1
            }
        );
    }
    assert!(map.validate().is_empty());
}

#[test]
fn restore_of_a_mixed_hostile_list_gives_a_valid_map_and_every_issue_in_order() {
    let entries: Vec<(u64, Option<&str>)> = vec![
        (1, Some("KeyA")),
        (2, Some("keya")),
        (3, Some("KeyAA")),
        (4, Some("")),
        (5, Some("Digit1")),
        (2, Some("KeyB")), // a repeated group id
        (6, Some("KeyA")), // a contested letter
        (7, None),
        (8, Some("KeyZ")),
    ];
    let (map, issues) = KeyMap::restore(entries);
    assert_eq!(
        shape(&map),
        vec![
            (1, 'A'),
            (2, '-'),
            (3, '-'),
            (4, '-'),
            (5, '-'),
            (6, '-'),
            (7, '-'),
            (8, 'Z')
        ]
    );
    let invalid = |group: u64, text: &str| KeyIssue::Invalid {
        group,
        text: text.to_owned(),
    };
    assert_eq!(
        issues,
        vec![
            invalid(2, "keya"),
            invalid(3, "KeyAA"),
            invalid(4, ""),
            invalid(5, "Digit1"),
            KeyIssue::DuplicateGroup { group: 2 },
            KeyIssue::Duplicate {
                key: LetterKey::A,
                kept_by: 1,
                lost_by: 6
            },
        ]
    );
    assert!(map.validate().is_empty());
}

#[test]
fn restore_cuts_a_huge_invalid_key_in_the_issue() {
    let huge = "x".repeat(1_000_000);
    let (map, issues) = KeyMap::restore([(1, Some(huge.as_str()))]);
    assert_eq!(map.key_of(1), None);
    match issues.as_slice() {
        [KeyIssue::Invalid { group: 1, text }] => assert_eq!(text.chars().count(), 32),
        other => panic!("unexpected issues {other:?}"),
    }
    // Multi-byte text is cut on a character boundary.
    let wide = "é".repeat(100);
    let (_, issues) = KeyMap::restore([(1, Some(wide.as_str()))]);
    match issues.as_slice() {
        [KeyIssue::Invalid { text, .. }] => assert_eq!(text, &"é".repeat(32)),
        other => panic!("unexpected issues {other:?}"),
    }
}

#[test]
fn restore_reads_what_code_name_writes() {
    let names: Vec<&str> = LetterKey::ALL.iter().map(|k| k.code_name()).collect();
    let (map, issues) = KeyMap::restore(
        names
            .iter()
            .enumerate()
            .map(|(group, name)| (group as u64, Some(*name))),
    );
    assert!(issues.is_empty());
    for (group, letter) in LetterKey::ALL.iter().enumerate() {
        assert_eq!(map.key_of(group as u64), Some(*letter));
        assert_eq!(map.owner_of(*letter), Some(group as u64));
    }
    assert!(map.free_keys().is_empty());
}

// ---- Lookups ------------------------------------------------------------------------------

#[test]
fn lookups_answer_for_groups_and_letters() {
    let map = map_of(&[(10, Some('A')), (20, None), (30, Some('S'))]);
    assert_eq!(map.groups().collect::<Vec<_>>(), vec![10, 20, 30]);
    assert_eq!(
        map.entries(),
        &[
            (10, Some(LetterKey::A)),
            (20, None),
            (30, Some(LetterKey::S))
        ]
    );

    assert_eq!(map.key_of(10), Some(LetterKey::A));
    assert_eq!(map.key_of(20), None);
    assert_eq!(map.key_of(99), None);
    assert_eq!(map.owner_of(LetterKey::S), Some(30));
    assert_eq!(map.owner_of(LetterKey::D), None);

    assert_eq!(map.position(30), Some(2));
    assert_eq!(map.position(99), None);
    assert!(map.contains(20));
    assert!(!map.contains(21));

    let free: String = map.free_keys().iter().map(|k| k.letter()).collect();
    assert_eq!(free.len(), 24);
    assert!(!free.contains('A') && !free.contains('S'));
    assert!(free.starts_with("BCDEFGH"));
}

// ---- assign -------------------------------------------------------------------------------

#[test]
fn assign_answers_every_case() {
    // (setup, group, key to assign, answer, resulting shape)
    type Case = (
        Vec<(u64, Option<char>)>,
        u64,
        Option<char>,
        Assign,
        Vec<(u64, char)>,
    );
    let cases: Vec<Case> = vec![
        (
            vec![(1, None)],
            1,
            Some('A'),
            Assign::Assigned,
            vec![(1, 'A')],
        ),
        (
            vec![(1, Some('A'))],
            1,
            Some('B'),
            Assign::Assigned,
            vec![(1, 'B')],
        ),
        (
            vec![(1, Some('A'))],
            1,
            None,
            Assign::Cleared,
            vec![(1, '-')],
        ),
        (
            vec![(1, Some('A'))],
            1,
            Some('A'),
            Assign::Unchanged,
            vec![(1, 'A')],
        ),
        (vec![(1, None)], 1, None, Assign::Unchanged, vec![(1, '-')]),
        (
            vec![(1, Some('A')), (2, Some('B'))],
            2,
            Some('A'),
            Assign::Taken { by: 1 },
            vec![(1, 'A'), (2, 'B')],
        ),
        (
            vec![(1, Some('A')), (2, None)],
            2,
            Some('A'),
            Assign::Taken { by: 1 },
            vec![(1, 'A'), (2, '-')],
        ),
        (
            vec![(1, Some('A'))],
            9,
            Some('B'),
            Assign::UnknownGroup,
            vec![(1, 'A')],
        ),
        (
            vec![(1, Some('A'))],
            9,
            None,
            Assign::UnknownGroup,
            vec![(1, 'A')],
        ),
    ];
    for (index, (setup, group, wanted, answer, after)) in cases.into_iter().enumerate() {
        let mut map = map_of(&setup);
        assert_eq!(
            map.assign(group, wanted.map(key)),
            answer,
            "case {index}: {setup:?}"
        );
        assert_eq!(shape(&map), after, "case {index}: {setup:?}");
        assert!(map.validate().is_empty(), "case {index}");
    }
}

#[test]
fn a_taken_letter_changes_nothing() {
    let mut map = map_of(&[(1, Some('A')), (2, Some('S')), (3, None)]);
    let before = map.clone();

    assert_eq!(map.assign(2, Some(key('A'))), Assign::Taken { by: 1 });
    assert_eq!(map, before);
    assert_eq!(map.assign(3, Some(key('S'))), Assign::Taken { by: 2 });
    assert_eq!(map, before);
    assert_eq!(
        map.key_of(2),
        Some(LetterKey::S),
        "the asker keeps its own key"
    );

    // The interface then offers Swap: the two groups exchange letters.
    assert!(map.swap(2, 1));
    assert_eq!(shape(&map), vec![(1, 'S'), (2, 'A'), (3, '-')]);
}

#[test]
fn assigning_a_new_letter_frees_the_old_one() {
    let mut map = map_of(&[(1, Some('A')), (2, None)]);
    assert_eq!(map.assign(1, Some(key('B'))), Assign::Assigned);
    assert_eq!(map.owner_of(LetterKey::A), None);
    assert_eq!(map.assign(2, Some(key('A'))), Assign::Assigned);
    assert_eq!(shape(&map), vec![(1, 'B'), (2, 'A')]);
}

// ---- swap ---------------------------------------------------------------------------------

#[test]
fn swap_exchanges_keys_including_a_missing_one() {
    // (setup, first group, second group, answer, resulting shape)
    type Case = (Vec<(u64, Option<char>)>, u64, u64, bool, Vec<(u64, char)>);
    let cases: Vec<Case> = vec![
        // both keyed
        (
            vec![(1, Some('A')), (2, Some('B'))],
            1,
            2,
            true,
            vec![(1, 'B'), (2, 'A')],
        ),
        // either order of arguments
        (
            vec![(1, Some('A')), (2, Some('B'))],
            2,
            1,
            true,
            vec![(1, 'B'), (2, 'A')],
        ),
        // the first has none: it gains, the other loses
        (
            vec![(1, None), (2, Some('B')), (3, Some('C'))],
            1,
            2,
            true,
            vec![(1, 'B'), (2, '-'), (3, 'C')],
        ),
        // the second has none
        (
            vec![(1, Some('A')), (2, None)],
            1,
            2,
            true,
            vec![(1, '-'), (2, 'A')],
        ),
        // neither has one
        (
            vec![(1, None), (2, None)],
            1,
            2,
            true,
            vec![(1, '-'), (2, '-')],
        ),
        // a group with itself
        (vec![(1, Some('A'))], 1, 1, true, vec![(1, 'A')]),
        // unknown groups change nothing
        (vec![(1, Some('A'))], 1, 9, false, vec![(1, 'A')]),
        (vec![(1, Some('A'))], 9, 1, false, vec![(1, 'A')]),
        (vec![(1, Some('A'))], 8, 9, false, vec![(1, 'A')]),
    ];
    for (index, (setup, a, b, answer, after)) in cases.into_iter().enumerate() {
        let mut map = map_of(&setup);
        assert_eq!(map.swap(a, b), answer, "case {index}: {setup:?}");
        assert_eq!(shape(&map), after, "case {index}: {setup:?}");
        assert!(map.validate().is_empty(), "case {index}");
    }
}

#[test]
fn swap_does_not_change_ring_order() {
    let mut map = map_of(&[(3, Some('A')), (1, Some('B')), (2, Some('C'))]);
    assert!(map.swap(3, 2));
    assert_eq!(map.groups().collect::<Vec<_>>(), vec![3, 1, 2]);
}

// ---- add_group, remove_group --------------------------------------------------------------

#[test]
fn new_groups_take_the_sequence_in_order() {
    let sequence = parse_sequence(DEFAULT_KEY_SEQUENCE);
    let mut map = KeyMap::new();
    let mut given = Vec::new();
    for group in 100..109_u64 {
        given.push(map.add_group(group, &sequence));
    }
    let letters: String = given.iter().flatten().map(|k| k.letter()).collect();
    assert_eq!(letters, "ASDFGHJKL");
    assert_eq!(
        map.groups().collect::<Vec<_>>(),
        (100..109).collect::<Vec<_>>()
    );
    assert!(map.validate().is_empty());
}

#[test]
fn a_new_group_skips_letters_the_user_chose_for_other_groups() {
    let sequence = seq("ASDFG");
    let mut map = map_of(&[(1, Some('A')), (2, Some('D')), (3, Some('Q'))]);
    assert_eq!(map.add_group(4, &sequence), Some(LetterKey::S));
    assert_eq!(map.add_group(5, &sequence), Some(LetterKey::F));
    assert_eq!(map.add_group(6, &sequence), Some(LetterKey::G));
    assert_eq!(map.add_group(7, &sequence), None);
    assert_eq!(
        shape(&map),
        vec![
            (1, 'A'),
            (2, 'D'),
            (3, 'Q'),
            (4, 'S'),
            (5, 'F'),
            (6, 'G'),
            (7, '-')
        ]
    );
}

#[test]
fn the_sequence_running_out_gives_none_and_no_fallback_letters() {
    let sequence = seq("AS");
    let mut map = KeyMap::new();
    assert_eq!(map.add_group(1, &sequence), Some(LetterKey::A));
    assert_eq!(map.add_group(2, &sequence), Some(LetterKey::S));
    for group in 3..=40_u64 {
        assert_eq!(map.add_group(group, &sequence), None, "group {group}");
        assert_eq!(map.key_of(group), None);
        assert!(map.contains(group), "the group is added without a key");
    }
    assert_eq!(map.len(), 40);

    // An empty sequence never gives a letter.
    let mut bare = KeyMap::new();
    assert_eq!(bare.add_group(1, &[]), None);
    assert_eq!(bare.len(), 1);
}

#[test]
fn closing_a_group_frees_its_letter_for_the_next_add_group() {
    let sequence = seq("ASDF");
    let mut map = KeyMap::new();
    for group in 1..=4_u64 {
        let _ = map.add_group(group, &sequence);
    }
    assert_eq!(map.add_group(5, &sequence), None, "A S D F are all in use");

    assert!(map.remove_group(2));
    assert_eq!(map.owner_of(LetterKey::S), None);
    assert!(!map.contains(2));
    assert_eq!(map.add_group(6, &sequence), Some(LetterKey::S));

    // The first letter free wins, not the oldest freed one.
    assert!(map.remove_group(3));
    assert!(map.remove_group(1));
    assert_eq!(map.add_group(7, &sequence), Some(LetterKey::A));
    assert_eq!(map.add_group(8, &sequence), Some(LetterKey::D));
    assert_eq!(map.add_group(9, &sequence), None);
    assert!(map.validate().is_empty());
}

#[test]
fn removing_an_unknown_group_changes_nothing() {
    let mut map = map_of(&[(1, Some('A'))]);
    let before = map.clone();
    assert!(!map.remove_group(2));
    assert_eq!(map, before);
    assert!(map.remove_group(1));
    assert!(map.is_empty());
    assert!(!map.remove_group(1));
}

#[test]
fn adding_a_group_that_exists_leaves_it_alone() {
    let sequence = seq("ASDF");
    let mut map = map_of(&[(1, Some('D')), (2, None)]);
    let before = map.clone();
    assert_eq!(map.add_group(1, &sequence), Some(LetterKey::D));
    assert_eq!(map.add_group(2, &sequence), None);
    assert_eq!(map, before);
}

#[test]
fn add_group_with_a_repeating_sequence_never_gives_a_letter_twice() {
    let sequence = vec![LetterKey::A, LetterKey::A, LetterKey::B];
    let mut map = KeyMap::new();
    assert_eq!(map.add_group(1, &sequence), Some(LetterKey::A));
    assert_eq!(map.add_group(2, &sequence), Some(LetterKey::B));
    assert_eq!(map.add_group(3, &sequence), None);
    assert!(map.validate().is_empty());
}

// ---- move_group and reset_to_sequence -----------------------------------------------------

#[test]
fn move_group_changes_ring_order_and_keeps_keys() {
    let setup = [(1, Some('A')), (2, Some('S')), (3, None), (4, Some('F'))];
    let cases: [(u64, usize, bool, [u64; 4]); 8] = [
        (1, 0, true, [1, 2, 3, 4]),
        (1, 2, true, [2, 3, 1, 4]),
        (1, 3, true, [2, 3, 4, 1]),
        (1, 99, true, [2, 3, 4, 1]),
        (4, 0, true, [4, 1, 2, 3]),
        (3, 1, true, [1, 3, 2, 4]),
        (2, 3, true, [1, 3, 4, 2]),
        (9, 0, false, [1, 2, 3, 4]),
    ];
    for (group, to, answer, order) in cases {
        let mut map = map_of(&setup);
        assert_eq!(map.move_group(group, to), answer, "{group} to {to}");
        assert_eq!(map.groups().collect::<Vec<_>>(), order, "{group} to {to}");
        for (id, letter) in setup {
            assert_eq!(
                map.key_of(id),
                letter.map(key),
                "keys stay with their groups"
            );
        }
        if answer {
            let clamped = to.min(3);
            assert_eq!(map.position(group), Some(clamped));
        }
    }
}

#[test]
fn reset_after_a_reorder_follows_ring_order() {
    let sequence = parse_sequence(DEFAULT_KEY_SEQUENCE);
    let mut map = KeyMap::new();
    for group in 1..=4_u64 {
        let _ = map.add_group(group, &sequence);
    }
    assert_eq!(shape(&map), vec![(1, 'A'), (2, 'S'), (3, 'D'), (4, 'F')]);

    // The user drags group 4 to the front and group 1 to the back.
    assert!(map.move_group(4, 0));
    assert!(map.move_group(1, 3));
    assert_eq!(map.groups().collect::<Vec<_>>(), vec![4, 2, 3, 1]);
    assert_eq!(shape(&map), vec![(4, 'F'), (2, 'S'), (3, 'D'), (1, 'A')]);

    assert_eq!(map.reset_to_sequence(&sequence), 0);
    assert_eq!(shape(&map), vec![(4, 'A'), (2, 'S'), (3, 'D'), (1, 'F')]);
    assert!(map.validate().is_empty());
}

#[test]
fn reset_replaces_user_choices_and_leaves_groups_past_the_sequence_without_a_key() {
    let mut map = map_of(&[
        (1, Some('Q')),
        (2, None),
        (3, Some('A')),
        (4, Some('Z')),
        (5, None),
    ]);
    let without = map.reset_to_sequence(&seq("ASD"));
    assert_eq!(without, 2);
    assert_eq!(
        shape(&map),
        vec![(1, 'A'), (2, 'S'), (3, 'D'), (4, '-'), (5, '-')]
    );
    assert_eq!(map.owner_of(LetterKey::Q), None);
    assert!(map.validate().is_empty());
}

#[test]
fn reset_uses_a_repeated_letter_of_the_sequence_once() {
    let mut map = map_of(&[(1, None), (2, None), (3, None)]);
    let sequence = vec![LetterKey::A, LetterKey::A, LetterKey::B];
    assert_eq!(map.reset_to_sequence(&sequence), 1);
    assert_eq!(shape(&map), vec![(1, 'A'), (2, 'B'), (3, '-')]);
}

#[test]
fn reset_of_an_empty_map_or_an_empty_sequence() {
    let mut empty = KeyMap::new();
    assert_eq!(empty.reset_to_sequence(&seq("ASD")), 0);
    assert!(empty.is_empty());

    let mut map = map_of(&[(1, Some('A')), (2, Some('B'))]);
    assert_eq!(map.reset_to_sequence(&[]), 2);
    assert_eq!(shape(&map), vec![(1, '-'), (2, '-')]);

    // After a reset to nothing every letter is free again.
    assert_eq!(map.free_keys().len(), 26);
}

// ---- Invariants over a long run of operations ---------------------------------------------

/// A small deterministic generator, so the run is the same every time.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % below
    }
}

#[test]
fn no_run_of_operations_breaks_uniqueness() {
    let sequence = parse_sequence(DEFAULT_KEY_SEQUENCE);
    let mut random = Lcg(0x004d_6572_6375_7279);
    let mut map = KeyMap::new();
    let mut taken_seen = 0;
    for step in 0..4000 {
        let group = random.next(24);
        let letter = LetterKey::from_index(random.next(27) as usize);
        match random.next(7) {
            0 | 1 => {
                let _ = map.add_group(group, &sequence);
            }
            2 => {
                let _ = map.remove_group(group);
            }
            3 => {
                let before = map.clone();
                if let Assign::Taken { by } = map.assign(group, letter) {
                    taken_seen += 1;
                    assert_eq!(map, before, "step {step}: Taken must change nothing");
                    assert_eq!(map.owner_of(letter.expect("a taken letter")), Some(by));
                }
            }
            4 => {
                let _ = map.swap(group, random.next(24));
            }
            5 => {
                let _ = map.move_group(group, random.next(30) as usize);
            }
            _ => {
                if random.next(10) == 0 {
                    let _ = map.reset_to_sequence(&sequence);
                }
            }
        }
        assert!(
            map.validate().is_empty(),
            "step {step}: {:?}",
            map.entries()
        );

        let groups: Vec<u64> = map.groups().collect();
        let distinct: HashSet<u64> = groups.iter().copied().collect();
        assert_eq!(groups.len(), distinct.len(), "step {step}: a group twice");
        let letters: Vec<LetterKey> = map.entries().iter().filter_map(|&(_, k)| k).collect();
        let distinct: HashSet<LetterKey> = letters.iter().copied().collect();
        assert_eq!(letters.len(), distinct.len(), "step {step}: a letter twice");
        assert_eq!(map.free_keys().len(), 26 - letters.len());
    }
    assert!(taken_seen > 0, "the run must have met a taken letter");
}
