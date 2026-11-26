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
    let mut pinch = PinchTracker::default();
    // txt-1: `ime.rs` holds the composition state and queues commands itself (text from the
    // system's emoji picker and dictation arrives outside this handler).
    attach_ime(window, &controller, &web_view_focused);
    // input-hooks: the ⌥ group keys and the Orbit pinch follow these settings.
    crate::input_hooks::follow_settings(settings);

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
                    // input-hooks: the ⌥ group keys and the ⌃⇥ hold follow the modifier state.
                    crate::hook_chain::modifiers(
                        &window,
                        &controller,
                        web_view_focused.get(),
                        modifiers,
                    );
                }
                WindowEvent::KeyboardInput {
                    event,
                    is_synthetic,
                    ..
                } => {
                    // input-hooks: the ring's ⌥ keys, the ⌥ group keys, Orbit's keys and the ⌃⇥
                    // hold see every key before anything else, so a captured key's key-up is
                    // never taken by a popup first.
                    if crate::hook_chain::early(
                        &window,
                        &controller,
                        web_view_focused.get(),
                        event,
                        modifiers,
                        *is_synthetic,
                    ) {
                        window.window().request_redraw();
                        return EventResult::PreventDefault;
                    }
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
                    // input-hooks: F6 and ⇧F6 move through the window's regions, after the
                    // popups and the suggestions had their keys.
                    if crate::hook_chain::f6(&controller, event, modifiers, *is_synthetic) {
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
                // input-hooks: a gesture Orbit owns is consumed without the hover guard.
                WindowEvent::PinchGesture { delta, phase, .. }
                    if pinch.orbit_owned()
                        || (window.get_web_view_visible() && window.get_web_view_hovered()) =>
                {
                    let tab = controller.borrow().active_tab_id();
                    let step = pinch.step(tab, pinch_phase(*phase), *delta, orbit_pinch_enabled());
                    let orbit = crate::input_hooks::orbit_host();
                    if step.abandoned_orbit
                        && let Some(orbit) = &orbit
                    {
                        orbit.pinch_open_end(false);
                    }
                    match step.route {
                        PinchRoute::Forward => {
                            controller.borrow_mut().queue_command(pinch_zoom_command(
                                *delta,
                                window.get_web_view_pointer_x(),
                                window.get_web_view_pointer_y(),
                            ));
                            window.window().request_redraw();
                        }
                        PinchRoute::DriveOrbit { progress } => {
                            if let Some(orbit) = &orbit {
                                orbit.pinch_open_progress(progress);
                            }
                        }
                    }
                    if let (Some(commit), Some(orbit)) = (step.finished, &orbit) {
                        orbit.pinch_open_end(commit);
                    }
                    return EventResult::PreventDefault;
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
                    crate::hook_chain::reset(&window, &controller); // input-hooks
                }
                WindowEvent::Focused(false) => {
                    // Key-ups that happen while another window has focus never arrive.
                    shortcut_keys.clear();
                    dismiss_composition(&controller);
                    crate::hook_chain::reset(&window, &controller); // input-hooks
                }
                // input-hooks: a hidden window never delivers the key-up of a held ⌥ or ⌃.
                WindowEvent::Occluded(true) => {
                    crate::hook_chain::reset(&window, &controller);
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

/// A closing pinch shows Orbit from this product of `1 + delta` since the gesture started
/// (SPEC section 6.1) and commits at `PINCH_ORBIT_COMMIT`.
const PINCH_ORBIT_SHOW: f64 = 0.85;
const PINCH_ORBIT_COMMIT: f64 = 0.70;
const PINCH_ORBIT_SPAN: f64 = 0.15;
/// Float slack on the thresholds, so that a product that is 0.85 or 0.70 in decimal counts.
const PINCH_EPSILON: f64 = 1e-9;
/// Servo's pinch zoom range, and how far above 1.0 a page still counts as unzoomed.
const PAGE_PINCH_ZOOM_MAX: f32 = 10.0;
const PAGE_PINCH_ZOOM_TOLERANCE: f64 = 1e-3;

/// Where one pinch event goes (SPEC section 6.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PinchRoute {
    /// To the page, as before.
    Forward,
    /// To Orbit: the page lifts toward the ring. `progress` is 0.0 at rest and 1.0 at the
    /// committed distance.
    DriveOrbit { progress: f32 },
}

fn pinch_orbit_progress(cumulative: f64) -> f32 {
    ((PINCH_ORBIT_SHOW - cumulative) / PINCH_ORBIT_SPAN).clamp(0.0, 1.0) as f32
}

/// The route of one event of a pinch that is not owned yet. `page_pinch_zoom` is the page's zoom
/// at the start of the gesture, `delta` this event's change in magnification and `cumulative`
/// the product of `1 + delta` since the gesture started.
pub(crate) fn route_pinch(
    page_pinch_zoom: f32,
    delta: f64,
    cumulative: f64,
    orbit_enabled: bool,
) -> PinchRoute {
    if !orbit_enabled
        || !page_pinch_zoom.is_finite()
        || !delta.is_finite()
        || !cumulative.is_finite()
        || f64::from(page_pinch_zoom) > 1.0 + PAGE_PINCH_ZOOM_TOLERANCE
        || (delta >= 0.0 && cumulative >= 1.0)
        || cumulative > PINCH_ORBIT_SHOW + PINCH_EPSILON
    {
        return PinchRoute::Forward;
    }
    PinchRoute::DriveOrbit {
        progress: pinch_orbit_progress(cumulative),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PinchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

pub(crate) fn pinch_phase(phase: winit::event::TouchPhase) -> PinchPhase {
    match phase {
        winit::event::TouchPhase::Started => PinchPhase::Started,
        winit::event::TouchPhase::Moved => PinchPhase::Moved,
        winit::event::TouchPhase::Ended => PinchPhase::Ended,
        winit::event::TouchPhase::Cancelled => PinchPhase::Cancelled,
    }
}

/// Who a gesture belongs to. It is decided once and then kept for the whole gesture, so that a
/// gesture is never split between a half-zoomed page and a half-open Orbit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PinchOwner {
    /// Closing from rest and still above the threshold: the page gets the events (they cannot
    /// zoom an unzoomed page out), and the gesture can still become Orbit's.
    Pending,
    Page,
    Orbit,
}

#[derive(Clone, Copy, Debug)]
struct PinchGesture {
    start_zoom: f32,
    cumulative: f64,
    owner: PinchOwner,
}

/// What `PinchTracker::step` decided for one event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PinchStep {
    pub(crate) route: PinchRoute,
    /// A gesture that was Orbit's was still open when this one started: tell Orbit it ended
    /// without a commit (`pinch_open_end(false)`).
    pub(crate) abandoned_orbit: bool,
    /// The gesture ended on this event and was Orbit's: `Some(commit)`.
    pub(crate) finished: Option<bool>,
}

/// The state behind `route_pinch`: a shadow estimate of each tab's pinch zoom (the desktop cannot
/// read Servo's) and the gesture in progress.
#[derive(Default)]
pub(crate) struct PinchTracker {
    page_zoom: std::collections::HashMap<talaria_core::TabId, f32>,
    gesture: Option<PinchGesture>,
}

impl PinchTracker {
    /// The page's estimated pinch zoom; 1.0 for a tab with no entry.
    fn estimate(&self, tab: talaria_core::TabId) -> f32 {
        self.page_zoom.get(&tab).copied().unwrap_or(1.0)
    }

    /// A delta that was forwarded to the page. Servo clamps the zoom to 1.0..=10.0.
    fn forwarded(&mut self, tab: talaria_core::TabId, delta: f64) {
        if !delta.is_finite() {
            return;
        }
        let zoom = (f64::from(self.estimate(tab)) * (1.0 + delta))
            .clamp(1.0, f64::from(PAGE_PINCH_ZOOM_MAX)) as f32;
        if zoom > 1.0 {
            self.page_zoom.insert(tab, zoom);
        } else {
            self.page_zoom.remove(&tab);
        }
    }

    /// The tab closed.
    #[allow(dead_code)] // input-hooks: item 3 calls it where a tab closes
    pub(crate) fn forget(&mut self, tab: talaria_core::TabId) {
        self.page_zoom.remove(&tab);
    }

    pub(crate) fn orbit_owned(&self) -> bool {
        self.gesture
            .is_some_and(|gesture| gesture.owner == PinchOwner::Orbit)
    }

    /// Starts a gesture at the tab's current estimate. Returns whether it ended an open gesture
    /// that was Orbit's, without a commit.
    fn begin(&mut self, tab: talaria_core::TabId) -> bool {
        let abandoned = self.orbit_owned();
        self.gesture = Some(PinchGesture {
            start_zoom: self.estimate(tab),
            cumulative: 1.0,
            owner: PinchOwner::Pending,
        });
        abandoned
    }

    fn update(&mut self, delta: f64, orbit_enabled: bool) -> PinchRoute {
        let Some(gesture) = self.gesture.as_mut() else {
            return PinchRoute::Forward;
        };
        if delta.is_finite() {
            let next = gesture.cumulative * (1.0 + delta).max(0.0);
            if next.is_finite() {
                gesture.cumulative = next;
            }
        }
        match gesture.owner {
            PinchOwner::Page => PinchRoute::Forward,
            PinchOwner::Orbit => PinchRoute::DriveOrbit {
                progress: pinch_orbit_progress(gesture.cumulative),
            },
            PinchOwner::Pending => {
                let route =
                    route_pinch(gesture.start_zoom, delta, gesture.cumulative, orbit_enabled);
                if matches!(route, PinchRoute::DriveOrbit { .. }) {
                    gesture.owner = PinchOwner::Orbit;
                } else if !orbit_enabled
                    || f64::from(gesture.start_zoom) > 1.0 + PAGE_PINCH_ZOOM_TOLERANCE
                    || gesture.cumulative > 1.0 + PAGE_PINCH_ZOOM_TOLERANCE
                {
                    gesture.owner = PinchOwner::Page;
                }
                route
            }
        }
    }

    /// The gesture ended: `Some(commit)` if it was Orbit's.
    fn finish(&mut self, cancelled: bool) -> Option<bool> {
        let gesture = self.gesture.take()?;
        let commit = !cancelled && gesture.cumulative <= PINCH_ORBIT_COMMIT + PINCH_EPSILON;
        (gesture.owner == PinchOwner::Orbit).then_some(commit)
    }

    /// Ends the gesture without a commit (focus lost, window closing). `Some(false)` if it was
    /// Orbit's, which must then be told.
    #[allow(dead_code)] // input-hooks: item 3 calls it on `Focused(false)` and `CloseRequested`
    pub(crate) fn cancel(&mut self) -> Option<bool> {
        self.finish(true)
    }

    /// One pinch event: the route, and what to tell Orbit about the gesture's end.
    pub(crate) fn step(
        &mut self,
        tab: talaria_core::TabId,
        phase: PinchPhase,
        delta: f64,
        orbit_enabled: bool,
    ) -> PinchStep {
        let abandoned_orbit = if phase == PinchPhase::Started || self.gesture.is_none() {
            self.begin(tab)
        } else {
            false
        };
        let route = self.update(delta, orbit_enabled);
        if route == PinchRoute::Forward {
            self.forwarded(tab, delta);
        }
        let finished = match phase {
            PinchPhase::Ended => self.finish(false),
            PinchPhase::Cancelled => self.finish(true),
            PinchPhase::Started | PinchPhase::Moved => None,
        };
        PinchStep {
            route,
            abandoned_orbit,
            finished,
        }
    }
}

/// Orbit takes a pinch only when it exists in this layout and is not already open.
fn orbit_pinch_enabled() -> bool {
    // input-hooks: item 3 adds the orbit_pinch setting
    crate::input_hooks::orbit_host().is_some_and(|host| host.available() && !host.is_open())
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

    // input-hooks (SPEC section 6.1): the pinch route.
    const TAB_A: TabId = TabId::new(1);
    const TAB_B: TabId = TabId::new(2);

    fn assert_drives_orbit(route: PinchRoute, expected_progress: f32) {
        match route {
            PinchRoute::DriveOrbit { progress } => assert!(
                (progress - expected_progress).abs() < 1e-5,
                "progress {progress}, expected {expected_progress}"
            ),
            PinchRoute::Forward => panic!("expected DriveOrbit, got Forward"),
        }
    }

    #[test]
    fn a_closing_pinch_on_an_unzoomed_page_opens_orbit_between_0_85_and_0_70() {
        let closing = |cumulative| route_pinch(1.0, -0.01, cumulative, true);

        assert_eq!(closing(1.0), PinchRoute::Forward);
        assert_eq!(closing(0.9), PinchRoute::Forward);
        // The boundary: 0.85 exactly is the first DriveOrbit, 0.8501 is not.
        assert_eq!(closing(0.85), PinchRoute::DriveOrbit { progress: 0.0 });
        assert_eq!(closing(0.8501), PinchRoute::Forward);
        assert_drives_orbit(closing(0.775), 0.5);
        assert_drives_orbit(closing(0.7), 1.0);
        assert_drives_orbit(closing(0.5), 1.0);
    }

    #[test]
    fn a_zoomed_page_always_gets_the_pinch() {
        for page_zoom in [1.3_f32, 10.0] {
            for cumulative in [1.0, 0.9, 0.85, 0.775, 0.7, 0.5, 0.1] {
                assert_eq!(
                    route_pinch(page_zoom, -0.05, cumulative, true),
                    PinchRoute::Forward,
                    "page {page_zoom}, cumulative {cumulative}"
                );
            }
        }
    }

    #[test]
    fn a_page_zoom_within_the_tolerance_counts_as_unzoomed() {
        assert_drives_orbit(route_pinch(1.0005, -0.05, 0.7, true), 1.0);
        assert_eq!(route_pinch(1.002, -0.05, 0.7, true), PinchRoute::Forward);
    }

    #[test]
    fn an_opening_pinch_goes_to_the_page() {
        assert_eq!(route_pinch(1.0, 0.05, 1.2, true), PinchRoute::Forward);
        assert_eq!(route_pinch(1.0, 0.0, 1.0, true), PinchRoute::Forward);
    }

    #[test]
    fn fingers_reopening_below_the_threshold_still_drive_orbit() {
        assert_drives_orbit(route_pinch(1.0, 0.02, 0.8, true), 1.0 / 3.0);
    }

    #[test]
    fn with_orbit_disabled_every_pinch_goes_to_the_page() {
        for cumulative in [1.2, 1.0, 0.9, 0.85, 0.775, 0.7, 0.5, 0.0] {
            assert_eq!(
                route_pinch(1.0, -0.05, cumulative, false),
                PinchRoute::Forward,
                "cumulative {cumulative}"
            );
        }
    }

    #[test]
    fn non_finite_pinch_input_goes_to_the_page() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(route_pinch(1.0, bad, 0.7, true), PinchRoute::Forward);
            assert_eq!(route_pinch(1.0, -0.05, bad, true), PinchRoute::Forward);
        }
        for bad in [f32::NAN, f32::INFINITY] {
            assert_eq!(route_pinch(bad, -0.05, 0.7, true), PinchRoute::Forward);
        }
    }

    #[test]
    fn the_orbit_progress_never_leaves_zero_to_one() {
        let mut cumulative = -0.5;
        while cumulative < 3.0 {
            for delta in [-0.9, -0.01, 0.0, 0.01, 0.9] {
                if let PinchRoute::DriveOrbit { progress } =
                    route_pinch(1.0, delta, cumulative, true)
                {
                    assert!(
                        (0.0..=1.0).contains(&progress),
                        "progress {progress} at {cumulative}"
                    );
                }
            }
            cumulative += 0.005;
        }
    }

    #[test]
    fn a_closing_gesture_enters_orbit_at_0_85_and_stays_there_when_the_fingers_reopen() {
        let mut tracker = PinchTracker::default();
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        let mut first_orbit_step = None;
        for step in 1..=10 {
            let result = tracker.step(TAB_A, PinchPhase::Moved, -0.03, true);
            if matches!(result.route, PinchRoute::DriveOrbit { .. }) {
                first_orbit_step.get_or_insert(step);
            } else {
                assert!(first_orbit_step.is_none(), "left Orbit at step {step}");
            }
        }
        // 0.97^5 = 0.8587, 0.97^6 = 0.8330.
        assert_eq!(first_orbit_step, Some(6));

        // The product rises from 0.737 to above 0.95: still Orbit's.
        let mut cumulative: f64 = 0.97_f64.powi(10);
        while cumulative < 0.95 {
            cumulative *= 1.03;
            let result = tracker.step(TAB_A, PinchPhase::Moved, 0.03, true);
            let PinchRoute::DriveOrbit { progress } = result.route else {
                panic!("the gesture left Orbit at {cumulative}");
            };
            assert!((0.0..=1.0).contains(&progress));
        }
        assert_eq!(
            tracker.step(TAB_A, PinchPhase::Ended, 0.0, true).finished,
            Some(false)
        );
    }

    #[test]
    fn a_gesture_on_a_zoomed_page_stays_the_pages_through_a_product_of_0_6() {
        let mut tracker = PinchTracker::default();
        tracker.forwarded(TAB_A, 1.0); // the page is at 2.0
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        let mut cumulative = 1.0;
        while cumulative > 0.5 {
            cumulative *= 0.97;
            let result = tracker.step(TAB_A, PinchPhase::Moved, -0.03, true);
            assert_eq!(result.route, PinchRoute::Forward, "at {cumulative}");
        }
        assert_eq!(
            tracker.step(TAB_A, PinchPhase::Ended, 0.0, true).finished,
            None
        );
    }

    #[test]
    fn a_gesture_that_opens_first_stays_the_pages_when_it_then_closes() {
        let mut tracker = PinchTracker::default();
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        assert_eq!(
            tracker.step(TAB_A, PinchPhase::Moved, 0.2, true).route,
            PinchRoute::Forward
        );
        // 1.2 * 0.5 = 0.6
        assert_eq!(
            tracker.step(TAB_A, PinchPhase::Moved, -0.5, true).route,
            PinchRoute::Forward
        );
        assert_eq!(
            tracker.step(TAB_A, PinchPhase::Ended, 0.0, true).finished,
            None
        );
    }

    #[test]
    fn a_gesture_that_began_with_orbit_unavailable_stays_the_pages() {
        let mut tracker = PinchTracker::default();
        tracker.step(TAB_A, PinchPhase::Started, 0.0, false);
        assert_eq!(
            tracker.step(TAB_A, PinchPhase::Moved, -0.4, true).route,
            PinchRoute::Forward
        );
    }

    #[test]
    fn ending_an_orbit_gesture_commits_at_or_below_0_70() {
        for (delta, commit) in [(-0.3, true), (-0.31, true), (-0.29, false), (-0.5, true)] {
            let mut tracker = PinchTracker::default();
            tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
            let moved = tracker.step(TAB_A, PinchPhase::Moved, delta, true);
            assert!(matches!(moved.route, PinchRoute::DriveOrbit { .. }));
            let ended = tracker.step(TAB_A, PinchPhase::Ended, 0.0, true);
            assert_eq!(ended.finished, Some(commit), "delta {delta}");
        }
    }

    #[test]
    fn the_final_delta_of_the_ended_event_counts_towards_the_commit() {
        let mut tracker = PinchTracker::default();
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        tracker.step(TAB_A, PinchPhase::Moved, -0.25, true); // 0.75
        let ended = tracker.step(TAB_A, PinchPhase::Ended, -0.1, true); // 0.675
        assert_drives_orbit(ended.route, 1.0);
        assert_eq!(ended.finished, Some(true));
    }

    #[test]
    fn a_cancelled_orbit_gesture_never_commits() {
        let mut tracker = PinchTracker::default();
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        tracker.step(TAB_A, PinchPhase::Moved, -0.5, true);
        let cancelled = tracker.step(TAB_A, PinchPhase::Cancelled, 0.0, true);
        assert_eq!(cancelled.finished, Some(false));
        assert!(!tracker.orbit_owned());
    }

    #[test]
    fn a_new_started_ends_a_stale_orbit_gesture_without_a_commit() {
        let mut tracker = PinchTracker::default();
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        tracker.step(TAB_A, PinchPhase::Moved, -0.5, true);
        assert!(tracker.orbit_owned());

        let restarted = tracker.step(TAB_A, PinchPhase::Started, 0.0, true);

        assert!(restarted.abandoned_orbit);
        assert_eq!(restarted.finished, None);
        // The new gesture starts again at 1.0.
        assert_eq!(restarted.route, PinchRoute::Forward);
        assert!(!tracker.orbit_owned());
        assert_eq!(
            tracker.step(TAB_A, PinchPhase::Ended, 0.0, true).finished,
            None
        );
    }

    #[test]
    fn a_new_started_after_a_stale_page_gesture_abandons_nothing() {
        let mut tracker = PinchTracker::default();
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        tracker.step(TAB_A, PinchPhase::Moved, 0.2, true);
        let restarted = tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        assert!(!restarted.abandoned_orbit);
    }

    #[test]
    fn a_moved_event_without_started_is_forwarded_as_before() {
        let mut tracker = PinchTracker::default();
        let step = tracker.step(TAB_A, PinchPhase::Moved, 0.1, true);
        assert_eq!(step.route, PinchRoute::Forward);
        assert!(!step.abandoned_orbit);
        assert!((tracker.estimate(TAB_A) - 1.1).abs() < 1e-6);
    }

    #[test]
    fn the_page_zoom_estimate_multiplies_forwarded_deltas() {
        let mut tracker = PinchTracker::default();
        tracker.forwarded(TAB_A, 0.25);
        tracker.forwarded(TAB_A, 0.25);
        assert_eq!(tracker.estimate(TAB_A), 1.5625);
    }

    #[test]
    fn the_page_zoom_estimate_drops_its_entry_when_it_returns_to_one() {
        let mut tracker = PinchTracker::default();
        tracker.forwarded(TAB_A, 0.25);
        assert_eq!(tracker.page_zoom.len(), 1);
        tracker.forwarded(TAB_A, -0.9);
        assert_eq!(tracker.estimate(TAB_A), 1.0);
        assert!(tracker.page_zoom.is_empty());
        // Zooming out of a page that is not zoomed makes no entry.
        tracker.forwarded(TAB_A, -0.5);
        assert!(tracker.page_zoom.is_empty());
    }

    #[test]
    fn the_page_zoom_estimate_clamps_at_ten() {
        let mut tracker = PinchTracker::default();
        for _ in 0..40 {
            tracker.forwarded(TAB_A, 0.25);
        }
        assert_eq!(tracker.estimate(TAB_A), 10.0);
    }

    #[test]
    fn the_page_zoom_estimate_ignores_non_finite_deltas() {
        let mut tracker = PinchTracker::default();
        tracker.forwarded(TAB_A, 0.5);
        tracker.forwarded(TAB_A, f64::NAN);
        tracker.forwarded(TAB_A, f64::INFINITY);
        assert_eq!(tracker.estimate(TAB_A), 1.5);
    }

    #[test]
    fn the_page_zoom_estimate_is_per_tab() {
        let mut tracker = PinchTracker::default();
        tracker.forwarded(TAB_A, 1.0); // tab A at 2.0
        tracker.step(TAB_B, PinchPhase::Started, 0.0, true);
        assert_drives_orbit(
            tracker.step(TAB_B, PinchPhase::Moved, -0.3, true).route,
            1.0,
        );
        tracker.step(TAB_B, PinchPhase::Ended, 0.0, true);

        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        assert_eq!(
            tracker.step(TAB_A, PinchPhase::Moved, -0.3, true).route,
            PinchRoute::Forward
        );
    }

    #[test]
    fn a_gestures_route_uses_the_page_zoom_at_its_start() {
        // Unzoomed at the start: a forwarded delta that raises the estimate mid-gesture does
        // not take Orbit away.
        let mut tracker = PinchTracker::default();
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        tracker.forwarded(TAB_A, 0.5);
        assert_drives_orbit(
            tracker.step(TAB_A, PinchPhase::Moved, -0.3, true).route,
            1.0,
        );

        // Zoomed at the start: the estimate falling to 1.0 mid-gesture does not give Orbit the
        // rest of it.
        let mut tracker = PinchTracker::default();
        tracker.forwarded(TAB_A, 0.5);
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        tracker.forwarded(TAB_A, -0.9);
        assert_eq!(tracker.estimate(TAB_A), 1.0);
        assert_eq!(
            tracker.step(TAB_A, PinchPhase::Moved, -0.3, true).route,
            PinchRoute::Forward
        );
    }

    #[test]
    fn forwarded_deltas_move_the_estimate_and_orbit_driven_ones_do_not() {
        let mut tracker = PinchTracker::default();
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        tracker.step(TAB_A, PinchPhase::Moved, -0.1, true); // Forward at 0.9
        tracker.step(TAB_A, PinchPhase::Moved, -0.2, true); // Orbit at 0.72
        tracker.step(TAB_A, PinchPhase::Moved, -0.2, true); // Orbit at 0.576
        assert_eq!(tracker.estimate(TAB_A), 1.0);

        let mut tracker = PinchTracker::default();
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        tracker.step(TAB_A, PinchPhase::Moved, 0.5, true);
        assert_eq!(tracker.estimate(TAB_A), 1.5);
    }

    #[test]
    fn forgetting_a_tab_drops_its_estimate() {
        let mut tracker = PinchTracker::default();
        tracker.forwarded(TAB_A, 0.5);
        tracker.forwarded(TAB_B, 0.5);
        tracker.forget(TAB_A);
        assert_eq!(tracker.estimate(TAB_A), 1.0);
        assert_eq!(tracker.estimate(TAB_B), 1.5);
    }

    #[test]
    fn non_finite_gesture_deltas_do_not_poison_the_product() {
        let mut tracker = PinchTracker::default();
        tracker.step(TAB_A, PinchPhase::Started, 0.0, true);
        assert_eq!(
            tracker.step(TAB_A, PinchPhase::Moved, f64::NAN, true).route,
            PinchRoute::Forward
        );
        assert_drives_orbit(
            tracker.step(TAB_A, PinchPhase::Moved, -0.3, true).route,
            1.0,
        );
        // Orbit keeps the gesture through a bad delta.
        assert!(matches!(
            tracker
                .step(TAB_A, PinchPhase::Moved, f64::INFINITY, true)
                .route,
            PinchRoute::DriveOrbit { .. }
        ));
    }

    #[test]
    fn winit_touch_phases_map_to_pinch_phases() {
        use winit::event::TouchPhase;

        assert_eq!(pinch_phase(TouchPhase::Started), PinchPhase::Started);
        assert_eq!(pinch_phase(TouchPhase::Moved), PinchPhase::Moved);
        assert_eq!(pinch_phase(TouchPhase::Ended), PinchPhase::Ended);
        assert_eq!(pinch_phase(TouchPhase::Cancelled), PinchPhase::Cancelled);
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
