// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The one-letter ⌥ group keys: which letter belongs to which tab group, and the rules that keep
//! them unique (SPEC §3.3).
//!
//! This is pure logic. It knows nothing of winit, of Slint or of the browser core: a group is a
//! `u64` id, and a key is one of the 26 physical letter keys, stored as the strings `"KeyA"` to
//! `"KeyZ"` (the names winit gives its `KeyCode`s, and the strings of `GroupSnapshot.key` in
//! `session.json`). A key is a physical key, not a character, so changing the keyboard layout
//! never moves it.
//!
//! # The pieces
//!
//! - [`LetterKey`]: one of the 26 keys, with its name, its letter, and the character macOS types
//!   with ⌥ on a U.S. layout ([`OptionChar`]), for the fallback of the "⌥ + key types" column in
//!   Settings.
//! - [`parse_sequence`]: the stored default sequence (`"ASDFGHJKL"`) as keys.
//! - [`KeyMap`]: the groups of a window in ring order with their keys, and every change the
//!   interface makes to them. Two groups never hold the same letter; a map built through this API
//!   cannot break that, and [`KeyMap::restore`] repairs a stored list that does.
//! - [`DeadKeyTable`] and [`dead_key_note`]: the keys on which ⌥ starts an accent instead of
//!   typing a character. Such a key is allowed; the interface shows a note.
//!
//! # Rules the map keeps
//!
//! - **A letter belongs to at most one group.** [`KeyMap::assign`] to a taken letter changes
//!   nothing and says who has it ([`Assign::Taken`]); the interface then offers [`KeyMap::swap`]
//!   or another letter.
//! - **A new group gets the first letter of the default sequence that no group owns, and none
//!   once the sequence is used up** ([`KeyMap::add_group`]). Mercury never picks letters the user
//!   did not choose.
//! - **Ring order decides.** The earlier group keeps a contested letter on restore, and "Reset to
//!   A, S, D, F, G…" gives the letters to the groups in the order the ring shows them
//!   ([`KeyMap::reset_to_sequence`]).

use std::collections::HashSet;
use std::fmt;

/// How many letter keys there are: A to Z.
const LETTERS: usize = 26;

/// The longest text a [`KeyIssue::Invalid`] carries, in characters. A damaged file can hold any
/// string; the issue keeps only enough of it to show.
const ISSUE_TEXT_CHARS: usize = 32;

macro_rules! letter_keys {
    ($($name:ident)*) => {
        /// One of the 26 physical letter keys, A to Z.
        ///
        /// Ordered alphabetically. The key is physical: it is the same key on every keyboard
        /// layout, whatever character the layout puts on it.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        pub enum LetterKey {
            $(
                #[doc = concat!("The key `Key", stringify!($name), "`.")]
                $name,
            )*
        }

        impl LetterKey {
            /// All 26 keys, in alphabetical order.
            pub const ALL: [LetterKey; LETTERS] = [$(LetterKey::$name,)*];
        }
    };
}

letter_keys!(A B C D E F G H I J K L M N O P Q R S T U V W X Y Z);

/// The names winit and `session.json` use, indexed by [`LetterKey::index`].
const CODE_NAMES: [&str; LETTERS] = [
    "KeyA", "KeyB", "KeyC", "KeyD", "KeyE", "KeyF", "KeyG", "KeyH", "KeyI", "KeyJ", "KeyK", "KeyL",
    "KeyM", "KeyN", "KeyO", "KeyP", "KeyQ", "KeyR", "KeyS", "KeyT", "KeyU", "KeyV", "KeyW", "KeyX",
    "KeyY", "KeyZ",
];

/// What ⌥ plus a letter key does on the macOS U.S. layout, indexed by [`LetterKey::index`].
/// Written as code points so that look-alike characters (the increment sign U+2206 and the Greek
/// capital delta, the micro sign U+00B5 and the Greek mu) cannot be swapped by accident.
const US_OPTION: [OptionChar; LETTERS] = [
    OptionChar::Char('\u{e5}'),   // A  å
    OptionChar::Char('\u{222b}'), // B  ∫
    OptionChar::Char('\u{e7}'),   // C  ç
    OptionChar::Char('\u{2202}'), // D  ∂
    OptionChar::Dead('\u{b4}'),   // E  ´ (acute)
    OptionChar::Char('\u{192}'),  // F  ƒ
    OptionChar::Char('\u{a9}'),   // G  ©
    OptionChar::Char('\u{2d9}'),  // H  ˙
    OptionChar::Dead('\u{2c6}'),  // I  ˆ (circumflex)
    OptionChar::Char('\u{2206}'), // J  ∆
    OptionChar::Char('\u{2da}'),  // K  ˚
    OptionChar::Char('\u{ac}'),   // L  ¬
    OptionChar::Char('\u{b5}'),   // M  µ
    OptionChar::Dead('\u{2dc}'),  // N  ˜ (tilde)
    OptionChar::Char('\u{f8}'),   // O  ø
    OptionChar::Char('\u{3c0}'),  // P  π
    OptionChar::Char('\u{153}'),  // Q  œ
    OptionChar::Char('\u{ae}'),   // R  ®
    OptionChar::Char('\u{df}'),   // S  ß
    OptionChar::Char('\u{2020}'), // T  †
    OptionChar::Dead('\u{a8}'),   // U  ¨ (diaeresis)
    OptionChar::Char('\u{221a}'), // V  √
    OptionChar::Char('\u{2211}'), // W  ∑
    OptionChar::Char('\u{2248}'), // X  ≈
    OptionChar::Char('\u{a5}'),   // Y  ¥
    OptionChar::Char('\u{3a9}'),  // Z  Ω
];

/// What ⌥ plus a letter key gives: a character, or the start of an accent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OptionChar {
    /// The key types this character (⌥S types ß).
    Char(char),
    /// The key is a dead key: ⌥ plus it starts an accent, shown here by its spacing form, and
    /// types nothing until the next key (⌥E, then E, types é).
    Dead(char),
}

impl OptionChar {
    /// The character to show: the typed character, or the accent a dead key starts.
    pub const fn glyph(self) -> char {
        match self {
            Self::Char(glyph) | Self::Dead(glyph) => glyph,
        }
    }

    /// Whether this is a dead key.
    pub const fn is_dead(self) -> bool {
        matches!(self, Self::Dead(_))
    }
}

impl LetterKey {
    /// The key's place in [`ALL`](Self::ALL): 0 for A to 25 for Z.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The key at that place in [`ALL`](Self::ALL), or `None` past 25.
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// The key a letter names, in either case. Only the ASCII letters `A` to `Z` and `a` to `z`
    /// name a key; accented letters, digits and everything else do not.
    pub const fn from_char(letter: char) -> Option<Self> {
        match letter {
            'A'..='Z' => Some(Self::ALL[(letter as u32 - 'A' as u32) as usize]),
            'a'..='z' => Some(Self::ALL[(letter as u32 - 'a' as u32) as usize]),
            _ => None,
        }
    }

    /// The key a stored name refers to: exactly `"KeyA"` to `"KeyZ"`. Any other text, including
    /// a lower-case `"keya"`, a longer `"KeyAA"`, an empty string and `"Digit1"`, is `None`.
    pub fn from_code_name(name: &str) -> Option<Self> {
        let letter = name.strip_prefix("Key")?;
        let mut chars = letter.chars();
        match (chars.next(), chars.next()) {
            (Some(letter @ 'A'..='Z'), None) => Self::from_char(letter),
            _ => None,
        }
    }

    /// The name stored in `session.json` and used by winit: `"KeyA"` to `"KeyZ"`.
    pub const fn code_name(self) -> &'static str {
        CODE_NAMES[self.index()]
    }

    /// The key's letter, in capitals.
    pub const fn letter(self) -> char {
        (b'A' + self as u8) as char
    }

    /// What ⌥ plus this key gives on the macOS U.S. layout. This is only the fallback of the
    /// "⌥ + key types" column in Settings: the app learns the real character on the user's
    /// layout from key events and prefers that.
    pub const fn us_option_char(self) -> OptionChar {
        US_OPTION[self.index()]
    }
}

impl fmt::Display for LetterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.letter())
    }
}

/// The keys named by a stored key sequence such as `"ASDFGHJKL"`, in the order written.
///
/// Only the ASCII letters count, in either case; every other character is skipped; a letter that
/// came earlier is not taken again. This is the rule `mercury-settings` applies when it repairs
/// `tab_groups.keys.default_sequence`, so a stored sequence always parses to what it spells.
pub fn parse_sequence(text: &str) -> Vec<LetterKey> {
    let mut seen = [false; LETTERS];
    let mut keys = Vec::new();
    for key in text.chars().filter_map(LetterKey::from_char) {
        if !std::mem::replace(&mut seen[key.index()], true) {
            keys.push(key);
        }
    }
    keys
}

/// The keys on which ⌥ starts an accent instead of typing a character, with the accent each
/// starts.
///
/// [`us`](Self::us) is the macOS U.S. layout (E, I, N and U). Other layouts differ, so the app
/// adds what it learns from key events with [`with_learnt`](Self::with_learnt).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeadKeyTable {
    accents: [Option<char>; LETTERS],
}

impl DeadKeyTable {
    /// A table with no dead keys: a layout where ⌥ plus every letter types a character.
    pub const fn empty() -> Self {
        Self {
            accents: [None; LETTERS],
        }
    }

    /// The macOS U.S. layout: E (´), I (ˆ), N (˜) and U (¨).
    pub fn us() -> Self {
        let mut table = Self::empty();
        for key in LetterKey::ALL {
            if let OptionChar::Dead(accent) = key.us_option_char() {
                table.accents[key.index()] = Some(accent);
            }
        }
        table
    }

    /// This table with what the app has learnt about the user's layout. Each pair says what ⌥
    /// plus a key gave: [`OptionChar::Dead`] marks the key dead with that accent, replacing what
    /// the table knew; [`OptionChar::Char`] marks it as typing a character, so it is no longer
    /// dead. A key not mentioned keeps what the table says.
    #[must_use]
    pub fn with_learnt(
        mut self,
        learnt: impl IntoIterator<Item = (LetterKey, OptionChar)>,
    ) -> Self {
        for (key, gave) in learnt {
            self.accents[key.index()] = match gave {
                OptionChar::Dead(accent) => Some(accent),
                OptionChar::Char(_) => None,
            };
        }
        self
    }

    /// The accent ⌥ plus this key starts, if it is a dead key.
    pub const fn accent(&self, key: LetterKey) -> Option<char> {
        self.accents[key.index()]
    }

    /// The dead keys, in alphabetical order.
    pub fn keys(&self) -> impl Iterator<Item = LetterKey> + '_ {
        LetterKey::ALL
            .into_iter()
            .filter(|key| self.accents[key.index()].is_some())
    }
}

impl Default for DeadKeyTable {
    /// The U.S. layout, so that without anything learnt the interface errs towards the note.
    fn default() -> Self {
        Self::us()
    }
}

/// The note for a key that is a dead key: ⌥ plus it starts an accent rather than typing a
/// character. The key is still allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeadKey {
    /// The accent the key starts, in its spacing form (´ for E on the U.S. layout).
    pub accent: char,
}

/// The note to show for `key`, or `None` when ⌥ plus it types a character. A dead key is
/// allowed as a group key (SPEC §3.3): the note only tells the user that ⌥ plus that key starts
/// an accent rather than typing a character.
pub fn dead_key_note(key: LetterKey, table: &DeadKeyTable) -> Option<DeadKey> {
    table.accent(key).map(|accent| DeadKey { accent })
}

/// What was wrong with a stored or live key map, and what was done about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyIssue {
    /// A stored key was not `"KeyA"` to `"KeyZ"`; the group lost its key.
    Invalid {
        /// The group whose key was refused.
        group: u64,
        /// The stored text, cut to 32 characters.
        text: String,
    },
    /// Two groups held the same letter; the earlier one in ring order keeps it.
    Duplicate {
        /// The contested letter.
        key: LetterKey,
        /// The earlier group, which keeps the letter.
        kept_by: u64,
        /// The later group, which loses it.
        lost_by: u64,
    },
    /// A group id appeared twice; the later entry was dropped.
    DuplicateGroup {
        /// The repeated group id.
        group: u64,
    },
}

/// The result of [`KeyMap::assign`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Assign {
    /// The group now has the key.
    Assigned,
    /// The group's key was removed.
    Cleared,
    /// Nothing to do: the group already had that key, or had none and none was asked.
    Unchanged,
    /// Another group has the letter and nothing changed. The interface offers to swap the two
    /// ([`KeyMap::swap`]) or to choose another letter.
    Taken {
        /// The group that holds the letter.
        by: u64,
    },
    /// The map has no such group; nothing changed.
    UnknownGroup,
}

/// Which letters are claimed, and which group ids were seen: the one check that
/// [`KeyMap::restore`] and [`KeyMap::validate`] share.
struct Claims {
    groups: HashSet<u64>,
    owners: [Option<u64>; LETTERS],
}

impl Claims {
    fn new() -> Self {
        Self {
            groups: HashSet::new(),
            owners: [None; LETTERS],
        }
    }

    /// Notes a group id; `false` when it was seen before.
    fn admit(&mut self, group: u64) -> bool {
        self.groups.insert(group)
    }

    /// Claims a letter for a group, or reports who already holds it.
    fn claim(&mut self, group: u64, key: LetterKey) -> Result<(), KeyIssue> {
        let owner = &mut self.owners[key.index()];
        match *owner {
            Some(kept_by) => Err(KeyIssue::Duplicate {
                key,
                kept_by,
                lost_by: group,
            }),
            None => {
                *owner = Some(group);
                Ok(())
            }
        }
    }
}

/// The tab groups of a window in ring order, each with its one-letter ⌥ key, if it has one.
///
/// Invariants, kept by every method: a group id appears once, and a letter belongs to at most one
/// group. A group may have no key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyMap {
    entries: Vec<(u64, Option<LetterKey>)>,
}

impl KeyMap {
    /// A map with no groups.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a map from stored `(group, key name)` pairs in ring order, repairing what a
    /// damaged or hand-edited file can hold. This is what session restore calls.
    ///
    /// - A key that is not `"KeyA"` to `"KeyZ"` is dropped: [`KeyIssue::Invalid`].
    /// - When two groups hold a letter, the earlier in ring order keeps it and the later one
    ///   loses it: [`KeyIssue::Duplicate`].
    /// - A group id that already appeared is dropped with its key, not examined further:
    ///   [`KeyIssue::DuplicateGroup`].
    ///
    /// Issues come in ring order. A group without a stored key (`None`) is not an issue.
    pub fn restore<'a>(
        entries: impl IntoIterator<Item = (u64, Option<&'a str>)>,
    ) -> (Self, Vec<KeyIssue>) {
        let mut map = Self::new();
        let mut issues = Vec::new();
        let mut claims = Claims::new();
        for (group, stored) in entries {
            if !claims.admit(group) {
                issues.push(KeyIssue::DuplicateGroup { group });
                continue;
            }
            let key = match stored {
                None => None,
                Some(text) => match LetterKey::from_code_name(text) {
                    None => {
                        issues.push(KeyIssue::Invalid {
                            group,
                            text: text.chars().take(ISSUE_TEXT_CHARS).collect(),
                        });
                        None
                    }
                    Some(key) => match claims.claim(group, key) {
                        Ok(()) => Some(key),
                        Err(issue) => {
                            issues.push(issue);
                            None
                        }
                    },
                },
            };
            map.entries.push((group, key));
        }
        (map, issues)
    }

    /// The groups and their keys in ring order.
    pub fn entries(&self) -> &[(u64, Option<LetterKey>)] {
        &self.entries
    }

    /// The groups in ring order.
    pub fn groups(&self) -> impl Iterator<Item = u64> + '_ {
        self.entries.iter().map(|&(group, _)| group)
    }

    /// How many groups the map holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map holds no group.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the map holds this group.
    pub fn contains(&self, group: u64) -> bool {
        self.position(group).is_some()
    }

    /// The group's place in ring order, from 0.
    pub fn position(&self, group: u64) -> Option<usize> {
        self.entries.iter().position(|&(id, _)| id == group)
    }

    /// The group's key; `None` when the group has none or is not in the map.
    pub fn key_of(&self, group: u64) -> Option<LetterKey> {
        self.entries
            .iter()
            .find(|&&(id, _)| id == group)
            .and_then(|&(_, key)| key)
    }

    /// The group that holds this letter.
    pub fn owner_of(&self, key: LetterKey) -> Option<u64> {
        self.entries
            .iter()
            .find(|&&(_, held)| held == Some(key))
            .map(|&(group, _)| group)
    }

    /// The letters no group holds, in alphabetical order: what "Choose another" can offer.
    pub fn free_keys(&self) -> Vec<LetterKey> {
        LetterKey::ALL
            .into_iter()
            .filter(|&key| self.owner_of(key).is_none())
            .collect()
    }

    /// Checks the invariants and reports every break, as [`restore`](Self::restore) would
    /// repair it. Always empty for a map built through this API; it is for tests and for
    /// asserting after a restore.
    pub fn validate(&self) -> Vec<KeyIssue> {
        let mut issues = Vec::new();
        let mut claims = Claims::new();
        for &(group, key) in &self.entries {
            if !claims.admit(group) {
                issues.push(KeyIssue::DuplicateGroup { group });
            } else if let Some(key) = key
                && let Err(issue) = claims.claim(group, key)
            {
                issues.push(issue);
            }
        }
        issues
    }

    /// Gives a group a key, or removes it (`None`).
    ///
    /// A letter another group holds is never taken: nothing changes and the answer is
    /// [`Assign::Taken`], so the interface can offer [`swap`](Self::swap) or another letter.
    pub fn assign(&mut self, group: u64, key: Option<LetterKey>) -> Assign {
        let Some(index) = self.position(group) else {
            return Assign::UnknownGroup;
        };
        if self.entries[index].1 == key {
            return Assign::Unchanged;
        }
        let Some(key) = key else {
            self.entries[index].1 = None;
            return Assign::Cleared;
        };
        match self.owner_of(key) {
            Some(by) => Assign::Taken { by },
            None => {
                self.entries[index].1 = Some(key);
                Assign::Assigned
            }
        }
    }

    /// Exchanges the keys of two groups; either may have none (the other then loses its key and
    /// the first gains it). Swapping a group with itself changes nothing. Returns `false`, with
    /// nothing changed, when either group is not in the map.
    pub fn swap(&mut self, a: u64, b: u64) -> bool {
        let (Some(first), Some(second)) = (self.position(a), self.position(b)) else {
            return false;
        };
        let key_a = self.entries[first].1;
        self.entries[first].1 = self.entries[second].1;
        self.entries[second].1 = key_a;
        true
    }

    /// Adds a group at the end of the ring and gives it the first letter of `sequence` that no
    /// group holds. Returns that letter, or `None` when the sequence is used up: the group then
    /// has no key and the user can assign one (D10: Mercury never falls back to letters outside
    /// the sequence).
    ///
    /// A group already in the map is left as it is and its current key is returned.
    pub fn add_group(&mut self, group: u64, sequence: &[LetterKey]) -> Option<LetterKey> {
        if let Some(index) = self.position(group) {
            return self.entries[index].1;
        }
        let key = sequence
            .iter()
            .copied()
            .find(|&candidate| self.owner_of(candidate).is_none());
        self.entries.push((group, key));
        key
    }

    /// Removes a group and frees its letter. Returns `false` when the group is not in the map.
    pub fn remove_group(&mut self, group: u64) -> bool {
        match self.position(group) {
            Some(index) => {
                self.entries.remove(index);
                true
            }
            None => false,
        }
    }

    /// Moves a group to a place in ring order, from 0; a place past the end is the last place.
    /// Keys do not change. Returns `false` when the group is not in the map.
    pub fn move_group(&mut self, group: u64, index: usize) -> bool {
        let Some(from) = self.position(group) else {
            return false;
        };
        let entry = self.entries.remove(from);
        let to = index.min(self.entries.len());
        self.entries.insert(to, entry);
        true
    }

    /// Gives the letters of `sequence`, in order, to the groups in their current ring order,
    /// replacing every key. A group past the end of the sequence gets no key; a letter that
    /// repeats in `sequence` is used once. Returns how many groups ended with no key.
    ///
    /// This is "Reset to A, S, D, F, G…" in Settings.
    pub fn reset_to_sequence(&mut self, sequence: &[LetterKey]) -> usize {
        let mut seen = [false; LETTERS];
        let mut letters = sequence
            .iter()
            .copied()
            .filter(|key| !std::mem::replace(&mut seen[key.index()], true));
        let mut without = 0;
        for entry in &mut self.entries {
            entry.1 = letters.next();
            without += usize::from(entry.1.is_none());
        }
        without
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_reports_what_a_damaged_map_would_be_repaired_from() {
        // Fields are private, so only this module can build a map that breaks the invariants.
        let map = KeyMap {
            entries: vec![
                (1, Some(LetterKey::A)),
                (2, Some(LetterKey::A)),
                (1, Some(LetterKey::B)),
                (3, None),
            ],
        };
        assert_eq!(
            map.validate(),
            vec![
                KeyIssue::Duplicate {
                    key: LetterKey::A,
                    kept_by: 1,
                    lost_by: 2
                },
                KeyIssue::DuplicateGroup { group: 1 },
            ]
        );
    }

    #[test]
    fn the_code_name_table_matches_the_letters() {
        for key in LetterKey::ALL {
            assert_eq!(key.code_name(), format!("Key{}", key.letter()));
        }
    }
}
