// SPDX-License-Identifier: MPL-2.0
use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use mercury_ui::MainWindow;
use slint::ComponentHandle;
use slint::winit_030::winit;
use talaria_core::{BrowserCommand, BrowserController};
use winit::event::ElementState;
use winit::keyboard::{Key, ModifiersState, NamedKey, PhysicalKey};

use crate::ui_sync::sync_ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    MacOs,
    Other,
}

impl Platform {
    pub(crate) const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShortcutAction {
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    /// Selects the tab at this zero-based index.
    SelectTab(usize),
    SelectLastTab,
    Back,
    Forward,
    Reload,
    FocusAddressBar,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Quit,
    /// Escape while the active tab's page is fullscreen.
    ExitFullscreen,
    // downloads-ui lane
    /// Save page as… (the page's HTML only).
    SavePage,
    /// Opens or closes the list of downloads.
    ShowDownloads,
    // settings-ui
    ShowSettings,
    // tabs-plus
    /// Opens tab search.
    SearchTabs,
    /// Moves the active tab one place left or right within its group.
    MoveTabLeft,
    MoveTabRight,
    /// Opens the browser menu (Alt+F, Alt+E or F10 off macOS, as other browsers), the
    /// keyboard path to the tab commands where there is no menu bar.
    ShowBrowserMenu,
    // history
    ShowHistory,
    ClearBrowsingData,
    // bookmarks
    /// Cmd+D (Ctrl+D): the star.
    BookmarkPage,
    /// Cmd+Shift+B (Ctrl+Shift+B).
    ToggleBookmarksBar,
    /// Cmd+Alt+B on macOS, as Safari and Chrome; Ctrl+Shift+O elsewhere, as Chrome.
    ShowBookmarksManager,
    // session
    /// Brings back the most recently closed tab where it was.
    ReopenClosedTab,
    /// Opens the active tab's address again in a new tab just after it. No default shortcut;
    /// File ▸ "Duplicate Tab", the browser menu and the tab menu.
    DuplicateTab,
    /// Restores the last run's tabs when they were not restored at start-up. No default shortcut;
    /// History ▸ "Restore Previous Session".
    RestorePreviousSession,
    // media-session lane
    /// Cmd+Shift+M on macOS (Cmd+M stays Minimize), Ctrl+M elsewhere (as Firefox and Edge).
    ToggleMute,
    // find
    Find,
    FindNext,
    FindPrevious,
    // sh-pages
    /// Cmd+Option+U on macOS, Ctrl+U elsewhere: the page's source in a new tab
    /// (`view_source::shortcut`).
    ViewSource,
    // layout-switch
    /// View ▸ "Show Tab Ring": opens the hub's ring popover. No key.
    ShowTabRing,
    /// Ctrl+Cmd+B on macOS, Ctrl+Alt+B elsewhere: shrinks the dock to its small band, or opens it
    /// again.
    ToggleDock,
    /// Cmd+Shift+E on macOS, Ctrl+Shift+E elsewhere: opens or closes the corner dial's fan.
    ToggleDial,
}

impl ShortcutAction {
    /// Whether holding the keys down repeats the action. Repeats of other shortcuts are still
    /// consumed, so the page never sees them, but do nothing.
    pub(crate) const fn repeats(self) -> bool {
        matches!(
            self,
            Self::ZoomIn
                | Self::ZoomOut
                | Self::NextTab
                | Self::PreviousTab
                // find
                | Self::FindNext
                | Self::FindPrevious
        )
    }
}

/// What to do with one key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyDisposition {
    /// Not a browser shortcut: give it to whoever has focus.
    Forward,
    /// Part of a shortcut that has already run: swallow it.
    Consume,
    /// Run this shortcut and swallow the event.
    Run(ShortcutAction),
}

/// Pairs key releases (and auto-repeats) with presses: a release is swallowed exactly when its
/// press ran a shortcut, whatever modifiers are held by then, and every other release is
/// forwarded. Shortcuts only start on a first press, never on a repeat.
#[derive(Debug, Default)]
pub(crate) struct ShortcutKeys {
    consumed: HashSet<PhysicalKey>,
}

impl ShortcutKeys {
    /// `action` is the shortcut the key would trigger with the modifiers held right now.
    pub(crate) fn press(
        &mut self,
        key: PhysicalKey,
        repeat: bool,
        action: Option<ShortcutAction>,
    ) -> KeyDisposition {
        if repeat {
            if !self.consumed.contains(&key) {
                return KeyDisposition::Forward;
            }
            return match action {
                Some(action) if action.repeats() => KeyDisposition::Run(action),
                _ => KeyDisposition::Consume,
            };
        }

        match action {
            Some(action) => {
                self.consumed.insert(key);
                KeyDisposition::Run(action)
            }
            None => {
                self.consumed.remove(&key);
                KeyDisposition::Forward
            }
        }
    }

    pub(crate) fn release(&mut self, key: PhysicalKey) -> KeyDisposition {
        if self.consumed.remove(&key) {
            KeyDisposition::Consume
        } else {
            KeyDisposition::Forward
        }
    }

    /// Forgets every pressed shortcut key, for when key-ups can no longer be relied on.
    pub(crate) fn clear(&mut self) {
        self.consumed.clear();
    }

    // menus
    /// The native menu bar ran an action for this key's press, which therefore never reached
    /// the window: swallow its release as if the press had run a shortcut here.
    pub(crate) fn claim(&mut self, key: PhysicalKey) {
        self.consumed.insert(key);
    }
}

/// Maps a key press to a browser shortcut. "Primary" is Cmd on macOS and Ctrl elsewhere; the
/// other of the two only takes part in Ctrl+Tab, so macOS Ctrl combinations reach the page.
pub(crate) fn shortcut_action(
    key: &Key,
    modifiers: ModifiersState,
    platform: Platform,
) -> Option<ShortcutAction> {
    use ShortcutAction::*;

    let control = modifiers.control_key();
    let shift = modifiers.shift_key();
    let alt = modifiers.alt_key();
    let command = modifiers.super_key();

    if matches!(key, Key::Named(NamedKey::Tab)) && control && !alt && !command {
        return Some(if shift { PreviousTab } else { NextTab });
    }

    let (primary, secondary) = match platform {
        Platform::MacOs => (command, control),
        Platform::Other => (control, command),
    };
    let character = match key {
        Key::Character(character) => Some(character.to_lowercase()),
        _ => None,
    };
    let character = character.as_deref();

    // layout-switch: the dock's chord has the platform's other modifier in it, so it comes first
    if let Some(action) = dock_shortcut(character, modifiers, platform) {
        return Some(action);
    }
    if secondary {
        return None;
    }

    // bookmarks lane
    if let Some(action) = bookmark_shortcut(character, primary, shift, alt, platform) {
        return Some(action);
    }
    // sh-pages
    if let Some(action) = crate::view_source::shortcut(character, primary, shift, alt, platform) {
        return Some(action);
    }

    if primary && !alt {
        let common = match (character, shift) {
            (Some("t"), false) => Some(NewTab),
            // session
            (Some("t"), true) => Some(ReopenClosedTab),
            (Some("w"), false) => Some(CloseTab),
            (Some("r"), false) => Some(Reload),
            (Some("l"), false) => Some(FocusAddressBar),
            (Some("q"), false) => Some(Quit),
            (Some("=" | "+"), _) => Some(ZoomIn),
            (Some("-"), false) => Some(ZoomOut),
            (Some("0"), false) => Some(ZoomReset),
            (Some("9"), false) => Some(SelectLastTab),
            (Some("s"), false) => Some(SavePage), // downloads-ui lane
            // settings-ui
            (Some(","), false) => Some(ShowSettings),
            // tabs-plus
            (Some("a"), true) => Some(SearchTabs),
            // media-session lane
            (Some("m"), true) if platform == Platform::MacOs => Some(ToggleMute),
            (Some("m"), false) if platform == Platform::Other => Some(ToggleMute),
            // find
            (Some("f"), false) => Some(Find),
            (Some("g"), false) => Some(FindNext),
            (Some("g"), true) => Some(FindPrevious),
            // layout-switch
            (Some("e"), true) => Some(ToggleDial),
            (Some(digit), false) => match digit.parse::<usize>() {
                Ok(number @ 1..=8) => Some(SelectTab(number - 1)),
                _ => None,
            },
            _ => None,
        };
        if common.is_some() {
            return common;
        }

        return match (platform, key, character, shift) {
            (Platform::MacOs, _, Some("j"), true) => Some(ShowDownloads), // downloads-ui lane
            (Platform::Other, _, Some("j"), false) => Some(ShowDownloads), // downloads-ui lane
            (Platform::MacOs, _, Some("["), false) => Some(Back),
            (Platform::MacOs, _, Some("]"), false) => Some(Forward),
            (Platform::MacOs, _, Some("[" | "{"), true) => Some(PreviousTab),
            (Platform::MacOs, _, Some("]" | "}"), true) => Some(NextTab),
            (Platform::MacOs, Key::Named(NamedKey::ArrowLeft), _, false) => Some(Back),
            (Platform::MacOs, Key::Named(NamedKey::ArrowRight), _, false) => Some(Forward),
            (Platform::Other, Key::Named(NamedKey::PageDown), _, false) => Some(NextTab),
            (Platform::Other, Key::Named(NamedKey::PageUp), _, false) => Some(PreviousTab),
            // tabs-plus
            (Platform::Other, Key::Named(NamedKey::PageUp), _, true) => Some(MoveTabLeft),
            (Platform::Other, Key::Named(NamedKey::PageDown), _, true) => Some(MoveTabRight),
            // history lane
            (Platform::MacOs, _, Some("y"), false) => Some(ShowHistory),
            (Platform::Other, _, Some("h"), false) => Some(ShowHistory),
            (Platform::MacOs, Key::Named(NamedKey::Backspace), _, true) => Some(ClearBrowsingData),
            (Platform::Other, Key::Named(NamedKey::Delete), _, true) => Some(ClearBrowsingData),
            _ => None,
        };
    }

    // find: F3 / Shift+F3 outside macOS
    if platform == Platform::Other && !primary && !alt && matches!(key, Key::Named(NamedKey::F3)) {
        return Some(if shift { FindPrevious } else { FindNext });
    }

    if platform != Platform::Other || primary || shift {
        return None;
    }
    match (key, character, alt) {
        (Key::Named(NamedKey::ArrowLeft), _, true) => Some(Back),
        // tabs-plus: the browser menu, as other browsers open theirs
        (_, Some("f" | "e"), true) => Some(ShowBrowserMenu),
        (Key::Named(NamedKey::F10), _, false) => Some(ShowBrowserMenu),
        (Key::Named(NamedKey::ArrowRight), _, true) => Some(Forward),
        (_, Some("d"), true) => Some(FocusAddressBar),
        (Key::Named(NamedKey::F5), _, false) => Some(Reload),
        _ => None,
    }
}

/// layout-switch: Ctrl+Cmd+B on macOS, Ctrl+Alt+B elsewhere, with no other modifier. It is the
/// one chord of the table that holds both Ctrl and Cmd on macOS, and it holds Ctrl and Alt
/// elsewhere (which some layouts use as AltGr: see the user guide), so it is matched on the
/// whole set of modifiers rather than through "primary".
fn dock_shortcut(
    character: Option<&str>,
    modifiers: ModifiersState,
    platform: Platform,
) -> Option<ShortcutAction> {
    if character != Some("b") || modifiers.shift_key() {
        return None;
    }
    let chord = match platform {
        Platform::MacOs => {
            modifiers.super_key() && modifiers.control_key() && !modifiers.alt_key()
        }
        Platform::Other => {
            modifiers.control_key() && modifiers.alt_key() && !modifiers.super_key()
        }
    };
    chord.then_some(ShortcutAction::ToggleDock)
}

/// The bookmarks lane's shortcuts.
fn bookmark_shortcut(
    character: Option<&str>,
    primary: bool,
    shift: bool,
    alt: bool,
    platform: Platform,
) -> Option<ShortcutAction> {
    if !primary {
        return None;
    }
    match (platform, character, shift, alt) {
        (_, Some("d"), false, false) => Some(ShortcutAction::BookmarkPage),
        (_, Some("b"), true, false) => Some(ShortcutAction::ToggleBookmarksBar),
        (Platform::MacOs, Some("b"), false, true) => Some(ShortcutAction::ShowBookmarksManager),
        (Platform::Other, Some("o"), true, false) => Some(ShortcutAction::ShowBookmarksManager),
        _ => None,
    }
}

/// Cmd+Left/Right moves to the start or end of a line in a text field (the address bar, or a
/// field in the page, which Servo handles). Focus inside the page cannot be known, so on macOS
/// these keys are only browser shortcuts while neither the browser UI nor the page has focus.
pub(crate) fn yields_to_focused_text(key: &Key, platform: Platform) -> bool {
    platform == Platform::MacOs
        && matches!(key, Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight))
}

/// Escape leaves page-requested fullscreen and is then not given to the page.
pub(crate) fn exits_fullscreen(key: &Key, page_fullscreen: bool) -> bool {
    page_fullscreen && matches!(key, Key::Named(NamedKey::Escape))
}

/// The browser action for a key press, including Escape while a page is fullscreen.
pub(crate) fn browser_action(
    key: &Key,
    modifiers: ModifiersState,
    platform: Platform,
    page_fullscreen: bool,
) -> Option<ShortcutAction> {
    if exits_fullscreen(key, page_fullscreen) {
        return Some(ShortcutAction::ExitFullscreen);
    }
    shortcut_action(key, modifiers, platform)
}

/// Returns true if the event belongs to a browser shortcut and must not reach the page.
pub(crate) fn handle_browser_shortcut(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
    web_view_focused: &Cell<bool>,
    shortcut_keys: &mut ShortcutKeys,
    event: &winit::event::KeyEvent,
    modifiers: ModifiersState,
) -> bool {
    let disposition = match event.state {
        ElementState::Released => shortcut_keys.release(event.physical_key),
        ElementState::Pressed => {
            let platform = Platform::current();
            let page_fullscreen = controller.borrow().active_state().is_fullscreen();
            let action = browser_action(&event.logical_key, modifiers, platform, page_fullscreen)
                .filter(|_| {
                    !(yields_to_focused_text(&event.logical_key, platform)
                        && (web_view_focused.get() || window.get_text_input_focused()))
                });
            shortcut_keys.press(event.physical_key, event.repeat, action)
        }
    };

    match disposition {
        KeyDisposition::Forward => false,
        KeyDisposition::Consume => true,
        KeyDisposition::Run(action) => {
            run_shortcut(window, controller, web_view_focused, action);
            window.window().request_redraw();
            true
        }
    }
}

pub(crate) fn run_shortcut(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
    web_view_focused: &Cell<bool>,
    action: ShortcutAction,
) {
    match action {
        ShortcutAction::NewTab => {
            unfocus_web_view(controller, web_view_focused);
            controller.borrow_mut().open_tab();
            sync_ui(window, &controller.borrow());
            window.invoke_focus_address_bar();
        }
        ShortcutAction::CloseTab => {
            let active_tab_id = controller.borrow().active_tab_id();
            // tabs-plus: Cmd+W never closes a pinned tab
            if crate::tab_strip::step_off_pinned_tab(
                window,
                controller,
                web_view_focused,
                active_tab_id,
            ) {
                return;
            }
            let outcome = controller.borrow_mut().close_tab(active_tab_id);
            // tabs: closing the last tab quits
            crate::tab_close::finish_closing_tab(window, &controller.borrow(), outcome);
        }
        ShortcutAction::NextTab | ShortcutAction::PreviousTab => {
            controller
                .borrow_mut()
                .select_next_tab(action == ShortcutAction::PreviousTab);
            sync_ui(window, &controller.borrow());
        }
        ShortcutAction::SelectTab(index) => {
            if controller.borrow_mut().select_tab_at(index) {
                sync_ui(window, &controller.borrow());
            }
        }
        ShortcutAction::SelectLastTab => {
            controller.borrow_mut().select_last_tab();
            sync_ui(window, &controller.borrow());
        }
        // tabs-plus: these navigate, so the address draft ends
        ShortcutAction::Back | ShortcutAction::Forward | ShortcutAction::Reload => {
            window.invoke_accept_address_draft();
            let command = match action {
                ShortcutAction::Back => BrowserCommand::GoBack,
                ShortcutAction::Forward => BrowserCommand::GoForward,
                _ => BrowserCommand::Reload,
            };
            controller.borrow_mut().queue_command(command);
        }
        ShortcutAction::FocusAddressBar => {
            unfocus_web_view(controller, web_view_focused);
            window.invoke_focus_address_bar();
        }
        ShortcutAction::ZoomIn | ShortcutAction::ZoomOut | ShortcutAction::ZoomReset => {
            {
                let mut controller = controller.borrow_mut();
                match action {
                    ShortcutAction::ZoomIn => controller.zoom_in(),
                    ShortcutAction::ZoomOut => controller.zoom_out(),
                    _ => controller.reset_zoom(),
                }
            }
            sync_ui(window, &controller.borrow());
        }
        ShortcutAction::Quit => {
            // downloads-ui lane: asks first while downloads are running.
            crate::quit::request_quit_asking();
        }
        ShortcutAction::ExitFullscreen => controller
            .borrow_mut()
            .queue_command(BrowserCommand::ExitFullscreen),
        // downloads-ui lane
        ShortcutAction::SavePage => crate::downloads::save_page(controller),
        ShortcutAction::ShowDownloads => crate::downloads::show_list(),
        // settings-ui
        ShortcutAction::ShowSettings => window.invoke_show_settings(),
        // tabs-plus
        ShortcutAction::SearchTabs => crate::tab_strip::show_tab_search(window),
        ShortcutAction::ShowBrowserMenu => window.invoke_show_browser_menu(),
        ShortcutAction::MoveTabLeft | ShortcutAction::MoveTabRight => {
            let delta = if action == ShortcutAction::MoveTabLeft {
                -1
            } else {
                1
            };
            let active_tab_id = controller.borrow().active_tab_id();
            if controller.borrow_mut().move_tab_by(active_tab_id, delta) {
                sync_ui(window, &controller.borrow());
            }
        }
        // history lane
        ShortcutAction::ShowHistory => crate::history_ui::show_history(),
        ShortcutAction::ClearBrowsingData => crate::history_ui::show_clear_dialog(),
        // bookmarks lane
        ShortcutAction::BookmarkPage => crate::bookmarks::bookmark_this_page(),
        ShortcutAction::ToggleBookmarksBar => crate::bookmarks::toggle_bookmarks_bar(),
        ShortcutAction::ShowBookmarksManager => crate::bookmarks::show_bookmarks_manager(),
        // session
        ShortcutAction::ReopenClosedTab => crate::session::reopen_closed_tab(window, controller),
        ShortcutAction::DuplicateTab => crate::session::duplicate_tab(window, controller),
        ShortcutAction::RestorePreviousSession => {
            crate::session::restore_previous_session(window, controller)
        }
        // media-session lane
        ShortcutAction::ToggleMute => {
            let active_tab_id = controller.borrow().active_tab_id();
            crate::media_session::toggle_mute(window, controller, active_tab_id);
        }
        // find
        ShortcutAction::Find => crate::find::open_find_bar(window, controller, web_view_focused),
        ShortcutAction::FindNext | ShortcutAction::FindPrevious => crate::find::step_find(
            window,
            controller,
            web_view_focused,
            action == ShortcutAction::FindNext,
        ),
        // sh-pages
        ShortcutAction::ViewSource => {
            crate::view_source::open_source_of_active_tab(window, controller)
        }
        // layout-switch
        ShortcutAction::ShowTabRing => crate::layout_switch::show_tab_ring(window, controller),
        ShortcutAction::ToggleDock => crate::layout_switch::toggle_dock(window, controller),
        ShortcutAction::ToggleDial => crate::layout_switch::toggle_dial(window, controller),
    }
}

pub(crate) fn unfocus_web_view(
    controller: &RefCell<BrowserController>,
    web_view_focused: &Cell<bool>,
) {
    if web_view_focused.replace(false) {
        controller
            .borrow_mut()
            .queue_command(BrowserCommand::FocusChanged { focused: false });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap};

    use super::*;
    use ShortcutAction::*;
    use winit::keyboard::KeyCode;

    const NONE: ModifiersState = ModifiersState::empty();
    const CMD: ModifiersState = ModifiersState::SUPER;
    const CTRL: ModifiersState = ModifiersState::CONTROL;
    const ALT: ModifiersState = ModifiersState::ALT;
    const SHIFT: ModifiersState = ModifiersState::SHIFT;

    fn character(text: &str) -> Key {
        Key::Character(text.into())
    }

    fn named(key: NamedKey) -> Key {
        Key::Named(key)
    }

    fn mac(key: Key, modifiers: ModifiersState) -> Option<ShortcutAction> {
        shortcut_action(&key, modifiers, Platform::MacOs)
    }

    fn other(key: Key, modifiers: ModifiersState) -> Option<ShortcutAction> {
        shortcut_action(&key, modifiers, Platform::Other)
    }

    #[test]
    fn new_and_close_tab_on_macos() {
        assert_eq!(mac(character("t"), CMD), Some(NewTab));
        assert_eq!(mac(character("w"), CMD), Some(CloseTab));
    }

    #[test]
    fn new_and_close_tab_elsewhere() {
        assert_eq!(other(character("t"), CTRL), Some(NewTab));
        assert_eq!(other(character("w"), CTRL), Some(CloseTab));
    }

    #[test]
    fn next_and_previous_tab_on_macos() {
        assert_eq!(mac(named(NamedKey::Tab), CTRL), Some(NextTab));
        assert_eq!(mac(named(NamedKey::Tab), CTRL | SHIFT), Some(PreviousTab));
        assert_eq!(mac(character("]"), CMD | SHIFT), Some(NextTab));
        assert_eq!(mac(character("["), CMD | SHIFT), Some(PreviousTab));
        // macOS reports the shifted character while Cmd is held.
        assert_eq!(mac(character("}"), CMD | SHIFT), Some(NextTab));
        assert_eq!(mac(character("{"), CMD | SHIFT), Some(PreviousTab));
    }

    #[test]
    fn next_and_previous_tab_elsewhere() {
        assert_eq!(other(named(NamedKey::Tab), CTRL), Some(NextTab));
        assert_eq!(other(named(NamedKey::Tab), CTRL | SHIFT), Some(PreviousTab));
        assert_eq!(other(named(NamedKey::PageDown), CTRL), Some(NextTab));
        assert_eq!(other(named(NamedKey::PageUp), CTRL), Some(PreviousTab));
    }

    #[test]
    fn numbered_tabs_on_macos() {
        assert_eq!(mac(character("1"), CMD), Some(SelectTab(0)));
        assert_eq!(mac(character("8"), CMD), Some(SelectTab(7)));
    }

    #[test]
    fn numbered_tabs_elsewhere() {
        assert_eq!(other(character("1"), CTRL), Some(SelectTab(0)));
        assert_eq!(other(character("8"), CTRL), Some(SelectTab(7)));
        assert_eq!(other(character("9"), CTRL), Some(SelectLastTab));
    }

    #[test]
    fn command_9_selects_the_last_tab() {
        assert_eq!(mac(character("9"), CMD), Some(SelectLastTab));
    }

    #[test]
    fn back_and_forward_on_macos() {
        assert_eq!(mac(character("["), CMD), Some(Back));
        assert_eq!(mac(character("]"), CMD), Some(Forward));
        assert_eq!(mac(named(NamedKey::ArrowLeft), CMD), Some(Back));
        assert_eq!(mac(named(NamedKey::ArrowRight), CMD), Some(Forward));
    }

    #[test]
    fn back_and_forward_elsewhere() {
        assert_eq!(other(named(NamedKey::ArrowLeft), ALT), Some(Back));
        assert_eq!(other(named(NamedKey::ArrowRight), ALT), Some(Forward));
    }

    #[test]
    fn option_arrows_are_left_to_the_page_on_macos() {
        assert_eq!(mac(named(NamedKey::ArrowLeft), ALT), None);
        assert_eq!(mac(named(NamedKey::ArrowRight), ALT), None);
    }

    #[test]
    fn command_arrows_yield_to_focused_text_only_on_macos() {
        assert!(yields_to_focused_text(
            &named(NamedKey::ArrowLeft),
            Platform::MacOs
        ));
        assert!(yields_to_focused_text(
            &named(NamedKey::ArrowRight),
            Platform::MacOs
        ));
        assert!(!yields_to_focused_text(&character("["), Platform::MacOs));
        assert!(!yields_to_focused_text(
            &named(NamedKey::ArrowLeft),
            Platform::Other
        ));
    }

    #[test]
    fn reload_on_macos() {
        assert_eq!(mac(character("r"), CMD), Some(Reload));
    }

    #[test]
    fn reload_elsewhere() {
        assert_eq!(other(character("r"), CTRL), Some(Reload));
        assert_eq!(other(named(NamedKey::F5), NONE), Some(Reload));
    }

    #[test]
    fn focus_address_bar_on_macos() {
        assert_eq!(mac(character("l"), CMD), Some(FocusAddressBar));
    }

    #[test]
    fn focus_address_bar_elsewhere() {
        assert_eq!(other(character("l"), CTRL), Some(FocusAddressBar));
        assert_eq!(other(character("d"), ALT), Some(FocusAddressBar));
    }

    #[test]
    fn zoom_on_macos() {
        assert_eq!(mac(character("="), CMD), Some(ZoomIn));
        assert_eq!(mac(character("+"), CMD | SHIFT), Some(ZoomIn));
        assert_eq!(mac(character("-"), CMD), Some(ZoomOut));
        assert_eq!(mac(character("0"), CMD), Some(ZoomReset));
    }

    #[test]
    fn zoom_elsewhere() {
        assert_eq!(other(character("="), CTRL), Some(ZoomIn));
        assert_eq!(other(character("+"), CTRL | SHIFT), Some(ZoomIn));
        assert_eq!(other(character("-"), CTRL), Some(ZoomOut));
        assert_eq!(other(character("0"), CTRL), Some(ZoomReset));
    }

    #[test]
    fn quit_on_macos() {
        assert_eq!(mac(character("q"), CMD), Some(Quit));
    }

    #[test]
    fn quit_elsewhere() {
        assert_eq!(other(character("q"), CTRL), Some(Quit));
    }

    #[test]
    fn settings_open_with_primary_comma() {
        assert_eq!(mac(character(","), CMD), Some(ShowSettings));
        assert_eq!(other(character(","), CTRL), Some(ShowSettings));
        assert_eq!(mac(character(","), CTRL), None);
        assert_eq!(mac(character(","), NONE), None);
    }

    #[test]
    fn control_w_is_not_a_shortcut_on_macos() {
        assert_eq!(mac(character("w"), CTRL), None);
        assert_eq!(mac(character("r"), CTRL), None);
        assert_eq!(mac(character("1"), CTRL), None);
    }

    #[test]
    fn command_letters_are_not_shortcuts_elsewhere() {
        assert_eq!(other(character("w"), CMD), None);
        assert_eq!(other(character("["), CTRL), None);
        assert_eq!(other(named(NamedKey::ArrowLeft), CTRL), None);
    }

    #[test]
    fn macos_does_not_use_the_other_platforms_keys() {
        assert_eq!(mac(named(NamedKey::F5), NONE), None);
        assert_eq!(mac(character("d"), ALT), None);
        assert_eq!(mac(named(NamedKey::PageDown), CMD), None);
    }

    #[test]
    fn extra_modifiers_turn_a_shortcut_off() {
        assert_eq!(mac(character("r"), CMD | SHIFT), None);
        assert_eq!(mac(character("t"), CMD | ALT), None);
        assert_eq!(other(character("w"), CTRL | SHIFT), None);
    }

    #[test]
    fn reopen_closed_tab_on_macos() {
        assert_eq!(mac(character("t"), CMD | SHIFT), Some(ReopenClosedTab));
        // macOS may report the shifted letter.
        assert_eq!(mac(character("T"), CMD | SHIFT), Some(ReopenClosedTab));
        assert_eq!(mac(character("t"), CTRL | SHIFT), None);
    }

    #[test]
    fn reopen_closed_tab_elsewhere() {
        assert_eq!(other(character("t"), CTRL | SHIFT), Some(ReopenClosedTab));
        assert_eq!(other(character("T"), CTRL | SHIFT), Some(ReopenClosedTab));
        assert_eq!(other(character("t"), CTRL | SHIFT | ALT), None);
    }

    #[test]
    fn uppercase_letters_from_caps_lock_still_match() {
        assert_eq!(mac(character("T"), CMD), Some(NewTab));
    }

    #[test]
    fn escape_exits_fullscreen_only_while_a_page_is_fullscreen() {
        assert!(exits_fullscreen(&named(NamedKey::Escape), true));
        assert!(!exits_fullscreen(&named(NamedKey::Escape), false));
        assert!(!exits_fullscreen(&character("q"), true));
        assert_eq!(
            browser_action(&named(NamedKey::Escape), NONE, Platform::MacOs, true),
            Some(ExitFullscreen)
        );
        assert_eq!(
            browser_action(&named(NamedKey::Escape), NONE, Platform::MacOs, false),
            None
        );
        assert_eq!(
            browser_action(&character("t"), CMD, Platform::MacOs, true),
            Some(NewTab)
        );
    }

    #[test]
    fn only_zoom_and_tab_cycling_repeat() {
        let expectations = [
            (NewTab, false),
            (CloseTab, false),
            (NextTab, true),
            (PreviousTab, true),
            (SelectTab(0), false),
            (SelectTab(7), false),
            (SelectLastTab, false),
            (Back, false),
            (Forward, false),
            (Reload, false),
            (FocusAddressBar, false),
            (ZoomIn, true),
            (ZoomOut, true),
            (ZoomReset, false),
            (Quit, false),
            (ExitFullscreen, false),
            (SavePage, false),      // downloads-ui lane
            (ShowDownloads, false), // downloads-ui lane
            (ShowSettings, false),
            (SearchTabs, false),
            (MoveTabLeft, false),
            (MoveTabRight, false),
            (ShowBrowserMenu, false),
            (ShowHistory, false),
            (ClearBrowsingData, false),
            (BookmarkPage, false),
            (ToggleBookmarksBar, false),
            (ShowBookmarksManager, false),
            (ReopenClosedTab, false),
            (DuplicateTab, false),
            (RestorePreviousSession, false),
            (ToggleMute, false),
            (Find, false),
            (FindNext, true),
            (FindPrevious, true),
            (ViewSource, false), // sh-pages
            (ShowTabRing, false), // layout-switch
            (ToggleDock, false),  // layout-switch
            (ToggleDial, false),  // layout-switch
        ];
        for (action, repeats) in expectations {
            // Exhaustive, so a new variant cannot be added without deciding here.
            match action {
                NewTab
                | CloseTab
                | NextTab
                | PreviousTab
                | SelectTab(_)
                | SelectLastTab
                | Back
                | Forward
                | Reload
                | FocusAddressBar
                | ZoomIn
                | ZoomOut
                | ZoomReset
                | Quit
                | ExitFullscreen
                | ShowSettings
                | ShowHistory
                | ClearBrowsingData
                | BookmarkPage
                | ToggleBookmarksBar
                | ShowBookmarksManager
                | ReopenClosedTab
                | DuplicateTab
                | RestorePreviousSession
                | SearchTabs
                | MoveTabLeft
                | MoveTabRight
                | ShowBrowserMenu
                | ToggleMute
                | Find
                | FindNext
                | FindPrevious => {}
                SavePage | ShowDownloads => {} // downloads-ui lane
                ViewSource => {}               // sh-pages
                ShowTabRing | ToggleDock | ToggleDial => {} // layout-switch
            }
            assert_eq!(action.repeats(), repeats, "{action:?}");
        }
    }

    // history lane
    #[test]
    fn history_and_clearing_browsing_data() {
        assert_eq!(mac(character("y"), CMD), Some(ShowHistory));
        assert_eq!(other(character("h"), CTRL), Some(ShowHistory));
        assert_eq!(other(character("y"), CTRL), None);
        assert_eq!(mac(character("h"), CMD), None);
        // The Mac keyboard's Delete is Backspace.
        assert_eq!(
            mac(named(NamedKey::Backspace), CMD | SHIFT),
            Some(ClearBrowsingData)
        );
        assert_eq!(
            other(named(NamedKey::Delete), CTRL | SHIFT),
            Some(ClearBrowsingData)
        );
        assert_eq!(mac(named(NamedKey::Backspace), CMD), None);
        assert_eq!(other(named(NamedKey::Delete), CTRL), None);
    }

    // find
    #[test]
    fn find_on_macos() {
        assert_eq!(mac(character("f"), CMD), Some(Find));
        assert_eq!(mac(character("g"), CMD), Some(FindNext));
        assert_eq!(mac(character("g"), CMD | SHIFT), Some(FindPrevious));
        // macOS reports the shifted character while Cmd is held.
        assert_eq!(mac(character("G"), CMD | SHIFT), Some(FindPrevious));
        assert_eq!(mac(character("f"), CTRL), None);
        assert_eq!(mac(named(NamedKey::F3), NONE), None);
    }

    #[test]
    fn find_elsewhere() {
        assert_eq!(other(character("f"), CTRL), Some(Find));
        assert_eq!(other(character("g"), CTRL), Some(FindNext));
        assert_eq!(other(character("g"), CTRL | SHIFT), Some(FindPrevious));
        assert_eq!(other(named(NamedKey::F3), NONE), Some(FindNext));
        assert_eq!(other(named(NamedKey::F3), SHIFT), Some(FindPrevious));
        assert_eq!(other(character("f"), CMD), None);
    }

    const KEY_R: PhysicalKey = PhysicalKey::Code(KeyCode::KeyR);
    const KEY_T: PhysicalKey = PhysicalKey::Code(KeyCode::KeyT);
    const KEY_EQUAL: PhysicalKey = PhysicalKey::Code(KeyCode::Equal);
    const COMMAND: PhysicalKey = PhysicalKey::Code(KeyCode::SuperLeft);

    #[test]
    fn a_shortcut_key_released_after_its_modifier_is_still_swallowed() {
        let mut keys = ShortcutKeys::default();

        assert_eq!(keys.press(COMMAND, false, None), KeyDisposition::Forward);
        assert_eq!(
            keys.press(KEY_R, false, Some(Reload)),
            KeyDisposition::Run(Reload)
        );
        assert_eq!(keys.release(COMMAND), KeyDisposition::Forward);
        assert_eq!(keys.release(KEY_R), KeyDisposition::Consume);
    }

    #[test]
    fn a_key_pressed_before_its_modifier_gets_its_release() {
        let mut keys = ShortcutKeys::default();

        assert_eq!(keys.press(KEY_R, false, None), KeyDisposition::Forward);
        assert_eq!(keys.press(COMMAND, false, None), KeyDisposition::Forward);
        assert_eq!(keys.release(KEY_R), KeyDisposition::Forward);
        assert_eq!(keys.release(COMMAND), KeyDisposition::Forward);
    }

    #[test]
    fn a_release_is_swallowed_only_once() {
        let mut keys = ShortcutKeys::default();
        keys.press(KEY_R, false, Some(Reload));

        assert_eq!(keys.release(KEY_R), KeyDisposition::Consume);
        assert_eq!(keys.press(KEY_R, false, None), KeyDisposition::Forward);
        assert_eq!(keys.release(KEY_R), KeyDisposition::Forward);
    }

    #[test]
    fn repeats_run_only_repeating_actions() {
        let mut keys = ShortcutKeys::default();
        keys.press(KEY_T, false, Some(NewTab));
        keys.press(KEY_EQUAL, false, Some(ZoomIn));

        assert_eq!(
            keys.press(KEY_T, true, Some(NewTab)),
            KeyDisposition::Consume
        );
        assert_eq!(
            keys.press(KEY_EQUAL, true, Some(ZoomIn)),
            KeyDisposition::Run(ZoomIn)
        );
    }

    #[test]
    fn repeats_of_a_shortcut_stay_swallowed_after_the_modifier_is_released() {
        let mut keys = ShortcutKeys::default();
        keys.press(KEY_EQUAL, false, Some(ZoomIn));

        assert_eq!(keys.press(KEY_EQUAL, true, None), KeyDisposition::Consume);
        assert_eq!(keys.release(KEY_EQUAL), KeyDisposition::Consume);
    }

    #[test]
    fn a_repeat_never_starts_a_shortcut() {
        let mut keys = ShortcutKeys::default();
        keys.press(KEY_R, false, None);

        assert_eq!(
            keys.press(KEY_R, true, Some(Reload)),
            KeyDisposition::Forward
        );
        assert_eq!(keys.release(KEY_R), KeyDisposition::Forward);
    }

    // menus
    #[test]
    fn a_key_the_native_menu_claimed_has_its_release_swallowed() {
        let mut keys = ShortcutKeys::default();

        keys.claim(KEY_T);

        assert_eq!(keys.release(KEY_T), KeyDisposition::Consume);
        assert_eq!(keys.release(KEY_T), KeyDisposition::Forward);
    }

    #[test]
    fn a_later_press_of_a_claimed_key_is_handled_normally() {
        let mut keys = ShortcutKeys::default();
        keys.claim(KEY_T);

        assert_eq!(keys.press(KEY_T, false, None), KeyDisposition::Forward);
        assert_eq!(keys.release(KEY_T), KeyDisposition::Forward);
    }

    #[test]
    fn clearing_forwards_later_releases() {
        let mut keys = ShortcutKeys::default();
        keys.press(KEY_R, false, Some(Reload));

        keys.clear();

        assert_eq!(keys.release(KEY_R), KeyDisposition::Forward);
    }

    // tabs-plus
    #[test]
    fn shift_primary_a_searches_tabs() {
        assert_eq!(mac(character("a"), CMD | SHIFT), Some(SearchTabs));
        assert_eq!(mac(character("A"), CMD | SHIFT), Some(SearchTabs));
        assert_eq!(other(character("A"), CTRL | SHIFT), Some(SearchTabs));
        // Cmd+A stays Select All.
        assert_eq!(mac(character("a"), CMD), None);
    }

    #[test]
    fn the_browser_menu_opens_from_the_keyboard_off_macos() {
        assert_eq!(other(character("f"), ALT), Some(ShowBrowserMenu));
        assert_eq!(other(character("e"), ALT), Some(ShowBrowserMenu));
        assert_eq!(other(named(NamedKey::F10), NONE), Some(ShowBrowserMenu));
        // macOS has the menu bar; Option+F types a character there.
        assert_eq!(mac(character("f"), ALT), None);
        assert_eq!(mac(named(NamedKey::F10), NONE), None);
    }

    #[test]
    fn control_shift_page_keys_move_the_tab_elsewhere() {
        assert_eq!(
            other(named(NamedKey::PageUp), CTRL | SHIFT),
            Some(MoveTabLeft)
        );
        assert_eq!(
            other(named(NamedKey::PageDown), CTRL | SHIFT),
            Some(MoveTabRight)
        );
        assert_eq!(mac(named(NamedKey::PageUp), CMD | SHIFT), None);
    }

    #[test]
    fn bookmark_shortcuts_on_macos() {
        assert_eq!(mac(character("d"), CMD), Some(BookmarkPage));
        assert_eq!(mac(character("B"), CMD | SHIFT), Some(ToggleBookmarksBar));
        // winit reports Cmd+Option+B as "b" (Option is ignored while Cmd is held).
        assert_eq!(mac(character("b"), CMD | ALT), Some(ShowBookmarksManager));
        assert_eq!(mac(character("O"), CMD | SHIFT), None);
        assert_eq!(mac(character("d"), CTRL), None);
        assert_eq!(mac(character("b"), CMD), None);
    }

    #[test]
    fn bookmark_shortcuts_elsewhere() {
        assert_eq!(other(character("d"), CTRL), Some(BookmarkPage));
        assert_eq!(
            other(character("B"), CTRL | SHIFT),
            Some(ToggleBookmarksBar)
        );
        assert_eq!(
            other(character("O"), CTRL | SHIFT),
            Some(ShowBookmarksManager)
        );
        // Ctrl+Alt+B is the dock's (layout-switch), not the bookmarks manager's.
        assert_eq!(other(character("b"), CTRL | ALT), Some(ToggleDock));
        // Alt+D still focuses the address bar.
        assert_eq!(other(character("d"), ALT), Some(FocusAddressBar));
    }

    // layout-switch
    #[test]
    fn new_chords_on_both_platforms() {
        // macOS: Ctrl+Cmd+B shrinks or opens the dock (winit reports the letter as "b"; the
        // shifted letter is accepted for Caps Lock), Cmd+Shift+E toggles the dial.
        assert_eq!(mac(character("b"), CTRL | CMD), Some(ToggleDock));
        assert_eq!(mac(character("B"), CTRL | CMD), Some(ToggleDock));
        assert_eq!(mac(character("e"), CMD | SHIFT), Some(ToggleDial));
        assert_eq!(mac(character("E"), CMD | SHIFT), Some(ToggleDial));
        // Elsewhere: Ctrl+Alt+B and Ctrl+Shift+E.
        assert_eq!(other(character("b"), CTRL | ALT), Some(ToggleDock));
        assert_eq!(other(character("B"), CTRL | ALT), Some(ToggleDock));
        assert_eq!(other(character("e"), CTRL | SHIFT), Some(ToggleDial));
        assert_eq!(other(character("E"), CTRL | SHIFT), Some(ToggleDial));

        // Another platform's chord, or an extra or missing modifier, is not one of these.
        assert_eq!(mac(character("b"), CTRL | ALT), None);
        assert_eq!(mac(character("b"), CTRL), None);
        assert_eq!(mac(character("b"), CTRL | CMD | SHIFT), None);
        assert_eq!(mac(character("b"), CTRL | CMD | ALT), None);
        assert_eq!(mac(character("e"), CTRL | SHIFT), None);
        assert_eq!(mac(character("e"), CMD), None);
        assert_eq!(mac(character("e"), CMD | SHIFT | ALT), None);
        assert_eq!(other(character("b"), CTRL | CMD), None);
        assert_eq!(other(character("b"), CTRL | ALT | SHIFT), None);
        assert_eq!(other(character("b"), CTRL | ALT | CMD), None);
        assert_eq!(other(character("b"), ALT), None);
        assert_eq!(other(character("e"), CMD | SHIFT), None);
        assert_eq!(other(character("e"), CTRL), None);
        assert_eq!(other(character("e"), CTRL | SHIFT | ALT), None);

        // Neither key runs more than once per press, and the ring has no key at all.
        assert!(!ToggleDock.repeats() && !ToggleDial.repeats() && !ShowTabRing.repeats());
        for platform in [Platform::MacOs, Platform::Other] {
            for (token, modifiers) in
                (0..16_u32).flat_map(|bits| ["b", "e"].map(|token| (token, bits)))
            {
                let flags = [CTRL, ALT, SHIFT, CMD]
                    .into_iter()
                    .enumerate()
                    .filter(|(bit, _)| (modifiers >> *bit) & 1 == 1)
                    .fold(NONE, |all, (_, flag)| all | flag);
                assert_ne!(
                    shortcut_action(&character(token), flags, platform),
                    Some(ShowTabRing),
                    "{platform:?} {token} {flags:?}"
                );
            }
        }
    }

    // layout-switch: the keys other lanes chose next to the new ones
    #[test]
    fn no_clash_with_the_existing_table() {
        assert_eq!(other(character("e"), ALT), Some(ShowBrowserMenu));
        assert_eq!(other(character("f"), ALT), Some(ShowBrowserMenu));
        assert_eq!(mac(character("b"), CMD | SHIFT), Some(ToggleBookmarksBar));
        assert_eq!(mac(character("b"), CMD | ALT), Some(ShowBookmarksManager));
        assert_eq!(mac(character("a"), CMD | SHIFT), Some(SearchTabs));
        assert_eq!(mac(character("m"), CMD | SHIFT), Some(ToggleMute));
        assert_eq!(other(character("b"), CTRL | SHIFT), Some(ToggleBookmarksBar));
        assert_eq!(other(character("o"), CTRL | SHIFT), Some(ShowBookmarksManager));
        assert_eq!(other(character("a"), CTRL | SHIFT), Some(SearchTabs));
        assert_eq!(other(character("m"), CTRL), Some(ToggleMute));
        // Cmd+E and Ctrl+E (Use Selection for Find, in a text field) stay with the page.
        assert_eq!(mac(character("e"), CMD), None);
        assert_eq!(mac(character("e"), CTRL), None);
        assert_eq!(other(character("e"), CTRL), None);
    }

    #[test]
    fn plain_letters_are_not_shortcuts() {
        for platform in [Platform::MacOs, Platform::Other] {
            for letter in ["t", "w", "r", "l", "q", "d", "1", "9", "=", "-", "0", "["] {
                assert_eq!(
                    shortcut_action(&character(letter), NONE, platform),
                    None,
                    "{letter}"
                );
                assert_eq!(
                    shortcut_action(&character(letter), SHIFT, platform),
                    None,
                    "Shift+{letter}"
                );
            }
            assert_eq!(shortcut_action(&named(NamedKey::Tab), NONE, platform), None);
            assert_eq!(
                shortcut_action(&named(NamedKey::ArrowLeft), NONE, platform),
                None
            );
        }
    }

    // media-session lane
    #[test]
    fn toggle_mute_shortcuts() {
        assert_eq!(mac(character("m"), CMD | SHIFT), Some(ToggleMute));
        assert_eq!(mac(character("M"), CMD | SHIFT), Some(ToggleMute));
        // Cmd+M is the system's Minimize; Ctrl+M belongs to the page on macOS.
        assert_eq!(mac(character("m"), CMD), None);
        assert_eq!(mac(character("m"), CTRL), None);
        assert_eq!(other(character("m"), CTRL), Some(ToggleMute));
        assert_eq!(other(character("m"), CTRL | SHIFT), None);
    }

    // downloads-ui lane
    #[test]
    fn save_page_and_the_downloads_list() {
        assert_eq!(mac(character("s"), CMD), Some(SavePage));
        assert_eq!(other(character("s"), CTRL), Some(SavePage));
        assert_eq!(mac(character("J"), CMD | SHIFT), Some(ShowDownloads));
        assert_eq!(mac(character("j"), CMD), None);
        assert_eq!(other(character("j"), CTRL), Some(ShowDownloads));
        assert!(!SavePage.repeats() && !ShowDownloads.repeats());
    }

    // sh-pages
    #[test]
    fn view_source_is_option_command_u_on_macos_and_control_u_elsewhere() {
        // winit reports Cmd+Option+B as "b" (Option is ignored while Cmd is held); Option+U is a
        // dead key on a US layout, so Cmd+Option+U is expected to arrive as "u" the same way. Not
        // yet seen with a real key event (hands-on list); if it is not "u", match the physical
        // U key as the KEY_R / KEY_T tests do.
        assert_eq!(mac(character("u"), CMD | ALT), Some(ViewSource));
        assert_eq!(mac(character("U"), CMD | ALT), Some(ViewSource));
        assert_eq!(other(character("u"), CTRL), Some(ViewSource));
        assert_eq!(other(character("U"), CTRL), Some(ViewSource));
        // Cmd+U stays the page's (underline in an editor); Option+U and Ctrl+U are the page's
        // on macOS; Ctrl+Alt and Ctrl+Shift belong to the layout and to Linux's Unicode input.
        assert_eq!(mac(character("u"), CMD), None);
        assert_eq!(mac(character("u"), ALT), None);
        assert_eq!(mac(character("u"), CTRL), None);
        assert_eq!(mac(character("u"), CMD | ALT | SHIFT), None);
        assert_eq!(other(character("u"), CTRL | ALT), None);
        assert_eq!(other(character("u"), CTRL | SHIFT), None);
        assert_eq!(other(character("u"), CMD), None);
        assert_eq!(other(character("u"), NONE), None);
        assert!(!ViewSource.repeats());
    }

    // --- the merged table: no two actions share a key ---
    //
    // Several lanes added keys to the one table (tab search, moving tabs and the browser menu,
    // mute, find, downloads, bookmarks, history). `shortcut_action` answers with one action per
    // key, so a clash shows as a key that no longer reaches the action it was written for (an
    // arm above it takes the key) or as an action that gained a key. The test below tries every
    // key with every modifier combination, on both platforms, and compares what each action
    // answers to with the table written out in `bindings`.

    /// The keys tried: letters, digits, punctuation, the named keys that have shortcuts and a
    /// few that never do. `plus` is the key `+` (it stands apart in `chord` strings).
    fn scanned_keys() -> Vec<String> {
        let mut keys = ('a'..='z')
            .chain('0'..='9')
            .map(String::from)
            .collect::<Vec<_>>();
        keys.extend(
            [
                "=",
                "plus",
                "-",
                ",",
                ".",
                "/",
                ";",
                "'",
                "[",
                "]",
                "{",
                "}",
                "\\",
                "`",
                "tab",
                "pageup",
                "pagedown",
                "left",
                "right",
                "up",
                "down",
                "home",
                "end",
                "backspace",
                "delete",
                "escape",
                "enter",
                "space",
            ]
            .map(String::from),
        );
        keys.extend((1..=12).map(|number| format!("f{number}")));
        keys
    }

    fn key_for(token: &str) -> Key {
        let key = match token {
            "tab" => NamedKey::Tab,
            "pageup" => NamedKey::PageUp,
            "pagedown" => NamedKey::PageDown,
            "left" => NamedKey::ArrowLeft,
            "right" => NamedKey::ArrowRight,
            "up" => NamedKey::ArrowUp,
            "down" => NamedKey::ArrowDown,
            "home" => NamedKey::Home,
            "end" => NamedKey::End,
            "backspace" => NamedKey::Backspace,
            "delete" => NamedKey::Delete,
            "escape" => NamedKey::Escape,
            "enter" => NamedKey::Enter,
            "space" => NamedKey::Space,
            "f1" => NamedKey::F1,
            "f2" => NamedKey::F2,
            "f3" => NamedKey::F3,
            "f4" => NamedKey::F4,
            "f5" => NamedKey::F5,
            "f6" => NamedKey::F6,
            "f7" => NamedKey::F7,
            "f8" => NamedKey::F8,
            "f9" => NamedKey::F9,
            "f10" => NamedKey::F10,
            "f11" => NamedKey::F11,
            "f12" => NamedKey::F12,
            "plus" => return character("+"),
            text => return character(text),
        };
        named(key)
    }

    /// A key with its modifiers, always written ctrl, alt, shift, cmd, then the key.
    fn show(token: &str, modifiers: ModifiersState) -> String {
        let mut text = String::new();
        for (flag, name) in [(CTRL, "ctrl"), (ALT, "alt"), (SHIFT, "shift"), (CMD, "cmd")] {
            if modifiers.contains(flag) {
                text.push_str(name);
                text.push('+');
            }
        }
        text + token
    }

    /// `spec` ("cmd+shift+m", "ctrl+plus", "f10": modifiers in any order, then the key) as
    /// `show` writes it.
    fn chord(spec: &str) -> String {
        let mut modifiers = NONE;
        let mut token = None;
        for part in spec.split('+') {
            match part {
                "ctrl" => modifiers |= CTRL,
                "alt" => modifiers |= ALT,
                "shift" => modifiers |= SHIFT,
                "cmd" => modifiers |= CMD,
                key => token = Some(key),
            }
        }
        show(token.expect("a chord names a key"), modifiers)
    }

    /// Every key the table binds on `platform`, by action (its `Debug` name).
    fn bound_keys(platform: Platform) -> BTreeMap<String, BTreeSet<String>> {
        let mut bound = BTreeMap::<String, BTreeSet<String>>::new();
        for token in scanned_keys() {
            let key = key_for(&token);
            for combination in 0..16_u32 {
                let modifiers = [CTRL, ALT, SHIFT, CMD]
                    .into_iter()
                    .enumerate()
                    .filter(|(bit, _)| (combination >> *bit) & 1 == 1)
                    .fold(NONE, |all, (_, flag)| all | flag);
                if let Some(action) = shortcut_action(&key, modifiers, platform) {
                    bound
                        .entry(format!("{action:?}"))
                        .or_default()
                        .insert(show(&token, modifiers));
                }
            }
        }
        bound
    }

    /// What each action answers to: (action, on macOS, elsewhere). Written out in full on
    /// purpose, so that a key taken from one lane's action by another fails the test below.
    /// Actions with no default key are left out (`DuplicateTab`, `RestorePreviousSession`), and
    /// Escape for `ExitFullscreen` is not a `shortcut_action` key.
    fn bindings() -> Vec<(String, Vec<String>, Vec<String>)> {
        let row =
            |action: &str, mac: &[&str], other: &[&str]| -> (String, Vec<String>, Vec<String>) {
                (
                    action.to_owned(),
                    mac.iter().map(|spec| (*spec).to_owned()).collect(),
                    other.iter().map(|spec| (*spec).to_owned()).collect(),
                )
            };
        let mut table = vec![
            row("NewTab", &["cmd+t"], &["ctrl+t"]),
            row("CloseTab", &["cmd+w"], &["ctrl+w"]),
            row(
                "NextTab",
                &["ctrl+tab", "cmd+shift+]", "cmd+shift+}"],
                &["ctrl+tab", "ctrl+pagedown"],
            ),
            row(
                "PreviousTab",
                &["ctrl+shift+tab", "cmd+shift+[", "cmd+shift+{"],
                &["ctrl+shift+tab", "ctrl+pageup"],
            ),
            row("SelectLastTab", &["cmd+9"], &["ctrl+9"]),
            row("Back", &["cmd+[", "cmd+left"], &["alt+left"]),
            row("Forward", &["cmd+]", "cmd+right"], &["alt+right"]),
            row("Reload", &["cmd+r"], &["ctrl+r", "f5"]),
            row("FocusAddressBar", &["cmd+l"], &["ctrl+l", "alt+d"]),
            row(
                "ZoomIn",
                &["cmd+=", "cmd+shift+=", "cmd+plus", "cmd+shift+plus"],
                &["ctrl+=", "ctrl+shift+=", "ctrl+plus", "ctrl+shift+plus"],
            ),
            row("ZoomOut", &["cmd+-"], &["ctrl+-"]),
            row("ZoomReset", &["cmd+0"], &["ctrl+0"]),
            row("Quit", &["cmd+q"], &["ctrl+q"]),
            // downloads-ui lane
            row("SavePage", &["cmd+s"], &["ctrl+s"]),
            row("ShowDownloads", &["cmd+shift+j"], &["ctrl+j"]),
            // settings-ui
            row("ShowSettings", &["cmd+,"], &["ctrl+,"]),
            // tabs-plus: Alt+F is Alt, not Ctrl, so it does not meet Find's Ctrl+F
            row("SearchTabs", &["cmd+shift+a"], &["ctrl+shift+a"]),
            row("MoveTabLeft", &[], &["ctrl+shift+pageup"]),
            row("MoveTabRight", &[], &["ctrl+shift+pagedown"]),
            row("ShowBrowserMenu", &[], &["alt+f", "alt+e", "f10"]),
            // history
            row("ShowHistory", &["cmd+y"], &["ctrl+h"]),
            row(
                "ClearBrowsingData",
                &["cmd+shift+backspace"],
                &["ctrl+shift+delete"],
            ),
            // bookmarks
            row("BookmarkPage", &["cmd+d"], &["ctrl+d"]),
            row("ToggleBookmarksBar", &["cmd+shift+b"], &["ctrl+shift+b"]),
            row("ShowBookmarksManager", &["cmd+alt+b"], &["ctrl+shift+o"]),
            // session
            row("ReopenClosedTab", &["cmd+shift+t"], &["ctrl+shift+t"]),
            // media-session lane: Cmd+M stays Minimize on macOS
            row("ToggleMute", &["cmd+shift+m"], &["ctrl+m"]),
            // find
            row("Find", &["cmd+f"], &["ctrl+f"]),
            row("FindNext", &["cmd+g"], &["ctrl+g", "f3"]),
            row(
                "FindPrevious",
                &["cmd+shift+g"],
                &["ctrl+shift+g", "shift+f3"],
            ),
            // sh-pages: Cmd+U alone stays the page's (underline); Ctrl+Alt is AltGr elsewhere
            row("ViewSource", &["cmd+alt+u"], &["ctrl+u"]),
            // layout-switch: the tab ring has no key; Ctrl+Cmd+B is a Control chord on macOS (the
            // table's one), Ctrl+Alt+B elsewhere; Cmd+Shift+E and Ctrl+Shift+E open the dial
            row("ToggleDock", &["ctrl+cmd+b"], &["ctrl+alt+b"]),
            row("ToggleDial", &["cmd+shift+e"], &["ctrl+shift+e"]),
        ];
        for number in 1..=8_usize {
            table.push(row(
                &format!("SelectTab({})", number - 1),
                &[format!("cmd+{number}").as_str()],
                &[format!("ctrl+{number}").as_str()],
            ));
        }
        table
    }

    #[test]
    fn no_two_actions_share_a_key_and_every_action_keeps_its_own() {
        let table = bindings();
        for platform in [Platform::MacOs, Platform::Other] {
            // The table itself gives no key to two actions...
            let mut owners = HashMap::<String, &String>::new();
            let mut expected = BTreeMap::<String, BTreeSet<String>>::new();
            for (action, mac, other) in &table {
                let specs = if platform == Platform::MacOs {
                    mac
                } else {
                    other
                };
                for spec in specs {
                    let key = chord(spec);
                    if let Some(first) = owners.insert(key.clone(), action) {
                        panic!("{platform:?}: {key} is written for both {first} and {action}");
                    }
                    expected.entry(action.clone()).or_default().insert(key);
                }
            }

            // ...and the code answers to exactly those keys: none is shadowed or taken by
            // another action, and no action has a key the table does not list.
            let bound = bound_keys(platform);
            let actions = expected.keys().chain(bound.keys()).collect::<BTreeSet<_>>();
            for action in actions {
                assert_eq!(
                    bound.get(action),
                    expected.get(action),
                    "{platform:?}: the keys of {action}"
                );
            }
        }
    }

    /// The keys that the lanes merged here chose side by side, spelled out.
    #[test]
    fn mute_tab_search_moving_tabs_the_browser_menu_and_find_keep_their_own_keys() {
        // macOS: Cmd+M is the system's Minimize, so mute is Cmd+Shift+M; Cmd+Shift+A is not
        // Select All (Cmd+A); there is no tab-moving or browser-menu key.
        assert_eq!(mac(character("m"), CMD | SHIFT), Some(ToggleMute));
        assert_eq!(mac(character("a"), CMD | SHIFT), Some(SearchTabs));
        assert_eq!(mac(character("f"), CMD), Some(Find));
        assert_eq!(mac(character("m"), CMD), None);
        assert_eq!(mac(character("f"), ALT), None);
        assert_eq!(mac(named(NamedKey::PageUp), CMD | SHIFT), None);

        // Elsewhere: Ctrl+M mutes, Ctrl+Shift+A searches tabs, Ctrl+Shift+PageUp/PageDown move
        // the tab (Ctrl+PageUp/PageDown still switch), Ctrl+F finds and Alt+F opens the browser
        // menu: Alt, not Ctrl, so they do not meet.
        assert_eq!(other(character("m"), CTRL), Some(ToggleMute));
        assert_eq!(other(character("a"), CTRL | SHIFT), Some(SearchTabs));
        assert_eq!(other(character("f"), CTRL), Some(Find));
        assert_eq!(other(character("f"), ALT), Some(ShowBrowserMenu));
        assert_eq!(other(character("f"), CTRL | ALT), None);
        assert_eq!(other(character("m"), CTRL | SHIFT), None);
        assert_eq!(other(character("m"), ALT), None);
        assert_eq!(
            other(named(NamedKey::PageUp), CTRL | SHIFT),
            Some(MoveTabLeft)
        );
        assert_eq!(other(named(NamedKey::PageUp), CTRL), Some(PreviousTab));
        assert_eq!(other(named(NamedKey::F10), NONE), Some(ShowBrowserMenu));
        assert_eq!(other(named(NamedKey::F3), NONE), Some(FindNext));
    }
}
