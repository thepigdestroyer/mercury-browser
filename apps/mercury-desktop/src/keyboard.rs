// SPDX-License-Identifier: MPL-2.0
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use mercury_ui::MainWindow;
use slint::ComponentHandle;
use slint::language::ColorScheme as SlintColorScheme;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use talaria_core::{
    BrowserCommand, BrowserController, ColorScheme, KeyLocation, KeyState, KeyboardInput,
    KeyboardModifiers,
};
use winit::event::{ElementState, Ime, WindowEvent};
use winit::keyboard::{Key, KeyLocation as WinitKeyLocation, ModifiersState, PhysicalKey};
use winit::window::Theme;

use crate::keyutils;
use crate::shortcuts::{ShortcutKeys, handle_browser_shortcut};

pub(crate) fn connect_native_keyboard_input(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    web_view_focused: Rc<Cell<bool>>,
    settings: &Rc<crate::settings::SettingsHandle>, // input-hooks
) {
    let weak_window = window.as_weak();
    let mut modifiers = ModifiersState::empty();
    let mut shortcut_keys = ShortcutKeys::default();
    // gestures: the two-finger swipe, and its settings switch
    let mut swipe_state = crate::swipe::SwipeState::new();
    let swipe_enabled = crate::swipe::follow_settings(settings);
    // txt-1: `ime.rs` holds the composition state and queues commands itself (text from the
    // system's emoji picker and dictation arrives outside this handler).
    attach_ime(window, &controller, &web_view_focused);

    window
        .window()
        .on_winit_window_event(move |_slint_window, event| {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            let Some(window) = weak_window.upgrade() else {
                return EventResult::Propagate;
            };
            // menus lane: focus tracking, and releases of keys the native menu performed.
            crate::menus::observe_window_event(&window, event, &mut shortcut_keys);

            match event {
                WindowEvent::ModifiersChanged(changed) => {
                    modifiers = changed.state();
                    // controls: Control-click opens the context menu on macOS.
                    crate::controls::set_control_key(modifiers.control_key());
                }
                WindowEvent::KeyboardInput {
                    event,
                    is_synthetic,
                    ..
                } => {
                    if crate::downloads::handle_key(event) {
                        return EventResult::PreventDefault; // downloads-ui lane
                    }
                    // controls: an open popup takes the keys; the page keeps its focus.
                    if crate::controls::handle_key(&window, event, modifiers) {
                        window.window().request_redraw();
                        return EventResult::PreventDefault;
                    }
                    // history lane: the suggestion list's keys come before everything else.
                    if crate::history_ui::handle_key(&window, event) {
                        return EventResult::PreventDefault;
                    }
                    let handled_as_shortcut = handle_browser_shortcut(
                        &window,
                        &controller,
                        &web_view_focused,
                        &mut shortcut_keys,
                        event,
                        modifiers,
                    );
                    if handled_as_shortcut {
                        return EventResult::PreventDefault;
                    }

                    // downloads-ui lane: the decision, as a pure function
                    if forwards_to_page(web_view_focused.get(), handled_as_shortcut) {
                        let active_tab_id = controller.borrow().active_tab_id();
                        // activation (tabs lane)
                        if crate::activation::is_activating_key(
                            &event.logical_key,
                            event.state,
                            event.repeat,
                            *is_synthetic,
                        ) {
                            crate::activation::note_user_activation(active_tab_id, Instant::now());
                        }
                        let keyboard_input = keyboard_input_from_winit(
                            event,
                            modifiers,
                            crate::ime::composing_in(active_tab_id),
                        );
                        controller
                            .borrow_mut()
                            .queue_command(BrowserCommand::KeyboardInput(keyboard_input));
                        window.window().request_redraw();
                    }
                }
                // dialogs: a page under a sheet gets no IME input; its composition ends.
                WindowEvent::Ime(_) if crate::page_requests::sheet_shown(&controller.borrow()) => {
                    dismiss_composition(&controller);
                }
                WindowEvent::Ime(ime)
                    if web_view_focused.get() || crate::ime::composition_active() =>
                {
                    queue_ime_input(&controller, ime);
                    window.window().request_redraw();
                }
                WindowEvent::PinchGesture { delta, .. }
                    if window.get_web_view_visible() && window.get_web_view_hovered() =>
                {
                    controller.borrow_mut().queue_command(pinch_zoom_command(
                        *delta,
                        window.get_web_view_pointer_x(),
                        window.get_web_view_pointer_y(),
                    ));
                    window.window().request_redraw();
                    return EventResult::PreventDefault;
                }
                // gestures: the wheel's phase, for the two-finger swipe. The event is never
                // consumed: the page and Slint still get it. Pinch (above) is input-hooks'.
                WindowEvent::MouseWheel { delta, phase, .. } => {
                    crate::swipe::on_wheel(
                        &window,
                        &controller,
                        &mut swipe_state,
                        &swipe_enabled,
                        delta,
                        *phase,
                        modifiers,
                    );
                }
                // The user left window fullscreen through the system (for example Ctrl+Cmd+F)
                // while a page is fullscreen: let the page leave too, so the chrome returns.
                // winit keeps its fullscreen state until the exit animation has finished, so
                // this fires on the first resize after that, which may be a later one; until
                // then Escape restores the chrome. macOS only: on Wayland winit's state follows
                // the compositor's last configure, so an unrelated resize before the compositor
                // answers Mercury's own request would end page fullscreen.
                WindowEvent::Resized(_)
                    if cfg!(target_os = "macos")
                        && window.get_chrome_hidden()
                        && controller.borrow().active_state().is_fullscreen()
                        && window
                            .window()
                            .with_winit_window(|winit_window| winit_window.fullscreen().is_none())
                            .unwrap_or(false) =>
                {
                    controller
                        .borrow_mut()
                        .queue_command(BrowserCommand::ExitFullscreen);
                    window.window().request_redraw();
                }
                WindowEvent::ThemeChanged(theme) => {
                    controller
                        .borrow_mut()
                        .set_color_scheme(color_scheme_from_winit(*theme));
                    // settings-ui: the settings window follows the system under "Follow the system".
                    crate::settings::system_theme_changed();
                    window.window().request_redraw();
                }
                // settings-ui: the settings window closes with the browser window, so the
                // event loop can end; import-ui lane: the import window too.
                WindowEvent::CloseRequested => {
                    crate::settings::main_window_closing();
                    crate::import_ui::main_window_closing();
                }
                WindowEvent::Focused(false) => {
                    // Key-ups that happen while another window has focus never arrive.
                    shortcut_keys.clear();
                    dismiss_composition(&controller);
                }
                _ => {}
            }

            EventResult::Propagate
        });
}

/// Whether a key goes to the page: only while the page has the keyboard (the downloads list,
/// the quit question and the address field take it away) and only if it was not a browser
/// shortcut. (downloads-ui lane)
pub(crate) fn forwards_to_page(web_view_focused: bool, handled_as_shortcut: bool) -> bool {
    web_view_focused && !handled_as_shortcut
}

pub(crate) fn keyboard_input_from_winit(
    event: &winit::event::KeyEvent,
    modifiers: ModifiersState,
    is_composing: bool,
) -> KeyboardInput {
    KeyboardInput {
        state: match event.state {
            ElementState::Pressed => KeyState::Down,
            ElementState::Released => KeyState::Up,
        },
        key: logical_key_string(&event.logical_key),
        code: physical_key_string(event.physical_key),
        location: key_location(event.location),
        modifiers: KeyboardModifiers {
            alt: modifiers.alt_key(),
            control: modifiers.control_key(),
            meta: modifiers.super_key(),
            shift: modifiers.shift_key(),
        },
        repeat: event.repeat,
        is_composing,
    }
}

pub(crate) fn color_scheme_from_winit(theme: Theme) -> ColorScheme {
    match theme {
        Theme::Light => ColorScheme::Light,
        Theme::Dark => ColorScheme::Dark,
    }
}

/// The system appearance when rendering starts: winit's report, or else the colour scheme
/// Slint determined for the window (on Linux it also asks the desktop settings portal).
pub(crate) fn system_color_scheme(
    winit_theme: Option<Theme>,
    slint_scheme: SlintColorScheme,
) -> Option<ColorScheme> {
    if let Some(theme) = winit_theme {
        return Some(color_scheme_from_winit(theme));
    }
    match slint_scheme {
        SlintColorScheme::Dark => Some(ColorScheme::Dark),
        SlintColorScheme::Light => Some(ColorScheme::Light),
        _ => None,
    }
}

/// winit reports the change in magnification (positive when zooming in); Servo multiplies the
/// current pinch zoom by the factor. The position is in logical web-view pixels.
pub(crate) fn pinch_zoom_command(delta: f64, x: f32, y: f32) -> BrowserCommand {
    BrowserCommand::PinchZoom {
        delta: delta as f32 + 1.0,
        x,
        y,
    }
}

pub(crate) fn physical_key_string(key: PhysicalKey) -> String {
    keyutils::code_from_winit(key).to_string()
}

pub(crate) fn logical_key_string(key: &Key) -> String {
    keyutils::key_from_winit(key).to_string()
}

fn key_location(location: WinitKeyLocation) -> KeyLocation {
    match location {
        WinitKeyLocation::Standard => KeyLocation::Standard,
        WinitKeyLocation::Left => KeyLocation::Left,
        WinitKeyLocation::Right => KeyLocation::Right,
        WinitKeyLocation::Numpad => KeyLocation::Numpad,
    }
}

/// winit's input-method events for the page: the state machine in `ime.rs` turns them into
/// commands for the tab that started the composition. (Kept here as the entry point the key and
/// window handlers have always used.)
pub(crate) fn queue_ime_input(controller: &RefCell<BrowserController>, ime: &Ime) {
    crate::ime::handle(
        controller,
        match ime {
            Ime::Enabled => crate::ime::Input::Enabled,
            Ime::Preedit(text, cursor) => crate::ime::Input::Preedit {
                text: text.clone(),
                cursor: *cursor,
            },
            Ime::Commit(text) => crate::ime::Input::Commit(text.clone()),
            Ime::Disabled => crate::ime::Input::Disabled,
        },
    );
}

/// Ends the composition without a result (a sheet is shown, the focus left): the page's
/// composition is dismissed.
pub(crate) fn dismiss_composition(controller: &RefCell<BrowserController>) {
    crate::ime::dismiss_composition(controller);
}

/// Tells `ime.rs` what it needs besides a controller per call: a redraw, and whether the page
/// may take text from the system now (it has the keyboard, no sheet covers it, and no field of
/// the window itself has the text focus: the address bar takes an emoji for itself).
fn attach_ime(
    window: &MainWindow,
    controller: &Rc<RefCell<BrowserController>>,
    web_view_focused: &Rc<Cell<bool>>,
) {
    let redraw_window = window.as_weak();
    let accepts_window = window.as_weak();
    let accepts_controller = Rc::downgrade(controller);
    let web_view_focused = Rc::clone(web_view_focused);
    crate::ime::attach(
        controller,
        move || {
            if let Some(window) = redraw_window.upgrade() {
                window.window().request_redraw();
            }
        },
        move || {
            web_view_focused.get()
                && accepts_window
                    .upgrade()
                    .is_some_and(|window| !window.get_text_input_focused())
                && accepts_controller.upgrade().is_some_and(|controller| {
                    controller
                        .try_borrow()
                        .is_ok_and(|controller| !crate::page_requests::sheet_shown(&controller))
                })
        },
    );
}

#[cfg(test)]
mod tests {
    use talaria_core::{BrowserEvent, ImeClause, ImeClauseKind, ImeInput, TabId, Utf16Range};

    use super::*;

    fn mark_engine_ready(controller: &RefCell<BrowserController>, tab_id: TabId) {
        let handled = controller
            .borrow_mut()
            .handle_event(tab_id, BrowserEvent::AddressChanged { url: String::new() });
        assert!(handled);
    }

    #[test]
    fn logical_keys_use_web_keyboard_event_values() {
        assert_eq!(logical_key_string(&Key::Character("ç".into())), "ç");
        assert_eq!(
            logical_key_string(&Key::Named(winit::keyboard::NamedKey::Enter)),
            "Enter"
        );
        assert_eq!(
            logical_key_string(&Key::Named(winit::keyboard::NamedKey::Space)),
            " "
        );
        assert_eq!(logical_key_string(&Key::Dead(Some('´'))), "Dead");
    }

    #[test]
    fn physical_keys_use_web_keyboard_event_code_values() {
        use winit::keyboard::KeyCode;

        assert_eq!(physical_key_string(KeyCode::KeyA.into()), "KeyA");
        assert_eq!(physical_key_string(KeyCode::SuperLeft.into()), "MetaLeft");
        assert_eq!(physical_key_string(KeyCode::SuperRight.into()), "MetaRight");
        assert_eq!(physical_key_string(KeyCode::Meta.into()), "Super");
    }

    #[test]
    fn arrow_keys_and_function_keys_map_to_w3c_names() {
        use winit::keyboard::{KeyCode, NamedKey};

        assert_eq!(
            logical_key_string(&Key::Named(NamedKey::ArrowLeft)),
            "ArrowLeft"
        );
        assert_eq!(
            logical_key_string(&Key::Named(NamedKey::ArrowDown)),
            "ArrowDown"
        );
        assert_eq!(logical_key_string(&Key::Named(NamedKey::F5)), "F5");
        assert_eq!(logical_key_string(&Key::Named(NamedKey::F12)), "F12");
        assert_eq!(logical_key_string(&Key::Named(NamedKey::Super)), "Meta");
        assert_eq!(
            physical_key_string(KeyCode::ArrowRight.into()),
            "ArrowRight"
        );
        assert_eq!(physical_key_string(KeyCode::F5.into()), "F5");
        assert_eq!(
            physical_key_string(KeyCode::NumpadEnter.into()),
            "NumpadEnter"
        );
    }

    #[test]
    fn pinch_gestures_become_pinch_zoom_factors_at_the_pointer() {
        assert_eq!(
            pinch_zoom_command(0.25, 10.0, 20.0),
            BrowserCommand::PinchZoom {
                delta: 1.25,
                x: 10.0,
                y: 20.0,
            }
        );
        assert_eq!(
            pinch_zoom_command(-0.5, 0.0, 0.0),
            BrowserCommand::PinchZoom {
                delta: 0.5,
                x: 0.0,
                y: 0.0,
            }
        );
    }

    #[test]
    fn winit_themes_become_colour_schemes() {
        assert_eq!(color_scheme_from_winit(Theme::Dark), ColorScheme::Dark);
        assert_eq!(color_scheme_from_winit(Theme::Light), ColorScheme::Light);
    }

    #[test]
    fn the_start_up_scheme_prefers_winit_then_slint() {
        assert_eq!(
            system_color_scheme(Some(Theme::Dark), SlintColorScheme::Light),
            Some(ColorScheme::Dark)
        );
        assert_eq!(
            system_color_scheme(None, SlintColorScheme::Dark),
            Some(ColorScheme::Dark)
        );
        assert_eq!(
            system_color_scheme(None, SlintColorScheme::Light),
            Some(ColorScheme::Light)
        );
        assert_eq!(system_color_scheme(None, SlintColorScheme::Unknown), None);
    }

    #[test]
    fn every_key_location_maps_to_its_core_location() {
        assert_eq!(
            key_location(WinitKeyLocation::Standard),
            KeyLocation::Standard
        );
        assert_eq!(key_location(WinitKeyLocation::Left), KeyLocation::Left);
        assert_eq!(key_location(WinitKeyLocation::Right), KeyLocation::Right);
        assert_eq!(key_location(WinitKeyLocation::Numpad), KeyLocation::Numpad);
    }

    // txt-1: the entry points hand winit's events to `ime.rs`; the rules are tested there.
    #[test]
    fn ime_preedit_and_commit_form_one_composition_sequence() {
        let _guard = crate::ime::test_guard();
        let controller = RefCell::new(BrowserController::default());
        let tab_id = controller.borrow().active_tab_id();
        mark_engine_ready(&controller, tab_id);

        // "aç" is 3 bytes in UTF-8 and 2 units in UTF-16: the cursor at its end.
        queue_ime_input(&controller, &Ime::Preedit("aç".to_owned(), Some((3, 3))));
        assert!(crate::ime::composition_active());
        assert!(crate::ime::composing_in(tab_id));
        // winit sends an empty preedit right before the commit.
        queue_ime_input(&controller, &Ime::Preedit(String::new(), None));
        queue_ime_input(&controller, &Ime::Commit("ação".to_owned()));

        let commands =
            std::iter::from_fn(|| controller.borrow_mut().next_command()).collect::<Vec<_>>();
        assert_eq!(commands.len(), 2);
        assert!(commands.iter().all(|command| command.tab_id() == tab_id));
        assert_eq!(
            commands[0].command(),
            &BrowserCommand::ImeInput(ImeInput::SetMarkedText {
                text: "aç".to_owned(),
                selected: Some(Utf16Range::collapsed(2)),
                clauses: vec![ImeClause {
                    range: Utf16Range::new(0, 2),
                    kind: ImeClauseKind::Raw,
                }],
                replacement: None,
            })
        );
        assert_eq!(
            commands[1].command(),
            &BrowserCommand::ImeInput(ImeInput::Commit("ação".to_owned()))
        );
        assert!(!crate::ime::composition_active());
    }

    #[test]
    fn ime_dismissal_stays_with_the_tab_that_started_composition() {
        let _guard = crate::ime::test_guard();
        let controller = RefCell::new(BrowserController::default());
        let composing_tab_id = controller.borrow().active_tab_id();
        mark_engine_ready(&controller, composing_tab_id);
        queue_ime_input(&controller, &Ime::Preedit("her".to_owned(), None));
        controller.borrow_mut().open_tab();

        queue_ime_input(&controller, &Ime::Disabled);

        let commands =
            std::iter::from_fn(|| controller.borrow_mut().next_command()).collect::<Vec<_>>();
        assert!(
            commands
                .iter()
                .all(|command| command.tab_id() == composing_tab_id)
        );
        assert_eq!(
            commands.last().map(|command| command.command()),
            Some(&BrowserCommand::ImeInput(ImeInput::Dismissed))
        );
        assert!(!crate::ime::composition_active());
    }

    #[test]
    fn an_empty_preedit_that_nothing_follows_is_a_cancel_at_the_end_of_the_turn() {
        let _guard = crate::ime::test_guard();
        let controller = RefCell::new(BrowserController::default());
        let tab_id = controller.borrow().active_tab_id();
        mark_engine_ready(&controller, tab_id);
        queue_ime_input(&controller, &Ime::Preedit("ni".to_owned(), Some((2, 2))));
        queue_ime_input(&controller, &Ime::Preedit(String::new(), None));

        // What the zero-delay timer runs.
        crate::ime::flush(&controller);

        let commands =
            std::iter::from_fn(|| controller.borrow_mut().next_command()).collect::<Vec<_>>();
        assert_eq!(
            commands.last().map(|command| command.command()),
            Some(&BrowserCommand::ImeInput(ImeInput::Commit(String::new())))
        );
        assert!(!crate::ime::composition_active());
    }

    #[test]
    fn a_sheet_over_the_page_dismisses_the_composition() {
        let _guard = crate::ime::test_guard();
        let controller = RefCell::new(BrowserController::default());
        let tab_id = controller.borrow().active_tab_id();
        mark_engine_ready(&controller, tab_id);
        queue_ime_input(&controller, &Ime::Preedit("ni".to_owned(), Some((2, 2))));

        dismiss_composition(&controller);

        let commands =
            std::iter::from_fn(|| controller.borrow_mut().next_command()).collect::<Vec<_>>();
        assert_eq!(
            commands.last().map(|command| command.command()),
            Some(&BrowserCommand::ImeInput(ImeInput::Dismissed))
        );
        assert!(!crate::ime::composition_active());
    }

    // downloads-ui lane
    #[test]
    fn a_key_goes_to_the_page_only_while_it_has_the_keyboard_and_was_no_shortcut() {
        assert!(forwards_to_page(true, false));
        assert!(!forwards_to_page(true, true));
        // The quit question or the downloads list has taken the keyboard.
        assert!(!forwards_to_page(false, false));
        assert!(!forwards_to_page(false, true));
    }
}
