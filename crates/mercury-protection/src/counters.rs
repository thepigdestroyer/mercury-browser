// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Per-tab counters and the list of what was blocked on the tab's page (MH14).

/// A tab, as the caller names it (any stable number).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabKey(pub u64);

/// What kind of resource a blocked request was for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    /// A page or frame.
    Document,
    /// A script.
    Script,
    /// An image.
    Image,
    /// A style sheet.
    Stylesheet,
    /// A font.
    Font,
    /// Audio or video.
    Media,
    /// `fetch` or `XMLHttpRequest`.
    Xhr,
    /// A WebSocket.
    WebSocket,
    /// Anything else.
    Other,
}
