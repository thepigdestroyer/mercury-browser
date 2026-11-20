// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The ⌥ state machine (lane brief §5.2 to §5.4; SPEC §5.1).
//!
//! GROUNDWORK STUB. The signatures below are the frozen contract of the lane; item 1 `machine`
//! writes the body. Remove this paragraph when you do.

use std::time::Duration;

use crate::types::{CardView, Config, Event, Focus, KeyedGroup, Outcome, Phase};

/// The state machine of the ⌥ group keys. See the brief's section 5.4 for every transition.
#[derive(Debug)]
pub struct OptionKeys {
    #[allow(dead_code)] // the stub keeps the configuration; item 1 replaces the fields
    config: Config,
}

impl OptionKeys {
    /// A machine in [`Phase::Idle`] with no groups and the focus outside any text field.
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Follows a changed setting. Disabling the keys, or choosing a mode without a hold path,
    /// closes the card (`Hide`) and ends any capture.
    pub fn set_config(&mut self, _config: Config) -> Outcome {
        todo!("key-switcher item 1: OptionKeys::set_config")
    }

    /// Follows the groups that have keys, in ring order: at most 26, the first group keeps a
    /// duplicated letter. A selection that no longer exists is cleared (`Update`).
    pub fn set_groups(&mut self, _groups: &[KeyedGroup]) -> Outcome {
        todo!("key-switcher item 1: OptionKeys::set_groups")
    }

    /// Follows the keyboard focus. `composing` cancels Armed (to `PassThrough`) and an open card
    /// (`Hide`).
    pub fn set_focus(&mut self, _focus: Focus) -> Outcome {
        todo!("key-switcher item 1: OptionKeys::set_focus")
    }

    /// Handles one event at `now`. The hold deadline is applied first, so a late timer never
    /// changes what a key does.
    pub fn on_event(&mut self, _now: Duration, _event: Event) -> Outcome {
        todo!("key-switcher item 1: OptionKeys::on_event")
    }

    /// The hold timer: opens the card when `now` has reached the deadline.
    pub fn poll(&mut self, _now: Duration) -> Outcome {
        todo!("key-switcher item 1: OptionKeys::poll")
    }

    /// When the card will open if nothing else happens: `Some(t0 + hold)` only in Armed with a
    /// hold path (the keys are on, the mode has one, and some group has a key).
    pub fn deadline(&self) -> Option<Duration> {
        todo!("key-switcher item 1: OptionKeys::deadline")
    }

    /// What the card, the dock and the ring show now.
    pub fn view(&self) -> CardView {
        todo!("key-switcher item 1: OptionKeys::view")
    }

    /// The current phase.
    pub fn phase(&self) -> Phase {
        todo!("key-switcher item 1: OptionKeys::phase")
    }
}
