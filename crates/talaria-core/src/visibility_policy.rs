// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! When the visible page is hidden because its window is covered or minimised (energy-1 lane).
//!
//! Servo 0.8 couples visibility and throttling: a hidden web view's timers slow down and its
//! animation frames stop. So "throttle" and "hide" are one decision, made here without
//! touching the engine; the caller applies the answer to the visible tab's web view.
//!
//! The clock is the `now` argument, as in the other policies, so a test never sleeps.

use std::time::{Duration, Instant};

/// How long a window must stay covered before its page is hidden. Covering comes and goes
/// (a window dragged across another, Mission Control, the Command-Tab animation); hiding and
/// showing on every flap would cost more than it saves.
pub const OCCLUDED_HIDE_DELAY: Duration = Duration::from_secs(1);
/// A minimised window is hidden at once: nobody is about to look at it.
pub const MINIMISED_HIDE_DELAY: Duration = Duration::ZERO;

/// Whether the user can see the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowCover {
    #[default]
    Uncovered,
    /// Behind other windows, on another Space, or on a sleeping display.
    Occluded,
    Minimised,
}

/// Why a covered page keeps running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exemption {
    /// The page makes sound: throttled timers would stutter it.
    PlayingAudio,
    /// The page may use the camera or the microphone: a call or a recording goes on.
    Capture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VisibilityInputs {
    pub cover: WindowCover,
    /// The visible tab plays audible sound (a tab the user muted does not).
    pub playing_audio: bool,
    /// The visible tab's page may use the camera or the microphone.
    pub capturing: bool,
    /// The system's Low Power Mode is on.
    pub low_power_mode: bool,
}

impl VisibilityInputs {
    pub const fn exemption(&self) -> Option<Exemption> {
        if self.playing_audio {
            Some(Exemption::PlayingAudio)
        } else if self.capturing {
            Some(Exemption::Capture)
        } else {
            None
        }
    }

    /// How long the cover must last before the page is hidden. Low Power Mode does not wait.
    pub const fn hide_delay(&self) -> Duration {
        if self.low_power_mode {
            return Duration::ZERO;
        }
        match self.cover {
            WindowCover::Uncovered => Duration::ZERO,
            WindowCover::Occluded => OCCLUDED_HIDE_DELAY,
            WindowCover::Minimised => MINIMISED_HIDE_DELAY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewVisibility {
    #[default]
    Shown,
    Hidden,
}

/// Decides, from the window's cover and the exemptions, whether the visible page is shown.
///
/// Showing is immediate. Hiding waits out `VisibilityInputs::hide_delay`, counted from when
/// the window was last uncovered or last exempt; every exemption that ends restarts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VisibilityPolicy {
    visibility: ViewVisibility,
    covered_since: Option<Instant>,
}

impl VisibilityPolicy {
    /// What the visible page's visibility should be at `now`.
    pub fn update(&mut self, inputs: VisibilityInputs, now: Instant) -> ViewVisibility {
        if inputs.cover == WindowCover::Uncovered || inputs.exemption().is_some() {
            self.covered_since = None;
            self.visibility = ViewVisibility::Shown;
            return self.visibility;
        }
        let since = *self.covered_since.get_or_insert(now);
        if self.visibility == ViewVisibility::Shown
            && now.saturating_duration_since(since) >= inputs.hide_delay()
        {
            self.visibility = ViewVisibility::Hidden;
        }
        self.visibility
    }

    pub const fn visibility(&self) -> ViewVisibility {
        self.visibility
    }

    /// When `update` will hide the page if `inputs` stay as they are: the moment to look
    /// again. `None` when nothing is pending (uncovered, exempt, already hidden, or not yet
    /// updated since the cover began).
    pub fn hide_due(&self, inputs: VisibilityInputs) -> Option<Instant> {
        if self.visibility == ViewVisibility::Hidden
            || inputs.cover == WindowCover::Uncovered
            || inputs.exemption().is_some()
        {
            return None;
        }
        self.covered_since.map(|since| since + inputs.hide_delay())
    }
}
