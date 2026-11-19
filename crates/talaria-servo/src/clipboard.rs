// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The clipboard pages reach through Servo (clipboard lane).
//!
//! Every web view gets Mercury's `ClipboardDelegate` (the tab's `BrowserDelegate`), so Servo's
//! default one, which reads and writes the system clipboard for any page, is never used. What a
//! page may do is decided by the tab's `ClipboardGate` (`talaria-core`); the clipboard itself is
//! the `Pasteboard` the browser installs with `ServoRuntime::set_pasteboard`, a
//! `MemoryPasteboard` until then. A script's read (`navigator.clipboard.readText()`) asks the
//! user only once `talaria_core::CLIPBOARD_READ_FRAME_GATE` is on; until then it is refused.
//!
//! While `talaria_core::ASYNC_CLIPBOARD_OFFERED` is off, pages have no `navigator.clipboard`, and
//! every request that reaches the delegate is the user's own Cut, Copy or Paste. The tab must be
//! told about each of those before Servo performs it (`TabClipboard::key_pressed`,
//! `context_menu_chosen` and `user_command`, called from `web_view.rs`): a Paste that was not
//! noted reads as a script's read and inserts empty text.

use std::cell::RefCell;
use std::rc::Rc;

use servo::StringRequest;
use talaria_core::{
    ClipboardCommand, ClipboardGate, ClipboardReadDecision, ContextMenuAction, KeyboardInput,
    MemoryPasteboard, Pasteboard, TabId, clipboard_shortcut, context_menu_clipboard_command,
};

use crate::page_request_clock;

/// Whether Cut, Copy and Paste are the Command key's (macOS) or the Control key's; Servo maps the
/// same keys the same way.
pub(crate) const COMMAND_KEY_IS_META: bool = cfg!(target_os = "macos");

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
        Self::with_gate(tab_id, access, ClipboardGate::default())
    }

    fn with_gate(tab_id: TabId, access: Rc<ClipboardAccess>, gate: ClipboardGate) -> Self {
        Self {
            tab_id,
            gate: Rc::new(RefCell::new(gate)),
            access,
        }
    }

    /// The user chose Cut, Copy or Paste for this tab's page.
    pub(crate) fn user_command(&self, command: ClipboardCommand) {
        self.gate
            .borrow_mut()
            .user_command(command, page_request_clock());
    }

    /// A key press the user made in this tab's page: if it is Cut, Copy or Paste, Servo performs
    /// the edit itself, and the edit's clipboard request that follows is the user's.
    pub(crate) fn key_pressed(&self, input: &KeyboardInput) {
        if let Some(command) = clipboard_shortcut(input, COMMAND_KEY_IS_META) {
            self.user_command(command);
        }
    }

    /// The user chose `action` in the page's context menu: its Cut, Copy and Paste, and copying a
    /// link or image address, are the user's own clipboard commands.
    pub(crate) fn context_menu_chosen(&self, action: ContextMenuAction) {
        if let Some(command) = context_menu_clipboard_command(action) {
            self.user_command(command);
        }
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

/// Fakes for the tests of this module and of `web_view`, which drives a tab's clipboard the way
/// the engine does.
#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::rc::Rc;

    use talaria_core::{
        KeyLocation, KeyState, KeyboardInput, KeyboardModifiers, MemoryPasteboard, Pasteboard,
        TabId,
    };

    use super::{ActivationUse, COMMAND_KEY_IS_META, ClipboardAccess, StringAnswer, TabClipboard};

    /// What the page was told for each read: `Some(text)`, or `None` for a refusal.
    pub(crate) type Answers = Rc<RefCell<Vec<Option<String>>>>;

    /// Servo's answer object for a read, recording what the page was told.
    pub(crate) struct FakeAnswer(pub(crate) Answers);

    impl StringAnswer for FakeAnswer {
        fn succeed(self, text: String) {
            self.0.borrow_mut().push(Some(text));
        }

        fn fail(self) {
            self.0.borrow_mut().push(None);
        }
    }

    /// The activation uses a tab's clipboard asked about, in order.
    pub(crate) type ActivationLog = Rc<RefCell<Vec<ActivationUse>>>;

    /// A tab built the way the runtime builds every tab (`TabClipboard::new`, so with the gate a
    /// tab really starts with), over a clipboard in memory holding `text`. `activation` is what
    /// the tab's user activation check says; the uses it was asked about are logged.
    pub(crate) fn runtime_tab(
        text: &str,
        activation: bool,
    ) -> (TabClipboard, Rc<MemoryPasteboard>, ActivationLog) {
        let pasteboard = Rc::new(MemoryPasteboard::with_text(text));
        let access = Rc::new(ClipboardAccess::default());
        access.set_pasteboard(Rc::clone(&pasteboard) as Rc<dyn Pasteboard>);
        let uses = ActivationLog::default();
        let logged = Rc::clone(&uses);
        access.set_user_activation_check(Rc::new(move |_tab_id, activation_use| {
            logged.borrow_mut().push(activation_use);
            activation
        }));
        (TabClipboard::new(TabId::new(9), access), pasteboard, uses)
    }

    /// A key press down; with the platform's command key (Command on macOS, Control elsewhere)
    /// held when `with_command_key`.
    pub(crate) fn key_press(key: &str, with_command_key: bool) -> KeyboardInput {
        KeyboardInput {
            state: KeyState::Down,
            key: key.to_owned(),
            code: String::new(),
            location: KeyLocation::Standard,
            modifiers: KeyboardModifiers {
                alt: false,
                control: with_command_key && !COMMAND_KEY_IS_META,
                meta: with_command_key && COMMAND_KEY_IS_META,
                shift: false,
            },
            repeat: false,
            is_composing: false,
        }
    }

    /// What the page is told when it reads now: the text, or `None` when the read is refused.
    pub(crate) fn page_read(clipboard: &TabClipboard) -> Option<String> {
        let answers = Answers::default();
        let _read = clipboard.read(FakeAnswer(Rc::clone(&answers)));
        let told = answers.borrow().clone();
        assert_eq!(told.len(), 1, "a read is answered at once, once");
        told.into_iter().next().flatten()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::test_support::{Answers, FakeAnswer, key_press, page_read, runtime_tab};
    use super::*;

    /// A tab whose activation is `activation`, with the activation uses logged; its gate offers
    /// script reads, as once `CLIPBOARD_READ_FRAME_GATE` is on.
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
        let gate = ClipboardGate::new(true);
        (TabClipboard::with_gate(TabId::new(7), access, gate), uses)
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
    fn a_tabs_script_read_is_refused_while_the_frame_gate_is_off() {
        let pasteboard = Rc::new(MemoryPasteboard::with_text("secret"));
        let access = Rc::new(ClipboardAccess::default());
        access.set_pasteboard(Rc::clone(&pasteboard) as Rc<dyn Pasteboard>);
        let uses = Rc::new(RefCell::new(Vec::new()));
        let logged = Rc::clone(&uses);
        access.set_user_activation_check(Rc::new(move |_tab_id, activation_use| {
            logged.borrow_mut().push(activation_use);
            true
        }));
        let clipboard = TabClipboard::with_gate(TabId::new(3), access, ClipboardGate::new(false));
        let answers = Answers::default();
        assert!(matches!(
            clipboard.read(FakeAnswer(Rc::clone(&answers))),
            ClipboardRead::Refused
        ));
        assert_eq!(*answers.borrow(), vec![None]);
        // The click is not used up by the refused read; a write in the same gesture still goes
        // through (and only looks at it).
        assert!(uses.borrow().is_empty());
        assert!(clipboard.write_text("copy button"));
        assert_eq!(*uses.borrow(), vec![ActivationUse::Peek]);
        // The user's own Paste reads.
        clipboard.user_command(ClipboardCommand::Paste);
        let answers = Answers::default();
        assert!(matches!(
            clipboard.read(FakeAnswer(Rc::clone(&answers))),
            ClipboardRead::Text(text) if text == "copy button"
        ));
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
    /// The pin for what tabs really start with (`TabClipboard::new`; the tests above build the
    /// gate by hand): no page may read the clipboard on its own, with or without a click, until
    /// `CLIPBOARD_READ_FRAME_GATE` is turned on. The password manager's copied passwords rely on
    /// it. Changed together with the turn-on checklist, not before.
    #[test]
    fn a_tab_as_the_runtime_builds_it_refuses_a_script_read_even_after_a_click() {
        let (clipboard, _pasteboard, uses) = runtime_tab("a copied password", true);

        // A script's `readText()` straight after a click: the case that would ask the user once
        // script reads are offered. It is refused, and the click is left alone.
        let answers = Answers::default();
        assert!(matches!(
            clipboard.read(FakeAnswer(Rc::clone(&answers))),
            ClipboardRead::Refused
        ));
        assert_eq!(*answers.borrow(), vec![None]);
        assert!(uses.borrow().is_empty());

        // Neither the user's Copy or Cut nor a new document changes that.
        clipboard.user_command(ClipboardCommand::Copy);
        clipboard.user_command(ClipboardCommand::Cut);
        clipboard.document_started();
        assert_eq!(page_read(&clipboard), None);
        assert!(uses.borrow().is_empty());
    }

    #[test]
    fn a_tab_as_the_runtime_builds_it_still_pastes_for_the_user_once() {
        let (clipboard, _pasteboard, uses) = runtime_tab("pasted text", true);
        clipboard.user_command(ClipboardCommand::Paste);
        assert_eq!(page_read(&clipboard).as_deref(), Some("pasted text"));
        // The paste is used up: whatever reads next, a script, is refused.
        assert_eq!(page_read(&clipboard), None);
        assert!(uses.borrow().is_empty());
    }

    #[test]
    fn without_a_noted_command_the_users_paste_would_insert_nothing() {
        // Why `key_pressed`, `context_menu_chosen` and the Edit menu's `user_command` must be
        // called for every command the user gives: Servo's Paste asks like any read.
        let (clipboard, _pasteboard, _uses) = runtime_tab("pasted text", false);
        assert_eq!(page_read(&clipboard), None);
    }

    #[test]
    fn the_command_key_with_v_is_the_users_paste() {
        let (clipboard, _pasteboard, _uses) = runtime_tab("pasted text", false);
        clipboard.key_pressed(&key_press("v", true));
        assert_eq!(page_read(&clipboard).as_deref(), Some("pasted text"));
        assert_eq!(page_read(&clipboard), None);
    }

    #[test]
    fn the_command_key_with_c_or_x_lets_the_page_write_what_the_user_copied() {
        for letter in ["c", "x"] {
            let (clipboard, pasteboard, _uses) = runtime_tab("old", false);
            assert!(!clipboard.write_text("before the key"), "{letter}");
            clipboard.key_pressed(&key_press(letter, true));
            // A copy the page handles itself clears, then writes.
            assert!(clipboard.clear(), "{letter}");
            assert!(clipboard.write_text("the selection"), "{letter}");
            assert_eq!(pasteboard.text().as_deref(), Some("the selection"));
            // A key press alone is not a paste.
            assert_eq!(page_read(&clipboard), None, "{letter}");
        }
    }

    #[test]
    fn plain_typing_is_not_a_clipboard_command() {
        let (clipboard, pasteboard, _uses) = runtime_tab("kept", false);
        for letter in ["v", "c", "x", "a"] {
            clipboard.key_pressed(&key_press(letter, false));
        }
        clipboard.key_pressed(&key_press("a", true));
        assert_eq!(page_read(&clipboard), None);
        assert!(!clipboard.write_text("typed"));
        assert_eq!(pasteboard.text().as_deref(), Some("kept"));
    }

    #[test]
    fn the_context_menus_paste_is_the_users_paste() {
        let (clipboard, _pasteboard, _uses) = runtime_tab("pasted text", false);
        clipboard.context_menu_chosen(ContextMenuAction::Paste);
        assert_eq!(page_read(&clipboard).as_deref(), Some("pasted text"));
        assert_eq!(page_read(&clipboard), None);
    }

    #[test]
    fn the_context_menus_copies_let_servo_write_the_clipboard() {
        for action in [
            ContextMenuAction::Copy,
            ContextMenuAction::Cut,
            ContextMenuAction::CopyLink,
            ContextMenuAction::CopyImageLink,
        ] {
            let (clipboard, pasteboard, _uses) = runtime_tab("old", false);
            clipboard.context_menu_chosen(action);
            assert!(clipboard.write_text("https://example.test/"), "{action:?}");
            assert_eq!(
                pasteboard.text().as_deref(),
                Some("https://example.test/"),
                "{action:?}"
            );
        }
    }

    #[test]
    fn the_other_context_menu_items_are_not_clipboard_commands() {
        let (clipboard, pasteboard, _uses) = runtime_tab("kept", false);
        for action in [
            ContextMenuAction::GoBack,
            ContextMenuAction::GoForward,
            ContextMenuAction::Reload,
            ContextMenuAction::OpenLinkInNewView,
            ContextMenuAction::OpenImageInNewView,
            ContextMenuAction::SelectAll,
        ] {
            clipboard.context_menu_chosen(action);
        }
        assert_eq!(page_read(&clipboard), None);
        assert!(!clipboard.write_text("page"));
        assert_eq!(pasteboard.text().as_deref(), Some("kept"));
    }
}
