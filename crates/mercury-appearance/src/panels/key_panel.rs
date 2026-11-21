// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Tab groups section's pure half: the table of the window's groups with their one-letter
//! keys, the duplicate-key conflict, the dead-key note, and the capture mode (brief decisions D9
//! to D12 and D14; SPEC §3.3, §4.8, §5.1).
//!
//! The panel is a pure function of what it was last given by [`KeyPanel::refresh`] and of the one
//! conflict that may be pending. The application applies each [`Change`] to the browser's groups
//! and calls `refresh` again; the panel never changes the keys it was given.
//!
//! Item 2 `key-panel` of lane `settings-ui`; the tests are in `tests/key_panel.rs`. The types and
//! signatures are the lane's frozen contract. The panel never panics: any text, any id, any
//! number of groups and any stored value give an answer.
//!
//! # The one pending state
//!
//! Typing into a group's key cap is an edit. It replaces whatever was pending (a conflict or a
//! refusal, on any group) and may leave a new one: a conflict when another group holds the letter,
//! a refusal when the text is not a letter. [`KeyPanel::refresh`] keeps a pending state while it is
//! still true and drops it when it is not. A panel that outlives a visit to the section keeps it
//! too, so the application starts a fresh [`KeyPanel`] when the section is opened again.

use std::collections::HashSet;

use mercury_settings::{
    DEFAULT_KEY_HOLD_MS, GroupKeySettings, KeyCapture, MAX_KEY_HOLD_MS, MIN_KEY_HOLD_MS,
};

use crate::colour::Colour;
use crate::group_colour::{GroupColour, PRESETS, preset};
use crate::keymap::{
    Assign, DeadKeyTable, KeyMap, LetterKey, OptionChar, dead_key_note, parse_sequence,
};

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

/// The sentence of a refused key (D11).
const REFUSED: &str = "Type a letter from A to Z.";

/// The line above the table while group keys are switched off (D9).
const KEYS_OFF: &str = "Group keys are off. Turn them on in Appearance.";

/// The one note of D4. The same words as the Appearance panel's note; the two sections show it
/// side by side and must never disagree.
const INTERFACE_NOT_LIVE: &str = "These choices are saved, but the tab-group interface they control arrives in a later version. Until then they change nothing you see.";

/// The header of the "types" column while nothing is learnt, and once something is (D12).
const TYPES_HEADER_US: &str = "\u{2325} + KEY TYPES (U.S. LAYOUT)";
const TYPES_HEADER_LEARNT: &str = "\u{2325} + KEY TYPES ON THIS KEYBOARD";

/// The hold times offered, in milliseconds, in order (D8).
const HOLD_CHOICES: [u16; 5] = [250, 400, 600, 800, 1000];

// The offered times span exactly what the settings accept, and the default is one of them.
const _: () = {
    assert!(HOLD_CHOICES[0] == MIN_KEY_HOLD_MS);
    assert!(HOLD_CHOICES[HOLD_CHOICES.len() - 1] == MAX_KEY_HOLD_MS);
    assert!(HOLD_CHOICES[1] == DEFAULT_KEY_HOLD_MS);
};

/// How many letters the label of Reset spells out.
const RESET_LABEL_LETTERS: usize = 5;

/// What the user was last told in one row: the single pending state (D11). A conflict and a
/// refusal share the slot, so "one at a time" holds by construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    /// `group` was asked to take `wanted`, which `holder` has.
    Conflict {
        group: u64,
        wanted: LetterKey,
        holder: u64,
    },
    /// `group` was given something that is not a letter.
    Refused { group: u64 },
}

impl Pending {
    /// Whether it still describes the groups as they are now (D11: the holder was closed, the
    /// letter was freed, the holder changed, or the group itself went or took the letter).
    fn still_true(self, keys: &KeyMap) -> bool {
        match self {
            Self::Refused { group } => keys.contains(group),
            Self::Conflict {
                group,
                wanted,
                holder,
            } => holder != group && keys.contains(group) && keys.owner_of(wanted) == Some(holder),
        }
    }
}

/// What a typed text asks for (D11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Typed {
    /// Backspace or Delete: take the key away.
    Clear,
    /// One ASCII letter, either case.
    Letter(LetterKey),
    /// Anything else.
    Refused,
}

impl Typed {
    /// Reads at most two characters of `text`, so a huge text costs no more than a short one.
    fn read(text: &str) -> Self {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (None, _) => Self::Clear,
            (Some(only), None) => LetterKey::from_char(only).map_or(Self::Refused, Self::Letter),
            _ => Self::Refused,
        }
    }
}

/// The Tab groups table.
#[derive(Debug)]
pub struct KeyPanel {
    /// The groups of the last refresh, in the order given, each id once (a later repeat of an
    /// id is dropped, as [`KeyMap::restore`] drops it).
    groups: Vec<GroupInput>,
    /// Their keys as the map rules read them: the earlier group in ring order keeps a letter
    /// two groups claim, so a letter is never shown twice.
    map: KeyMap,
    loose_tabs: usize,
    settings: GroupKeySettings,
    facts: KeyboardFacts,
    interface_live: bool,
    pending: Option<Pending>,
}

impl Default for KeyPanel {
    /// No groups, the default key settings, the U.S. facts, the interface not live.
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            map: KeyMap::new(),
            loose_tabs: 0,
            settings: GroupKeySettings::default(),
            facts: KeyboardFacts::us(),
            interface_live: false,
            pending: None,
        }
    }
}

impl KeyPanel {
    /// An empty panel.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the groups, the settings and the facts as they are now. A pending conflict that is
    /// no longer true is dropped (D11). `interface_live` is
    /// `mercury_ui::settings_sections::TAB_GROUP_INTERFACE_LIVE` (D4).
    ///
    /// A group id that appears twice is one row (the first), and a letter two groups claim is
    /// shown for the earlier one only; the application's own groups never do either, but the
    /// panel must not show what the rules forbid.
    pub fn refresh(
        &mut self,
        groups: &[GroupInput],
        loose_tabs: usize,
        keys: &GroupKeySettings,
        facts: &KeyboardFacts,
        interface_live: bool,
    ) {
        let mut seen = HashSet::new();
        self.groups = groups
            .iter()
            .filter(|group| seen.insert(group.id))
            .cloned()
            .collect();
        let (map, _repairs) = KeyMap::restore(
            self.groups
                .iter()
                .map(|group| (group.id, group.key.map(LetterKey::code_name))),
        );
        self.map = map;
        self.loose_tabs = loose_tabs;
        self.settings = keys.clone();
        self.facts = *facts;
        self.interface_live = interface_live;
        self.pending = self.pending.filter(|pending| pending.still_true(&self.map));
    }

    /// What the section shows now.
    pub fn view(&self) -> PanelView {
        let hold = self.settings.hold_ms;
        let choices = hold_choices(hold);
        PanelView {
            rows: self.groups.iter().map(|group| self.row(group)).collect(),
            loose_tabs: self.loose_tabs,
            empty: self.groups.is_empty(),
            keys_enabled: self.settings.enabled,
            keys_off_note: if self.settings.enabled {
                String::new()
            } else {
                KEYS_OFF.to_owned()
            },
            types_header: if self.facts.is_learnt() {
                TYPES_HEADER_LEARNT
            } else {
                TYPES_HEADER_US
            }
            .to_owned(),
            show_beside_names: self.settings.show_beside_names,
            capture_index: capture_index(self.settings.capture),
            hold_text: hold_text(hold),
            hold_index: choices
                .iter()
                .position(|&choice| choice == hold)
                .unwrap_or_default(),
            hold_choices: choices.iter().map(|&choice| hold_text(choice)).collect(),
            reset_label: reset_label(&self.settings.default_sequence),
            reset_enabled: !self.reset_plan().is_empty(),
            note: if self.interface_live {
                String::new()
            } else {
                INTERFACE_NOT_LIVE.to_owned()
            },
        }
    }

    /// A character typed in a group's key cap (rules of D11). `Some` when the application is to
    /// change something; `None` when nothing changes (the same key, a refusal, a conflict).
    ///
    /// Typing into a group of the table is an edit and replaces whatever was pending; typing
    /// into a group that is not in the table does nothing at all.
    pub fn key_typed(&mut self, group: u64, text: &str) -> Option<Change> {
        if !self.map.contains(group) {
            return None;
        }
        self.pending = None;
        let key = match Typed::read(text) {
            Typed::Clear => None,
            Typed::Letter(letter) => Some(letter),
            Typed::Refused => {
                self.pending = Some(Pending::Refused { group });
                return None;
            }
        };
        // The panel keeps no keys of its own: the rules run on a copy.
        match self.map.clone().assign(group, key) {
            Assign::Assigned | Assign::Cleared => Some(Change::SetKey { group, key }),
            Assign::Taken { by } => {
                self.pending = key.map(|wanted| Pending::Conflict {
                    group,
                    wanted,
                    holder: by,
                });
                None
            }
            Assign::Unchanged | Assign::UnknownGroup => None,
        }
    }

    /// "Swap" on the group's conflict: the [`Change::Swap`] that was promised, and the conflict
    /// is cleared. `None` when the group has no conflict.
    pub fn swap(&mut self, group: u64) -> Option<Change> {
        let Some(Pending::Conflict {
            group: pending,
            wanted,
            holder,
        }) = self.pending
        else {
            return None;
        };
        if pending != group {
            return None;
        }
        self.pending = None;
        // True by construction (a refresh drops a conflict that is not); checked once more so
        // that a Swap is never promised for letters that are not as the message said.
        (holder != group && self.map.contains(group) && self.map.owner_of(wanted) == Some(holder))
            .then_some(Change::Swap {
                group,
                other: holder,
            })
    }

    /// "Choose another": the conflict of this group is dismissed, its key stays as it was.
    pub fn choose_another(&mut self, group: u64) {
        if matches!(self.pending, Some(Pending::Conflict { group: held, .. }) if held == group) {
            self.pending = None;
        }
    }

    /// "Reset to A, S, D, F, G…": the changes that give the groups, in ring order, the letters of
    /// the stored default sequence; empty when the keys already are those.
    ///
    /// The changes are meant to be applied one after the other: each names a letter that is free
    /// at that moment, or swaps with the group that holds it (the holder is always later in
    /// ring order and gets its own turn). Reset is an edit and replaces whatever was pending.
    pub fn reset_keys(&mut self) -> Vec<Change> {
        self.pending = None;
        self.reset_plan()
    }

    /// One row of the table.
    fn row(&self, group: &GroupInput) -> KeyRow {
        let key = self.map.key_of(group.id);
        let (state, message) = self.row_state(group.id, key);
        KeyRow {
            id: group.id,
            name: group.name.clone(),
            colour: resolved_colour(&group.colour),
            tabs_text: tabs_text(group.tabs),
            letter: key.map(|key| key.letter().to_string()).unwrap_or_default(),
            types: key.map(|key| self.types_text(key)).unwrap_or_default(),
            state,
            message,
        }
    }

    /// How a row is drawn and what it says: the pending conflict or refusal first, then the
    /// amber note of a dead key, else nothing.
    fn row_state(&self, id: u64, key: Option<LetterKey>) -> (RowState, String) {
        match self.pending {
            Some(Pending::Conflict {
                group,
                wanted,
                holder,
            }) if group == id => {
                return (
                    RowState::Conflict,
                    format!("{wanted} is already {}'s key.", self.name_of(holder)),
                );
            }
            Some(Pending::Refused { group }) if group == id => {
                return (RowState::Refused, REFUSED.to_owned());
            }
            _ => {}
        }
        match key {
            Some(key) if dead_key_note(key, self.facts.dead_keys()).is_some() => {
                (RowState::DeadKey, self.dead_key_message(key))
            }
            _ => (RowState::Plain, String::new()),
        }
    }

    /// A group's name for a sentence: as stored, or "an unnamed group" when nothing is there to
    /// show (the core never stores such a name; the panel does not rely on it).
    fn name_of(&self, id: u64) -> &str {
        self.groups
            .iter()
            .find(|group| group.id == id)
            .map(|group| group.name.as_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("an unnamed group")
    }

    /// The "⌥ + key types" cell of a key: what ⌥ types, or the accent a dead key starts.
    fn types_text(&self, key: LetterKey) -> String {
        match dead_key_note(key, self.facts.dead_keys()) {
            Some(dead) => format!("{} (accent key)", dead.accent),
            None => self.facts.option_char(key).glyph().to_string(),
        }
    }

    /// The amber note of a dead key (D12), by capture mode, quoting the chosen hold.
    fn dead_key_message(&self, key: LetterKey) -> String {
        // The window never claims a layout it has not seen.
        let layout = if self.facts.is_learnt() {
            "this keyboard"
        } else {
            "the U.S. layout"
        };
        match self.settings.capture {
            KeyCapture::OutsideTextOrHold => format!(
                "\u{2325}{key} is an accent key on {layout}: it switches groups only outside text fields, or after the {} hold.",
                hold_text(self.settings.hold_ms)
            ),
            KeyCapture::OutsideText => format!(
                "\u{2325}{key} is an accent key on {layout}: it switches groups only outside text fields."
            ),
            KeyCapture::Always => format!(
                "\u{2325}{key} is an accent key on {layout}. With Always chosen it cannot start an accent in Mercury."
            ),
        }
    }

    /// The changes of Reset, in ring order, each valid when the ones before it were applied.
    fn reset_plan(&self) -> Vec<Change> {
        let mut target = self.map.clone();
        let _without = target.reset_to_sequence(&parse_sequence(&self.settings.default_sequence));
        // The keys as the application will have them after each change.
        let mut now = self.map.clone();
        let mut changes = Vec::new();
        for &(group, wanted) in target.entries() {
            if now.key_of(group) == wanted {
                continue;
            }
            match wanted {
                None => {
                    let _ = now.assign(group, None);
                    changes.push(Change::SetKey { group, key: None });
                }
                Some(letter) => match now.owner_of(letter) {
                    None => {
                        let _ = now.assign(group, Some(letter));
                        changes.push(Change::SetKey {
                            group,
                            key: Some(letter),
                        });
                    }
                    Some(holder) => {
                        // The holder is a later group whose own turn comes: it ends with this
                        // group's old letter, which its turn then replaces.
                        let _ = now.swap(group, holder);
                        changes.push(Change::Swap {
                            group,
                            other: holder,
                        });
                    }
                },
            }
        }
        changes
    }
}

/// The colour a group's stored text names, as a colour; slate when it names none (D15).
fn resolved_colour(stored: &str) -> Colour {
    GroupColour::parse(stored).map_or_else(slate, |colour| colour.base())
}

/// Slate, the colour of the Loose pseudo-group and of every unreadable colour.
fn slate() -> Colour {
    preset("slate")
        .or_else(|| PRESETS.last())
        .map_or(Colour::rgb(0x80, 0x80, 0x80), |slate| slate.base)
}

/// "1 tab", "5 tabs".
fn tabs_text(tabs: usize) -> String {
    if tabs == 1 {
        "1 tab".to_owned()
    } else {
        format!("{tabs} tabs")
    }
}

/// Milliseconds as seconds without trailing zeros: 250 is "0.25", 400 "0.4", 1000 "1".
fn seconds(ms: u16) -> String {
    let (whole, part) = (ms / 1000, ms % 1000);
    if part == 0 {
        return whole.to_string();
    }
    let digits = format!("{part:03}");
    format!("{whole}.{}", digits.trim_end_matches('0'))
}

/// A hold time as the window writes it: "0.4 s".
fn hold_text(ms: u16) -> String {
    format!("{} s", seconds(ms))
}

/// The label of Reset, from the first letters of the stored sequence as the settings read it:
/// "Reset to A, S, D, F, G…" ("…" only when more letters follow).
fn reset_label(sequence: &str) -> String {
    let keys = parse_sequence(sequence);
    if keys.is_empty() {
        return "Reset to no keys".to_owned();
    }
    let shown: Vec<String> = keys
        .iter()
        .take(RESET_LABEL_LETTERS)
        .map(|key| key.letter().to_string())
        .collect();
    let more = if keys.len() > RESET_LABEL_LETTERS {
        "\u{2026}"
    } else {
        ""
    };
    format!("Reset to {}{more}", shown.join(", "))
}

/// The capture radio of a mode: 0 outside text or hold, 1 outside text, 2 always.
fn capture_index(capture: KeyCapture) -> usize {
    match capture {
        KeyCapture::OutsideTextOrHold => 0,
        KeyCapture::OutsideText => 1,
        KeyCapture::Always => 2,
    }
}

/// The hold times offered, in milliseconds: 250, 400, 600, 800 and 1000, plus `current_ms` when
/// it is another value, in order (D8).
pub fn hold_choices(current_ms: u16) -> Vec<u16> {
    let mut choices = HOLD_CHOICES.to_vec();
    if let Err(at) = choices.binary_search(&current_ms) {
        choices.insert(at, current_ms);
    }
    choices
}

/// Chooses a capture mode: 0 outside text or hold, 1 outside text, 2 always. `false`, and no
/// change, for another index.
pub fn set_capture(keys: &mut GroupKeySettings, index: usize) -> bool {
    keys.capture = match index {
        0 => KeyCapture::OutsideTextOrHold,
        1 => KeyCapture::OutsideText,
        2 => KeyCapture::Always,
        _ => return false,
    };
    true
}

/// Chooses an item of [`hold_choices`] for `keys.hold_ms`. `false`, and no change, for an index
/// outside the list.
pub fn set_hold(keys: &mut GroupKeySettings, index: usize) -> bool {
    match hold_choices(keys.hold_ms).get(index) {
        Some(&ms) => {
            keys.hold_ms = ms;
            true
        }
        None => false,
    }
}
