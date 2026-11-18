// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Fakes of the operating-system services, for tests and automated runs: nothing here ever
//! shows a prompt or touches the real pasteboard.

use std::cell::{Cell, RefCell};

use crate::{AuthOutcome, Authenticator, Pasteboard};

/// A started authentication: its reason and who waits for the answer.
type Waiting = (String, Box<dyn FnOnce(AuthOutcome)>);

/// An authenticator that keeps each request until the test answers it.
#[derive(Default)]
pub struct FakeAuthenticator {
    waiting: RefCell<Vec<Waiting>>,
    asked: Cell<usize>,
}

impl FakeAuthenticator {
    /// How many authentications were started.
    pub fn asked(&self) -> usize {
        self.asked.get()
    }

    /// The reason of every authentication still waiting.
    pub fn waiting_reasons(&self) -> Vec<String> {
        self.waiting
            .borrow()
            .iter()
            .map(|(reason, _)| reason.clone())
            .collect()
    }

    /// Answers the oldest waiting authentication; false if none was waiting.
    pub fn answer(&self, outcome: AuthOutcome) -> bool {
        let next = {
            let mut waiting = self.waiting.borrow_mut();
            (!waiting.is_empty()).then(|| waiting.remove(0))
        };
        match next {
            Some((_, done)) => {
                done(outcome);
                true
            }
            None => false,
        }
    }
}

impl Authenticator for FakeAuthenticator {
    fn authenticate(&self, reason: &str, done: Box<dyn FnOnce(AuthOutcome)>) {
        self.asked.set(self.asked.get() + 1);
        self.waiting.borrow_mut().push((reason.to_owned(), done));
    }
}

/// A pasteboard in memory.
#[derive(Default)]
pub struct FakePasteboard {
    contents: RefCell<Option<(String, bool)>>,
    change_count: Cell<i64>,
}

impl FakePasteboard {
    /// The text on it and whether it was marked concealed.
    pub fn contents(&self) -> Option<(String, bool)> {
        self.contents.borrow().clone()
    }
}

impl Pasteboard for FakePasteboard {
    fn write_text(&self, text: &str, concealed: bool) -> Option<i64> {
        *self.contents.borrow_mut() = Some((text.to_owned(), concealed));
        self.change_count.set(self.change_count.get() + 1);
        Some(self.change_count.get())
    }

    fn change_count(&self) -> i64 {
        self.change_count.get()
    }

    fn clear(&self) {
        *self.contents.borrow_mut() = None;
        self.change_count.set(self.change_count.get() + 1);
    }
}
