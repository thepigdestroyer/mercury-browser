// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Password fields and input methods (txt-1).
//!
//! While the page has the keyboard and its focused text field is a password field, two things
//! hold, as in Safari and Chromium:
//!
//! - the input method is off for the window (`set_ime_allowed(false)`), so Pinyin or Kotoeri does
//!   not show what is typed in a candidate window, and the field gets plain key events;
//! - macOS secure event input is on (`EnableSecureEventInput`), so other processes cannot watch
//!   the keystrokes.
//!
//! Both are restored when the field loses the focus, another field gets it, the tab changes (the
//! text state is the active tab's), the page crashes (the state is cleared) or the page loses the
//! keyboard. Secure input is held only while the page has the keyboard: it is global to the
//! session, and leaving it on in the background would block other applications' input methods.
//!
//! [`Policy`] is pure and tested without a window. The platform calls are in [`apply`] and
//! [`set_secure_event_input`]; secure input is balanced (one enable, one disable) because the
//! system counts them.

use std::cell::RefCell;

use mercury_ui::MainWindow;
use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;

/// What changed, for the caller to apply. `None` means "leave it as it is".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Effects {
    /// `Some(allowed)`: call `set_ime_allowed(allowed)` on the window.
    pub(crate) ime_allowed: Option<bool>,
    /// `Some(on)`: enable or disable macOS secure event input.
    pub(crate) secure_input: Option<bool>,
}

/// The state behind the two switches.
#[derive(Debug, Default)]
pub(crate) struct Policy {
    page_focused: bool,
    password: bool,
    secure_input_held: bool,
}

impl Policy {
    /// The page gained or lost the keyboard. The input method follows the focus as before, except
    /// that a password field keeps it off.
    pub(crate) fn set_page_focused(&mut self, focused: bool) -> Effects {
        self.page_focused = focused;
        Effects {
            ime_allowed: Some(focused && !self.password),
            secure_input: self.secure_input_change(),
        }
    }

    /// The focused text field of the active tab is, or is no longer, a password field.
    pub(crate) fn set_password(&mut self, password: bool) -> Effects {
        if self.password == password {
            return Effects::default();
        }
        self.password = password;
        Effects {
            // Slint turns the input method on and off for its own text fields; the page's
            // setting is only touched while the page has the keyboard.
            ime_allowed: self.page_focused.then_some(!password),
            secure_input: self.secure_input_change(),
        }
    }

    pub(crate) fn secure_input_held(&self) -> bool {
        self.secure_input_held
    }

    fn secure_input_change(&mut self) -> Option<bool> {
        let wanted = self.page_focused && self.password;
        (wanted != self.secure_input_held).then(|| {
            self.secure_input_held = wanted;
            wanted
        })
    }
}

thread_local! {
    static POLICY: RefCell<Policy> = RefCell::new(Policy::default());
}

/// The web view gained or lost the keyboard (called with the window's own callback).
pub(crate) fn page_focus_changed(window: &MainWindow, focused: bool) {
    let effects = POLICY.with(|policy| policy.borrow_mut().set_page_focused(focused));
    apply(window, effects);
}

/// The active tab's focused text field is a password field, or not (called from the controls'
/// state sync, so it is cheap when nothing changed).
pub(crate) fn password_changed(window: &MainWindow, password: bool) {
    let effects = POLICY.with(|policy| policy.borrow_mut().set_password(password));
    apply(window, effects);
}

fn apply(window: &MainWindow, effects: Effects) {
    if let Some(allowed) = effects.ime_allowed {
        let _applied = window
            .window()
            .with_winit_window(|native_window| native_window.set_ime_allowed(allowed));
    }
    if let Some(on) = effects.secure_input {
        set_secure_event_input(on);
    }
}

#[cfg(all(target_os = "macos", not(test)))]
fn set_secure_event_input(on: bool) {
    // Carbon's HIToolbox: the one supported way to turn secure event input on (Terminal and the
    // password fields of AppKit use it). The system counts the calls, so they stay balanced.
    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn EnableSecureEventInput() -> i32;
        fn DisableSecureEventInput() -> i32;
    }
    // SAFETY: both functions take no arguments and only change a process-wide counter.
    let status = unsafe {
        if on {
            EnableSecureEventInput()
        } else {
            DisableSecureEventInput()
        }
    };
    if status != 0 {
        log::warn!("secure event input: the system answered {status}");
    }
}

#[cfg(all(not(target_os = "macos"), not(test)))]
fn set_secure_event_input(_on: bool) {}

#[cfg(test)]
thread_local! {
    static SECURE_CALLS: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn set_secure_event_input(on: bool) {
    SECURE_CALLS.with(|calls| calls.borrow_mut().push(on));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focusing_the_page_allows_the_input_method_as_before() {
        let mut policy = Policy::default();

        let effects = policy.set_page_focused(true);

        assert_eq!(effects.ime_allowed, Some(true));
        assert_eq!(effects.secure_input, None);
        assert_eq!(policy.set_page_focused(false).ime_allowed, Some(false));
    }

    #[test]
    fn a_password_field_turns_the_input_method_off_and_secure_input_on() {
        let mut policy = Policy::default();
        policy.set_page_focused(true);

        let effects = policy.set_password(true);

        assert_eq!(
            effects,
            Effects {
                ime_allowed: Some(false),
                secure_input: Some(true)
            }
        );
        assert!(policy.secure_input_held());
    }

    #[test]
    fn leaving_the_password_field_restores_both() {
        let mut policy = Policy::default();
        policy.set_page_focused(true);
        policy.set_password(true);

        let effects = policy.set_password(false);

        assert_eq!(
            effects,
            Effects {
                ime_allowed: Some(true),
                secure_input: Some(false)
            }
        );
        assert!(!policy.secure_input_held());
    }

    #[test]
    fn secure_input_is_released_when_the_page_loses_the_keyboard_and_taken_again_on_return() {
        let mut policy = Policy::default();
        policy.set_page_focused(true);
        policy.set_password(true);

        let lost = policy.set_page_focused(false);
        assert_eq!(lost.secure_input, Some(false));
        assert_eq!(lost.ime_allowed, Some(false));

        let back = policy.set_page_focused(true);
        assert_eq!(back.secure_input, Some(true));
        // The input method stays off: the focused field is still a password field.
        assert_eq!(back.ime_allowed, Some(false));
    }

    #[test]
    fn a_password_field_in_a_page_without_the_keyboard_touches_nothing() {
        let mut policy = Policy::default();

        let effects = policy.set_password(true);

        assert_eq!(effects, Effects::default());
        assert!(!policy.secure_input_held());
        // The keyboard arrives: now it applies.
        let focused = policy.set_page_focused(true);
        assert_eq!(focused.ime_allowed, Some(false));
        assert_eq!(focused.secure_input, Some(true));
    }

    #[test]
    fn calls_are_balanced_and_repeated_states_change_nothing() {
        let mut policy = Policy::default();
        let mut enables = 0_i32;
        let mut count = |effects: Effects| match effects.secure_input {
            Some(true) => enables += 1,
            Some(false) => enables -= 1,
            None => {}
        };
        count(policy.set_page_focused(true));
        count(policy.set_password(true));
        count(policy.set_password(true));
        count(policy.set_page_focused(true));
        count(policy.set_password(false));
        count(policy.set_password(false));
        count(policy.set_password(true));
        count(policy.set_page_focused(false));
        count(policy.set_page_focused(false));

        assert_eq!(enables, 0);
        assert!(!policy.secure_input_held());
    }

    #[test]
    fn a_tab_switch_or_a_crash_clears_the_password_state() {
        // The active tab's text state is `None` after a crash or for a tab without a field, so the
        // controls' sync reports "not a password": both switches are restored.
        let mut policy = Policy::default();
        policy.set_page_focused(true);
        policy.set_password(true);

        let effects = policy.set_password(false);

        assert_eq!(effects.secure_input, Some(false));
        assert_eq!(effects.ime_allowed, Some(true));
    }

    #[test]
    fn the_platform_call_is_recorded_in_tests_and_never_reaches_the_system() {
        SECURE_CALLS.with(|calls| calls.borrow_mut().clear());

        set_secure_event_input(true);
        set_secure_event_input(false);

        SECURE_CALLS.with(|calls| assert_eq!(*calls.borrow(), vec![true, false]));
    }
}
