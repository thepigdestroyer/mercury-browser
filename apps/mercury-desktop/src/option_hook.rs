// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The ⌥ hook: what `keyboard.rs` calls so that the `mercury-option-keys` machine gets every key
//! and modifier change, the focus, the groups and the settings, and its answer decides
//! `PreventDefault`, a switch of group or tab, and the "Switch group" card (lane input-hooks,
//! brief `lane-input-hooks-brief.md` D1, D2, D4, D5, D9 to D11, D15, D16).
//!
//! # API for the winit closure (item 3)
//!
//! All of it runs on the main thread, inside the winit event hook.
//!
//! | Call | When | Answer |
//! | --- | --- | --- |
//! | [`key`] | every `KeyboardInput`, first in the arm (D7) | `true`: `PreventDefault` |
//! | [`modifiers`] | every `ModifiersChanged` | nothing (modifier changes are never consumed) |
//! | [`reset`] | `Focused(false)`, `CloseRequested`, `Occluded(true)` (D4) | nothing |
//! | [`card_event`] | what the card's Slint callbacks do (`input_hooks::card_*`) | nothing |
//! | [`set_settings`], [`settings`] | `input_hooks::follow_settings` | the current [`HookSettings`] |
//!
//! A tab change that the hook did not cause also resets the machine (D4): every call above
//! compares the active tab with the one it remembers.
//!
//! # How a call runs (D1)
//!
//! One function, `run_at`, does every machine call: it applies the hold deadline (`poll(now)`), then
//! lets the machine follow the active tab, the configuration (D5), the groups and the focus (D2),
//! then runs the call itself and applies its answer to the host (the card, a switch of tab, a
//! redraw) and re-arms the hold timer (D15). The host is never called from inside
//! `option_keys::with_machine`, so a host that calls back into the hook gets a defined answer.

#![allow(dead_code)] // wired by the input-hooks `wire` item

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use mercury_option_keys::{
    CardChange, CardView, Config, Event, Focus, KeyedGroup, LetterKey, OptionKeys, Outcome, Phase,
    Target,
};
use mercury_settings::Settings;
use mercury_ui::{MainWindow, orbit_view};
use slint::winit_030::winit;
use slint::{ComponentHandle, Timer, TimerMode};
use talaria_core::{BrowserController, GroupId, TabId};
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::option_keys::{
    clock_now, event_from_key_event, mods_from_winit, poll_delay, with_machine,
};

// ---------------------------------------------------------------------------------------------
// Settings.
// ---------------------------------------------------------------------------------------------

/// What the hook follows in the settings (`appearance.tab_groups`), cached so that no key event
/// clones the settings (D8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HookSettings {
    /// The machine's configuration as the settings give it. The hook turns `enabled` off while
    /// no `CardHost` is registered (D5).
    pub(crate) config: Config,
    /// `tab_groups.orbit_pinch`: a pinch that closes on an unzoomed page opens Orbit.
    pub(crate) orbit_pinch: bool,
    /// `tab_groups.orbit`.
    pub(crate) orbit: bool,
}

impl Default for HookSettings {
    /// The defaults of `Settings::default()`.
    fn default() -> Self {
        Self {
            config: Config::default(),
            orbit_pinch: true,
            orbit: true,
        }
    }
}

impl HookSettings {
    pub(crate) fn from_settings(settings: &Settings) -> Self {
        let groups = &settings.appearance.tab_groups;
        Self {
            config: Config::from(&groups.keys),
            orbit_pinch: groups.orbit_pinch,
            orbit: groups.orbit,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The state of the hook, and the host it acts on.
// ---------------------------------------------------------------------------------------------

/// What the hook remembers between calls. Nothing here is the machine's: it is what the hook has
/// told the machine, so that it tells it again only when it changed.
#[derive(Default)]
struct HookState {
    settings: HookSettings,
    /// The configuration last sent to the machine.
    sent_config: Option<Config>,
    sent_focus: Option<Focus>,
    sent_groups: Option<Vec<KeyedGroup>>,
    /// The active tab as of the last call or the last switch the hook made itself (D4).
    active_tab: Option<TabId>,
    /// The machine deadline the hold timer is armed for.
    armed_deadline: Option<Duration>,
}

thread_local! {
    static STATE: RefCell<HookState> = RefCell::new(HookState::default());
    static HOLD_TIMER: Timer = Timer::default();
}

/// Short borrows only: nothing may call a host while holding the state.
fn with_state<R>(f: impl FnOnce(&mut HookState) -> R) -> R {
    STATE.with(|state| f(&mut state.borrow_mut()))
}

/// The settings the hook follows now.
pub(crate) fn settings() -> HookSettings {
    with_state(|state| state.settings)
}

/// What the hook acts on. Private so that tests use a fake; the real ones are [`LiveHost`] (a
/// window and a controller) and [`DetachedHost`] (the hold timer, settings changes).
trait Host {
    /// A `CardHost` is registered, so a card can be shown (D5).
    fn card_available(&self) -> bool;
    /// The card appeared, moved or went away. Never called inside `with_machine`.
    fn card_changed(&mut self, change: CardChange, view: CardView);
    /// Switch there (D10). The active tab afterwards, or `None` when the switch could not be made.
    fn activate(&mut self, target: Target) -> Option<TabId>;
    fn redraw(&mut self);
    /// Fire the hold timer after `delay`, or stop it.
    fn arm_timer(&mut self, delay: Option<Duration>);
}

/// What the hook reads from the window and the controller at one event. `None` leaves that part
/// of the machine's knowledge as it is.
#[derive(Default)]
struct World {
    focus: Option<Focus>,
    /// `None`: the controller was busy.
    active_tab: Option<TabId>,
    groups: Option<Vec<KeyedGroup>>,
}

// ---------------------------------------------------------------------------------------------
// The core: every machine call goes through here (D1).
// ---------------------------------------------------------------------------------------------

/// The configuration the machine gets: the settings', but off while no card can be shown (D5).
fn effective_config(settings: &HookSettings, card_available: bool) -> Config {
    Config {
        enabled: settings.config.enabled && card_available,
        ..settings.config
    }
}

/// Runs `f` on the machine at `now`, after everything the machine must know first, and applies
/// the answer. See the module documentation.
fn run_at(
    host: &mut dyn Host,
    now: Duration,
    world: &World,
    f: impl FnOnce(&mut OptionKeys, Duration) -> Outcome,
) -> Outcome {
    // The deadline first: `set_*` do not apply it, so a hold that is due must show its card
    // before anything below can end it.
    call(host, |machine| machine.poll(now));

    if let Some(tab) = world.active_tab {
        let changed = with_state(|state| {
            state
                .active_tab
                .replace(tab)
                .is_some_and(|before| before != tab)
        });
        if changed {
            call(host, |machine| machine.on_event(now, Event::Reset));
        }
    }

    let available = host.card_available();
    let wanted = with_state(|state| effective_config(&state.settings, available));
    if with_state(|state| state.sent_config.replace(wanted)) != Some(wanted) {
        call(host, |machine| machine.set_config(wanted));
    }

    if let Some(groups) = &world.groups {
        let changed = with_state(|state| {
            let changed = state.sent_groups.as_ref() != Some(groups);
            if changed {
                state.sent_groups = Some(groups.clone());
            }
            changed
        });
        if changed {
            call(host, |machine| machine.set_groups(groups));
        }
    }

    if let Some(focus) = world.focus
        && with_state(|state| state.sent_focus.replace(focus)) != Some(focus)
    {
        call(host, |machine| machine.set_focus(focus));
    }

    let outcome = call(host, |machine| f(machine, now));

    let deadline = with_machine(|machine| machine.deadline());
    if with_state(|state| std::mem::replace(&mut state.armed_deadline, deadline)) != deadline {
        host.arm_timer(next_timer_delay(deadline, now));
    }
    outcome
}

/// One machine call, then its answer applied to the host.
fn call(host: &mut dyn Host, f: impl FnOnce(&mut OptionKeys) -> Outcome) -> Outcome {
    let (outcome, view) = with_machine(|machine| {
        let outcome = f(machine);
        (outcome, machine.view())
    });
    apply(host, outcome, view);
    outcome
}

/// Applies what the machine decided: the switch, the card, one redraw.
fn apply(host: &mut dyn Host, outcome: Outcome, view: CardView) {
    let mut acted = false;
    if let Some(target) = outcome.activate {
        if let Some(tab) = host.activate(target) {
            // The hook's own switch is not a change to reset on (D4).
            with_state(|state| state.active_tab = Some(tab));
        }
        acted = true;
    }
    if outcome.card != CardChange::None {
        host.card_changed(outcome.card, view);
        acted = true;
    }
    if acted {
        host.redraw();
    }
}

/// One event for the machine; `true` when it is consumed.
fn dispatch(host: &mut dyn Host, now: Duration, world: &World, event: Event) -> bool {
    run_at(host, now, world, |machine, now| {
        machine.on_event(now, event)
    })
    .consume
}

fn key_with(host: &mut dyn Host, now: Duration, world: &World, event: Option<Event>) -> bool {
    match event {
        Some(event) => dispatch(host, now, world, event),
        None => false,
    }
}

fn modifiers_with(host: &mut dyn Host, now: Duration, world: &World, state: ModifiersState) {
    dispatch(host, now, world, Event::Modifiers(mods_from_winit(state)));
}

fn reset_with(host: &mut dyn Host, now: Duration, world: &World) {
    dispatch(host, now, world, Event::Reset);
}

/// The hold timer fired (or a settings change): apply the deadline.
fn tick_with(host: &mut dyn Host, now: Duration) {
    with_state(|state| state.armed_deadline = None);
    run_at(host, now, &World::default(), |machine, now| {
        machine.poll(now)
    });
}

fn set_settings_with(host: &mut dyn Host, now: Duration, settings: HookSettings) {
    with_state(|state| state.settings = settings);
    run_at(host, now, &World::default(), |machine, now| {
        machine.poll(now)
    });
}

// ---------------------------------------------------------------------------------------------
// Plain functions of plain values.
// ---------------------------------------------------------------------------------------------

/// Whether the machine must treat the focus as a text field, and whether an IME composition is
/// open (D2). Every state in which the page does not have the keyboard counts as a text field:
/// a wrong "text field" costs a 400 ms hold, a wrong "no text field" eats å.
pub(crate) fn focus_inputs(
    text_input_focused: bool,
    web_view_focused: bool,
    has_input_method: bool,
    composing: bool,
) -> Focus {
    Focus {
        text_field: text_input_focused || !web_view_focused || has_input_method,
        composing,
    }
}

/// The groups that have a key, in ring order (the machine keeps the first of two groups with
/// one letter).
pub(crate) fn keyed_groups(controller: &BrowserController) -> Vec<KeyedGroup> {
    controller
        .groups()
        .iter()
        .filter_map(|group| {
            let key = LetterKey::from_code_name(group.key?.code_name())?;
            Some(KeyedGroup {
                id: group.id.value(),
                key,
                tab_count: controller.group_tabs(group.id).len(),
            })
        })
        .collect()
}

/// How a switch the machine asked for ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    /// The group's most recently shown tab is up.
    Group,
    /// The chosen tab of the group is up.
    Tab,
    /// The tab is gone (it closed while the card was open): the group's most recent tab is up.
    GroupInsteadOfTab,
    /// Nothing happened: the group is unknown.
    Nothing,
}

/// D10: switches to `target`. The caller then runs `sync_ui` and `request_redraw`.
pub(crate) fn activate(controller: &mut BrowserController, target: Target) -> Applied {
    let (group, index) = match target {
        Target::Group(group) => (GroupId::new(group), None),
        Target::Tab { group, index } => (GroupId::new(group), Some(index)),
    };
    let chosen = index.and_then(|index| controller.group_tabs(group).get(index).copied());
    match (index, chosen) {
        (Some(_), Some(tab)) if controller.select_tab(tab) => Applied::Tab,
        (Some(_), Some(_)) => Applied::Nothing,
        (Some(_), None) if controller.select_group(group) => Applied::GroupInsteadOfTab,
        (None, _) if controller.select_group(group) => Applied::Group,
        _ => Applied::Nothing,
    }
}

/// How long until the hold timer must fire (D15): `None` without a deadline, zero when it has
/// passed.
pub(crate) fn next_timer_delay(deadline: Option<Duration>, now: Duration) -> Option<Duration> {
    poll_delay(deadline, now)
}

/// Orbit's key for a winit logical key (D11): `Escape`, `Enter`, the arrows, `n` and `N` as `N`,
/// `/` as `Slash`, every other single printable character as `Char`, anything else `None`.
/// A control character is not printable: it never starts a search.
pub(crate) fn orbit_key_from_winit(key: &Key) -> Option<orbit_view::Key> {
    match key {
        Key::Named(NamedKey::Escape) => Some(orbit_view::Key::Escape),
        Key::Named(NamedKey::Enter) => Some(orbit_view::Key::Enter),
        Key::Named(NamedKey::ArrowUp) => Some(orbit_view::Key::Up),
        Key::Named(NamedKey::ArrowDown) => Some(orbit_view::Key::Down),
        Key::Named(NamedKey::ArrowLeft) => Some(orbit_view::Key::Left),
        Key::Named(NamedKey::ArrowRight) => Some(orbit_view::Key::Right),
        Key::Character(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some('n' | 'N'), None) => Some(orbit_view::Key::N),
                (Some('/'), None) => Some(orbit_view::Key::Slash),
                (Some(c), None) if !c.is_control() => Some(orbit_view::Key::Char(c)),
                _ => None,
            }
        }
        _ => None,
    }
}

pub(crate) fn orbit_mods_from_winit(state: ModifiersState) -> orbit_view::Mods {
    orbit_view::Mods {
        command: state.super_key(),
        shift: state.shift_key(),
        alt: state.alt_key(),
        control: state.control_key(),
    }
}

/// What the hook reads from a controller. `inputs` is `(text_input_focused, web_view_focused)`
/// when the caller knows the focus; the groups are read only when asked (they cost a pass over the
/// tabs).
fn world_from(
    controller: &BrowserController,
    inputs: Option<(bool, bool)>,
    want_groups: bool,
) -> World {
    let has_input_method = controller.active_state().input_method().is_some();
    World {
        focus: inputs.map(|(text_input_focused, web_view_focused)| {
            focus_inputs(
                text_input_focused,
                web_view_focused,
                has_input_method,
                crate::ime::composition_active(),
            )
        }),
        active_tab: Some(controller.active_tab_id()),
        groups: want_groups.then(|| keyed_groups(controller)),
    }
}

// ---------------------------------------------------------------------------------------------
// The real hosts and the calls the winit closure makes.
// ---------------------------------------------------------------------------------------------

fn card_available() -> bool {
    crate::input_hooks::card_host().is_some()
}

fn notify_card(change: CardChange, view: CardView) {
    if let Some(host) = crate::input_hooks::card_host() {
        host.card_changed(change, view);
    }
}

/// [`activate`] on a controller that may be borrowed: `None` when it is.
fn activate_in(
    controller: &RefCell<BrowserController>,
    target: Target,
) -> Option<(Applied, TabId)> {
    match controller.try_borrow_mut() {
        Ok(mut controller) => {
            let applied = activate(&mut controller, target);
            Some((applied, controller.active_tab_id()))
        }
        Err(_) => {
            log::warn!("option hook: the controller is busy, the group switch was not made");
            None
        }
    }
}

/// The hold timer: one `slint::Timer`, single shot, restarted for the machine's deadline.
fn arm_hold_timer(delay: Option<Duration>) {
    HOLD_TIMER.with(|timer| match delay {
        Some(delay) => timer.start(TimerMode::SingleShot, delay, move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            hold_timer_fired();
        }),
        None => timer.stop(),
    });
}

fn hold_timer_fired() {
    tick_with(&mut DetachedHost, clock_now());
}

/// A window and its controller: the host of keys and card clicks.
struct LiveHost<'a> {
    window: &'a MainWindow,
    controller: &'a Rc<RefCell<BrowserController>>,
}

impl Host for LiveHost<'_> {
    fn card_available(&self) -> bool {
        card_available()
    }

    fn card_changed(&mut self, change: CardChange, view: CardView) {
        notify_card(change, view);
    }

    fn activate(&mut self, target: Target) -> Option<TabId> {
        let (applied, tab) = activate_in(self.controller, target)?;
        if applied != Applied::Nothing {
            match self.controller.try_borrow() {
                Ok(controller) => crate::ui_sync::sync_ui(self.window, &controller),
                Err(_) => log::warn!("option hook: the controller is busy, the UI was not synced"),
            }
            self.window.window().request_redraw();
        }
        Some(tab)
    }

    fn redraw(&mut self) {
        self.window.window().request_redraw();
    }

    fn arm_timer(&mut self, delay: Option<Duration>) {
        arm_hold_timer(delay);
    }
}

/// No window: what the hold timer and a settings change act with. Neither switches tabs.
struct DetachedHost;

impl Host for DetachedHost {
    fn card_available(&self) -> bool {
        card_available()
    }

    fn card_changed(&mut self, change: CardChange, view: CardView) {
        notify_card(change, view);
    }

    fn activate(&mut self, target: Target) -> Option<TabId> {
        log::warn!("option hook: a switch ({target:?}) came without a window and was dropped");
        None
    }

    fn redraw(&mut self) {}

    fn arm_timer(&mut self, delay: Option<Duration>) {
        arm_hold_timer(delay);
    }
}

/// [`world_from`] for the shared controller. The groups are read while ⌥ is involved (it is down
/// or went up in this event) or the machine is not idle: that is when it uses them. A busy
/// controller gives no tab and no groups, and a focus that counts as a text field (a wrong "no"
/// would eat å).
fn read_world(
    controller: &RefCell<BrowserController>,
    inputs: Option<(bool, bool)>,
    alt_involved: bool,
) -> World {
    let want_groups = alt_involved || with_machine(|machine| machine.phase()) != Phase::Idle;
    match controller.try_borrow() {
        Ok(controller) => world_from(&controller, inputs, want_groups),
        Err(_) => {
            log::warn!("option hook: the controller is busy, reading the focus as a text field");
            World {
                focus: inputs.map(|(text_input_focused, web_view_focused)| {
                    focus_inputs(
                        text_input_focused,
                        web_view_focused,
                        true,
                        crate::ime::composition_active(),
                    )
                }),
                ..World::default()
            }
        }
    }
}

/// A key event. `true` consumes it (`PreventDefault`): it is neither forwarded to the page nor to
/// Slint. A synthetic event (winit replaying held keys at focus gain) is never consumed.
pub(crate) fn key(
    window: &MainWindow,
    controller: &Rc<RefCell<BrowserController>>,
    web_view_focused: bool,
    event: &KeyEvent,
    modifiers: ModifiersState,
    is_synthetic: bool,
) -> bool {
    let Some(event) = event_from_key_event(event, modifiers, is_synthetic) else {
        return false;
    };
    let world = read_world(
        controller,
        Some((window.get_text_input_focused(), web_view_focused)),
        modifiers.alt_key(),
    );
    dispatch(
        &mut LiveHost { window, controller },
        clock_now(),
        &world,
        event,
    )
}

/// `WindowEvent::ModifiersChanged`.
pub(crate) fn modifiers(
    window: &MainWindow,
    controller: &Rc<RefCell<BrowserController>>,
    web_view_focused: bool,
    state: ModifiersState,
) {
    let world = read_world(
        controller,
        Some((window.get_text_input_focused(), web_view_focused)),
        state.alt_key(),
    );
    modifiers_with(
        &mut LiveHost { window, controller },
        clock_now(),
        &world,
        state,
    );
}

/// Focus lost, window closing, window hidden: forget everything transient (D4).
pub(crate) fn reset(window: &MainWindow, controller: &Rc<RefCell<BrowserController>>) {
    let world = read_world(controller, None, false);
    reset_with(&mut LiveHost { window, controller }, clock_now(), &world);
}

/// An event from the card (a click, "Change keys…", the backdrop). The card callbacks of
/// `input_hooks` call this; they have no key or focus to report.
pub(crate) fn card_event(
    window: &MainWindow,
    controller: &Rc<RefCell<BrowserController>>,
    event: Event,
) {
    let world = read_world(controller, None, true);
    dispatch(
        &mut LiveHost { window, controller },
        clock_now(),
        &world,
        event,
    );
}

/// Follows a changed setting: the machine gets the new configuration through `run` (D1).
pub(crate) fn set_settings(settings: HookSettings) {
    set_settings_with(&mut DetachedHost, clock_now(), settings);
}

/// Every test that reaches the machine, the hook state, the hosts or `ime::composition_active`
/// holds this guard (the pattern of `ime::test_guard`); it starts the test from a clean state.
#[cfg(test)]
pub(crate) fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    let guard = crate::ime::test_guard();
    with_machine(|machine| *machine = OptionKeys::new(Config::default()));
    with_state(|state| *state = HookState::default());
    crate::input_hooks::clear_hosts();
    HOLD_TIMER.with(Timer::stop);
    guard
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::SystemTime;

    use talaria_core::{GroupKey, TabActivation};
    use winit::event::ElementState;
    use winit::keyboard::{KeyCode, NativeKey, PhysicalKey};

    use super::*;
    use crate::option_keys::event_from_parts;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    // ---- the fake host --------------------------------------------------------------------

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Call {
        Card(CardChange),
        Activate(Target),
        Timer(Option<Duration>),
    }

    /// A window-less host over a real `BrowserController` with two keyed groups: `Home` (key A,
    /// the tab `a`) and `Work` (key S; strip order `d`, `b`, `c`).
    struct TestHost {
        controller: BrowserController,
        tabs: Vec<TabId>,
        work: GroupId,
        home: GroupId,
        card: bool,
        text_input: bool,
        web_view: bool,
        calls: Vec<Call>,
        redraws: usize,
        /// Sent to the machine from inside the next `card_changed`: a host that calls back in.
        nested: Option<Event>,
        now: Duration,
    }

    impl TestHost {
        fn new() -> Self {
            let mut controller = BrowserController::default();
            let a = controller.active_tab_id();
            let mut open = || {
                controller
                    .open_tab_with(None, TabActivation::Background)
                    .expect("a blank tab opens")
            };
            let (b, c, d) = (open(), open(), open());
            let work = controller
                .create_group(&[b, c, d], Some("Work"), None, SystemTime::now())
                .expect("the group is made");
            controller
                .move_tab_to_group(d, Some(work), 0)
                .expect("d moves to the front of the group");
            let home = controller
                .create_group(&[a], Some("Home"), None, SystemTime::now())
                .expect("the group is made");
            for group in [work, home] {
                controller
                    .set_group_key(group, None)
                    .expect("a key is cleared");
            }
            controller
                .set_group_key(work, GroupKey::from_letter('S'))
                .expect("S is free");
            controller
                .set_group_key(home, GroupKey::from_letter('A'))
                .expect("A is free");
            assert_eq!(controller.group_tabs(work), vec![d, b, c]);
            assert_eq!(controller.active_tab_id(), a);
            Self {
                controller,
                tabs: vec![a, b, c, d],
                work,
                home,
                card: true,
                text_input: false,
                web_view: true,
                calls: Vec::new(),
                redraws: 0,
                nested: None,
                now: Duration::ZERO,
            }
        }

        fn world(&self) -> World {
            world_from(
                &self.controller,
                Some((self.text_input, self.web_view)),
                true,
            )
        }

        fn cards(&self) -> Vec<CardChange> {
            self.calls
                .iter()
                .filter_map(|call| match call {
                    Call::Card(change) => Some(*change),
                    _ => None,
                })
                .collect()
        }

        fn activations(&self) -> Vec<Target> {
            self.calls
                .iter()
                .filter_map(|call| match call {
                    Call::Activate(target) => Some(*target),
                    _ => None,
                })
                .collect()
        }

        fn timers(&self) -> Vec<Option<Duration>> {
            self.calls
                .iter()
                .filter_map(|call| match call {
                    Call::Timer(delay) => Some(*delay),
                    _ => None,
                })
                .collect()
        }

        fn active(&self) -> TabId {
            self.controller.active_tab_id()
        }

        fn modifiers(&mut self, at: u64, state: ModifiersState) {
            self.now = ms(at);
            let world = self.world();
            modifiers_with(self, ms(at), &world, state);
        }

        fn key_full(
            &mut self,
            at: u64,
            code: KeyCode,
            state: ElementState,
            mods: ModifiersState,
            repeat: bool,
            synthetic: bool,
        ) -> bool {
            self.now = ms(at);
            let world = self.world();
            let event = event_from_parts(&PhysicalKey::Code(code), state, repeat, mods, synthetic);
            key_with(self, ms(at), &world, event)
        }

        fn press(&mut self, at: u64, code: KeyCode, mods: ModifiersState) -> bool {
            self.key_full(at, code, ElementState::Pressed, mods, false, false)
        }

        fn release(&mut self, at: u64, code: KeyCode, mods: ModifiersState) -> bool {
            self.key_full(at, code, ElementState::Released, mods, false, false)
        }

        /// ⌥ goes down the way winit reports it: the modifier state, then the key.
        fn alt_down(&mut self, at: u64) {
            self.modifiers(at, ModifiersState::ALT);
            self.press(at, KeyCode::AltLeft, ModifiersState::ALT);
        }

        fn alt_up(&mut self, at: u64) {
            self.release(at, KeyCode::AltLeft, ModifiersState::empty());
            self.modifiers(at, ModifiersState::empty());
        }

        fn tick(&mut self, at: u64) {
            self.now = ms(at);
            tick_with(self, ms(at));
        }

        fn reset(&mut self, at: u64) {
            self.now = ms(at);
            let world = self.world();
            reset_with(self, ms(at), &world);
        }

        fn event(&mut self, at: u64, event: Event) -> bool {
            self.now = ms(at);
            let world = self.world();
            dispatch(self, ms(at), &world, event)
        }

        fn phase(&self) -> Phase {
            with_machine(|machine| machine.phase())
        }
    }

    impl Host for TestHost {
        fn card_available(&self) -> bool {
            self.card
        }

        fn card_changed(&mut self, change: CardChange, _view: CardView) {
            self.calls.push(Call::Card(change));
            if let Some(event) = self.nested.take() {
                let world = World::default();
                let now = self.now;
                dispatch(self, now, &world, event);
            }
        }

        fn activate(&mut self, target: Target) -> Option<TabId> {
            self.calls.push(Call::Activate(target));
            activate(&mut self.controller, target);
            Some(self.controller.active_tab_id())
        }

        fn redraw(&mut self) {
            self.redraws += 1;
        }

        fn arm_timer(&mut self, delay: Option<Duration>) {
            self.calls.push(Call::Timer(delay));
        }
    }

    const ALT: ModifiersState = ModifiersState::ALT;
    const NONE: ModifiersState = ModifiersState::empty();

    // ---- the five behaviours of SPEC lane 11, through the hook ----------------------------

    #[test]
    fn a_quick_option_s_in_a_text_field_reaches_the_page_and_no_card_follows() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.text_input = true;

        host.alt_down(0);
        assert!(!host.press(80, KeyCode::KeyS, ALT), "⌥S types ß in a field");
        assert!(!host.release(120, KeyCode::KeyS, ALT));
        host.alt_up(150);
        host.tick(1_000);

        assert!(host.cards().is_empty(), "{:?}", host.calls);
        assert!(host.activations().is_empty());
        assert_eq!(host.phase(), Phase::Idle);
    }

    #[test]
    fn a_400_ms_hold_shows_the_card_once() {
        let _guard = test_guard();
        let mut host = TestHost::new();

        host.alt_down(1_000);
        host.tick(1_399);
        assert!(host.cards().is_empty(), "399 ms is too early");
        host.tick(1_400);
        assert_eq!(host.cards(), vec![CardChange::Show]);
        host.tick(2_000);
        assert_eq!(
            host.cards(),
            vec![CardChange::Show],
            "once, not once per tick"
        );

        host.alt_up(2_500);
        assert_eq!(host.cards(), vec![CardChange::Show, CardChange::Hide]);
        assert!(host.activations().is_empty(), "no choice, no switch");
    }

    #[test]
    fn option_s_then_3_then_option_up_lands_on_the_third_tab_in_strip_order() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        let third_in_strip_order = host.tabs[2];
        assert_ne!(
            host.controller.group_tabs(host.work)[2],
            host.tabs[3],
            "the strip order must differ from the creation order for this test to mean anything"
        );

        host.alt_down(0);
        assert!(
            host.press(80, KeyCode::KeyS, ALT),
            "S is the group's: consumed"
        );
        assert!(
            host.release(120, KeyCode::KeyS, ALT),
            "and so is its key-up"
        );
        assert!(host.press(200, KeyCode::Digit3, ALT));
        assert!(host.release(240, KeyCode::Digit3, ALT));
        host.alt_up(400);

        let work = host.work.value();
        assert_eq!(
            host.activations(),
            vec![
                Target::Group(work),
                Target::Tab {
                    group: work,
                    index: 2
                }
            ]
        );
        assert_eq!(host.active(), third_in_strip_order);
        assert_eq!(
            host.controller.group_tabs(host.work)[2],
            host.active(),
            "the third tab of the group in strip order"
        );
        assert!(host.cards().is_empty());
        assert!(host.redraws > 0);
    }

    #[test]
    fn command_option_s_passes_through() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        let both = ModifiersState::ALT | ModifiersState::SUPER;

        host.modifiers(0, both);
        host.press(1, KeyCode::SuperLeft, both);
        assert!(
            !host.press(80, KeyCode::KeyS, both),
            "⌘⌥S is not a group key"
        );
        assert!(!host.release(120, KeyCode::KeyS, both));
        host.tick(1_000);

        assert!(host.activations().is_empty());
        assert!(host.cards().is_empty());
    }

    #[test]
    fn composing_blocks_capture() {
        let _guard = test_guard();
        let scratch = RefCell::new(BrowserController::default());
        crate::ime::handle(
            &scratch,
            crate::ime::Input::Preedit {
                text: "か".to_owned(),
                cursor: None,
            },
        );
        assert!(crate::ime::composition_active());
        let mut host = TestHost::new();

        host.alt_down(0);
        assert!(
            !host.press(80, KeyCode::KeyS, ALT),
            "the input method owns the key"
        );
        host.tick(1_000);

        assert!(host.activations().is_empty());
        assert!(host.cards().is_empty(), "no card over a composition");
    }

    // ---- the hold timer (D15) -------------------------------------------------------------

    #[test]
    fn a_held_option_arms_the_timer_for_the_hold_and_an_early_tick_arms_it_again() {
        let _guard = test_guard();
        let mut host = TestHost::new();

        host.modifiers(1_000, ALT);
        assert_eq!(host.timers(), vec![Some(ms(400))]);

        host.tick(1_399);
        assert_eq!(
            host.timers(),
            vec![Some(ms(400)), Some(ms(1))],
            "a timer that fired a millisecond early waits for the rest"
        );
        assert!(host.cards().is_empty());
    }

    #[test]
    fn a_tick_that_opens_the_card_arms_nothing_more() {
        let _guard = test_guard();
        let mut host = TestHost::new();

        host.modifiers(1_000, ALT);
        host.tick(1_400);

        assert_eq!(host.cards(), vec![CardChange::Show]);
        assert_eq!(
            host.timers(),
            vec![Some(ms(400))],
            "the single shot has fired and nothing is left to wait for"
        );
    }

    #[test]
    fn a_card_opened_by_a_key_event_stops_the_pending_timer() {
        let _guard = test_guard();
        let mut host = TestHost::new();

        host.modifiers(1_000, ALT);
        // the timer is late: the next event applies the deadline itself
        host.press(1_500, KeyCode::AltLeft, ALT);

        assert_eq!(host.cards(), vec![CardChange::Show]);
        assert_eq!(host.timers(), vec![Some(ms(400)), None]);
    }

    #[test]
    fn the_timer_is_not_restarted_while_its_deadline_has_not_changed() {
        let _guard = test_guard();
        let mut host = TestHost::new();

        host.modifiers(1_000, ALT);
        host.press(1_050, KeyCode::AltLeft, ALT);
        host.modifiers(1_100, ALT);

        assert_eq!(host.timers(), vec![Some(ms(400))]);
    }

    #[test]
    fn a_key_that_ends_the_arming_stops_the_timer() {
        let _guard = test_guard();
        let mut host = TestHost::new();

        host.modifiers(0, ALT);
        host.press(100, KeyCode::KeyX, ALT);

        assert_eq!(host.timers(), vec![Some(ms(400)), None]);
    }

    // ---- Reset (D4) -----------------------------------------------------------------------

    #[test]
    fn reset_closes_the_card_and_forgets_the_hold() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);
        host.tick(400);
        assert_eq!(host.cards(), vec![CardChange::Show]);

        host.reset(500);

        assert_eq!(host.cards(), vec![CardChange::Show, CardChange::Hide]);
        assert_eq!(host.phase(), Phase::Idle);
        assert!(host.activations().is_empty());
    }

    #[test]
    fn reset_with_nothing_going_on_calls_no_host() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.reset(10);
        assert!(host.cards().is_empty());
        assert!(host.activations().is_empty());
    }

    #[test]
    fn reset_forgets_which_keys_were_consumed() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);
        assert!(host.press(80, KeyCode::KeyS, ALT));

        host.reset(100);

        assert!(
            !host.release(120, KeyCode::KeyS, ALT),
            "the machine forgot the down, so the up is the page's"
        );
    }

    #[test]
    fn a_tab_change_the_hook_did_not_cause_resets() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);
        assert!(host.press(80, KeyCode::KeyS, ALT));
        let outside = host.tabs[1];
        assert!(host.controller.select_tab(outside), "a click on a tab");

        // Without the reset the machine would still be in Switched and take the 2.
        assert!(
            !host.press(200, KeyCode::Digit2, ALT),
            "the tab changed under a held ⌥: the hold starts over"
        );
        assert_eq!(host.activations().len(), 1, "only the first switch");
    }

    #[test]
    fn a_tab_change_the_hook_caused_does_not_reset() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);
        assert!(host.press(80, KeyCode::KeyS, ALT));
        assert_ne!(host.active(), host.tabs[0], "the switch changed the tab");

        assert!(
            host.press(200, KeyCode::Digit2, ALT),
            "digits still pick tabs"
        );
        assert!(host.press(300, KeyCode::Digit1, ALT));

        let work = host.work.value();
        assert_eq!(
            host.activations(),
            vec![
                Target::Group(work),
                Target::Tab {
                    group: work,
                    index: 1
                },
                Target::Tab {
                    group: work,
                    index: 0
                }
            ]
        );
    }

    #[test]
    fn the_first_event_only_remembers_the_active_tab() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);
        host.tick(400);
        // the first event that carries a tab id must not reset a card that is already up
        assert_eq!(host.cards(), vec![CardChange::Show]);
        host.modifiers(450, ALT);
        assert_eq!(host.cards(), vec![CardChange::Show]);
    }

    // ---- poll before set_* (D1) -----------------------------------------------------------

    #[test]
    fn the_hold_that_is_due_shows_the_card_before_a_composition_hides_it() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);
        assert!(host.cards().is_empty());

        // 500 ms on, the timer has not fired; a composition has opened since
        let scratch = RefCell::new(BrowserController::default());
        crate::ime::handle(
            &scratch,
            crate::ime::Input::Preedit {
                text: "か".to_owned(),
                cursor: None,
            },
        );
        host.modifiers(500, ALT);

        assert_eq!(
            host.cards(),
            vec![CardChange::Show, CardChange::Hide],
            "poll(now) first, then set_focus(composing)"
        );
    }

    #[test]
    fn the_hold_that_is_due_shows_the_card_before_the_keys_are_turned_off() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);

        set_settings_with(
            &mut host,
            ms(500),
            HookSettings {
                config: Config {
                    enabled: false,
                    ..Config::default()
                },
                ..HookSettings::default()
            },
        );

        assert_eq!(
            host.cards(),
            vec![CardChange::Show, CardChange::Hide],
            "poll(now) first, then set_config"
        );
    }

    #[test]
    fn the_hold_that_is_due_still_shows_the_card_when_the_groups_go() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);

        // 500 ms on, both groups are gone: with no group the hold has no path any more
        let world = World {
            groups: Some(Vec::new()),
            ..World::default()
        };
        dispatch(
            &mut host,
            ms(500),
            &world,
            Event::Modifiers(mods_from_winit(ALT)),
        );

        assert_eq!(host.cards(), vec![CardChange::Show]);
    }

    // ---- no re-entry (D1) -----------------------------------------------------------------

    #[test]
    fn a_card_host_that_calls_back_into_the_hook_gets_a_defined_answer() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);
        // the host answers the card appearing by dismissing it, from inside card_changed
        host.nested = Some(Event::Dismiss);

        host.tick(400);

        assert_eq!(
            host.cards(),
            vec![CardChange::Show, CardChange::Hide],
            "the nested call ran after the machine call had returned"
        );
        assert_eq!(
            host.phase(),
            Phase::Idle,
            "a nested call that met a borrowed machine would have got a spare one and left this at Overlay"
        );
    }

    // ---- registering a host re-sends the configuration (D5) -------------------------------

    #[test]
    fn registering_a_card_host_turns_the_keys_on_at_the_next_event() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.card = false;

        host.alt_down(0);
        assert!(
            !host.press(80, KeyCode::KeyS, ALT),
            "no card host: nothing is captured"
        );
        host.alt_up(100);

        host.card = true;
        host.alt_down(200);
        assert!(host.press(280, KeyCode::KeyS, ALT));
        assert_eq!(host.activations(), vec![Target::Group(host.work.value())]);
    }

    #[test]
    fn the_card_host_going_away_closes_the_card() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);
        host.tick(400);

        host.card = false;
        host.modifiers(500, ALT);

        assert_eq!(host.cards(), vec![CardChange::Show, CardChange::Hide]);
    }

    #[test]
    fn with_no_card_host_nothing_is_consumed_or_switched_in_5000_events() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.card = false;

        let report = fuzz(&mut host, 0x5eed_0005, 5_000);

        assert_eq!(report.consumed, 0, "no key may be consumed");
        assert!(host.cards().is_empty());
        assert!(host.activations().is_empty());
        assert!(
            host.timers().iter().all(Option::is_none),
            "no hold may be waited for: {:?}",
            host.timers()
        );
    }

    // ---- the fuzz: no panic, and every consumed down gets a consumed up -------------------

    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }
    }

    struct FuzzReport {
        consumed: usize,
    }

    /// Keys that are one machine key each: the numpad digits are left out, because a digit and the
    /// same numpad digit are one `Key` and release one memory (documented in the crate).
    const FUZZ_KEYS: [KeyCode; 13] = [
        KeyCode::KeyS,
        KeyCode::KeyA,
        KeyCode::KeyZ,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Escape,
        KeyCode::ArrowLeft,
        KeyCode::F1,
        KeyCode::Space,
        KeyCode::AltLeft,
        KeyCode::ShiftLeft,
        KeyCode::SuperLeft,
    ];

    /// `steps` seeded random events through the hook. Panics when a consumed key-down is not
    /// followed by a consumed key-up of the same key (unless a reset came between).
    fn fuzz(host: &mut TestHost, seed: u64, steps: usize) -> FuzzReport {
        let mut rng = Rng(seed);
        let mut now = 0u64;
        let mut mods = NONE;
        let mut down: Vec<KeyCode> = Vec::new();
        let mut consumed_downs: HashSet<KeyCode> = HashSet::new();
        let mut consumed = 0;
        for step in 0..steps {
            now += if rng.below(10) == 0 {
                500
            } else {
                rng.below(120)
            };
            match rng.below(100) {
                0..=24 => {
                    mods = NONE;
                    if rng.below(3) != 0 {
                        mods |= ALT;
                    }
                    if rng.below(5) == 0 {
                        mods |= ModifiersState::SHIFT;
                    }
                    if rng.below(8) == 0 {
                        mods |= ModifiersState::CONTROL;
                    }
                    if rng.below(8) == 0 {
                        mods |= ModifiersState::SUPER;
                    }
                    host.modifiers(now, mods);
                }
                25..=54 => {
                    let code = FUZZ_KEYS[rng.below(FUZZ_KEYS.len() as u64) as usize];
                    let repeat = down.contains(&code) && rng.below(2) == 0;
                    let synthetic = rng.below(10) == 0;
                    let answer =
                        host.key_full(now, code, ElementState::Pressed, mods, repeat, synthetic);
                    assert!(
                        !(synthetic && answer),
                        "step {step}: a synthetic key was consumed"
                    );
                    if !synthetic {
                        if !down.contains(&code) {
                            down.push(code);
                        }
                        if answer {
                            consumed += 1;
                            consumed_downs.insert(code);
                        }
                    }
                }
                55..=79 => {
                    if down.is_empty() {
                        continue;
                    }
                    let code = down.swap_remove(rng.below(down.len() as u64) as usize);
                    let answer =
                        host.key_full(now, code, ElementState::Released, mods, false, false);
                    if consumed_downs.remove(&code) {
                        assert!(
                            answer,
                            "step {step}: the down of {code:?} was consumed, its up was not"
                        );
                    }
                }
                80..=84 => {
                    host.reset(now);
                    consumed_downs.clear();
                }
                85..=89 => {
                    let tab = host.tabs[rng.below(host.tabs.len() as u64) as usize];
                    if host.active() != tab {
                        host.controller.select_tab(tab);
                        // the hook resets at its next event, so earlier downs are forgotten
                        consumed_downs.clear();
                    }
                }
                90..=94 => host.tick(now),
                _ => {
                    let event = match rng.below(4) {
                        0 => Event::PointerDown,
                        1 => Event::Dismiss,
                        2 => Event::TileClicked {
                            group: host.work.value(),
                        },
                        _ => Event::ChipClicked {
                            index: rng.below(4) as usize,
                        },
                    };
                    host.event(now, event);
                }
            }
        }
        FuzzReport { consumed }
    }

    #[test]
    fn a_2000_event_seeded_sequence_never_panics_and_every_consumed_down_has_a_consumed_up() {
        let _guard = test_guard();
        let mut host = TestHost::new();

        let report = fuzz(&mut host, 0x0c0f_fee1, 2_000);

        assert!(
            report.consumed > 0,
            "the sequence consumed nothing: it proves nothing"
        );
        assert!(!host.cards().is_empty(), "no card was ever shown");
        assert!(!host.activations().is_empty(), "no switch was ever made");
    }

    // ---- both orders in which winit reports ⌥ ---------------------------------------------

    #[test]
    fn option_s_works_when_the_modifier_event_comes_before_the_key() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.modifiers(0, ALT);
        host.press(1, KeyCode::AltLeft, ALT);
        assert!(host.press(80, KeyCode::KeyS, ALT));
        assert_eq!(host.activations(), vec![Target::Group(host.work.value())]);
    }

    #[test]
    fn option_s_works_when_the_key_event_comes_before_the_modifier_event() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.press(0, KeyCode::AltLeft, ALT);
        host.modifiers(1, ALT);
        assert!(host.press(80, KeyCode::KeyS, ALT));
        assert_eq!(host.activations(), vec![Target::Group(host.work.value())]);
    }

    #[test]
    fn option_s_works_when_no_modifier_event_came_at_all() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        // the S carries its own modifiers
        assert!(host.press(80, KeyCode::KeyS, ALT));
        assert_eq!(host.activations(), vec![Target::Group(host.work.value())]);
    }

    #[test]
    fn releasing_option_closes_the_card_in_either_order() {
        for key_first in [true, false] {
            let _guard = test_guard();
            let mut host = TestHost::new();
            host.alt_down(0);
            host.tick(400);

            if key_first {
                host.release(500, KeyCode::AltLeft, NONE);
                host.modifiers(501, NONE);
            } else {
                host.modifiers(500, NONE);
                host.release(501, KeyCode::AltLeft, NONE);
            }

            assert_eq!(
                host.cards(),
                vec![CardChange::Show, CardChange::Hide],
                "key_first={key_first}"
            );
        }
    }

    #[test]
    fn the_card_commits_a_chosen_tab_when_option_comes_up() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);
        host.tick(400);
        assert!(
            host.press(500, KeyCode::KeyS, ALT),
            "the card owns the letters"
        );
        assert!(host.press(550, KeyCode::Digit2, ALT));
        host.alt_up(600);

        assert_eq!(
            host.activations(),
            vec![Target::Tab {
                group: host.work.value(),
                index: 1
            }]
        );
        assert_eq!(host.active(), host.controller.group_tabs(host.work)[1]);
    }

    #[test]
    fn a_chip_click_on_the_card_opens_that_tab() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);
        host.tick(400);
        host.press(500, KeyCode::KeyS, ALT);

        host.event(600, Event::ChipClicked { index: 2 });

        assert_eq!(
            host.activations(),
            vec![Target::Tab {
                group: host.work.value(),
                index: 2
            }]
        );
        assert_eq!(host.cards().last(), Some(&CardChange::Hide));
    }

    // ---- focus_inputs (D2) ----------------------------------------------------------------

    #[test]
    fn focus_inputs_truth_table() {
        // (text_input_focused, web_view_focused, has_input_method) -> text_field
        //   the chrome has a text field: text
        //   the page does not have the keyboard (a sheet, the downloads list): text
        //   the page has it and an editable element is focused: text
        //   the page has it and nothing is editable: not text
        let table = [
            (false, false, false, true),
            (false, false, true, true),
            (false, true, false, false),
            (false, true, true, true),
            (true, false, false, true),
            (true, false, true, true),
            (true, true, false, true),
            (true, true, true, true),
        ];
        assert_eq!(table.len() * 2, 16);
        for (text_input, web_view, input_method, text_field) in table {
            for composing in [false, true] {
                assert_eq!(
                    focus_inputs(text_input, web_view, input_method, composing),
                    Focus {
                        text_field,
                        composing
                    },
                    "text_input={text_input} web_view={web_view} input_method={input_method} composing={composing}"
                );
            }
        }
    }

    #[test]
    fn the_focus_is_sent_to_the_machine_only_when_it_changed() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.modifiers(0, NONE);
        let first = with_state(|state| state.sent_focus);
        assert_eq!(first, Some(Focus::default()));

        host.text_input = true;
        host.modifiers(10, NONE);
        assert_eq!(
            with_state(|state| state.sent_focus),
            Some(Focus {
                text_field: true,
                composing: false
            })
        );
    }

    #[test]
    fn an_editable_element_in_the_page_counts_as_a_text_field() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        let tab = host.active();
        host.controller.handle_event(
            tab,
            talaria_core::BrowserEvent::InputMethodShown {
                id: talaria_core::ControlId::new(1),
                rect: talaria_core::ControlRect {
                    x: 0,
                    y: 0,
                    width: 10,
                    height: 10,
                },
            },
        );

        host.alt_down(0);

        assert!(
            !host.press(80, KeyCode::KeyS, ALT),
            "⌥S types in the page's field"
        );
        assert!(host.activations().is_empty());
    }

    // ---- keyed_groups ---------------------------------------------------------------------

    #[test]
    fn keyed_groups_are_the_groups_with_a_key_in_ring_order_with_their_tab_counts() {
        let host = TestHost::new();
        assert_eq!(
            keyed_groups(&host.controller),
            vec![
                KeyedGroup {
                    id: host.home.value(),
                    key: LetterKey::A,
                    tab_count: 1
                },
                KeyedGroup {
                    id: host.work.value(),
                    key: LetterKey::S,
                    tab_count: 3
                },
            ]
        );
    }

    #[test]
    fn a_group_without_a_key_is_skipped_and_a_reordered_ring_is_followed() {
        let mut host = TestHost::new();
        host.controller
            .set_group_key(host.home, None)
            .expect("the key is cleared");
        assert_eq!(
            keyed_groups(&host.controller)
                .iter()
                .map(|group| group.id)
                .collect::<Vec<_>>(),
            vec![host.work.value()]
        );

        host.controller
            .set_group_key(host.home, GroupKey::from_letter('A'))
            .expect("A is free");
        host.controller
            .reorder_group(host.work, 0)
            .expect("the group moves");
        assert_eq!(
            keyed_groups(&host.controller)
                .iter()
                .map(|group| group.id)
                .collect::<Vec<_>>(),
            vec![host.work.value(), host.home.value()],
            "ring order is the controller's order"
        );
    }

    #[test]
    fn two_groups_with_one_letter_leave_the_first_to_the_machine() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        // The controller never gives two groups one key; the machine's rule for it is tested
        // here through the hook with a hand-made list.
        let world = World {
            groups: Some(vec![
                KeyedGroup {
                    id: 11,
                    key: LetterKey::S,
                    tab_count: 2,
                },
                KeyedGroup {
                    id: 12,
                    key: LetterKey::S,
                    tab_count: 2,
                },
            ]),
            focus: Some(Focus::default()),
            ..World::default()
        };
        dispatch(
            &mut host,
            ms(0),
            &world,
            Event::Modifiers(mods_from_winit(ALT)),
        );
        let event = event_from_parts(
            &PhysicalKey::Code(KeyCode::KeyS),
            ElementState::Pressed,
            false,
            ALT,
            false,
        );
        assert!(key_with(&mut host, ms(50), &World::default(), event));
        assert_eq!(host.activations(), vec![Target::Group(11)]);
    }

    #[test]
    fn the_groups_are_sent_when_they_change_and_not_otherwise() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.modifiers(0, ALT);
        let first = with_state(|state| state.sent_groups.clone());
        assert_eq!(first, Some(keyed_groups(&host.controller)));

        host.controller
            .set_group_key(host.work, None)
            .expect("the key is cleared");
        host.modifiers(10, ALT);
        assert_eq!(
            with_state(|state| state.sent_groups.clone()),
            Some(keyed_groups(&host.controller))
        );
        assert_eq!(keyed_groups(&host.controller).len(), 1);
    }

    #[test]
    fn groups_are_not_read_while_alt_is_up_and_the_machine_is_idle() {
        let _guard = test_guard();
        let controller = RefCell::new(TestHost::new().controller);
        let world = read_world(&controller, Some((false, true)), false);
        assert_eq!(world.groups, None);
        let world = read_world(&controller, Some((false, true)), true);
        assert_eq!(world.groups.map(|groups| groups.len()), Some(2));
    }

    // ---- activate (D10) -------------------------------------------------------------------

    #[test]
    fn activating_a_group_shows_its_most_recently_shown_tab() {
        let mut host = TestHost::new();
        let work = host.work;
        let c = host.controller.group_tabs(work)[2];
        host.controller.select_tab(c);
        host.controller.select_tab(host.tabs[0]);

        let applied = activate(&mut host.controller, Target::Group(work.value()));

        assert_eq!(applied, Applied::Group);
        assert_eq!(host.active(), c, "the tab last shown in that group");
    }

    #[test]
    fn activating_a_tab_in_range_selects_it_in_strip_order() {
        let mut host = TestHost::new();
        let work = host.work;
        let strip = host.controller.group_tabs(work);

        for (index, expected) in strip.iter().enumerate() {
            let applied = activate(
                &mut host.controller,
                Target::Tab {
                    group: work.value(),
                    index,
                },
            );
            assert_eq!(applied, Applied::Tab);
            assert_eq!(host.active(), *expected, "index {index}");
        }
    }

    #[test]
    fn a_tab_that_is_gone_falls_back_to_its_group() {
        let mut host = TestHost::new();
        let work = host.work;
        let before = host.active();

        let applied = activate(
            &mut host.controller,
            Target::Tab {
                group: work.value(),
                index: 7,
            },
        );

        assert_eq!(applied, Applied::GroupInsteadOfTab);
        assert_ne!(host.active(), before);
        assert_eq!(host.controller.group_of(host.active()), Some(work));
    }

    #[test]
    fn an_unknown_group_does_nothing() {
        let mut host = TestHost::new();
        let before = host.active();
        for target in [
            Target::Group(999),
            Target::Tab {
                group: 999,
                index: 0,
            },
        ] {
            assert_eq!(activate(&mut host.controller, target), Applied::Nothing);
            assert_eq!(host.active(), before);
        }
    }

    #[test]
    fn a_busy_controller_is_not_a_panic() {
        let host = TestHost::new();
        let work = host.work.value();
        let cell = RefCell::new(host.controller);
        let _reading = cell.borrow();
        assert_eq!(activate_in(&cell, Target::Group(work)), None);
    }

    #[test]
    fn a_busy_controller_is_read_as_a_text_field_with_no_groups_and_no_tab() {
        let _guard = test_guard();
        let host = TestHost::new();
        let cell = RefCell::new(host.controller);
        let _writing = cell.borrow_mut();
        let world = read_world(&cell, Some((false, true)), true);
        assert_eq!(world.active_tab, None);
        assert_eq!(world.groups, None);
        assert_eq!(
            world.focus,
            Some(Focus {
                text_field: true,
                composing: false
            }),
            "unknown counts as a text field: a wrong 'no' eats å"
        );
    }

    // ---- next_timer_delay (D15) -----------------------------------------------------------

    #[test]
    fn the_timer_delay_is_none_zero_or_exact() {
        assert_eq!(next_timer_delay(None, ms(5)), None);
        assert_eq!(
            next_timer_delay(Some(ms(400)), ms(500)),
            Some(Duration::ZERO)
        );
        assert_eq!(
            next_timer_delay(Some(ms(400)), ms(400)),
            Some(Duration::ZERO)
        );
        assert_eq!(next_timer_delay(Some(ms(400)), ms(150)), Some(ms(250)));
        assert_eq!(next_timer_delay(Some(ms(400)), ms(0)), Some(ms(400)));
    }

    // ---- settings -------------------------------------------------------------------------

    #[test]
    fn hook_settings_follow_appearance_tab_groups() {
        let defaults = HookSettings::from_settings(&Settings::default());
        assert_eq!(defaults, HookSettings::default());
        assert_eq!(defaults.config, Config::default());
        assert!(defaults.orbit && defaults.orbit_pinch);

        let mut settings = Settings::default();
        let groups = &mut settings.appearance.tab_groups;
        groups.keys.enabled = false;
        groups.keys.capture = mercury_option_keys::KeyCapture::Always;
        groups.keys.hold_ms = 700;
        groups.orbit = false;
        groups.orbit_pinch = false;
        let hook = HookSettings::from_settings(&settings);
        assert!(!hook.config.enabled);
        assert_eq!(hook.config.capture, mercury_option_keys::KeyCapture::Always);
        assert_eq!(hook.config.hold, ms(700));
        assert!(!hook.orbit && !hook.orbit_pinch);
    }

    #[test]
    fn a_changed_hold_reaches_the_machine() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.modifiers(0, NONE);
        set_settings_with(
            &mut host,
            ms(10),
            HookSettings {
                config: Config::new(true, Default::default(), ms(900)),
                ..HookSettings::default()
            },
        );
        assert_eq!(settings().config.hold, ms(900));

        host.modifiers(1_000, ALT);

        assert_eq!(host.timers().last(), Some(&Some(ms(900))));
    }

    #[test]
    fn keys_turned_off_in_the_settings_close_the_card() {
        let _guard = test_guard();
        let mut host = TestHost::new();
        host.alt_down(0);
        host.tick(400);

        set_settings_with(
            &mut host,
            ms(450),
            HookSettings {
                config: Config {
                    enabled: false,
                    ..Config::default()
                },
                ..HookSettings::default()
            },
        );

        assert_eq!(host.cards(), vec![CardChange::Show, CardChange::Hide]);
    }

    // ---- Orbit's keys (D11) ---------------------------------------------------------------

    #[test]
    fn orbit_keys_from_winit() {
        use orbit_view::Key as Orbit;
        let named = Key::Named;
        let character = |text: &str| Key::Character(text.into());
        let table: Vec<(&str, Key, Option<Orbit>)> = vec![
            ("Escape", named(NamedKey::Escape), Some(Orbit::Escape)),
            ("Enter", named(NamedKey::Enter), Some(Orbit::Enter)),
            ("Up", named(NamedKey::ArrowUp), Some(Orbit::Up)),
            ("Down", named(NamedKey::ArrowDown), Some(Orbit::Down)),
            ("Left", named(NamedKey::ArrowLeft), Some(Orbit::Left)),
            ("Right", named(NamedKey::ArrowRight), Some(Orbit::Right)),
            ("n", character("n"), Some(Orbit::N)),
            ("N", character("N"), Some(Orbit::N)),
            ("/", character("/"), Some(Orbit::Slash)),
            ("a", character("a"), Some(Orbit::Char('a'))),
            ("é", character("é"), Some(Orbit::Char('é'))),
            ("F1", named(NamedKey::F1), None),
            ("Dead", Key::Dead(Some('`')), None),
            ("Dead(none)", Key::Dead(None), None),
            ("Space", named(NamedKey::Space), None),
            ("Tab", named(NamedKey::Tab), None),
            (
                "Unidentified",
                Key::Unidentified(NativeKey::Unidentified),
                None,
            ),
            ("two characters", character("ab"), None),
            ("empty", character(""), None),
            ("a control character", character("\u{3}"), None),
        ];
        for (name, key, expected) in table {
            assert_eq!(orbit_key_from_winit(&key), expected, "{name}");
        }
    }

    #[test]
    fn orbit_mods_from_winit_maps_each_modifier() {
        for bits in 0u8..16 {
            let mut state = ModifiersState::empty();
            if bits & 1 != 0 {
                state |= ModifiersState::SUPER;
            }
            if bits & 2 != 0 {
                state |= ModifiersState::SHIFT;
            }
            if bits & 4 != 0 {
                state |= ModifiersState::ALT;
            }
            if bits & 8 != 0 {
                state |= ModifiersState::CONTROL;
            }
            assert_eq!(
                orbit_mods_from_winit(state),
                orbit_view::Mods {
                    command: bits & 1 != 0,
                    shift: bits & 2 != 0,
                    alt: bits & 4 != 0,
                    control: bits & 8 != 0,
                },
                "bits {bits:04b}"
            );
        }
    }
}
