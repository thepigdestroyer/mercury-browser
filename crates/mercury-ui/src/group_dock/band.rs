// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The reserved band of the dock and the settle rule (SPEC §4.4: "the dock reserves a band at the
//! bottom of the page area: 72 px when open, 40 px when shrunk. Shrinking or expanding animates the
//! dock, then resizes the web view once at the end"). Pure: no Slint types, no clock of its own
//! (the caller passes the time in milliseconds, from any monotonic source). The public signatures
//! are the pinned contract of lane group-dock.

/// Height of the dock at rest.
pub const DOCK_HEIGHT: f32 = 52.0;
/// Height of a pill at rest (the dock is `2 * PAD_Y + PILL_HEIGHT * max_scale` high).
pub const PILL_HEIGHT: f32 = 30.0;
/// Space above and below the pills inside the dock.
pub const PAD_Y: f32 = 11.0;
/// Space below the dock and below the capsule, inside the band.
pub const BOTTOM_MARGIN: f32 = 12.0;
/// The band reserved while the dock is open.
pub const BAND_OPEN: f32 = 72.0;
/// The band reserved while the dock is shrunk.
pub const BAND_SHRUNK: f32 = 40.0;
/// The capsule's width.
pub const CAPSULE_WIDTH: f32 = 236.0;
/// The capsule's height.
pub const CAPSULE_HEIGHT: f32 = 28.0;
/// The shrink / expand animation (SPEC §7); the web view is resized when it ends.
pub const SHRINK_MS: u32 = 180;

/// The band the dock reserves: 72 open, 40 shrunk.
pub fn reserved_band_height(shrunk: bool) -> f32 {
    if shrunk { BAND_SHRUNK } else { BAND_OPEN }
}

/// The dock's height for the largest scale in use: `2 * PAD_Y + PILL_HEIGHT * max_scale`; 52 at
/// 1.0, and a NaN, infinite or smaller-than-1 scale counts as 1.0.
pub fn dock_height(max_scale: f32) -> f32 {
    let scale = if max_scale.is_finite() {
        max_scale.max(1.0)
    } else {
        1.0
    };
    2.0 * PAD_Y + PILL_HEIGHT * scale
}

/// SPEC §3.5: the window's saved `dock_shrunk` wins; `None` follows `appearance.tab_groups.dock_starts`.
pub fn initial_shrunk(session: Option<bool>, starts_shrunk: bool) -> bool {
    session.unwrap_or(starts_shrunk)
}

/// The height of the page area: `content_height` minus the band (open or shrunk), never negative; a
/// NaN or infinite input gives 0.
pub fn page_height(content_height: f32, shrunk_band: bool) -> f32 {
    if !content_height.is_finite() {
        return 0.0;
    }
    (content_height - reserved_band_height(shrunk_band)).max(0.0)
}

/// Which band the web view has been sized for, and when the animation ends (D5 of the brief: the
/// web view follows the SETTLED state, the dock follows the request).
#[derive(Clone, Debug)]
pub struct DockState {
    shrunk: bool,
    settled: bool,
    deadline_ms: Option<u64>,
}

impl DockState {
    /// A dock in the given state, settled.
    pub fn new(shrunk: bool) -> Self {
        Self {
            shrunk,
            settled: shrunk,
            deadline_ms: None,
        }
    }

    /// Flips the request and starts the `SHRINK_MS` animation; returns the new `shrunk`.
    pub fn toggle(&mut self, now_ms: u64) -> bool {
        self.shrunk = !self.shrunk;
        self.deadline_ms = Some(now_ms.saturating_add(u64::from(SHRINK_MS)));
        self.shrunk
    }

    /// Sets the request; a no-op when it is equal.
    pub fn set(&mut self, shrunk: bool, now_ms: u64) {
        if shrunk != self.shrunk {
            self.toggle(now_ms);
        }
    }

    /// `Some(new band height)` exactly once, when the animation has ended and the settled band
    /// really changed: the host resizes the web view then. A request and its reversal before the
    /// deadline settle on the original band, so the web view is not resized at all. A clock that
    /// goes backwards only delays the end.
    pub fn tick(&mut self, now_ms: u64) -> Option<f32> {
        let deadline = self.deadline_ms?;
        if now_ms < deadline {
            return None;
        }
        self.deadline_ms = None;
        if self.settled == self.shrunk {
            return None;
        }
        self.settled = self.shrunk;
        Some(reserved_band_height(self.settled))
    }

    /// What the user asked for; the dock draws this at once.
    pub fn shrunk(&self) -> bool {
        self.shrunk
    }

    /// The band for the web view: the SETTLED state's (the old one while animating).
    pub fn reserved_band(&self) -> f32 {
        reserved_band_height(self.settled)
    }

    /// Whether the animation is running.
    pub fn animating(&self) -> bool {
        self.deadline_ms.is_some()
    }
}
