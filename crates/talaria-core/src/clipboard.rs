//! Which of a page's clipboard requests reach the clipboard.
//!
//! Servo 0.8.0 sends every clipboard access of a page to the embedder the same way, whether the
//! user chose Copy or Paste or a script called `navigator.clipboard`: the request names the
//! web view and nothing else. The browser therefore knows what the user asked for itself:
//! - Cut, Copy and Paste the user chose (Edit menu, its shortcuts, the page's context menu)
//!   allow the writes, and one read, that follow within `USER_CLIPBOARD_ACTION_WINDOW`;
//! - otherwise a page may write only while it has a user activation (a click or key press in
//!   it, see the desktop app's `activation.rs`), and may read only after such an activation and
//!   the user's permission (`PagePermission::ClipboardRead`), asked for each read unless the
//!   user chose to remember the answer for the site;
//! - everything else is refused. Servo cannot tell a page that a write was refused (its
//!   `writeText()` promise resolves anyway), and a refused read resolves with empty text.

use std::cell::RefCell;
use std::time::Duration;

use crate::{ClipboardCommand, ContextMenuAction, KeyState, KeyboardInput};

/// How long after the user chose Cut, Copy or Paste the page's clipboard access for it is
/// allowed. Servo performs the edit on the page's script thread, which may be busy for a moment.
pub const USER_CLIPBOARD_ACTION_WINDOW: Duration = Duration::from_secs(2);

/// The text clipboard pages read and write. The browser uses the system clipboard in a normal
/// run and a `MemoryPasteboard` in automated runs and tests, which must never read or change
/// the user's clipboard.
pub trait Pasteboard {
    /// The plain text on the clipboard, if any.
    fn read_text(&self) -> Option<String>;
    /// Replaces the clipboard's contents with `text`.
    fn write_text(&self, text: &str);
    /// Empties the clipboard.
    fn clear(&self);
}

/// A clipboard that lives in memory only.
#[derive(Debug, Default)]
pub struct MemoryPasteboard {
    text: RefCell<Option<String>>,
}

impl MemoryPasteboard {
    pub fn with_text(text: impl Into<String>) -> Self {
        Self {
            text: RefCell::new(Some(text.into())),
        }
    }

    /// What the clipboard holds.
    pub fn text(&self) -> Option<String> {
        self.text.borrow().clone()
    }
}

impl Pasteboard for MemoryPasteboard {
    fn read_text(&self) -> Option<String> {
        self.text()
    }

    fn write_text(&self, text: &str) {
        *self.text.borrow_mut() = Some(text.to_owned());
    }

    fn clear(&self) {
        *self.text.borrow_mut() = None;
    }
}

/// What to do with a page's request to read the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardReadDecision {
    /// The user chose Paste: give the page the clipboard's text.
    Read,
    /// A script asked right after a user activation: ask the user
    /// (`PagePermission::ClipboardRead`).
    AskPermission,
    /// Give the page no text.
    Refuse,
}

/// One tab's clipboard state. Times are on one monotonic clock chosen by the caller.
#[derive(Debug, Default)]
pub struct ClipboardGate {
    /// When the user last chose Cut or Copy (or a context-menu copy) in this tab.
    user_write_at: Option<Duration>,
    /// When the user last chose Paste in this tab and the page has not read it yet.
    user_paste_at: Option<Duration>,
}

impl ClipboardGate {
    /// The user chose `command` for this tab's page.
    pub fn user_command(&mut self, command: ClipboardCommand, now: Duration) {
        match command {
            ClipboardCommand::Cut | ClipboardCommand::Copy => self.user_write_at = Some(now),
            ClipboardCommand::Paste => self.user_paste_at = Some(now),
        }
    }

    /// Whether the page may write to (or clear) the clipboard now. `page_has_user_activation`:
    /// the page has a user activation that has not expired; it is not used up.
    pub fn may_write(&self, now: Duration, page_has_user_activation: bool) -> bool {
        page_has_user_activation || within_window(self.user_write_at, now)
    }

    /// Decides a read. A Paste the user chose is used up by the first read; otherwise
    /// `take_user_activation` is called, and uses up the page's activation if it has one, so
    /// that one click asks at most once.
    pub fn read(
        &mut self,
        now: Duration,
        take_user_activation: impl FnOnce() -> bool,
    ) -> ClipboardReadDecision {
        if within_window(self.user_paste_at.take(), now) {
            ClipboardReadDecision::Read
        } else if take_user_activation() {
            ClipboardReadDecision::AskPermission
        } else {
            ClipboardReadDecision::Refuse
        }
    }

    /// A new document started loading in the tab: the last one's grants do not carry over.
    pub fn document_started(&mut self) {
        *self = Self::default();
    }
}

fn within_window(at: Option<Duration>, now: Duration) -> bool {
    at.is_some_and(|at| now.saturating_sub(at) < USER_CLIPBOARD_ACTION_WINDOW)
}

/// The clipboard edit a key press asks Servo to perform in the page: Command-X, C or V on
/// macOS (`command_is_meta`), Control-X, C or V elsewhere, with no other modifier, in either
/// case, held or not. Servo maps the same keys itself (`editing_action_from_keyboard_event`,
/// exact modifiers, any case, auto-repeat included); the browser notes them as the
/// user's command when it forwards them, so that the page's paste is not taken for a script's.
pub fn clipboard_shortcut(
    input: &KeyboardInput,
    command_is_meta: bool,
) -> Option<ClipboardCommand> {
    let modifiers = input.modifiers;
    let command_only = !modifiers.alt
        && !modifiers.shift
        && if command_is_meta {
            modifiers.meta && !modifiers.control
        } else {
            modifiers.control && !modifiers.meta
        };
    if input.state != KeyState::Down || !command_only {
        return None;
    }
    match input.key.as_str() {
        "x" | "X" => Some(ClipboardCommand::Cut),
        "c" | "C" => Some(ClipboardCommand::Copy),
        "v" | "V" => Some(ClipboardCommand::Paste),
        _ => None,
    }
}

/// The clipboard edit a context-menu item performs: its Cut, Copy and Paste, and the copying of a
/// link or image address, which Servo writes to the clipboard itself.
pub const fn context_menu_clipboard_command(action: ContextMenuAction) -> Option<ClipboardCommand> {
    match action {
        ContextMenuAction::Cut => Some(ClipboardCommand::Cut),
        ContextMenuAction::Copy
        | ContextMenuAction::CopyLink
        | ContextMenuAction::CopyImageLink => Some(ClipboardCommand::Copy),
        ContextMenuAction::Paste => Some(ClipboardCommand::Paste),
        ContextMenuAction::GoBack
        | ContextMenuAction::GoForward
        | ContextMenuAction::Reload
        | ContextMenuAction::OpenLinkInNewView
        | ContextMenuAction::OpenImageInNewView
        | ContextMenuAction::SelectAll => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeyLocation, KeyboardModifiers};

    const fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn a_memory_pasteboard_keeps_text_in_memory() {
        let pasteboard = MemoryPasteboard::default();
        assert_eq!(pasteboard.read_text(), None);
        pasteboard.write_text("copied");
        assert_eq!(pasteboard.read_text().as_deref(), Some("copied"));
        pasteboard.clear();
        assert_eq!(pasteboard.read_text(), None);
        assert_eq!(
            MemoryPasteboard::with_text("seed").text().as_deref(),
            Some("seed")
        );
    }

    #[test]
    fn a_page_without_activation_or_user_copy_may_not_write() {
        let gate = ClipboardGate::default();
        assert!(!gate.may_write(ms(10), false));
    }

    #[test]
    fn a_page_with_activation_may_write_and_keeps_it() {
        let gate = ClipboardGate::default();
        assert!(gate.may_write(ms(10), true));
        assert!(gate.may_write(ms(10), true));
    }

    #[test]
    fn copy_and_cut_allow_writes_within_the_window() {
        for command in [ClipboardCommand::Copy, ClipboardCommand::Cut] {
            let mut gate = ClipboardGate::default();
            gate.user_command(command, ms(1_000));
            // A copy the page handles itself clears the clipboard, then writes: both pass.
            assert!(gate.may_write(ms(1_000), false));
            assert!(gate.may_write(ms(1_000) + USER_CLIPBOARD_ACTION_WINDOW - ms(1), false));
            assert!(!gate.may_write(ms(1_000) + USER_CLIPBOARD_ACTION_WINDOW, false));
        }
    }

    #[test]
    fn copy_does_not_allow_a_read() {
        let mut gate = ClipboardGate::default();
        gate.user_command(ClipboardCommand::Copy, ms(0));
        assert_eq!(gate.read(ms(0), || false), ClipboardReadDecision::Refuse);
    }

    #[test]
    fn paste_allows_one_read_without_asking() {
        let mut gate = ClipboardGate::default();
        gate.user_command(ClipboardCommand::Paste, ms(500));
        let mut activation_taken = false;
        assert_eq!(
            gate.read(ms(600), || {
                activation_taken = true;
                true
            }),
            ClipboardReadDecision::Read
        );
        // The user's paste is not a script's read: the page's activation stays.
        assert!(!activation_taken);
        assert_eq!(gate.read(ms(700), || false), ClipboardReadDecision::Refuse);
    }

    #[test]
    fn paste_does_not_allow_writes() {
        let mut gate = ClipboardGate::default();
        gate.user_command(ClipboardCommand::Paste, ms(0));
        assert!(!gate.may_write(ms(0), false));
    }

    #[test]
    fn an_old_paste_is_not_used() {
        let mut gate = ClipboardGate::default();
        gate.user_command(ClipboardCommand::Paste, ms(0));
        assert_eq!(
            gate.read(USER_CLIPBOARD_ACTION_WINDOW, || false),
            ClipboardReadDecision::Refuse
        );
        // It is gone, not waiting for a later read.
        assert_eq!(
            gate.read(USER_CLIPBOARD_ACTION_WINDOW, || false),
            ClipboardReadDecision::Refuse
        );
    }

    #[test]
    fn a_script_read_after_an_activation_asks() {
        let mut gate = ClipboardGate::default();
        assert_eq!(
            gate.read(ms(0), || true),
            ClipboardReadDecision::AskPermission
        );
    }

    #[test]
    fn a_script_read_without_an_activation_is_refused() {
        let mut gate = ClipboardGate::default();
        assert_eq!(gate.read(ms(0), || false), ClipboardReadDecision::Refuse);
    }

    #[test]
    fn a_new_document_drops_the_users_commands() {
        let mut gate = ClipboardGate::default();
        gate.user_command(ClipboardCommand::Copy, ms(0));
        gate.user_command(ClipboardCommand::Paste, ms(0));
        gate.document_started();
        assert!(!gate.may_write(ms(1), false));
        assert_eq!(gate.read(ms(1), || false), ClipboardReadDecision::Refuse);
    }

    #[test]
    fn a_clock_that_goes_back_does_not_extend_a_grant() {
        let mut gate = ClipboardGate::default();
        gate.user_command(ClipboardCommand::Copy, ms(5_000));
        // Not expected of a monotonic clock, but an earlier time is not "within the window"
        // forever either: it counts as no time elapsed.
        assert!(gate.may_write(ms(4_000), false));
    }

    fn key(key: &str, modifiers: KeyboardModifiers) -> KeyboardInput {
        KeyboardInput {
            state: KeyState::Down,
            key: key.to_owned(),
            code: String::new(),
            location: KeyLocation::Standard,
            modifiers,
            repeat: false,
            is_composing: false,
        }
    }

    const META: KeyboardModifiers = KeyboardModifiers {
        alt: false,
        control: false,
        meta: true,
        shift: false,
    };
    const CONTROL: KeyboardModifiers = KeyboardModifiers {
        alt: false,
        control: true,
        meta: false,
        shift: false,
    };

    #[test]
    fn command_keys_are_clipboard_shortcuts_on_macos() {
        assert_eq!(
            clipboard_shortcut(&key("c", META), true),
            Some(ClipboardCommand::Copy)
        );
        assert_eq!(
            clipboard_shortcut(&key("x", META), true),
            Some(ClipboardCommand::Cut)
        );
        assert_eq!(
            clipboard_shortcut(&key("v", META), true),
            Some(ClipboardCommand::Paste)
        );
        // Caps Lock gives an upper-case letter.
        assert_eq!(
            clipboard_shortcut(&key("V", META), true),
            Some(ClipboardCommand::Paste)
        );
        assert_eq!(clipboard_shortcut(&key("v", CONTROL), true), None);
    }

    #[test]
    fn control_keys_are_clipboard_shortcuts_elsewhere() {
        assert_eq!(
            clipboard_shortcut(&key("v", CONTROL), false),
            Some(ClipboardCommand::Paste)
        );
        assert_eq!(clipboard_shortcut(&key("v", META), false), None);
    }

    #[test]
    fn context_menu_copies_and_pastes_are_the_users_commands() {
        assert_eq!(
            context_menu_clipboard_command(ContextMenuAction::CopyLink),
            Some(ClipboardCommand::Copy)
        );
        assert_eq!(
            context_menu_clipboard_command(ContextMenuAction::CopyImageLink),
            Some(ClipboardCommand::Copy)
        );
        assert_eq!(
            context_menu_clipboard_command(ContextMenuAction::Copy),
            Some(ClipboardCommand::Copy)
        );
        assert_eq!(
            context_menu_clipboard_command(ContextMenuAction::Cut),
            Some(ClipboardCommand::Cut)
        );
        assert_eq!(
            context_menu_clipboard_command(ContextMenuAction::Paste),
            Some(ClipboardCommand::Paste)
        );
        for action in [
            ContextMenuAction::GoBack,
            ContextMenuAction::GoForward,
            ContextMenuAction::Reload,
            ContextMenuAction::OpenLinkInNewView,
            ContextMenuAction::OpenImageInNewView,
            ContextMenuAction::SelectAll,
        ] {
            assert_eq!(context_menu_clipboard_command(action), None, "{action:?}");
        }
    }

    #[test]
    fn a_held_shortcut_repeats_as_servo_repeats_the_edit() {
        let repeated = KeyboardInput {
            repeat: true,
            ..key("v", META)
        };
        assert_eq!(
            clipboard_shortcut(&repeated, true),
            Some(ClipboardCommand::Paste)
        );
    }

    #[test]
    fn other_keys_releases_and_extra_modifiers_are_not_shortcuts() {
        assert_eq!(
            clipboard_shortcut(&key("v", KeyboardModifiers::default()), true),
            None
        );
        assert_eq!(clipboard_shortcut(&key("a", META), true), None);
        let shifted = KeyboardModifiers {
            shift: true,
            ..META
        };
        assert_eq!(clipboard_shortcut(&key("V", shifted), true), None);
        let with_option = KeyboardModifiers { alt: true, ..META };
        assert_eq!(clipboard_shortcut(&key("v", with_option), true), None);
        let released = KeyboardInput {
            state: KeyState::Up,
            ..key("v", META)
        };
        assert_eq!(clipboard_shortcut(&released, true), None);
    }
}
