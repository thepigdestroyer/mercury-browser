// SPDX-License-Identifier: MPL-2.0
mod adblocker;
mod clipboard;
mod content_process;
mod document_starts;
mod download_offers;
mod downloads;
mod embedder_controls;
// engine prefs: the preferences Mercury sets for the engine, in one file.
mod engine_prefs;
mod error;
mod error_pages;
mod exit_clearing;
// media-capture
#[cfg(target_os = "macos")]
mod fake_media_devices;
mod find_in_page;
mod frame_ready;
mod geolocation;
mod history_events;
mod http_disk_cache;
mod load_metrics;
mod media_session;
// engine messages: routes the engine's Mercury message variants to the lanes that own them.
mod mercury_messages;
// net: the engine's network policy from the privacy settings, and the net lane's engine messages.
mod net_policy;
mod opengl_context;
// openurl
mod openurl;
mod page_defaults;
mod page_feedback;
mod page_requests;
mod page_tabs;
mod panics;
mod proxy;
mod runtime;
mod screen;
mod session_data;
mod site_data;
mod web_view;
// site security
mod certificate_exceptions;
mod resources;
mod site_information;
mod site_security;

pub use resources::record_error_page_secret;
pub use site_information::SiteDataSummary;

pub use adblocker::FilterEngineSlot;
pub use clipboard::{ActivationUse, UserActivationCheck};
pub use content_process::{
    ContentProcessArgumentError, record_browser_process_id, run_content_process_if_requested,
};
pub use error::ServoWebViewError;
pub use error_pages::error_page_titles;
pub use exit_clearing::{EngineSwitches, ExitClearing};
// media-capture
#[cfg(target_os = "macos")]
pub use fake_media_devices::{
    FAKE_MEDIA_DEVICES_VARIABLE, executable_lib_dir, fake_media_devices_problem,
};
pub use geolocation::{
    FakeLocation, FixedPosition, GeolocationDiagnostic, GeolocationSetup, LocationError,
};
pub use http_disk_cache::remove_http_disk_cache;
// net
pub use net_policy::{NetPolicyControl, NetPolicySetup};
pub use opengl_context::ViewportSize;
pub use page_requests::page_request_clock;
pub use page_tabs::{
    AdoptedWebView, DocumentRequestListener, DocumentStartedListener, NewTabAllocator,
};
pub use panics::{hard_fail_in_single_process, should_panic_hook_suppress_termination};
pub use proxy::{ProxyPreferences, ProxyServer, SystemProxySettings};
pub use runtime::{EventLoopWaker, NEW_TAB_URL, RuntimeOptions, ServoRuntime};
pub use screen::ScreenMetrics;
pub use site_data::SiteDataKinds;
pub use web_view::ServoWebView;
