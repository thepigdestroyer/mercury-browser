// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A scripted [`UserVerifier`] for tests of the built-in authenticator and of the engine and UI
//! lanes' code that sits on top of it.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::{
    PasskeyError, PasskeySummary, UserVerifier, Verification, VerificationRequest,
    VerifierCapabilities,
};

/// What a [`FakeVerifier`] answers to a [`UserVerifier::verify`] call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeAnswer {
    /// The user is present and agrees. Verification succeeds when the verifier's capabilities say
    /// it can verify (`verification == Some(true)`).
    Approve,
    /// The user is present and agrees, but is not verified (a failed Touch ID, or a verifier with
    /// no verification).
    ApproveWithoutVerification,
    /// The user declines: neither presence nor verification.
    Decline,
    /// The verifier itself fails with this error.
    Fail(PasskeyError),
}

/// What a [`FakeVerifier`] answers to a [`UserVerifier::choose_passkey`] call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakePick {
    /// The user picks the candidate at this index (it may be out of range, to test the caller).
    Index(usize),
    /// The user dismisses the picker.
    Dismiss,
    /// The picker fails with this error.
    Fail(PasskeyError),
}

#[derive(Debug)]
struct Script {
    answers: VecDeque<FakeAnswer>,
    default_answer: FakeAnswer,
    picks: VecDeque<FakePick>,
    default_pick: FakePick,
    requests: Vec<VerificationRequest>,
    picker_calls: Vec<(String, Vec<PasskeySummary>)>,
    cancel_calls: usize,
}

/// A [`UserVerifier`] that answers from a script and records everything it was asked.
///
/// Answers queued with [`FakeVerifier::push_answer`] are used first, one per `verify` call; when
/// the queue is empty the default answer ([`FakeAnswer::Approve`] unless changed) is used. Picks
/// work the same way (default: the first candidate).
pub struct FakeVerifier {
    capabilities: VerifierCapabilities,
    script: Mutex<Script>,
}

impl FakeVerifier {
    /// A verifier that approves everything and can verify the user (Touch ID set up).
    pub fn new() -> Self {
        Self::with_capabilities(VerifierCapabilities {
            presence: true,
            verification: Some(true),
        })
    }

    /// A verifier that approves everything, with the given capabilities.
    pub fn with_capabilities(capabilities: VerifierCapabilities) -> Self {
        Self {
            capabilities,
            script: Mutex::new(Script {
                answers: VecDeque::new(),
                default_answer: FakeAnswer::Approve,
                picks: VecDeque::new(),
                default_pick: FakePick::Index(0),
                requests: Vec::new(),
                picker_calls: Vec::new(),
                cancel_calls: 0,
            }),
        }
    }

    fn script(&self) -> MutexGuard<'_, Script> {
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues an answer for the next `verify` call that has none queued before it.
    pub fn push_answer(&self, answer: FakeAnswer) -> &Self {
        self.script().answers.push_back(answer);
        self
    }

    /// Changes the answer used when the queue is empty.
    pub fn set_default_answer(&self, answer: FakeAnswer) -> &Self {
        self.script().default_answer = answer;
        self
    }

    /// Queues an answer for the next `choose_passkey` call that has none queued before it.
    pub fn push_pick(&self, pick: FakePick) -> &Self {
        self.script().picks.push_back(pick);
        self
    }

    /// Changes the pick used when the queue is empty.
    pub fn set_default_pick(&self, pick: FakePick) -> &Self {
        self.script().default_pick = pick;
        self
    }

    /// Every [`VerificationRequest`] seen so far, in order.
    pub fn requests(&self) -> Vec<VerificationRequest> {
        self.script().requests.clone()
    }

    /// Every `choose_passkey` call so far: the origin and the candidates offered.
    pub fn picker_calls(&self) -> Vec<(String, Vec<PasskeySummary>)> {
        self.script().picker_calls.clone()
    }

    /// How many times [`UserVerifier::cancel`] was called.
    pub fn cancel_calls(&self) -> usize {
        self.script().cancel_calls
    }
}

impl Default for FakeVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for FakeVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let script = self.script();
        formatter
            .debug_struct("FakeVerifier")
            .field("capabilities", &self.capabilities)
            .field("requests", &script.requests.len())
            .field("picker_calls", &script.picker_calls.len())
            .field("cancel_calls", &script.cancel_calls)
            .finish()
    }
}

impl UserVerifier for FakeVerifier {
    fn capabilities(&self) -> VerifierCapabilities {
        self.capabilities
    }

    fn verify(&self, request: &VerificationRequest) -> Result<Verification, PasskeyError> {
        let mut script = self.script();
        script.requests.push(request.clone());
        let answer = script
            .answers
            .pop_front()
            .unwrap_or_else(|| script.default_answer.clone());
        match answer {
            FakeAnswer::Approve => Ok(Verification {
                presence: true,
                verification: self.capabilities.verification == Some(true),
            }),
            FakeAnswer::ApproveWithoutVerification => Ok(Verification {
                presence: true,
                verification: false,
            }),
            FakeAnswer::Decline => Ok(Verification {
                presence: false,
                verification: false,
            }),
            FakeAnswer::Fail(error) => Err(error),
        }
    }

    fn choose_passkey(
        &self,
        origin: &str,
        candidates: &[PasskeySummary],
    ) -> Result<Option<usize>, PasskeyError> {
        let mut script = self.script();
        script
            .picker_calls
            .push((origin.to_owned(), candidates.to_vec()));
        let pick = script
            .picks
            .pop_front()
            .unwrap_or_else(|| script.default_pick.clone());
        match pick {
            FakePick::Index(index) => Ok(Some(index)),
            FakePick::Dismiss => Ok(None),
            FakePick::Fail(error) => Err(error),
        }
    }

    fn cancel(&self) {
        self.script().cancel_calls += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PasskeyKey, VerificationPurpose};

    fn request() -> VerificationRequest {
        VerificationRequest {
            origin: "https://example.com".into(),
            purpose: VerificationPurpose::NoPasskeyFound {
                rp_id: "example.com".into(),
            },
            require_presence: true,
            require_verification: false,
        }
    }

    fn summary(id: u8) -> PasskeySummary {
        PasskeySummary {
            key: PasskeyKey {
                rp_id: "example.com".into(),
                credential_id: vec![id; 16],
            },
            user_handle: None,
            username: String::new(),
            display_name: String::new(),
            algorithm: -7,
            counter: None,
            created_at_ms: None,
            modified_at_ms: None,
        }
    }

    #[test]
    fn queued_answers_come_first_then_the_default() {
        let verifier = FakeVerifier::new();
        verifier.push_answer(FakeAnswer::Decline);
        verifier.push_answer(FakeAnswer::Fail(PasskeyError::Locked));
        assert_eq!(
            verifier.verify(&request()),
            Ok(Verification {
                presence: false,
                verification: false
            })
        );
        assert_eq!(verifier.verify(&request()), Err(PasskeyError::Locked));
        assert_eq!(
            verifier.verify(&request()),
            Ok(Verification {
                presence: true,
                verification: true
            })
        );
        assert_eq!(verifier.requests().len(), 3);
    }

    #[test]
    fn approve_follows_the_capabilities() {
        let verifier = FakeVerifier::with_capabilities(VerifierCapabilities {
            presence: true,
            verification: None,
        });
        assert_eq!(
            verifier.verify(&request()),
            Ok(Verification {
                presence: true,
                verification: false
            })
        );
        assert_eq!(
            verifier.capabilities(),
            VerifierCapabilities {
                presence: true,
                verification: None
            }
        );
    }

    #[test]
    fn picks_are_scripted_and_recorded() {
        let verifier = FakeVerifier::new();
        verifier.push_pick(FakePick::Index(1));
        verifier.push_pick(FakePick::Dismiss);
        verifier.push_pick(FakePick::Fail(PasskeyError::Cancelled));
        let candidates = [summary(1), summary(2)];
        assert_eq!(
            verifier.choose_passkey("https://example.com", &candidates),
            Ok(Some(1))
        );
        assert_eq!(
            verifier.choose_passkey("https://example.com", &candidates),
            Ok(None)
        );
        assert_eq!(
            verifier.choose_passkey("https://example.com", &candidates),
            Err(PasskeyError::Cancelled)
        );
        assert_eq!(
            verifier.choose_passkey("https://example.com", &candidates),
            Ok(Some(0))
        );
        assert_eq!(verifier.picker_calls().len(), 4);
        assert_eq!(verifier.picker_calls()[0].1.len(), 2);
    }

    #[test]
    fn cancel_is_counted() {
        let verifier = FakeVerifier::new();
        verifier.cancel();
        verifier.cancel();
        assert_eq!(verifier.cancel_calls(), 2);
    }
}
