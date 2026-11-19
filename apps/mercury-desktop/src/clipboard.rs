// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

// The clipboard pages use (clipboard lane).
//
// A normal run gives pages the system clipboard, behind the engine's gate (`talaria_core::
// ClipboardGate`: writes after a user gesture, a script's reads only once the engine names the
// frame that asks, and the Cut, Copy and Paste the user chooses). Automated runs
// (`--screenshot`, `--background`, `--import-from`) and tests get a clipboard in memory, so that
// they never read or overwrite the user's: in such a run `MERCURY_TEST_CLIPBOARD_TEXT` sets its
// starting text, for test pages that check a read is refused.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use talaria_core::{MemoryPasteboard, Pasteboard};
use talaria_servo::{ActivationUse, ServoRuntime};

use crate::activation::{consume_user_activation, has_user_activation};

/// The starting text of an automated run's clipboard.
pub(crate) const TEST_CLIPBOARD_TEXT_VARIABLE: &str = "MERCURY_TEST_CLIPBOARD_TEXT";

thread_local! {
    /// Until `configure` says otherwise, a run is treated as automated: never the real clipboard.
    static AUTOMATED: Cell<bool> = const { Cell::new(true) };
}

/// Called once at start, before the engine is connected.
pub(crate) fn configure(automated: bool) {
    AUTOMATED.with(|cell| cell.set(automated));
}

/// Installs the run's clipboard and the activation check in the engine.
pub(crate) fn connect(runtime: &ServoRuntime) {
    let automated = AUTOMATED.with(Cell::get);
    let seed = std::env::var(TEST_CLIPBOARD_TEXT_VARIABLE).ok();
    runtime.set_pasteboard(pasteboard(pasteboard_for_run(automated, seed)));
    runtime.set_user_activation_check(Rc::new(|tab, activation_use| match activation_use {
        ActivationUse::Peek => has_user_activation(tab, Instant::now()),
        ActivationUse::Take => consume_user_activation(tab, Instant::now()),
    }));
}

#[derive(Debug, PartialEq, Eq)]
enum PasteboardChoice {
    System,
    Memory(Option<String>),
}

fn pasteboard_for_run(automated: bool, seed: Option<String>) -> PasteboardChoice {
    if automated {
        PasteboardChoice::Memory(seed)
    } else {
        PasteboardChoice::System
    }
}

fn pasteboard(choice: PasteboardChoice) -> Rc<dyn Pasteboard> {
    match choice {
        PasteboardChoice::System => Rc::new(SystemPasteboard::default()),
        PasteboardChoice::Memory(Some(text)) => Rc::new(MemoryPasteboard::with_text(text)),
        PasteboardChoice::Memory(None) => Rc::new(MemoryPasteboard::default()),
    }
}

/// The system clipboard (`NSPasteboard.generalPasteboard` on macOS), opened at its first use.
#[derive(Default)]
struct SystemPasteboard {
    clipboard: RefCell<Option<arboard::Clipboard>>,
    /// Set once the "not available" warning has been logged: a machine without a clipboard
    /// (headless Linux, Wayland without data-control) fails at every access, and one warning per
    /// run is enough; later failures are logged at debug level.
    warned: Cell<bool>,
}

impl SystemPasteboard {
    fn with<T>(
        &self,
        action: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
    ) -> Option<T> {
        let mut clipboard = self.clipboard.borrow_mut();
        if clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(opened) => *clipboard = Some(opened),
                Err(error) => {
                    if self.warned.replace(true) {
                        log::debug!("the system clipboard is not available: {error}");
                    } else {
                        log::warn!("the system clipboard is not available: {error}");
                    }
                    return None;
                }
            }
        }
        let result = action(clipboard.as_mut()?);
        result
            .map_err(|error| log::debug!("clipboard access failed: {error}"))
            .ok()
    }
}

impl Pasteboard for SystemPasteboard {
    fn read_text(&self) -> Option<String> {
        self.with(|clipboard| clipboard.get_text())
    }

    fn write_text(&self, text: &str) {
        let _written = self.with(|clipboard| clipboard.set_text(text));
    }

    fn clear(&self) {
        let _cleared = self.with(arboard::Clipboard::clear);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automated_runs_get_a_clipboard_in_memory() {
        assert_eq!(
            pasteboard_for_run(true, None),
            PasteboardChoice::Memory(None)
        );
        assert_eq!(
            pasteboard_for_run(true, Some("seed".to_owned())),
            PasteboardChoice::Memory(Some("seed".to_owned()))
        );
    }

    #[test]
    fn only_a_normal_run_gets_the_system_clipboard_and_the_test_text_is_ignored_there() {
        assert_eq!(
            pasteboard_for_run(false, Some("seed".to_owned())),
            PasteboardChoice::System
        );
    }

    #[test]
    fn a_seeded_memory_clipboard_starts_with_the_text() {
        let pasteboard = pasteboard(PasteboardChoice::Memory(Some("seed".to_owned())));
        assert_eq!(pasteboard.read_text().as_deref(), Some("seed"));
        pasteboard.write_text("page");
        assert_eq!(pasteboard.read_text().as_deref(), Some("page"));
    }

    #[test]
    fn runs_are_automated_until_configured() {
        assert!(AUTOMATED.with(Cell::get));
    }
}
