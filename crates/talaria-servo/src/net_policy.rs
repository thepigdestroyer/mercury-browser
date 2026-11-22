// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The engine's network policy, set from Mercury's privacy settings (net lane: EC5, EC6, EC14, EC25).
//!
//! The patched network crate (`third_party/servo-net/net_policy.rs`) has one handle, [`NetPolicy`], that says
//! what the network layer allows: whether the cookies of third-party requests are withheld, whether
//! `SameSite` is enforced, whether the referrer is capped, whether `Sec-GPC: 1` goes out. Its `new()` behaves
//! as Servo 0.8 did; the one process-wide instance ([`NetPolicy::process`]) is what both of Servo's HTTP states
//! (public and private) use, so setting it once, before the engine starts, configures every request.
//!
//! * [`NetPolicySetup`] is what the browser decides at start: the two privacy settings. [`install`] applies it
//!   once, from `ServoRuntime::new`, and sets everything else Mercury ships on the handle: `SameSite`
//!   enforced, Lax-by-default **as the engine preference says** (pinned off, owner decision OD-6), the
//!   referrer capped.
//! * Global Privacy Control has two outputs that must agree: the `Sec-GPC` request header (the handle) and
//!   `navigator.globalPrivacyControl` (the preference `privacy_gpc_enabled`, which content processes copy at
//!   their start). [`install`] sets both from the one value, and a test pins that they cannot differ. So
//!   Global Privacy Control is read **at start**; a later change of the setting applies at the next start.
//! * Third-party cookie blocking is also **live**: [`EngineSwitches::set_block_third_party_cookies`]
//!   (`exit_clearing.rs`) and [`NetPolicyControl`] flip the handle, and the next request sees it.
//!
//! The handle is read by the network thread of the browser process, which is where it lives in the
//! multi-process build too (content processes ask it through messages), so a change needs no message.
//!
//! [`EngineSwitches::set_block_third_party_cookies`]: crate::EngineSwitches::set_block_third_party_cookies

use servo::{NetPolicy, NetPolicyStats, Preferences, ServoUrl};
use url::Url;

/// What the browser decides about the network policy when it starts the engine. The default is the owner's
/// "Standard + GPC on + third-party cookies blocked" (decision O-18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetPolicySetup {
    /// Withhold the cookies of third-party requests, and ignore the `Set-Cookie` of their answers.
    pub block_third_party_cookies: bool,
    /// Send `Sec-GPC: 1` and expose `navigator.globalPrivacyControl` as true.
    pub global_privacy_control: bool,
}

impl Default for NetPolicySetup {
    fn default() -> Self {
        Self {
            block_third_party_cookies: true,
            global_privacy_control: true,
        }
    }
}

/// Sets the process-wide policy from `setup`, and `preferences.privacy_gpc_enabled` from the same value.
/// Call once, after the engine preferences are built (so that this has the last word on Global Privacy
/// Control) and before the engine starts.
pub(crate) fn install(setup: &NetPolicySetup, preferences: &mut Preferences) {
    install_into(&NetPolicy::process(), setup, preferences);
}

/// [`install`] for a policy of the caller's choice (the tests use one of their own).
pub(crate) fn install_into(
    policy: &NetPolicy,
    setup: &NetPolicySetup,
    preferences: &mut Preferences,
) {
    policy.set_block_third_party_cookies(setup.block_third_party_cookies);
    policy.set_same_site_enforcement(true);
    // Lax-by-default stays off (the preference is pinned off in `engine_prefs.rs`); whoever flips the pin
    // gets the handle with it.
    policy.set_same_site_lax_default(preferences.network_cookie_samesite_lax_default);
    policy.set_referrer_cap(true);
    // The header and `navigator.globalPrivacyControl` come from one value.
    policy.set_global_privacy_control(setup.global_privacy_control);
    preferences.privacy_gpc_enabled = setup.global_privacy_control;
}

/// The network policy as the browser controls it while the engine runs: a handle on the same state the
/// network thread reads. Clones share it.
#[derive(Clone, Debug)]
pub struct NetPolicyControl {
    policy: NetPolicy,
}

impl NetPolicyControl {
    /// A control for the process-wide policy, the one the engine uses.
    pub fn process() -> Self {
        Self::of(NetPolicy::process())
    }

    pub(crate) fn of(policy: NetPolicy) -> Self {
        Self { policy }
    }

    /// Blocks (or stops blocking) the cookies of third-party requests, for every request from now on.
    pub fn set_block_third_party_cookies(&self, on: bool) {
        self.policy.set_block_third_party_cookies(on);
    }

    pub fn block_third_party_cookies(&self) -> bool {
        self.policy.block_third_party_cookies()
    }

    /// The site the engine files `url` under (its registrable domain, in lower case; an address or a
    /// single-label host is its own site), or `None` for a URL with no site (an opaque origin, such as a
    /// `data:` URL).
    pub fn site_of(&self, url: &Url) -> Option<String> {
        let sites = self
            .policy
            .sites_of_document(&ServoUrl::from_url(url.clone()), &None);
        Some(sites.request).filter(|site| !site.is_empty())
    }

    /// Lets the page at `page` (and every page of its site) have third-party cookies, or takes the
    /// permission back. Does nothing for a URL with no site.
    pub fn allow_third_party_cookies_on(&self, page: &Url, allow: bool) {
        if let Some(site) = self.site_of(page) {
            self.policy.allow_third_party_cookies_on(site, allow);
        }
    }

    /// Lets the page at `page` (and every page of its site) load mixed content, or takes the permission
    /// back. Does nothing for a URL with no site.
    pub fn set_mixed_content_allowed_on(&self, page: &Url, allow: bool) {
        if let Some(site) = self.site_of(page) {
            self.policy.set_mixed_content_allowed_on(site, allow);
        }
    }

    /// What the policy has done so far.
    pub fn stats(&self) -> NetPolicyStats {
        self.policy.stats()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// What the engine preferences are when Mercury has applied its own (`engine_prefs::apply`).
    fn mercury_preferences() -> Preferences {
        let mut preferences = Preferences::default();
        crate::engine_prefs::apply(&mut preferences);
        preferences
    }

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a URL")
    }

    #[test]
    fn the_default_setup_is_the_owners_standard_plus_gpc_plus_blocked_third_party_cookies() {
        let setup = NetPolicySetup::default();
        assert!(setup.block_third_party_cookies);
        assert!(setup.global_privacy_control);
        assert_eq!(crate::RuntimeOptions::default().net_policy, setup);
    }

    #[test]
    fn a_fresh_profile_has_the_owner_defaults_in_the_engine() {
        // The acceptance test of the net lane: what a fresh profile's engine reads, read back.
        let policy = NetPolicy::new();
        let mut preferences = mercury_preferences();
        install_into(&policy, &NetPolicySetup::default(), &mut preferences);

        assert!(
            policy.block_third_party_cookies(),
            "third-party cookies blocked"
        );
        assert!(policy.global_privacy_control(), "Sec-GPC on");
        assert!(
            preferences.privacy_gpc_enabled,
            "navigator.globalPrivacyControl on"
        );
        assert!(policy.referrer_cap(), "referrer cap on");
        assert!(policy.same_site_enforcement(), "SameSite enforced");
        assert!(!policy.same_site_lax_default(), "Lax-by-default off (OD-6)");
        assert!(!preferences.network_cookie_samesite_lax_default);
        assert!(
            !preferences.dom_cookiestore_enabled,
            "cookieStore pinned off"
        );
        assert!(
            !preferences.dom_web_locks_enabled,
            "Web Locks pinned off until a page run"
        );
    }

    #[test]
    fn install_sets_what_it_says() {
        let policy = NetPolicy::new();
        let mut preferences = mercury_preferences();
        let setup = NetPolicySetup {
            block_third_party_cookies: false,
            global_privacy_control: false,
        };
        install_into(&policy, &setup, &mut preferences);

        assert!(!policy.block_third_party_cookies());
        assert!(!policy.global_privacy_control());
        assert!(!preferences.privacy_gpc_enabled);
        // The settings do not touch these.
        assert!(policy.same_site_enforcement());
        assert!(policy.referrer_cap());
        assert!(!policy.same_site_lax_default());
    }

    #[test]
    fn the_header_and_navigator_global_privacy_control_cannot_disagree() {
        // Whatever the preference held before (the engine preferences set it on), the one value in the setup
        // decides both.
        for global_privacy_control in [true, false] {
            for before in [true, false] {
                for block in [true, false] {
                    let policy = NetPolicy::new();
                    let mut preferences = mercury_preferences();
                    preferences.privacy_gpc_enabled = before;
                    install_into(
                        &policy,
                        &NetPolicySetup {
                            block_third_party_cookies: block,
                            global_privacy_control,
                        },
                        &mut preferences,
                    );
                    assert_eq!(policy.global_privacy_control(), global_privacy_control);
                    assert_eq!(
                        preferences.privacy_gpc_enabled,
                        policy.global_privacy_control(),
                        "the attribute and the header agree (setting {global_privacy_control}, \
                         preference before {before})"
                    );
                }
            }
        }
    }

    #[test]
    fn lax_by_default_follows_the_engine_preference() {
        let policy = NetPolicy::new();
        let mut preferences = mercury_preferences();
        preferences.network_cookie_samesite_lax_default = true;
        install_into(&policy, &NetPolicySetup::default(), &mut preferences);
        assert!(policy.same_site_lax_default());

        preferences.network_cookie_samesite_lax_default = false;
        install_into(&policy, &NetPolicySetup::default(), &mut preferences);
        assert!(!policy.same_site_lax_default());
    }

    #[test]
    fn installing_twice_changes_nothing() {
        let policy = NetPolicy::new();
        let mut once = mercury_preferences();
        install_into(&policy, &NetPolicySetup::default(), &mut once);
        let mut twice = once.clone();
        install_into(&policy, &NetPolicySetup::default(), &mut twice);
        assert!(twice.diff(&once).is_empty());
        assert!(policy.block_third_party_cookies() && policy.global_privacy_control());
    }

    /// Both policies named below are the process-wide one: the tests that touch it take turns.
    static PROCESS_POLICY: Mutex<()> = Mutex::new(());

    #[test]
    fn install_configures_the_process_policy_that_the_engine_uses() {
        let _turn = PROCESS_POLICY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let process = NetPolicy::process();
        let (block_before, gpc_before) = (
            process.block_third_party_cookies(),
            process.global_privacy_control(),
        );

        let mut preferences = mercury_preferences();
        install(
            &NetPolicySetup {
                block_third_party_cookies: true,
                global_privacy_control: false,
            },
            &mut preferences,
        );
        assert!(process.block_third_party_cookies());
        assert!(!process.global_privacy_control());
        assert!(!preferences.privacy_gpc_enabled);
        assert!(NetPolicyControl::process().block_third_party_cookies());

        process.set_block_third_party_cookies(block_before);
        process.set_global_privacy_control(gpc_before);
    }

    #[test]
    fn the_control_flips_the_live_switch_for_every_clone() {
        let policy = NetPolicy::new();
        let control = NetPolicyControl::of(policy.clone());
        let copy = control.clone();
        assert!(!control.block_third_party_cookies());

        control.set_block_third_party_cookies(true);
        assert!(policy.block_third_party_cookies());
        assert!(copy.block_third_party_cookies());

        copy.set_block_third_party_cookies(false);
        assert!(!policy.block_third_party_cookies());
        assert_eq!(control.stats(), NetPolicyStats::default());
    }

    #[test]
    fn the_site_of_a_url_is_its_registrable_domain() {
        let control = NetPolicyControl::of(NetPolicy::new());
        let site = |text: &str| control.site_of(&url(text));

        assert_eq!(site("https://example.com/"), Some("example.com".to_owned()));
        assert_eq!(
            site("https://www.Example.com:8443/a?b#c"),
            Some("example.com".to_owned())
        );
        assert_eq!(
            site("http://news.bbc.co.uk/"),
            Some("bbc.co.uk".to_owned()),
            "a public suffix of two labels"
        );
        // An address or a single-label host is its own site.
        assert_eq!(site("http://localhost:3000/"), Some("localhost".to_owned()));
        assert_eq!(
            site("http://192.168.1.10/"),
            Some("192.168.1.10".to_owned())
        );
        // The scheme is not part of it: the site of `http://a.example` is the site of `https://a.example`.
        assert_eq!(site("http://a.example/"), site("https://a.example/"));
        // No site: nothing to allow.
        assert_eq!(site("data:text/plain,hello"), None);
        assert_eq!(site("about:blank"), None);
    }

    #[test]
    fn allowing_a_page_without_a_site_does_nothing_and_does_not_panic() {
        let control = NetPolicyControl::of(NetPolicy::new());
        control.allow_third_party_cookies_on(&url("data:text/plain,hello"), true);
        control.set_mixed_content_allowed_on(&url("about:blank"), true);
        control.allow_third_party_cookies_on(&url("https://example.com/"), true);
        control.allow_third_party_cookies_on(&url("https://example.com/"), false);
        control.set_mixed_content_allowed_on(&url("https://example.com/"), true);
        control.set_mixed_content_allowed_on(&url("https://example.com/"), false);
    }
}
