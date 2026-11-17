//! Servo's side: `servo::GeolocationDelegate` for each web view, answering from the hub.

use std::cell::RefCell;
use std::rc::Rc;

use servo::{
    GeolocationError, GeolocationPositionData, GeolocationWatchId, LoadStatus, PermissionFeature,
    PermissionRequest, PositionRequest, PositionWatch, WebViewBuilder,
};
use url::Url;

use super::fake::FakeLocationSource;
use super::history::HistoryRule;
use super::hub::{Forget, LocationHub, OwnerGuard, PendingPosition, PositionSubscriber, WatchKey};
#[cfg(target_os = "macos")]
use super::system_source;
use super::{
    FakeLocation, GeolocationDiagnostic, GeolocationSetup, LocationError, LocationFix,
    LocationSource,
};

type ServoHub = LocationHub<PositionRequest, PositionWatch, GeolocationWatchId>;

/// The browser's one location hub, and the diagnostic's permission answer if any.
pub(crate) struct GeolocationService {
    hub: Rc<ServoHub>,
    /// `Some(allow)` only in a diagnostic run.
    diagnostic_permission: Option<bool>,
}

impl GeolocationService {
    /// `None` when geolocation is off: web views then get no delegate of Mercury's.
    pub(crate) fn for_setup(setup: &GeolocationSetup) -> Option<Rc<Self>> {
        let (diagnostic_permission, hub) = match setup {
            GeolocationSetup::Off => return None,
            #[cfg(target_os = "macos")]
            GeolocationSetup::System => (None, ServoHub::new(system_source)),
            // No location source on this platform: geolocation stays off.
            #[cfg(not(target_os = "macos"))]
            GeolocationSetup::System => return None,
            GeolocationSetup::Diagnostic(GeolocationDiagnostic::Deny) => {
                (Some(false), fake_hub(Err(LocationError::Unavailable)))
            }
            GeolocationSetup::Diagnostic(GeolocationDiagnostic::Allow(fake)) => {
                let answer = match fake {
                    FakeLocation::Position(position) => Ok(position.fix()),
                    FakeLocation::Error(error) => Err(*error),
                };
                (Some(true), fake_hub(answer))
            }
        };
        Some(Rc::new(Self {
            hub,
            diagnostic_permission,
        }))
    }

    /// The delegate for one new web view.
    pub(crate) fn web_view(self: &Rc<Self>) -> Rc<WebViewGeolocation> {
        Rc::new(WebViewGeolocation::new(
            &self.hub,
            self.diagnostic_permission,
        ))
    }
}

fn fake_hub(answer: Result<LocationFix, LocationError>) -> Rc<ServoHub> {
    ServoHub::new(move || {
        Rc::new(FakeLocationSource::answering(answer.clone())) as Rc<dyn LocationSource>
    })
}

/// One web view's geolocation, generic over the hub's types so the rules can be tested
/// without Servo. Dropped with the web view, which forgets the web view's requests and watches.
pub(crate) struct PageGeolocationOf<R, W, K>
where
    R: PendingPosition + 'static,
    W: PositionSubscriber + 'static,
    K: WatchKey + 'static,
{
    owner: OwnerGuard<R, W, K>,
    diagnostic_permission: Option<bool>,
    history: RefCell<HistoryRule>,
}

/// Servo's `GeolocationDelegate` for one web view; also held by its `BrowserDelegate` (as
/// `PageGeolocation`) and its `ServoWebView` (for visibility).
pub(crate) type WebViewGeolocation =
    PageGeolocationOf<PositionRequest, PositionWatch, GeolocationWatchId>;

impl<R, W, K> PageGeolocationOf<R, W, K>
where
    R: PendingPosition + 'static,
    W: PositionSubscriber + 'static,
    K: WatchKey + 'static,
{
    pub(crate) fn new(hub: &Rc<LocationHub<R, W, K>>, diagnostic_permission: Option<bool>) -> Self {
        Self {
            owner: OwnerGuard::new(hub),
            diagnostic_permission,
            history: RefCell::default(),
        }
    }

    /// The web view is shown or hidden (tab switches).
    pub(crate) fn set_visible(&self, visible: bool) {
        self.owner
            .hub()
            .set_owner_visible(self.owner.owner(), visible);
    }
}

/// What the web view's `BrowserDelegate` and `ServoWebView` tell geolocation.
pub(crate) trait PageGeolocation {
    /// A new top-level document starts (or is about to, when Servo will not report it): the
    /// document left drops its waiting one-shot requests and parks its watches (it may come
    /// back from the back-forward cache), and its late watches are refused. Servo itself ends
    /// them only when that document's pipeline exits.
    fn document_started(&self);
    /// Servo's history notification; see `HistoryRule` and `LocationHub::history_changed`.
    fn history_changed(&self, entries: &[Url], current: usize);
    /// In a diagnostic run, answers Servo's geolocation permission request and returns `None`;
    /// otherwise hands the request back.
    fn answer_permission_in_diagnostic(
        &self,
        request: PermissionRequest,
    ) -> Option<PermissionRequest>;
}

impl<R, W, K> PageGeolocation for PageGeolocationOf<R, W, K>
where
    R: PendingPosition + 'static,
    W: PositionSubscriber + 'static,
    K: WatchKey + 'static,
{
    fn document_started(&self) {
        self.owner
            .hub()
            .forget_owner(self.owner.owner(), Forget::DocumentStarted);
    }

    fn history_changed(&self, entries: &[Url], current: usize) {
        let change = self.history.borrow_mut().history_changed(entries, current);
        self.owner.hub().history_changed(self.owner.owner(), change);
    }

    fn answer_permission_in_diagnostic(
        &self,
        request: PermissionRequest,
    ) -> Option<PermissionRequest> {
        match diagnostic_answer(self.diagnostic_permission, request.feature()) {
            Some(true) => request.allow(),
            Some(false) => request.deny(),
            None => return Some(request),
        }
        None
    }
}

/// `LoadStatus::Started` is a new top-level document (Servo sends it for top-level documents
/// only, `servo-script` `document.rs` `set_document_readiness_to_loading_for_initialization`).
pub(crate) fn on_load_status(geolocation: Option<&dyn PageGeolocation>, status: LoadStatus) {
    if let (Some(geolocation), LoadStatus::Started) = (geolocation, status) {
        geolocation.document_started();
    }
}

pub(crate) fn on_history_changed(
    geolocation: Option<&dyn PageGeolocation>,
    entries: &[Url],
    current: usize,
) {
    if let Some(geolocation) = geolocation {
        geolocation.history_changed(entries, current);
    }
}

fn diagnostic_answer(
    diagnostic_permission: Option<bool>,
    feature: PermissionFeature,
) -> Option<bool> {
    match feature {
        PermissionFeature::Geolocation => diagnostic_permission,
        _ => None,
    }
}

impl servo::GeolocationDelegate for WebViewGeolocation {
    fn request_position(&self, _webview: servo::WebView, request: PositionRequest) {
        let high_accuracy = request.options().high_accuracy;
        self.owner
            .hub()
            .request_position(self.owner.owner(), high_accuracy, request);
    }

    fn start_watch(&self, _webview: servo::WebView, watch: PositionWatch) {
        let high_accuracy = watch.options().high_accuracy;
        self.owner
            .hub()
            .start_watch(self.owner.owner(), high_accuracy, watch.id(), watch);
    }

    fn stop_watch(&self, _webview: servo::WebView, id: &GeolocationWatchId) {
        self.owner.hub().stop_watch(id);
    }
}

/// The document is the watch's pipeline (`servo` does not re-export `PipelineId`, so it is kept
/// as a watch id with index 0).
impl WatchKey for GeolocationWatchId {
    type Document = GeolocationWatchId;

    fn document(&self) -> GeolocationWatchId {
        GeolocationWatchId {
            index: 0,
            ..self.clone()
        }
    }
}

impl PendingPosition for PositionRequest {
    fn succeed(self, fix: &LocationFix) {
        self.success(position_data(fix));
    }

    fn fail(self, error: LocationError) {
        self.failure(geolocation_error(error));
    }
}

impl PositionSubscriber for PositionWatch {
    fn update(&self, fix: &LocationFix) {
        PositionWatch::update(self, position_data(fix));
    }

    fn error(&self, error: LocationError) {
        PositionWatch::error(self, geolocation_error(error));
    }
}

fn position_data(fix: &LocationFix) -> GeolocationPositionData {
    GeolocationPositionData {
        accuracy: fix.accuracy,
        latitude: fix.latitude,
        longitude: fix.longitude,
        altitude: fix.altitude,
        altitude_accuracy: fix.altitude_accuracy,
        heading: fix.heading,
        speed: fix.speed,
    }
}

fn geolocation_error(error: LocationError) -> GeolocationError {
    match error {
        LocationError::PermissionDenied => {
            GeolocationError::permission_denied("Location services are off for Mercury Browser.")
        }
        LocationError::Unavailable => {
            GeolocationError::position_unavailable("The position is not available.")
        }
        LocationError::Timeout => GeolocationError::timeout("Finding the position took too long."),
    }
}

/// Where a web view's geolocation delegate is set: Servo's `WebViewBuilder` (and a fake in
/// tests).
pub(crate) trait GeolocationDelegateSlot: Sized {
    fn set_geolocation_delegate(self, delegate: Rc<dyn servo::GeolocationDelegate>) -> Self;
}

impl GeolocationDelegateSlot for WebViewBuilder {
    fn set_geolocation_delegate(self, delegate: Rc<dyn servo::GeolocationDelegate>) -> Self {
        self.geolocation_delegate(delegate)
    }
}

/// Adds the web view's geolocation delegate, if geolocation is on, to a `WebViewBuilder`.
pub(crate) trait WithGeolocation {
    fn with_geolocation(self, geolocation: Option<&Rc<WebViewGeolocation>>) -> Self;
}

impl<T: GeolocationDelegateSlot> WithGeolocation for T {
    fn with_geolocation(self, geolocation: Option<&Rc<WebViewGeolocation>>) -> Self {
        match geolocation {
            Some(geolocation) => self.set_geolocation_delegate(Rc::clone(geolocation) as _),
            None => self,
        }
    }
}

#[cfg(test)]
mod tests {
    use servo::GeolocationErrorKind;

    use super::*;
    use crate::geolocation::FixedPosition;

    #[test]
    fn geolocation_off_has_no_service() {
        assert!(GeolocationService::for_setup(&GeolocationSetup::Off).is_none());
    }

    #[test]
    fn the_system_setup_has_a_service_only_where_there_is_a_location_source() {
        assert_eq!(
            GeolocationService::for_setup(&GeolocationSetup::System).is_some(),
            cfg!(target_os = "macos")
        );
    }

    #[test]
    fn only_diagnostic_runs_answer_the_permission_and_only_for_geolocation() {
        let permission = |setup: GeolocationSetup| {
            GeolocationService::for_setup(&setup)
                .expect("geolocation is on")
                .diagnostic_permission
        };
        let allow = GeolocationSetup::Diagnostic(GeolocationDiagnostic::Allow(
            FakeLocation::Position(FixedPosition::new(1.0, 2.0, 3.0).expect("valid")),
        ));

        // `System` has a service only where there is a location source.
        #[cfg(target_os = "macos")]
        assert_eq!(permission(GeolocationSetup::System), None);
        assert_eq!(permission(allow), Some(true));
        assert_eq!(
            permission(GeolocationSetup::Diagnostic(GeolocationDiagnostic::Deny)),
            Some(false)
        );

        assert_eq!(
            diagnostic_answer(Some(true), PermissionFeature::Geolocation),
            Some(true)
        );
        assert_eq!(
            diagnostic_answer(Some(false), PermissionFeature::Geolocation),
            Some(false)
        );
        assert_eq!(
            diagnostic_answer(None, PermissionFeature::Geolocation),
            None
        );
        for other in [
            PermissionFeature::Notifications,
            PermissionFeature::Camera,
            PermissionFeature::PersistentStorage,
        ] {
            assert_eq!(diagnostic_answer(Some(true), other), None);
        }
    }

    #[test]
    fn a_fix_becomes_servos_position_data_field_for_field() {
        let fix = LocationFix {
            latitude: 1.0,
            longitude: 2.0,
            accuracy: 3.0,
            altitude: Some(4.0),
            altitude_accuracy: Some(5.0),
            heading: Some(6.0),
            speed: Some(7.0),
        };

        let data = position_data(&fix);

        assert!(
            data == GeolocationPositionData {
                accuracy: 3.0,
                latitude: 1.0,
                longitude: 2.0,
                altitude: Some(4.0),
                altitude_accuracy: Some(5.0),
                heading: Some(6.0),
                speed: Some(7.0),
            }
        );
    }

    #[test]
    fn errors_map_onto_the_pages_error_codes() {
        let kind = |error| geolocation_error(error).kind;

        assert!(kind(LocationError::PermissionDenied) == GeolocationErrorKind::PermissionDenied);
        assert!(kind(LocationError::Unavailable) == GeolocationErrorKind::PositionUnavailable);
        assert!(kind(LocationError::Timeout) == GeolocationErrorKind::Timeout);
    }

    // Review round 1

    use std::cell::RefCell;

    use crate::geolocation::LocationError;
    use crate::geolocation::fake::SourceCall;
    use crate::geolocation::test_support::{Harness, Seen, TestHub, fix};

    type TestPage = PageGeolocationOf<
        crate::geolocation::test_support::Request,
        crate::geolocation::test_support::Watch,
        u32,
    >;

    fn page(hub: &Rc<TestHub>) -> TestPage {
        TestPage::new(hub, None)
    }

    fn urls(entries: &[&str]) -> Vec<Url> {
        entries
            .iter()
            .map(|entry| Url::parse(&format!("https://{entry}.test/")).expect("valid"))
            .collect()
    }

    #[test]
    fn a_new_document_ends_the_old_ones_watches_and_refuses_its_late_ones() {
        let harness = Harness::silent();
        let page = page(&harness.hub);
        harness
            .hub
            .start_watch(page.owner.owner(), false, 101, harness.watch("old"));

        page.document_started();
        harness
            .hub
            .start_watch(page.owner.owner(), false, 102, harness.watch("late"));

        assert_eq!(
            harness.seen(),
            [
                Seen::Error("old", LocationError::Unavailable),
                Seen::Error("late", LocationError::Unavailable),
                Seen::Dropped("late"),
            ]
        );
    }

    #[test]
    fn going_back_parks_the_watches_and_forward_resumes_them() {
        let harness = Harness::silent();
        let page = page(&harness.hub);
        page.document_started();
        page.history_changed(&urls(&["a"]), 0);
        page.document_started();
        page.history_changed(&urls(&["a", "b"]), 1);
        harness
            .hub
            .start_watch(page.owner.owner(), false, 201, harness.watch("b"));
        harness.source.take_calls();

        page.history_changed(&urls(&["a", "b"]), 0);

        assert_eq!(
            harness.seen(),
            [Seen::Error("b", LocationError::Unavailable)]
        );
        assert_eq!(harness.source.take_calls(), [SourceCall::Stop]);

        page.history_changed(&urls(&["a", "b"]), 1);
        assert_eq!(
            harness.source.take_calls(),
            [SourceCall::Start(
                crate::geolocation::LocationAccuracy::Coarse
            )]
        );

        // An iframe navigation adds an entry and leaves the watch alone.
        page.history_changed(&urls(&["a", "b", "b"]), 2);
        harness.source.deliver(Ok(fix(5.0)));
        assert_eq!(harness.seen(), [Seen::Position("b", 5.0)]);
    }

    #[test]
    fn a_hidden_page_lets_the_source_stop() {
        let harness = Harness::silent();
        let page = page(&harness.hub);
        harness
            .hub
            .start_watch(page.owner.owner(), false, 1, harness.watch("watch"));
        harness.source.take_calls();

        page.set_visible(false);
        assert_eq!(harness.source.take_calls(), [SourceCall::Stop]);
        page.set_visible(true);
        assert_eq!(
            harness.source.take_calls(),
            [SourceCall::Start(
                crate::geolocation::LocationAccuracy::Coarse
            )]
        );
        harness.source.deliver(Ok(fix(1.0)));
        assert_eq!(harness.seen(), [Seen::Position("watch", 1.0)]);
    }

    #[derive(Default)]
    struct RecordingPage {
        calls: RefCell<Vec<String>>,
    }

    impl PageGeolocation for RecordingPage {
        fn document_started(&self) {
            self.calls.borrow_mut().push("document started".to_owned());
        }

        fn history_changed(&self, entries: &[Url], current: usize) {
            self.calls
                .borrow_mut()
                .push(format!("history {} {current}", entries.len()));
        }

        fn answer_permission_in_diagnostic(
            &self,
            request: PermissionRequest,
        ) -> Option<PermissionRequest> {
            Some(request)
        }
    }

    #[test]
    fn only_a_started_load_is_a_new_document() {
        let page = RecordingPage::default();
        for status in [
            LoadStatus::Started,
            LoadStatus::HeadParsed,
            LoadStatus::Complete,
            LoadStatus::Stopped,
        ] {
            on_load_status(Some(&page), status);
        }
        on_load_status(None, LoadStatus::Started);

        assert_eq!(*page.calls.borrow(), ["document started"]);
    }

    #[test]
    fn history_notifications_reach_the_page() {
        let page = RecordingPage::default();

        on_history_changed(Some(&page), &urls(&["a", "b"]), 1);
        on_history_changed(None, &urls(&["a"]), 0);

        assert_eq!(*page.calls.borrow(), ["history 2 1"]);
    }

    #[derive(Default)]
    struct FakeBuilder {
        delegate: Option<Rc<dyn servo::GeolocationDelegate>>,
    }

    impl GeolocationDelegateSlot for FakeBuilder {
        fn set_geolocation_delegate(
            mut self,
            delegate: Rc<dyn servo::GeolocationDelegate>,
        ) -> Self {
            self.delegate = Some(delegate);
            self
        }
    }

    #[test]
    fn a_web_views_builder_gets_its_geolocation_delegate_when_geolocation_is_on() {
        let service = GeolocationService::for_setup(&GeolocationSetup::Diagnostic(
            GeolocationDiagnostic::Deny,
        ))
        .expect("on");
        let geolocation = service.web_view();

        let builder = FakeBuilder::default().with_geolocation(Some(&geolocation));
        let attached = builder.delegate.expect("attached");
        assert!(std::ptr::addr_eq(
            Rc::as_ptr(&attached),
            Rc::as_ptr(&geolocation)
        ));

        assert!(
            FakeBuilder::default()
                .with_geolocation(None)
                .delegate
                .is_none()
        );
    }
}
