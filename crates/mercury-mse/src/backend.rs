// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `MseBackend`: the worker thread, the event posting, the [`MediaSourceBackend`] and [`FrameFeed`]
//! implementations.
//!
//! OWNER: item 3 of lane med-1. The two entry points below are the contract with the glue (item 5);
//! item 3 adds the trait implementations.
//!
//! [`MediaSourceBackend`]: crate::api::MediaSourceBackend
//! [`FrameFeed`]: crate::api::FrameFeed

use std::sync::Arc;

use crate::api::{EventSink, Limits, TypeSupport};

/// The backend of one MediaSource.
pub struct MseBackend {
    _private: (),
}

impl MseBackend {
    /// Events go to `events` (any thread); `types` answers decoder questions; `limits` is the quota.
    pub fn new(events: EventSink, types: Arc<dyn TypeSupport>, limits: Limits) -> MseBackend {
        log::debug!("mercury-mse: backend created");
        let _ = (events, types, limits);
        todo!("item 3: the backend")
    }

    /// `MediaSource.isTypeSupported` (`mime::is_type_supported`).
    pub fn is_type_supported(mime: &str, types: &dyn TypeSupport) -> bool {
        crate::mime::is_type_supported(mime, types)
    }
}
