// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `HubRing`: owns the ring's state (rows, query, focus, centre, held), builds the
//! [`RingBandsData`](crate::ring_bands::RingBandsData), fills the `HubRingBridge` global of any
//! window that has it and calls the browser through [`HubRingHost`].
//!
//! STUB in the scaffold commit (the signatures are the contract; item 4 `popover` writes the
//! bodies).

use std::rc::Rc;

use slint::{ComponentHandle, Global};

use super::{HubRingHost, HubRingInput};
use crate::HubRingBridge;
use crate::ring_geometry::GlyphMetrics;

/// The controller of one hub ring popover; keep it alive as long as the window.
pub struct HubRing {
    _host: Rc<dyn HubRingHost>,
}

impl HubRing {
    /// Sets the persistent models of the window's `HubRingBridge` once and every handler on it.
    /// `metrics` measures the ring's labels (`ring_bands::metrics()`; keep one for the window).
    pub fn install<C>(
        component: &C,
        host: Rc<dyn HubRingHost>,
        _metrics: Box<dyn GlyphMetrics>,
    ) -> Self
    where
        C: ComponentHandle + 'static,
        for<'a> HubRingBridge<'a>: Global<'a, C>,
    {
        let _ = component.global::<HubRingBridge>();
        Self { _host: host }
    }

    /// Rebuilds everything from new rows, theme or size: runs the search again and keeps the query
    /// and the focus while the tab still exists.
    pub fn set_input(&self, _input: HubRingInput) {}

    /// The popover opened: the focus is the active tab, the query is empty, the mode is rest.
    pub fn opened(&self) {}

    /// The key look from outside (the window-level ⌥ hook, D5).
    pub fn set_held(&self, _held: bool) {}
}
