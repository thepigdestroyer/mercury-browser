// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! One answer to "which proxy does Mercury use?", shared by Servo and every other HTTP client.
//!
//! Servo gets its proxy from three preferences (`network_http_proxy_uri`,
//! `network_https_proxy_uri`, `network_http_no_proxy`). Their value is decided in one place,
//! [`choose`]: the `http_proxy`, `https_proxy` and `no_proxy` environment variables (and their
//! upper-case forms) if any is set, otherwise the system's settings (macOS: System Settings >
//! Network > Proxies, read once at start). `talaria-servo` uses [`choose`] for the preferences;
//! the desktop app calls [`install`] once at start with the same inputs, so that a client that is
//! not Servo, such as the download engine that re-fetches a PDF for the viewer, asks
//! [`process_preferences`] (or just [`apply_process_proxy`]) and sends its request through the
//! same proxy with the same bypass list. A request that Servo would send through a proxy must
//! never go out directly because a second client was built without one.
//!
//! A reqwest client that talks to the web for the user adopts it with one line:
//!
//! ```ignore
//! let builder = mercury_proxy::apply_process_proxy(reqwest::Client::builder())?;
//! ```

use std::sync::OnceLock;

mod system;

pub use system::{ProxyPreferences, ProxyServer, SystemProxySettings};

/// The proxy variables as Servo reads them (`servo-config` `Preferences::default`): the lower-case
/// name first, then the upper-case one. `lookup` returns a variable's value, if it is set.
pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> ProxyPreferences {
    let variable = |lower: &str, upper: &str| lookup(lower).or_else(|| lookup(upper));
    ProxyPreferences {
        http_uri: variable("http_proxy", "HTTP_PROXY").unwrap_or_default(),
        https_uri: variable("https_proxy", "HTTPS_PROXY").unwrap_or_default(),
        no_proxy: variable("no_proxy", "NO_PROXY").unwrap_or_default(),
    }
}

/// The proxy environment variables of this process.
pub fn from_environment() -> ProxyPreferences {
    from_lookup(|name| std::env::var(name).ok())
}

/// The proxy to use: `environment` if any of its three values is set (as in command-line tools,
/// the variables win and the system settings are not looked at), otherwise what the system
/// settings say, or nothing.
pub fn choose(
    environment: ProxyPreferences,
    system: Option<&SystemProxySettings>,
) -> ProxyPreferences {
    if environment != ProxyPreferences::default() {
        return environment;
    }
    system
        .map(SystemProxySettings::preferences)
        .unwrap_or_default()
}

static PROCESS: OnceLock<ProxyPreferences> = OnceLock::new();

/// Records the proxy of this process, once, at start: [`choose`] of the environment variables and
/// `system`. Returns what was recorded. A second call changes nothing and returns the first
/// call's value, so every client sees one proxy for the whole run.
pub fn install(system: Option<&SystemProxySettings>) -> ProxyPreferences {
    install_preferences(choose(from_environment(), system))
}

/// [`install`] with the proxy given; for tests of the clients that use [`process_preferences`].
pub fn install_preferences(preferences: ProxyPreferences) -> ProxyPreferences {
    PROCESS.get_or_init(|| preferences).clone()
}

/// The proxy of this process: what [`install`] recorded, or, when nothing was installed (tests,
/// tools), the proxy environment variables alone, which is what Servo would use without system
/// settings.
pub fn process_preferences() -> ProxyPreferences {
    PROCESS.get().cloned().unwrap_or_else(from_environment)
}

/// Makes `builder` send its requests the way Servo sends them with `preferences`: `http://`
/// requests through the HTTP proxy, `https://` requests through the HTTPS proxy (a `CONNECT`
/// tunnel, so the proxy sees the host name and nothing else), except for hosts on the bypass
/// list; with no proxy set, directly. Whatever reqwest would read from the environment itself is
/// switched off first, so the answer is the same on every platform and the bypass list has
/// Servo's syntax (`hyper-util`'s).
///
/// Fails, and so the client is not built, when a proxy address cannot be used: a request that
/// should go through a proxy must not go out directly.
pub fn apply(
    builder: reqwest::ClientBuilder,
    preferences: &ProxyPreferences,
) -> reqwest::Result<reqwest::ClientBuilder> {
    let mut builder = builder.no_proxy();
    let bypass = reqwest::NoProxy::from_string(&preferences.no_proxy);
    if !preferences.http_uri.is_empty() {
        builder =
            builder.proxy(reqwest::Proxy::http(&preferences.http_uri)?.no_proxy(bypass.clone()));
    }
    if !preferences.https_uri.is_empty() {
        builder = builder.proxy(reqwest::Proxy::https(&preferences.https_uri)?.no_proxy(bypass));
    }
    Ok(builder)
}

/// [`apply`] with [`process_preferences`].
pub fn apply_process_proxy(
    builder: reqwest::ClientBuilder,
) -> reqwest::Result<reqwest::ClientBuilder> {
    apply(builder, &process_preferences())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(variables: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let variables: HashMap<String, String> = variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name| variables.get(name).cloned()
    }

    fn system() -> SystemProxySettings {
        SystemProxySettings {
            http: Some(ProxyServer::new("proxy.example", Some(3128))),
            exceptions: vec!["*.local".to_owned()],
            ..SystemProxySettings::default()
        }
    }

    #[test]
    fn the_variables_are_read_as_servo_reads_them() {
        let preferences = from_lookup(lookup(&[
            ("http_proxy", "http://lower.example:1"),
            ("HTTP_PROXY", "http://upper.example:1"),
            ("HTTPS_PROXY", "http://upper.example:2"),
            ("NO_PROXY", "internal"),
        ]));

        assert_eq!(
            preferences,
            ProxyPreferences {
                http_uri: "http://lower.example:1".to_owned(),
                https_uri: "http://upper.example:2".to_owned(),
                no_proxy: "internal".to_owned(),
            }
        );
        assert_eq!(from_lookup(lookup(&[])), ProxyPreferences::default());
    }

    #[test]
    fn the_system_proxy_is_used_when_no_variable_is_set() {
        let preferences = choose(ProxyPreferences::default(), Some(&system()));

        assert_eq!(preferences.http_uri, "http://proxy.example:3128");
        assert_eq!(preferences.no_proxy, "localhost,127.0.0.0/8,::1,.local");
    }

    #[test]
    fn no_variable_and_no_system_settings_mean_no_proxy() {
        assert_eq!(
            choose(ProxyPreferences::default(), None),
            ProxyPreferences::default()
        );
        assert_eq!(
            choose(
                ProxyPreferences::default(),
                Some(&SystemProxySettings::default())
            ),
            ProxyPreferences::default()
        );
    }

    #[test]
    fn any_one_variable_wins_over_the_system_settings_whole() {
        for environment in [
            ProxyPreferences {
                http_uri: "http://env.example:1".to_owned(),
                ..ProxyPreferences::default()
            },
            ProxyPreferences {
                https_uri: "http://env.example:2".to_owned(),
                ..ProxyPreferences::default()
            },
            ProxyPreferences {
                no_proxy: "env.example".to_owned(),
                ..ProxyPreferences::default()
            },
        ] {
            assert_eq!(choose(environment.clone(), Some(&system())), environment);
        }
    }

    #[test]
    fn an_unusable_proxy_address_stops_the_client_instead_of_going_direct() {
        let preferences = ProxyPreferences {
            http_uri: "http://".to_owned(),
            ..ProxyPreferences::default()
        };

        assert!(apply(reqwest::Client::builder(), &preferences).is_err());
    }

    #[test]
    fn no_proxy_builds_a_client() {
        let builder = apply(reqwest::Client::builder(), &ProxyPreferences::default())
            .expect("no proxy is fine");
        builder.build().expect("a client");
    }
}
