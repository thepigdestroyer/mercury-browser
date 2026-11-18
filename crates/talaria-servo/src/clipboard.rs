// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The clipboard pages reach through Servo (clipboard lane).
//!
//! Every web view gets Mercury's `ClipboardDelegate` (the tab's `BrowserDelegate`), so Servo's
//! default one, which reads and writes the system clipboard for any page, is never used. What a
//! page may do is decided by the tab's `ClipboardGate` (`talaria-core`); the clipboard itself is
//! the `Pasteboard` the browser installs with `ServoRuntime::set_pasteboard`, a
//! `MemoryPasteboard` until then.

use std::cell::RefCell;
use std::rc::Rc;

use servo::StringRequest;
use talaria_core::{
    ClipboardCommand, ClipboardGate, ClipboardReadDecision, MemoryPasteboard, Pasteboard, TabId,
};

use crate::page_request_clock;

/// What a user activation check is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationUse {
    /// A write: the tab's activation is looked at and kept.
    Peek,
    /// A read that will ask the user: the tab's activation is used up.
    Take,
}

/// Whether a tab has a recent user activation; see `ServoRuntime::set_user_activation_check`.
pub type UserActivationCheck = Rc<dyn Fn(TabId, ActivationUse) -> bool>;

/// The clipboard and the activation check, shared by every tab of a runtime.
pub(crate) struct ClipboardAccess {
    pasteboard: RefCell<Rc<dyn Pasteboard>>,
    user_activation: RefCell<Option<UserActivationCheck>>,
}

impl Default for ClipboardAccess {
    fn default() -> Self {
        Self {
            pasteboard: RefCell::new(Rc::new(MemoryPasteboard::default())),
            user_activation: RefCell::default(),
        }
    }
}

impl ClipboardAccess {
    pub(crate) fn set_pasteboard(&self, pasteboard: Rc<dyn Pasteboard>) {
        self.pasteboard.replace(pasteboard);
    }

    pub(crate) fn set_user_activation_check(&self, check: UserActivationCheck) {
        self.user_activation.replace(Some(check));
    }

    fn pasteboard(&self) -> Rc<dyn Pasteboard> {
        Rc::clone(&self.pasteboard.borrow())
    }

    /// Without a check installed no tab has an activation.
    fn has_user_activation(&self, tab_id: TabId, activation_use: ActivationUse) -> bool {
        let check = self.user_activation.borrow().clone();
        check.is_some_and(|check| check(tab_id, activation_use))
    }
}

/// One tab's clipboard; shared by its delegate (the page's requests) and its `ServoWebView`
/// (the user's commands).
#[derive(Clone)]
pub(crate) struct TabClipboard {
    tab_id: TabId,
    gate: Rc<RefCell<ClipboardGate>>,
    access: Rc<ClipboardAccess>,
}

/// What became of a page's read.
pub(crate) enum ClipboardRead<R: StringAnswer = StringRequest> {
    /// The clipboard's text, for the page now.
    Text(String),
    /// The user must be asked first.
    AskPermission(ClipboardReadRequest<R>),
    Refused,
}

impl TabClipboard {
    pub(crate) fn new(tab_id: TabId, access: Rc<ClipboardAccess>) -> Self {
        Self {
            tab_id,
            gate: Rc::default(),
            access,
        }
    }

    /// The user chose Cut, Copy or Paste for this tab's page.
    pub(crate) fn user_command(&self, command: ClipboardCommand) {
        self.gate
            .borrow_mut()
            .user_command(command, page_request_clock());
    }

    pub(crate) fn document_started(&self) {
        self.gate.borrow_mut().document_started();
    }

    fn may_write(&self) -> bool {
        let has_activation = self
            .access
            .has_user_activation(self.tab_id, ActivationUse::Peek);
        self.gate
            .borrow()
            .may_write(page_request_clock(), has_activation)
    }

    /// The page writes `text`; returns whether it reached the clipboard.
    pub(crate) fn write_text(&self, text: &str) -> bool {
        let allowed = self.may_write();
        if allowed {
            self.access.pasteboard().write_text(text);
        }
        allowed
    }

    /// The page clears the clipboard; returns whether it was cleared.
    pub(crate) fn clear(&self) -> bool {
        let allowed = self.may_write();
        if allowed {
            self.access.pasteboard().clear();
        }
        allowed
    }

    /// The page reads; `request` is answered at once unless the user is to be asked.
    pub(crate) fn read<R: StringAnswer>(&self, request: R) -> ClipboardRead<R> {
        let decision = self.gate.borrow_mut().read(page_request_clock(), || {
            self.access
                .has_user_activation(self.tab_id, ActivationUse::Take)
        });
        match decision {
            ClipboardReadDecision::Read => {
                let text = self.access.pasteboard().read_text().unwrap_or_default();
                request.succeed(text.clone());
                ClipboardRead::Text(text)
            }
            ClipboardReadDecision::AskPermission => {
                ClipboardRead::AskPermission(ClipboardReadRequest {
                    request,
                    pasteboard: self.access.pasteboard(),
                })
            }
            ClipboardReadDecision::Refuse => {
                request.fail();
                ClipboardRead::Refused
            }
        }
    }
}

/// Servo's answer object for a read; a trait so the logic is tested without Servo.
pub(crate) trait StringAnswer {
    fn succeed(self, text: String);
    /// The page gets no text (Servo 0.8.0 resolves `readText()` with "" then).
    fn fail(self);
}

impl StringAnswer for StringRequest {
    fn succeed(self, text: String) {
        self.success(text);
    }

    fn fail(self) {
        self.failure("The clipboard is not available to this page.".to_owned());
    }
}

/// A page's read that waits for the user's permission. Dropped unanswered, the page gets no
/// text (Servo's `StringRequest` fails itself when dropped).
pub(crate) struct ClipboardReadRequest<R: StringAnswer = StringRequest> {
    request: R,
    pasteboard: Rc<dyn Pasteboard>,
}

impl<R: StringAnswer> ClipboardReadRequest<R> {
    /// Gives the page what is on the clipboard now.
    pub(crate) fn allow(self) {
        let text = self.pasteboard.read_text().unwrap_or_default();
        self.request.succeed(text);
    }

    pub(crate) fn deny(self) {
        self.request.fail();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    type Answers = Rc<RefCell<Vec<Option<String>>>>;

    struct FakeAnswer(Answers);

    impl StringAnswer for FakeAnswer {
        fn succeed(self, text: String) {
            self.0.borrow_mut().push(Some(text));
        }

        fn fail(self) {
            self.0.borrow_mut().push(None);
        }
    }

    /// A tab whose activation is `activation`, with the activation uses logged.
    fn tab(
        pasteboard: &Rc<MemoryPasteboard>,
        activation: bool,
    ) -> (TabClipboard, Rc<RefCell<Vec<ActivationUse>>>) {
        let access = Rc::new(ClipboardAccess::default());
        access.set_pasteboard(Rc::clone(pasteboard) as Rc<dyn Pasteboard>);
        let uses = Rc::new(RefCell::new(Vec::new()));
        let logged = Rc::clone(&uses);
        access.set_user_activation_check(Rc::new(move |tab_id, activation_use| {
            assert_eq!(tab_id, TabId::new(7));
            logged.borrow_mut().push(activation_use);
            activation
        }));
        (TabClipboard::new(TabId::new(7), access), uses)
    }

    #[test]
    fn the_default_clipboard_is_in_memory_and_no_tab_has_an_activation() {
        let access = Rc::new(ClipboardAccess::default());
        let clipboard = TabClipboard::new(TabId::new(1), Rc::clone(&access));
        assert!(!clipboard.write_text("from the page"));
        assert_eq!(access.pasteboard().read_text(), None);

        clipboard.user_command(ClipboardCommand::Copy);
        assert!(clipboard.write_text("copied"));
        assert_eq!(access.pasteboard().read_text().as_deref(), Some("copied"));
    }

    #[test]
    fn a_write_without_a_gesture_leaves_the_clipboard_alone() {
        let pasteboard = Rc::new(MemoryPasteboard::with_text("the user's"));
        let (clipboard, uses) = tab(&pasteboard, false);
        assert!(!clipboard.write_text("page"));
        assert!(!clipboard.clear());
        assert_eq!(pasteboard.text().as_deref(), Some("the user's"));
        assert_eq!(
            *uses.borrow(),
            vec![ActivationUse::Peek, ActivationUse::Peek]
        );
    }

    #[test]
    fn a_write_after_a_click_reaches_the_clipboard_and_keeps_the_activation() {
        let pasteboard = Rc::new(MemoryPasteboard::default());
        let (clipboard, uses) = tab(&pasteboard, true);
        assert!(clipboard.write_text("copy button"));
        assert_eq!(pasteboard.text().as_deref(), Some("copy button"));
        assert_eq!(*uses.borrow(), vec![ActivationUse::Peek]);
    }

    #[test]
    fn the_users_paste_reads_without_asking_or_using_the_activation() {
        let pasteboard = Rc::new(MemoryPasteboard::with_text("pasted"));
        let (clipboard, uses) = tab(&pasteboard, true);
        let answers = Answers::default();
        clipboard.user_command(ClipboardCommand::Paste);
        assert!(matches!(
            clipboard.read(FakeAnswer(Rc::clone(&answers))),
            ClipboardRead::Text(text) if text == "pasted"
        ));
        assert_eq!(*answers.borrow(), vec![Some("pasted".to_owned())]);
        assert!(uses.borrow().is_empty());
    }

    #[test]
    fn a_script_read_after_a_click_waits_for_the_user() {
        let pasteboard = Rc::new(MemoryPasteboard::with_text("secret"));
        let (clipboard, uses) = tab(&pasteboard, true);
        let answers = Answers::default();
        let ClipboardRead::AskPermission(request) = clipboard.read(FakeAnswer(Rc::clone(&answers)))
        else {
            panic!("expected a permission request");
        };
        assert!(answers.borrow().is_empty());
        assert_eq!(*uses.borrow(), vec![ActivationUse::Take]);

        // The clipboard is read when the user allows it, not when the page asked.
        pasteboard.write_text("newer");
        request.allow();
        assert_eq!(*answers.borrow(), vec![Some("newer".to_owned())]);
    }

    #[test]
    fn a_denied_script_read_gets_no_text() {
        let pasteboard = Rc::new(MemoryPasteboard::with_text("secret"));
        let (clipboard, _uses) = tab(&pasteboard, true);
        let answers = Answers::default();
        let ClipboardRead::AskPermission(request) = clipboard.read(FakeAnswer(Rc::clone(&answers)))
        else {
            panic!("expected a permission request");
        };
        request.deny();
        assert_eq!(*answers.borrow(), vec![None]);
    }

    #[test]
    fn a_script_read_without_a_gesture_is_refused_at_once() {
        let pasteboard = Rc::new(MemoryPasteboard::with_text("secret"));
        let (clipboard, _uses) = tab(&pasteboard, false);
        let answers = Answers::default();
        assert!(matches!(
            clipboard.read(FakeAnswer(Rc::clone(&answers))),
            ClipboardRead::Refused
        ));
        assert_eq!(*answers.borrow(), vec![None]);
    }

    #[test]
    fn a_new_document_does_not_inherit_the_users_paste() {
        let pasteboard = Rc::new(MemoryPasteboard::with_text("secret"));
        let (clipboard, _uses) = tab(&pasteboard, false);
        clipboard.user_command(ClipboardCommand::Paste);
        clipboard.document_started();
        let answers = Answers::default();
        assert!(matches!(
            clipboard.read(FakeAnswer(Rc::clone(&answers))),
            ClipboardRead::Refused
        ));
    }
}
