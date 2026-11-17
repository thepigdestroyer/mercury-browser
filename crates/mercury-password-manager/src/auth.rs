// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::time::{Duration, Instant};

/// How long one successful authentication lets the user reveal and copy passwords without being
/// asked again. Closing the settings window ends it early.
pub const AUTH_GRACE_PERIOD: Duration = Duration::from_secs(60);

/// How an authentication ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthOutcome {
    /// The user proved who they are (Touch ID, or the login password).
    Allowed,
    /// The user cancelled, or failed.
    Denied,
    /// This Mac cannot ask (no password set, or the service is not available), or the run may
    /// not ask (tests and automated runs).
    Unavailable,
}

/// Asks the user to prove who they are before a password is shown or copied.
///
/// `done` is called exactly once, on the thread that owns the browser's interface, possibly
/// after `authenticate` has returned. `reason` completes the system's sentence "Mercury Browser
/// is trying to …".
pub trait Authenticator {
    /// Starts an authentication.
    fn authenticate(&self, reason: &str, done: Box<dyn FnOnce(AuthOutcome)>);
}

/// Never asks and always answers [`AuthOutcome::Unavailable`]: for automated runs, which must
/// never show a system prompt.
#[derive(Debug, Default)]
pub struct RefusingAuthenticator;

impl Authenticator for RefusingAuthenticator {
    fn authenticate(&self, _reason: &str, done: Box<dyn FnOnce(AuthOutcome)>) {
        done(AuthOutcome::Unavailable);
    }
}

/// Remembers a successful authentication for [`AUTH_GRACE_PERIOD`].
#[derive(Debug, Default)]
pub struct AuthGate {
    allowed_at: Option<Instant>,
}

impl AuthGate {
    /// A gate that has not been opened.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the user authenticated within the grace period before `now`.
    pub fn is_open(&self, now: Instant) -> bool {
        self.allowed_at.is_some_and(|allowed_at| {
            now >= allowed_at && now.duration_since(allowed_at) < AUTH_GRACE_PERIOD
        })
    }

    /// Records an authentication's outcome; only [`AuthOutcome::Allowed`] opens the gate, and
    /// any other outcome closes it.
    pub fn record(&mut self, outcome: AuthOutcome, now: Instant) {
        self.allowed_at = (outcome == AuthOutcome::Allowed).then_some(now);
    }

    /// Ends the grace period (the settings window closed).
    pub fn close(&mut self) {
        self.allowed_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_gate_is_closed() {
        assert!(!AuthGate::new().is_open(Instant::now()));
    }

    #[test]
    fn an_allowed_authentication_opens_the_gate_for_the_grace_period_only() {
        let start = Instant::now();
        let mut gate = AuthGate::new();

        gate.record(AuthOutcome::Allowed, start);

        assert!(gate.is_open(start));
        assert!(gate.is_open(start + AUTH_GRACE_PERIOD - Duration::from_millis(1)));
        assert!(!gate.is_open(start + AUTH_GRACE_PERIOD));
    }

    #[test]
    fn a_denied_or_unavailable_answer_closes_an_open_gate() {
        let start = Instant::now();
        for outcome in [AuthOutcome::Denied, AuthOutcome::Unavailable] {
            let mut gate = AuthGate::new();
            gate.record(AuthOutcome::Allowed, start);

            gate.record(outcome, start);

            assert!(!gate.is_open(start), "{outcome:?}");
        }
    }

    #[test]
    fn closing_ends_the_grace_period() {
        let start = Instant::now();
        let mut gate = AuthGate::new();
        gate.record(AuthOutcome::Allowed, start);

        gate.close();

        assert!(!gate.is_open(start));
    }

    #[test]
    fn the_refusing_authenticator_never_allows() {
        let outcome = std::rc::Rc::new(std::cell::Cell::new(None));
        let seen = std::rc::Rc::clone(&outcome);

        RefusingAuthenticator.authenticate(
            "show a password",
            Box::new(move |answer| seen.set(Some(answer))),
        );

        assert_eq!(outcome.get(), Some(AuthOutcome::Unavailable));
    }
}
