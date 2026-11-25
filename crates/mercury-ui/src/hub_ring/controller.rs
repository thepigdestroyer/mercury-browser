// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `HubRing`: owns the ring's state (rows, query, focus, centre, held), builds the
//! [`RingBandsData`](crate::ring_bands::RingBandsData), fills the `HubRingBridge` global of any
//! window that has it and calls the browser through [`HubRingHost`].
//!
//! # How it works
//!
//! - **One state, one push.** Every event (a key, a pointer callback, new rows) changes the private
//!   `Core` under a short borrow, then the host is called (never while the state is borrowed: a host
//!   may call [`HubRing::set_input`] again at once), then `Shared::refresh` computes a plain `View`
//!   and writes it to the bridge. The ring's data is rebuilt only when the rows, the query or the
//!   ⌥ look changed, not for a hover or a focus move.
//! - **The keys.** The popover's `FocusScope` sends every key here first. [`map_key`] turns Slint's
//!   key text and modifiers into a [`NavKey`]; [`nav::step`] decides; the [`Intent`] is carried out.
//!   While a query is active ↑ ↓ and ⏎ drive the result rows instead of the ring.
//! - **Typing.** The search field has the keyboard focus when the popover opens (so an input method
//!   can start with the first key). While it has (`field-focused` on the bridge), printable keys and
//!   Backspace go to the field, whose `edited` calls `query-edited`; the controller types into the
//!   query itself only when the field has no focus (the fallback). Chords, arrows, Esc, ⏎ and M
//!   always reach the controller first.
//! - **⌘ and ⌥ on macOS.** Slint reports ⌘ as `modifiers.control` and ⌃ as `modifiers.meta` there
//!   ([`is_command`]), and ⌥ with a letter or digit as the composed character ("ß", "¡"). So the
//!   popover maps only the Alt key itself and ⌥ with arrows; the window-level hook, which sees the
//!   physical key, calls [`HubRing::option_key`] for ⌥ letters and digits.
//! - **The dragged tab.** The bridge's `drop(group, index)` carries no tab id, so the controller
//!   remembers the `dragging` argument of the last `drop-target` call (RingBands asks for it while
//!   it drags) and uses it for the drop.
//! - **The ⌥ hold** is the only timer: `hold_ms` after ⌥ goes down the key look starts.
//! - **Closing.** After the controller asks the popover to close (`close-request`), keys and
//!   activations do nothing until the popover reports `closed` (or `opened` again), so nothing acts
//!   during the 120 ms close animation.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::Key;
use slint::{
    Color, ComponentHandle, Global, Image, Model, ModelRc, SharedString, Timer, TimerMode, VecModel,
};

use super::beads::{self, PinnedBead};
use super::centre::{self, CentreFocus, CentreInput, CentreView, HeldGroup};
use super::hub_button;
use super::nav::{self, Cursor, FocusOrder, Intent, NavKey, NavState};
use super::search::{self, SearchDoc, SearchResult};
use super::{HubRingHost, HubRingInput, TabMedia};
use crate::ring_bands::{RingBandsData, RingBandsModels, RingColours, RingOptions};
use crate::ring_geometry::{ApproxMetrics, GlyphMetrics, Point, RingSpec};
use crate::{
    BeadRow, ChromePalette, DonutArc, FlagKind, GroupRow, HubRingBridge, RingDropTarget, RingHit,
    RingHitKind, SearchRow, TabRow,
};

/// The ring's box edge and its centre (the hub ring of the mockup).
const RING_CENTRE: f32 = 258.0;
/// The radius of the circle the beads sit on.
const BEAD_RADIUS: f32 = 180.0;
/// The tooltip's height and the room it keeps from the ring's right edge.
const TOOLTIP_HEIGHT: f32 = 22.0;
const TOOLTIP_PAD: f32 = 16.0;
const RING_EDGE: f32 = 516.0;

/// The controller of one hub ring popover; keep it alive as long as the window.
pub struct HubRing {
    shared: Rc<Shared>,
}

/// Calls `f` with the window's bridge, if the window still exists.
type BridgeAccess = Rc<dyn Fn(&mut dyn FnMut(&HubRingBridge<'_>))>;

/// The ring's label colours from the window's live `ChromePalette`, if the window still exists.
type PaletteAccess = Rc<dyn Fn() -> Option<RingColours>>;

impl HubRing {
    /// Sets the persistent models of the window's `HubRingBridge` once and every handler on it.
    /// `metrics` measures the ring's labels (`ring_bands::metrics()`; keep one for the window).
    pub fn install<C>(
        component: &C,
        host: Rc<dyn HubRingHost>,
        metrics: Box<dyn GlyphMetrics>,
    ) -> Self
    where
        C: ComponentHandle + 'static,
        for<'a> HubRingBridge<'a>: Global<'a, C>,
        for<'a> ChromePalette<'a>: Global<'a, C>,
    {
        let weak = component.as_weak();
        let access: BridgeAccess = Rc::new({
            let weak = weak.clone();
            move |f: &mut dyn FnMut(&HubRingBridge<'_>)| {
                if let Some(component) = weak.upgrade() {
                    f(&component.global::<HubRingBridge>());
                }
            }
        });
        let palette: PaletteAccess = Rc::new(move || {
            weak.upgrade()
                .map(|component| RingColours::from_palette(&component.global::<ChromePalette>()))
        });
        let shared = Rc::new(Shared {
            core: RefCell::new(Core::default()),
            host,
            metrics,
            access,
            palette,
            models: Models::default(),
            timer: Timer::default(),
            dragging: Cell::new(-1),
        });
        let live = (shared.palette)();
        shared.with_core(|core| core.live_colours = live);

        let bridge = component.global::<HubRingBridge>();
        let ring = &shared.models.ring;
        bridge.set_groups(ring.groups());
        bridge.set_tabs(ring.tabs());
        bridge.set_segments(ring.segments());
        bridge.set_lifted_segments(ring.lifted_segments());
        bridge.set_segment_labels(ring.segment_labels());
        bridge.set_label_bands(ring.label_bands());
        bridge.set_label_glyphs(ring.label_glyphs());
        bridge.set_flags(ring.flags());
        bridge.set_beads(ModelRc::from(Rc::clone(&shared.models.beads)));
        bridge.set_results(ModelRc::from(Rc::clone(&shared.models.results)));
        bridge.set_donut(ModelRc::from(Rc::clone(&shared.models.donut)));

        install_handlers(&bridge, &shared);
        Self { shared }
    }

    /// Rebuilds everything from new rows, theme or size: runs the search again and keeps the query
    /// and the focus while the tab still exists. The label colours are `input.colours`, or the
    /// window's live `ChromePalette` read now when that is `None`.
    pub fn set_input(&self, input: HubRingInput) {
        let live = (self.shared.palette)();
        self.shared.with_core(|core| {
            core.live_colours = live;
            core.set_input(input);
        });
        self.shared.refresh();
    }

    /// The popover opened: the focus is the active tab, the query is empty, the mode is rest.
    pub fn opened(&self) {
        self.shared.opened();
    }

    /// The key look from outside (the window-level ⌥ hook, D5): `true` shows it at once (no
    /// `hold_ms` wait), `false` ends it and, when a group was chosen, commits it as releasing ⌥ does.
    pub fn set_held(&self, held: bool) {
        self.shared.set_held(held);
    }

    /// An override of the labels' key-letter and count colours; it wins over `HubRingInput::colours`
    /// and the live palette. Normally not needed: leave `HubRingInput::colours` `None`.
    pub fn set_colours(&self, colours: RingColours) {
        self.shared.with_core(|core| {
            core.override_colours = Some(colours);
            core.input_version += 1;
        });
        self.shared.refresh();
    }

    /// ⌥ with a letter or a digit, from the window-level hook that sees the PHYSICAL key (Slint
    /// reports the composed character, "ß" for ⌥S, so the popover cannot use its own key events for
    /// this). `key` is an upper-case ASCII letter `'A'..='Z'` (selects the group with that key) or a
    /// digit `'1'..='9'` (shows that tab of the chosen group); anything else is ignored. It sets
    /// "⌥ is down" first when it is not, as if the Alt key had come first, so the ⌥ release that
    /// follows commits the group.
    pub fn option_key(&self, key: char) {
        self.shared.option_key(key);
    }
}

/// The persistent models that this controller owns besides the ring's.
#[derive(Default)]
struct Models {
    ring: RingBandsModels,
    beads: Rc<VecModel<BeadRow>>,
    results: Rc<VecModel<SearchRow>>,
    donut: Rc<VecModel<DonutArc>>,
}

struct Shared {
    core: RefCell<Core>,
    host: Rc<dyn HubRingHost>,
    metrics: Box<dyn GlyphMetrics>,
    access: BridgeAccess,
    palette: PaletteAccess,
    models: Models,
    timer: Timer,
    /// The tab being dragged (the last `drop-target` argument), -1 for none.
    dragging: Cell<i32>,
}

/// Everything the controller knows. Plain data: no Slint handles.
struct Core {
    input: HubRingInput,
    data: RingBandsData,
    /// What `data` was built for: (held, selected group, search version, input version).
    built: Option<(bool, Option<i32>, u64, u64)>,
    search_version: u64,
    input_version: u64,
    /// Every row with `matches_query` set.
    rows: Vec<TabRow>,
    docs: Vec<SearchDoc>,
    /// Shown tabs in ring order (ids) and pinned tabs in input order.
    ring_ids: Vec<i32>,
    pinned_ids: Vec<i32>,
    order: FocusOrder,
    nav: NavState,
    query: String,
    selected_row: usize,
    search: SearchResult,
    hovered: i32,
    /// The window's live palette colours, read at install and on every `set_input`.
    live_colours: Option<RingColours>,
    /// `HubRing::set_colours`.
    override_colours: Option<RingColours>,
    close_requests: i32,
    /// The popover was asked to close and has not reported `closed` yet: nothing acts.
    closing: bool,
    hub_open: bool,
    /// The focus ring is drawn: a key moved the focus since the popover opened (a pointer-only
    /// session shows none, as the mockups).
    focus_visible: bool,
}

impl Default for Core {
    fn default() -> Self {
        Self {
            input: HubRingInput::default(),
            data: empty_data(),
            built: None,
            search_version: 0,
            input_version: 0,
            rows: Vec::new(),
            docs: Vec::new(),
            ring_ids: Vec::new(),
            pinned_ids: Vec::new(),
            order: FocusOrder::default(),
            nav: NavState::default(),
            query: String::new(),
            selected_row: 0,
            search: SearchResult::default(),
            hovered: -1,
            live_colours: None,
            override_colours: None,
            close_requests: 0,
            closing: false,
            hub_open: false,
            focus_visible: false,
        }
    }
}

fn empty_data() -> RingBandsData {
    RingBandsData::new(
        RingSpec::hub(Point::new(RING_CENTRE, RING_CENTRE)),
        &[],
        &[],
        &RingOptions::default(),
        &ApproxMetrics,
    )
}

fn key_char(group: &GroupRow) -> Option<char> {
    group
        .key_label
        .chars()
        .next()
        .map(|letter| letter.to_ascii_uppercase())
}

impl Core {
    fn row(&self, id: i32) -> Option<&TabRow> {
        self.input.tabs.iter().find(|row| row.id == id)
    }

    fn group(&self, id: i32) -> Option<&GroupRow> {
        self.input.groups.iter().find(|group| group.id == id)
    }

    fn media(&self, tab: i32) -> Option<TabMedia> {
        self.input.media.iter().find(|m| m.tab == tab).copied()
    }

    fn has_sound(&self, tab: i32) -> bool {
        self.row(tab).is_some_and(|row| row.audible || row.muted) || self.media(tab).is_some()
    }

    /// The ids of the shown tabs of `group`, in ring order.
    fn tabs_of(&self, group: i32) -> Vec<i32> {
        self.ring_ids
            .iter()
            .copied()
            .filter(|&id| self.row(id).is_some_and(|row| row.group == group))
            .collect()
    }

    /// The tab a group's key switches to: its last active tab, else its first.
    fn commit_target(&self, group: i32) -> Option<i32> {
        let tabs = self.tabs_of(group);
        self.input
            .last_active
            .iter()
            .find(|&&(g, tab)| g == group && tabs.contains(&tab))
            .map(|&(_, tab)| tab)
            .or_else(|| tabs.first().copied())
    }

    /// The tab the centre is about: the hovered one, else the focused one, else the active one.
    fn centre_tab(&self) -> Option<i32> {
        if self.hovered >= 0 && self.row(self.hovered).is_some() {
            return Some(self.hovered);
        }
        match self.nav.cursor {
            Cursor::Tab(id) | Cursor::Bead(id) => return Some(id),
            Cursor::None => {}
        }
        self.input
            .tabs
            .iter()
            .find(|row| row.active)
            .or_else(|| self.input.tabs.first())
            .map(|row| row.id)
    }

    /// The ring order, the pinned list, the search documents and the focus order from the rows.
    fn reindex(&mut self) {
        let mut seen = HashSet::new();
        let mut ring: Vec<&TabRow> = Vec::new();
        let mut with_tabs: Vec<(i32, Option<char>)> = Vec::new();
        for group in &self.input.groups {
            if !seen.insert(group.id) {
                continue;
            }
            let mut tabs: Vec<&TabRow> = self
                .input
                .tabs
                .iter()
                .filter(|row| !row.pinned && row.group == group.id)
                .collect();
            tabs.sort_by_key(|row| row.index_in_group);
            if !tabs.is_empty() {
                with_tabs.push((group.id, key_char(group)));
            }
            ring.extend(tabs);
        }
        let pinned: Vec<&TabRow> = self.input.tabs.iter().filter(|row| row.pinned).collect();

        let mut docs = Vec::with_capacity(ring.len() + pinned.len());
        let mut position_in_group: Vec<(i32, usize)> = Vec::new();
        for row in &ring {
            let slot = match position_in_group.iter_mut().find(|(g, _)| *g == row.group) {
                Some((_, count)) => {
                    *count += 1;
                    *count
                }
                None => {
                    position_in_group.push((row.group, 1));
                    1
                }
            };
            let group = self.group(row.group);
            docs.push(SearchDoc {
                tab: row.id,
                title: row.title.to_string(),
                host: row.host.to_string(),
                group_id: row.group,
                group_name: group.map(|g| g.name.to_string()).unwrap_or_default(),
                key_label: group.map(|g| g.key_label.to_string()).unwrap_or_default(),
                index_in_group: slot,
                pinned: false,
            });
        }
        for row in &pinned {
            docs.push(SearchDoc {
                tab: row.id,
                title: row.title.to_string(),
                host: row.host.to_string(),
                group_id: row.group,
                group_name: String::new(),
                key_label: String::new(),
                index_in_group: 0,
                pinned: true,
            });
        }

        let pairs: Vec<(i32, i32)> = ring.iter().map(|row| (row.id, row.group)).collect();
        self.ring_ids = ring.iter().map(|row| row.id).collect();
        self.pinned_ids = pinned.iter().map(|row| row.id).collect();
        self.order = FocusOrder::new(&pairs, &with_tabs, &self.pinned_ids);
        self.docs = docs;
    }

    /// Runs the search for `query` and marks the rows.
    fn research(&mut self) {
        self.search = search::run(&self.docs, &self.query);
        self.rows = self.input.tabs.clone();
        search::set_matches(&mut self.rows, &self.search);
        self.nav.query_active = self.search.active;
        let last = self.search.rows.len();
        self.selected_row = self.selected_row.min(last);
        self.search_version += 1;
    }

    fn set_query(&mut self, query: String) {
        self.query = query;
        self.selected_row = 0;
        self.research();
    }

    fn push_char(&mut self, c: char) {
        if self.query.is_empty() && c.is_whitespace() {
            return;
        }
        self.query.push(c);
        self.hovered = -1;
        self.selected_row = 0;
        self.research();
    }

    fn pop_char(&mut self) -> bool {
        if self.query.pop().is_none() {
            return false;
        }
        self.hovered = -1;
        self.selected_row = 0;
        self.research();
        true
    }

    /// Where the focus is in the list "segments, then beads".
    fn focus_list(order: &FocusOrder) -> Vec<Cursor> {
        order
            .tabs
            .iter()
            .map(|slot| Cursor::Tab(slot.id))
            .chain(order.beads.iter().map(|&id| Cursor::Bead(id)))
            .collect()
    }

    fn set_input(&mut self, input: HubRingInput) {
        let old_list = Self::focus_list(&self.order);
        let old_cursor = self.nav.cursor;
        self.input = input;
        self.input_version += 1;
        self.reindex();

        let list = Self::focus_list(&self.order);
        self.nav.cursor = if old_cursor == Cursor::None {
            self.active_cursor()
        } else if list.contains(&old_cursor) {
            old_cursor
        } else {
            let at = old_list.iter().position(|c| *c == old_cursor).unwrap_or(0);
            list.get(at.min(list.len().saturating_sub(1)))
                .copied()
                .unwrap_or(Cursor::None)
        };
        if self.hovered >= 0 && self.row(self.hovered).is_none() {
            self.hovered = -1;
        }
        if let Some(group) = self.nav.held_group
            && !self.order.groups.iter().any(|slot| slot.id == group)
        {
            self.nav.held_group = None;
        }
        self.research();
    }

    /// The focus on the active tab (else the first one).
    fn active_cursor(&self) -> Cursor {
        let list = Self::focus_list(&self.order);
        let active = self.input.tabs.iter().find(|row| row.active);
        active
            .and_then(|row| {
                list.iter().copied().find(|cursor| match cursor {
                    Cursor::Tab(id) | Cursor::Bead(id) => *id == row.id,
                    Cursor::None => false,
                })
            })
            .or_else(|| list.first().copied())
            .unwrap_or(Cursor::None)
    }

    /// The override, else the input's colours, else the window's live palette (the default only when
    /// the window is gone).
    fn ring_colours(&self) -> RingColours {
        self.override_colours
            .or(self.input.colours)
            .or(self.live_colours)
            .unwrap_or_default()
    }

    /// Rebuilds the ring's data when something it depends on changed since the last build.
    fn build_data_if_needed(&mut self, metrics: &dyn GlyphMetrics) -> bool {
        let want = (
            self.nav.held,
            self.nav.held_group,
            self.search_version,
            self.input_version,
        );
        if self.built == Some(want) {
            return false;
        }
        let opts = RingOptions {
            held: self.nav.held,
            selected_group: self.nav.held_group,
            flag_size: 20.0,
            colours: self.ring_colours(),
        };
        self.data = RingBandsData::new(
            RingSpec::hub(Point::new(RING_CENTRE, RING_CENTRE)),
            &self.input.groups,
            &self.rows,
            &opts,
            metrics,
        );
        self.built = Some(want);
        true
    }

    fn centre_focus(&self) -> Option<CentreFocus> {
        let tab = self.centre_tab()?;
        let row = self.row(tab)?;
        let doc = self.docs.iter().find(|doc| doc.tab == tab);
        let group = self.group(row.group);
        Some(CentreFocus {
            tab,
            title: row.title.to_string(),
            host: row.host.to_string(),
            group_name: group.map(|g| g.name.to_string()).unwrap_or_default(),
            key_label: group.map(|g| g.key_label.to_string()).unwrap_or_default(),
            index_in_group: doc.map_or(0, |doc| doc.index_in_group),
            pinned: row.pinned,
            media: self.media(tab),
        })
    }

    fn held_group(&self) -> Option<HeldGroup> {
        let id = self.nav.held_group?;
        let group = self.group(id)?;
        Some(HeldGroup {
            name: group.name.to_string(),
            key_label: group.key_label.to_string(),
            tab_count: self.tabs_of(id).len(),
        })
    }

    /// The label next to the hovered tab's loading or sound flag (D11), with its top-left corner.
    fn tooltip(&self) -> Option<(String, f32, f32)> {
        if self.hovered < 0 {
            return None;
        }
        let title = self.row(self.hovered)?.title.to_string();
        let pin = |kind: FlagKind| {
            self.data
                .flags
                .iter()
                .find(|pin| pin.tab == self.hovered && pin.kind == kind)
        };
        let (text, pin) = if let Some(pin) = pin(FlagKind::Spinner) {
            let text = if pin.progress >= 0.0 {
                format!(
                    "{title} · loading {}%",
                    (pin.progress * 100.0).round() as i32
                )
            } else {
                format!("{title} · loading")
            };
            (text, pin)
        } else if let Some(pin) = pin(FlagKind::Audible) {
            (format!("{title} · playing sound"), pin)
        } else {
            let pin = pin(FlagKind::Muted)?;
            (format!("{title} · muted"), pin)
        };
        let width = text.chars().count() as f32 * 5.6 + TOOLTIP_PAD;
        let x = (pin.x + pin.size / 2.0 + 2.0)
            .min(RING_EDGE - width - 4.0)
            .max(4.0);
        Some((text, x, pin.y - TOOLTIP_HEIGHT / 2.0))
    }
}

/// How [`patch`] brought a model up to date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Patched {
    /// The model already held these rows: not touched.
    Unchanged,
    /// This many rows were replaced in place (`set_row_data`).
    Rows(usize),
    /// The length changed: the whole model was replaced (`set_vec`).
    Reset,
}

/// Brings `model` to `rows` touching as little as possible. A model reset recreates the repeater's
/// elements: a bead's 120 ms fade would never play and a screen reader would lose its place. Equal
/// rows are left alone and a same-length change replaces only the rows that differ.
fn patch<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) -> Patched {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return Patched::Reset;
    }
    let mut replaced = 0;
    for (index, row) in rows.into_iter().enumerate() {
        if model.row_data(index).as_ref() != Some(&row) {
            model.set_row_data(index, row);
            replaced += 1;
        }
    }
    if replaced == 0 {
        Patched::Unchanged
    } else {
        Patched::Rows(replaced)
    }
}

/// What the bridge shows, computed under a borrow and written after it.
struct View {
    data: Option<RingBandsData>,
    beads: Vec<BeadRow>,
    results: Vec<SearchRow>,
    donut: Vec<DonutArc>,
    hub_label: SharedString,
    centre: CentreView,
    preview: Option<Image>,
    preview_mark: Color,
    tooltip: (String, f32, f32),
    focused_tab: i32,
    focused_bead: i32,
    held: bool,
    selected_group: i32,
    query_active: bool,
    query: String,
    orbit_available: bool,
    reduce_motion: bool,
    close_requests: i32,
    hub_open: bool,
}

impl Core {
    fn view(&mut self, metrics: &dyn GlyphMetrics) -> View {
        let rebuilt = self.build_data_if_needed(metrics);

        let layout = beads::layout(
            self.pinned_ids.len(),
            self.data.model.layout.spec.reserve_deg,
            BEAD_RADIUS,
            (RING_CENTRE, RING_CENTRE),
        );
        let pinned: Vec<PinnedBead> = self
            .pinned_ids
            .iter()
            .filter_map(|&id| self.row(id))
            .map(PinnedBead::from_row)
            .collect();
        let focused_bead = match self.nav.cursor {
            Cursor::Bead(id) if self.focus_visible => Some(id),
            _ => None,
        };
        let active = self.search.active;
        let matches: HashSet<i32> = self.search.matches.iter().copied().collect();
        let bead_rows = beads::rows(&layout, &pinned, focused_bead, &|tab| {
            active && !matches.contains(&tab)
        });

        let mut results: Vec<SearchRow> = Vec::new();
        if active {
            for (index, row) in self.search.rows.iter().enumerate() {
                results.push(SearchRow {
                    tab: row.tab,
                    title: row.title.as_str().into(),
                    path: row.path.as_str().into(),
                    selected: index == self.selected_row,
                });
            }
            results.push(SearchRow {
                tab: -1,
                title: search::web_row_text(&self.query).into(),
                path: SharedString::default(),
                selected: self.selected_row >= self.search.rows.len(),
            });
        }

        let centre = centre::view(&CentreInput {
            query_active: active,
            held: self.nav.held,
            held_group: self.held_group(),
            total_tabs: self.input.tabs.len(),
            focus: self.centre_focus(),
            more_matches: self.search.more,
            pinned_collapsed: if layout.capacity == 0 {
                self.pinned_ids.len()
            } else {
                0
            },
        });

        let shown = self.centre_tab().and_then(|tab| self.row(tab));
        let preview = shown.and_then(|row| {
            self.input
                .previews
                .iter()
                .find(|(id, _)| *id == row.id)
                .map(|(_, image)| image.clone())
        });
        let preview_mark = shown.map(|row| row.shade).unwrap_or_default();

        let (focused_tab, focused_bead) = match self.nav.cursor {
            Cursor::Tab(id) if self.focus_visible => (id, -1),
            Cursor::Bead(id) if self.focus_visible => (-1, id),
            _ => (-1, -1),
        };
        let fallback = self.ring_colours().count;
        View {
            data: rebuilt.then(|| self.data.clone()),
            beads: bead_rows,
            results,
            donut: hub_button::donut(&self.input.groups, fallback),
            hub_label: hub_button::label_with_pinned(&self.input.groups, self.pinned_ids.len())
                .into(),
            centre,
            preview,
            preview_mark,
            tooltip: self.tooltip().unwrap_or_default(),
            focused_tab,
            focused_bead,
            held: self.nav.held,
            selected_group: self.nav.held_group.unwrap_or(-1),
            query_active: active,
            query: self.query.clone(),
            orbit_available: self.input.orbit_available,
            reduce_motion: self.input.reduce_motion,
            close_requests: self.close_requests,
            hub_open: self.hub_open,
        }
    }
}

impl Shared {
    fn with_core<R>(&self, f: impl FnOnce(&mut Core) -> R) -> Option<R> {
        self.core.try_borrow_mut().ok().map(|mut core| f(&mut core))
    }

    fn with_bridge(&self, mut f: impl FnMut(&HubRingBridge<'_>)) {
        (self.access)(&mut f);
    }

    /// Writes the current state to the bridge.
    fn refresh(&self) {
        let Some(view) = self.with_core(|core| core.view(&*self.metrics)) else {
            return;
        };
        if let Some(data) = &view.data {
            self.models.ring.update(data);
        }
        patch(&self.models.beads, view.beads.clone());
        patch(&self.models.results, view.results.clone());
        patch(&self.models.donut, view.donut.clone());
        let centre = &view.centre;
        self.with_bridge(|bridge| {
            bridge.set_mode(centre.mode);
            bridge.set_title(centre.title.as_str().into());
            bridge.set_subtitle(centre.subtitle.as_str().into());
            bridge.set_play_state(centre.play_state.as_str().into());
            bridge.set_can_pause(centre.can_pause);
            bridge.set_paused(centre.paused);
            bridge.set_muted(centre.muted);
            bridge.set_field_placeholder(centre.field_placeholder.as_str().into());
            bridge.set_held_key(centre.held_key.as_str().into());
            bridge.set_held_name(centre.held_name.as_str().into());
            bridge.set_held_help(centre.held_help.as_str().into());
            bridge.set_footer(centre.footer.as_str().into());
            bridge.set_has_preview(view.preview.is_some());
            bridge.set_preview(view.preview.clone().unwrap_or_default());
            bridge.set_preview_mark(view.preview_mark);
            bridge.set_tooltip(view.tooltip.0.as_str().into());
            bridge.set_tooltip_x(view.tooltip.1);
            bridge.set_tooltip_y(view.tooltip.2);
            bridge.set_focused_tab(view.focused_tab);
            bridge.set_focused_bead(view.focused_bead);
            bridge.set_held(view.held);
            bridge.set_selected_group(view.selected_group);
            bridge.set_query_active(view.query_active);
            if bridge.get_query().as_str() != view.query {
                bridge.set_query(view.query.as_str().into());
            }
            bridge.set_orbit_available(view.orbit_available);
            bridge.set_reduce_motion(view.reduce_motion);
            bridge.set_hub_label(view.hub_label.clone());
            bridge.set_hub_open(view.hub_open);
            if bridge.get_close_request() != view.close_requests {
                bridge.set_close_request(view.close_requests);
            }
        });
    }

    fn hit(&self, x: f32, y: f32, hovered: i32) -> RingHit {
        let outside = RingHit {
            kind: RingHitKind::Outside,
            id: -1,
        };
        self.core
            .try_borrow()
            .map_or(outside.clone(), |core| core.data.hit(x, y, hovered))
    }

    fn drop_target(&self, x: f32, y: f32, dragging: i32) -> RingDropTarget {
        self.dragging.set(dragging);
        match self.core.try_borrow() {
            Ok(core) => core.data.drop_target(x, y, dragging),
            Err(_) => RingDropTarget {
                valid: false,
                group: -1,
                index: -1,
                indicator: Default::default(),
            },
        }
    }

    fn request_close(&self) {
        self.with_core(|core| {
            core.close_requests += 1;
            core.closing = true;
        });
    }

    /// The popover was asked to close and has not reported `closed` yet.
    fn is_closing(&self) -> bool {
        self.with_core(|core| core.closing).unwrap_or(false)
    }

    fn activate(&self, tab: i32) {
        if self.is_closing() {
            return;
        }
        self.host.activate_tab(tab);
        self.request_close();
    }

    /// Esc while ⌥ is down or the key look is on: back to idle, nothing is shown.
    fn cancel_look(&self) {
        self.timer.stop();
        self.with_core(|core| {
            core.nav.option_down = false;
            core.nav.held = false;
            core.nav.held_group = None;
        });
    }

    fn opened(&self) {
        self.timer.stop();
        self.with_core(|core| {
            core.nav = NavState::default();
            core.hovered = -1;
            core.closing = false;
            core.hub_open = true;
            core.focus_visible = false;
            core.nav.cursor = core.active_cursor();
            core.set_query(String::new());
        });
        self.refresh();
    }

    fn closed(&self) {
        self.timer.stop();
        self.dragging.set(-1);
        self.with_core(|core| {
            core.hub_open = false;
            core.closing = false;
            core.focus_visible = false;
            core.hovered = -1;
            core.nav.option_down = false;
            core.nav.held = false;
            core.nav.held_group = None;
        });
        self.refresh();
    }

    /// One key of the machine, then its intent.
    fn feed(self: &Rc<Self>, key: NavKey) -> bool {
        let moves_focus = matches!(
            key,
            NavKey::Left
                | NavKey::Right
                | NavKey::Up
                | NavKey::Down
                | NavKey::MoveLeft
                | NavKey::MoveRight
                | NavKey::MoveToPrevGroup
                | NavKey::MoveToNextGroup
                | NavKey::CloseTab
        );
        let Some(outcome) = self.with_core(|core| {
            core.focus_visible |= moves_focus;
            if moves_focus {
                // The keyboard takes the focus from the pointer: the centre, ⏎ and M agree.
                core.hovered = -1;
            }
            let outcome = nav::step(&core.order, &core.nav, key);
            core.nav = outcome.state;
            core.nav.query_active = core.search.active;
            outcome
        }) else {
            return false;
        };
        if let Some(intent) = outcome.intent {
            self.run(intent);
        }
        outcome.handled
    }

    fn option_key(self: &Rc<Self>, key: char) {
        let nav_key = match key {
            'A'..='Z' => NavKey::Letter(key),
            '1'..='9' => NavKey::Digit(key as u8 - b'0'),
            _ => return,
        };
        if self.is_closing() {
            return;
        }
        if !self.with_core(|core| core.nav.option_down).unwrap_or(true) {
            self.feed(NavKey::OptionDown);
        }
        self.feed(nav_key);
        self.refresh();
    }

    fn set_held(self: &Rc<Self>, held: bool) {
        if self.is_closing() {
            return;
        }
        if held {
            self.with_core(|core| {
                let down = nav::step(&core.order, &core.nav, NavKey::OptionDown);
                core.nav = down.state;
                let elapsed = nav::step(&core.order, &core.nav, NavKey::HoldElapsed);
                core.nav = elapsed.state;
            });
        } else {
            self.feed(NavKey::OptionUp);
        }
        self.refresh();
    }

    fn start_hold(self: &Rc<Self>) {
        let ms = self.with_core(|core| core.input.hold_ms).unwrap_or(400);
        let weak = Rc::downgrade(self);
        self.timer.start(
            TimerMode::SingleShot,
            Duration::from_millis(u64::from(ms)),
            move || {
                if let Some(shared) = weak.upgrade() {
                    shared.feed(NavKey::HoldElapsed);
                    shared.refresh();
                }
            },
        );
    }

    fn run(self: &Rc<Self>, intent: Intent) {
        match intent {
            Intent::Activate(tab) | Intent::ActivateBead(tab) => self.activate(tab),
            Intent::Close(tab) => self.host.close_tab(tab),
            Intent::Mute(tab) => {
                if self.with_core(|core| core.has_sound(tab)).unwrap_or(false) {
                    self.host.toggle_mute(tab);
                }
            }
            Intent::Move { tab, group, index } => self.host.move_tab(tab, group, index),
            Intent::TypeChar(c) => {
                self.with_core(|core| core.push_char(c));
            }
            Intent::ClearQuery => {
                self.with_core(|core| core.set_query(String::new()));
            }
            Intent::ClosePopover => self.request_close(),
            Intent::BeginHold => self.start_hold(),
            Intent::CancelHold => self.timer.stop(),
            Intent::CommitGroup(group) => {
                if let Some(tab) = self.with_core(|core| core.commit_target(group)).flatten() {
                    self.activate(tab);
                }
            }
            Intent::ActivateDigit { group, n } => {
                let tab = self
                    .with_core(|core| {
                        core.tabs_of(group)
                            .get(usize::from(n).saturating_sub(1))
                            .copied()
                    })
                    .flatten();
                if let Some(tab) = tab {
                    self.activate(tab);
                }
            }
        }
    }

    /// A result row of the centre (the last one is "Search the web").
    fn activate_row(self: &Rc<Self>, row: usize) {
        enum Pick {
            Tab(i32),
            Web(String),
        }
        if self.is_closing() {
            return;
        }
        let pick = self
            .with_core(|core| match core.search.rows.get(row) {
                Some(found) => Some(Pick::Tab(found.tab)),
                None if core.search.active => Some(Pick::Web(core.query.trim().to_owned())),
                None => None,
            })
            .flatten();
        match pick {
            Some(Pick::Tab(tab)) => self.activate(tab),
            Some(Pick::Web(query)) => {
                self.host.search_web(&query);
                self.request_close();
            }
            None => {}
        }
        self.refresh();
    }

    fn key_pressed(self: &Rc<Self>, text: &str, mods: Mods) -> bool {
        let mut field_focused = false;
        self.with_bridge(|bridge| field_focused = bridge.get_field_focused());
        let Some((mapped, searching, selected, rows, closing, looking)) = self.with_core(|core| {
            let context = KeyContext {
                option_look: core.nav.option_down || core.nav.held,
                query_active: core.search.active,
                can_mute: core.centre_tab().is_some_and(|tab| core.has_sound(tab)),
                can_pause: core
                    .centre_tab()
                    .and_then(|tab| core.media(tab))
                    .is_some_and(|media| media.can_pause),
                field_focused,
            };
            (
                map_key(text, mods, &context),
                core.search.active,
                core.selected_row,
                core.search.rows.len() + 1,
                core.closing,
                core.nav.option_down || core.nav.held,
            )
        }) else {
            return false;
        };
        if closing {
            // The popover is closing: the key is taken and nothing happens.
            return true;
        }

        let handled = match mapped {
            Mapped::Ignored => false,
            Mapped::Backspace => self.with_core(Core::pop_char).unwrap_or(false),
            Mapped::TogglePause => {
                if let Some(tab) = self.centre_tab() {
                    self.host.toggle_pause(tab);
                }
                true
            }
            Mapped::Nav(NavKey::Escape) if looking => {
                self.cancel_look();
                true
            }
            Mapped::Nav(NavKey::OptionDown)
                if self.with_core(|core| core.nav.option_down).unwrap_or(false) =>
            {
                true
            }
            Mapped::Nav(NavKey::Up | NavKey::Down) if searching => {
                let up = matches!(mapped, Mapped::Nav(NavKey::Up));
                let next = search::step_row(selected, rows, up);
                self.with_core(|core| core.selected_row = next);
                true
            }
            Mapped::Nav(NavKey::Enter) if searching => {
                self.activate_row(selected);
                true
            }
            Mapped::Nav(key) => self.feed(key),
        };
        self.refresh();
        handled
    }

    fn key_released(self: &Rc<Self>, text: &str) {
        if self.is_closing() {
            return;
        }
        let mut chars = text.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && c == char::from(Key::Alt)
        {
            self.feed(NavKey::OptionUp);
            self.refresh();
        }
    }

    /// The pointer is over a tab (-1: none): the centre shows it and the keyboard focus follows, as
    /// in a menu, so ← →, ⏎ and M then act on the tab the centre names.
    fn tab_hovered(&self, id: i32) {
        self.with_core(|core| {
            core.hovered = id;
            if id >= 0 && core.ring_ids.contains(&id) {
                core.nav.cursor = Cursor::Tab(id);
            }
        });
        self.refresh();
    }

    fn query_edited(&self, query: &str) {
        self.with_core(|core| {
            core.hovered = -1;
            core.set_query(query.to_owned());
        });
        self.refresh();
    }

    fn drop_tab(&self, group: i32, index: i32) {
        let tab = self.dragging.replace(-1);
        if tab >= 0 {
            self.host
                .move_tab(tab, group, usize::try_from(index).unwrap_or(0));
        }
    }

    fn centre_tab(&self) -> Option<i32> {
        self.with_core(|core| core.centre_tab()).flatten()
    }
}

fn install_handlers(bridge: &HubRingBridge<'_>, shared: &Rc<Shared>) {
    let weak = Rc::downgrade(shared);
    macro_rules! with {
        (|$s:ident $(, $arg:ident)*| $body:expr) => {{
            let weak = weak.clone();
            move |$($arg),*| {
                if let Some($s) = weak.upgrade() {
                    $body
                }
            }
        }};
    }

    let hit_weak = weak.clone();
    bridge.on_hit_test(move |x, y, hovered| match hit_weak.upgrade() {
        Some(shared) => shared.hit(x, y, hovered),
        None => RingHit {
            kind: RingHitKind::Outside,
            id: -1,
        },
    });
    let drop_weak = weak.clone();
    bridge.on_drop_target(move |x, y, dragging| match drop_weak.upgrade() {
        Some(shared) => shared.drop_target(x, y, dragging),
        None => RingDropTarget {
            valid: false,
            group: -1,
            index: -1,
            indicator: Default::default(),
        },
    });

    let key_weak = weak.clone();
    bridge.on_key_pressed(move |text, alt, control, meta, _shift| {
        key_weak
            .upgrade()
            .is_some_and(|shared| shared.key_pressed(&text, Mods { alt, control, meta }))
    });
    bridge.on_key_released(with!(
        |shared, text, _alt, _control, _meta, _shift| shared.key_released(&text)
    ));

    bridge.on_query_edited(with!(|shared, query| shared.query_edited(&query)));
    bridge.on_tab_hovered(with!(|shared, id| shared.tab_hovered(id)));
    bridge.on_tab_activated(with!(|shared, id| {
        shared.activate(id);
        shared.refresh();
    }));
    bridge.on_group_activated(with!(|shared, id| {
        if let Some(tab) = shared.with_core(|core| core.commit_target(id)).flatten() {
            shared.activate(tab);
            shared.refresh();
        }
    }));
    bridge.on_tab_close(with!(|shared, id| {
        if !shared.is_closing() {
            shared.host.close_tab(id);
        }
    }));
    bridge.on_tab_mute(with!(|shared, id| {
        if !shared.is_closing() {
            shared.host.toggle_mute(id);
        }
    }));
    bridge.on_bead_activated(with!(|shared, id| {
        shared.activate(id);
        shared.refresh();
    }));
    bridge.on_drop(with!(|shared, group, index| shared.drop_tab(group, index)));
    bridge.on_drag_cancelled(with!(|shared| shared.dragging.set(-1)));
    bridge.on_pause_clicked(with!(|shared| {
        if let Some(tab) = shared.centre_tab().filter(|_| !shared.is_closing()) {
            shared.host.toggle_pause(tab);
        }
    }));
    bridge.on_mute_clicked(with!(|shared| {
        if let Some(tab) = shared.centre_tab().filter(|_| !shared.is_closing()) {
            shared.host.toggle_mute(tab);
        }
    }));
    bridge.on_see_everything(with!(|shared| {
        if !shared.is_closing() {
            shared.host.open_orbit();
            shared.request_close();
            shared.refresh();
        }
    }));
    bridge.on_result_activated(with!(
        |shared, row| shared.activate_row(usize::try_from(row).unwrap_or(0))
    ));
    bridge.on_opened(with!(|shared| shared.opened()));
    bridge.on_closed(with!(|shared| shared.closed()));
}

/// The modifiers of one key event (Shift is not used by any ring key).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Mods {
    alt: bool,
    control: bool,
    meta: bool,
}

/// Whether ⌘ is down. Slint's winit backend maps the keys like Qt does on Apple platforms: ⌘ is
/// reported as `control` and ⌃ as `meta`; elsewhere the "command" chord is `meta` (Super).
fn is_command(mods: Mods) -> bool {
    if cfg!(target_os = "macos") {
        mods.control
    } else {
        mods.meta
    }
}

/// What the mapping of a key needs to know about the ring's state.
#[derive(Clone, Copy, Debug)]
struct KeyContext {
    /// ⌥ is down or the key look is on: printable keys are not text.
    option_look: bool,
    query_active: bool,
    /// The tab the centre is about has sound (M mutes it).
    can_mute: bool,
    /// The tab the centre is about has a media session with a pause command (Space).
    can_pause: bool,
    /// The search field has the keyboard focus and is visible: it takes printable keys and
    /// Backspace itself (an input method starts there).
    field_focused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mapped {
    Nav(NavKey),
    /// Delete the last character of the query.
    Backspace,
    /// Space on an audio tab: pause or play it.
    TogglePause,
    /// Not for the ring: the key goes on to the search field.
    Ignored,
}

/// Slint's key text (one character; the named keys are private-use characters) and modifiers as a
/// [`NavKey`]. ⌥ with a letter or digit arrives composed on macOS ("ß" for ⌥S), so it is never
/// mapped here: the window-level hook calls [`HubRing::option_key`] with the physical key.
fn map_key(text: &str, mods: Mods, context: &KeyContext) -> Mapped {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return Mapped::Ignored;
    };
    let is = |key: Key| c == char::from(key);

    if is(Key::Alt) {
        return Mapped::Nav(NavKey::OptionDown);
    }
    if [
        Key::AltGr,
        Key::Control,
        Key::ControlR,
        Key::Shift,
        Key::ShiftR,
        Key::Meta,
        Key::MetaR,
        Key::CapsLock,
    ]
    .into_iter()
    .any(is)
    {
        return Mapped::Ignored;
    }
    if is(Key::LeftArrow) {
        return Mapped::Nav(if mods.alt {
            NavKey::MoveLeft
        } else {
            NavKey::Left
        });
    }
    if is(Key::RightArrow) {
        return Mapped::Nav(if mods.alt {
            NavKey::MoveRight
        } else {
            NavKey::Right
        });
    }
    if is(Key::UpArrow) {
        return Mapped::Nav(if mods.alt {
            NavKey::MoveToPrevGroup
        } else {
            NavKey::Up
        });
    }
    if is(Key::DownArrow) {
        return Mapped::Nav(if mods.alt {
            NavKey::MoveToNextGroup
        } else {
            NavKey::Down
        });
    }
    if is(Key::Backspace) || is(Key::Delete) {
        return if is_command(mods) {
            Mapped::Nav(NavKey::CloseTab)
        } else if mods.control || mods.meta || context.field_focused {
            // ⌃⌫ (or ⌘⌫ off macOS' mapping) is not ours; with the field focused it edits itself.
            Mapped::Ignored
        } else {
            Mapped::Backspace
        };
    }
    if is(Key::Return) {
        return Mapped::Nav(NavKey::Enter);
    }
    if is(Key::Escape) {
        return Mapped::Nav(NavKey::Escape);
    }
    // ⌘ and ⌃ chords are never text (both, whichever way the platform reports them).
    if c.is_control()
        || ('\u{e000}'..='\u{f8ff}').contains(&c)
        || is_command(mods)
        || mods.control
        || mods.meta
    {
        return Mapped::Ignored;
    }
    // ⌥ with a letter or digit is the composed character here; the hook's `option_key` is the way.
    if mods.alt || context.option_look {
        return Mapped::Ignored;
    }
    if matches!(c, 'm' | 'M') && !context.query_active && context.can_mute {
        return Mapped::Nav(NavKey::Mute);
    }
    if c == ' ' && !context.query_active && !context.field_focused && context.can_pause {
        return Mapped::TogglePause;
    }
    if context.field_focused {
        return Mapped::Ignored;
    }
    Mapped::Nav(NavKey::Type(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REST: KeyContext = KeyContext {
        option_look: false,
        query_active: false,
        can_mute: false,
        can_pause: false,
        field_focused: false,
    };

    fn key(key: Key) -> String {
        char::from(key).to_string()
    }

    fn alt() -> Mods {
        Mods {
            alt: true,
            ..Mods::default()
        }
    }

    /// ⌘ as the platform reports it (`control` on macOS, `meta` elsewhere).
    fn command() -> Mods {
        if cfg!(target_os = "macos") {
            Mods {
                control: true,
                ..Mods::default()
            }
        } else {
            Mods {
                meta: true,
                ..Mods::default()
            }
        }
    }

    /// ⌃ as the platform reports it: the other one.
    fn control() -> Mods {
        if cfg!(target_os = "macos") {
            Mods {
                meta: true,
                ..Mods::default()
            }
        } else {
            Mods {
                control: true,
                ..Mods::default()
            }
        }
    }

    #[test]
    fn arrows_are_moves_with_alt_and_steps_without() {
        let none = Mods::default();
        assert_eq!(
            map_key(&key(Key::LeftArrow), none, &REST),
            Mapped::Nav(NavKey::Left)
        );
        assert_eq!(
            map_key(&key(Key::LeftArrow), alt(), &REST),
            Mapped::Nav(NavKey::MoveLeft)
        );
        assert_eq!(
            map_key(&key(Key::DownArrow), alt(), &REST),
            Mapped::Nav(NavKey::MoveToNextGroup)
        );
        assert_eq!(
            map_key(&key(Key::UpArrow), none, &REST),
            Mapped::Nav(NavKey::Up)
        );
    }

    // Each of these fails on the code before the review round: it read ⌘ from `meta`, so on macOS
    // ⌘⌫ was a plain Backspace and ⌃⌫ closed the tab.
    #[test]
    fn command_backspace_closes_a_tab() {
        assert_eq!(
            map_key(&key(Key::Backspace), command(), &REST),
            Mapped::Nav(NavKey::CloseTab)
        );
    }

    #[test]
    fn control_backspace_does_not_close_a_tab_and_does_not_edit() {
        assert_eq!(
            map_key(&key(Key::Backspace), control(), &REST),
            Mapped::Ignored
        );
    }

    #[test]
    fn plain_backspace_edits_the_query_unless_the_field_has_the_focus() {
        assert_eq!(
            map_key(&key(Key::Backspace), Mods::default(), &REST),
            Mapped::Backspace
        );
        let focused = KeyContext {
            field_focused: true,
            ..REST
        };
        assert_eq!(
            map_key(&key(Key::Backspace), Mods::default(), &focused),
            Mapped::Ignored,
            "the field deletes its own character"
        );
    }

    #[test]
    fn printable_keys_go_to_the_field_while_it_has_the_focus_and_to_the_query_otherwise() {
        let focused = KeyContext {
            field_focused: true,
            ..REST
        };
        assert_eq!(map_key("t", Mods::default(), &focused), Mapped::Ignored);
        assert_eq!(
            map_key("t", Mods::default(), &REST),
            Mapped::Nav(NavKey::Type('t'))
        );
        assert_eq!(
            map_key(&key(Key::RightArrow), Mods::default(), &focused),
            Mapped::Nav(NavKey::Right),
            "arrows always reach the ring first"
        );
    }

    #[test]
    fn space_pauses_only_an_audio_tab_with_an_empty_query_and_no_field_focus() {
        let audio = KeyContext {
            can_pause: true,
            ..REST
        };
        assert_eq!(map_key(" ", Mods::default(), &audio), Mapped::TogglePause);
        let typing = KeyContext {
            query_active: true,
            ..audio
        };
        assert_eq!(
            map_key(" ", Mods::default(), &typing),
            Mapped::Nav(NavKey::Type(' '))
        );
        let focused = KeyContext {
            field_focused: true,
            ..audio
        };
        assert_eq!(map_key(" ", Mods::default(), &focused), Mapped::Ignored);
        assert_eq!(
            map_key(" ", Mods::default(), &REST),
            Mapped::Nav(NavKey::Type(' ')),
            "no media: a plain space"
        );
    }

    #[test]
    fn m_mutes_only_a_tab_with_sound_and_only_before_a_query_starts() {
        let sound = KeyContext {
            can_mute: true,
            ..REST
        };
        let typing = KeyContext {
            query_active: true,
            ..sound
        };
        assert_eq!(
            map_key("m", Mods::default(), &sound),
            Mapped::Nav(NavKey::Mute)
        );
        assert_eq!(
            map_key("m", Mods::default(), &REST),
            Mapped::Nav(NavKey::Type('m'))
        );
        assert_eq!(
            map_key("m", Mods::default(), &typing),
            Mapped::Nav(NavKey::Type('m'))
        );
    }

    #[test]
    fn option_letters_and_digits_are_not_mapped_here_composed_or_not() {
        let looking = KeyContext {
            option_look: true,
            ..REST
        };
        for text in ["s", "S", "2", "ß", "¡", "å"] {
            assert_eq!(map_key(text, alt(), &looking), Mapped::Ignored, "{text}");
            assert_eq!(map_key(text, alt(), &REST), Mapped::Ignored, "{text}");
        }
    }

    #[test]
    fn command_and_control_keys_and_bare_modifiers_are_not_typed() {
        assert_eq!(map_key("v", command(), &REST), Mapped::Ignored);
        assert_eq!(map_key("v", control(), &REST), Mapped::Ignored);
        assert_eq!(
            map_key(&key(Key::Shift), Mods::default(), &REST),
            Mapped::Ignored
        );
        assert_eq!(
            map_key(&key(Key::Alt), alt(), &REST),
            Mapped::Nav(NavKey::OptionDown)
        );
        assert_eq!(
            map_key("é", Mods::default(), &REST),
            Mapped::Nav(NavKey::Type('é'))
        );
    }

    #[test]
    fn patch_touches_nothing_when_the_rows_are_equal_and_only_the_changed_rows_otherwise() {
        let model = VecModel::from(vec![1, 2, 3]);
        assert_eq!(patch(&model, vec![1, 2, 3]), Patched::Unchanged);
        assert_eq!(patch(&model, vec![1, 9, 3]), Patched::Rows(1));
        assert_eq!(model.row_data(1), Some(9));
        assert_eq!(patch(&model, vec![1, 9]), Patched::Reset);
        assert_eq!(model.row_count(), 2);
        assert_eq!(patch(&model, vec![]), Patched::Reset);
        assert_eq!(patch(&model, vec![]), Patched::Unchanged);
    }
}
