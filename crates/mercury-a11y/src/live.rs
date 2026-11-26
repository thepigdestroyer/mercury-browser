// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Live regions: which nodes are live, what text was added, and what is spoken, with a per-second
//! budget.
//!
//! `aria-relevant` is not read: AccessKit has no such property, so every change counts as an
//! addition of text (the ARIA default).

use std::collections::HashMap;

use a11y_dom::{Node, Rendering};
use accesskit::Role;

use crate::host::HostState;
use crate::roles;

/// The most characters of added text that are spoken for one non-atomic update.
const MAX_ADDED: usize = 1000;

/// How urgently a live region is spoken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Politeness {
    Off,
    Polite,
    Assertive,
}

/// The live-region attributes of one node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveState {
    pub politeness: Politeness,
    pub atomic: bool,
    pub busy: bool,
}

/// The live state of a node: `None` when it is not a live region. An explicit `aria-live` wins over
/// the implicit politeness of the role; `aria-atomic` wins over the role's default.
pub fn live_state<'n, D: Rendering + HostState>(doc: &'n D, node: D::N<'n>) -> Option<LiveState> {
    let explicit =
        node.attr("aria-live")
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "off" => Some(Politeness::Off),
                "polite" => Some(Politeness::Polite),
                "assertive" => Some(Politeness::Assertive),
                _ => None,
            });
    let (implicit, implicit_atomic) = match roles::role(doc, node) {
        Role::Alert => (Some(Politeness::Assertive), true),
        Role::Status => (Some(Politeness::Polite), true),
        Role::Log => (Some(Politeness::Polite), false),
        Role::Marquee | Role::Timer => (Some(Politeness::Off), false),
        _ => (None, false),
    };
    let politeness = explicit.or(implicit)?;
    let atomic = flag(node.attr("aria-atomic")).unwrap_or(implicit_atomic);
    let busy = flag(node.attr("aria-busy")) == Some(true);
    Some(LiveState {
        politeness,
        atomic,
        busy,
    })
}

/// `true` or `false`; anything else is "not specified".
fn flag(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// The text after the longest common prefix of `old` and `new`, at most `limit` characters; `None`
/// when nothing was added.
pub fn diff_added(old: &str, new: &str, limit: usize) -> Option<String> {
    let common: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let added: String = new[common..].chars().take(limit).collect();
    (!added.is_empty()).then_some(added)
}

/// Text to speak, and whether it interrupts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spoken {
    pub text: String,
    pub assertive: bool,
}

/// Remembers the last text of each live region and how much is spoken per second.
pub struct Announcer {
    max_per_second: u32,
    texts: HashMap<u64, String>,
    window_start: Option<u64>,
    spoken_in_window: u32,
}

impl Announcer {
    pub fn new(max_per_second: u32) -> Self {
        Self {
            max_per_second,
            texts: HashMap::new(),
            window_start: None,
            spoken_in_window: 0,
        }
    }

    /// The text to speak for `text` in the region `key` at `now_ms`, or `None`. The first sight of a
    /// region (`initial`) only remembers its text, so a page that loads is silent. A busy region is
    /// silent and keeps its old text until it clears. An atomic region speaks the whole text; any
    /// other speaks what was added.
    pub fn observe(
        &mut self,
        now_ms: u64,
        key: u64,
        state: &LiveState,
        text: &str,
        initial: bool,
    ) -> Option<Spoken> {
        if initial {
            self.texts.insert(key, text.to_string());
            return None;
        }
        if state.busy {
            return None;
        }
        let old = self.texts.insert(key, text.to_string()).unwrap_or_default();
        if state.politeness == Politeness::Off {
            return None;
        }
        let spoken = if state.atomic {
            (text != old && !text.is_empty()).then(|| text.to_string())
        } else {
            diff_added(&old, text, MAX_ADDED)
        }?;
        if !self.take_budget(now_ms) {
            return None;
        }
        Some(Spoken {
            text: spoken,
            assertive: state.politeness == Politeness::Assertive,
        })
    }

    /// Drops the remembered text of the region `key`.
    pub fn forget(&mut self, key: u64) {
        self.texts.remove(&key);
    }

    /// Counts one spoken update in the one-second window that starts at the first spoken update of
    /// that window. `false` once `max_per_second` have been spoken in it.
    fn take_budget(&mut self, now_ms: u64) -> bool {
        match self.window_start {
            Some(start) if now_ms.saturating_sub(start) < 1000 => {}
            _ => {
                self.window_start = Some(now_ms);
                self.spoken_in_window = 0;
            }
        }
        if self.spoken_in_window < self.max_per_second {
            self.spoken_in_window += 1;
            true
        } else {
            false
        }
    }
}
