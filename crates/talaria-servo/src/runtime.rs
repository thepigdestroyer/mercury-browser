// SPDX-License-Identifier: MPL-2.0
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::Instant;

use euclid::Scale;
use log::info;
use rustls::crypto::CryptoProvider;
use servo::{
    Cursor, LoadStatus, Opts, PixelFormat, PrefValue, Preferences, RenderingContext, ServoBuilder,
    WebViewBuilder,
};
use talaria_core::{BrowserEvent, LoadState, PageIcon, PointerCursor, TabEvent, TabId};
use talaria_core::{PageRequest, PageRequestKind};
use url::Url;

use crate::accessibility::AccessibilitySink; // a11y-1
use crate::adblocker::ContentBlocker;
use crate::clipboard::{ClipboardRead, TabClipboard, UserActivationCheck};
use crate::document_starts::DocumentStarts;
use crate::embedder_controls::{ControlIds, EmbedderControls};
use crate::exit_clearing::{EngineSwitches, ExitClearing, clear_at_exit};
use crate::frame_ready::FrameReadySignal;
use crate::geolocation::{
    GeolocationService, GeolocationSetup, PageGeolocation, WithGeolocation, on_history_changed,
    on_load_status,
};
use crate::http_disk_cache::{CacheCheck, http_disk_cache_database, repair_http_disk_cache};
use crate::load_metrics::{LoadMilestone, PageLoadMetrics};
// media session
use crate::media_session::{EngineMediaSession, browser_event_from_servo, describe_for_log};
use crate::page_defaults::page_defaults_user_content;
use crate::page_feedback::{DOCUMENT_TITLE_SCRIPT, PageFeedback, reads_title_at};
use crate::page_requests::{
    ClosesOnDrop, PendingEngineRequests, ServoPageRequest, page_permission, simple_dialog_kind,
};
use crate::page_tabs::{
    AdoptedWebView, DocumentRequestListener, DocumentStartedListener, NewTabAllocator, PageTabs,
    SharedDocumentRequestListener,
};
use crate::screen::ScreenMetrics;
use crate::session_data::{
    AfterEngineCleanup, HandleThenCleanup, clean_up_before_engine_start, end_session_cookies,
};
use crate::web_view::TabParts;
use crate::{ServoWebView, ServoWebViewError};
use mercury_proxy::{ProxyPreferences, SystemProxySettings};
// site security
use crate::certificate_exceptions::{AcceptedHosts, mercury_protocols};
use crate::site_information::{self, SiteDataSummary};
use crate::site_security::{TabHttpsUpgrades, probe_error_page};
use talaria_core::HttpsPolicy;
// sh-pages: view source and images per site
use crate::site_images::SiteImagesHandle;
use crate::view_source::ViewSourceConfig;
use mercury_view_source::ProxyConfig;

type WakeCallback = dyn Fn() + Send + Sync + 'static;

pub const NEW_TAB_URL: &str = "about:blank";

#[derive(Clone)]
pub struct EventLoopWaker {
    callback: Arc<WakeCallback>,
    wake_pending: Arc<AtomicBool>,
}

impl EventLoopWaker {
    pub fn new(callback: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            callback: Arc::new(callback),
            wake_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn acknowledge(&self) {
        self.wake_pending.store(false, Ordering::Release);
    }
}

impl servo::EventLoopWaker for EventLoopWaker {
    fn clone_box(&self) -> Box<dyn servo::EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        if self
            .wake_pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            (self.callback)();
        }
    }
}

/// Engine settings chosen at launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeOptions {
    /// Draw 2D canvases on the GPU (Servo's vello backend) instead of the CPU.
    pub gpu_canvas: bool,
    /// Where Servo keeps cookies, HSTS, HTTP authentication and site storage. `None` keeps
    /// nothing across runs.
    pub config_dir: Option<PathBuf>,
    /// Turn on servoshell's experimental web-platform features, except the async clipboard,
    /// WebGPU and container queries (see `EXCLUDED_EXPERIMENTAL_PREFS`). The web-compatibility
    /// preferences of `engine_prefs.rs` do not depend on it.
    pub experimental_web_features: bool,
    // settings
    /// Block ads and trackers from the start; changed later through `ServoRuntime::switches`.
    pub block_ads_and_trackers: bool,
    /// What to clear at quit; also cleared before the engine starts, in case the last quit
    /// never got to it. Changed later through `ServoRuntime::switches`.
    pub clear_on_exit: ExitClearing,
    /// The system's proxy settings, read at start; used unless the proxy environment
    /// variables are set. Servo builds its HTTP clients once, so a change applies at the
    /// next start.
    pub system_proxy: Option<SystemProxySettings>,
    /// Where Servo keeps its HTTP disk cache. `None` keeps the cache in memory only.
    pub http_disk_cache_dir: Option<PathBuf>,
    /// Whether pages get `navigator.geolocation`, and where positions come from.
    pub geolocation: GeolocationSetup,
    // sh-pages
    /// Run pages in the browser's own process instead of content processes. For tests that start
    /// the engine inside a test binary, which cannot be a content process (the engine tests of
    /// `tests/view_source_engine.rs`); the application never sets it.
    pub single_process: bool,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            gpu_canvas: true,
            config_dir: None,
            experimental_web_features: true,
            block_ads_and_trackers: true,
            clear_on_exit: ExitClearing::default(),
            system_proxy: None,
            http_disk_cache_dir: None,
            geolocation: GeolocationSetup::Off,
            single_process: false, // sh-pages
        }
    }
}

pub struct ServoRuntime {
    inner: servo::Servo,
    redraw_waker: EventLoopWaker,
    screen_metrics: Rc<Cell<Option<ScreenMetrics>>>,
    /// The user-origin style sheet every web view shares. Servo's `UserContentManager` holds a
    /// handle to Servo of its own, so this field must stay before `after_engine_cleanup`.
    user_content: Rc<servo::UserContentManager>,
    /// The `User-Agent` pages are loaded with; downloads send the same. (downloads)
    user_agent: String,
    /// Tabs opened by pages; it holds a handle to Servo too, so it stays before
    /// `after_engine_cleanup`.
    page_tabs: Rc<PageTabs>,
    /// Declared after `inner` and `user_content`, the two fields that hold a handle to Servo,
    /// so it is dropped after the last of them. Every web view holds a clone too, released
    /// after its own handle; see `AfterEngineCleanup`.
    after_engine_cleanup: Option<Rc<AfterEngineCleanup>>,
    /// Content blocking and what to clear at quit, shared with the browser.
    switches: EngineSwitches,
    // site security
    /// Hosts whose invalid certificate the user accepted, until the browser takes them.
    accepted_certificates: AcceptedHosts,
    // sh-pages
    /// The sites whose images are refused, shared with every tab's delegate.
    site_images: SiteImagesHandle,
    /// HTTPS-only mode as the `view-source:` fetch reads it (a copy of the tabs' policy).
    view_source_https: crate::view_source::SharedHttpsPolicy,
}

impl ServoRuntime {
    pub fn new(event_loop_waker: EventLoopWaker, runtime_options: RuntimeOptions) -> Self {
        ensure_crypto_provider();

        let redraw_waker = event_loop_waker.clone();
        let switches = EngineSwitches::new(
            runtime_options.block_ads_and_trackers,
            runtime_options.clear_on_exit,
        );
        // Servo reads its saved files when it starts. Session cookies and HTTP credentials in
        // them belong to an earlier session that was not ended (killed, or cut short by the
        // shutdown watchdog); remove them from the files before Servo can load them.
        if let Some(config_dir) = &runtime_options.config_dir {
            clean_up_before_engine_start(
                config_dir,
                runtime_options.http_disk_cache_dir.as_deref(),
                runtime_options.clear_on_exit,
            );
        }
        // Servo panics at start on some damaged caches; nothing has the file open yet.
        let cache_check = runtime_options
            .http_disk_cache_dir
            .as_deref()
            .map(repair_http_disk_cache);
        let mut options = mercury_options(runtime_options.config_dir.clone());
        options.multiprocess &= !runtime_options.single_process; // sh-pages
        let multiprocess_enabled = options.multiprocess;
        let sandbox_enabled = options.sandbox;
        let preferences = mercury_preferences(&runtime_options);
        let user_agent = preferences.user_agent.clone(); // downloads
        // site security: certificate exceptions from Mercury's certificate page
        let accepted_certificates = AcceptedHosts::default();
        // sh-pages: `view-source:`; its fetch goes through the proxy the engine uses
        let view_source_https = crate::view_source::SharedHttpsPolicy::default();
        let mut registry = mercury_protocols(&accepted_certificates);
        crate::view_source::register(
            &mut registry,
            ViewSourceConfig {
                proxy: ProxyConfig {
                    http: preferences.network_http_proxy_uri.clone(),
                    https: preferences.network_https_proxy_uri.clone(),
                    no_proxy: preferences.network_http_no_proxy.clone(),
                },
                https_policy: Arc::clone(&view_source_https),
            },
        );
        let inner = ServoBuilder::default()
            .opts(options)
            .preferences(preferences)
            .event_loop_waker(Box::new(event_loop_waker))
            .protocol_registry(registry)
            .build();
        inner.setup_logging();
        log_proxy_settings(runtime_options.system_proxy.as_ref());
        if let Some(CacheCheck::Removed(reason)) = cache_check {
            log::warn!("removed the HTTP disk cache, which the engine could not use: {reason}");
        }
        let after_engine_cleanup = runtime_options.config_dir.clone().map(|config_dir| {
            // Session cookies saved by a run that did not end them (it was killed) end now.
            end_session_cookies(&inner);
            Rc::new(AfterEngineCleanup::new(
                config_dir,
                runtime_options.http_disk_cache_dir.clone(),
                switches.clone(),
            ))
        });
        if multiprocess_enabled {
            info!(
                "Servo content process isolation enabled; operating-system sandbox {}",
                if sandbox_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
        } else {
            info!("Servo single-process mode enabled; operating-system sandbox disabled");
        }
        let content_blocker = Rc::new(ContentBlocker::new(switches.clone()));
        inner.set_delegate(Rc::new(BrowserServoDelegate {
            content_blocker: Rc::clone(&content_blocker),
        }));
        let user_content = page_defaults_user_content(&inner);
        let screen_metrics = Rc::default();
        // tabs opened by pages
        let page_tabs = Rc::new(PageTabs {
            adopted: RefCell::default(),
            allocator: RefCell::default(),
            document_started_listener: Rc::default(),
            // media session
            engine_media_session: EngineMediaSession::default(),
            document_request_listener: Rc::default(), // downloads
            control_ids: ControlIds::default(),
            content_blocker,
            screen_metrics: Rc::clone(&screen_metrics),
            user_content: Rc::clone(&user_content),
            after_engine_cleanup: after_engine_cleanup.clone(),
            clipboard: Rc::default(), // clipboard
            geolocation: GeolocationService::for_setup(&runtime_options.geolocation),
            https_policy: Rc::default(), // site security
        });

        Self {
            inner,
            redraw_waker,
            screen_metrics,
            user_content,
            user_agent,
            page_tabs,
            after_engine_cleanup,
            switches,
            accepted_certificates,
            site_images: SiteImagesHandle::default(), // sh-pages
            view_source_https,                        // sh-pages
        }
    }

    // site security

    /// HTTPS-only mode and its exceptions, as the browser's controller has them. Applies to
    /// loads that start from now on, in every tab.
    pub fn set_https_policy(&self, policy: &HttpsPolicy) {
        if *self.page_tabs.https_policy.borrow() != *policy {
            self.page_tabs.https_policy.replace(policy.clone());
        }
        // sh-pages: the `view-source:` fetch applies the same mode to the addresses it requests.
        *self
            .view_source_https
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = policy.clone();
    }

    /// The hosts whose invalid certificate the user accepted since the last call (ASCII).
    pub fn take_certificate_exceptions(&self) -> Vec<String> {
        self.accepted_certificates.take()
    }

    /// The cookies and web storage the engine keeps for the site of `url`.
    pub fn site_data_summary(&self, url: &Url) -> SiteDataSummary {
        site_information::site_data_summary(&self.inner, url)
    }

    /// Clears the cookies and web storage of the site of `url`; returns how many sites.
    pub fn clear_data_for_site(&self, url: &Url) -> usize {
        site_information::clear_data_for_site(&self.inner, url)
    }

    // settings

    /// The switches the browser changes while the engine runs; they share their state with
    /// the engine, so keep the clone and set it directly.
    pub fn switches(&self) -> EngineSwitches {
        self.switches.clone()
    }

    // sh-pages
    /// The sites whose images are refused; the handle shares its state with every tab's
    /// delegate, so keep the clone and replace the set through it.
    pub fn site_images(&self) -> SiteImagesHandle {
        self.site_images.clone()
    }

    // blocklists
    /// Where compiled filter-list engines are handed to the content blocker.
    pub fn filter_engine_slot(&self) -> crate::FilterEngineSlot {
        self.page_tabs.content_blocker.engine_slot()
    }

    /// What pages see as the screen and window geometry; `None` lets Servo use its defaults.
    pub fn set_screen_metrics(&self, metrics: Option<ScreenMetrics>) {
        self.screen_metrics.set(metrics);
    }

    pub fn create_web_view(
        &self,
        rendering_context: Rc<dyn RenderingContext>,
        initial_url: &str,
        scale_factor: f32,
        tab_id: TabId,
        browser_event_sender: Sender<TabEvent>,
    ) -> Result<ServoWebView, ServoWebViewError> {
        let initial_url = Url::parse(initial_url)?;
        let (delegate, parts) = new_tab_delegate(
            &self.page_tabs,
            tab_id,
            &browser_event_sender,
            &self.site_images, // sh-pages
        );

        let inner = WebViewBuilder::new(&self.inner, rendering_context)
            .url(initial_url)
            .hidpi_scale_factor(Scale::new(scale_factor))
            .clipboard_delegate(Rc::clone(&delegate) as Rc<dyn servo::ClipboardDelegate>) // clipboard
            .delegate(delegate)
            .user_content_manager(Rc::clone(&self.user_content))
            .with_geolocation(parts.geolocation.as_ref())
            .build();

        for event in new_web_view_feedback(inner.cursor(), inner.status_text()) {
            let _send_result = browser_event_sender.send(TabEvent::new(tab_id, event));
        }
        info!("created Servo WebView for tab {}", tab_id.value());
        Ok(ServoWebView::new(
            HandleThenCleanup::new(inner, self.after_engine_cleanup.clone()),
            tab_id,
            scale_factor,
            parts,
        ))
    }

    /// The engine, for the crate's helpers (history lane: `site_data`).
    pub(crate) fn servo(&self) -> &servo::Servo {
        &self.inner
    }

    pub fn spin_event_loop(&self) {
        self.redraw_waker.acknowledge();
        self.inner.spin_event_loop();
    }

    // tabs opened by pages
    /// Lets pages open tabs (`window.open`, `target=_blank`). Without an allocator, or when it
    /// returns None, such requests are refused.
    pub fn set_new_tab_allocator(&self, allocator: NewTabAllocator) {
        self.page_tabs.allocator.replace(Some(allocator));
    }

    /// Called synchronously, inside `spin_event_loop`, when a tab starts loading a new document
    /// (Servo's `LoadStatus::Started`, which comes before any of the document's scripts run).
    pub fn set_document_started_listener(&self, listener: DocumentStartedListener) {
        self.page_tabs
            .document_started_listener
            .replace(Some(listener));
    }

    // a11y-1
    /// Where every tab's accessibility tree updates go: called synchronously, inside
    /// `spin_event_loop`, in the order the engine sent them, with the tab and the update (which
    /// is the page's text: never print it). A tab sends none until it is told to build its tree
    /// (`ServoWebView::set_page_accessible`).
    pub fn set_accessibility_sink(&self, sink: AccessibilitySink) {
        crate::accessibility::set_sink(sink);
    }

    // a11y-1
    /// Sends an action request of assistive technology to the page it names (`target_tree` is
    /// the tree id the engine gave: the web view's, the document's or an iframe's). A request
    /// for a tree that is gone is ignored by the engine.
    pub fn forward_accessibility_action(&self, request: servo::accesskit::ActionRequest) {
        self.inner.forward_accessibility_action(request);
    }

    // media session
    /// The tab whose page Servo sends media-session actions to (one for all web views).
    pub fn engine_media_session_tab(&self) -> Option<TabId> {
        self.page_tabs.engine_media_session.get()
    }

    /// Web views created by pages since the last call, in creation order, each with the channel
    /// its tab's events arrive on.
    pub fn take_adopted_web_views(&self) -> Vec<AdoptedWebView> {
        self.page_tabs.adopted.take()
    }

    // downloads
    /// Called synchronously, inside `spin_event_loop`, as each top-level request of a tab starts
    /// (`redirect` for the next hop of a navigation); answers whether the user started it.
    pub fn set_document_request_listener(&self, listener: DocumentRequestListener) {
        self.page_tabs
            .document_request_listener
            .replace(Some(listener));
    }

    /// The `User-Agent` the engine sends.
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// The cookies the engine would send with an HTTP request to `url`, as name and value
    /// pairs (`HttpOnly` ones included), given to `answer` once the network thread replies
    /// (during a later `spin_event_loop`). Does not block.
    pub fn cookies_for_url(&self, url: &Url, answer: impl FnOnce(Vec<(String, String)>) + 'static) {
        crate::downloads::cookies_for_url(&self.inner, url, answer);
    }

    // clipboard
    /// The clipboard pages read and write; a `MemoryPasteboard` until this is called. Automated
    /// runs and tests must never install the system clipboard.
    pub fn set_pasteboard(&self, pasteboard: Rc<dyn talaria_core::Pasteboard>) {
        self.page_tabs.clipboard.set_pasteboard(pasteboard);
    }

    /// Tells whether a tab has a recent user activation; without it no page may write to the
    /// clipboard or ask to read it, except for the Cut, Copy and Paste the user chooses.
    pub fn set_user_activation_check(&self, check: UserActivationCheck) {
        self.page_tabs.clipboard.set_user_activation_check(check);
    }
}

impl Drop for ServoRuntime {
    /// Clears what the user asked to have cleared at exit, then ends the session cookies,
    /// while Servo still runs, before it saves its files.
    fn drop(&mut self) {
        // Web views opened by pages and never taken close while Servo still runs.
        drop(self.page_tabs.adopted.take());
        let clearing = self.switches.clear_on_exit();
        if clearing.clears_anything() {
            clear_at_exit(&self.inner, clearing);
        }
        if self.after_engine_cleanup.is_some() {
            end_session_cookies(&self.inner);
        }
    }
}

fn mercury_options(config_dir: Option<PathBuf>) -> Opts {
    Opts {
        // Servo 0.8.0 content-process isolation renders correctly in Mercury.
        // Keep the operating-system sandbox disabled independently: the
        // current Linux gaol path is not yet reliable enough for a release.
        multiprocess: true,
        sandbox: false,
        // With `hard_fail`, a page that panics would exit the whole browser; without it,
        // Servo replaces the page with its crash page and reports it through `notify_crashed`.
        hard_fail: false,
        config_dir,
        ..Opts::default()
    }
}

struct BrowserServoDelegate {
    content_blocker: Rc<ContentBlocker>,
}

impl servo::ServoDelegate for BrowserServoDelegate {
    fn load_web_resource(&self, load: servo::WebResourceLoad) {
        // sh-pages: a load without a web view never loads a `view-source:` address.
        let Some(load) = crate::view_source::refuse_view_source_without_web_view(load) else {
            return;
        };
        // openurl: a load without a web view (a worker's, say) never reads local files either.
        let Some(load) = crate::openurl::refuse_local_file_load(None, None, load) else {
            return;
        };
        self.content_blocker.handle_web_resource_load(None, load);
    }

    // engine messages: Mercury's lane messages that name no web view, or whose web view has gone.
    fn notify_mercury_message(&self, message: servo::MercuryEmbedderMsg) {
        crate::mercury_messages::handle(None, message, &|_event| {});
    }
}

fn mercury_preferences(runtime_options: &RuntimeOptions) -> Preferences {
    let mut preferences = Preferences::default();
    if runtime_options.gpu_canvas {
        preferences.dom_canvas_backend = "vello".to_owned();
    }
    if runtime_options.experimental_web_features {
        for name in EXPERIMENTAL_PREFS
            .iter()
            .filter(|name| !EXCLUDED_EXPERIMENTAL_PREFS.contains(name))
        {
            preferences.set_value(name, PrefValue::Bool(true));
        }
    }
    // clipboard: off, as `talaria_core::ASYNC_CLIPBOARD_OFFERED` says (its doc comment is the
    // turn-on checklist), also with the experimental features. Servo 0.8.0 checks neither
    // permission nor user activation for `navigator.clipboard` (`dom/clipboard/clipboard.rs`, the
    // "check clipboard read permission" step is a TODO) and its requests name no frame, so any
    // frame of a page, an advert too, could write the clipboard after any click of the user's,
    // and its `readText()` could race the user's Paste for what is being pasted. Every web view's
    // `ClipboardDelegate` (`crate::clipboard`) gates the requests that do reach it, which with
    // the API off are the user's own Cut, Copy and Paste.
    preferences.dom_async_clipboard_enabled = talaria_core::ASYNC_CLIPBOARD_OFFERED;
    // Always off while Mercury does not build Servo's `webgpu` cargo feature: the preference alone
    // exposes `GPUCanvasContext` and other WebGPU names over unimplemented stubs, which makes
    // feature detection report WebGPU that is not there.
    preferences.dom_webgpu_enabled = false;
    // media-capture: camera and microphone (`navigator.mediaDevices`), on macOS only for now; on
    // other platforms the preference stays off and `navigator.mediaDevices` does not exist.
    // Mercury's vendored servo-script asks the embedder before opening a device and releases
    // devices when the page goes away (`third_party/servo-script/MERCURY.md`); in Servo 0.8.0
    // as published `getUserMedia` asked nobody. The usage descriptions in `Info.plist.in` and
    // the device entitlements must stay with this line: macOS ends a process that opens a
    // device without them.
    preferences.dom_webrtc_enabled = cfg!(target_os = "macos");
    // media-capture: always off. The vendored bindings gate `RTCPeerConnection` and the other
    // WebRTC interfaces by this preference (not by `dom_webrtc_enabled`); Servo 0.8's WebRTC
    // gives every local IP address to the page and cannot call modern sites (R8 section 4).
    preferences.dom_webrtc_transceiver_enabled = false;
    // Off unless geolocation is set up: positions then come from Mercury's own source, and
    // Servo asks for the "geolocation" permission before every first request of a page.
    preferences.dom_geolocation_enabled = runtime_options.geolocation.exposes_geolocation();
    // controls: always off. With it on, a page can call `selectFiles()` on a file input and have
    // Servo accept the paths it names without any file panel (`htmlinputelement.rs`); it exists
    // for Servo's own tests.
    preferences.dom_testing_html_input_element_select_files_enabled = false;
    // engine prefs: the product values and pins of the web-platform preferences, set for every
    // launch (not behind the experimental-features switch); see `engine_prefs.rs`.
    crate::engine_prefs::apply(&mut preferences);
    preferences.user_agent = firefox_compatible_user_agent(&preferences.user_agent);
    apply_system_proxy(&mut preferences, runtime_options.system_proxy.as_ref());
    // Servo's size limit (`network_http_disk_cache_size`, 100 MB) is kept.
    if let Some(database) = runtime_options
        .http_disk_cache_dir
        .as_deref()
        .map(http_disk_cache_database)
        .and_then(|database| database.to_str().map(str::to_owned))
    {
        preferences.network_http_disk_cache = database;
    }
    preferences
}

/// Servo's `Preferences::default` has already read the `http_proxy`, `https_proxy` and
/// `no_proxy` environment variables (and their upper-case forms); if any was set, those win
/// and the system settings are not used, as in command-line tools. The choice is
/// `mercury_proxy::choose`, the same one the desktop app records with `mercury_proxy::install`
/// for the clients that are not Servo (downloads, the PDF viewer's fetch).
fn apply_system_proxy(preferences: &mut Preferences, system: Option<&SystemProxySettings>) {
    let environment = ProxyPreferences {
        http_uri: std::mem::take(&mut preferences.network_http_proxy_uri),
        https_uri: std::mem::take(&mut preferences.network_https_proxy_uri),
        no_proxy: std::mem::take(&mut preferences.network_http_no_proxy),
    };
    let proxy = mercury_proxy::choose(environment, system);
    preferences.network_http_proxy_uri = proxy.http_uri;
    preferences.network_https_proxy_uri = proxy.https_uri;
    preferences.network_http_no_proxy = proxy.no_proxy;
}

/// Once, at start: which proxies are used (in the log), and what of the system settings is not
/// (a warning that is always shown).
fn log_proxy_settings(system: Option<&SystemProxySettings>) {
    let preferences = servo::prefs::get();
    for (scheme, uri) in [
        ("http", &preferences.network_http_proxy_uri),
        ("https", &preferences.network_https_proxy_uri),
    ] {
        if !uri.is_empty() {
            // The address may carry credentials (from the environment); log the host only.
            let host = Url::parse(uri)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_else(|| "(unparsed)".to_owned());
            info!("{scheme} requests go through the proxy at {host}");
        }
    }
    // A warning on standard error, always shown: Servo's logger prints only errors unless
    // `RUST_LOG` asks for more, and someone whose network needs a PAC file must be able to see
    // why pages do not load.
    for message in system
        .map(SystemProxySettings::unsupported)
        .unwrap_or_default()
    {
        eprintln!("mercury-browser: warning: {message}");
    }
}

// From Servo's servoshell, https://github.com/servo/servo, tag `v0.8.0` (commit
// `4e136d8ef31b7c4748fbfac606ca463f3efd7537`), `ports/servoshell/prefs.rs` lines 29-53
// (`EXPERIMENTAL_PREFS`). Licensed under the Mozilla Public License 2.0.
/// Preferences that servoshell turns on with `--enable-experimental-web-platform-features`:
/// implemented in Servo but off by default. Mercury enables this list minus
/// `EXCLUDED_EXPERIMENTAL_PREFS`.
const EXPERIMENTAL_PREFS: &[&str] = &[
    "dom_async_clipboard_enabled",
    "dom_exec_command_enabled",
    "dom_fontface_enabled",
    "dom_indexeddb_enabled",
    "dom_intersection_observer_enabled",
    "dom_navigator_protocol_handlers_enabled",
    "dom_notification_enabled",
    "dom_offscreen_canvas_enabled",
    "dom_permissions_enabled",
    "dom_sanitizer_enabled",
    "dom_storage_manager_api_enabled",
    "dom_webgl2_enabled",
    "dom_webgpu_enabled",
    "layout_css_backdrop_filter_enabled",
    "layout_css_columns_enabled",
    "layout_css_container_queries_enabled",
    "layout_css_ellipse_corners_enabled",
    "layout_css_font_variations_enabled",
];

/// Entries of `EXPERIMENTAL_PREFS` that Mercury leaves off; the reasons are given where they
/// are pinned in `mercury_preferences`, except for container queries: Servo parses `@container`
/// blocks and drops them (`@container` is Gecko-only in Stylo), and
/// `CSS.supports('container-type', 'inline-size')` and `@supports` would report support that is
/// not there, so pages would take the container query branch and get none of its layout.
const EXCLUDED_EXPERIMENTAL_PREFS: &[&str] = &[
    "dom_async_clipboard_enabled",
    "dom_webgpu_enabled",
    "layout_css_container_queries_enabled",
];

/// Servo's user agent already names the Firefox release it matches, but carries a `Servo/x.y`
/// token where Firefox has `Gecko/20100101`; some sites refuse to serve unrecognised browsers.
/// Replaces that token, keeping the rest of Servo's string (platform and Firefox version).
fn firefox_compatible_user_agent(servo_user_agent: &str) -> String {
    servo_user_agent
        .split(' ')
        .map(|token| {
            if token.starts_with("Servo/") {
                "Gecko/20100101"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

struct BrowserDelegate {
    tab_id: TabId,
    browser_event_sender: Sender<TabEvent>,
    frame_ready: FrameReadySignal,
    load_metrics: RefCell<PageLoadMetrics>,
    content_blocker: Rc<ContentBlocker>,
    screen_metrics: Rc<Cell<Option<ScreenMetrics>>>,
    /// Weak: queued web views hold their delegates, which must not keep the queue alive.
    page_tabs: Weak<PageTabs>,
    /// Title and icon kept right across documents; see `PageFeedback`.
    page_feedback: Rc<PageFeedback>,
    // controls
    controls: EmbedderControls,
    /// Dialogs, leave prompts, sign-ins and permission prompts waiting for the user; shared with
    /// the tab's `ServoWebView`, which answers them.
    page_requests: Rc<RefCell<PendingEngineRequests<ServoPageRequest>>>,
    /// The tab's one document-start signal (dialogs, tabs); shared with the tab's `ServoWebView`,
    /// which arms it when it navigates.
    document_start: Rc<DocumentStarts>,
    // clipboard: shared with the tab's `ServoWebView`, which notes the user's commands.
    clipboard: TabClipboard,
    /// The tab's geolocation, also Servo's delegate for it; `None` while it is off.
    geolocation: Option<Rc<dyn PageGeolocation>>,
    // media session
    engine_media_session: EngineMediaSession,
    /// downloads: the method, URL and referrer of the last top-level request.
    last_document_request: RefCell<Option<(String, Url, Option<Url>)>>,
    /// downloads: shared by every tab's delegate.
    document_request_listener: SharedDocumentRequestListener,
    // site security
    https_upgrades: TabHttpsUpgrades,
    // sh-pages: shared by every tab's delegate
    site_images: SiteImagesHandle,
    // sh-pages: the page this tab's image loads belong to while it navigates
    navigating_page: crate::site_images::NavigatingPage,
}

/// A fresh delegate for a tab, used for web views the browser creates and for those pages open.
fn new_tab_delegate(
    shared: &Rc<PageTabs>,
    tab_id: TabId,
    browser_event_sender: &Sender<TabEvent>,
    site_images: &SiteImagesHandle, // sh-pages
) -> (Rc<BrowserDelegate>, TabParts) {
    let page_feedback = PageFeedback::new(tab_id, browser_event_sender.clone());
    // geolocation: one per tab, shared by Servo, the delegate, `DocumentStarts` and the web view
    let geolocation = shared
        .geolocation
        .as_ref()
        .map(GeolocationService::web_view);
    let page_geolocation = geolocation
        .clone()
        .map(|geolocation| geolocation as Rc<dyn PageGeolocation>);
    let parts = TabParts {
        frame_ready: FrameReadySignal::default(),
        controls: EmbedderControls::new(shared.control_ids.clone()),
        page_requests: ClosesOnDrop::new(PendingEngineRequests::default()),
        document_start: DocumentStarts::new(
            tab_id,
            Rc::clone(&page_feedback),
            Rc::clone(&shared.document_started_listener),
            page_geolocation.clone(),
        ),
        page_feedback,
        clipboard: TabClipboard::new(tab_id, Rc::clone(&shared.clipboard)),
        geolocation,
        // media session
        browser_event_sender: browser_event_sender.clone(),
    };
    let delegate = Rc::new(BrowserDelegate {
        tab_id,
        browser_event_sender: browser_event_sender.clone(),
        frame_ready: parts.frame_ready.clone(),
        load_metrics: RefCell::new(PageLoadMetrics::default()),
        content_blocker: Rc::clone(&shared.content_blocker),
        screen_metrics: Rc::clone(&shared.screen_metrics),
        page_tabs: Rc::downgrade(shared),
        page_feedback: Rc::clone(&parts.page_feedback),
        controls: parts.controls.clone(),
        page_requests: parts.page_requests.shared(),
        document_start: Rc::clone(&parts.document_start),
        clipboard: parts.clipboard.clone(),
        geolocation: page_geolocation,
        // media session
        engine_media_session: Rc::clone(&shared.engine_media_session),
        last_document_request: RefCell::new(None), // downloads
        document_request_listener: Rc::clone(&shared.document_request_listener), // downloads
        https_upgrades: TabHttpsUpgrades::new(Rc::clone(&shared.https_policy)),
        site_images: site_images.clone(), // sh-pages
        navigating_page: crate::site_images::NavigatingPage::default(), // sh-pages
    });
    (delegate, parts)
}

impl BrowserDelegate {
    // downloads
    fn note_document_request(
        &self,
        web_view: &servo::WebView,
        request: &servo::WebResourceRequest,
    ) {
        if request.is_for_main_frame {
            let url = request.url.clone().into_url();
            *self.last_document_request.borrow_mut() = Some((
                request.method.to_string(),
                url.clone(),
                request
                    .referrer_url
                    .clone()
                    .map(|referrer| referrer.into_url()),
            ));
            // Decided now, inside the engine's event loop as the request starts, so neither the
            // server's latency nor a new document starting in the same spin (which ends the
            // click's activation) can make the user's click look like the page's own doing.
            let listener = self.document_request_listener.borrow().clone();
            let user_started =
                listener.is_some_and(|listener| listener(self.tab_id, &url, request.is_redirect));
            self.send_browser_event(BrowserEvent::DocumentRequestStarted {
                url: url.to_string(),
                redirect: request.is_redirect,
                from: web_view.url().map(|from| from.to_string()),
                user_started,
            });
        }
    }

    // downloads
    /// A complete top-level page without a title may be Servo's "Unknown content type" page;
    /// if the page confirms it, the tab is told it was offered a download.
    fn check_for_download_offer(&self, web_view: &servo::WebView) {
        if web_view.page_title().is_some_and(|title| !title.is_empty()) {
            return;
        }
        let sender = self.browser_event_sender.clone();
        let tab_id = self.tab_id;
        let last_request = self.last_document_request.borrow().clone();
        web_view.evaluate_javascript(crate::download_offers::RECOGNISE_SCRIPT, move |answer| {
            let Ok(servo::JSValue::String(answer)) = answer else {
                return;
            };
            let Some((mime_type, url)) = crate::download_offers::parse_answer(&answer) else {
                return;
            };
            let method_and_url = last_request
                .as_ref()
                .map(|(method, requested, _)| (method.clone(), requested.clone()));
            let posted = crate::download_offers::was_posted(method_and_url.as_ref(), &url);
            let referrer = last_request
                .filter(|(_, requested, _)| *requested == url)
                .and_then(|(_, _, referrer)| referrer)
                .map(String::from);
            let _sent = sender.send(TabEvent::new(
                tab_id,
                BrowserEvent::DownloadOffered {
                    url: url.to_string(),
                    mime_type,
                    posted,
                    referrer,
                },
            ));
        });
    }

    fn send_browser_event(&self, event: BrowserEvent) {
        let _send_result = self
            .browser_event_sender
            .send(TabEvent::new(self.tab_id, event));
    }

    // tabs
    /// Reads `document.title` and sends it, unless the document or address changed meanwhile.
    fn read_document_title(&self, web_view: &servo::WebView) {
        self.page_feedback.request_title(|callback| {
            web_view.evaluate_javascript(DOCUMENT_TITLE_SCRIPT, callback);
        });
    }

    // controls
    fn drop_controls(&self) {
        for event in self.controls.drop_all() {
            self.send_browser_event(event);
        }
    }

    fn record_load_milestone(&self, milestone: LoadMilestone) {
        let elapsed = self
            .load_metrics
            .borrow_mut()
            .record(milestone, Instant::now());

        if let Some(elapsed) = elapsed {
            info!(
                "tab {}: page load milestone: {} at {:.1} ms",
                self.tab_id.value(),
                milestone.label(),
                elapsed.as_secs_f64() * 1_000.0
            );
        }
    }
}

impl servo::WebViewDelegate for BrowserDelegate {
    fn screen_geometry(&self, _web_view: servo::WebView) -> Option<servo::ScreenGeometry> {
        self.screen_metrics
            .get()
            .map(ScreenMetrics::to_servo_geometry)
    }

    fn load_web_resource(&self, web_view: servo::WebView, load: servo::WebResourceLoad) {
        // sh-pages: a `view-source:` address loads only as the tab's own page, never as a frame
        // or a resource.
        let Some(load) = crate::view_source::refuse_embedded_view_source(load) else {
            return;
        };
        // openurl: a web page's frames and resources never come from local files.
        let Some(load) =
            crate::openurl::refuse_local_file_load(Some(self.tab_id), web_view.url(), load)
        else {
            return;
        };
        // site security: HTTPS-only mode, before content blocking
        let Some(load) = self.https_upgrades.intercept(load, |event| {
            self.send_browser_event(event);
        }) else {
            return;
        };
        // sh-pages: images per site, after the HTTPS-only step and before content blocking. The
        // page is the main-frame document this tab loads last (`web_view.url()` lags behind a
        // cross-site navigation until the new document is active).
        self.navigating_page.note_load(
            load.request().is_for_main_frame,
            load.request().url.as_url(),
        );
        let page = self.navigating_page.page(web_view.url());
        let Some(load) = crate::site_images::refuse_blocked_image(&self.site_images, page, load)
        else {
            return;
        };
        self.note_document_request(&web_view, &load.request); // downloads
        self.content_blocker
            .handle_web_resource_load(Some(&web_view), load);
    }

    fn notify_url_changed(&self, web_view: servo::WebView, url: Url) {
        // sh-pages: the new document is active; `web_view.url()` is the page again.
        self.navigating_page.settle();
        // dialogs: user info is left out; after a sign-in it holds the password.
        log::debug!(
            "tab {}: address changed to {}",
            self.tab_id.value(),
            talaria_core::strip_user_info(url.as_str()),
        );
        self.send_browser_event(BrowserEvent::AddressChanged {
            url: display_url(&url),
        });
        // history
        self.page_feedback.set_address(display_url(&url));
        // tabs: a same-document navigation keeps the document's title and icon
        if self.page_feedback.address_changed(web_view.load_status()) {
            self.read_document_title(&web_view);
        }
    }

    fn notify_page_title_changed(&self, web_view: servo::WebView, title: Option<String>) {
        // A title read in flight is older than this notification.
        self.page_feedback.title_notified();
        self.send_browser_event(BrowserEvent::PageTitleChanged {
            title: title.clone(),
        });
        // history
        let url = web_view.url();
        self.send_browser_event(crate::history_events::page_title_at(url.as_ref(), title));
    }

    fn notify_favicon_changed(&self, web_view: servo::WebView) {
        let icon = web_view.favicon().and_then(|favicon| {
            page_icon_from_pixels(
                favicon.width,
                favicon.height,
                favicon.format,
                favicon.data(),
            )
        });
        // tabs: remembered for same-document navigations
        self.page_feedback.icon_changed(icon.clone());

        self.send_browser_event(BrowserEvent::PageIconChanged { icon });
    }

    fn notify_load_status_changed(&self, web_view: servo::WebView, status: LoadStatus) {
        // geolocation: a reported start ends the old document's requests and parks its watches
        on_load_status(self.geolocation.as_deref(), status);
        if status == LoadStatus::Complete {
            self.check_for_download_offer(&web_view); // downloads
        }
        // dialogs: a new document; the previous one's requests and page state no longer apply.
        // tabs: the same signal resets the title and icon and forgets the tab's activation,
        // synchronously, before any script of the new document runs.
        if self.document_start.load_status_changed(status) {
            self.send_browser_event(BrowserEvent::PageLoadStarted);
            self.clipboard.document_started(); // clipboard
        }
        match status {
            LoadStatus::Started => {
                // controls: requests of the page being left are dropped.
                self.drop_controls();
                self.load_metrics.borrow_mut().start(Instant::now());
                info!("tab {}: page load started", self.tab_id.value());
            }
            LoadStatus::HeadParsed => self.record_load_milestone(LoadMilestone::HeadParsed),
            LoadStatus::Complete => self.record_load_milestone(LoadMilestone::Complete),
            LoadStatus::Stopped => info!("tab {}: page load stopped", self.tab_id.value()),
        }

        self.send_browser_event(BrowserEvent::LoadStateChanged {
            state: load_state_from_servo(status),
        });
        // tabs: the icon again after a load, and the title, which Servo only notifies when it
        // differs from its cached one
        if status == LoadStatus::Complete {
            self.page_feedback.load_completed();
            // site security: a new HTTPS-only upgrade chain; one of Mercury's error pages?
            self.https_upgrades.load_finished();
            probe_error_page(&web_view, self.tab_id, self.browser_event_sender.clone());
        }
        if reads_title_at(status) {
            self.read_document_title(&web_view);
        }
    }

    fn notify_history_changed(&self, _web_view: servo::WebView, entries: Vec<Url>, current: usize) {
        // geolocation: back/forward may restore a cached document without a new start
        on_history_changed(self.geolocation.as_deref(), &entries, current);
        self.send_browser_event(history_changed_event(entries.len(), current));
        // history
        if let Some(event) = crate::history_events::page_committed(&entries, current) {
            self.send_browser_event(event);
        }
    }

    fn notify_cursor_changed(&self, _web_view: servo::WebView, cursor: Cursor) {
        self.send_browser_event(BrowserEvent::CursorChanged {
            cursor: pointer_cursor_from_servo(cursor),
        });
    }

    fn notify_status_text_changed(&self, _web_view: servo::WebView, status: Option<String>) {
        self.send_browser_event(BrowserEvent::StatusTextChanged { text: status });
    }

    fn notify_fullscreen_state_changed(&self, _web_view: servo::WebView, is_fullscreen: bool) {
        self.send_browser_event(BrowserEvent::FullscreenChanged {
            fullscreen: is_fullscreen,
        });
    }

    fn notify_new_frame_ready(&self, _web_view: servo::WebView) {
        self.record_load_milestone(LoadMilestone::FirstFrame);
        self.frame_ready.mark_ready();
    }

    fn notify_crashed(&self, web_view: servo::WebView, reason: String, _backtrace: Option<String>) {
        // controls
        self.drop_controls();
        for event in crash_feedback(reason, web_view.cursor(), web_view.status_text()) {
            self.send_browser_event(event);
        }
    }

    // openurl
    /// Navigations a page asks for (links, forms, `location`, in the main frame or a frame;
    /// not typed addresses, which never ask). Answered at once.
    fn request_navigation(&self, web_view: servo::WebView, request: servo::NavigationRequest) {
        if let Some(event) = crate::openurl::judge_navigation(self.tab_id, web_view.url(), request)
        {
            self.send_browser_event(event);
        }
    }

    // tabs opened and closed by pages
    /// Must be answered here: the constellation and the opener's script thread wait for it.
    fn request_create_new(&self, parent: servo::WebView, request: servo::CreateNewWebViewRequest) {
        let Some(page_tabs) = self.page_tabs.upgrade() else {
            return;
        };
        let Some(allocator) = page_tabs.allocator() else {
            info!(
                "tab {}: refused a page's new tab: no allocator",
                self.tab_id.value()
            );
            return;
        };
        let Some(tab_id) = allocator(self.tab_id) else {
            info!("tab {}: refused a page's new tab", self.tab_id.value());
            return;
        };

        let (browser_event_sender, event_receiver) = mpsc::channel::<TabEvent>();
        let (delegate, parts) = new_tab_delegate(
            &page_tabs,
            tab_id,
            &browser_event_sender,
            &self.site_images, // sh-pages: the opener's handle is the runtime's
        );
        let scale_factor = parent.hidpi_scale_factor();
        let inner = request
            .builder(parent.rendering_context())
            .hidpi_scale_factor(scale_factor)
            .clipboard_delegate(Rc::clone(&delegate) as Rc<dyn servo::ClipboardDelegate>) // clipboard
            .delegate(delegate)
            .user_content_manager(Rc::clone(&page_tabs.user_content))
            .with_geolocation(parts.geolocation.as_ref())
            .build();
        // Servo creates auxiliary web views visible, and all tabs share one rendering context:
        // the engine shows whichever tab becomes active.
        inner.hide();
        // geolocation: hidden here without `ServoWebView::hide`, so told directly
        if let Some(geolocation) = &parts.geolocation {
            geolocation.set_visible(false);
        }

        for event in new_web_view_feedback(inner.cursor(), inner.status_text()) {
            let _send_result = browser_event_sender.send(TabEvent::new(tab_id, event));
        }
        info!(
            "tab {}: page opened tab {}",
            self.tab_id.value(),
            tab_id.value()
        );
        let web_view = ServoWebView::new(
            HandleThenCleanup::new(inner, page_tabs.after_engine_cleanup.clone()),
            tab_id,
            scale_factor.get(),
            parts,
        );
        page_tabs
            .adopted
            .borrow_mut()
            .push((tab_id, web_view, event_receiver));
    }

    fn notify_closed(&self, _web_view: servo::WebView) {
        info!("tab {}: the page closed its tab", self.tab_id.value());
        self.send_browser_event(BrowserEvent::CloseRequested);
    }

    // controls, dialogs: simple dialogs are page requests; every other kind is a control.
    fn show_embedder_control(&self, web_view: servo::WebView, control: servo::EmbedderControl) {
        match control {
            servo::EmbedderControl::SimpleDialog(dialog) => {
                self.show_simple_dialog(&web_view, dialog);
            }
            control => {
                if let Some(event) = self.controls.show(control) {
                    // passwords: a context menu over the focused password field
                    let password_field_menu = self.controls.password_field_menu(&event);
                    self.send_browser_event(event);
                    if let Some(event) = password_field_menu {
                        self.send_browser_event(event);
                    }
                }
            }
        }
    }

    // controls: Servo never sends a hide for simple dialogs.
    fn hide_embedder_control(
        &self,
        _web_view: servo::WebView,
        control_id: servo::EmbedderControlId,
    ) {
        if let Some(event) = self.controls.hide(control_id) {
            self.send_browser_event(event);
        }
    }

    fn request_unload(&self, web_view: servo::WebView, request: servo::AllowOrDenyRequest) {
        self.forward_page_request(
            web_view.url(),
            PageRequestKind::LeavePage,
            ServoPageRequest::Unload(request),
        );
    }

    fn request_authentication(
        &self,
        _web_view: servo::WebView,
        request: servo::AuthenticationRequest,
    ) {
        let url = Some(request.url().clone());
        let kind = PageRequestKind::Authentication {
            for_proxy: request.for_proxy(),
        };
        self.forward_page_request(url, kind, ServoPageRequest::Authentication(request));
    }

    fn request_permission(&self, web_view: servo::WebView, request: servo::PermissionRequest) {
        // geolocation: a diagnostic run answers geolocation without a prompt
        let request = match &self.geolocation {
            Some(geolocation) => match geolocation.answer_permission_in_diagnostic(request) {
                Some(request) => request,
                None => return,
            },
            None => request,
        };
        // Servo does not say which frame asked; the request is shown for the tab's page.
        let kind = PageRequestKind::Permission(page_permission(request.feature()));
        self.forward_page_request(web_view.url(), kind, ServoPageRequest::Permission(request));
    }

    // media session
    fn notify_media_session_event(
        &self,
        _web_view: servo::WebView,
        event: servo::MediaSessionEvent,
    ) {
        // The constellation forwards an event only after making its sender the active session.
        self.engine_media_session.set(Some(self.tab_id));
        let event = browser_event_from_servo(event);
        // Metadata and positions are the page's; playback changes are the browser's own.
        if matches!(event, BrowserEvent::MediaPlaybackChanged { .. }) {
            info!("tab {}: media session: {event:?}", self.tab_id.value());
        } else {
            log::debug!(
                "tab {}: media session: {}",
                self.tab_id.value(),
                describe_for_log(&event)
            );
        }
        self.send_browser_event(event);
    }

    // engine messages: Mercury's lane messages for this tab, routed by `mercury_messages.rs`.
    fn notify_mercury_message(
        &self,
        _web_view: servo::WebView,
        message: servo::MercuryEmbedderMsg,
    ) {
        crate::mercury_messages::handle(Some(self.tab_id), message, &|event| {
            self.send_browser_event(event);
        });
    }

    // a11y-1: a tree update of this tab's page (the web view's tree, the document's or an iframe's).
    // It carries the page's text: handed on as it is, to the sink the window installed, and never
    // printed.
    fn notify_accessibility_tree_update(
        &self,
        _web_view: servo::WebView,
        tree_update: servo::accesskit::TreeUpdate,
    ) {
        crate::accessibility::deliver(self.tab_id, tree_update);
    }

    // notifications: the notification UI is a later lane's; until then the title is logged, at
    // debug level, and nothing of the page's other text (body, tag, icons) is.
    fn show_notification(&self, _web_view: servo::WebView, notification: servo::Notification) {
        log::debug!("page notification: {}", notification.title);
    }
}

// dialogs
impl BrowserDelegate {
    fn show_simple_dialog(&self, web_view: &servo::WebView, dialog: servo::SimpleDialog) {
        let kind = simple_dialog_kind(&dialog);
        self.forward_page_request(web_view.url(), kind, ServoPageRequest::Dialog(dialog));
    }

    /// Keeps the engine's request and tells the browser about it. A request that arrives while
    /// the web view is closing is dropped at once, which gives the engine's default answer.
    fn forward_page_request(
        &self,
        url: Option<Url>,
        kind: PageRequestKind,
        request: ServoPageRequest,
    ) {
        let Some(id) = self.page_requests.borrow_mut().insert(request) else {
            return;
        };
        log::debug!(
            "tab {}: page request {} ({kind:?}) for {}",
            self.tab_id.value(),
            id.value(),
            url.as_ref().map_or_else(
                || "no address".to_owned(),
                |url| talaria_core::strip_user_info(url.as_str())
            ),
        );
        self.send_browser_event(BrowserEvent::PageRequested(PageRequest {
            id,
            url: url.map(String::from).unwrap_or_default(),
            kind,
            received_at: crate::page_request_clock(),
        }));
    }
}

// clipboard: every request Servo sends for this tab's pages, from the user's Cut, Copy and Paste
// as well as from `navigator.clipboard`; see `crate::clipboard`.
impl servo::ClipboardDelegate for BrowserDelegate {
    fn clear(&self, _web_view: servo::WebView) {
        if !self.clipboard.clear() {
            log::debug!(
                "tab {}: refused a page's clipboard clear without a user gesture",
                self.tab_id.value()
            );
        }
    }

    fn get_text(&self, web_view: servo::WebView, request: servo::StringRequest) {
        match self.clipboard.read(request) {
            ClipboardRead::Text(text) => log::debug!(
                "tab {}: pasted {} characters",
                self.tab_id.value(),
                text.chars().count()
            ),
            ClipboardRead::AskPermission(request) => self.forward_page_request(
                web_view.url(),
                PageRequestKind::Permission(talaria_core::PagePermission::ClipboardRead),
                ServoPageRequest::ClipboardRead(request),
            ),
            ClipboardRead::Refused => log::debug!(
                "tab {}: refused a page's clipboard read (no user gesture, or script reads are off)",
                self.tab_id.value()
            ),
        }
    }

    fn set_text(&self, _web_view: servo::WebView, new_contents: String) {
        if !self.clipboard.write_text(&new_contents) {
            log::debug!(
                "tab {}: refused a page's clipboard write without a user gesture",
                self.tab_id.value()
            );
        }
    }
}

/// What a freshly built web view's own caches hold. Servo only notifies changes against these
/// caches (and starts with `Cursor::Pointer`), so the tab's copies must start from the same
/// values, also when the web view replaces an earlier one.
fn new_web_view_feedback(cursor: Cursor, status_text: Option<String>) -> [BrowserEvent; 3] {
    [
        BrowserEvent::CursorChanged {
            cursor: pointer_cursor_from_servo(cursor),
        },
        BrowserEvent::StatusTextChanged { text: status_text },
        BrowserEvent::FullscreenChanged { fullscreen: false },
    ]
}

/// A crashed page never answers a pointer leave, so the web view's caches keep their pre-crash
/// values and Servo will only notify changes against those. The tab copies them right after
/// the failure, so it stays in step when the page is reloaded.
fn crash_feedback(
    reason: String,
    cursor: Cursor,
    status_text: Option<String>,
) -> [BrowserEvent; 4] {
    [
        BrowserEvent::LoadStateChanged {
            state: LoadState::Failed { message: reason },
        },
        BrowserEvent::CursorChanged {
            cursor: pointer_cursor_from_servo(cursor),
        },
        BrowserEvent::StatusTextChanged { text: status_text },
        // media-capture: the page is gone, and with it any camera or microphone access.
        BrowserEvent::PageCrashed,
    ]
}

fn pointer_cursor_from_servo(cursor: Cursor) -> PointerCursor {
    match cursor {
        Cursor::None => PointerCursor::None,
        Cursor::Default => PointerCursor::Default,
        Cursor::Pointer => PointerCursor::Pointer,
        Cursor::ContextMenu => PointerCursor::ContextMenu,
        Cursor::Help => PointerCursor::Help,
        Cursor::Progress => PointerCursor::Progress,
        Cursor::Wait => PointerCursor::Wait,
        Cursor::Cell => PointerCursor::Cell,
        Cursor::Crosshair => PointerCursor::Crosshair,
        Cursor::Text => PointerCursor::Text,
        Cursor::VerticalText => PointerCursor::VerticalText,
        Cursor::Alias => PointerCursor::Alias,
        Cursor::Copy => PointerCursor::Copy,
        Cursor::Move => PointerCursor::Move,
        Cursor::NoDrop => PointerCursor::NoDrop,
        Cursor::NotAllowed => PointerCursor::NotAllowed,
        Cursor::Grab => PointerCursor::Grab,
        Cursor::Grabbing => PointerCursor::Grabbing,
        Cursor::EResize => PointerCursor::EResize,
        Cursor::NResize => PointerCursor::NResize,
        Cursor::NeResize => PointerCursor::NeResize,
        Cursor::NwResize => PointerCursor::NwResize,
        Cursor::SResize => PointerCursor::SResize,
        Cursor::SeResize => PointerCursor::SeResize,
        Cursor::SwResize => PointerCursor::SwResize,
        Cursor::WResize => PointerCursor::WResize,
        Cursor::EwResize => PointerCursor::EwResize,
        Cursor::NsResize => PointerCursor::NsResize,
        Cursor::NeswResize => PointerCursor::NeswResize,
        Cursor::NwseResize => PointerCursor::NwseResize,
        Cursor::ColResize => PointerCursor::ColResize,
        Cursor::RowResize => PointerCursor::RowResize,
        Cursor::AllScroll => PointerCursor::AllScroll,
        Cursor::ZoomIn => PointerCursor::ZoomIn,
        Cursor::ZoomOut => PointerCursor::ZoomOut,
    }
}

pub(crate) fn display_url(url: &Url) -> String {
    if url.as_str() == NEW_TAB_URL {
        String::new()
    } else {
        url.to_string()
    }
}

fn page_icon_from_pixels(
    width: u32,
    height: u32,
    format: PixelFormat,
    pixels: &[u8],
) -> Option<PageIcon> {
    let pixel_count = width.checked_mul(height)? as usize;
    let channel_count = match format {
        PixelFormat::K8 => 1,
        PixelFormat::KA8 => 2,
        PixelFormat::RGB8 => 3,
        PixelFormat::RGBA8 | PixelFormat::BGRA8 => 4,
    };

    if width == 0 || height == 0 || pixels.len() != pixel_count.checked_mul(channel_count)? {
        return None;
    }

    let mut rgba = Vec::with_capacity(pixel_count.checked_mul(4)?);

    match format {
        PixelFormat::K8 => {
            for &luminance in pixels {
                rgba.extend_from_slice(&[luminance, luminance, luminance, 255]);
            }
        }
        PixelFormat::KA8 => {
            for pixel in pixels.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        PixelFormat::RGB8 => {
            for pixel in pixels.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        PixelFormat::RGBA8 => rgba.extend_from_slice(pixels),
        PixelFormat::BGRA8 => {
            for pixel in pixels.as_chunks::<4>().0 {
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        }
    }

    PageIcon::from_rgba(width, height, rgba)
}

fn load_state_from_servo(status: LoadStatus) -> LoadState {
    match status {
        LoadStatus::Started | LoadStatus::HeadParsed => LoadState::Loading,
        LoadStatus::Complete | LoadStatus::Stopped => LoadState::Loaded,
    }
}

fn history_changed_event(entry_count: usize, current: usize) -> BrowserEvent {
    let can_go_back = current > 0;
    let can_go_forward = current
        .checked_add(1)
        .is_some_and(|next| next < entry_count);

    BrowserEvent::HistoryChanged {
        can_go_back,
        can_go_forward,
    }
}

fn ensure_crypto_provider() {
    if CryptoProvider::get_default().is_none() {
        let _installation_result = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }
}

#[cfg(test)]
mod tests {
    use super::mercury_options;

    #[test]
    fn enables_content_process_isolation() {
        assert!(mercury_options(None).multiprocess);
    }

    #[test]
    fn keeps_the_unreliable_os_sandbox_disabled() {
        assert!(!mercury_options(None).sandbox);
    }

    use super::*;

    #[test]
    fn hard_fail_is_disabled() {
        assert!(!mercury_options(None).hard_fail);
    }

    #[test]
    fn config_dir_is_passed_to_servo() {
        let config_dir = PathBuf::from("/profile/servo");

        assert_eq!(
            mercury_options(Some(config_dir.clone())).config_dir,
            Some(config_dir)
        );
        assert_eq!(mercury_options(None).config_dir, None);
    }

    #[test]
    fn mercury_keeps_servos_default_script_settings() {
        for gpu_canvas in [true, false] {
            let preferences = mercury_preferences(&RuntimeOptions {
                gpu_canvas,
                ..RuntimeOptions::default()
            });

            assert_eq!(
                preferences.dom_script_asynch,
                Preferences::default().dom_script_asynch
            );
        }
    }

    #[test]
    fn gpu_canvas_is_enabled_by_default() {
        let preferences = mercury_preferences(&RuntimeOptions::default());

        assert_eq!(preferences.dom_canvas_backend, "vello");
    }

    #[test]
    fn gpu_canvas_can_be_disabled() {
        let preferences = mercury_preferences(&RuntimeOptions {
            gpu_canvas: false,
            ..RuntimeOptions::default()
        });

        assert_eq!(
            preferences.dom_canvas_backend,
            Preferences::default().dom_canvas_backend
        );
    }

    fn preferences_with_experimental_web_features(enabled: bool) -> Preferences {
        mercury_preferences(&RuntimeOptions {
            experimental_web_features: enabled,
            ..RuntimeOptions::default()
        })
    }

    #[test]
    fn experimental_features_enable_servoshells_list_except_the_excluded_ones() {
        let preferences = preferences_with_experimental_web_features(true);

        assert_eq!(EXPERIMENTAL_PREFS.len(), 18);
        assert_eq!(
            EXCLUDED_EXPERIMENTAL_PREFS,
            &[
                "dom_async_clipboard_enabled",
                "dom_webgpu_enabled",
                "layout_css_container_queries_enabled",
            ]
        );
        let enabled = EXPERIMENTAL_PREFS
            .iter()
            .filter(|name| !EXCLUDED_EXPERIMENTAL_PREFS.contains(name))
            .collect::<Vec<_>>();
        assert_eq!(enabled.len(), 15);
        for name in enabled {
            assert_eq!(preferences.get_value(name), PrefValue::Bool(true), "{name}");
        }
    }

    // engine prefs
    #[test]
    fn container_queries_stay_off_so_that_pages_do_not_detect_them() {
        // `@supports` and `CSS.supports` follow this preference, but Servo drops `@container`.
        for enabled in [true, false] {
            let preferences = preferences_with_experimental_web_features(enabled);

            assert!(
                !preferences.layout_css_container_queries_enabled,
                "experimental features: {enabled}"
            );
        }
    }

    #[test]
    fn web_compatibility_preferences_do_not_need_the_experimental_switch() {
        for enabled in [true, false] {
            let preferences = preferences_with_experimental_web_features(enabled);

            assert!(preferences.dom_composition_event_enabled, "{enabled}");
            assert!(preferences.dom_adoptedstylesheet_enabled, "{enabled}");
            assert!(preferences.dom_visual_viewport_enabled, "{enabled}");
        }
    }

    #[test]
    fn async_clipboard_stays_disabled() {
        // `navigator.clipboard` stays off until `talaria_core::ASYNC_CLIPBOARD_OFFERED` is turned
        // on, which its doc comment lets happen only with the engine's frame gate (or the
        // owner's written acceptance of the cross-origin limit). Nothing here may turn it on
        // for the experimental features or any other setting.
        for enabled in [true, false] {
            assert!(
                !preferences_with_experimental_web_features(enabled).dom_async_clipboard_enabled,
                "the async clipboard is on: see ASYNC_CLIPBOARD_OFFERED for what must hold first"
            );
        }
    }

    #[test]
    fn webgpu_stays_disabled_while_the_webgpu_feature_is_not_built() {
        for enabled in [true, false] {
            assert!(!preferences_with_experimental_web_features(enabled).dom_webgpu_enabled);
        }
    }

    #[test]
    fn experimental_features_are_on_by_default() {
        assert!(RuntimeOptions::default().experimental_web_features);
    }

    #[test]
    fn experimental_features_can_be_disabled() {
        let preferences = preferences_with_experimental_web_features(false);
        let defaults = Preferences::default();

        for name in EXPERIMENTAL_PREFS {
            assert_eq!(
                preferences.get_value(name),
                defaults.get_value(name),
                "{name}"
            );
        }
    }

    #[test]
    fn pages_cannot_choose_their_own_upload_files() {
        for enabled in [true, false] {
            assert!(
                !preferences_with_experimental_web_features(enabled)
                    .dom_testing_html_input_element_select_files_enabled
            );
        }
    }

    // media-capture
    #[test]
    fn camera_and_microphone_capture_is_on_for_macos_only() {
        for enabled in [true, false] {
            assert_eq!(
                preferences_with_experimental_web_features(enabled).dom_webrtc_enabled,
                cfg!(target_os = "macos")
            );
        }
    }

    // media-capture
    #[test]
    fn webrtc_peer_connections_stay_hidden() {
        for enabled in [true, false] {
            assert!(
                !preferences_with_experimental_web_features(enabled).dom_webrtc_transceiver_enabled
            );
        }
    }

    // engine prefs
    #[test]
    fn the_first_two_waves_preferences_exist_and_stay_off() {
        for enabled in [true, false] {
            let preferences = preferences_with_experimental_web_features(enabled);
            for name in [
                "dom_media_source_enabled",
                "dom_media_capabilities_enabled",
                "dom_eme_enabled",
                "dom_picture_in_picture_enabled",
                "dom_video_frame_callback_enabled",
                "dom_webcodecs_enabled",
                "dom_display_capture_enabled",
                "dom_audio_output_enabled",
                "dom_audioworklet_enabled",
                "dom_media_recorder_enabled",
                "dom_webrtc_peerconnection_enabled",
                "media_eme_clearkey_enabled",
                "network_cookie_samesite_lax_default",
                "dom_web_locks_enabled",
                // Pinned although Servo's default is already off: `cookieStore` would bypass the
                // third-party cookie rule and Global Privacy Control.
                "dom_cookiestore_enabled",
                "dom_web_animations_enabled",
                "dom_serviceworker_enabled",
                "dom_webgpu_enabled",
                "layout_css_container_queries_enabled",
                // a11y-1: `accessibility_enabled` is on now (`engine_prefs.rs`); its checks stay off.
                "expensive_accessibility_test_assertions_enabled",
                // The media-capture pin stays: the new switch above replaces it later.
                "dom_webrtc_transceiver_enabled",
            ] {
                assert!(
                    Preferences::exists(name),
                    "{name} is not a Servo preference"
                );
                assert_eq!(
                    preferences.get_value(name),
                    PrefValue::Bool(false),
                    "{name} (experimental features: {enabled})"
                );
            }
        }
    }

    // a11y-1
    #[test]
    fn page_accessibility_is_on_whatever_the_experimental_switch_says() {
        for enabled in [true, false] {
            let preferences = preferences_with_experimental_web_features(enabled);
            assert!(preferences.accessibility_enabled, "{enabled}");
            assert!(
                !preferences.expensive_accessibility_test_assertions_enabled,
                "{enabled}"
            );
        }
    }

    #[test]
    fn mercury_sets_global_privacy_control_and_blocks_audible_autoplay() {
        // Not behind the experimental switch: both values are the product's.
        for enabled in [true, false] {
            let preferences = preferences_with_experimental_web_features(enabled);

            assert!(preferences.privacy_gpc_enabled, "{enabled}");
            assert_eq!(preferences.media_autoplay_policy, "block-audible");
            for name in [
                "media_autoplay_allow_origins",
                "media_autoplay_block_origins",
                "media_webrtc_ip_policy",
            ] {
                assert_eq!(
                    preferences.get_value(name),
                    PrefValue::Str(String::new()),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn mercury_hands_its_stylo_preferences_to_the_engine() {
        for enabled in [true, false] {
            let preferences = preferences_with_experimental_web_features(enabled);

            assert_eq!(
                preferences.stylo_prefs,
                "layout.css.nth-child-of.enabled=true,layout.css.counters.enabled=true,\
                 layout.css.text-overflow.enabled=true"
            );
            // Every name is one Stylo has (the engine ignores names it does not know).
            assert!(servo::prefs::apply_stylo_prefs(&preferences.stylo_prefs).is_empty());
            assert_eq!(
                servo::prefs::stylo_bool_pref("layout.css.nth-child-of.enabled"),
                Some(true)
            );
        }
    }

    use mercury_proxy::ProxyServer;

    fn proxy_preferences(preferences: &Preferences) -> [&str; 3] {
        [
            &preferences.network_http_proxy_uri,
            &preferences.network_https_proxy_uri,
            &preferences.network_http_no_proxy,
        ]
    }

    fn preferences_without_environment_proxies() -> Preferences {
        let mut preferences = Preferences::default();
        preferences.network_http_proxy_uri.clear();
        preferences.network_https_proxy_uri.clear();
        preferences.network_http_no_proxy.clear();
        preferences
    }

    fn system_proxy() -> SystemProxySettings {
        SystemProxySettings {
            http: Some(ProxyServer::new("proxy.example", Some(3128))),
            exceptions: vec!["*.local".to_owned()],
            ..SystemProxySettings::default()
        }
    }

    #[test]
    fn the_system_proxy_fills_servos_proxy_preferences() {
        let mut preferences = preferences_without_environment_proxies();

        apply_system_proxy(&mut preferences, Some(&system_proxy()));

        assert_eq!(
            proxy_preferences(&preferences),
            [
                "http://proxy.example:3128",
                "",
                "localhost,127.0.0.0/8,::1,.local"
            ]
        );
    }

    #[test]
    fn without_system_proxy_settings_the_preferences_are_unchanged() {
        let mut preferences = preferences_without_environment_proxies();

        apply_system_proxy(&mut preferences, None);
        apply_system_proxy(&mut preferences, Some(&SystemProxySettings::default()));

        assert_eq!(proxy_preferences(&preferences), ["", "", ""]);
    }

    #[test]
    fn proxy_environment_variables_win_over_the_system_proxy() {
        // Servo's `Preferences::default` reads `http_proxy`, `https_proxy` and `no_proxy`.
        for (index, value) in [
            "http://env.example:1",
            "http://env.example:2",
            "env.example",
        ]
        .into_iter()
        .enumerate()
        {
            let mut preferences = preferences_without_environment_proxies();
            match index {
                0 => preferences.network_http_proxy_uri = value.to_owned(),
                1 => preferences.network_https_proxy_uri = value.to_owned(),
                _ => preferences.network_http_no_proxy = value.to_owned(),
            }
            let before = proxy_preferences(&preferences).map(str::to_owned);

            apply_system_proxy(&mut preferences, Some(&system_proxy()));

            assert_eq!(proxy_preferences(&preferences).map(str::to_owned), before);
        }
    }

    #[test]
    fn the_http_cache_stays_in_memory_without_a_cache_directory() {
        let preferences = mercury_preferences(&RuntimeOptions::default());

        assert_eq!(preferences.network_http_disk_cache, "");
    }

    #[test]
    fn the_http_disk_cache_is_a_database_in_the_cache_directory() {
        let preferences = mercury_preferences(&RuntimeOptions {
            http_disk_cache_dir: Some(PathBuf::from("/profile/http-cache")),
            ..RuntimeOptions::default()
        });

        assert_eq!(
            preferences.network_http_disk_cache,
            "/profile/http-cache/cache.sqlite3"
        );
        assert_eq!(
            preferences.network_http_disk_cache_size,
            Preferences::default().network_http_disk_cache_size
        );
        assert_eq!(preferences.network_http_disk_cache_size, 100 * 1024 * 1024);
    }

    // geolocation
    #[test]
    fn geolocation_is_off_by_default() {
        assert_eq!(RuntimeOptions::default().geolocation, GeolocationSetup::Off);
        for enabled in [true, false] {
            assert!(!preferences_with_experimental_web_features(enabled).dom_geolocation_enabled);
        }
    }

    #[test]
    fn geolocation_setups_that_answer_pages_turn_the_preference_on() {
        for setup in [
            GeolocationSetup::System,
            GeolocationSetup::Diagnostic(crate::GeolocationDiagnostic::Deny),
        ] {
            let expected = setup.exposes_geolocation();
            let preferences = mercury_preferences(&RuntimeOptions {
                geolocation: setup,
                ..RuntimeOptions::default()
            });

            assert_eq!(preferences.dom_geolocation_enabled, expected);
            // Servo's own fixed test position stays off: positions come from Mercury's source.
            assert!(!preferences.dom_geolocation_test_position_enabled);
        }
    }

    #[test]
    fn user_agent_drops_the_servo_token() {
        assert_eq!(
            firefox_compatible_user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:153.0) Servo/0.8.0 Firefox/153.0"
            ),
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:153.0) Gecko/20100101 Firefox/153.0"
        );
        assert_eq!(
            firefox_compatible_user_agent(
                "Mozilla/5.0 (X11; Linux aarch64; rv:153.0) Servo/0.8.0 Firefox/153.0"
            ),
            "Mozilla/5.0 (X11; Linux aarch64; rv:153.0) Gecko/20100101 Firefox/153.0"
        );
    }

    #[test]
    fn user_agent_without_a_servo_token_is_unchanged() {
        let user_agent = "Mozilla/5.0 (X11; Linux x86_64; rv:153.0) Gecko/20100101 Firefox/153.0";

        assert_eq!(firefox_compatible_user_agent(user_agent), user_agent);
    }

    #[test]
    fn the_user_agent_is_firefox_compatible_in_both_modes() {
        for enabled in [true, false] {
            let user_agent = preferences_with_experimental_web_features(enabled).user_agent;

            assert!(!user_agent.contains("Servo/"), "{user_agent}");
            assert!(
                user_agent.contains(" Gecko/20100101 Firefox/"),
                "{user_agent}"
            );
        }
    }

    #[test]
    fn started_and_head_parsed_are_loading_states() {
        assert_eq!(
            load_state_from_servo(LoadStatus::Started),
            LoadState::Loading
        );
        assert_eq!(
            load_state_from_servo(LoadStatus::HeadParsed),
            LoadState::Loading
        );
    }

    #[test]
    fn complete_is_a_loaded_state() {
        assert_eq!(
            load_state_from_servo(LoadStatus::Complete),
            LoadState::Loaded
        );
    }

    #[test]
    fn stopped_is_a_loaded_state() {
        assert_eq!(
            load_state_from_servo(LoadStatus::Stopped),
            LoadState::Loaded
        );
    }

    #[test]
    fn history_event_disables_both_directions_for_the_only_entry() {
        assert_eq!(
            history_changed_event(1, 0),
            BrowserEvent::HistoryChanged {
                can_go_back: false,
                can_go_forward: false,
            }
        );
    }

    #[test]
    fn history_event_enables_both_directions_for_a_middle_entry() {
        assert_eq!(
            history_changed_event(3, 1),
            BrowserEvent::HistoryChanged {
                can_go_back: true,
                can_go_forward: true,
            }
        );
    }

    #[test]
    fn history_event_disables_forward_at_the_last_entry() {
        assert_eq!(
            history_changed_event(3, 2),
            BrowserEvent::HistoryChanged {
                can_go_back: true,
                can_go_forward: false,
            }
        );
    }

    #[test]
    fn history_event_handles_an_overflowing_current_index() {
        assert_eq!(
            history_changed_event(3, usize::MAX),
            BrowserEvent::HistoryChanged {
                can_go_back: true,
                can_go_forward: false,
            }
        );
    }

    #[test]
    fn every_servo_cursor_maps_to_a_pointer_cursor() {
        let pairs = [
            (Cursor::None, PointerCursor::None),
            (Cursor::Default, PointerCursor::Default),
            (Cursor::Pointer, PointerCursor::Pointer),
            (Cursor::ContextMenu, PointerCursor::ContextMenu),
            (Cursor::Help, PointerCursor::Help),
            (Cursor::Progress, PointerCursor::Progress),
            (Cursor::Wait, PointerCursor::Wait),
            (Cursor::Cell, PointerCursor::Cell),
            (Cursor::Crosshair, PointerCursor::Crosshair),
            (Cursor::Text, PointerCursor::Text),
            (Cursor::VerticalText, PointerCursor::VerticalText),
            (Cursor::Alias, PointerCursor::Alias),
            (Cursor::Copy, PointerCursor::Copy),
            (Cursor::Move, PointerCursor::Move),
            (Cursor::NoDrop, PointerCursor::NoDrop),
            (Cursor::NotAllowed, PointerCursor::NotAllowed),
            (Cursor::Grab, PointerCursor::Grab),
            (Cursor::Grabbing, PointerCursor::Grabbing),
            (Cursor::EResize, PointerCursor::EResize),
            (Cursor::NResize, PointerCursor::NResize),
            (Cursor::NeResize, PointerCursor::NeResize),
            (Cursor::NwResize, PointerCursor::NwResize),
            (Cursor::SResize, PointerCursor::SResize),
            (Cursor::SeResize, PointerCursor::SeResize),
            (Cursor::SwResize, PointerCursor::SwResize),
            (Cursor::WResize, PointerCursor::WResize),
            (Cursor::EwResize, PointerCursor::EwResize),
            (Cursor::NsResize, PointerCursor::NsResize),
            (Cursor::NeswResize, PointerCursor::NeswResize),
            (Cursor::NwseResize, PointerCursor::NwseResize),
            (Cursor::ColResize, PointerCursor::ColResize),
            (Cursor::RowResize, PointerCursor::RowResize),
            (Cursor::AllScroll, PointerCursor::AllScroll),
            (Cursor::ZoomIn, PointerCursor::ZoomIn),
            (Cursor::ZoomOut, PointerCursor::ZoomOut),
        ];
        assert_eq!(pairs.len(), 35);
        for (servo_cursor, pointer_cursor) in pairs {
            assert_eq!(
                pointer_cursor_from_servo(servo_cursor),
                pointer_cursor,
                "{servo_cursor:?}"
            );
        }
    }

    #[test]
    fn a_crash_reports_the_failure_then_the_web_views_cached_feedback() {
        assert_eq!(
            crash_feedback(
                "boom".to_owned(),
                Cursor::Pointer,
                Some("https://a.test/".to_owned())
            ),
            [
                BrowserEvent::LoadStateChanged {
                    state: LoadState::Failed {
                        message: "boom".to_owned(),
                    },
                },
                BrowserEvent::CursorChanged {
                    cursor: PointerCursor::Pointer,
                },
                BrowserEvent::StatusTextChanged {
                    text: Some("https://a.test/".to_owned()),
                },
                // media-capture
                BrowserEvent::PageCrashed,
            ]
        );
    }

    #[test]
    fn a_new_web_view_reports_the_engines_starting_feedback() {
        assert_eq!(
            new_web_view_feedback(Cursor::Pointer, None),
            [
                BrowserEvent::CursorChanged {
                    cursor: PointerCursor::Pointer,
                },
                BrowserEvent::StatusTextChanged { text: None },
                BrowserEvent::FullscreenChanged { fullscreen: false },
            ]
        );
    }

    #[test]
    fn new_tab_url_is_hidden_from_the_address_bar() {
        let url = Url::parse(NEW_TAB_URL).expect("the new tab URL should be valid");

        assert_eq!(display_url(&url), "");
    }

    #[test]
    fn regular_url_is_preserved_for_the_address_bar() {
        let url = Url::parse("https://example.com/path").expect("the test URL should be valid");

        assert_eq!(display_url(&url), "https://example.com/path");
    }

    #[test]
    fn rgba_favicon_pixels_are_preserved() {
        let icon = page_icon_from_pixels(1, 1, PixelFormat::RGBA8, &[10, 20, 30, 40])
            .expect("valid RGBA pixels should create an icon");

        assert_eq!(icon.rgba(), &[10, 20, 30, 40]);
    }

    #[test]
    fn bgra_favicon_pixels_are_converted_to_rgba() {
        let icon = page_icon_from_pixels(1, 1, PixelFormat::BGRA8, &[30, 20, 10, 40])
            .expect("valid BGRA pixels should create an icon");

        assert_eq!(icon.rgba(), &[10, 20, 30, 40]);
    }

    #[test]
    fn rgb_favicon_pixels_receive_an_opaque_alpha_channel() {
        let icon = page_icon_from_pixels(1, 1, PixelFormat::RGB8, &[10, 20, 30])
            .expect("valid RGB pixels should create an icon");

        assert_eq!(icon.rgba(), &[10, 20, 30, 255]);
    }

    #[test]
    fn luminance_favicon_pixels_expand_to_rgb() {
        let icon = page_icon_from_pixels(1, 1, PixelFormat::K8, &[42])
            .expect("valid luminance pixels should create an icon");

        assert_eq!(icon.rgba(), &[42, 42, 42, 255]);
    }

    #[test]
    fn luminance_alpha_favicon_pixels_expand_to_rgba() {
        let icon = page_icon_from_pixels(1, 1, PixelFormat::KA8, &[42, 128])
            .expect("valid luminance-alpha pixels should create an icon");

        assert_eq!(icon.rgba(), &[42, 42, 42, 128]);
    }

    #[test]
    fn favicon_pixels_with_an_invalid_length_are_rejected() {
        assert_eq!(
            page_icon_from_pixels(1, 1, PixelFormat::RGBA8, &[10, 20, 30]),
            None
        );
    }
}
