//! The geolocation diagnostic: `MERCURY_FAKE_GEOLOCATION` turns `navigator.geolocation` on
//! with a fake location source and answers the page's permission request, so automated runs
//! can check the whole path without a prompt and without the Mac's real location.
//!
//! - `allow:<latitude>,<longitude>,<accuracy in metres>`: grant; the page gets this position.
//! - `allow:denied-by-system` / `allow:unavailable` / `allow:timeout`: grant; the source
//!   reports that error, as macOS refusing access, a failed fix or a time-out would.
//! - `deny`: refuse the permission, as the user would in the prompt.
//!
//! Honoured only in `--screenshot` and `--background` runs. Otherwise geolocation stays off.

use talaria_servo::{
    FakeLocation, FixedPosition, GeolocationDiagnostic, GeolocationSetup, LocationError,
};

const FAKE_GEOLOCATION_VARIABLE: &str = "MERCURY_FAKE_GEOLOCATION";

/// This run's geolocation setup, from the environment.
pub(crate) fn setup_for_this_run(automated: bool) -> GeolocationSetup {
    let variable = std::env::var(FAKE_GEOLOCATION_VARIABLE).ok();
    match setup(automated, variable.as_deref()) {
        Ok(setup) => {
            if setup != GeolocationSetup::Off {
                log::warn!(
                    "{FAKE_GEOLOCATION_VARIABLE}: pages get a fake location in this automated run"
                );
            }
            setup
        }
        Err(message) => {
            log::warn!("{FAKE_GEOLOCATION_VARIABLE} is ignored: {message}");
            GeolocationSetup::Off
        }
    }
}

/// Geolocation stays off until the permission prompt can ask about it (see the report of the
/// geolocation lane for what to change when it does).
fn setup(automated: bool, variable: Option<&str>) -> Result<GeolocationSetup, String> {
    let Some(variable) = variable else {
        return Ok(GeolocationSetup::Off);
    };
    if !automated {
        return Err("only --screenshot and --background runs use it".to_owned());
    }
    let diagnostic = match variable.trim() {
        "deny" => GeolocationDiagnostic::Deny,
        "allow:denied-by-system" => {
            GeolocationDiagnostic::Allow(FakeLocation::Error(LocationError::PermissionDenied))
        }
        "allow:unavailable" => {
            GeolocationDiagnostic::Allow(FakeLocation::Error(LocationError::Unavailable))
        }
        "allow:timeout" => {
            GeolocationDiagnostic::Allow(FakeLocation::Error(LocationError::Timeout))
        }
        other => {
            let position = other
                .strip_prefix("allow:")
                .and_then(parse_position)
                .ok_or_else(|| format!("'{other}' is not deny, allow:<latitude>,<longitude>,<accuracy> or allow:<error>"))?;
            GeolocationDiagnostic::Allow(FakeLocation::Position(position))
        }
    };
    Ok(GeolocationSetup::Diagnostic(diagnostic))
}

fn parse_position(text: &str) -> Option<FixedPosition> {
    let mut numbers = text.split(',').map(|number| number.trim().parse::<f64>());
    let (Some(Ok(latitude)), Some(Ok(longitude)), Some(Ok(accuracy)), None) = (
        numbers.next(),
        numbers.next(),
        numbers.next(),
        numbers.next(),
    ) else {
        return None;
    };
    FixedPosition::new(latitude, longitude, accuracy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow(position: (f64, f64, f64)) -> GeolocationSetup {
        GeolocationSetup::Diagnostic(GeolocationDiagnostic::Allow(FakeLocation::Position(
            FixedPosition::new(position.0, position.1, position.2).expect("valid"),
        )))
    }

    #[test]
    fn without_the_variable_geolocation_is_off() {
        assert_eq!(setup(true, None), Ok(GeolocationSetup::Off));
        assert_eq!(setup(false, None), Ok(GeolocationSetup::Off));
    }

    #[test]
    fn the_variable_counts_only_in_automated_runs() {
        assert!(setup(false, Some("allow:1,2,3")).is_err());
        assert!(setup(false, Some("deny")).is_err());
    }

    #[test]
    fn a_position_is_granted_with_that_position() {
        assert_eq!(
            setup(true, Some("allow:51.5007,-0.1246,25")),
            Ok(allow((51.5007, -0.1246, 25.0)))
        );
        assert_eq!(
            setup(true, Some(" allow: 1 , 2 , 0 ")),
            Ok(allow((1.0, 2.0, 0.0)))
        );
    }

    #[test]
    fn deny_refuses_the_permission() {
        assert_eq!(
            setup(true, Some("deny")),
            Ok(GeolocationSetup::Diagnostic(GeolocationDiagnostic::Deny))
        );
    }

    #[test]
    fn source_errors_are_granted_and_then_reported() {
        for (text, error) in [
            ("allow:denied-by-system", LocationError::PermissionDenied),
            ("allow:unavailable", LocationError::Unavailable),
            ("allow:timeout", LocationError::Timeout),
        ] {
            assert_eq!(
                setup(true, Some(text)),
                Ok(GeolocationSetup::Diagnostic(GeolocationDiagnostic::Allow(
                    FakeLocation::Error(error)
                ))),
                "{text}"
            );
        }
    }

    #[test]
    fn malformed_values_are_refused() {
        for text in [
            "",
            "allow",
            "allow:",
            "allow:1,2",
            "allow:1,2,3,4",
            "allow:a,2,3",
            "allow:91,0,1",
            "allow:0,181,1",
            "allow:0,0,-1",
            "allow:0,0,inf",
            "grant:1,2,3",
        ] {
            assert!(setup(true, Some(text)).is_err(), "{text}");
        }
    }
}
