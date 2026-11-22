// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: sign-in prompts for the 401 of a `fetch` or XHR (EC29). This is a new Mercury file; nothing in it
// is copied from Servo or another project. The hooks are two calls in `HttpState::request_authentication`
// (`http_loader.rs`) and the `mod` lines in `lib.rs`; `MERCURY.md` lists them.

//! When may a 401 answered to `fetch` or XHR open a sign-in prompt (EC29, decision D12).
//!
//! Servo 0.8 prompts only for navigations: `HttpState::request_authentication` answers `None` for every
//! other request, so a page that calls `fetch("/api")` on a site behind Basic authentication gets its 401
//! back and the user is never asked. Browsers do ask in that case, but a prompt that any request can raise
//! is a phishing and spam surface, so Mercury asks only when **all** of these hold:
//!
//! * the request is a `fetch` or XHR (destination none, mode not `Navigate`): images, scripts, styles,
//!   fonts, frames and workers never prompt;
//! * it has a web view (without one there is nowhere to show the prompt);
//! * the page asked for **its own origin** (request origin == the URL's origin): a page cannot make the
//!   user type a password for somebody else's server;
//! * the page is a document of the web view's **top-level site** ([`Party::First`] by the policy's
//!   record): an embedded frame or a worker of another site cannot raise a prompt over the page;
//! * no prompt for the same (web view, origin) is open ([`PromptDecision::InFlight`]), and the user did not
//!   refuse one in the last [`REFUSAL_SILENCE`] ([`PromptDecision::Silenced`]).
//!
//! Everything else keeps today's answer: the 401 goes back to the page. A 407 (proxy) is not changed: the
//! caller must not ask this module about one, see "Hook" below. Navigations keep prompting exactly as before
//! and do not use the limiter.
//!
//! The prompt itself carries no realm or initiator line (non-goal of this lane: the sign-in dialog shows the
//! URL only).
//!
//! # Hook
//!
//! `HttpState::request_authentication` (`http_loader.rs`) asks [`subresource_prompt_allowed`] for a request
//! whose mode is not `Navigate` (and never for a 407, which it refuses first: only the caller knows the
//! status), and calls [`prompt_finished`] for such a request when the embedder has answered. A navigation is
//! not asked and does not touch the limiter; asked anyway, this module says `true` for one, so a call site
//! that replaces the old `mode != Navigate` test with this function keeps navigations as they were.
//!
//! # Limiter
//!
//! [`AuthLimiter`] is a small bounded map under a `Mutex` (at most [`MAX_ENTRIES`] keys; a key that is not
//! in flight is evicted before another, oldest first; when every slot is in flight the answer is
//! [`PromptDecision::Full`], which is a refusal). It reads time from an injectable [`Clock`], so the
//! 60 second silence is tested without waiting. It lives in the network policy
//! ([`NetPolicy::auth_limiter`](crate::net_policy::NetPolicy::auth_limiter)), one per process, shared by
//! the public and the private `HttpState`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::debug;
use net_traits::request::{Destination, Origin, Request, RequestMode};
use parking_lot::Mutex;
use servo_base::id::WebViewId;

use crate::http_loader::HttpState;
use crate::net_policy::{NetPolicy, Party};

/// How many (web view, origin) pairs the limiter remembers.
pub const MAX_ENTRIES: usize = 256;

/// How long a refused prompt silences the same (web view, origin).
pub const REFUSAL_SILENCE: Duration = Duration::from_secs(60);

/// The limiter's source of time. Tests give it a clock they can move.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// The wall clock (monotonic).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// What the limiter says about a new prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptDecision {
    /// Go ahead: the pair is now marked as in flight until [`AuthLimiter::finish`].
    Allowed,
    /// A prompt for this (web view, origin) is open.
    InFlight,
    /// The user refused a prompt for this (web view, origin) less than [`REFUSAL_SILENCE`] ago.
    Silenced,
    /// Every slot is a prompt in flight; nothing can be recorded, so nothing is allowed.
    Full,
}

#[derive(Clone, Copy, Debug)]
enum State {
    InFlight,
    SilentUntil(Instant),
}

#[derive(Debug)]
struct Slot {
    state: State,
    /// Counts up with every touch; the smallest value is the oldest.
    touched: u64,
}

#[derive(Debug, Default)]
struct Inner {
    slots: HashMap<(WebViewId, String), Slot>,
    ticks: u64,
}

impl Inner {
    fn touch(&mut self) -> u64 {
        self.ticks = self.ticks.wrapping_add(1);
        self.ticks
    }
}

/// At most one prompt in flight per (web view, origin), and a silence after a refusal. See the module
/// documentation.
pub struct AuthLimiter {
    clock: Arc<dyn Clock>,
    capacity: usize,
    silence: Duration,
    inner: Mutex<Inner>,
}

impl Default for AuthLimiter {
    fn default() -> AuthLimiter {
        AuthLimiter::with_clock(Arc::new(SystemClock))
    }
}

impl std::fmt::Debug for AuthLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthLimiter")
            .field("capacity", &self.capacity)
            .field("silence", &self.silence)
            .field("entries", &self.len())
            .finish()
    }
}

impl AuthLimiter {
    /// A limiter with the default bound and silence that reads time from `clock`.
    pub fn with_clock(clock: Arc<dyn Clock>) -> AuthLimiter {
        AuthLimiter::with_parts(clock, MAX_ENTRIES, REFUSAL_SILENCE)
    }

    /// A limiter with its own bound and silence (tests use a small bound). A bound of 0 refuses everything.
    pub fn with_parts(clock: Arc<dyn Clock>, capacity: usize, silence: Duration) -> AuthLimiter {
        AuthLimiter {
            clock,
            capacity,
            silence,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// How many (web view, origin) pairs are recorded.
    pub fn len(&self) -> usize {
        self.inner.lock().slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Ask to open a prompt for `origin` (an ASCII serialisation) in `webview`. On
    /// [`PromptDecision::Allowed`] the pair is in flight, and the caller must call [`AuthLimiter::finish`]
    /// when the prompt is over, whatever the user did.
    pub fn begin(&self, webview: WebViewId, origin: &str) -> PromptDecision {
        let now = self.clock.now();
        let mut inner = self.inner.lock();
        let key = (webview, origin.to_owned());

        match inner.slots.get(&key).map(|slot| slot.state) {
            Some(State::InFlight) => return PromptDecision::InFlight,
            Some(State::SilentUntil(until)) if now < until => return PromptDecision::Silenced,
            // A silence that has run out: the slot is reused below.
            Some(State::SilentUntil(_)) => {}
            None => {
                if inner.slots.len() >= self.capacity && !self.make_room(&mut inner, now) {
                    return PromptDecision::Full;
                }
            }
        }

        let touched = inner.touch();
        inner.slots.insert(
            key,
            Slot {
                state: State::InFlight,
                touched,
            },
        );
        PromptDecision::Allowed
    }

    /// The prompt for (`webview`, `origin`) is over. `answered` is true when the user supplied
    /// credentials: the pair is forgotten. Otherwise (cancelled, dismissed, nobody there) the pair is
    /// silenced for the refusal period. A pair that is not in flight is left alone.
    pub fn finish(&self, webview: WebViewId, origin: &str, answered: bool) {
        let now = self.clock.now();
        let mut inner = self.inner.lock();
        let key = (webview, origin.to_owned());
        if !matches!(
            inner.slots.get(&key).map(|slot| slot.state),
            Some(State::InFlight)
        ) {
            return;
        }
        if answered {
            inner.slots.remove(&key);
        } else {
            let touched = inner.touch();
            inner.slots.insert(
                key,
                Slot {
                    state: State::SilentUntil(now + self.silence),
                    touched,
                },
            );
        }
    }

    /// Free one slot: drop every silence that has run out, then, if still full, the oldest silence. A
    /// prompt in flight is never dropped (that would allow a second one). Returns whether a slot is free.
    fn make_room(&self, inner: &mut Inner, now: Instant) -> bool {
        inner.slots.retain(|_, slot| match slot.state {
            State::InFlight => true,
            State::SilentUntil(until) => now < until,
        });
        if inner.slots.len() < self.capacity {
            return true;
        }
        let oldest_silence = inner
            .slots
            .iter()
            .filter(|(_, slot)| matches!(slot.state, State::SilentUntil(_)))
            .min_by_key(|(_, slot)| slot.touched)
            .map(|(key, _)| key.clone());
        match oldest_silence {
            Some(key) => {
                inner.slots.remove(&key);
                true
            }
            None => false,
        }
    }
}

/// The request's own origin as an ASCII serialisation, when the request is one that may prompt: a
/// `fetch` or XHR (not a navigation) whose origin is the origin of its URL, and a tuple origin. `None`
/// for everything else.
fn own_origin_of_subresource(request: &Request) -> Option<String> {
    if request.mode == RequestMode::Navigate || request.destination != Destination::None {
        return None;
    }
    let Origin::Origin(origin) = &request.origin else {
        return None;
    };
    if !origin.is_tuple() || *origin != request.current_url().origin() {
        return None;
    }
    Some(origin.ascii_serialization().into_owned())
}

/// Whether a 401 answered to `request` may open a sign-in prompt. See the module documentation for the
/// rules. A navigation is not this module's business and gives `true` (it prompts as it always did).
///
/// A `true` for a non-navigation request has recorded the prompt as in flight: call [`prompt_finished`]
/// when it is over.
pub fn subresource_prompt_allowed(state: &HttpState, request: &Request) -> bool {
    if request.mode == RequestMode::Navigate {
        return true;
    }
    let Some(webview) = request.target_webview_id else {
        return false;
    };
    let Some(origin) = own_origin_of_subresource(request) else {
        return false;
    };
    // The page must be a document of the web view's top-level site.
    if state.net_policy.sites_of_request(request).party != Party::First {
        debug!("sign-in prompt for a subresource refused: not the top-level site's own request");
        return false;
    }
    match state.net_policy.auth_limiter().begin(webview, &origin) {
        PromptDecision::Allowed => true,
        decision => {
            debug!("sign-in prompt for a subresource refused: {decision:?}");
            false
        }
    }
}

/// The prompt that [`subresource_prompt_allowed`] allowed is over; `answered` is whether the user
/// supplied credentials (true clears the limiter's record, false silences the pair for
/// [`REFUSAL_SILENCE`]). Does nothing for a navigation or for a request that could not have been allowed.
pub fn prompt_finished(state: &HttpState, request: &Request, answered: bool) {
    let Some(webview) = request.target_webview_id else {
        return;
    };
    let Some(origin) = own_origin_of_subresource(request) else {
        return;
    };
    state
        .net_policy
        .auth_limiter()
        .finish(webview, &origin, answered);
}

/// The mark "a prompt for this (web view, origin) is open" that [`subresource_prompt_allowed`] set, owned by
/// the fetch that waits for the answer. [`PromptGuard::finish`] ends it with the user's answer; when the guard
/// is dropped without that (the fetch future was dropped while the prompt was open: an aborted `fetch`, a
/// navigation away) it ends it as a refusal, so the pair is silenced for [`REFUSAL_SILENCE`] and not stuck "in
/// flight" for the life of the web view. A silence, not a release, because the prompt of the embedder may still
/// be on screen: a page must not be able to stack prompts by aborting its own requests.
#[must_use = "dropping the guard ends the prompt as a refusal"]
pub struct PromptGuard {
    policy: NetPolicy,
    webview: WebViewId,
    origin: String,
    finished: bool,
}

impl PromptGuard {
    /// The guard of the prompt that [`subresource_prompt_allowed`] allowed for `request`. `None` for a request
    /// that cannot have been allowed (a navigation, no web view, not its own origin).
    pub fn of(state: &HttpState, request: &Request) -> Option<PromptGuard> {
        Some(PromptGuard {
            policy: state.net_policy.clone(),
            webview: request.target_webview_id?,
            origin: own_origin_of_subresource(request)?,
            finished: false,
        })
    }

    /// The prompt is over; `answered` is whether the user supplied credentials (see [`prompt_finished`]).
    pub fn finish(mut self, answered: bool) {
        self.finished = true;
        self.policy
            .auth_limiter()
            .finish(self.webview, &self.origin, answered);
    }
}

impl Drop for PromptGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.policy
                .auth_limiter()
                .finish(self.webview, &self.origin, false);
        }
    }
}
