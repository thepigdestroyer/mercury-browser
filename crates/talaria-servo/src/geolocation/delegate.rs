//! Servo's side: `servo::GeolocationDelegate` for each web view, answering from the hub.

use std::rc::Rc;

use servo::{
    GeolocationError, GeolocationPositionData, GeolocationWatchId, PermissionFeature,
    PermissionRequest, PositionRequest, PositionWatch, WebViewBuilder,
};

use super::fake::FakeLocationSource;
use super::hub::{LocationHub, OwnerGuard, PendingPosition, PositionSubscriber};
use super::{
    FakeLocation, GeolocationDiagnostic, GeolocationSetup, LocationError, LocationFix,
    LocationSource, system_source,
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
            GeolocationSetup::System => (None, ServoHub::new(system_source)),
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
        Rc::new(WebViewGeolocation {
            owner: OwnerGuard::new(&self.hub),
            diagnostic_permission: self.diagnostic_permission,
        })
    }
}

fn fake_hub(answer: Result<LocationFix, LocationError>) -> Rc<ServoHub> {
    ServoHub::new(move || {
        Rc::new(FakeLocationSource::answering(answer.clone())) as Rc<dyn LocationSource>
    })
}

/// One web view's geolocation: Servo's delegate for it, shared with its `BrowserDelegate`.
/// Dropped with the web view, which forgets the web view's requests and watches.
pub(crate) struct WebViewGeolocation {
    owner: OwnerGuard<PositionRequest, PositionWatch, GeolocationWatchId>,
    diagnostic_permission: Option<bool>,
}

impl WebViewGeolocation {
    /// The web view started loading a new top-level document: the requests and watches of
    /// the one it leaves end. (Servo ends them itself only when that document's pipeline
    /// exits, which a page kept in the back-forward cache does not.)
    pub(crate) fn page_load_started(&self) {
        self.owner.hub().forget_owner(self.owner.owner());
    }

    /// In a diagnostic run, answers Servo's geolocation permission request and returns
    /// `None`; otherwise hands the request back.
    pub(crate) fn answer_permission_in_diagnostic(
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

/// Adds the web view's geolocation delegate, if geolocation is on, to a `WebViewBuilder`.
pub(crate) trait WithGeolocation {
    fn with_geolocation(self, geolocation: Option<&Rc<WebViewGeolocation>>) -> Self;
}

impl WithGeolocation for WebViewBuilder {
    fn with_geolocation(self, geolocation: Option<&Rc<WebViewGeolocation>>) -> Self {
        match geolocation {
            Some(geolocation) => self.geolocation_delegate(Rc::clone(geolocation) as _),
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
    fn only_diagnostic_runs_answer_the_permission_and_only_for_geolocation() {
        let permission = |setup: GeolocationSetup| {
            GeolocationService::for_setup(&setup)
                .expect("geolocation is on")
                .diagnostic_permission
        };
        let allow = GeolocationSetup::Diagnostic(GeolocationDiagnostic::Allow(
            FakeLocation::Position(FixedPosition::new(1.0, 2.0, 3.0).expect("valid")),
        ));

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
}
