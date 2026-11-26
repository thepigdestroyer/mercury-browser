// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The window-level input hooks in the order `keyboard.rs` runs them (lane input-hooks, brief
//! `lane-input-hooks-brief.md` D7, D11, D18, D20, D21, D23, D24).
//!
//! `keyboard.rs`' winit closure stays a list of short calls; the decisions are here, over plain
//! values and the host traits of `input_hooks.rs`, so they are tested without a window:
//!
//! | Call | Where in the `KeyboardInput` arm | What |
//! | --- | --- | --- |
//! | [`early`] | first, before `downloads::handle_key` | the ring's ⌥ keys, the ⌥ group-key machine, Orbit's keys, the ⌃⇥ hold |
//! | [`f6`] | after `controls::handle_key` and `history_ui::handle_key`, before the browser shortcuts | the F6 region order |
//! | [`modifiers`] | `ModifiersChanged` | the ⌥ machine and the ⌃⇥ hold see the modifier state |
//! | [`reset`] | `Focused(false)`, `CloseRequested`, `Occluded(true)` | forget everything transient |
//!
//! With no host registered (`input_hooks`) every call passes the event on and does nothing new
//! (D5, D22).

#![allow(dead_code)] // wired by keyboard.rs; kept separate so its tests need no window

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use mercury_ui::{MainWindow, orbit_view};
use slint::winit_030::winit;
use slint::{Timer, TimerMode};
use talaria_core::BrowserController;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

use crate::ctrl_tab::{self, CtrlTab, CtrlTabEnv, CtrlTabOutcome, RING_HOLD};
use crate::focus_regions::{self, F6Event, F6Memory};
use crate::input_hooks::{FocusHost, RingHost, focus_host, orbit_host, ring_host};
use crate::option_hook;
use crate::option_keys::{clock_now, mods_from_winit};

// ---------------------------------------------------------------------------------------------
// Modifier naming (D24).
// ---------------------------------------------------------------------------------------------

/// ⌘ in winit's `ModifiersState`: `super_key()` on every platform.
pub(crate) fn command(mods: ModifiersState) -> bool {
    mods.super_key()
}

/// ⌃ in winit's `ModifiersState`.
pub(crate) fn control(mods: ModifiersState) -> bool {
    mods.control_key()
}

/// ⌘ as Slint's `KeyEvent.modifiers` reports it: on macOS Slint sets `control` for ⌘ and `meta`
/// for ⌃ (the hub-ring review found the swap), so the Command chord is the `control` flag.
/// Nothing in the hooks reads Slint's modifiers today; code that ever does goes through this and
/// [`slint_control`].
pub(crate) fn slint_command(control: bool, _meta: bool) -> bool {
    control
}

/// ⌃ as Slint's `KeyEvent.modifiers` reports it (see [`slint_command`]).
pub(crate) fn slint_control(control: bool, meta: bool) -> bool {
    if cfg!(target_os = "macos") {
        meta
    } else {
        control
    }
}

// ---------------------------------------------------------------------------------------------
// The ring's ⌥ keys (D23).
// ---------------------------------------------------------------------------------------------

/// The character `HubRing::option_key` takes for a physical key: `'A'..='Z'` for a letter key and
/// `'1'..='9'` for a digit or numpad digit. Never from the typed text: macOS makes ⌥S into "ß".
pub(crate) fn ring_option_char(physical: &PhysicalKey) -> Option<char> {
    let PhysicalKey::Code(code) = physical else {
        return None;
    };
    match code {
        KeyCode::KeyA => Some('A'),
        KeyCode::KeyB => Some('B'),
        KeyCode::KeyC => Some('C'),
        KeyCode::KeyD => Some('D'),
        KeyCode::KeyE => Some('E'),
        KeyCode::KeyF => Some('F'),
        KeyCode::KeyG => Some('G'),
        KeyCode::KeyH => Some('H'),
        KeyCode::KeyI => Some('I'),
        KeyCode::KeyJ => Some('J'),
        KeyCode::KeyK => Some('K'),
        KeyCode::KeyL => Some('L'),
        KeyCode::KeyM => Some('M'),
        KeyCode::KeyN => Some('N'),
        KeyCode::KeyO => Some('O'),
        KeyCode::KeyP => Some('P'),
        KeyCode::KeyQ => Some('Q'),
        KeyCode::KeyR => Some('R'),
        KeyCode::KeyS => Some('S'),
        KeyCode::KeyT => Some('T'),
        KeyCode::KeyU => Some('U'),
        KeyCode::KeyV => Some('V'),
        KeyCode::KeyW => Some('W'),
        KeyCode::KeyX => Some('X'),
        KeyCode::KeyY => Some('Y'),
        KeyCode::KeyZ => Some('Z'),
        KeyCode::Digit1 | KeyCode::Numpad1 => Some('1'),
        KeyCode::Digit2 | KeyCode::Numpad2 => Some('2'),
        KeyCode::Digit3 | KeyCode::Numpad3 => Some('3'),
        KeyCode::Digit4 | KeyCode::Numpad4 => Some('4'),
        KeyCode::Digit5 | KeyCode::Numpad5 => Some('5'),
        KeyCode::Digit6 | KeyCode::Numpad6 => Some('6'),
        KeyCode::Digit7 | KeyCode::Numpad7 => Some('7'),
        KeyCode::Digit8 | KeyCode::Numpad8 => Some('8'),
        KeyCode::Digit9 | KeyCode::Numpad9 => Some('9'),
        _ => None,
    }
}

/// What the ring does with one key event while it may be open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RingKey {
    /// Not the ring's.
    None,
    /// ⌥ and this key: call `RingHost::option_key`, and consume the event.
    Send(char),
    /// The repeat or the up of a key that was sent: consume it, do nothing.
    FollowUp,
}

/// Where the hooks keep what they remember between events.
#[derive(Debug, Default)]
struct Hooks {
    ctrl_tab: Option<CtrlTab>,
    f6: F6Memory,
    /// The ring was open at the last event (both edges reset the ⌥ machine, D23).
    ring_was_open: bool,
    /// Keys whose ⌥ press went to the ring: their repeats and ups are consumed too, even after
    /// the ring has closed (the ⌥ release that commits comes first).
    ring_keys: Vec<KeyCode>,
}

impl Hooks {
    fn ctrl_tab(&mut self) -> &mut CtrlTab {
        self.ctrl_tab.get_or_insert_with(|| CtrlTab::new(RING_HOLD))
    }

    /// D23. `ring_open` is `RingHost::is_open()` now.
    fn ring_key(
        &mut self,
        ring_open: bool,
        physical: &PhysicalKey,
        state: ElementState,
        repeat: bool,
        mods: ModifiersState,
        synthetic: bool,
    ) -> RingKey {
        if synthetic {
            return RingKey::None;
        }
        let PhysicalKey::Code(code) = physical else {
            return RingKey::None;
        };
        let remembered = self.ring_keys.contains(code);
        if !state.is_pressed() {
            if remembered {
                self.ring_keys.retain(|held| held != code);
                return RingKey::FollowUp;
            }
            return RingKey::None;
        }
        if repeat {
            return if remembered {
                RingKey::FollowUp
            } else {
                RingKey::None
            };
        }
        let alt_only = mods.alt_key() && !mods.shift_key() && !control(mods) && !command(mods);
        match ring_option_char(physical) {
            Some(letter) if ring_open && alt_only => {
                if !remembered {
                    self.ring_keys.push(*code);
                }
                RingKey::Send(letter)
            }
            _ => RingKey::None,
        }
    }

    /// The edge of "the ring is open": `true` when it changed since the last call.
    fn ring_edge(&mut self, ring_open: bool) -> bool {
        std::mem::replace(&mut self.ring_was_open, ring_open) != ring_open
    }
}

thread_local! {
    static HOOKS: RefCell<Hooks> = RefCell::new(Hooks::default());
    static CTRL_TAB_TIMER: Timer = Timer::default();
}

fn with_hooks<R>(f: impl FnOnce(&mut Hooks) -> R) -> R {
    HOOKS.with(|hooks| f(&mut hooks.borrow_mut()))
}

// ---------------------------------------------------------------------------------------------
// The ⌃⇥ hold (D18).
// ---------------------------------------------------------------------------------------------

/// One call of the ⌃⇥ machine, then its effect on the ring and the timer for its deadline. The
/// machine is borrowed only inside `call`; the ring is called after.
fn step_ctrl_tab(
    ring: Option<&dyn RingHost>,
    call: impl FnOnce(&mut CtrlTab, Duration, CtrlTabEnv) -> CtrlTabOutcome,
) -> CtrlTabOutcome {
    let env = CtrlTabEnv::from_host(ring, crate::ime::composition_active());
    let now = clock_now();
    let (outcome, deadline) = with_hooks(|hooks| {
        let machine = hooks.ctrl_tab();
        let outcome = call(machine, now, env);
        (outcome, machine.deadline())
    });
    arm_ctrl_tab_timer(deadline.map(|deadline| deadline.saturating_sub(now)));
    if let (Some(effect), Some(ring)) = (outcome.effect, ring) {
        ctrl_tab::apply(ring, effect);
    }
    outcome
}

fn arm_ctrl_tab_timer(delay: Option<Duration>) {
    CTRL_TAB_TIMER.with(|timer| match delay {
        Some(delay) => timer.start(TimerMode::SingleShot, delay, || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            let ring = ring_host();
            step_ctrl_tab(ring.as_deref(), |machine, now, env| machine.poll(now, env));
        }),
        None => timer.stop(),
    });
}

// ---------------------------------------------------------------------------------------------
// F6 (D20).
// ---------------------------------------------------------------------------------------------

/// What the hook does with one classified F6 event. `blocked`: Orbit is open, a sheet covers the
/// page or an IME composition is open, so F6 passes on unchanged.
pub(crate) fn f6_decide(
    memory: &mut F6Memory,
    event: F6Event,
    host: Option<&dyn FocusHost>,
    blocked: bool,
) -> bool {
    match event {
        F6Event::Pass => false,
        F6Event::FollowUp => true,
        F6Event::Press { backwards } => {
            let Some(host) = host else {
                return false;
            };
            if blocked || !focus_regions::run(host, backwards) {
                return false;
            }
            memory.consumed();
            true
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The calls `keyboard.rs` makes.
// ---------------------------------------------------------------------------------------------

/// What Orbit is told about the keyboard besides the key.
fn orbit_context(window: &MainWindow) -> orbit_view::KeyContext {
    orbit_view::KeyContext {
        text_input_focused: window.get_text_input_focused(),
    }
}

/// The first stage of the `KeyboardInput` arm (D7, D21): the ring's ⌥ keys, the ⌥ group-key
/// machine, Orbit's keys, the ⌃⇥ hold. `true` consumes the event (`PreventDefault`).
pub(crate) fn early(
    window: &MainWindow,
    controller: &Rc<RefCell<BrowserController>>,
    web_view_focused: bool,
    event: &KeyEvent,
    modifiers: ModifiersState,
    is_synthetic: bool,
) -> bool {
    let ring = ring_host();
    let ring_open = ring.as_ref().is_some_and(|ring| ring.is_open());
    if with_hooks(|hooks| hooks.ring_edge(ring_open)) {
        option_hook::reset(window, controller);
    }

    let ring_key = with_hooks(|hooks| {
        hooks.ring_key(
            ring_open,
            &event.physical_key,
            event.state,
            event.repeat,
            modifiers,
            is_synthetic,
        )
    });
    match (ring_key, ring.as_deref()) {
        (RingKey::Send(key), Some(ring)) => {
            ring.option_key(key);
            return true;
        }
        (RingKey::FollowUp, _) => return true,
        _ => {}
    }

    // The ring is on top while it is open: the ⌥ machine is suspended (D23).
    if !ring_open
        && option_hook::key(
            window,
            controller,
            web_view_focused,
            event,
            modifiers,
            is_synthetic,
        )
    {
        return true;
    }

    if let Some(orbit) = orbit_host()
        && orbit.is_open()
        && event.state == ElementState::Pressed
        && !is_synthetic
        && let Some(key) = option_hook::orbit_key_from_winit(&event.logical_key)
        && orbit.handle_key(
            key,
            option_hook::orbit_mods_from_winit(modifiers),
            orbit_context(window),
        )
    {
        return true;
    }

    step_ctrl_tab(ring.as_deref(), |machine, now, env| {
        machine.on_key(
            now,
            ctrl_tab::key_from_winit(event, modifiers, is_synthetic),
            env,
        )
    })
    .consume
}

/// F6 and ⇧F6, after the popups and the suggestion list had their turn (D21). `true` consumes.
pub(crate) fn f6(
    controller: &RefCell<BrowserController>,
    event: &KeyEvent,
    modifiers: ModifiersState,
    is_synthetic: bool,
) -> bool {
    let classified = with_hooks(|hooks| hooks.f6.event_from_winit(event, modifiers, is_synthetic));
    if classified == F6Event::Pass {
        return false;
    }
    let sheet = match controller.try_borrow() {
        Ok(controller) => crate::page_requests::sheet_shown(&controller),
        Err(_) => true,
    };
    let blocked = sheet
        || crate::ime::composition_active()
        || orbit_host().is_some_and(|orbit| orbit.is_open());
    let host = focus_host();
    // The memory is borrowed only to record the decision: the host is called outside it.
    let mut memory = with_hooks(|hooks| std::mem::take(&mut hooks.f6));
    let consumed = f6_decide(&mut memory, classified, host.as_deref(), blocked);
    with_hooks(|hooks| hooks.f6 = memory);
    consumed
}

/// `WindowEvent::ModifiersChanged`: the ⌥ machine (unless the ring is open, D23) and the ⌃⇥ hold
/// see the new state.
pub(crate) fn modifiers(
    window: &MainWindow,
    controller: &Rc<RefCell<BrowserController>>,
    web_view_focused: bool,
    state: ModifiersState,
) {
    let ring = ring_host();
    let ring_open = ring.as_ref().is_some_and(|ring| ring.is_open());
    if with_hooks(|hooks| hooks.ring_edge(ring_open)) {
        option_hook::reset(window, controller);
    }
    if !ring_open {
        option_hook::modifiers(window, controller, web_view_focused, state);
    }
    step_ctrl_tab(ring.as_deref(), |machine, now, env| {
        machine.on_modifiers(now, mods_from_winit(state), env)
    });
}

/// Focus lost, window closing, window hidden: forget everything transient (D4, D13, D18).
pub(crate) fn reset(window: &MainWindow, controller: &Rc<RefCell<BrowserController>>) {
    option_hook::reset(window, controller);
    let effect = with_hooks(|hooks| {
        hooks.f6.reset();
        hooks.ring_keys.clear();
        hooks.ring_was_open = false;
        hooks.ctrl_tab().reset()
    });
    arm_ctrl_tab_timer(None);
    if let (Some(effect), Some(ring)) = (effect, ring_host()) {
        ctrl_tab::apply(&*ring, effect);
    }
}

#[cfg(test)]
pub(crate) fn clear_for_tests() {
    with_hooks(|hooks| *hooks = Hooks::default());
    CTRL_TAB_TIMER.with(Timer::stop);
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use winit::keyboard::NativeKeyCode;

    use super::*;
    use crate::ctrl_tab::CtrlTabKey;
    use crate::input_hooks::Region;

    const NONE: ModifiersState = ModifiersState::empty();
    const ALT: ModifiersState = ModifiersState::ALT;

    fn code(code: KeyCode) -> PhysicalKey {
        PhysicalKey::Code(code)
    }

    fn ring_key(
        hooks: &mut Hooks,
        open: bool,
        key: KeyCode,
        state: ElementState,
        mods: ModifiersState,
    ) -> RingKey {
        hooks.ring_key(open, &code(key), state, false, mods, false)
    }

    // ---- modifier naming (D24) --------------------------------------------------------------

    #[test]
    fn winit_names_command_and_control_the_same_on_every_platform() {
        assert!(command(ModifiersState::SUPER));
        assert!(!command(ModifiersState::CONTROL));
        assert!(control(ModifiersState::CONTROL));
        assert!(!control(ModifiersState::SUPER));
        assert!(!command(ALT) && !control(ALT));
    }

    #[test]
    fn slints_swapped_flags_are_read_through_one_helper() {
        // On macOS Slint reports ⌘ as `control` and ⌃ as `meta`.
        assert!(slint_command(true, false));
        assert!(!slint_command(false, true));
        if cfg!(target_os = "macos") {
            assert!(slint_control(false, true));
            assert!(!slint_control(true, false));
        } else {
            assert!(slint_control(true, false));
        }
    }

    // ---- ring_option_char -------------------------------------------------------------------

    #[test]
    fn ring_option_char_maps_physical_letters_and_digits() {
        let letters = [
            (KeyCode::KeyA, 'A'),
            (KeyCode::KeyB, 'B'),
            (KeyCode::KeyC, 'C'),
            (KeyCode::KeyD, 'D'),
            (KeyCode::KeyE, 'E'),
            (KeyCode::KeyF, 'F'),
            (KeyCode::KeyG, 'G'),
            (KeyCode::KeyH, 'H'),
            (KeyCode::KeyI, 'I'),
            (KeyCode::KeyJ, 'J'),
            (KeyCode::KeyK, 'K'),
            (KeyCode::KeyL, 'L'),
            (KeyCode::KeyM, 'M'),
            (KeyCode::KeyN, 'N'),
            (KeyCode::KeyO, 'O'),
            (KeyCode::KeyP, 'P'),
            (KeyCode::KeyQ, 'Q'),
            (KeyCode::KeyR, 'R'),
            (KeyCode::KeyS, 'S'),
            (KeyCode::KeyT, 'T'),
            (KeyCode::KeyU, 'U'),
            (KeyCode::KeyV, 'V'),
            (KeyCode::KeyW, 'W'),
            (KeyCode::KeyX, 'X'),
            (KeyCode::KeyY, 'Y'),
            (KeyCode::KeyZ, 'Z'),
        ];
        for (key, expected) in letters {
            assert_eq!(ring_option_char(&code(key)), Some(expected), "{key:?}");
        }
        let digits = [
            (KeyCode::Digit1, KeyCode::Numpad1, '1'),
            (KeyCode::Digit2, KeyCode::Numpad2, '2'),
            (KeyCode::Digit3, KeyCode::Numpad3, '3'),
            (KeyCode::Digit4, KeyCode::Numpad4, '4'),
            (KeyCode::Digit5, KeyCode::Numpad5, '5'),
            (KeyCode::Digit6, KeyCode::Numpad6, '6'),
            (KeyCode::Digit7, KeyCode::Numpad7, '7'),
            (KeyCode::Digit8, KeyCode::Numpad8, '8'),
            (KeyCode::Digit9, KeyCode::Numpad9, '9'),
        ];
        for (row, pad, expected) in digits {
            assert_eq!(ring_option_char(&code(row)), Some(expected));
            assert_eq!(ring_option_char(&code(pad)), Some(expected));
        }
    }

    #[test]
    fn ring_option_char_ignores_everything_else() {
        for key in [
            KeyCode::Digit0,
            KeyCode::Numpad0,
            KeyCode::Escape,
            KeyCode::Tab,
            KeyCode::ArrowLeft,
            KeyCode::F6,
            KeyCode::Space,
            KeyCode::Minus,
            KeyCode::AltLeft,
        ] {
            assert_eq!(ring_option_char(&code(key)), None, "{key:?}");
        }
        assert_eq!(
            ring_option_char(&PhysicalKey::Unidentified(NativeKeyCode::MacOS(7))),
            None
        );
    }

    // ---- the ring's ⌥ keys (D23) ------------------------------------------------------------

    #[test]
    fn option_s_goes_to_the_ring_while_it_is_open_and_its_up_is_consumed() {
        let mut hooks = Hooks::default();

        let down = ring_key(&mut hooks, true, KeyCode::KeyS, ElementState::Pressed, ALT);
        assert_eq!(down, RingKey::Send('S'));
        let repeat = hooks.ring_key(
            true,
            &code(KeyCode::KeyS),
            ElementState::Pressed,
            true,
            ALT,
            false,
        );
        assert_eq!(repeat, RingKey::FollowUp);
        let up = ring_key(&mut hooks, true, KeyCode::KeyS, ElementState::Released, ALT);
        assert_eq!(up, RingKey::FollowUp);
        let again = ring_key(&mut hooks, true, KeyCode::KeyS, ElementState::Released, ALT);
        assert_eq!(again, RingKey::None, "one up per down");
    }

    #[test]
    fn the_up_of_a_ring_key_is_consumed_after_the_ring_closed() {
        let mut hooks = Hooks::default();
        assert_eq!(
            ring_key(
                &mut hooks,
                true,
                KeyCode::Digit3,
                ElementState::Pressed,
                ALT
            ),
            RingKey::Send('3')
        );

        // The ⌥ release commits and closes the ring first; the 3's up comes later.
        let up = ring_key(
            &mut hooks,
            false,
            KeyCode::Digit3,
            ElementState::Released,
            NONE,
        );

        assert_eq!(up, RingKey::FollowUp);
    }

    #[test]
    fn nothing_goes_to_the_ring_when_it_is_closed() {
        let mut hooks = Hooks::default();
        assert_eq!(
            ring_key(&mut hooks, false, KeyCode::KeyS, ElementState::Pressed, ALT),
            RingKey::None
        );
    }

    #[test]
    fn only_option_alone_reaches_the_ring() {
        let mut hooks = Hooks::default();
        for mods in [
            NONE,
            ALT | ModifiersState::SHIFT,
            ALT | ModifiersState::SUPER,
            ALT | ModifiersState::CONTROL,
            ModifiersState::SUPER,
        ] {
            assert_eq!(
                ring_key(&mut hooks, true, KeyCode::KeyS, ElementState::Pressed, mods),
                RingKey::None,
                "{mods:?}"
            );
        }
    }

    #[test]
    fn a_synthetic_or_unmapped_key_never_reaches_the_ring() {
        let mut hooks = Hooks::default();
        let synthetic = hooks.ring_key(
            true,
            &code(KeyCode::KeyS),
            ElementState::Pressed,
            false,
            ALT,
            true,
        );
        assert_eq!(synthetic, RingKey::None);
        assert_eq!(
            ring_key(
                &mut hooks,
                true,
                KeyCode::ArrowLeft,
                ElementState::Pressed,
                ALT
            ),
            RingKey::None
        );
        assert_eq!(
            ring_key(
                &mut hooks,
                true,
                KeyCode::Digit0,
                ElementState::Pressed,
                ALT
            ),
            RingKey::None
        );
    }

    #[test]
    fn the_ring_edge_is_reported_once_per_change() {
        let mut hooks = Hooks::default();
        assert!(!hooks.ring_edge(false));
        assert!(hooks.ring_edge(true));
        assert!(!hooks.ring_edge(true));
        assert!(hooks.ring_edge(false));
    }

    // ---- ⌃⇥ through the hook (D18) ----------------------------------------------------------

    #[derive(Default)]
    struct FakeRing {
        available: Cell<bool>,
        open: Cell<bool>,
        log: RefCell<Vec<String>>,
    }

    impl RingHost for FakeRing {
        fn available(&self) -> bool {
            self.available.get()
        }
        fn is_open(&self) -> bool {
            self.open.get()
        }
        fn open_held(&self) {
            self.open.set(true);
            self.log.borrow_mut().push("open".into());
        }
        fn cycle(&self, backwards: bool) {
            self.log.borrow_mut().push(format!("cycle {backwards}"));
        }
        fn commit_and_close(&self) {
            self.open.set(false);
            self.log.borrow_mut().push("commit".into());
        }
        fn close(&self) {
            self.open.set(false);
            self.log.borrow_mut().push("close".into());
        }
        fn option_key(&self, key: char) {
            self.log.borrow_mut().push(format!("option {key}"));
        }
    }

    fn tab_down(shift: bool) -> CtrlTabKey {
        CtrlTabKey {
            tab: true,
            control: true,
            shift,
            down: true,
            ..CtrlTabKey::default()
        }
    }

    fn fresh() -> std::sync::MutexGuard<'static, ()> {
        let guard = crate::option_hook::test_guard();
        clear_for_tests();
        guard
    }

    #[test]
    fn a_held_ctrl_tab_opens_the_ring_cycles_and_commits_on_release() {
        let _guard = fresh();
        let fake = FakeRing::default();
        fake.available.set(true);
        let ring: &dyn RingHost = &fake;

        // The arming press is never consumed (it is shortcuts.rs's tab switch).
        let first = step_ctrl_tab(Some(ring), |machine, now, env| {
            machine.on_key(now, tab_down(false), env)
        });
        assert!(!first.consume);
        assert!(fake.log.borrow().is_empty());
        // The hold: the timer's poll once the deadline has come.
        let deadline = with_hooks(|hooks| hooks.ctrl_tab().deadline()).expect("the hold is armed");
        step_ctrl_tab(Some(ring), |machine, _now, env| machine.poll(deadline, env));
        assert_eq!(*fake.log.borrow(), vec!["open"]);
        // A ⇧⇥ press while open cycles backwards and is consumed.
        let second = step_ctrl_tab(Some(ring), |machine, _now, env| {
            machine.on_key(deadline, tab_down(true), env)
        });
        assert!(second.consume);
        assert_eq!(*fake.log.borrow(), vec!["open", "cycle true"]);
        // ⌃ released: the focused tab is chosen.
        step_ctrl_tab(Some(ring), |machine, _now, env| {
            machine.on_modifiers(deadline, mercury_option_keys::Mods::NONE, env)
        });
        assert_eq!(*fake.log.borrow(), vec!["open", "cycle true", "commit"]);
    }

    #[test]
    fn ctrl_tab_does_nothing_without_a_ring_host() {
        let _guard = fresh();
        let outcome = step_ctrl_tab(None, |machine, now, env| {
            machine.on_key(now, tab_down(false), env)
        });
        assert_eq!(outcome, CtrlTabOutcome::PASS);
        let after = step_ctrl_tab(None, |machine, now, env| {
            machine.poll(now + Duration::from_secs(2), env)
        });
        assert_eq!(after, CtrlTabOutcome::PASS);
        assert!(with_hooks(|hooks| hooks.ctrl_tab().deadline()).is_none());
    }

    // ---- F6 (D20, D21) ----------------------------------------------------------------------

    struct FakeFocus {
        regions: Vec<Region>,
        current: Cell<Option<Region>>,
        focused: RefCell<Vec<Region>>,
    }

    impl FocusHost for FakeFocus {
        fn regions(&self) -> Vec<Region> {
            self.regions.clone()
        }
        fn current(&self) -> Option<Region> {
            self.current.get()
        }
        fn focus(&self, region: Region) {
            self.current.set(Some(region));
            self.focused.borrow_mut().push(region);
        }
    }

    fn focus_host_with(current: Option<Region>) -> FakeFocus {
        FakeFocus {
            regions: vec![Region::Address, Region::Toolbar, Region::Dock, Region::Page],
            current: Cell::new(current),
            focused: RefCell::new(Vec::new()),
        }
    }

    fn classify(memory: &mut F6Memory, state: ElementState, mods: ModifiersState) -> F6Event {
        memory.event(&code(KeyCode::F6), state, false, mods, false)
    }

    #[test]
    fn f6_moves_the_focus_and_consumes_the_press_and_its_up() {
        let mut memory = F6Memory::new();
        let host = focus_host_with(Some(Region::Toolbar));

        let press = classify(&mut memory, ElementState::Pressed, NONE);
        assert!(f6_decide(&mut memory, press, Some(&host), false));
        assert_eq!(*host.focused.borrow(), vec![Region::Dock]);

        let up = classify(&mut memory, ElementState::Released, NONE);
        assert!(f6_decide(&mut memory, up, Some(&host), false));
    }

    #[test]
    fn shift_f6_goes_backwards() {
        let mut memory = F6Memory::new();
        let host = focus_host_with(Some(Region::Toolbar));
        let press = classify(&mut memory, ElementState::Pressed, ModifiersState::SHIFT);
        assert!(f6_decide(&mut memory, press, Some(&host), false));
        assert_eq!(*host.focused.borrow(), vec![Region::Address]);
    }

    #[test]
    fn f6_passes_without_a_host_and_its_up_passes_too() {
        let mut memory = F6Memory::new();
        let press = classify(&mut memory, ElementState::Pressed, NONE);
        assert!(!f6_decide(&mut memory, press, None, false));
        let up = classify(&mut memory, ElementState::Released, NONE);
        assert!(!f6_decide(&mut memory, up, None, false));
    }

    #[test]
    fn f6_passes_while_blocked_by_orbit_a_sheet_or_a_composition() {
        let mut memory = F6Memory::new();
        let host = focus_host_with(Some(Region::Address));
        let press = classify(&mut memory, ElementState::Pressed, NONE);
        assert!(!f6_decide(&mut memory, press, Some(&host), true));
        assert!(host.focused.borrow().is_empty());
        let up = classify(&mut memory, ElementState::Released, NONE);
        assert!(!f6_decide(&mut memory, up, Some(&host), true));
    }

    #[test]
    fn f6_with_a_modifier_is_not_ours() {
        let mut memory = F6Memory::new();
        let host = focus_host_with(None);
        for mods in [ModifiersState::SUPER, ModifiersState::CONTROL, ALT] {
            let event = classify(&mut memory, ElementState::Pressed, mods);
            assert!(
                !f6_decide(&mut memory, event, Some(&host), false),
                "{mods:?}"
            );
        }
        assert!(host.focused.borrow().is_empty());
    }

    #[test]
    fn a_host_with_no_regions_leaves_f6_to_the_page() {
        let mut memory = F6Memory::new();
        let host = FakeFocus {
            regions: Vec::new(),
            current: Cell::new(None),
            focused: RefCell::new(Vec::new()),
        };
        let press = classify(&mut memory, ElementState::Pressed, NONE);
        assert!(!f6_decide(&mut memory, press, Some(&host), false));
    }

    // ---- the order of the stages in keyboard.rs (D21) ---------------------------------------

    /// The production part of `keyboard.rs`: the hooks must run in the order D21 gives. A later
    /// edit that moves F6 before a popup's keys, or the ⌥ hook after the downloads list, fails
    /// here.
    #[test]
    fn the_stages_run_in_the_order_of_d21() {
        let source = include_str!("keyboard.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("the file has a production part");
        let at = |needle: &str| {
            production
                .find(needle)
                .unwrap_or_else(|| panic!("keyboard.rs no longer contains `{needle}`"))
        };
        let early = at("hook_chain::early(");
        let downloads = at("downloads::handle_key(");
        let controls = at("controls::handle_key(");
        let history = at("history_ui::handle_key(");
        let f6 = at("hook_chain::f6(");
        let shortcuts = at("handle_browser_shortcut(");
        assert!(
            early < downloads,
            "the option hook sees every key first (D7)"
        );
        assert!(downloads < controls && controls < history);
        assert!(
            history < f6,
            "F6 must not take a key from a popup or the suggestions"
        );
        assert!(f6 < shortcuts);
    }
}
