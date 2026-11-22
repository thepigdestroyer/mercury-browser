// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Orbit overview (SPEC `design/ui-final/SPEC.md` §4.6): the pure parts. The Slint view is
//! `ui/orbit/orbit.slint`; the desktop glue is `apps/mercury-desktop/src/orbit.rs`.
//!
//! Lane orbit-view (`.superpowers/sdd/lane-orbit-view-brief.md`). The shared types below are what
//! the machine, the layout, the data builder, the drag resolver and the desktop glue build against;
//! an addition is fine, a change goes through the lane lead. The submodules:
//!
//! | Module | Item | What |
//! | --- | --- | --- |
//! | [`machine`] | 1 | the zoom state machine ([`OrbitMachine`]), clock-free |
//! | [`search`] | 1 | `matches`, `top_result` |
//! | [`layout`] | 2 | the stage layout of the three levels and Orbit's own hit test |
//! | [`data`] | 2 | [`data::OrbitData`] and the persistent Slint models |
//! | [`drag`] | 3 | the payload, `resolve`, slot to final index, group operations |
//! | [`fixtures`] | lead | the mockup's 5 groups and 40 tabs (test-support) |
//!
//! # Rules
//!
//! - The machine has no clock and no animation: it changes level at once, Slint animates (D2).
//! - Rows come from the host as an [`OrbitInput`]; loose and pinned tabs are folded into one pseudo
//!   planet with [`LOOSE_GROUP_ID`] when the input is made (D7).
//! - Ids are the rows' ids: `GroupRow.id` and `TabRow.id` (Slint `int`).

use slint::SharedString;

use crate::{GroupRow, TabRow};

pub mod data;
pub mod drag;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
pub mod keys;
pub mod layout;
pub mod machine;
pub mod search;

pub use drag::{Decision, DropTarget, GroupOp, OrbitPayload, PayloadError, Source};
pub use machine::OrbitMachine;

/// The id of the pseudo group that holds the loose and the pinned tabs (D7).
pub const LOOSE_GROUP_ID: i32 = -2;
/// The name of that pseudo group.
pub const LOOSE_GROUP_NAME: &str = "Loose tabs";

/// The height of the title bar (mockup: 48 px).
pub const TITLE_BAR_HEIGHT: f32 = 48.0;
/// The height of the strip under the stage that holds the key hints (mockup: the stage ends at 892).
pub const FOOTER_HEIGHT: f32 = 8.0;

/// The three zoom levels (SPEC §4.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Level 1: every group as a planet round the sun.
    #[default]
    Groups,
    /// Level 2: one group, its ring in the centre and its tabs as previews.
    Group,
    /// Level 3: one tab almost full size.
    Tab,
}

/// What a pointer position or the keyboard focus is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A planet's halo (level 1) or its ring: the `GroupRow.id`.
    Planet(i32),
    /// The sun.
    Sun,
    /// A level-2 preview card or a segment of the centre ring: the `TabRow.id`.
    Preview(i32),
    /// An "Other groups" chip (level 2): the `GroupRow.id`.
    Chip(i32),
    /// The dashed "New group" ring.
    NewGroup,
    /// The level-3 card of the shown tab.
    Card,
    /// The previous or next tab's card at level 3: the `TabRow.id`.
    Neighbour(i32),
    /// Empty stage, the hole of a ring, anything that is not one of the above.
    Backdrop,
}

/// The key a host reports to the machine (⌘↑ is `Up` with `Mods::command`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Escape,
    Enter,
    Up,
    Down,
    Left,
    Right,
    /// `N`: a new group.
    N,
    /// `/`: focus the search field.
    Slash,
    /// Any other typed character (starts a search).
    Char(char),
}

/// The modifiers that count for Orbit's keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

impl Mods {
    pub const NONE: Self = Self {
        command: false,
        shift: false,
        alt: false,
        control: false,
    };
    pub const COMMAND: Self = Self {
        command: true,
        ..Self::NONE
    };
}

/// What the host tells the machine about the keyboard besides the key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyContext {
    /// A text field of Orbit (the search field, the group-name field) has the focus: `N`, `/` and
    /// typed characters are text then, not commands.
    pub text_input_focused: bool,
}

/// The state of the new-group popover (`orbit-new-group`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewGroupPopover {
    /// The tab that was dropped on the "New group" ring (it becomes the group's first tab).
    pub tab: i32,
    pub name: String,
    /// The preset colour names of the six dots (`blue`, `amber`, `violet`, `rose`, `green`, `teal`).
    pub colour: String,
}

/// The drag in flight, as the view needs to show it.
#[derive(Clone, Debug, PartialEq)]
pub struct DragView {
    pub source: Source,
    /// What the pointer is over now (`None` until the first `drag_over`).
    pub over: Option<DropTarget>,
}

/// A read-only picture of the machine for the layout, the data builder and the view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub level: Level,
    /// The group shown at levels 2 and 3 (`GroupRow.id`), and the group the focus is in at level 1.
    pub group: Option<i32>,
    /// The tab shown at level 3, the focused tab at level 2 (`TabRow.id`).
    pub tab: Option<i32>,
    /// What the pointer is over.
    pub hover: Option<Target>,
    /// What the keyboard focus is on (level 1: a planet; level 2: a preview; level 3: the card).
    pub focus: Option<Target>,
    pub drag: Option<DragView>,
    pub query: String,
    pub popover: Option<NewGroupPopover>,
    /// While a pinch runs: `p` in -1..=1 (the view scales the stage with it); `None` otherwise.
    pub pinch: Option<f32>,
}

/// The window's size, in logical px (the overlay covers the whole window, title bar included).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub width: f32,
    pub height: f32,
}

impl Stage {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// The mockup's window.
    pub const MOCKUP: Self = Self::new(1440.0, 900.0);

    /// The centre of the area below the title bar and above the footer (mockup: (720, 470)).
    pub fn centre(&self) -> (f32, f32) {
        (
            self.width / 2.0,
            (TITLE_BAR_HEIGHT + self.height - FOOTER_HEIGHT) / 2.0,
        )
    }
}

/// What the host gives Orbit when it opens and when rows change. Make it with [`OrbitInput::new`],
/// which folds the loose and pinned tabs into the pseudo planet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrbitInput {
    /// The groups in ring order, the pseudo planet last when there are loose or pinned tabs.
    pub groups: Vec<GroupRow>,
    /// All tabs, group by group in `index-in-group` order; a loose or pinned tab has `group`
    /// [`LOOSE_GROUP_ID`] and `pinned` false here (ring-bands draws no pinned tab).
    pub tabs: Vec<TabRow>,
    /// The ids of the tabs that were pinned (read-only in Orbit: not draggable, not a drop source).
    pub pinned: Vec<i32>,
    pub profile_name: SharedString,
    pub window_label: SharedString,
    /// `TabRow.id` of the active tab (it is where Orbit opens and what level 2 puts first).
    pub active_tab: i32,
}

impl OrbitInput {
    /// Folds the tabs that are in no known group, and the pinned ones, into the pseudo planet
    /// ([`LOOSE_GROUP_ID`], D7). `dark` picks the slate label colours of the pseudo group.
    pub fn new(
        groups: &[GroupRow],
        tabs: &[TabRow],
        profile_name: &str,
        window_label: &str,
        active_tab: i32,
        dark: bool,
    ) -> Self {
        let known = |group: i32| groups.iter().any(|g| g.id == group);
        let mut pinned = Vec::new();
        let mut kept: Vec<TabRow> = Vec::with_capacity(tabs.len());
        let mut loose: Vec<TabRow> = Vec::new();
        for tab in tabs {
            if tab.pinned || !known(tab.group) {
                if tab.pinned {
                    pinned.push(tab.id);
                }
                let mut row = tab.clone();
                row.group = LOOSE_GROUP_ID;
                row.pinned = false;
                row.index_in_group = loose.len() as i32;
                loose.push(row);
            } else {
                kept.push(tab.clone());
            }
        }
        let mut out_groups: Vec<GroupRow> = groups
            .iter()
            .map(|group| {
                let mut group = group.clone();
                group.count = kept.iter().filter(|t| t.group == group.id).count() as i32;
                group
            })
            .collect();
        if !loose.is_empty() {
            out_groups.push(loose_group(loose.len(), dark));
        }
        kept.extend(loose);
        Self {
            groups: out_groups,
            tabs: kept,
            pinned,
            profile_name: profile_name.into(),
            window_label: window_label.into(),
            active_tab,
        }
    }

    /// The group row with this id.
    pub fn group(&self, id: i32) -> Option<&GroupRow> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// The tab row with this id.
    pub fn tab(&self, id: i32) -> Option<&TabRow> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    /// The tabs of a group in `index-in-group` order.
    pub fn tabs_of(&self, group: i32) -> Vec<&TabRow> {
        let mut own: Vec<&TabRow> = self.tabs.iter().filter(|t| t.group == group).collect();
        own.sort_by_key(|t| t.index_in_group);
        own
    }

    /// Whether this tab was pinned (read-only).
    pub fn is_pinned(&self, tab: i32) -> bool {
        self.pinned.contains(&tab)
    }
}

/// The pseudo group's row: SPEC slate (`#7C8088`) with the same light and dark label colours as
/// the other group colours (`ring_bands::fixtures::GROUP_PALETTES`).
fn loose_group(count: usize, dark: bool) -> GroupRow {
    let rgb = |hex: u32| slint::Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
    GroupRow {
        id: LOOSE_GROUP_ID,
        name: LOOSE_GROUP_NAME.into(),
        colour: rgb(0x7C8088),
        tint: rgb(if dark { 0x39354D } else { 0xE7E8EA }),
        deep: rgb(if dark { 0xB3B5BA } else { 0x54575C }),
        key_label: SharedString::new(),
        count: count as i32,
        last_used: SharedString::new(),
        active: false,
        any_loading: false,
        any_audible: false,
        any_unread: false,
    }
}

/// What the machine asks the host to do. The host animates, closes, runs group commands.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Leave Orbit; with `Some(tab)` activate that tab first.
    Close { activate: Option<i32> },
    /// A pinch ended without committing: animate `pinch-progress` back to 0 in 160 ms.
    Springback,
    /// The level, group or tab changed: the host rebuilds the view and animates to it.
    ZoomedTo(View),
    /// A drag ended over a target: what to do with it (the host maps it with `Decision::op`).
    Resolve(Decision),
    /// A group operation that does not come from a drag (the new-group popover's confirm).
    Apply(GroupOp),
    /// Show the new-group popover for this tab.
    OpenNewGroupPopover { tab: i32 },
    /// Move the keyboard focus to the search field.
    FocusSearch,
    /// Clear the search field (Esc, layer 3).
    ClearSearch,
    /// Take a fresh picture of the active tab (the host calls `capture_active_now`).
    CapturePicture,
}
