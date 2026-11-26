// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the View menu needs to know about the window's layout: which of the tab ring, the dock
//! and the dial it can offer right now. The code that pushes the plan and drives the dock and the
//! dial (`layout_switch` tasks 7 and 8) writes it with `set_menu_state`; `menus::sync` and the
//! native menu read it with `menu_state`. Until something writes it, the window is today's: no
//! ring, no dock, no dial, so the three View items are disabled.

use std::cell::Cell;

/// The layout facts behind the View menu's tab-group items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LayoutMenuState {
    /// The hub button is on screen, so "Show Tab Ring" has a ring to open.
    pub(crate) ring_available: bool,
    /// The dock is on screen, so "Shrink Dock" / "Show Dock" has a band to change.
    pub(crate) dock_available: bool,
    /// The dock is shrunk to its small band (the menu item then reads "Show Dock").
    pub(crate) dock_shrunk: bool,
    /// The corner dial is on in this window, so "Show Tab Dial" has a dial to open.
    pub(crate) dial_enabled: bool,
}

thread_local! {
    static STATE: Cell<LayoutMenuState> = const { Cell::new(LayoutMenuState {
        ring_available: false,
        dock_available: false,
        dock_shrunk: false,
        dial_enabled: false,
    }) };
}

/// What the menu shows now (the last `set_menu_state` of this thread; nothing is available
/// before the first).
pub(crate) fn menu_state() -> LayoutMenuState {
    STATE.with(Cell::get)
}

/// Records what the window offers; called whenever the plan, the dock or the dial changes.
pub(crate) fn set_menu_state(state: LayoutMenuState) {
    STATE.with(|stored| stored.set(state));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_available_until_the_window_says_so() {
        assert_eq!(menu_state(), LayoutMenuState::default());
        let state = menu_state();
        assert!(!state.ring_available);
        assert!(!state.dock_available);
        assert!(!state.dock_shrunk);
        assert!(!state.dial_enabled);
    }

    #[test]
    fn the_last_state_set_is_the_one_read() {
        let first = LayoutMenuState {
            ring_available: true,
            dock_available: true,
            dock_shrunk: false,
            dial_enabled: true,
        };
        set_menu_state(first);
        assert_eq!(menu_state(), first);

        let second = LayoutMenuState {
            dock_shrunk: true,
            ..first
        };
        set_menu_state(second);
        assert_eq!(menu_state(), second);
    }
}
