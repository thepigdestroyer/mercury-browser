// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Which of a page's clipboard requests reach the clipboard.
//!
//! Servo 0.8.0 sends every clipboard access of a page to the embedder the same way, whether the
//! user chose Copy or Paste or a script called `navigator.clipboard`: the request names the
//! web view and nothing else. The browser therefore knows what the user asked for itself:
//! - Cut and Copy the user chose (Edit menu, its shortcuts, the page's context menu) allow the
//!   writes that follow within `USER_CLIPBOARD_ACTION_WINDOW`; Paste allows the first read that
//!   follows within it, and no writes;
//! - otherwise a page may write only while it has a user activation (a click or key press in
//!   it, see the desktop app's `activation.rs`); a script's read (`navigator.clipboard.
//!   readText()`) is refused while `CLIPBOARD_READ_FRAME_GATE` is off; once it is on, it is
//!   asked as `PagePermission::ClipboardRead` (only after such an activation) and allowed only
//!   if the user, or a decision remembered earlier, says yes;
//! - everything else is refused. Servo cannot tell a page that a write was refused (its
//!   `writeText()` promise resolves anyway), and a refused read gives the page empty text.
//!
//! `navigator.clipboard` itself is not offered to pages while `ASYNC_CLIPBOARD_OFFERED` is off, so
//! until then every request that reaches the gate is the user's own Cut, Copy or Paste.
//!
//! Known limit once the API is offered while the request names no frame: the tab's user
//! activation and the user's Cut and Copy also let the page's frames from other sites write, and
//! the user's Paste lets them read, in the short window each grants.

use std::cell::RefCell;
use std::time::Duration;

use crate::{ClipboardCommand, ContextMenuAction, KeyState, KeyboardInput};

/// How long after the user chose Cut, Copy or Paste the page's clipboard access for it is
/// allowed. Servo performs the edit on the page's script thread, which may be busy for a moment.
pub const USER_CLIPBOARD_ACTION_WINDOW: Duration = Duration::from_secs(2);

/// Whether pages get `navigator.clipboard` at all (Servo's `dom_async_clipboard_enabled`). Off,
/// as it was before the clipboard lane: the user's own Cut, Copy and Paste work, a site's own
/// Copy button does not.
///
/// Servo 0.8.0 names no frame in its clipboard requests, so with the API on, a frame from
/// another site (an advert) shares every window this module grants the page: it can overwrite
/// the clipboard within a second of any click or key press in the tab or within two seconds of
/// the user's Copy or Cut, and its `readText()` can race Servo's own read for the user's Paste
/// and get what the user is pasting (a password copied from the password manager, say). Turn on
/// either when the vendored `servo-script` frame gate patch lands (same change as
/// `CLIPBOARD_READ_FRAME_GATE`, see below), or when the owner has accepted that limit in
/// writing. Turning it on is one flip of this constant, plus:
/// - the preference test `async_clipboard_stays_disabled` and the count of enabled experimental
///   preferences in `talaria-servo`'s `runtime.rs` (the clipboard API is excluded from the
///   experimental features today);
/// - `tests/pages/clipboard.html` and its screenshot check in
///   `docs/developer-guide/testing.md` (the page then shows `navigator.clipboard: present`);
/// - the hands-on checklist (`docs/developer-guide/hands-on-checklist.md`, Clipboard), the user
///   guide (`docs/user-guide/privacy-and-security.md`, Clipboard) and the help text of
///   `--disable-experimental-web-features` in the desktop app's `cli.rs`.
pub const ASYNC_CLIPBOARD_OFFERED: bool = false;

/// The switch for a script's clipboard reads (`navigator.clipboard.readText()`) behind the
/// user's permission. Off until the vendored `servo-script` refuses the clipboard to a document
/// that is not same-origin with its top-level document (the frame gate engine patch planned for
/// geolocation, `GEOLOCATION_FRAME_GATE`, applied to `Clipboard::ReadText` too). Servo 0.8.0's
/// request names only the web view, so without the patch a cross-origin frame (an advert) would
/// read the clipboard under the top-level site's name, and silently for as long as a remembered
/// "Allow" exists for that site (the core can remember a permission decision; the desktop app
/// offers no remember choice for permission prompts today). Turn on in the change that lands
/// the patch.
///
/// Before turning it on:
/// - the vendored `servo-script` patch must refuse `Clipboard::ReadText` to any document that is
///   not same-origin with its top-level document (same patch family as `GEOLOCATION_FRAME_GATE`
///   in `page_request.rs`; procedure in `docs/developer-guide/vendored-patches.md`);
/// - Servo's user Paste does a blocking `recv` of `GetClipboardText` on the page's script thread
///   (`fire_clipboard_event`, servo-script 0.8.0 `dom/document/editing.rs`). A Paste read that
///   reaches the delegate after `USER_CLIPBOARD_ACTION_WINDOW` is therefore classified as a
///   script read and, with the gate on and a user activation in the page, would raise a
///   permission prompt while the script thread is blocked. The patch (or the grant's lifetime)
///   must make that impossible;
/// - text copied from Mercury's own password manager lands on this same clipboard, so a
///   user-approved read would expose it while it is there (passwords lane: it clears the
///   clipboard after a timeout; check that it is short enough);
/// - `ASYNC_CLIPBOARD_OFFERED` must be on, or no page has a `readText()` to ask with;
/// - flipping the constant also turns on `PagePermission::ClipboardRead::is_promptable`: update
///   the tests that pin it off (`script_reads_and_the_clipboard_api_stay_off` below, and
///   `clipboard_reading_is_not_asked_while_script_reads_are_off` in `tests/page_requests.rs`),
///   `docs/developer-guide/hands-on-checklist.md` (Clipboard) and
///   `docs/user-guide/privacy-and-security.md` (Clipboard), in the same change.
///
/// While it is off, the password manager's "copy password" is safe from a page's script: a
/// script's read is refused here (and, with `ASYNC_CLIPBOARD_OFFERED` off, no page has one).
pub const CLIPBOARD_READ_FRAME_GATE: bool = false;

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
#[derive(Debug)]
pub struct ClipboardGate {
    /// When the user last chose Cut or Copy (or a context-menu copy) in this tab.
    user_write_at: Option<Duration>,
    /// When the user last chose Paste in this tab; the first read after it uses it up.
    user_paste_at: Option<Duration>,
    /// Whether a script's read may ask the user (`CLIPBOARD_READ_FRAME_GATE`).
    script_reads: bool,
}

/// The gate a tab starts with: a script's reads are offered as `CLIPBOARD_READ_FRAME_GATE` says.
impl Default for ClipboardGate {
    fn default() -> Self {
        Self::new(CLIPBOARD_READ_FRAME_GATE)
    }
}

impl ClipboardGate {
    /// A gate with no grants; `script_reads`: whether a script's read after a user activation
    /// asks the user (otherwise it is refused).
    pub const fn new(script_reads: bool) -> Self {
        Self {
            user_write_at: None,
            user_paste_at: None,
            script_reads,
        }
    }

    /// The user chose `command` for this tab's page.
    pub fn user_command(&mut self, command: ClipboardCommand, now: Duration) {
        match command {
            ClipboardCommand::Cut | ClipboardCommand::Copy => self.user_write_at = Some(now),
            ClipboardCommand::Paste => self.user_paste_at = Some(now),
        }
    }

    /// Whether the page may write to (or clear) the clipboard now. `page_has_user_activation`:
    /// the page has a user activation that has not expired (a write does not use it up).
    pub fn may_write(&self, now: Duration, page_has_user_activation: bool) -> bool {
        page_has_user_activation || within_window(self.user_write_at, now)
    }

    /// Decides a read. A Paste the user chose is used up by the first read after it, even one
    /// too late to use it; otherwise, if script reads are offered, `take_user_activation` is
    /// called: it uses up the page's user activation if it has one and says whether it did, so
    /// that one click asks at most once. With script reads off it is not called, and the
    /// activation is left alone.
    pub fn read(
        &mut self,
        now: Duration,
        take_user_activation: impl FnOnce() -> bool,
    ) -> ClipboardReadDecision {
        if within_window(self.user_paste_at.take(), now) {
            ClipboardReadDecision::Read
        } else if self.script_reads && take_user_activation() {
            ClipboardReadDecision::AskPermission
        } else {
            ClipboardReadDecision::Refuse
        }
    }

    /// A new document started loading in the tab: the Cut, Copy and Paste noted for the last one
    /// do not carry over. Whether script reads are offered is the gate's, not the document's, and
    /// stays.
    pub fn document_started(&mut self) {
        *self = Self::new(self.script_reads);
    }
}

fn within_window(at: Option<Duration>, now: Duration) -> bool {
    at.is_some_and(|at| now.saturating_sub(at) < USER_CLIPBOARD_ACTION_WINDOW)
}

/// The clipboard edit a key press asks Servo to perform in the page: Command-X, C or V on
/// macOS (`command_is_meta`), Control-X, C or V elsewhere, with no other modifier, whether the
/// letter is upper or lower case and whether the key is held down or not. Servo maps the same
/// keys itself (`editing_action_from_keyboard_event`, exact modifiers, any case, auto-repeat
/// included); the browser notes them as the user's command when it forwards them, so that the
/// page's paste is not taken for a script's.
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
    use crate::{KeyLocation, KeyboardModifiers, PagePermission};

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
        let mut gate = ClipboardGate::new(true);
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
    fn a_paste_read_after_the_window_is_taken_for_a_script_read() {
        // Servo blocks the page's script thread for a user Paste (see
        // `CLIPBOARD_READ_FRAME_GATE`): one that reaches the gate too late is a script's read.
        let mut gate = ClipboardGate::new(true);
        gate.user_command(ClipboardCommand::Paste, ms(0));
        assert_eq!(
            gate.read(USER_CLIPBOARD_ACTION_WINDOW, || true),
            ClipboardReadDecision::AskPermission
        );
        // One just inside the window is still the user's.
        let mut gate = ClipboardGate::new(true);
        gate.user_command(ClipboardCommand::Paste, ms(0));
        assert_eq!(
            gate.read(USER_CLIPBOARD_ACTION_WINDOW - ms(1), || true),
            ClipboardReadDecision::Read
        );
    }

    #[test]
    fn a_script_read_after_an_activation_asks() {
        let mut gate = ClipboardGate::new(true);
        assert_eq!(
            gate.read(ms(0), || true),
            ClipboardReadDecision::AskPermission
        );
    }

    #[test]
    fn a_script_read_without_an_activation_is_refused() {
        let mut gate = ClipboardGate::new(true);
        assert_eq!(gate.read(ms(0), || false), ClipboardReadDecision::Refuse);
    }

    #[test]
    fn a_new_document_drops_the_users_commands() {
        let mut gate = ClipboardGate::new(true);
        gate.user_command(ClipboardCommand::Copy, ms(0));
        gate.user_command(ClipboardCommand::Paste, ms(0));
        gate.document_started();
        assert!(!gate.may_write(ms(1), false));
        assert_eq!(gate.read(ms(1), || false), ClipboardReadDecision::Refuse);
        // The switch is the gate's, not the document's.
        assert_eq!(
            gate.read(ms(1), || true),
            ClipboardReadDecision::AskPermission
        );
    }

    /// The pin for the password manager's "copy password" (a copied password sits on the same
    /// clipboard): no page may read the clipboard on its own until the engine names the frame that
    /// asks. This fails the moment either constant is turned on; whoever does that must work
    /// through the turn-on checklists in the doc comments of both constants (frame gate patch,
    /// blocking Paste, password manager, docs) and change this test in the same commit.
    #[test]
    fn script_reads_and_the_clipboard_api_stay_off() {
        let (api_offered, reads_gate) = (ASYNC_CLIPBOARD_OFFERED, CLIPBOARD_READ_FRAME_GATE);
        assert!(
            !api_offered,
            "ASYNC_CLIPBOARD_OFFERED is on: see its doc comment for what must hold first"
        );
        assert!(
            !reads_gate,
            "CLIPBOARD_READ_FRAME_GATE is on: see its doc comment for what must hold first \
             (frame gate engine patch, the blocking Paste, the password manager)"
        );
        // What the tabs actually use, not just the constants.
        assert!(
            !ClipboardGate::default().script_reads,
            "a tab's gate offers script reads: the password manager's copied passwords are exposed"
        );
        assert!(
            !PagePermission::ClipboardRead.is_promptable(),
            "a script's clipboard read would be asked"
        );
    }

    #[test]
    fn a_script_read_is_refused_by_the_gate_a_tab_starts_with() {
        // `ClipboardGate::default()` is what every tab starts with.
        let mut gate = ClipboardGate::default();
        let mut activation_taken = false;
        assert_eq!(
            gate.read(ms(0), || {
                activation_taken = true;
                true
            }),
            ClipboardReadDecision::Refuse
        );
        // The click is left for whatever else the page does with it (a popup, a write).
        assert!(!activation_taken);
        // The user's own Paste still reads.
        gate.user_command(ClipboardCommand::Paste, ms(10));
        assert_eq!(gate.read(ms(20), || false), ClipboardReadDecision::Read);
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
