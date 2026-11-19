// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Tab groups (groups-core lane): the model, with no tabs and no clock.
//!
//! The types and signatures below are the frozen contract of the lane
//! (`.superpowers/sdd/lane-groups-core-brief.md` §5); the `groups-model` item filled in the
//! behaviour. Nobody else edits this file: a change to a signature goes through the integrator.
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

use std::collections::HashSet;
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
        match self {
            Self::Preset(preset) => preset.name().to_owned(),
            Self::Custom(rgb) => format!("#{:02x}{:02x}{:02x}", rgb.red, rgb.green, rgb.blue),
        }
    }

    /// Reads the stored form; either case. `None` for anything else: a name that is not one of the
    /// presets (so a name of any length is refused), `#` with other than six hex digits, or text
    /// with spaces around it.
    pub fn parse(text: &str) -> Option<Self> {
        match text.strip_prefix('#') {
            Some(digits) => parse_hex_colour(digits).map(Self::Custom),
            None => GroupPreset::from_name(text).map(Self::Preset),
        }
    }
}

/// Six hex digits (either case) as a colour. Checked digit by digit: `u8::from_str_radix` would
/// also take a leading `+`.
fn parse_hex_colour(digits: &str) -> Option<Rgb> {
    let bytes = digits.as_bytes();
    if bytes.len() != 6 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let channel = |at: usize| (hex_digit(bytes[at]) << 4) | hex_digit(bytes[at + 1]);
    Some(Rgb {
        red: channel(0),
        green: channel(2),
        blue: channel(4),
    })
}

/// The value of an ASCII hex digit; the caller has checked that it is one.
fn hex_digit(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => byte - b'A' + 10,
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
        let mut keys = Vec::new();
        for letter in letters.chars() {
            if let Some(key) = Self::from_letter(letter)
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        keys
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
    // One pass that stops at the limit, so a huge input costs no more than a short one.
    let mut name = String::new();
    for letter in raw.chars().filter(|letter| !letter.is_control()) {
        if name.is_empty() && letter.is_whitespace() {
            continue;
        }
        if name.len() + letter.len_utf8() > MAX_GROUP_NAME_BYTES {
            break;
        }
        name.push(letter);
    }
    // The cut may have left white space at the end.
    name.truncate(name.trim_end().len());
    if name.is_empty() { None } else { Some(name) }
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
    /// An empty book. The key sequence is [`DEFAULT_GROUP_KEY_SEQUENCE`].
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            next_id: 1,
            key_sequence: GroupKey::parse_sequence(DEFAULT_GROUP_KEY_SEQUENCE),
            touched: Vec::new(),
        }
    }

    fn position(&self, id: GroupId) -> Option<usize> {
        self.groups.iter().position(|group| group.id == id)
    }

    /// The group that has `key`, if any.
    fn key_holder(&self, key: GroupKey) -> Option<GroupId> {
        self.groups
            .iter()
            .find(|group| group.key == Some(key))
            .map(|group| group.id)
    }

    /// The first letter of the key sequence that no group has.
    fn first_free_key(&self) -> Option<GroupKey> {
        self.key_sequence
            .iter()
            .copied()
            .find(|key| self.key_holder(*key).is_none())
    }

    /// Adds a group at the end under `id`, which the caller has checked is free and in range. A
    /// missing name or colour is the default one.
    fn push_group(
        &mut self,
        id: GroupId,
        name: Option<String>,
        colour: Option<GroupColour>,
        key: Option<GroupKey>,
        now: SystemTime,
    ) {
        let name = name.unwrap_or_else(|| default_group_name(&self.groups));
        let colour = colour.unwrap_or_else(|| default_colour(&self.groups));
        self.groups.push(TabGroup {
            id,
            name,
            colour,
            key,
            created: now,
            last_active: now,
        });
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
        if self.groups.len() >= MAX_GROUPS {
            return Err(GroupError::TooManyGroups);
        }
        let id = self.next_id;
        if id > MAX_GROUP_ID {
            return Err(GroupError::IdsExhausted);
        }
        let id = GroupId(id);
        let name = name.and_then(clean_group_name);
        let key = self.first_free_key();
        self.push_group(id, name, colour, key, now);
        self.next_id = id.0 + 1;
        Ok(id)
    }

    /// Like [`create`](Self::create) for a group read from a file: the id hint if free and in
    /// range, the key if free. A restored group takes no automatic key: one saved without a key
    /// (the user cleared it, or the sequence had run out) comes back without one, so saving and
    /// restoring change nothing.
    pub fn restore(&mut self, spec: RestoredGroup, now: SystemTime) -> Result<GroupId, GroupError> {
        if self.groups.len() >= MAX_GROUPS {
            return Err(GroupError::TooManyGroups);
        }
        // Ids start at 1 (0 is not a group), stay within Slint's `int`, and are never shared.
        let hint = spec
            .id_hint
            .filter(|hint| (1..=MAX_GROUP_ID).contains(hint) && self.get(GroupId(*hint)).is_none());
        let id = match hint {
            Some(hint) => hint,
            None if self.next_id <= MAX_GROUP_ID => self.next_id,
            None => return Err(GroupError::IdsExhausted),
        };
        let name = spec.name.as_deref().and_then(clean_group_name);
        let key = spec.key.filter(|key| self.key_holder(*key).is_none());
        self.push_group(GroupId(id), name, spec.colour, key, now);
        // A hint below the counter (an id that was freed) leaves it alone; one above raises it.
        self.next_id = self.next_id.max(id + 1);
        Ok(GroupId(id))
    }

    /// Takes a group out; its key is free again.
    pub fn remove(&mut self, id: GroupId) -> Option<TabGroup> {
        let index = self.position(id)?;
        self.touched.retain(|touched| *touched != id);
        Some(self.groups.remove(index))
    }

    pub fn rename(&mut self, id: GroupId, name: &str) -> Result<(), GroupError> {
        let index = self.position(id).ok_or(GroupError::NotFound)?;
        self.groups[index].name = clean_group_name(name).ok_or(GroupError::EmptyName)?;
        Ok(())
    }

    pub fn recolour(&mut self, id: GroupId, colour: GroupColour) -> Result<(), GroupError> {
        let index = self.position(id).ok_or(GroupError::NotFound)?;
        self.groups[index].colour = colour;
        Ok(())
    }

    /// `Err(KeyTaken)` when another group has `key`; the same group keeping its key is `Ok`.
    pub fn set_key(&mut self, id: GroupId, key: Option<GroupKey>) -> Result<(), GroupError> {
        let index = self.position(id).ok_or(GroupError::NotFound)?;
        if let Some(key) = key
            && let Some(holder) = self.key_holder(key)
            && holder != id
        {
            return Err(GroupError::KeyTaken { holder });
        }
        self.groups[index].key = key;
        Ok(())
    }

    /// Gives `key` to `id`; a group that had it gets `id`'s old key (or none).
    pub fn swap_key(&mut self, id: GroupId, key: GroupKey) -> Result<(), GroupError> {
        let index = self.position(id).ok_or(GroupError::NotFound)?;
        let old_key = self.groups[index].key;
        if let Some(holder) = self
            .groups
            .iter_mut()
            .find(|group| group.id != id && group.key == Some(key))
        {
            holder.key = old_key;
        }
        self.groups[index].key = Some(key);
        Ok(())
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
        let mut rest = std::mem::take(&mut self.groups);
        let mut named = Vec::with_capacity(rest.len());
        for id in order {
            // A repeated or unknown id finds nothing: the group is already out of `rest`.
            if let Some(index) = rest.iter().position(|group| group.id == *id) {
                named.push(rest.remove(index));
            }
        }
        named.extend(rest);
        self.groups = named;
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
        let touched = std::mem::take(&mut self.touched);
        for id in touched.into_iter().chain(active) {
            // A group that was closed since it was touched is simply not there.
            if let Some(group) = self.groups.iter_mut().find(|group| group.id == id) {
                group.last_active = now;
            }
        }
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
        let mut summary = Self::default();
        for status in statuses {
            summary.tabs += 1;
            summary.loading_tabs += usize::from(status.loading);
            summary.unread_tabs += usize::from(status.unread);
            summary.any_audible |= status.audible;
            summary.any_crashed |= status.crashed;
            summary.any_capture |= !status.capture.is_empty();
        }
        summary.any_loading = summary.loading_tabs > 0;
        summary.any_unread = summary.unread_tabs > 0;
        summary
    }
}

/// `Group N` for the smallest positive N that no group has as its exact name.
fn default_group_name(groups: &[TabGroup]) -> String {
    let taken: HashSet<&str> = groups.iter().map(|group| group.name.as_str()).collect();
    // At most `groups.len()` numbers are taken, so the search ends by `groups.len() + 1`.
    let mut number = 1usize;
    loop {
        let candidate = format!("Group {number}");
        if !taken.contains(candidate.as_str()) {
            return candidate;
        }
        number += 1;
    }
}

/// The first preset no group uses; with every preset in use, the least-used one, the earliest on
/// a tie. A custom colour uses no preset.
fn default_colour(groups: &[TabGroup]) -> GroupColour {
    let uses = |preset: &GroupPreset| {
        groups
            .iter()
            .filter(|group| group.colour == GroupColour::Preset(*preset))
            .count()
    };
    // `min_by_key` keeps the first of equal minimums, which is the earliest preset.
    let preset = GroupPreset::ALL
        .iter()
        .min_by_key(|preset| uses(preset))
        .copied()
        .unwrap_or(GroupPreset::Blue);
    GroupColour::Preset(preset)
}

/// How far a page load has probably got after `elapsed`, in percent: 0 at the start, rising
/// without ever passing 90 (the interface shows 100 and fades when loading ends). Servo reports no
/// load percentage, so this is an easing curve, not a measurement.
pub fn load_progress_estimate(elapsed: Duration) -> u8 {
    // `as` saturates, and the curve is already within 0 to 90.
    progress_curve(elapsed.as_secs_f64()).round() as u8
}

/// The most the estimate ever shows, in percent.
const PROGRESS_CEILING: f64 = 90.0;
/// How fast the estimate rises: after this many seconds it is at 63% of the ceiling.
const PROGRESS_TIME_CONSTANT_SECONDS: f64 = 1.5;

/// `90 * (1 - exp(-t / 1.5 s))`: 0 at 0, rising ever more slowly towards 90.
fn progress_curve(seconds: f64) -> f64 {
    // `exp_m1(x)` is `exp(x) - 1` without losing the small values to rounding.
    let share = -(-seconds / PROGRESS_TIME_CONSTANT_SECONDS).exp_m1();
    (PROGRESS_CEILING * share).clamp(0.0, PROGRESS_CEILING)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, colour: GroupColour) -> TabGroup {
        TabGroup {
            id: GroupId::new(1),
            name: name.to_owned(),
            colour,
            key: None,
            created: SystemTime::UNIX_EPOCH,
            last_active: SystemTime::UNIX_EPOCH,
        }
    }

    fn preset_group(name: &str, preset: GroupPreset) -> TabGroup {
        group(name, GroupColour::Preset(preset))
    }

    /// The obvious way to clean a name, in four steps; `clean_group_name` does it in one pass.
    fn clean_in_four_steps(raw: &str) -> Option<String> {
        let without_controls: String = raw.chars().filter(|letter| !letter.is_control()).collect();
        let trimmed = without_controls.trim();
        let mut end = trimmed.len().min(MAX_GROUP_NAME_BYTES);
        while !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        let cut = trimmed[..end].trim_end();
        if cut.is_empty() {
            None
        } else {
            Some(cut.to_owned())
        }
    }

    #[test]
    fn the_one_pass_cleaning_matches_the_four_step_one() {
        let pieces = [
            "a", " ", "é", "😀", "\n", "\u{0}", "\u{a0}", "日本", "Group 1", "\t", "xxxxxxxx",
        ];
        // Every string of up to four pieces, and each repeated until it is far past the limit.
        let mut cases = vec![String::new()];
        for first in pieces {
            cases.push(first.repeat(30));
            for second in pieces {
                cases.push(format!("{first}{second}"));
                cases.push(format!("{}{}", first.repeat(9), second.repeat(9)));
                for third in pieces {
                    cases.push(format!("{first}{second}{third}"));
                    cases.push(format!(
                        "{}{}{}",
                        first.repeat(5),
                        second.repeat(7),
                        third.repeat(11)
                    ));
                }
            }
        }
        for case in cases {
            assert_eq!(
                clean_group_name(&case),
                clean_in_four_steps(&case),
                "{case:?}"
            );
        }
    }

    #[test]
    fn a_huge_name_is_cut_without_reading_the_rest() {
        let huge = format!("  {}", "é".repeat(1_000_000));
        let cut = clean_group_name(&huge).expect("something is left");
        assert_eq!(cut, "é".repeat(32));
    }

    #[test]
    fn hex_digits_have_their_value() {
        let digits = b"0123456789abcdefABCDEF";
        let values = [
            0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 10, 11, 12, 13, 14, 15,
        ];
        for (digit, value) in digits.iter().zip(values) {
            assert_eq!(hex_digit(*digit), value, "{}", *digit as char);
        }
    }

    #[test]
    fn six_hex_digits_are_a_colour_and_nothing_else_is() {
        assert_eq!(
            parse_hex_colour("2f6fd6"),
            Some(Rgb {
                red: 0x2f,
                green: 0x6f,
                blue: 0xd6
            })
        );
        assert_eq!(
            parse_hex_colour("FFFFFF"),
            Some(Rgb {
                red: 255,
                green: 255,
                blue: 255
            })
        );
        for text in [
            "", "12345", "1234567", "+12345", "-12345", "12345g", "é2345", "12 456",
        ] {
            assert_eq!(parse_hex_colour(text), None, "{text:?}");
        }
    }

    #[test]
    fn the_default_name_fills_the_smallest_gap() {
        assert_eq!(default_group_name(&[]), "Group 1");
        let colour = GroupColour::Preset(GroupPreset::Blue);
        let taken = |names: &[&str]| -> Vec<TabGroup> {
            names.iter().map(|name| group(name, colour)).collect()
        };
        assert_eq!(default_group_name(&taken(&["Group 1"])), "Group 2");
        assert_eq!(
            default_group_name(&taken(&["Group 2", "Group 3"])),
            "Group 1"
        );
        assert_eq!(
            default_group_name(&taken(&["Group 1", "Group 3"])),
            "Group 2"
        );
        assert_eq!(
            default_group_name(&taken(&["group 1", "Group  1", "x"])),
            "Group 1"
        );
        assert_eq!(
            default_group_name(&taken(&["Group 1", "Group 1"])),
            "Group 2"
        );
        let all: Vec<TabGroup> = (1..=MAX_GROUPS)
            .map(|number| group(&format!("Group {number}"), colour))
            .collect();
        assert_eq!(default_group_name(&all), "Group 65");
    }

    #[test]
    fn the_default_colour_is_the_first_unused_preset() {
        assert_eq!(default_colour(&[]), GroupColour::Preset(GroupPreset::Blue));
        let used = [
            preset_group("a", GroupPreset::Blue),
            preset_group("b", GroupPreset::Violet),
        ];
        assert_eq!(
            default_colour(&used),
            GroupColour::Preset(GroupPreset::Amber)
        );
        let custom = [group(
            "c",
            GroupColour::Custom(Rgb {
                red: 1,
                green: 2,
                blue: 3,
            }),
        )];
        assert_eq!(
            default_colour(&custom),
            GroupColour::Preset(GroupPreset::Blue)
        );
    }

    #[test]
    fn the_default_colour_with_all_presets_used_is_the_least_used_then_the_earliest() {
        let mut groups: Vec<TabGroup> = GroupPreset::ALL
            .iter()
            .map(|preset| preset_group("g", *preset))
            .collect();
        assert_eq!(
            default_colour(&groups),
            GroupColour::Preset(GroupPreset::Blue)
        );
        // Blue and Amber twice: Violet is the earliest of the once-used ones.
        groups.push(preset_group("g", GroupPreset::Blue));
        groups.push(preset_group("g", GroupPreset::Amber));
        assert_eq!(
            default_colour(&groups),
            GroupColour::Preset(GroupPreset::Violet)
        );
        // Everything twice except Brown: the least used wins over the earliest.
        for preset in &GroupPreset::ALL[2..13] {
            groups.push(preset_group("g", *preset));
        }
        assert_eq!(
            default_colour(&groups),
            GroupColour::Preset(GroupPreset::Brown)
        );
    }

    #[test]
    fn the_curve_is_zero_at_zero_and_settles_at_the_ceiling() {
        assert_eq!(progress_curve(0.0), 0.0);
        assert!(progress_curve(0.001) > 0.0);
        assert!((progress_curve(1.5) - 90.0 * (1.0 - (-1.0f64).exp())).abs() < 1e-9);
        assert_eq!(progress_curve(1e6), 90.0);
        assert_eq!(progress_curve(f64::MAX), 90.0);
        assert_eq!(progress_curve(f64::INFINITY), 90.0);
        // A clock that ran backwards cannot make it negative.
        assert_eq!(progress_curve(-5.0), 0.0);
    }

    #[test]
    fn the_first_free_key_follows_the_sequence() {
        let mut book = GroupBook::new();
        assert_eq!(book.first_free_key().map(GroupKey::letter), Some('A'));
        let first = book
            .create(None, None, SystemTime::UNIX_EPOCH)
            .expect("created");
        assert_eq!(
            book.key_holder(GroupKey::from_letter('a').unwrap()),
            Some(first)
        );
        assert_eq!(book.key_holder(GroupKey::from_letter('s').unwrap()), None);
        assert_eq!(book.first_free_key().map(GroupKey::letter), Some('S'));
        book.set_key_sequence(Vec::new());
        assert_eq!(book.first_free_key(), None);
    }
}
