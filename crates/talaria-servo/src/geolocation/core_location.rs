//! Positions from CoreLocation (`CLLocationManager`), on the main thread as CoreLocation
//! requires: the manager is created there, and its delegate's callbacks arrive on the main
//! run loop, which Slint's event loop runs.
//!
//! Nothing here runs until a page that holds Mercury's geolocation permission asks for a
//! position: the manager is created at the first `start`, and the system's "when in use"
//! authorisation is requested only then, if macOS has not been asked before.
//!
//! Verified by reading and compiling only: automated runs and tests never use this source.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use log::{info, warn};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_location::{
    CLAuthorizationStatus, CLError, CLLocation, CLLocationManager, CLLocationManagerDelegate,
    kCLErrorDomain, kCLLocationAccuracyBest, kCLLocationAccuracyHundredMeters,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};

use super::{LocationAccuracy, LocationError, LocationFix, LocationSink, LocationSource};

pub(crate) struct CoreLocationSource {
    state: Rc<SourceState>,
}

struct SourceState {
    /// Created at the first `start`; kept for later ones.
    manager: RefCell<Option<Manager>>,
    /// `Some` between `start` and `stop`.
    sink: RefCell<Option<LocationSink>>,
    accuracy: Cell<LocationAccuracy>,
    /// Whether `startUpdatingLocation` is in effect.
    updating: Cell<bool>,
}

struct Manager {
    manager: Retained<CLLocationManager>,
    /// The manager holds its delegate weakly; this keeps it alive.
    _delegate: Retained<LocationDelegate>,
}

impl CoreLocationSource {
    pub(crate) fn new() -> Self {
        Self {
            state: Rc::new(SourceState {
                manager: RefCell::default(),
                sink: RefCell::default(),
                accuracy: Cell::new(LocationAccuracy::Coarse),
                updating: Cell::new(false),
            }),
        }
    }
}

impl LocationSource for CoreLocationSource {
    fn start(&self, accuracy: LocationAccuracy, sink: LocationSink) {
        *self.state.sink.borrow_mut() = Some(sink);
        self.state.accuracy.set(accuracy);
        let Some(mtm) = MainThreadMarker::new() else {
            warn!("geolocation: CoreLocation can only be used on the main thread");
            self.state.deliver(Err(LocationError::Unavailable));
            return;
        };
        if self.state.manager.borrow().is_none() {
            let manager = Manager::new(mtm, Rc::downgrade(&self.state));
            *self.state.manager.borrow_mut() = Some(manager);
        }
        let manager = self.state.manager.borrow();
        if let Some(manager) = manager.as_ref() {
            manager.set_accuracy(accuracy);
            self.state.follow_authorization(&manager.manager);
        }
    }

    fn set_accuracy(&self, accuracy: LocationAccuracy) {
        self.state.accuracy.set(accuracy);
        if let Some(manager) = self.state.manager.borrow().as_ref() {
            manager.set_accuracy(accuracy);
        }
    }

    fn stop(&self) {
        *self.state.sink.borrow_mut() = None;
        if self.state.updating.replace(false)
            && let Some(manager) = self.state.manager.borrow().as_ref()
        {
            // SAFETY: called on the main thread, where the manager was created.
            unsafe { manager.manager.stopUpdatingLocation() };
        }
    }
}

impl Manager {
    fn new(mtm: MainThreadMarker, state: Weak<SourceState>) -> Self {
        let delegate = LocationDelegate::new(mtm, state);
        // SAFETY: on the main thread, so the delegate's callbacks arrive on the main run loop;
        // the delegate outlives the manager's use of it (both live in this struct).
        let manager = unsafe { CLLocationManager::new() };
        unsafe { manager.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        Self {
            manager,
            _delegate: delegate,
        }
    }

    fn set_accuracy(&self, accuracy: LocationAccuracy) {
        // SAFETY: reading CoreLocation's constants and setting a property on the main thread.
        unsafe {
            let desired = match accuracy {
                LocationAccuracy::High => kCLLocationAccuracyBest,
                LocationAccuracy::Coarse => kCLLocationAccuracyHundredMeters,
            };
            self.manager.setDesiredAccuracy(desired);
        }
    }
}

impl SourceState {
    /// Hands `result` to the hub. The sink is cloned out first: the hub may call `stop`.
    fn deliver(&self, result: Result<LocationFix, LocationError>) {
        let sink = self.sink.borrow().clone();
        if let Some(sink) = sink {
            sink.deliver(result);
        }
    }

    /// Acts on the app's authorisation, while started.
    fn follow_authorization(&self, manager: &CLLocationManager) {
        if self.sink.borrow().is_none() {
            return;
        }
        // SAFETY: a property read on the main thread.
        let status = unsafe { manager.authorizationStatus() };
        match authorization_step(status) {
            AuthorizationStep::Update => {
                if !self.updating.replace(true) {
                    info!("geolocation: CoreLocation is authorised; updating the location");
                    // SAFETY: on the main thread, with a delegate set.
                    unsafe { manager.startUpdatingLocation() };
                }
            }
            AuthorizationStep::Ask => {
                info!("geolocation: asking macOS for location access");
                // SAFETY: on the main thread; the bundle's Info.plist carries the usage strings.
                unsafe { manager.requestWhenInUseAuthorization() };
            }
            AuthorizationStep::Refuse => {
                info!("geolocation: macOS denies Mercury Browser location access");
                self.deliver(Err(LocationError::PermissionDenied));
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AuthorizationStep {
    Update,
    Ask,
    Refuse,
}

/// Denied covers both the user refusing Mercury and Location Services being off.
fn authorization_step(status: CLAuthorizationStatus) -> AuthorizationStep {
    match status {
        CLAuthorizationStatus::AuthorizedAlways | CLAuthorizationStatus::AuthorizedWhenInUse => {
            AuthorizationStep::Update
        }
        CLAuthorizationStatus::NotDetermined => AuthorizationStep::Ask,
        _ => AuthorizationStep::Refuse,
    }
}

/// `None` for `kCLErrorLocationUnknown`: CoreLocation has no position yet and keeps trying, so,
/// as Apple advises, it is ignored; the page's own `timeout` decides how long to wait.
fn location_error(is_core_location_error: bool, code: isize) -> Option<LocationError> {
    match (is_core_location_error, code) {
        (true, code) if code == CLError::LocationUnknown.0 => None,
        (true, code) if code == CLError::Denied.0 => Some(LocationError::PermissionDenied),
        _ => Some(LocationError::Unavailable),
    }
}

/// A reading as pages see it; `None` for a reading CoreLocation marks invalid (negative
/// horizontal accuracy) or that is off the globe. Negative vertical accuracy, course or speed
/// mean "unknown".
fn fix_from_reading(
    latitude: f64,
    longitude: f64,
    horizontal_accuracy: f64,
    altitude: f64,
    vertical_accuracy: f64,
    course: f64,
    speed: f64,
) -> Option<LocationFix> {
    let valid = horizontal_accuracy.is_finite()
        && horizontal_accuracy >= 0.0
        && (-90.0..=90.0).contains(&latitude)
        && (-180.0..=180.0).contains(&longitude);
    if !valid {
        return None;
    }
    let altitude_known = vertical_accuracy.is_finite() && vertical_accuracy > 0.0;
    Some(LocationFix {
        latitude,
        longitude,
        accuracy: horizontal_accuracy,
        altitude: altitude_known.then_some(altitude),
        altitude_accuracy: altitude_known.then_some(vertical_accuracy),
        heading: (course.is_finite() && course >= 0.0).then_some(course),
        speed: (speed.is_finite() && speed >= 0.0).then_some(speed),
    })
}

fn fix_from_location(location: &CLLocation) -> Option<LocationFix> {
    // SAFETY: property reads of an immutable CLLocation.
    unsafe {
        let coordinate = location.coordinate();
        fix_from_reading(
            coordinate.latitude,
            coordinate.longitude,
            location.horizontalAccuracy(),
            location.altitude(),
            location.verticalAccuracy(),
            location.course(),
            location.speed(),
        )
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MercuryLocationManagerDelegate"]
    #[ivars = Weak<SourceState>]
    struct LocationDelegate;

    unsafe impl NSObjectProtocol for LocationDelegate {}

    unsafe impl CLLocationManagerDelegate for LocationDelegate {
        #[unsafe(method(locationManager:didUpdateLocations:))]
        fn did_update_locations(&self, _manager: &CLLocationManager, locations: &NSArray<CLLocation>) {
            let Some(state) = self.ivars().upgrade() else {
                return;
            };
            if let Some(fix) = locations.lastObject().as_deref().and_then(fix_from_location) {
                state.deliver(Ok(fix));
            }
        }

        #[unsafe(method(locationManager:didFailWithError:))]
        fn did_fail(&self, _manager: &CLLocationManager, error: &NSError) {
            let Some(state) = self.ivars().upgrade() else {
                return;
            };
            // SAFETY: reading CoreLocation's error domain constant.
            let is_core_location_error = *error.domain() == *unsafe { kCLErrorDomain };
            let code = error.code();
            info!("geolocation: CoreLocation reported error {code}");
            if let Some(error) = location_error(is_core_location_error, code) {
                state.deliver(Err(error));
            }
        }

        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        fn did_change_authorization(&self, manager: &CLLocationManager) {
            if let Some(state) = self.ivars().upgrade() {
                state.follow_authorization(manager);
            }
        }
    }
);

impl LocationDelegate {
    fn new(mtm: MainThreadMarker, state: Weak<SourceState>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(state);
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorised_apps_update_undetermined_ones_ask_and_the_rest_are_refused() {
        assert_eq!(
            authorization_step(CLAuthorizationStatus::AuthorizedWhenInUse),
            AuthorizationStep::Update
        );
        assert_eq!(
            authorization_step(CLAuthorizationStatus::AuthorizedAlways),
            AuthorizationStep::Update
        );
        assert_eq!(
            authorization_step(CLAuthorizationStatus::NotDetermined),
            AuthorizationStep::Ask
        );
        assert_eq!(
            authorization_step(CLAuthorizationStatus::Denied),
            AuthorizationStep::Refuse
        );
        assert_eq!(
            authorization_step(CLAuthorizationStatus::Restricted),
            AuthorizationStep::Refuse
        );
        assert_eq!(
            authorization_step(CLAuthorizationStatus(42)),
            AuthorizationStep::Refuse
        );
    }

    #[test]
    fn only_core_locations_denied_error_is_a_permission_error() {
        assert_eq!(
            location_error(true, CLError::Denied.0),
            Some(LocationError::PermissionDenied)
        );
        assert_eq!(
            location_error(true, CLError::Network.0),
            Some(LocationError::Unavailable)
        );
        assert_eq!(
            location_error(false, CLError::Denied.0),
            Some(LocationError::Unavailable)
        );
    }

    #[test]
    fn location_unknown_is_waited_out() {
        assert_eq!(location_error(true, CLError::LocationUnknown.0), None);
        assert_eq!(
            location_error(false, CLError::LocationUnknown.0),
            Some(LocationError::Unavailable)
        );
    }

    #[test]
    fn a_full_reading_keeps_every_field() {
        let fix = fix_from_reading(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0).expect("valid");

        assert!(
            fix == LocationFix {
                latitude: 1.0,
                longitude: 2.0,
                accuracy: 3.0,
                altitude: Some(4.0),
                altitude_accuracy: Some(5.0),
                heading: Some(6.0),
                speed: Some(7.0),
            }
        );
    }

    #[test]
    fn negative_extras_are_unknown() {
        let fix = fix_from_reading(1.0, 2.0, 3.0, 4.0, -1.0, -1.0, -1.0).expect("valid");

        assert_eq!(fix.altitude, None);
        assert_eq!(fix.altitude_accuracy, None);
        assert_eq!(fix.heading, None);
        assert_eq!(fix.speed, None);
    }

    #[test]
    fn invalid_readings_are_dropped() {
        assert!(fix_from_reading(1.0, 2.0, -1.0, 0.0, 0.0, 0.0, 0.0).is_none());
        assert!(fix_from_reading(1.0, 2.0, f64::NAN, 0.0, 0.0, 0.0, 0.0).is_none());
        assert!(fix_from_reading(91.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0).is_none());
        assert!(fix_from_reading(1.0, 181.0, 3.0, 0.0, 0.0, 0.0, 0.0).is_none());
    }
}
