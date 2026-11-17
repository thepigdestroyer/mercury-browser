//! Positions for `navigator.geolocation`.
//!
//! Servo asks the embedder for a position only after the page holds the "geolocation"
//! permission (`servo-script` 0.8.0, `dom/geolocation/geolocation.rs`: `request_permission_to_use_async`
//! first, then `RequestGeolocationPosition` / `StartGeolocationWatch`). Every web view gets a
//! `WebViewGeolocation` (`delegate.rs`) that hands those requests to one `LocationHub`
//! (`hub.rs`) per browser, which runs a single `LocationSource` while any page waits for or
//! watches the position, and stops it when none does.
//!
//! Sources: CoreLocation on macOS (`core_location.rs`), none on other platforms (pages get a
//! permission-denied error), and a fixed fake (`fake.rs`) for tests and the diagnostic run.
//!
//! Privacy: positions are never logged. `LocationFix` has no derived `Debug`, and nothing here
//! formats its fields into a log line.

#[cfg(target_os = "macos")]
mod core_location;
mod delegate;
mod fake;
mod hub;

use std::fmt;
use std::rc::{Rc, Weak};

pub(crate) use delegate::{GeolocationService, WebViewGeolocation, WithGeolocation};

/// How Mercury answers `navigator.geolocation` in this run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GeolocationSetup {
    /// `navigator.geolocation` is absent (Servo's `dom_geolocation_enabled` is off) and no
    /// location source exists.
    #[default]
    Off,
    /// `navigator.geolocation` is present; positions come from the system's location service
    /// once the user has allowed the page in Mercury's permission prompt.
    System,
    /// For automated checks: `navigator.geolocation` is present, Servo's permission request
    /// for geolocation is answered without a prompt, and positions come from a fake source.
    Diagnostic(GeolocationDiagnostic),
}

impl GeolocationSetup {
    /// Whether pages see `navigator.geolocation` (Servo's `dom_geolocation_enabled`).
    pub fn exposes_geolocation(&self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// The diagnostic's answer to the page's permission request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeolocationDiagnostic {
    /// Refuse the permission, as a user would in the prompt.
    Deny,
    /// Grant the permission; the fake source answers with this.
    Allow(FakeLocation),
}

/// What the fake source reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeLocation {
    Position(FixedPosition),
    Error(LocationError),
}

/// A fixed, made-up position. Compared bit for bit so that setups can be `Eq`.
#[derive(Clone, Copy)]
pub struct FixedPosition {
    pub latitude: f64,
    pub longitude: f64,
    /// Metres.
    pub accuracy: f64,
}

impl FixedPosition {
    /// `None` unless latitude is in -90..=90, longitude in -180..=180 and the accuracy is a
    /// finite number of metres, not negative.
    pub fn new(latitude: f64, longitude: f64, accuracy: f64) -> Option<Self> {
        let valid = (-90.0..=90.0).contains(&latitude)
            && (-180.0..=180.0).contains(&longitude)
            && accuracy.is_finite()
            && accuracy >= 0.0;
        valid.then_some(Self {
            latitude,
            longitude,
            accuracy,
        })
    }

    fn fix(self) -> LocationFix {
        LocationFix {
            latitude: self.latitude,
            longitude: self.longitude,
            accuracy: self.accuracy,
            altitude: None,
            altitude_accuracy: None,
            heading: None,
            speed: None,
        }
    }
}

impl PartialEq for FixedPosition {
    fn eq(&self, other: &Self) -> bool {
        self.latitude.to_bits() == other.latitude.to_bits()
            && self.longitude.to_bits() == other.longitude.to_bits()
            && self.accuracy.to_bits() == other.accuracy.to_bits()
    }
}

impl Eq for FixedPosition {}

impl fmt::Debug for FixedPosition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FixedPosition { .. }")
    }
}

/// One position reading. Never logged: its `Debug` prints no fields.
#[derive(Clone, PartialEq)]
pub(crate) struct LocationFix {
    /// Degrees north of the Equator.
    pub latitude: f64,
    /// Degrees east of the Prime Meridian.
    pub longitude: f64,
    /// Metres, not negative.
    pub accuracy: f64,
    /// Metres above the WGS84 ellipsoid.
    pub altitude: Option<f64>,
    /// Metres, not negative.
    pub altitude_accuracy: Option<f64>,
    /// Degrees clockwise from true north.
    pub heading: Option<f64>,
    /// Metres per second, not negative.
    pub speed: Option<f64>,
}

impl fmt::Debug for LocationFix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LocationFix { .. }")
    }
}

/// Why a source has no position. Maps onto `GeolocationPositionError`'s codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocationError {
    /// The operating system refused (the user denied or restricted Mercury, or location
    /// services are off), or the platform has no location service.
    PermissionDenied,
    /// The location service failed or has no position right now.
    Unavailable,
    /// The source gave up waiting for a position.
    Timeout,
}

/// The accuracy a source is asked for: `High` while any page passed
/// `enableHighAccuracy: true`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocationAccuracy {
    Coarse,
    High,
}

/// A provider of positions. Called on the main thread only. A source may deliver through the
/// sink at any time between `start` and `stop`, including from inside `start`; deliveries
/// after `stop` are ignored.
pub(crate) trait LocationSource {
    /// Begin acquiring positions and report each one, or each failure, through `sink`.
    fn start(&self, accuracy: LocationAccuracy, sink: LocationSink);
    /// The pages' accuracy wish changed while running.
    fn set_accuracy(&self, accuracy: LocationAccuracy);
    /// Stop acquiring positions; nobody is waiting for one.
    fn stop(&self);
}

/// Something a sink delivers to: the hub.
pub(crate) trait LocationReceiver {
    fn receive(&self, generation: u64, result: Result<LocationFix, LocationError>);
}

/// Where a running source reports. Each `start` gets a new sink; an old one goes quiet.
#[derive(Clone)]
pub(crate) struct LocationSink {
    receiver: Weak<dyn LocationReceiver>,
    generation: u64,
}

impl LocationSink {
    pub(crate) fn new(receiver: Weak<dyn LocationReceiver>, generation: u64) -> Self {
        Self {
            receiver,
            generation,
        }
    }

    pub(crate) fn deliver(&self, result: Result<LocationFix, LocationError>) {
        if let Some(receiver) = self.receiver.upgrade() {
            receiver.receive(self.generation, result);
        }
    }
}

/// The source for `GeolocationSetup::System` on this platform. Created only when a page that
/// holds the permission first asks for a position.
fn system_source() -> Rc<dyn LocationSource> {
    #[cfg(target_os = "macos")]
    {
        Rc::new(core_location::CoreLocationSource::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Rc::new(fake::FakeLocationSource::answering(Err(
            LocationError::PermissionDenied,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geolocation_is_off_by_default() {
        assert_eq!(GeolocationSetup::default(), GeolocationSetup::Off);
        assert!(!GeolocationSetup::default().exposes_geolocation());
    }

    #[test]
    fn the_system_and_diagnostic_setups_expose_geolocation() {
        assert!(GeolocationSetup::System.exposes_geolocation());
        assert!(GeolocationSetup::Diagnostic(GeolocationDiagnostic::Deny).exposes_geolocation());
    }

    #[test]
    fn fixed_positions_must_be_on_the_globe() {
        assert!(FixedPosition::new(51.5, -0.12, 25.0).is_some());
        assert!(FixedPosition::new(90.0, 180.0, 0.0).is_some());
        assert!(FixedPosition::new(90.5, 0.0, 1.0).is_none());
        assert!(FixedPosition::new(0.0, -180.5, 1.0).is_none());
        assert!(FixedPosition::new(0.0, 0.0, -1.0).is_none());
        assert!(FixedPosition::new(f64::NAN, 0.0, 1.0).is_none());
        assert!(FixedPosition::new(0.0, 0.0, f64::INFINITY).is_none());
    }

    #[test]
    fn positions_print_no_coordinates() {
        let position = FixedPosition::new(51.5007, -0.1246, 25.0).expect("valid");
        let printed = format!("{position:?} {:?}", position.fix());

        assert!(!printed.contains("51"), "{printed}");
        assert!(!printed.contains("0.12"), "{printed}");
    }
}
