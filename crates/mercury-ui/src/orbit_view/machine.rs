// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Orbit zoom state machine (SPEC §4.6 and §6, brief D2, D5, D8, D9, D10): which level, group and
//! tab, what a click, ⏎, Esc, ⌘↑, an arrow key, a pinch or a drag does. Plain data in, [`Effect`]s
//! out; no clock, no animation, no Slint types beyond ids.
//!
//! The host calls [`OrbitMachine::view`] after every call and rebuilds from it; the returned
//! [`Effect`]s say what else to do. [`Effect::ZoomedTo`] is emitted when the level, the shown group
//! (levels 2 and 3) or the shown tab (level 3) changed, that is when the host animates; a move of the
//! focus inside a level (arrows at levels 1 and 2) emits nothing.
//!
//! # Rules the tests pin
//!
//! - While a drag is in flight only Esc is a key and clicks are ignored; while the new-group popover
//!   is open only Esc is a key (it is modal) and clicks are ignored too (a click on a planet must not
//!   zoom with the popover left on screen; Esc or the popover's own buttons close it); a pinch, a
//!   drag and the popover never overlap.
//! - Arrows and Tab are ignored while a text field has the focus (they move its cursor) and with ⌘,
//!   ⌥ or ⌃ (other than ⌘↑ and the ⌥ moves below); `N`, `/` and typed characters are commands only
//!   outside a text field. A typed character answers [`Effect::FocusSearch`]; the host puts the
//!   character into the field and then calls [`OrbitMachine::search_changed`].
//! - Keyboard focus walks the items of the level: ← → (and ⇥ forward, ⇧⇥ back) go to the next or
//!   previous item and clamp at the ends (no wrap). Level 1 walks the planets in group order (the
//!   order of the ring, not "by direction" as SPEC §5 words it: ← → never jump across the stage, a
//!   decision of the brief (D9) kept on purpose; ↑ ↓ take the nearest planet above or below). Level 2 walks every preview in the
//!   order of [`preview_order`], past the 24 that are drawn too: the focus is then a
//!   `Target::Preview(tab)` whose card is not drawn and the centre ring's segment shows it. Level 3
//!   shows the previous or next tab of the strip.
//! - ⌥ + ← → at levels 2 and 3 move the focused (level 2) or shown (level 3) tab one place earlier
//!   or later in its group; ⌥ + ↑ ↓ move it to the end of the previous or next group. The machine
//!   only emits [`Effect::Resolve`] (the host applies it and sends the new rows); nothing happens at
//!   the ends, for a pinned tab or when the move changes nothing. Level 1 ignores ⌥ + arrows.
//! - Every level change puts the keyboard focus on the new level's first item (level 1: the group's
//!   planet; level 2: the focused tab's preview, the active tab's or the first; level 3: the card)
//!   and clears the hover (a stale lit planet must not stay lit after ⌘↑ until the pointer moves).
//! - A tab that is pinned is read-only: no drag starts from it and it is never taken for a new group.

use crate::ring_geometry::Ellipse;
use crate::ring_geometry::orbit::planet_layout;

use super::drag::{self, Decision, DropTarget, GroupOp, Source};
use super::{
    DragView, Effect, Key, KeyContext, LOOSE_GROUP_ID, Level, Mods, NewGroupPopover, OrbitInput,
    Target, View, search,
};

/// `p = clamp((scale - 1) / PINCH_RANGE, -1, 1)` (SPEC §6).
const PINCH_RANGE: f32 = 0.35;
/// A pinch commits a zoom in at this progress ...
const PINCH_IN: f32 = 1.0;
/// ... and a zoom out at this one or below.
const PINCH_OUT: f32 = -0.7;
/// Float slack on the two thresholds (1.35 must commit whatever the rounding).
const PINCH_SLACK: f32 = 1e-4;
/// The stage's planet ellipse, for the order of the planets (↑ ↓ only need the order, which a uniform
/// scale does not change).
const PLANET_ELLIPSE: Ellipse = Ellipse {
    rx: 470.0,
    ry: 290.0,
};

/// A pinch in progress: what was under the pointer at `started` and the latest progress.
#[derive(Clone, Copy, Debug)]
struct Pinch {
    target: Target,
    progress: f32,
}

/// The tabs of `group` in level-2 order (brief D5): the active tab first when it is in the group,
/// then the rest in strip order. Level 2 draws the first 12 on the outer ring and the next 12 on
/// the inner one, so this is also the order of the arrow keys.
pub fn preview_order(input: &OrbitInput, group: i32) -> Vec<i32> {
    let own = input.tabs_of(group);
    let mut order = Vec::with_capacity(own.len());
    if own.iter().any(|tab| tab.id == input.active_tab) {
        order.push(input.active_tab);
    }
    for tab in own {
        if tab.id != input.active_tab {
            order.push(tab.id);
        }
    }
    order
}

/// The tabs of `group` in strip order (level 3's previous and next tab).
fn strip_order(input: &OrbitInput, group: i32) -> Vec<i32> {
    input.tabs_of(group).iter().map(|tab| tab.id).collect()
}

/// A tab Orbit may drag or put in a new group: it exists and is not pinned.
fn free_tab(input: &OrbitInput, tab: i32) -> bool {
    input.tab(tab).is_some() && !input.is_pinned(tab)
}

/// The id at the place where `gone` was in `old`, in `new` (the last one when the list got
/// shorter); `None` when `new` is empty.
fn heir(old: &[i32], gone: Option<i32>, new: &[i32]) -> Option<i32> {
    let index = gone
        .and_then(|id| old.iter().position(|&other| other == id))
        .unwrap_or(0);
    new.get(index.min(new.len().saturating_sub(1))).copied()
}

/// What the host animates: the level, and at levels 2 and 3 the group, and at level 3 the tab.
type Shown = (Level, Option<i32>, Option<i32>);

/// The Orbit state machine.
#[derive(Clone, Debug, Default)]
pub struct OrbitMachine {
    view: View,
    input: OrbitInput,
    session: u64,
    opened: bool,
    pinch: Option<Pinch>,
}

impl OrbitMachine {
    /// A machine for the Orbit that is about to open; `session` is the nonce of the payloads.
    pub fn new(session: u64) -> Self {
        Self {
            session,
            ..Self::default()
        }
    }

    /// The nonce this machine's drags carry.
    pub fn session(&self) -> u64 {
        self.session
    }

    /// Opens at level 1 with the focus on the active tab's group; returns `CapturePicture`.
    pub fn open(&mut self, input: &OrbitInput) -> Vec<Effect> {
        self.input = input.clone();
        self.opened = true;
        self.pinch = None;
        self.view = View::default();
        let group = input
            .tab(input.active_tab)
            .map(|tab| tab.group)
            .filter(|&group| input.group(group).is_some())
            .or_else(|| input.groups.first().map(|group| group.id));
        self.show_groups(group);
        vec![Effect::CapturePicture]
    }

    /// The current state.
    pub fn view(&self) -> View {
        self.view.clone()
    }

    /// A key; ⌘↑ is `Key::Up` with `Mods::COMMAND`.
    pub fn key(&mut self, key: Key, mods: Mods, context: KeyContext) -> Vec<Effect> {
        if key == Key::Escape {
            return self.escape();
        }
        if self.view.drag.is_some() || self.view.popover.is_some() {
            return Vec::new();
        }
        let text = context.text_input_focused;
        match key {
            Key::Up if mods.command => self.zoom_out(),
            Key::Left | Key::Right | Key::Up | Key::Down
                if mods.alt && !mods.command && !mods.control =>
            {
                if text {
                    Vec::new()
                } else {
                    self.move_tab(key)
                }
            }
            _ if mods.command || mods.alt || mods.control => Vec::new(),
            Key::Enter => self.enter(),
            Key::Tab if !text => self.step_focus(if mods.shift { -1 } else { 1 }),
            Key::Left | Key::Right | Key::Up | Key::Down if !text => self.arrow(key),
            Key::N if !text => self.new_group_from_focus(),
            Key::Slash if !text => vec![Effect::FocusSearch],
            Key::Char(c) if !text && !c.is_control() && !c.is_whitespace() => {
                vec![Effect::FocusSearch]
            }
            _ => Vec::new(),
        }
    }

    /// A click (primary button, released) on `target`. Ignored during a drag, a pinch and while the
    /// new-group popover is open (it is modal).
    pub fn click(&mut self, target: Target) -> Vec<Effect> {
        if self.view.drag.is_some() || self.pinch.is_some() || self.view.popover.is_some() {
            return Vec::new();
        }
        self.activate(target)
    }

    /// The pointer is over `target` (`Target::Backdrop` for nothing).
    pub fn hover(&mut self, target: Target) {
        self.view.hover = match target {
            Target::Backdrop => None,
            other => Some(other),
        };
    }

    /// A pinch started with the pointer over `target` (D10). Ignored while another gesture, a drag
    /// or the popover is up.
    pub fn pinch_started(&mut self, target: Target) {
        if self.pinch.is_some() || self.view.drag.is_some() || self.view.popover.is_some() {
            return;
        }
        self.pinch = Some(Pinch {
            target,
            progress: 0.0,
        });
        self.view.pinch = Some(0.0);
    }

    /// The pinch's cumulative scale (1.0 at the start).
    pub fn pinch_updated(&mut self, scale: f32) {
        if !scale.is_finite() {
            return;
        }
        if let Some(pinch) = self.pinch.as_mut() {
            let progress = ((scale - 1.0) / PINCH_RANGE).clamp(-1.0, 1.0);
            pinch.progress = progress;
            self.view.pinch = Some(progress);
        }
    }

    /// The pinch ended: commit, or `Springback`.
    pub fn pinch_ended(&mut self) -> Vec<Effect> {
        let Some(pinch) = self.pinch.take() else {
            return Vec::new();
        };
        self.view.pinch = None;
        if pinch.progress >= PINCH_IN - PINCH_SLACK {
            self.pinch_in(pinch.target)
        } else if pinch.progress <= PINCH_OUT + PINCH_SLACK {
            self.zoom_out()
        } else {
            vec![Effect::Springback]
        }
    }

    /// The pinch was cancelled: `Springback`.
    pub fn pinch_cancelled(&mut self) -> Vec<Effect> {
        if self.pinch.take().is_none() {
            return Vec::new();
        }
        self.view.pinch = None;
        vec![Effect::Springback]
    }

    /// A drag of `source` started. A pinned tab, the loose planet and unknown ids cannot be dragged.
    pub fn drag_started(&mut self, source: Source) {
        if self.view.drag.is_some() || self.view.popover.is_some() || self.pinch.is_some() {
            return;
        }
        if !self.can_drag(source) {
            return;
        }
        self.view.drag = Some(DragView { source, over: None });
    }

    /// The drag is over `target` now.
    pub fn drag_over(&mut self, target: Option<DropTarget>) {
        if let Some(drag) = self.view.drag.as_mut() {
            drag.over = target;
        }
    }

    /// The drag was released where it is: `Resolve(..)` or nothing. A drop that makes a new group
    /// also opens the popover (`Resolve(NewGroup)`, then `OpenNewGroupPopover`).
    pub fn drag_released(&mut self) -> Vec<Effect> {
        let Some(drag) = self.view.drag.take() else {
            return Vec::new();
        };
        let Some(over) = drag.over else {
            return Vec::new();
        };
        let decision = drag::resolve(drag.source, over, &self.input);
        let mut effects = vec![Effect::Resolve(decision.clone())];
        if let Decision::NewGroup { tab } = decision {
            self.view.popover = Some(NewGroupPopover {
                tab,
                ..NewGroupPopover::default()
            });
            effects.push(Effect::OpenNewGroupPopover { tab });
        }
        effects
    }

    /// The drag was cancelled.
    pub fn drag_cancelled(&mut self) {
        self.view.drag = None;
    }

    /// The rows changed while Orbit is open: reconcile the state with them.
    pub fn rows_changed(&mut self, input: &OrbitInput) -> Vec<Effect> {
        let old = std::mem::replace(&mut self.input, input.clone());
        if !self.opened {
            return Vec::new();
        }
        let before = self.shown();
        self.reconcile(&old);
        if self
            .view
            .drag
            .as_ref()
            .is_some_and(|drag| !self.can_drag(drag.source))
        {
            self.view.drag = None;
        }
        if self
            .view
            .popover
            .as_ref()
            .is_some_and(|popover| !free_tab(&self.input, popover.tab))
        {
            self.view.popover = None;
        }
        if let Some(hover) = self.view.hover {
            let there = match hover {
                Target::Planet(group) | Target::Chip(group) => self.input.group(group).is_some(),
                Target::Preview(tab) | Target::Neighbour(tab) => self.input.tab(tab).is_some(),
                Target::Sun | Target::NewGroup | Target::Card | Target::Backdrop => true,
            };
            if !there {
                self.view.hover = None;
            }
        }
        self.changed(before)
    }

    /// The search field's text changed.
    pub fn search_changed(&mut self, query: &str) {
        self.view.query = query.to_string();
    }

    /// The new-group popover was confirmed with this name and colour preset: `Apply(CreateGroup ..)`.
    /// An empty name or colour is left to the controller (`None`).
    pub fn popover_confirmed(&mut self, name: &str, colour: &str) -> Vec<Effect> {
        let Some(popover) = self.view.popover.take() else {
            return Vec::new();
        };
        if !free_tab(&self.input, popover.tab) {
            return Vec::new();
        }
        let non_empty = |text: &str| {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        };
        vec![Effect::Apply(GroupOp::CreateGroup {
            tabs: vec![popover.tab],
            name: non_empty(name),
            // The popover shows the first dot selected, so an untouched confirm sends that colour.
            colour: non_empty(colour)
                .or_else(|| Some(super::data::DEFAULT_POPOVER_COLOUR.to_string())),
        })]
    }

    /// The name or colour in the popover changed.
    pub fn popover_edited(&mut self, name: &str, colour: &str) {
        if let Some(popover) = self.view.popover.as_mut() {
            popover.name = name.to_string();
            popover.colour = colour.to_string();
        }
    }

    // -----------------------------------------------------------------------------------------
    // States

    fn shown(&self) -> Shown {
        let level = self.view.level;
        (
            level,
            (level != Level::Groups)
                .then_some(self.view.group)
                .flatten(),
            (level == Level::Tab).then_some(self.view.tab).flatten(),
        )
    }

    /// `ZoomedTo` when what is shown changed since `before`; the hover is cleared then (the pointer
    /// has not moved over the new stage yet).
    fn changed(&mut self, before: Shown) -> Vec<Effect> {
        if self.shown() == before {
            Vec::new()
        } else {
            self.view.hover = None;
            vec![Effect::ZoomedTo(self.view.clone())]
        }
    }

    fn show_groups(&mut self, group: Option<i32>) {
        self.view.level = Level::Groups;
        self.view.group = group;
        self.view.tab = None;
        self.view.focus = group.map(Target::Planet);
    }

    fn show_group(&mut self, group: i32, tab: Option<i32>) {
        self.view.level = Level::Group;
        self.view.group = Some(group);
        self.view.tab = tab;
        self.view.focus = tab.map(Target::Preview);
    }

    fn show_tab(&mut self, group: i32, tab: i32) {
        self.view.level = Level::Tab;
        self.view.group = Some(group);
        self.view.tab = Some(tab);
        self.view.focus = Some(Target::Card);
    }

    fn enter_group(&mut self, group: i32) {
        let first = preview_order(&self.input, group).first().copied();
        self.show_group(group, first);
    }

    fn in_shown_group(&self, tab: i32) -> bool {
        self.view.group.is_some() && self.input.tab(tab).map(|row| row.group) == self.view.group
    }

    /// Whether `source` may be dragged (or moved by a key): a tab that exists and is not pinned, or
    /// a real group (not the loose planet).
    pub fn can_drag(&self, source: Source) -> bool {
        match source {
            Source::Tab(tab) => free_tab(&self.input, tab),
            Source::Group(group) => group != LOOSE_GROUP_ID && self.input.group(group).is_some(),
        }
    }

    // -----------------------------------------------------------------------------------------
    // Click, ⏎, pinch in: "do what a click on this does" (D9)

    fn activate(&mut self, target: Target) -> Vec<Effect> {
        let before = self.shown();
        match (self.view.level, target) {
            (Level::Groups, Target::Planet(group)) if self.input.group(group).is_some() => {
                self.enter_group(group);
            }
            (Level::Group, Target::Preview(tab)) if self.in_shown_group(tab) => {
                if let Some(group) = self.view.group {
                    self.show_tab(group, tab);
                }
            }
            (Level::Group, Target::Chip(group))
                if self.input.group(group).is_some() && self.view.group != Some(group) =>
            {
                self.enter_group(group);
            }
            (Level::Tab, Target::Card) => {
                return vec![Effect::Close {
                    activate: self.view.tab,
                }];
            }
            (Level::Tab, Target::Neighbour(tab)) if self.in_shown_group(tab) => {
                if let Some(group) = self.view.group {
                    self.show_tab(group, tab);
                }
            }
            (Level::Tab, Target::Backdrop) => {
                if let Some(group) = self.view.group {
                    self.show_group(group, self.view.tab);
                }
            }
            _ => {}
        }
        self.changed(before)
    }

    fn enter(&mut self) -> Vec<Effect> {
        if self.view.level != Level::Tab && !self.view.query.trim().is_empty() {
            return match search::top_result(&self.view.query, &self.input) {
                Some(tab) => self.goto_tab(tab),
                None => Vec::new(),
            };
        }
        match self.view.focus {
            Some(target) => self.activate(target),
            None => Vec::new(),
        }
    }

    /// Level 3 of `tab`, whatever group it is in.
    fn goto_tab(&mut self, tab: i32) -> Vec<Effect> {
        let Some(row) = self.input.tab(tab) else {
            return Vec::new();
        };
        let group = row.group;
        let before = self.shown();
        self.show_tab(group, tab);
        self.changed(before)
    }

    fn zoom_out(&mut self) -> Vec<Effect> {
        let before = self.shown();
        match (self.view.level, self.view.group) {
            (Level::Tab, Some(group)) => self.show_group(group, self.view.tab),
            (Level::Group, group) => self.show_groups(group),
            _ => return vec![Effect::Close { activate: None }],
        }
        self.changed(before)
    }

    fn pinch_in(&mut self, target: Target) -> Vec<Effect> {
        if target == Target::Backdrop {
            return vec![Effect::Springback];
        }
        let effects = self.activate(target);
        if effects.is_empty() {
            vec![Effect::Springback]
        } else {
            effects
        }
    }

    // -----------------------------------------------------------------------------------------
    // Esc, the popover and the arrows

    /// Layers (D8): a drag, the popover, a non-empty search, then the page.
    fn escape(&mut self) -> Vec<Effect> {
        if self.view.drag.take().is_some() || self.view.popover.take().is_some() {
            return Vec::new();
        }
        if !self.view.query.is_empty() {
            self.view.query.clear();
            return vec![Effect::ClearSearch];
        }
        vec![Effect::Close { activate: None }]
    }

    fn new_group_from_focus(&mut self) -> Vec<Effect> {
        let Some(tab) = self.new_group_tab() else {
            return Vec::new();
        };
        self.view.popover = Some(NewGroupPopover {
            tab,
            ..NewGroupPopover::default()
        });
        vec![Effect::OpenNewGroupPopover { tab }]
    }

    /// Levels 2 and 3: the focused tab. Level 1: the active tab, else the first free tab of the
    /// focused group, else the first free tab there is.
    fn new_group_tab(&self) -> Option<i32> {
        if self.view.level != Level::Groups {
            return self.view.tab.filter(|&tab| free_tab(&self.input, tab));
        }
        let free = |tab: &i32| free_tab(&self.input, *tab);
        Some(self.input.active_tab)
            .filter(free)
            .or_else(|| {
                self.view
                    .group
                    .and_then(|group| strip_order(&self.input, group).into_iter().find(free))
            })
            .or_else(|| self.input.tabs.iter().map(|tab| tab.id).find(free))
    }

    fn arrow(&mut self, key: Key) -> Vec<Effect> {
        match key {
            Key::Left => self.step_focus(-1),
            Key::Right => self.step_focus(1),
            _ if self.view.level == Level::Groups => {
                if let Some(group) = self.vertical_neighbour(key == Key::Up) {
                    self.show_groups(Some(group));
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// ← → and ⇥ ⇧⇥: the focus (level 3: the shown tab) goes `delta` items on, clamped at the ends.
    /// Level 2 walks every preview, the undrawn ones past the first 24 included (F14).
    fn step_focus(&mut self, delta: isize) -> Vec<Effect> {
        let before = self.shown();
        match (self.view.level, self.view.group) {
            (Level::Groups, _) => {
                let ids: Vec<i32> = self.input.groups.iter().map(|group| group.id).collect();
                if let Some(group) = step(&ids, self.view.group, delta) {
                    self.show_groups(Some(group));
                }
            }
            (Level::Group, Some(group)) => {
                let order = preview_order(&self.input, group);
                if let Some(tab) = step(&order, self.view.tab, delta) {
                    self.show_group(group, Some(tab));
                }
            }
            (Level::Tab, Some(group)) => {
                let strip = strip_order(&self.input, group);
                if let Some(tab) = step(&strip, self.view.tab, delta) {
                    self.show_tab(group, tab);
                }
            }
            _ => {}
        }
        self.changed(before)
    }

    /// ⌥ + an arrow at levels 2 and 3: ← → one place earlier or later in the strip of the tab's
    /// group, ↑ ↓ the end of the previous or next group (group order). The machine's own view does
    /// not change; the host applies the decision and sends the new rows.
    fn move_tab(&mut self, key: Key) -> Vec<Effect> {
        if self.view.level == Level::Groups {
            return Vec::new();
        }
        let Some(tab) = self.view.tab else {
            return Vec::new();
        };
        if !self.can_drag(Source::Tab(tab)) {
            return Vec::new();
        }
        let Some(group) = self.input.tab(tab).map(|row| row.group) else {
            return Vec::new();
        };
        let target = match key {
            Key::Left | Key::Right => {
                let strip = strip_order(&self.input, group);
                let Some(at) = strip.iter().position(|&id| id == tab) else {
                    return Vec::new();
                };
                // The slot counts the moved tab itself: one place later is slot `at + 2`.
                let slot = if key == Key::Left {
                    at.checked_sub(1)
                } else {
                    (at + 1 < strip.len()).then_some(at + 2)
                };
                slot.map(|slot| DropTarget::Group {
                    group,
                    slot: Some(slot),
                })
            }
            _ => {
                let ids: Vec<i32> = self.input.groups.iter().map(|row| row.id).collect();
                let at = ids.iter().position(|&id| id == group);
                let other = match (key, at) {
                    (Key::Up, Some(at)) => at.checked_sub(1),
                    (_, Some(at)) => Some(at + 1),
                    _ => None,
                }
                .and_then(|index| ids.get(index).copied());
                other.map(|group| DropTarget::Group { group, slot: None })
            }
        };
        let Some(target) = target else {
            return Vec::new();
        };
        match drag::resolve(Source::Tab(tab), target, &self.input) {
            decision @ Decision::MoveTab { .. } => vec![Effect::Resolve(decision)],
            _ => Vec::new(),
        }
    }

    /// The planet above (`up`) or below the focused one that is nearest to it on the stage ellipse;
    /// ties go to the earlier planet.
    fn vertical_neighbour(&self, up: bool) -> Option<i32> {
        let ids: Vec<i32> = self.input.groups.iter().map(|group| group.id).collect();
        let current = ids.iter().position(|&id| Some(id) == self.view.group)?;
        let centres = planet_layout(ids.len(), PLANET_ELLIPSE).centres;
        let from = *centres.get(current)?;
        let mut best: Option<(f32, usize)> = None;
        for (index, centre) in centres.iter().enumerate() {
            let dy = centre.y - from.y;
            let on_that_side = if up { dy < -1.0 } else { dy > 1.0 };
            if index == current || !on_that_side {
                continue;
            }
            let distance = (centre.x - from.x).hypot(dy);
            if best.is_none_or(|(nearest, _)| distance < nearest - 0.5) {
                best = Some((distance, index));
            }
        }
        best.and_then(|(_, index)| ids.get(index).copied())
    }

    // -----------------------------------------------------------------------------------------
    // Rows changed

    fn reconcile(&mut self, old: &OrbitInput) {
        let old_groups: Vec<i32> = old.groups.iter().map(|group| group.id).collect();
        let groups: Vec<i32> = self.input.groups.iter().map(|group| group.id).collect();
        let group_is_there = |group: Option<i32>| group.is_some_and(|id| groups.contains(&id));
        let heir_group = |group: Option<i32>| heir(&old_groups, group, &groups);

        if self.input.tabs.is_empty() {
            let group = if group_is_there(self.view.group) {
                self.view.group
            } else {
                heir_group(self.view.group)
            };
            self.show_groups(group);
            return;
        }
        match self.view.level {
            Level::Groups => {
                let group = if group_is_there(self.view.group) {
                    self.view.group
                } else {
                    heir_group(self.view.group)
                };
                self.show_groups(group);
            }
            Level::Group | Level::Tab => {
                let Some(group) = self.view.group.filter(|&id| groups.contains(&id)) else {
                    let group = heir_group(self.view.group);
                    self.show_groups(group);
                    return;
                };
                let order = preview_order(&self.input, group);
                let tab_stays = self.view.tab.filter(|&tab| order.contains(&tab));
                if self.view.level == Level::Tab && tab_stays.is_some() {
                    return;
                }
                let tab =
                    tab_stays.or_else(|| heir(&preview_order(old, group), self.view.tab, &order));
                self.show_group(group, tab);
            }
        }
    }
}

/// The element `delta` places from `current` in `list`, clamped to the list; the current element is
/// where the list starts when it is not found; `None` when it does not change.
fn step(list: &[i32], current: Option<i32>, delta: isize) -> Option<i32> {
    if list.is_empty() {
        return None;
    }
    let index = current
        .and_then(|id| list.iter().position(|&other| other == id))
        .unwrap_or(0);
    let next = (index as isize + delta).clamp(0, list.len() as isize - 1) as usize;
    let id = list.get(next).copied()?;
    (Some(id) != current).then_some(id)
}
