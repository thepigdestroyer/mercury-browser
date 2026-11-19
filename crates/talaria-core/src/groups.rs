// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Tab groups (groups-core lane): the model, with no tabs and no clock.
//!
//! **SKELETON.** The types and signatures below are the frozen contract of the lane
//! (`.superpowers/sdd/lane-groups-core-brief.md` §5). Types are final; functions that
//! say `todo!("groups-model")` are filled in by the `groups-model` item, which also writes
//! the tests. Nobody else edits this file: a change to a signature goes through the integrator.
//!
//! What lives here:
//!
//! - [`GroupId`], [`GroupColour`] (14 named presets or a custom colour), [`GroupKey`] (a physical
//!   letter A..Z), [`TabGroup`] and [`GroupError`];
//! - [`GroupBook`], the ordered set of one window's groups: ids, names, colours, keys, limits and
//!   "last active" stamps. It knows nothing about tabs; the controller keeps the strip and the
//!   book in step (`controller/group_ops.rs`);
//! - [`TabStatus`] and [`GroupSummary`], the flags the interface shows, and
//!   [`load_progress_estimate`] (Servo reports no load percentage).
//!
//! `talaria-core` never reads the clock: every time here is handed in.

use std::fmt;
use std::time::{Duration, SystemTime};

use crate::{CaptureAccess, Rgb, TabId};

/// The most groups one window holds.
pub const MAX_GROUPS: usize = 64;
/// The longest group name, in bytes.
pub const MAX_GROUP_NAME_BYTES: usize = 64;
/// The largest group id. Slint's `int` carries ids, so they stay within `i32`.
pub const MAX_GROUP_ID: u64 = i32::MAX as u64;
/// The letters given to new groups, in creation order, until another sequence is set.
pub const DEFAULT_GROUP_KEY_SEQUENCE: &str = "ASDFGHJKL";

/// Names a group for the life of a profile: ids are never reused (the next id is saved in the
/// session file) and never exceed [`MAX_GROUP_ID`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupId(u64);

impl GroupId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

/// The colours a group can have out of the box. Only the name is stored: what each one looks like
/// is palette data (`mercury-appearance`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupPreset {
    Blue,
    Amber,
    Violet,
    Rose,
    Green,
    Teal,
    Slate,
    Red,
    Orange,
    Lime,
    Cyan,
    Indigo,
    Magenta,
    Brown,
}

impl GroupPreset {
    /// Every preset, in the order a new group takes them.
    pub const ALL: [Self; 14] = [
        Self::Blue,
        Self::Amber,
        Self::Violet,
        Self::Rose,
        Self::Green,
        Self::Teal,
        Self::Slate,
        Self::Red,
        Self::Orange,
        Self::Lime,
        Self::Cyan,
        Self::Indigo,
        Self::Magenta,
        Self::Brown,
    ];

    /// The stored name: lower case, for example `"blue"`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Amber => "amber",
            Self::Violet => "violet",
            Self::Rose => "rose",
            Self::Green => "green",
            Self::Teal => "teal",
            Self::Slate => "slate",
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Lime => "lime",
            Self::Cyan => "cyan",
            Self::Indigo => "indigo",
            Self::Magenta => "magenta",
            Self::Brown => "brown",
        }
    }

    /// The preset called `name`, ignoring case.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|preset| preset.name().eq_ignore_ascii_case(name))
    }
}

/// A group's colour: one of the presets, or a colour the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupColour {
    Preset(GroupPreset),
    Custom(Rgb),
}

impl GroupColour {
    /// The stored form: a preset's name (`"blue"`) or `"#rrggbb"` in lower case.
    pub fn to_name(self) -> String {
        todo!("groups-model")
    }

    /// Reads the stored form; either case. `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        let _ = text;
        todo!("groups-model")
    }
}

/// A group's key: the physical letter that, with ⌥, switches to the group. Stored as the winit
/// physical-key name (`"KeyA"`), so changing the keyboard layout does not move it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupKey(u8);

const KEY_NAMES: [&str; 26] = [
    "KeyA", "KeyB", "KeyC", "KeyD", "KeyE", "KeyF", "KeyG", "KeyH", "KeyI", "KeyJ", "KeyK", "KeyL",
    "KeyM", "KeyN", "KeyO", "KeyP", "KeyQ", "KeyR", "KeyS", "KeyT", "KeyU", "KeyV", "KeyW", "KeyX",
    "KeyY", "KeyZ",
];

impl GroupKey {
    /// 0 is A, 25 is Z.
    pub const fn from_index(index: u8) -> Option<Self> {
        if index < 26 { Some(Self(index)) } else { None }
    }

    /// `'a'..='z'` or `'A'..='Z'`.
    pub fn from_letter(letter: char) -> Option<Self> {
        let letter = letter.to_ascii_uppercase();
        if letter.is_ascii_uppercase() {
            Self::from_index(letter as u8 - b'A')
        } else {
            None
        }
    }

    /// The capital letter.
    pub const fn letter(self) -> char {
        (b'A' + self.0) as char
    }

    /// The stored name, `"KeyA"` to `"KeyZ"`.
    pub const fn code_name(self) -> &'static str {
        KEY_NAMES[self.0 as usize]
    }

    /// The key stored as `name`; exactly `"KeyA"` to `"KeyZ"`.
    pub fn from_code_name(name: &str) -> Option<Self> {
        KEY_NAMES
            .iter()
            .position(|candidate| *candidate == name)
            .and_then(|index| Self::from_index(index as u8))
    }

    /// The letters of `letters` as keys, ignoring anything that is not a letter; a letter that
    /// repeats counts at its first place.
    pub fn parse_sequence(letters: &str) -> Vec<Self> {
        let _ = letters;
        todo!("groups-model")
    }
}

/// One tab group of a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabGroup {
    pub id: GroupId,
    /// 1 to [`MAX_GROUP_NAME_BYTES`] bytes, trimmed, without control characters. Not unique.
    pub name: String,
    pub colour: GroupColour,
    /// Unique among the window's groups.
    pub key: Option<GroupKey>,
    /// When the group was made (or restored); not saved.
    pub created: SystemTime,
    /// The last time one of its tabs was shown, as of the last
    /// [`stamp_group_activity`](crate::BrowserController::stamp_group_activity); not saved.
    pub last_active: SystemTime,
}

/// Why a group operation was refused. A refused operation changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupError {
    /// There is no such group.
    NotFound,
    /// There is no such tab.
    TabNotFound,
    /// Pinned tabs are never in a group.
    TabPinned,
    /// A group needs at least one tab.
    NoTabs,
    /// A window holds at most [`MAX_GROUPS`] groups.
    TooManyGroups,
    /// A name with nothing left after cleaning.
    EmptyName,
    /// Another group has that key.
    KeyTaken { holder: GroupId },
    /// Group ids are used up ([`MAX_GROUP_ID`]).
    IdsExhausted,
}

impl fmt::Display for GroupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("there is no such tab group"),
            Self::TabNotFound => formatter.write_str("there is no such tab"),
            Self::TabPinned => formatter.write_str("a pinned tab cannot be in a group"),
            Self::NoTabs => formatter.write_str("a tab group needs at least one tab"),
            Self::TooManyGroups => write!(formatter, "a window holds at most {MAX_GROUPS} groups"),
            Self::EmptyName => formatter.write_str("a tab group needs a name"),
            Self::KeyTaken { .. } => formatter.write_str("another tab group has that key"),
            Self::IdsExhausted => formatter.write_str("tab group ids are used up"),
        }
    }
}

impl std::error::Error for GroupError {}

/// A group name as it is stored: control characters removed, trimmed, cut to
/// [`MAX_GROUP_NAME_BYTES`] at a character boundary. `None` when nothing is left.
pub fn clean_group_name(raw: &str) -> Option<String> {
    let _ = raw;
    todo!("groups-model")
}

/// A group read from a saved session (or a closed tab), to be made again.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RestoredGroup {
    /// The saved id, honoured only if it is free and at most [`MAX_GROUP_ID`].
    pub id_hint: Option<u64>,
    /// `None` or an empty name gives the default `Group N`.
    pub name: Option<String>,
    /// `None` gives the default colour.
    pub colour: Option<GroupColour>,
    /// Kept only if no group has it.
    pub key: Option<GroupKey>,
}

/// What `close_group` closed, for the session's closed-tab list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedGroup {
    /// The group as it was.
    pub group: TabGroup,
    /// Its tabs, in strip order.
    pub tabs: Vec<TabId>,
    /// The blank tab opened because the group held every tab of the window.
    pub opened_blank: Option<TabId>,
}

/// The groups of one window, in strip order. Knows nothing about tabs.
#[derive(Debug, Clone)]
pub struct GroupBook {
    groups: Vec<TabGroup>,
    next_id: u64,
    key_sequence: Vec<GroupKey>,
    /// Groups that were active since the last stamp.
    touched: Vec<GroupId>,
}

impl Default for GroupBook {
    fn default() -> Self {
        Self::new()
    }
}

impl GroupBook {
    /// An empty book. The key sequence is [`DEFAULT_GROUP_KEY_SEQUENCE`] (the skeleton leaves it
    /// empty until `groups-model` parses it).
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            next_id: 1,
            key_sequence: Vec::new(),
            touched: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.groups.len()
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The groups, in strip order.
    pub fn as_slice(&self) -> &[TabGroup] {
        &self.groups
    }

    pub fn get(&self, id: GroupId) -> Option<&TabGroup> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// Makes a group at the end: the default name, colour and key unless given. A name that
    /// cleans to nothing is the default.
    pub fn create(
        &mut self,
        name: Option<&str>,
        colour: Option<GroupColour>,
        now: SystemTime,
    ) -> Result<GroupId, GroupError> {
        let _ = (name, colour, now);
        todo!("groups-model")
    }

    /// Like [`create`](Self::create) for a group read from a file: the id hint if free and in
    /// range, the key if free.
    pub fn restore(&mut self, spec: RestoredGroup, now: SystemTime) -> Result<GroupId, GroupError> {
        let _ = (spec, now);
        todo!("groups-model")
    }

    /// Takes a group out; its key is free again.
    pub fn remove(&mut self, id: GroupId) -> Option<TabGroup> {
        let _ = id;
        todo!("groups-model")
    }

    pub fn rename(&mut self, id: GroupId, name: &str) -> Result<(), GroupError> {
        let _ = (id, name);
        todo!("groups-model")
    }

    pub fn recolour(&mut self, id: GroupId, colour: GroupColour) -> Result<(), GroupError> {
        let _ = (id, colour);
        todo!("groups-model")
    }

    /// `Err(KeyTaken)` when another group has `key`; the same group keeping its key is `Ok`.
    pub fn set_key(&mut self, id: GroupId, key: Option<GroupKey>) -> Result<(), GroupError> {
        let _ = (id, key);
        todo!("groups-model")
    }

    /// Gives `key` to `id`; a group that had it gets `id`'s old key (or none).
    pub fn swap_key(&mut self, id: GroupId, key: GroupKey) -> Result<(), GroupError> {
        let _ = (id, key);
        todo!("groups-model")
    }

    /// The letters new groups take, in order. Empty turns automatic keys off.
    pub fn set_key_sequence(&mut self, keys: Vec<GroupKey>) {
        self.key_sequence = keys;
    }

    pub fn key_sequence(&self) -> &[GroupKey] {
        &self.key_sequence
    }

    /// Puts the groups in the order of `order`; groups it does not name keep their relative order
    /// after the named ones; unknown ids are ignored.
    pub fn reorder_to_match(&mut self, order: &[GroupId]) {
        let _ = order;
        todo!("groups-model")
    }

    /// Notes that the group was active since the last stamp.
    pub fn touch(&mut self, id: GroupId) {
        if !self.touched.contains(&id) {
            self.touched.push(id);
        }
    }

    /// Sets `last_active` to `now` for every touched group and for `active`, then forgets the
    /// touches.
    pub fn stamp_activity(&mut self, now: SystemTime, active: Option<GroupId>) {
        let _ = (now, active);
        todo!("groups-model")
    }

    /// The id the next [`create`](Self::create) uses (at least 1).
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Makes sure the next id is at least `floor`, but never above [`MAX_GROUP_ID`].
    pub fn raise_next_id(&mut self, floor: u64) {
        self.next_id = self.next_id.max(floor.min(MAX_GROUP_ID));
    }
}

/// What the interface shows for a tab. Computed on demand, except `unread` and `crashed`, which
/// the tab stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TabStatus {
    /// The page is loading (an estimate of how far: [`load_progress_estimate`]).
    pub loading: bool,
    /// Playing sound the user can hear.
    pub audible: bool,
    /// The user muted the tab.
    pub muted: bool,
    /// Something happened in the background since the user last saw the tab.
    pub unread: bool,
    pub pinned: bool,
    /// The page's process crashed.
    pub crashed: bool,
    /// Camera or microphone the page may use.
    pub capture: CaptureAccess,
}

/// The flags of a group's tabs together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GroupSummary {
    pub tabs: usize,
    pub loading_tabs: usize,
    pub unread_tabs: usize,
    pub any_loading: bool,
    pub any_audible: bool,
    pub any_unread: bool,
    pub any_crashed: bool,
    pub any_capture: bool,
}

impl GroupSummary {
    pub fn from_statuses(statuses: impl IntoIterator<Item = TabStatus>) -> Self {
        let _ = statuses.into_iter();
        todo!("groups-model")
    }
}

/// How far a page load has probably got after `elapsed`, in percent: 0 at the start, rising
/// without ever passing 90 (the interface shows 100 and fades when loading ends). Servo reports no
/// load percentage, so this is an easing curve, not a measurement.
pub fn load_progress_estimate(elapsed: Duration) -> u8 {
    let _ = elapsed;
    todo!("groups-model")
}
