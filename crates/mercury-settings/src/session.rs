// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The session snapshot that restores windows and tabs, and the throttle that decides when to
//! write it.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::bounded;
use crate::settings::{MAX_ZOOM_PERCENT, MIN_ZOOM_PERCENT, session_url, truncated};

/// How many recently closed tabs are remembered (as Chrome's and Firefox's defaults).
pub const MAX_CLOSED_TABS: usize = 25;
/// How many crashed sessions the recovery list keeps until the user deals with them.
///
/// A crashed session is added only if it has a tab other than a new-tab page (`about:blank`,
/// `about:newtab`, `about:home`) and its windows differ from every entry already held. When
/// the list is full, the oldest entry is kept — usually the session the user most wants, from
/// before a run of crashes — and the newest fill the rest: the entry dropped is the oldest of
/// the others.
pub const MAX_RECOVERY_SESSIONS: usize = 3;
/// How many tabs are restored, across all windows.
pub const MAX_RESTORED_TABS: usize = 500;
/// The longest tab title kept, in bytes.
pub const MAX_TITLE_BYTES: usize = 1024;
/// The smallest restored window side, in logical pixels.
pub const MIN_WINDOW_SIDE: u32 = 100;
/// The largest restored window side, in logical pixels.
pub const MAX_WINDOW_SIDE: u32 = 32_768;
/// The furthest a restored window's corner may be from the origin on either axis, in logical
/// pixels.
pub const MAX_WINDOW_COORDINATE: u32 = 65_536;
/// How many tab groups one window keeps.
///
/// Equal to the limit of the browser core's group book; a test in the desktop application pins
/// the two together.
pub const MAX_GROUPS_PER_WINDOW: usize = 64;
/// The longest group name kept, in bytes.
pub const MAX_GROUP_NAME_BYTES: usize = 64;

/// The largest value the saved group-id counter may have: the interface carries group ids as
/// `i32`.
const MAX_GROUP_COUNTER: u64 = i32::MAX as u64;
/// The corners the dial may sit in, as saved.
const DIAL_CORNERS: [&str; 4] = ["top_left", "top_right", "bottom_left", "bottom_right"];

/// Everything needed to put the windows and tabs back.
///
/// Its `Deserialize` is bounded, for any caller: at most [`MAX_RESTORED_TABS`] windows and
/// [`MAX_CLOSED_TABS`] closed tabs are kept and the rest of each list is skipped unread (so are
/// the groups of a window beyond [`MAX_GROUPS_PER_WINDOW`]); a tab URL longer than
/// [`crate::MAX_URL_BYTES`] is read as empty and a title is cut to [`MAX_TITLE_BYTES`] (see
/// [`TabSnapshot`]; the strings of a group are cut only when sanitizing, so they are bounded
/// by the file's size limit until then). Within [`crate::SessionStore`] the
/// [`MAX_RESTORED_TABS`] tab cap is shared by all the windows of a file; a caller decoding a
/// snapshot directly gets that cap per window instead. Decoding never sanitizes: call
/// [`sanitized`](Self::sanitized).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSnapshot {
    /// The open windows, front to back as the caller chooses.
    #[serde(deserialize_with = "bounded::windows")]
    pub windows: Vec<WindowSnapshot>,
    /// Recently closed tabs, newest first.
    #[serde(deserialize_with = "bounded::closed_tabs")]
    pub closed_tabs: Vec<ClosedTab>,
    /// When the snapshot was taken, in milliseconds since the Unix epoch.
    pub saved_at_ms: i64,
}

/// One window.
///
/// Its `Deserialize` keeps at most [`MAX_RESTORED_TABS`] tabs (shared with the other windows
/// of the same file when read by [`crate::SessionStore`]) and skips the rest unread; it keeps
/// at most [`MAX_GROUPS_PER_WINDOW`] groups and skips the rest unread too.
///
/// The fields about groups are written only when they say something, so a window without
/// groups is saved as it was before groups existed, and a file saved by a browser that does not
/// know groups reads with none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSnapshot {
    /// The tabs, in strip order.
    #[serde(deserialize_with = "bounded::tabs")]
    pub tabs: Vec<TabSnapshot>,
    /// The index of the selected tab in `tabs`.
    pub active_tab: usize,
    /// Where the window was, if known and sensible.
    pub bounds: Option<WindowBounds>,
    /// Whether it was maximised (zoomed, on macOS).
    pub maximized: bool,
    /// Whether it was full screen.
    pub fullscreen: bool,
    /// The tab groups, in the order the interface shows them (the order of their tabs in the
    /// strip). A tab names its group by [`TabSnapshot::group`]. See [`GroupSnapshot`].
    #[serde(
        deserialize_with = "bounded::groups",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub groups: Vec<GroupSnapshot>,
    /// The id the next new group of this window would get: above every id handed out so far,
    /// so that the id of a group that no longer exists (in a [`ClosedTab`], say) can never name
    /// a different group later. Zero in a file without groups.
    #[serde(skip_serializing_if = "is_zero")]
    pub next_group_id: u64,
    /// Whether the group dock was shrunk. Saved and restored, not interpreted by this crate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dock_shrunk: Option<bool>,
    /// The corner the group dial sat in: `top_left`, `top_right`, `bottom_left` or
    /// `bottom_right`. Saved and restored, not interpreted by this crate; any other value is
    /// dropped by [`SessionSnapshot::sanitized`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dial_corner: Option<String>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// One tab group of a window.
///
/// The session crate does not know what the colours and keys mean (the browser core does): a
/// colour is a lower-case name such as `blue` or a hex colour such as `#2f6fd6`, a key is the
/// name of a physical key, `KeyA` to `KeyZ`. [`SessionSnapshot::sanitized`] keeps only values of
/// those shapes and lets the application decide what to make of them (an unknown colour name
/// restores as the default colour).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupSnapshot {
    /// The group's id, unique within its window. Tabs name it in [`TabSnapshot::group`]. An id
    /// is a hint when restoring: the application may give the group another one.
    pub id: u64,
    /// The name shown to the user: at most [`MAX_GROUP_NAME_BYTES`] bytes, never empty after
    /// sanitizing.
    pub name: String,
    /// The colour: a lower-case name of 1 to 16 letters, or `#` and six lower-case hex digits,
    /// or empty for "the default colour".
    pub colour: String,
    /// The group's key, `KeyA` to `KeyZ`, unique within the window, if it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// A window's position and size, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowBounds {
    /// The left edge.
    pub x: i32,
    /// The top edge.
    pub y: i32,
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
}

/// One tab.
///
/// Its `Deserialize` reads a URL longer than [`crate::MAX_URL_BYTES`] as empty (so sanitizing
/// drops the tab) and cuts a title to [`MAX_TITLE_BYTES`] at a character boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabSnapshot {
    /// The page's URL. A new-tab page must be an `about:` URL (for example `about:newtab`) to
    /// survive a restore.
    #[serde(deserialize_with = "bounded::url")]
    pub url: String,
    /// The page's title.
    #[serde(deserialize_with = "bounded::title")]
    pub title: String,
    /// The tab's zoom, in percent.
    pub zoom_percent: u16,
    /// Whether the tab is pinned.
    pub pinned: bool,
    /// The id, in the window's [`WindowSnapshot::groups`], of the group the tab belongs to. A
    /// pinned tab never has one; one naming no group is read as none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<u64>,
}

impl Default for TabSnapshot {
    fn default() -> Self {
        Self {
            url: String::new(),
            title: String::new(),
            zoom_percent: 100,
            pinned: false,
            group: None,
        }
    }
}

/// A tab that was closed, and where it was.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClosedTab {
    /// The tab. After sanitizing its [`TabSnapshot::group`] is the id of
    /// [`group`](Self::group), or none.
    pub tab: TabSnapshot,
    /// The index of the window it was in, at the time. A hint: the window may be gone.
    pub window: usize,
    /// Its index in that window's strip, at the time. A hint: clamp before use.
    pub index: usize,
    /// When it was closed, in milliseconds since the Unix epoch.
    pub closed_at_ms: i64,
    /// The group the tab was in when it was closed, as it was then, so that reopening the tab
    /// can put it back in that group or make the group again. Tabs closed together with their
    /// group share this group's id and `closed_at_ms`: see
    /// [`SessionSnapshot::pop_closed_run`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupSnapshot>,
}

impl SessionSnapshot {
    /// Drops what must not or cannot be restored and clamps the rest:
    ///
    /// - Tabs are kept only if their URL is `http`, `https`, `file` or `about` and at most
    ///   [`crate::MAX_URL_BYTES`] long before and after parsing; `javascript:`, `data:`, `blob:`,
    ///   other schemes and unparsable URLs are dropped. The URL kept is the parsed URL's text,
    ///   with any user name and password removed. That is all that is removed: a token in the
    ///   query string or the fragment (`?token=…`, `#access_token=…`) is kept, so a session
    ///   file can still hold secrets a site put in its URLs. Titles are cut to
    ///   [`MAX_TITLE_BYTES`] and zoom clamped to [`MIN_ZOOM_PERCENT`]..=[`MAX_ZOOM_PERCENT`].
    /// - Windows left without tabs are dropped. `active_tab` is clamped into range; if the
    ///   active tab itself was dropped, the nearest surviving tab to its right becomes active,
    ///   or failing that the nearest to its left.
    /// - Window bounds with a side outside [`MIN_WINDOW_SIDE`]..=[`MAX_WINDOW_SIDE`] or a
    ///   corner further than [`MAX_WINDOW_COORDINATE`] from the origin become `None`.
    /// - At most [`MAX_RESTORED_TABS`] tabs survive across all windows: first every pinned
    ///   tab (in strip order), then each window's active tab, then the rest by their distance
    ///   from the end of their window's strip — where new tabs open, so the most recently
    ///   opened go first — taking the windows in turn. Kept tabs stay in their original order.
    /// - At most [`MAX_CLOSED_TABS`] closed tabs survive, the newest; closed tabs with a URL
    ///   that would be dropped are dropped first.
    /// - Groups are settled last, in each window, after the URL drops and the tab cap have
    ///   removed tabs, so that no group is left without a tab:
    ///   1. at most [`MAX_GROUPS_PER_WINDOW`] groups are kept and a group whose `id` repeats an
    ///      earlier one is dropped;
    ///   2. a tab's group is cleared if it names no group, or the tab is pinned;
    ///   3. a group with no tab left is dropped;
    ///   4. a group's name loses its control characters and outer white space, is cut to
    ///      [`MAX_GROUP_NAME_BYTES`] at a character boundary and, if nothing is left, becomes
    ///      `Group N`, N being the group's position (from 1) among the groups kept;
    ///   5. a colour is kept only as a lower-case ASCII name of 1 to 16 letters or `#` and six
    ///      lower-case hex digits (upper case is lowered), otherwise it becomes empty; the
    ///      colour names themselves are not checked here;
    ///   6. a key is kept only as `KeyA` to `KeyZ`, and cleared on a group that comes after
    ///      another one with the same key;
    ///   7. `next_group_id` becomes the largest of itself and one above the highest id of the
    ///      groups kept, but at most `i32::MAX` (the interface carries ids as `i32`); a window
    ///      without groups keeps its value, within that limit;
    ///   8. `dial_corner` is kept only if it is `top_left`, `top_right`, `bottom_left` or
    ///      `bottom_right`.
    /// - A closed tab's group follows rules 4 to 6, an empty name becoming `Group 1`; the closed
    ///   tab's own [`TabSnapshot::group`] is set to the group's id, or to none when the closed
    ///   tab has no group or was pinned (a pinned tab is never in a group).
    ///
    /// Sanitizing the result again changes nothing.
    pub fn sanitized(self) -> Self {
        let windows: Vec<WindowSnapshot> = self
            .windows
            .into_iter()
            .filter_map(WindowSnapshot::sanitized)
            .collect();
        let closed_tabs = self
            .closed_tabs
            .into_iter()
            .filter_map(ClosedTab::sanitized)
            .take(MAX_CLOSED_TABS)
            .collect();
        Self {
            // The group rules come last: the cap is the final thing that removes tabs.
            windows: cap_tabs(windows, MAX_RESTORED_TABS)
                .into_iter()
                .map(WindowSnapshot::settled_groups)
                .collect(),
            closed_tabs,
            saved_at_ms: self.saved_at_ms,
        }
    }

    /// Records a closed tab, newest first, bounded by `MAX_CLOSED_TABS`.
    pub fn push_closed(&mut self, tab: ClosedTab) {
        self.closed_tabs.insert(0, tab);
        self.closed_tabs.truncate(MAX_CLOSED_TABS);
    }

    /// The most recently closed tab, removed from the list ("reopen closed tab").
    ///
    /// Its `window` and `index` say where it was; the caller clamps them to what exists now.
    pub fn pop_closed(&mut self) -> Option<ClosedTab> {
        (!self.closed_tabs.is_empty()).then(|| self.closed_tabs.remove(0))
    }

    /// What one "reopen closed tab" brings back: the most recently closed tab, and, when it was
    /// in a group, every entry directly after it that has the same group id and the same
    /// `closed_at_ms` — the tabs that were closed together with their group. All of them are
    /// removed from the list.
    ///
    /// A tab without a group, a lone tab of a group, and a tab whose neighbour in the list is
    /// of another group, of no group or of another time, pop alone. The run is returned with
    /// the lowest strip `index` first (the order to open the tabs in); the caller clamps the
    /// indices to what exists now. Empty if there are no closed tabs.
    pub fn pop_closed_run(&mut self) -> Vec<ClosedTab> {
        let Some(newest) = self.closed_tabs.first() else {
            return Vec::new();
        };
        let run = match newest.group.as_ref().map(|group| group.id) {
            None => 1,
            Some(id) => {
                let closed_at_ms = newest.closed_at_ms;
                let followers = self.closed_tabs[1..].iter().take_while(|closed| {
                    closed.closed_at_ms == closed_at_ms
                        && closed.group.as_ref().is_some_and(|group| group.id == id)
                });
                1 + followers.count()
            }
        };
        let mut popped: Vec<ClosedTab> = self.closed_tabs.drain(..run).collect();
        // A stable sort: entries with the same index keep the order they had in the list.
        popped.sort_by_key(|closed| closed.index);
        popped
    }
}

impl ClosedTab {
    fn sanitized(self) -> Option<Self> {
        let mut tab = self.tab.sanitized()?;
        let group = if tab.pinned {
            None
        } else {
            self.group.map(|mut group| {
                group.settle(0);
                group
            })
        };
        tab.group = group.as_ref().map(|group| group.id);
        Some(Self { tab, group, ..self })
    }
}

impl GroupSnapshot {
    /// The name, colour and key rules, for the group at `position` (counted from 0) of its
    /// window. That the key is unique within the window is the window's business.
    fn settle(&mut self, position: usize) {
        self.name = settled_name(&self.name, position);
        self.colour = settled_colour(&self.colour);
        if !self.key.as_deref().is_some_and(is_key_name) {
            self.key = None;
        }
    }
}

/// A group name: control characters removed, outer white space removed, at most
/// [`MAX_GROUP_NAME_BYTES`] bytes cut at a character boundary; `Group N` (`N` being
/// `position` + 1) when nothing is left. The same rule as the browser core's, in one pass that
/// stops at the limit, so a huge name costs no more than a short one.
fn settled_name(raw: &str, position: usize) -> String {
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
    if name.is_empty() {
        format!("Group {}", position + 1)
    } else {
        name
    }
}

/// A colour: a lower-case ASCII name of 1 to 16 letters, or `#` and six lower-case hex digits
/// (both lowered from upper case); empty for anything else.
fn settled_colour(raw: &str) -> String {
    const LONGEST_NAME: usize = 16;
    // The longest value that can be valid is a 16-letter name: look at nothing longer.
    if raw.len() > LONGEST_NAME {
        return String::new();
    }
    let lowered = raw.to_ascii_lowercase();
    let is_name = !lowered.is_empty() && lowered.bytes().all(|byte| byte.is_ascii_lowercase());
    let is_hex = lowered.len() == 7
        && lowered
            .strip_prefix('#')
            .is_some_and(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if is_name || is_hex {
        lowered
    } else {
        String::new()
    }
}

/// Whether `key` is `KeyA` to `KeyZ`.
fn is_key_name(key: &str) -> bool {
    matches!(key.as_bytes(), [b'K', b'e', b'y', letter] if letter.is_ascii_uppercase())
}

impl TabSnapshot {
    fn sanitized(self) -> Option<Self> {
        let url = session_url(&self.url)?;
        let title = truncated(&self.title, MAX_TITLE_BYTES).to_owned();
        Some(Self {
            url,
            title,
            zoom_percent: self.zoom_percent.clamp(MIN_ZOOM_PERCENT, MAX_ZOOM_PERCENT),
            ..self
        })
    }
}

impl WindowBounds {
    fn is_sensible(&self) -> bool {
        let sides = MIN_WINDOW_SIDE..=MAX_WINDOW_SIDE;
        sides.contains(&self.width)
            && sides.contains(&self.height)
            && self.x.unsigned_abs() <= MAX_WINDOW_COORDINATE
            && self.y.unsigned_abs() <= MAX_WINDOW_COORDINATE
    }
}

impl WindowSnapshot {
    fn sanitized(self) -> Option<Self> {
        let tabs: Vec<Option<TabSnapshot>> =
            self.tabs.into_iter().map(TabSnapshot::sanitized).collect();
        let keep: Vec<bool> = tabs.iter().map(Option::is_some).collect();
        // `active_tab` still refers to the strip before the drops; `retain_tabs` remaps it.
        let window = Self {
            tabs: tabs.into_iter().flatten().collect(),
            bounds: self.bounds.filter(WindowBounds::is_sensible),
            ..self
        };
        retain_tabs(window, &keep, true)
    }

    /// The group rules of [`SessionSnapshot::sanitized`], for a window whose tabs are final.
    fn settled_groups(mut self) -> Self {
        // Rule 1: the first 64, then the first of each id.
        self.groups.truncate(MAX_GROUPS_PER_WINDOW);
        let mut ids = HashSet::new();
        self.groups.retain(|group| ids.insert(group.id));
        // Rule 2: a tab keeps its group only if the group is there and the tab is not pinned.
        for tab in &mut self.tabs {
            if tab.pinned || tab.group.is_some_and(|id| !ids.contains(&id)) {
                tab.group = None;
            }
        }
        // Rule 3: a group with no tab is dropped, before it can take a key or a position.
        let used: HashSet<u64> = self.tabs.iter().filter_map(|tab| tab.group).collect();
        self.groups.retain(|group| used.contains(&group.id));
        // Rules 4 to 6.
        let mut keys: Vec<String> = Vec::new();
        for (position, group) in self.groups.iter_mut().enumerate() {
            group.settle(position);
            group.key = group.key.take().filter(|key| {
                let fresh = !keys.contains(key);
                if fresh {
                    keys.push(key.clone());
                }
                fresh
            });
        }
        // Rule 7.
        let above_every_id = self
            .groups
            .iter()
            .map(|group| group.id)
            .max()
            .map_or(0, |highest| highest.saturating_add(1));
        self.next_group_id = self
            .next_group_id
            .max(above_every_id)
            .min(MAX_GROUP_COUNTER);
        // Rule 8.
        self.dial_corner = self
            .dial_corner
            .filter(|corner| DIAL_CORNERS.contains(&corner.as_str()));
        self
    }
}

/// Keeps the tabs whose flag in `keep` is set and moves `active_tab` along with them. When
/// `already_filtered`, `window.tabs` holds only the kept tabs and `keep` describes the strip
/// `active_tab` refers to. `None` if no tab is left.
fn retain_tabs(
    window: WindowSnapshot,
    keep: &[bool],
    already_filtered: bool,
) -> Option<WindowSnapshot> {
    let last = keep.len().checked_sub(1)?;
    let active = window.active_tab.min(last);
    let kept_before = |index: usize| keep[..index].iter().filter(|kept| **kept).count();
    let new_active = if keep[active] {
        active
    } else {
        (active + 1..keep.len())
            .find(|&index| keep[index])
            .or_else(|| (0..active).rev().find(|&index| keep[index]))?
    };
    let tabs = if already_filtered {
        window.tabs
    } else {
        window
            .tabs
            .into_iter()
            .zip(keep)
            .filter_map(|(tab, kept)| kept.then_some(tab))
            .collect()
    };
    Some(WindowSnapshot {
        tabs,
        active_tab: kept_before(new_active),
        ..window
    })
}

/// Cuts the windows down to `max` tabs in all; see [`SessionSnapshot::sanitized`].
fn cap_tabs(windows: Vec<WindowSnapshot>, max: usize) -> Vec<WindowSnapshot> {
    let total: usize = windows.iter().map(|window| window.tabs.len()).sum();
    if total <= max {
        return windows;
    }
    // (class, order within the class, window, index): pinned, then active, then by distance
    // from the end of the strip, windows taking turns.
    let mut ranked: Vec<(u8, usize, usize, usize)> = Vec::with_capacity(total);
    for (window_index, window) in windows.iter().enumerate() {
        let length = window.tabs.len();
        for (index, tab) in window.tabs.iter().enumerate() {
            let (class, order) = if tab.pinned {
                (0, index)
            } else if index == window.active_tab {
                (1, 0)
            } else {
                (2, length - 1 - index)
            };
            ranked.push((class, order, window_index, index));
        }
    }
    ranked.sort_unstable();
    let mut keep: Vec<Vec<bool>> = windows
        .iter()
        .map(|window| vec![false; window.tabs.len()])
        .collect();
    for &(_, _, window_index, index) in ranked.iter().take(max) {
        keep[window_index][index] = true;
    }
    windows
        .into_iter()
        .zip(keep)
        .filter_map(|(window, keep)| retain_tabs(window, &keep, false))
        .collect()
}

/// Decides when a changed session is worth writing: at most once per interval, and at once
/// when asked to flush. Pure; the caller supplies the time.
///
/// The protocol:
///
/// 1. Call [`mark_dirty`](Self::mark_dirty) on every change.
/// 2. From a timer, call [`should_save`](Self::should_save). When it returns a [`SaveToken`],
///    take the snapshot, write it, and hand the token back to [`saved`](Self::saved) if the
///    write landed. If it failed, drop the token: the save is offered again once the interval
///    has passed. Changes marked after the token was issued stay dirty.
/// 3. To save without asking (on quit, or before something risky), write the snapshot and call
///    [`flushed`](Self::flushed): everything marked so far counts as saved.
///
/// **The quit rule:** the final save, with `exited_cleanly: true`, is made on every orderly quit
/// even when [`is_dirty`](Self::is_dirty) is false, because
/// [`SessionStore::begin_run`](crate::SessionStore::begin_run) marked the file as belonging
/// to a running browser; without that save the next start reports a crash.
#[derive(Clone, Debug)]
pub struct SaveThrottle {
    min_interval_ms: i64,
    /// Counts changes; `saved_changes` is the count known to be on disk.
    changes: u64,
    saved_changes: u64,
    last_offer_ms: Option<i64>,
    last_saved_ms: Option<i64>,
}

/// Permission to save, from [`SaveThrottle::should_save`], to be handed back to
/// [`SaveThrottle::saved`] once the write lands. It records which changes the save covers.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "hand the token back to `SaveThrottle::saved` once the session is written"]
pub struct SaveToken {
    changes: u64,
}

impl SaveThrottle {
    /// A throttle that offers a save at most once per `min_interval_ms` (a negative interval
    /// counts as zero).
    pub fn new(min_interval_ms: i64) -> Self {
        Self {
            min_interval_ms: min_interval_ms.max(0),
            changes: 0,
            saved_changes: 0,
            last_offer_ms: None,
            last_saved_ms: None,
        }
    }

    /// Records that the session changed.
    pub fn mark_dirty(&mut self) {
        self.changes = self.changes.saturating_add(1);
    }

    /// Whether there are changes not yet known to be saved.
    pub fn is_dirty(&self) -> bool {
        self.changes != self.saved_changes
    }

    /// A token to save with, at most once per interval while dirty: the interval runs from the
    /// last save and from the last token. A time earlier than either (the clock went back)
    /// counts as the interval having passed.
    #[must_use = "save the session when this returns a token"]
    pub fn should_save(&mut self, now_ms: i64) -> Option<SaveToken> {
        let interval = self.min_interval_ms;
        let passed = |since: Option<i64>| {
            since.is_none_or(|since| {
                let elapsed = now_ms.saturating_sub(since);
                elapsed < 0 || elapsed >= interval
            })
        };
        if !self.is_dirty() || !passed(self.last_saved_ms) || !passed(self.last_offer_ms) {
            return None;
        }
        self.last_offer_ms = Some(now_ms);
        Some(SaveToken {
            changes: self.changes,
        })
    }

    /// Records that the save `token` permitted landed at `now_ms`: the changes marked before
    /// the token was issued are saved.
    pub fn saved(&mut self, token: SaveToken, now_ms: i64) {
        self.saved_changes = self.saved_changes.max(token.changes);
        self.last_saved_ms = Some(now_ms);
        self.last_offer_ms = None;
    }

    /// Records that the caller saved without asking, at `now_ms`: everything marked so far is
    /// saved. Use it for the final save on quit.
    pub fn flushed(&mut self, now_ms: i64) {
        self.saved_changes = self.changes;
        self.last_saved_ms = Some(now_ms);
        self.last_offer_ms = None;
    }
}
