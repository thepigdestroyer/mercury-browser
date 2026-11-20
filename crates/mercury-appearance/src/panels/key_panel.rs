// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Tab groups section's pure half: the table of the window's groups with their one-letter
//! keys, the duplicate-key conflict, the dead-key note, and the capture mode (brief decisions D9
//! to D12 and D14; SPEC §3.3, §4.8, §5.1).
//!
//! The panel is a pure function of what it was last given by [`KeyPanel::refresh`] and of the one
//! conflict that may be pending. The application applies each [`Change`] to the browser's groups
//! and calls `refresh` again; the panel never keeps its own copy of the keys.
//!
//! Skeleton by the lane lead: the types and signatures are frozen; item 2 `key-panel` writes
//! the bodies (every `todo!` below) and the tests in `tests/key_panel.rs`.

use mercury_settings::GroupKeySettings;

use crate::colour::Colour;
use crate::keymap::{DeadKeyTable, LetterKey, OptionChar};

/// A group of the window, as the application hands it over, in ring order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupInput {
    /// The group's id (at most `i32::MAX`, the core's rule).
    pub id: u64,
    /// The group's name as stored, at most 64 bytes, possibly empty.
    pub name: String,
    /// The group's colour as stored: a preset id or `#rrggbb`. Read with
    /// [`GroupColour::parse`](crate::GroupColour::parse); unreadable text is slate (D15).
    pub colour: String,
    /// How many tabs the group has.
    pub tabs: usize,
    /// The group's key, if it has one.
    pub key: Option<LetterKey>,
}

/// What is known about what ⌥ plus a letter does on this keyboard (D12).
///
/// [`us`](Self::us) is the macOS U.S. layout. The key-switcher lanes learn the real layout from
/// key events and pass it in through [`with_learnt`](Self::with_learnt).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyboardFacts {
    dead: DeadKeyTable,
    typed: [Option<OptionChar>; 26],
    learnt: bool,
}

impl KeyboardFacts {
    /// The macOS U.S. layout, nothing learnt.
    pub fn us() -> Self {
        Self {
            dead: DeadKeyTable::us(),
            typed: [None; 26],
            learnt: false,
        }
    }

    /// These facts with what the application learnt: what ⌥ plus each key gave. Marks the facts
    /// as learnt. A key not mentioned keeps what the table says.
    #[must_use]
    pub fn with_learnt(
        mut self,
        learnt: impl IntoIterator<Item = (LetterKey, OptionChar)>,
    ) -> Self {
        let learnt: Vec<(LetterKey, OptionChar)> = learnt.into_iter().collect();
        for &(key, gave) in &learnt {
            self.typed[key.index()] = Some(gave);
        }
        self.dead = self.dead.with_learnt(learnt);
        self.learnt = true;
        self
    }

    /// Whether anything was learnt from the user's keyboard (the column header says so, D12).
    pub fn is_learnt(&self) -> bool {
        self.learnt
    }

    /// What ⌥ plus `key` gives: what was learnt, else the U.S. table's answer.
    pub fn option_char(&self, key: LetterKey) -> OptionChar {
        self.typed[key.index()].unwrap_or_else(|| key.us_option_char())
    }

    /// The dead keys of this keyboard.
    pub fn dead_keys(&self) -> &DeadKeyTable {
        &self.dead
    }
}

/// How a row is drawn (D11, D12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    /// Nothing to say.
    Plain,
    /// The key is a dead key: an amber note.
    DeadKey,
    /// The letter typed belongs to another group: a red field, the message, Swap and Choose
    /// another.
    Conflict,
    /// What was typed is not a letter: a red message, no buttons.
    Refused,
}

/// One group of the table.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyRow {
    /// The group's id.
    pub id: u64,
    /// The group's name.
    pub name: String,
    /// The group's colour, resolved (D15).
    pub colour: Colour,
    /// "1 tab", "5 tabs".
    pub tabs_text: String,
    /// The key as a capital letter, or empty when the group has none.
    pub letter: String,
    /// The "⌥ + key types" cell: the character, "´ (accent key)", or empty without a key.
    pub types: String,
    /// How the row is drawn.
    pub state: RowState,
    /// The note or error of the row: the amber note, the conflict sentence or the refusal.
    /// Empty for a plain row.
    pub message: String,
}

/// Everything the section shows. Plain fields, so the Slint side only copies them.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelView {
    /// The groups, in the order given.
    pub rows: Vec<KeyRow>,
    /// How many tabs belong to no group (D10); the Loose line is shown when this is above zero
    /// and there is at least one group.
    pub loose_tabs: usize,
    /// No groups at all.
    pub empty: bool,
    /// Whether group keys are switched on (`keys.enabled`).
    pub keys_enabled: bool,
    /// "Group keys are off. Turn them on in Appearance." when they are off (D9), else empty.
    pub keys_off_note: String,
    /// The header of the "types" column (D12).
    pub types_header: String,
    /// "Show keys beside names".
    pub show_beside_names: bool,
    /// The chosen capture radio: 0 outside text or hold, 1 outside text, 2 always.
    pub capture_index: usize,
    /// The hold time as text for the first radio's caption: "0.4 s".
    pub hold_text: String,
    /// The hold-time choices as text (D8).
    pub hold_choices: Vec<String>,
    /// The chosen item of `hold_choices`.
    pub hold_index: usize,
    /// "Reset to A, S, D, F, G…", built from the first five letters of the stored sequence.
    pub reset_label: String,
    /// Whether Reset has anything to do.
    pub reset_enabled: bool,
    /// The one note of D4: empty when the interface is live.
    pub note: String,
}

/// What the application is to do to the browser's groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Give `group` this key, or take its key away. The letter is free.
    SetKey {
        /// The group.
        group: u64,
        /// The new key.
        key: Option<LetterKey>,
    },
    /// Give `group` the letter `other` holds, and `other` the old letter of `group` (or none).
    Swap {
        /// The group the user was editing.
        group: u64,
        /// The group that held the letter.
        other: u64,
    },
}

/// What the user did in the Tab groups section, as `mercury-ui` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelAction {
    /// The section appeared: refresh the rows now (D14).
    Opened,
    /// A character was typed in a group's key cap, or the empty text for Backspace or Delete.
    KeyTyped {
        /// The group.
        group: u64,
        /// One typed character, or empty.
        text: String,
    },
    /// "Swap" on a group's conflict.
    Swap {
        /// The group in conflict.
        group: u64,
    },
    /// "Choose another" on a group's conflict.
    ChooseAnother {
        /// The group in conflict.
        group: u64,
    },
    /// "Reset to A, S, D, F, G…".
    Reset,
    /// "Show keys beside names".
    ShowBesideNames(bool),
    /// A capture radio: 0 outside text or hold, 1 outside text, 2 always.
    Capture(usize),
    /// An item of the hold-time choice.
    Hold(usize),
}

/// The Tab groups table.
#[derive(Debug, Default)]
pub struct KeyPanel {
    // Item 2 adds the private state: the last input, the settings, the facts and the pending
    // conflict.
}

impl KeyPanel {
    /// An empty panel.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the groups, the settings and the facts as they are now. A pending conflict that is
    /// no longer true is dropped (D11). `interface_live` is
    /// `mercury_ui::settings_sections::TAB_GROUP_INTERFACE_LIVE` (D4).
    pub fn refresh(
        &mut self,
        _groups: &[GroupInput],
        _loose_tabs: usize,
        _keys: &GroupKeySettings,
        _facts: &KeyboardFacts,
        _interface_live: bool,
    ) {
        todo!("key-panel")
    }

    /// What the section shows now.
    pub fn view(&self) -> PanelView {
        todo!("key-panel")
    }

    /// A character typed in a group's key cap (rules of D11). `Some` when the application is to
    /// change something; `None` when nothing changes (the same key, a refusal, a conflict).
    pub fn key_typed(&mut self, _group: u64, _text: &str) -> Option<Change> {
        todo!("key-panel")
    }

    /// "Swap" on the group's conflict: the [`Change::Swap`] that was promised, and the conflict
    /// is cleared. `None` when the group has no conflict.
    pub fn swap(&mut self, _group: u64) -> Option<Change> {
        todo!("key-panel")
    }

    /// "Choose another": the conflict of this group is dismissed, its key stays as it was.
    pub fn choose_another(&mut self, _group: u64) {
        todo!("key-panel")
    }

    /// "Reset to A, S, D, F, G…": the changes that give the groups, in ring order, the letters of
    /// the stored default sequence; empty when the keys already are those.
    pub fn reset_keys(&mut self) -> Vec<Change> {
        todo!("key-panel")
    }
}

/// The hold times offered, in milliseconds: 250, 400, 600, 800 and 1000, plus `current_ms` when
/// it is another value, in order (D8).
pub fn hold_choices(_current_ms: u16) -> Vec<u16> {
    todo!("key-panel")
}

/// Chooses a capture mode: 0 outside text or hold, 1 outside text, 2 always. `false`, and no
/// change, for another index.
pub fn set_capture(_keys: &mut GroupKeySettings, _index: usize) -> bool {
    todo!("key-panel")
}

/// Chooses an item of [`hold_choices`] for `keys.hold_ms`. `false`, and no change, for an index
/// outside the list.
pub fn set_hold(_keys: &mut GroupKeySettings, _index: usize) -> bool {
    todo!("key-panel")
}
