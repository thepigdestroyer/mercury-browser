// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Orbit zoom state machine (SPEC §4.6 and §6, brief D2, D8, D9, D10): which level, group and
//! tab, what a click, ⏎, Esc, ⌘↑, an arrow key, a pinch or a drag does. Plain data in, [`Effect`]s
//! out; no clock, no animation, no Slint types beyond ids.
//!
//! STUB in the scaffold commit: every method has its final signature and does nothing; item 1
//! `machine` of lane orbit-view writes the real bodies.

use super::drag::{DropTarget, Source};
use super::{Effect, Key, KeyContext, Mods, OrbitInput, Target, View};

/// The Orbit state machine.
#[derive(Clone, Debug, Default)]
pub struct OrbitMachine {
    view: View,
    input: OrbitInput,
    session: u64,
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
        Vec::new()
    }

    /// The current state.
    pub fn view(&self) -> View {
        self.view.clone()
    }

    /// A key; ⌘↑ is `Key::Up` with `Mods::COMMAND`.
    pub fn key(&mut self, _key: Key, _mods: Mods, _context: KeyContext) -> Vec<Effect> {
        Vec::new()
    }

    /// A click (primary button, released) on `target`.
    pub fn click(&mut self, _target: Target) -> Vec<Effect> {
        Vec::new()
    }

    /// The pointer is over `target` (`Target::Backdrop` for nothing).
    pub fn hover(&mut self, _target: Target) {}

    /// A pinch started with the pointer over `target` (D10).
    pub fn pinch_started(&mut self, _target: Target) {}

    /// The pinch's cumulative scale (1.0 at the start).
    pub fn pinch_updated(&mut self, _scale: f32) {}

    /// The pinch ended: commit, or `Springback`.
    pub fn pinch_ended(&mut self) -> Vec<Effect> {
        Vec::new()
    }

    /// The pinch was cancelled: `Springback`.
    pub fn pinch_cancelled(&mut self) -> Vec<Effect> {
        Vec::new()
    }

    /// A drag of `source` started.
    pub fn drag_started(&mut self, _source: Source) {}

    /// The drag is over `target` now.
    pub fn drag_over(&mut self, _target: Option<DropTarget>) {}

    /// The drag was released where it is: `Resolve(..)` or nothing.
    pub fn drag_released(&mut self) -> Vec<Effect> {
        Vec::new()
    }

    /// The drag was cancelled.
    pub fn drag_cancelled(&mut self) {}

    /// The rows changed while Orbit is open: reconcile the state with them.
    pub fn rows_changed(&mut self, input: &OrbitInput) -> Vec<Effect> {
        self.input = input.clone();
        Vec::new()
    }

    /// The search field's text changed.
    pub fn search_changed(&mut self, _query: &str) {}

    /// The new-group popover was confirmed with this name and colour preset: `Apply(CreateGroup ..)`.
    pub fn popover_confirmed(&mut self, _name: &str, _colour: &str) -> Vec<Effect> {
        Vec::new()
    }
}
