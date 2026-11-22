// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Services (MM09): "Open in Mercury Browser" and "Search with Mercury Browser" on selected text
//! in other apps. The rules for the text are here; the provider methods are in `services_macos.rs`.

use talaria_core::{BrowserController, TabId};

/// `NSMessage` of the two entries of `NSServices` in `Info.plist.in`.
pub(crate) const OPEN_URL_MESSAGE: &str = "mercuryOpenURLService";
pub(crate) const SEARCH_MESSAGE: &str = "mercurySearchService";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServiceKind {
    OpenUrl,
    Search,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ServiceRequest {
    /// An absolute `http` or `https` URL.
    Open(String),
    Search(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServiceRefusal {
    Empty,
    NotAnAddress,
    TooLong,
}

pub(crate) fn parse_service_text(
    kind: ServiceKind,
    text: &str,
) -> Result<ServiceRequest, ServiceRefusal> {
    let _ = (kind, text);
    Err(ServiceRefusal::Empty)
}

/// Opens the request in a new foreground tab.
pub(crate) fn open_service_request(
    controller: &mut BrowserController,
    request: &ServiceRequest,
) -> Option<TabId> {
    let _ = (controller, request);
    None
}
