/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#![deny(unsafe_code)]

pub mod async_runtime;
// Mercury: certificate exceptions bound to (host, certificate); see certificate_exceptions.rs.
pub mod certificate_exceptions;
pub mod connector;
pub mod cookie;
pub mod cookie_storage;
mod decoder;
mod devtools;
mod disk_cache;
pub mod embedder;
pub mod filemanager_thread;
mod hosts;
pub mod hsts;
pub mod http_cache;
pub mod http_loader;
pub mod image_cache;
pub mod local_directory_listing;
// Mercury: keeps cookie and credential values out of the logs (EC13), and refuses very deeply nested SVG
// images (D8). Both are Mercury files.
mod mercury_log;
mod mercury_svg;
pub mod protocols;
pub mod request_interceptor;
pub mod resource_thread;
pub mod subresource_integrity;
#[cfg(feature = "test-util")]
pub mod test_util;
mod websocket_loader;

// Mercury: the network policy handle and everything that asks it (net-1 lane). `net_policy` is the handle and
// the sites of a request (EC5, EC6, EC14, EC17, EC25, EC38); `mercury_cookies` its cookie rules (third-party
// blocking, SameSite); `mercury_rewrite` the request rewrite hook (EC37); `mercury_mixed_content` the
// mixed-content allow and report (EC17); `mercury_ws` WebSockets through the proxy (EC23a); `mercury_auth`
// the limits on sign-in prompts for `fetch` and XHR (EC29). All are Mercury files.
pub mod mercury_auth;
mod mercury_cookies;
mod mercury_mixed_content;
mod mercury_rewrite;
mod mercury_ws;
pub mod net_policy;

/// An implementation of the [Fetch specification](https://fetch.spec.whatwg.org/)
pub mod fetch {
    pub mod cors_cache;
    pub mod fetch_params;
    pub mod headers;
    pub mod methods;
}

/// A module for re-exports of items used in unit tests.
pub mod test {
    pub use crate::decoder::{BodyStreamError, DECODER_BUFFER_SIZE, map_decode_error};
    pub use crate::hosts::parse_hostsfile;
    pub use crate::http_loader::HttpState;
}
