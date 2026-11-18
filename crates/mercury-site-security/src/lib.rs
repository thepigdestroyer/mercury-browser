// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

mod error_pages;
mod exception_request;
mod idn;
mod site;

pub use error_pages::{
    CERTIFICATE_EXCEPTION_SCHEME, ERROR_PAGE_PROBE_SCRIPT, ErrorPage, certificate_error_page,
    crash_page, network_error_page, parse_error_page_marker,
};
pub use exception_request::{
    CertificateException, ExceptionRefusal, parse_certificate_exception_request,
};
pub use idn::{display_host, display_url};
pub use site::site_covers_host;
