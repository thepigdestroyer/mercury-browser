// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The ⌃⇥ hold (lane brief `lane-input-hooks-brief.md` D18, D19; SPEC §4.3 "hold ⌃⇥").
//!
//! The first ⌃⇥ press is never touched: it stays `shortcuts.rs`'s "next tab". The [`CtrlTab`]
//! machine only watches: when ⌃ is still down [`RING_HOLD`] after that press, the hub ring opens
//! on the tab the user landed on; while it is open ⇥ and ⇧⇥ move the ring's focus (and are
//! consumed); releasing ⌃ commits and closes.
//!
//! Like the ⌥ machine (`crates/mercury-option-keys/src/machine.rs`) it reads no clock and no
//! winit event: time is injected as a `Duration`, keys arrive as plain values, and the hold
//! deadline is applied lazily at the start of every call, so a hold timer that fires late never
//! changes what a key does. The hook in `keyboard.rs` calls [`CtrlTab::deadline`] after each call
//! to arm its timer, and applies the returned [`RingEffect`] to the `RingHost` with [`apply`].
//!
//! `RingHost::is_open` is read by the hook at every call (it is in [`CtrlTabEnv`]): a ring that
//! closed itself (Esc, a click, a tab chosen) ends the hold with no effect, and `close` is only
//! called when this machine ended the hold.

#![allow(dead_code)] // wired by the input-hooks `wire` item

use std::time::Duration;

use mercury_option_keys::{Key, Mods};
use slint::winit_030::winit;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

use crate::input_hooks::RingHost;
use crate::option_keys::{key_from_winit as option_key_from_winit, mods_from_winit};

/// How long ⌃ must stay down after the arming ⌃⇥ press before the ring opens (brief D18). A
/// constant, not a setting.
pub(crate) const RING_HOLD: Duration = Duration::from_millis(400);

/// One key event as the machine takes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CtrlTabKey {
    /// The physical key is ⇥.
    pub tab: bool,
    /// The key is itself a modifier key (its state arrives through [`CtrlTab::on_modifiers`]);
    /// such an event is never "another key".
    pub modifier: bool,
    /// ⇧ is down.
    pub shift: bool,
    /// ⌃ is down.
    pub control: bool,
    /// ⌘ is down.
    pub command: bool,
    /// ⌥ is down.
    pub alt: bool,
    /// A key-down (`false`: a key-up).
    pub down: bool,
    /// The key-down is an auto-repeat.
    pub repeat: bool,
    /// winit replayed the key when the window gained focus; the user is not pressing it.
    pub synthetic: bool,
}

/// What the hook reads from the outside at every call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CtrlTabEnv {
    /// `RingHost::available()`; `false` with no host registered.
    pub ring_available: bool,
    /// An IME composition is open (`ime::composition_active()`).
    pub composing: bool,
    /// `RingHost::is_open()`; `false` with no host registered.
    pub ring_open: bool,
}

impl CtrlTabEnv {
    /// The environment of one call. With no host registered every ring flag is `false`, so the
    /// machine arms nothing and consumes nothing (brief D22).
    pub(crate) fn from_host(host: Option<&dyn RingHost>, composing: bool) -> Self {
        Self {
            ring_available: host.is_some_and(|host| host.available()),
            composing,
            ring_open: host.is_some_and(|host| host.is_open()),
        }
    }
}

/// What the hook must do to the ring. Apply it with [`apply`] after the call returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RingEffect {
    /// Open the ring held (`RingHost::open_held`).
    Open,
    /// Move the ring's focus one tab (`RingHost::cycle`).
    Cycle { backwards: bool },
    /// [`RingEffect::Open`] then [`RingEffect::Cycle`]: a hold timer that fired late and a ⇥
    /// press that came before it. One value, because a call returns one effect.
    OpenAndCycle { backwards: bool },
    /// ⌃ was released: activate the ring's focused tab and close (`RingHost::commit_and_close`).
    CommitAndClose,
    /// The machine abandoned the hold (⌘ or ⌥ pressed, a reset): close without activating
    /// (`RingHost::close`).
    Close,
}

/// The answer of one call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CtrlTabOutcome {
    /// The key event must not reach Slint, the page or `shortcuts.rs` (`PreventDefault`).
    pub consume: bool,
    /// What to do to the ring, if anything.
    pub effect: Option<RingEffect>,
}

impl CtrlTabOutcome {
    /// Nothing consumed, nothing to do.
    pub(crate) const PASS: Self = Self {
        consume: false,
        effect: None,
    };
}

/// Where the hold stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Nothing to watch: ⌃ is up, or the press was not an arming one.
    Idle,
    /// The arming ⌃⇥ press happened at this time; ⌃ must still be down at `t0 + hold`.
    Armed(Duration),
    /// The ring is open and this machine owns the hold.
    Open,
    /// ⌃ is down but the hold is over or was never possible (cancelled by another key or by ⌘ or
    /// ⌥, the ring was not available, an IME was composing, the ring closed itself). Nothing
    /// happens until ⌃ comes up: the machine does not retry.
    Quiet,
}

/// The ⌃⇥ hold machine. See the module documentation.
///
/// What the caller must know:
///
/// - The first ⌃⇥ press is never consumed. A second ⇥ press before the deadline re-arms from the
///   new press (⌃ held while tapping ⇥ through the tabs: the hold counts from the last tap).
/// - Every call first notices a ring that closed itself, then applies the hold deadline, then
///   handles its own event. A late timer therefore never changes what a key does. If the deadline
///   and the event of the same call both act on the ring, the call returns one effect:
///   `Open` + ⇥ is [`RingEffect::OpenAndCycle`]; `Open` + ⌃ released or ⌘/⌥ pressed is no effect
///   at all (the ring would have been shown and closed with nothing chosen).
/// - [`CtrlTab::reset`] takes no environment: its `Close` may reach a ring that has just closed
///   itself, so `RingHost::close` must be harmless on a closed ring.
#[derive(Debug)]
pub(crate) struct CtrlTab {
    hold: Duration,
    state: State,
    /// The modifiers as the machine last saw them.
    mods: Mods,
    /// A ⇥ down was consumed and its up has not come yet (the key-up memory).
    tab_consumed: bool,
    /// The latest time seen: time that goes backwards counts as not having moved.
    clock: Duration,
}

impl CtrlTab {
    /// A machine with nothing armed; `hold` is [`RING_HOLD`] in the application.
    pub(crate) fn new(hold: Duration) -> Self {
        Self {
            hold,
            state: State::Idle,
            mods: Mods::NONE,
            tab_consumed: false,
            clock: Duration::ZERO,
        }
    }

    /// One key event at `now`. `key.synthetic` events and modifier keys themselves (their state
    /// arrives through [`CtrlTab::on_modifiers`]) never change the hold, but the deadline is still
    /// applied.
    pub(crate) fn on_key(
        &mut self,
        now: Duration,
        key: CtrlTabKey,
        env: CtrlTabEnv,
    ) -> CtrlTabOutcome {
        let mut effect = self.begin(now, env);
        if key.synthetic || key.modifier {
            return CtrlTabOutcome {
                consume: false,
                effect,
            };
        }
        let mods = Mods {
            alt: key.alt,
            shift: key.shift,
            control: key.control,
            command: key.command,
        };
        effect = merge(effect, self.follow_modifiers(mods));

        let consume = if !key.down {
            key.tab && std::mem::take(&mut self.tab_consumed)
        } else if key.repeat {
            // the repeat of a held ⇥: the ring's while it is open, or of a press we consumed
            key.tab && (self.state == State::Open || self.tab_consumed)
        } else {
            self.key_down(key.tab, mods, &mut effect, env)
        };
        CtrlTabOutcome { consume, effect }
    }

    /// The modifier state changed (winit's `ModifiersChanged`) at `now`. Never consumed.
    pub(crate) fn on_modifiers(
        &mut self,
        now: Duration,
        mods: Mods,
        env: CtrlTabEnv,
    ) -> CtrlTabOutcome {
        let effect = self.begin(now, env);
        CtrlTabOutcome {
            consume: false,
            effect: merge(effect, self.follow_modifiers(mods)),
        }
    }

    /// The hold timer: opens the ring when `now` has reached the deadline.
    pub(crate) fn poll(&mut self, now: Duration, env: CtrlTabEnv) -> CtrlTabOutcome {
        CtrlTabOutcome {
            consume: false,
            effect: self.begin(now, env),
        }
    }

    /// When the ring will open if nothing else happens; `Some` only while armed.
    pub(crate) fn deadline(&self) -> Option<Duration> {
        match self.state {
            State::Armed(t0) => Some(t0.saturating_add(self.hold)),
            _ => None,
        }
    }

    /// Whether the machine has the ring open (it emitted `Open` and has not ended the hold).
    pub(crate) fn holding(&self) -> bool {
        self.state == State::Open
    }

    /// Forget everything (focus lost, window closing, occlusion). `Close` if the ring was open.
    pub(crate) fn reset(&mut self) -> Option<RingEffect> {
        let effect = (self.state == State::Open).then_some(RingEffect::Close);
        self.state = State::Idle;
        self.mods = Mods::NONE;
        self.tab_consumed = false;
        effect
    }

    /// The start of every call: a ring that closed itself ends the hold (no effect), the clock
    /// moves forward (never back), and the hold deadline is applied.
    fn begin(&mut self, now: Duration, env: CtrlTabEnv) -> Option<RingEffect> {
        if self.state == State::Open && !env.ring_open {
            self.state = self.resting();
        }
        self.clock = self.clock.max(now);
        if let State::Armed(t0) = self.state
            && self.clock >= t0.saturating_add(self.hold)
        {
            if can_open(env) {
                self.state = State::Open;
                return Some(RingEffect::Open);
            }
            self.state = State::Quiet;
        }
        None
    }

    /// What a hold that ends on its own becomes: quiet while ⌃ is down, else idle.
    fn resting(&self) -> State {
        if self.mods.control {
            State::Quiet
        } else {
            State::Idle
        }
    }

    /// Takes the modifier state; ⌃ released ends every hold (committing an open ring), ⌘ or ⌥
    /// pressed abandons it (closing an open ring).
    fn follow_modifiers(&mut self, mods: Mods) -> Option<RingEffect> {
        self.mods = mods;
        match self.state {
            State::Idle => None,
            State::Quiet => {
                if !mods.control {
                    self.state = State::Idle;
                }
                None
            }
            State::Armed(_) => {
                if !mods.control {
                    self.state = State::Idle;
                } else if mods.command || mods.alt {
                    self.state = State::Quiet;
                }
                None
            }
            State::Open => {
                if !mods.control {
                    self.state = State::Idle;
                    Some(RingEffect::CommitAndClose)
                } else if mods.command || mods.alt {
                    self.state = State::Quiet;
                    Some(RingEffect::Close)
                } else {
                    None
                }
            }
        }
    }

    /// A non-repeat key-down. Returns whether it is consumed.
    fn key_down(
        &mut self,
        tab: bool,
        mods: Mods,
        effect: &mut Option<RingEffect>,
        env: CtrlTabEnv,
    ) -> bool {
        match self.state {
            State::Idle | State::Armed(_) => {
                if tab && mods.control && !mods.command && !mods.alt {
                    self.state = if can_open(env) {
                        State::Armed(self.clock)
                    } else {
                        State::Quiet
                    };
                } else if matches!(self.state, State::Armed(_)) {
                    self.state = State::Quiet;
                }
                false
            }
            State::Quiet => false,
            State::Open => {
                if !tab {
                    return false;
                }
                *effect = merge(
                    *effect,
                    Some(RingEffect::Cycle {
                        backwards: mods.shift,
                    }),
                );
                self.tab_consumed = true;
                true
            }
        }
    }
}

/// The ring can be opened by the hold now: it exists and nothing prevents it, no IME is composing,
/// and nobody else (the View menu) has it open.
fn can_open(env: CtrlTabEnv) -> bool {
    env.ring_available && !env.composing && !env.ring_open
}

/// Combines the effect of the deadline with the effect of the event of the same call.
fn merge(first: Option<RingEffect>, second: Option<RingEffect>) -> Option<RingEffect> {
    match (first, second) {
        (Some(RingEffect::Open), Some(RingEffect::CommitAndClose | RingEffect::Close)) => None,
        (Some(RingEffect::Open), Some(RingEffect::Cycle { backwards })) => {
            Some(RingEffect::OpenAndCycle { backwards })
        }
        (first, None) => first,
        (_, second) => second,
    }
}

/// Carry out an effect on the ring.
pub(crate) fn apply(host: &dyn RingHost, effect: RingEffect) {
    match effect {
        RingEffect::Open => host.open_held(),
        RingEffect::Cycle { backwards } => host.cycle(backwards),
        RingEffect::OpenAndCycle { backwards } => {
            host.open_held();
            host.cycle(backwards);
        }
        RingEffect::CommitAndClose => host.commit_and_close(),
        RingEffect::Close => host.close(),
    }
}

/// The machine's view of a winit key event (a `KeyEvent` cannot be built in a test, so the logic
/// takes its parts: the pattern of `option_keys::event_from_parts`).
pub(crate) fn key_from_parts(
    physical: &PhysicalKey,
    state: ElementState,
    repeat: bool,
    mods: ModifiersState,
    synthetic: bool,
) -> CtrlTabKey {
    let mods = mods_from_winit(mods);
    CtrlTabKey {
        tab: *physical == PhysicalKey::Code(KeyCode::Tab),
        modifier: option_key_from_winit(physical) == Key::Modifier,
        shift: mods.shift,
        control: mods.control,
        command: mods.command,
        alt: mods.alt,
        down: state.is_pressed(),
        repeat,
        synthetic,
    }
}

/// [`key_from_parts`] for a winit `KeyEvent`.
pub(crate) fn key_from_winit(
    event: &KeyEvent,
    mods: ModifiersState,
    synthetic: bool,
) -> CtrlTabKey {
    key_from_parts(
        &event.physical_key,
        event.state,
        event.repeat,
        mods,
        synthetic,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use winit::keyboard::NativeKeyCode;

    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    const fn mods(shift: bool, control: bool, command: bool, alt: bool) -> Mods {
        Mods {
            alt,
            shift,
            control,
            command,
        }
    }

    const NONE: Mods = Mods::NONE;
    const CTRL: Mods = mods(false, true, false, false);
    const CTRL_SHIFT: Mods = mods(true, true, false, false);
    const CTRL_CMD: Mods = mods(false, true, true, false);
    const CTRL_ALT: Mods = mods(false, true, false, true);

    fn key(tab: bool, m: Mods, down: bool, repeat: bool) -> CtrlTabKey {
        CtrlTabKey {
            tab,
            modifier: false,
            shift: m.shift,
            control: m.control,
            command: m.command,
            alt: m.alt,
            down,
            repeat,
            synthetic: false,
        }
    }

    fn tab_down(m: Mods) -> CtrlTabKey {
        key(true, m, true, false)
    }

    fn tab_up(m: Mods) -> CtrlTabKey {
        key(true, m, false, false)
    }

    fn tab_repeat(m: Mods) -> CtrlTabKey {
        key(true, m, true, true)
    }

    fn other_down(m: Mods) -> CtrlTabKey {
        key(false, m, true, false)
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Call {
        Open,
        Cycle(bool),
        Commit,
        Close,
    }

    /// A recording `RingHost`: `open` follows the calls the way the real ring would.
    struct FakeRing {
        available: Cell<bool>,
        open: Cell<bool>,
        calls: RefCell<Vec<Call>>,
    }

    impl FakeRing {
        fn new() -> Self {
            Self {
                available: Cell::new(true),
                open: Cell::new(false),
                calls: RefCell::new(Vec::new()),
            }
        }

        fn record(&self, call: Call) {
            self.calls.borrow_mut().push(call);
        }
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
            self.record(Call::Open);
        }
        fn cycle(&self, backwards: bool) {
            self.record(Call::Cycle(backwards));
        }
        fn commit_and_close(&self) {
            self.open.set(false);
            self.record(Call::Commit);
        }
        fn close(&self) {
            self.open.set(false);
            self.record(Call::Close);
        }
        fn option_key(&self, _key: char) {}
    }

    /// The machine, a fake ring and a manual clock: what the hook does around every call.
    struct Rig {
        machine: CtrlTab,
        ring: FakeRing,
        composing: bool,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                machine: CtrlTab::new(RING_HOLD),
                ring: FakeRing::new(),
                composing: false,
            }
        }

        fn env(&self) -> CtrlTabEnv {
            CtrlTabEnv::from_host(Some(&self.ring), self.composing)
        }

        fn key(&mut self, at: u64, key: CtrlTabKey) -> CtrlTabOutcome {
            let env = self.env();
            let outcome = self.machine.on_key(ms(at), key, env);
            self.apply(outcome);
            outcome
        }

        fn mods(&mut self, at: u64, m: Mods) -> CtrlTabOutcome {
            let env = self.env();
            let outcome = self.machine.on_modifiers(ms(at), m, env);
            self.apply(outcome);
            outcome
        }

        fn poll(&mut self, at: u64) -> CtrlTabOutcome {
            let env = self.env();
            let outcome = self.machine.poll(ms(at), env);
            self.apply(outcome);
            outcome
        }

        fn reset(&mut self) -> Option<RingEffect> {
            let effect = self.machine.reset();
            if let Some(effect) = effect {
                apply(&self.ring, effect);
            }
            effect
        }

        fn apply(&self, outcome: CtrlTabOutcome) {
            if let Some(effect) = outcome.effect {
                apply(&self.ring, effect);
            }
        }

        /// ⌃ goes down, then ⇥ (the ⌃⇥ press `shortcuts.rs` handles as a tab switch).
        fn press_ctrl_tab(&mut self, at: u64) -> CtrlTabOutcome {
            self.mods(at, CTRL);
            self.key(at, tab_down(CTRL))
        }

        fn calls(&self) -> Vec<Call> {
            self.ring.calls.borrow().clone()
        }
    }

    // ---- the first press and the hold ---------------------------------------------------

    #[test]
    fn the_hold_is_400_ms_and_a_new_machine_has_nothing_armed() {
        assert_eq!(RING_HOLD, ms(400));
        let machine = CtrlTab::new(RING_HOLD);
        assert_eq!(machine.deadline(), None);
        assert!(!machine.holding());
    }

    #[test]
    fn a_tap_gives_no_effect_and_consumes_nothing() {
        let mut rig = Rig::new();
        let down = rig.press_ctrl_tab(0);
        assert_eq!(
            down,
            CtrlTabOutcome::PASS,
            "the first press is shortcuts.rs's"
        );
        assert_eq!(rig.key(50, tab_up(CTRL)), CtrlTabOutcome::PASS);
        assert_eq!(rig.mods(100, NONE), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), None);
        assert_eq!(rig.poll(1_000), CtrlTabOutcome::PASS);
        assert!(rig.calls().is_empty());
    }

    #[test]
    fn the_deadline_is_the_arming_press_plus_the_hold() {
        let mut rig = Rig::new();
        assert_eq!(rig.machine.deadline(), None);
        rig.press_ctrl_tab(1_000);
        assert_eq!(rig.machine.deadline(), Some(ms(1_400)));
    }

    #[test]
    fn held_to_399_ms_does_not_open() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        assert_eq!(rig.poll(399), CtrlTabOutcome::PASS);
        assert!(rig.calls().is_empty());
        assert_eq!(rig.machine.deadline(), Some(ms(400)));
    }

    #[test]
    fn held_to_400_ms_opens_exactly_once() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        let outcome = rig.poll(400);
        assert_eq!(
            outcome,
            CtrlTabOutcome {
                consume: false,
                effect: Some(RingEffect::Open)
            }
        );
        assert!(rig.machine.holding());
        assert_eq!(rig.machine.deadline(), None);
        assert_eq!(rig.poll(401), CtrlTabOutcome::PASS);
        assert_eq!(rig.poll(5_000), CtrlTabOutcome::PASS);
        assert_eq!(rig.calls(), vec![Call::Open]);
    }

    #[test]
    fn the_deadline_is_lazy_and_a_later_key_event_applies_it() {
        // no poll ran (a late timer): the repeat of the held ⇥ at 400 ms finds the ring due
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        let outcome = rig.key(400, tab_repeat(CTRL));
        assert_eq!(
            outcome,
            CtrlTabOutcome {
                consume: true,
                effect: Some(RingEffect::Open)
            }
        );
        assert_eq!(rig.calls(), vec![Call::Open]);
    }

    #[test]
    fn the_deadline_is_lazy_for_a_later_modifier_event_too() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        let outcome = rig.mods(450, CTRL_SHIFT);
        assert_eq!(outcome.effect, Some(RingEffect::Open));
        assert!(!outcome.consume);
        assert_eq!(rig.calls(), vec![Call::Open]);
    }

    #[test]
    fn a_late_key_that_is_not_a_tab_passes_after_the_ring_opened() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        let outcome = rig.key(450, other_down(CTRL));
        assert_eq!(
            outcome,
            CtrlTabOutcome {
                consume: false,
                effect: Some(RingEffect::Open)
            },
            "the deadline came first: the ring opened and the key is the ring's to handle"
        );
    }

    #[test]
    fn a_late_tab_press_opens_then_cycles_in_one_effect() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        let outcome = rig.key(450, tab_down(CTRL_SHIFT));
        assert_eq!(
            outcome,
            CtrlTabOutcome {
                consume: true,
                effect: Some(RingEffect::OpenAndCycle { backwards: true })
            }
        );
        assert_eq!(rig.calls(), vec![Call::Open, Call::Cycle(true)]);
    }

    #[test]
    fn a_late_release_of_control_never_shows_the_ring() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        // the timer never fired; ⌃ came up at 450 ms: the ring would have opened and closed
        // again with nothing chosen, so it is not shown at all
        let outcome = rig.mods(450, NONE);
        assert_eq!(outcome, CtrlTabOutcome::PASS);
        assert!(rig.calls().is_empty());
        assert!(!rig.machine.holding());
        assert_eq!(rig.machine.deadline(), None);
    }

    #[test]
    fn a_late_command_press_never_shows_the_ring() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        assert_eq!(rig.mods(450, CTRL_CMD), CtrlTabOutcome::PASS);
        assert!(rig.calls().is_empty());
        assert!(!rig.machine.holding());
    }

    // ---- cycling, committing, closing ---------------------------------------------------

    /// A rig with the ring opened by a hold at 400 ms (⌃ down, the first ⇥ still down).
    fn open_rig() -> Rig {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        rig.poll(400);
        assert_eq!(rig.calls(), vec![Call::Open]);
        rig
    }

    #[test]
    fn in_open_a_tab_cycles_forward_and_is_consumed() {
        let mut rig = open_rig();
        assert_eq!(rig.key(500, tab_up(CTRL)), CtrlTabOutcome::PASS);
        let outcome = rig.key(600, tab_down(CTRL));
        assert_eq!(
            outcome,
            CtrlTabOutcome {
                consume: true,
                effect: Some(RingEffect::Cycle { backwards: false })
            }
        );
        assert_eq!(rig.calls(), vec![Call::Open, Call::Cycle(false)]);
    }

    #[test]
    fn in_open_shift_tab_cycles_backwards_and_is_consumed() {
        let mut rig = open_rig();
        let outcome = rig.key(600, tab_down(CTRL_SHIFT));
        assert_eq!(
            outcome,
            CtrlTabOutcome {
                consume: true,
                effect: Some(RingEffect::Cycle { backwards: true })
            }
        );
        assert_eq!(rig.calls(), vec![Call::Open, Call::Cycle(true)]);
    }

    #[test]
    fn in_open_a_repeat_of_tab_is_consumed_with_no_effect() {
        let mut rig = open_rig();
        for at in [450, 480, 510, 540] {
            assert_eq!(
                rig.key(at, tab_repeat(CTRL)),
                CtrlTabOutcome {
                    consume: true,
                    effect: None
                }
            );
        }
        assert_eq!(
            rig.calls(),
            vec![Call::Open],
            "a held key must not spin the ring"
        );
    }

    #[test]
    fn before_the_ring_opens_a_repeat_of_tab_is_the_pages() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        assert_eq!(rig.key(200, tab_repeat(CTRL)), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), Some(ms(400)), "still armed");
    }

    #[test]
    fn the_tab_up_of_a_consumed_down_is_consumed() {
        let mut rig = open_rig();
        rig.key(600, tab_down(CTRL));
        assert!(rig.key(650, tab_up(CTRL)).consume);
        // and only once: the memory is spent
        assert!(!rig.key(660, tab_up(CTRL)).consume);
    }

    #[test]
    fn the_tab_up_of_the_unconsumed_first_press_is_not_consumed() {
        let mut rig = open_rig();
        rig.key(450, tab_repeat(CTRL));
        assert!(
            !rig.key(500, tab_up(CTRL)).consume,
            "the repeats were consumed, the first press was not: its up is not"
        );
    }

    #[test]
    fn the_tab_up_after_the_control_release_is_still_consumed() {
        let mut rig = open_rig();
        rig.key(600, tab_down(CTRL));
        rig.mods(620, NONE);
        assert!(rig.key(650, tab_up(NONE)).consume);
    }

    #[test]
    fn a_repeat_of_a_consumed_tab_after_the_control_release_is_consumed() {
        let mut rig = open_rig();
        rig.key(600, tab_down(CTRL));
        rig.mods(620, NONE);
        assert_eq!(
            rig.key(640, tab_repeat(NONE)),
            CtrlTabOutcome {
                consume: true,
                effect: None
            }
        );
    }

    #[test]
    fn control_up_in_open_commits_once() {
        let mut rig = open_rig();
        let outcome = rig.mods(700, NONE);
        assert_eq!(
            outcome,
            CtrlTabOutcome {
                consume: false,
                effect: Some(RingEffect::CommitAndClose)
            }
        );
        assert!(!rig.machine.holding());
        assert_eq!(rig.mods(710, NONE), CtrlTabOutcome::PASS);
        assert_eq!(rig.poll(2_000), CtrlTabOutcome::PASS);
        assert_eq!(rig.calls(), vec![Call::Open, Call::Commit]);
    }

    #[test]
    fn a_key_event_without_control_in_open_commits_too() {
        // the key event carries its own modifiers; ⌃ is gone before ModifiersChanged arrived
        let mut rig = open_rig();
        let outcome = rig.key(700, other_down(NONE));
        assert_eq!(
            outcome,
            CtrlTabOutcome {
                consume: false,
                effect: Some(RingEffect::CommitAndClose)
            }
        );
    }

    #[test]
    fn command_down_in_open_closes_and_never_commits() {
        let mut rig = open_rig();
        let outcome = rig.mods(500, CTRL_CMD);
        assert_eq!(outcome.effect, Some(RingEffect::Close));
        assert!(!outcome.consume);
        // the ⌃ release that follows does nothing: the hold is over
        assert_eq!(
            rig.mods(600, mods(false, false, true, false)),
            CtrlTabOutcome::PASS
        );
        assert_eq!(rig.mods(700, NONE), CtrlTabOutcome::PASS);
        assert_eq!(rig.calls(), vec![Call::Open, Call::Close]);
    }

    #[test]
    fn option_down_in_open_closes_and_never_commits() {
        let mut rig = open_rig();
        let outcome = rig.mods(500, CTRL_ALT);
        assert_eq!(outcome.effect, Some(RingEffect::Close));
        assert_eq!(rig.mods(700, NONE), CtrlTabOutcome::PASS);
        assert_eq!(rig.calls(), vec![Call::Open, Call::Close]);
    }

    #[test]
    fn reset_in_open_closes() {
        let mut rig = open_rig();
        assert_eq!(rig.reset(), Some(RingEffect::Close));
        assert!(!rig.machine.holding());
        assert_eq!(rig.reset(), None, "only once");
        assert_eq!(rig.calls(), vec![Call::Open, Call::Close]);
    }

    #[test]
    fn reset_outside_open_has_no_effect_and_disarms() {
        let mut rig = Rig::new();
        assert_eq!(rig.reset(), None);
        rig.press_ctrl_tab(0);
        assert_eq!(rig.reset(), None);
        assert_eq!(rig.machine.deadline(), None);
        assert_eq!(rig.poll(1_000), CtrlTabOutcome::PASS);
        assert!(rig.calls().is_empty());
    }

    #[test]
    fn reset_forgets_a_consumed_tab_down() {
        let mut rig = open_rig();
        rig.key(600, tab_down(CTRL));
        rig.reset();
        assert!(!rig.key(650, tab_up(NONE)).consume);
    }

    #[test]
    fn a_closed_ring_ends_the_hold_with_no_effect_and_no_close() {
        let mut rig = open_rig();
        rig.ring.open.set(false); // Esc, a click or a chosen tab closed it
        let outcome = rig.key(500, tab_down(CTRL));
        assert_eq!(
            outcome,
            CtrlTabOutcome::PASS,
            "no cycle, nothing consumed: the key is the page's again"
        );
        assert!(!rig.machine.holding());
        assert_eq!(rig.mods(600, NONE), CtrlTabOutcome::PASS);
        assert_eq!(rig.calls(), vec![Call::Open], "no Close, no CommitAndClose");
    }

    #[test]
    fn a_closed_ring_is_noticed_by_a_modifier_event_and_a_poll_too() {
        let mut rig = open_rig();
        rig.ring.open.set(false);
        assert_eq!(
            rig.mods(500, CTRL_CMD),
            CtrlTabOutcome::PASS,
            "no Close for a closed ring"
        );
        assert!(!rig.machine.holding());

        let mut rig = open_rig();
        rig.ring.open.set(false);
        assert_eq!(rig.poll(500), CtrlTabOutcome::PASS);
        assert!(!rig.machine.holding());
        assert_eq!(rig.calls(), vec![Call::Open]);
    }

    #[test]
    fn after_the_ring_closed_itself_a_new_press_arms_once_control_has_been_released() {
        let mut rig = open_rig();
        rig.ring.open.set(false);
        rig.poll(500);
        // ⌃ is still down: quiet. A later release returns the machine to Idle.
        assert_eq!(rig.machine.deadline(), None);
        rig.mods(600, NONE);
        rig.press_ctrl_tab(1_000);
        assert_eq!(rig.machine.deadline(), Some(ms(1_400)));
        rig.poll(1_400);
        assert_eq!(rig.calls(), vec![Call::Open, Call::Open]);
    }

    // ---- when the ring does not open -----------------------------------------------------

    #[test]
    fn a_ring_that_is_not_available_at_the_deadline_never_opens_and_does_not_retry() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        rig.ring.available.set(false);
        assert_eq!(rig.poll(400), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), None);
        // available again, ⌃ still down: the machine is quiet until ⌃ comes up
        rig.ring.available.set(true);
        assert_eq!(rig.poll(500), CtrlTabOutcome::PASS);
        assert_eq!(rig.key(600, tab_down(CTRL)), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), None);
        assert_eq!(rig.poll(2_000), CtrlTabOutcome::PASS);
        assert!(rig.calls().is_empty());
        // ⌃ up, then a fresh press works
        rig.mods(2_100, NONE);
        rig.press_ctrl_tab(3_000);
        assert_eq!(rig.poll(3_400).effect, Some(RingEffect::Open));
    }

    #[test]
    fn a_ring_that_is_not_available_at_the_press_does_not_arm() {
        let mut rig = Rig::new();
        rig.ring.available.set(false);
        rig.press_ctrl_tab(0);
        assert_eq!(rig.machine.deadline(), None);
        assert_eq!(rig.poll(400), CtrlTabOutcome::PASS);
    }

    #[test]
    fn composing_at_the_deadline_blocks_the_ring() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        rig.composing = true;
        assert_eq!(rig.poll(400), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), None);
        rig.composing = false;
        assert_eq!(
            rig.poll(900),
            CtrlTabOutcome::PASS,
            "quiet until ⌃ comes up"
        );
        assert!(rig.calls().is_empty());
    }

    #[test]
    fn composing_at_the_press_does_not_arm() {
        let mut rig = Rig::new();
        rig.composing = true;
        rig.press_ctrl_tab(0);
        assert_eq!(rig.machine.deadline(), None);
        rig.composing = false;
        assert_eq!(rig.poll(400), CtrlTabOutcome::PASS);
    }

    #[test]
    fn a_ring_that_is_already_open_is_not_opened_again() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        rig.ring.open.set(true); // opened from the View menu
        assert_eq!(rig.poll(400), CtrlTabOutcome::PASS);
        assert!(!rig.machine.holding(), "not this machine's ring");
        assert!(rig.calls().is_empty());
    }

    // ---- what cancels or prevents arming -------------------------------------------------

    #[test]
    fn another_key_with_control_before_the_deadline_cancels_arming() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        assert_eq!(rig.key(100, other_down(CTRL)), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), None);
        assert_eq!(rig.poll(400), CtrlTabOutcome::PASS);
        assert!(rig.calls().is_empty());
    }

    #[test]
    fn a_modifier_key_is_not_another_key() {
        // ⇧ pressed after ⌃⇥ (a ⌃⇧⇥ hold built in two steps): its KeyboardInput must not cancel
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        rig.mods(100, CTRL_SHIFT);
        let shift_key = CtrlTabKey {
            modifier: true,
            ..key(false, CTRL_SHIFT, true, false)
        };
        assert_eq!(rig.key(100, shift_key), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), Some(ms(400)));
        assert_eq!(rig.poll(400).effect, Some(RingEffect::Open));
    }

    #[test]
    fn releasing_the_tab_key_does_not_cancel_while_control_stays_down() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        assert_eq!(rig.key(80, tab_up(CTRL)), CtrlTabOutcome::PASS);
        assert_eq!(rig.poll(400).effect, Some(RingEffect::Open));
    }

    #[test]
    fn a_second_tab_press_before_the_deadline_rearms_from_the_new_press() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(0);
        rig.key(100, tab_up(CTRL));
        assert_eq!(rig.key(300, tab_down(CTRL)), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), Some(ms(700)));
        assert_eq!(rig.poll(400), CtrlTabOutcome::PASS);
        assert_eq!(rig.poll(700).effect, Some(RingEffect::Open));
    }

    #[test]
    fn command_control_tab_never_arms() {
        let mut rig = Rig::new();
        rig.mods(0, CTRL_CMD);
        assert_eq!(rig.key(0, tab_down(CTRL_CMD)), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), None);
        assert_eq!(rig.poll(400), CtrlTabOutcome::PASS);
        assert!(rig.calls().is_empty());
    }

    #[test]
    fn option_control_tab_never_arms() {
        let mut rig = Rig::new();
        rig.mods(0, CTRL_ALT);
        assert_eq!(rig.key(0, tab_down(CTRL_ALT)), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), None);
        assert_eq!(rig.poll(400), CtrlTabOutcome::PASS);
        assert!(rig.calls().is_empty());
    }

    #[test]
    fn command_or_option_pressed_while_armed_cancels_arming() {
        for added in [CTRL_CMD, CTRL_ALT] {
            let mut rig = Rig::new();
            rig.press_ctrl_tab(0);
            assert_eq!(rig.mods(100, added), CtrlTabOutcome::PASS);
            assert_eq!(rig.machine.deadline(), None);
            assert_eq!(rig.poll(400), CtrlTabOutcome::PASS);
            assert!(rig.calls().is_empty());
        }
    }

    #[test]
    fn tab_without_control_never_arms() {
        let mut rig = Rig::new();
        assert_eq!(rig.key(0, tab_down(NONE)), CtrlTabOutcome::PASS);
        assert_eq!(
            rig.key(10, tab_down(mods(true, false, false, false))),
            CtrlTabOutcome::PASS
        );
        assert_eq!(rig.machine.deadline(), None);
    }

    #[test]
    fn a_synthetic_tab_never_arms() {
        let mut rig = Rig::new();
        rig.mods(0, CTRL);
        let replay = CtrlTabKey {
            synthetic: true,
            ..tab_down(CTRL)
        };
        assert_eq!(rig.key(0, replay), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), None);
        assert_eq!(rig.poll(400), CtrlTabOutcome::PASS);
    }

    #[test]
    fn synthetic_events_are_ignored_in_every_state() {
        let mut rig = open_rig();
        let replay = CtrlTabKey {
            synthetic: true,
            ..tab_down(CTRL)
        };
        assert_eq!(
            rig.key(500, replay),
            CtrlTabOutcome::PASS,
            "no cycle for a replay"
        );
        let replay_up = CtrlTabKey {
            synthetic: true,
            ..tab_up(CTRL)
        };
        rig.key(600, tab_down(CTRL)); // a real, consumed press
        assert!(
            !rig.key(610, replay_up).consume,
            "a replayed up is not the real up"
        );
        assert!(rig.key(620, tab_up(CTRL)).consume);
        assert_eq!(rig.calls(), vec![Call::Open, Call::Cycle(false)]);
    }

    #[test]
    fn shift_first_arms_like_control_tab_and_opens_the_same() {
        let mut rig = Rig::new();
        rig.mods(0, mods(true, false, false, false));
        rig.mods(10, CTRL_SHIFT);
        assert_eq!(rig.key(20, tab_down(CTRL_SHIFT)), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), Some(ms(420)));
        assert_eq!(
            rig.poll(420),
            CtrlTabOutcome {
                consume: false,
                effect: Some(RingEffect::Open)
            },
            "an Open, not a backwards cycle"
        );
        assert_eq!(rig.calls(), vec![Call::Open]);
    }

    // ---- sequences and time --------------------------------------------------------------

    #[test]
    fn two_holds_in_a_row_work() {
        let mut rig = Rig::new();
        for start in [0, 2_000] {
            rig.press_ctrl_tab(start);
            assert_eq!(rig.poll(start + 400).effect, Some(RingEffect::Open));
            assert_eq!(
                rig.key(start + 500, tab_down(CTRL)).effect,
                Some(RingEffect::Cycle { backwards: false })
            );
            rig.key(start + 550, tab_up(CTRL));
            assert_eq!(
                rig.mods(start + 600, NONE).effect,
                Some(RingEffect::CommitAndClose)
            );
            assert_eq!(rig.machine.deadline(), None);
            assert!(!rig.machine.holding());
        }
        assert_eq!(
            rig.calls(),
            vec![
                Call::Open,
                Call::Cycle(false),
                Call::Commit,
                Call::Open,
                Call::Cycle(false),
                Call::Commit
            ]
        );
    }

    #[test]
    fn time_going_backwards_counts_as_not_moving() {
        let mut rig = Rig::new();
        rig.press_ctrl_tab(1_000);
        assert_eq!(rig.poll(200), CtrlTabOutcome::PASS);
        assert_eq!(rig.machine.deadline(), Some(ms(1_400)));
        // a later press stamped earlier than the clock is armed at the clock
        rig.key(500, tab_down(CTRL));
        assert_eq!(rig.machine.deadline(), Some(ms(1_400)));
        assert_eq!(rig.poll(1_399), CtrlTabOutcome::PASS);
        assert_eq!(rig.poll(1_400).effect, Some(RingEffect::Open));
        assert_eq!(rig.poll(100), CtrlTabOutcome::PASS);
        assert_eq!(rig.calls(), vec![Call::Open]);
    }

    #[test]
    fn a_huge_time_does_not_overflow() {
        let mut machine = CtrlTab::new(RING_HOLD);
        let env = CtrlTabEnv {
            ring_available: true,
            ..CtrlTabEnv::default()
        };
        machine.on_modifiers(Duration::MAX, CTRL, env);
        machine.on_key(Duration::MAX, tab_down(CTRL), env);
        assert_eq!(machine.deadline(), Some(Duration::MAX));
        assert_eq!(
            machine.poll(Duration::MAX, env).effect,
            Some(RingEffect::Open)
        );
    }

    // ---- no host, apply, conversion ------------------------------------------------------

    #[test]
    fn the_environment_with_no_host_is_all_false() {
        assert_eq!(CtrlTabEnv::from_host(None, false), CtrlTabEnv::default());
        assert_eq!(
            CtrlTabEnv::from_host(None, true),
            CtrlTabEnv {
                composing: true,
                ..CtrlTabEnv::default()
            }
        );
        let ring = FakeRing::new();
        ring.open.set(true);
        assert_eq!(
            CtrlTabEnv::from_host(Some(&ring), false),
            CtrlTabEnv {
                ring_available: true,
                composing: false,
                ring_open: true
            }
        );
    }

    #[test]
    fn apply_maps_each_effect_to_its_host_call() {
        let cases = [
            (RingEffect::Open, vec![Call::Open]),
            (
                RingEffect::Cycle { backwards: false },
                vec![Call::Cycle(false)],
            ),
            (
                RingEffect::Cycle { backwards: true },
                vec![Call::Cycle(true)],
            ),
            (
                RingEffect::OpenAndCycle { backwards: false },
                vec![Call::Open, Call::Cycle(false)],
            ),
            (
                RingEffect::OpenAndCycle { backwards: true },
                vec![Call::Open, Call::Cycle(true)],
            ),
            (RingEffect::CommitAndClose, vec![Call::Commit]),
            (RingEffect::Close, vec![Call::Close]),
        ];
        for (effect, expected) in cases {
            let ring = FakeRing::new();
            apply(&ring, effect);
            assert_eq!(*ring.calls.borrow(), expected, "{effect:?}");
        }
    }

    #[test]
    fn with_no_host_a_long_sequence_passes_every_key_and_calls_nothing() {
        // brief D22: nothing registered, nothing changes
        let mut machine = CtrlTab::new(RING_HOLD);
        let env = CtrlTabEnv::from_host(None, false);
        let mut rng = Lcg(0x5eed_0004);
        let mut now = 0u64;
        let mut m = NONE;
        for _ in 0..5_000 {
            now += rng.below(300);
            let outcome = match rng.below(6) {
                0 => {
                    m.control = !m.control;
                    machine.on_modifiers(ms(now), m, env)
                }
                1 => machine.on_key(ms(now), tab_down(CTRL), env),
                2 => machine.on_key(ms(now), tab_repeat(CTRL), env),
                3 => machine.on_key(ms(now), tab_up(CTRL), env),
                4 => machine.poll(ms(now), env),
                _ => machine.on_key(ms(now), other_down(m), env),
            };
            assert_eq!(outcome, CtrlTabOutcome::PASS);
            assert_eq!(machine.deadline(), None);
        }
        assert_eq!(machine.reset(), None);
    }

    #[test]
    fn a_tab_key_converts_with_its_modifiers_and_flags() {
        let tab = PhysicalKey::Code(KeyCode::Tab);
        let converted = key_from_parts(
            &tab,
            ElementState::Pressed,
            false,
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            false,
        );
        assert_eq!(converted, tab_down(CTRL_SHIFT));

        let up = key_from_parts(
            &tab,
            ElementState::Released,
            false,
            ModifiersState::CONTROL,
            false,
        );
        assert_eq!(up, tab_up(CTRL));

        let repeat = key_from_parts(
            &tab,
            ElementState::Pressed,
            true,
            ModifiersState::CONTROL,
            false,
        );
        assert_eq!(repeat, tab_repeat(CTRL));

        let synthetic = key_from_parts(
            &tab,
            ElementState::Pressed,
            false,
            ModifiersState::CONTROL,
            true,
        );
        assert!(synthetic.synthetic);

        let command = key_from_parts(
            &tab,
            ElementState::Pressed,
            false,
            ModifiersState::CONTROL | ModifiersState::SUPER | ModifiersState::ALT,
            false,
        );
        assert!(command.command && command.alt && command.control && !command.shift);
    }

    #[test]
    fn only_the_tab_key_is_tab_and_modifier_keys_are_modifiers() {
        let convert = |code| {
            key_from_parts(
                &PhysicalKey::Code(code),
                ElementState::Pressed,
                false,
                ModifiersState::CONTROL,
                false,
            )
        };
        assert!(convert(KeyCode::Tab).tab);
        for other in [
            KeyCode::KeyA,
            KeyCode::Escape,
            KeyCode::F6,
            KeyCode::Backquote,
            KeyCode::ArrowRight,
        ] {
            let converted = convert(other);
            assert!(!converted.tab && !converted.modifier, "{other:?}");
        }
        for modifier in [
            KeyCode::ControlLeft,
            KeyCode::ShiftRight,
            KeyCode::SuperLeft,
            KeyCode::AltRight,
            KeyCode::CapsLock,
        ] {
            let converted = convert(modifier);
            assert!(converted.modifier && !converted.tab, "{modifier:?}");
        }
        let unknown = key_from_parts(
            &PhysicalKey::Unidentified(NativeKeyCode::Unidentified),
            ElementState::Pressed,
            false,
            ModifiersState::empty(),
            false,
        );
        assert!(!unknown.tab && !unknown.modifier);
    }

    // ---- the property test ---------------------------------------------------------------

    /// A tiny seeded generator (xorshift): the test needs repeatable noise, not a dependency.
    struct Lcg(u64);

    impl Lcg {
        fn next_u64(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, bound: u64) -> u64 {
            self.next_u64() % bound
        }

        fn chance(&mut self, percent: u64) -> bool {
            self.below(100) < percent
        }
    }

    #[test]
    fn three_thousand_random_events_never_panic_and_keep_open_and_close_paired() {
        for seed in [
            0x9e37_79b9_7f4a_7c15u64,
            0x1234_5678_9abc_def1,
            0xdead_beef_0000_0001,
        ] {
            run_random_sequence(seed, 3_000);
        }
    }

    /// What the effects of a sequence say about the ring, checked after every machine call.
    struct Tracker {
        open: bool,
        opens: usize,
    }

    impl Tracker {
        fn see(&mut self, step: usize, env: CtrlTabEnv, effect: Option<RingEffect>) {
            match effect {
                Some(RingEffect::Open) | Some(RingEffect::OpenAndCycle { .. }) => {
                    assert!(
                        !self.open,
                        "step {step}: Open twice without a Close or CommitAndClose"
                    );
                    assert!(
                        env.ring_available && !env.composing && !env.ring_open,
                        "step {step}: opened a ring that cannot open: {env:?}"
                    );
                    self.open = true;
                    self.opens += 1;
                }
                Some(RingEffect::Cycle { .. }) => {
                    assert!(self.open, "step {step}: Cycle with no Open");
                }
                Some(RingEffect::CommitAndClose) => {
                    assert!(self.open, "step {step}: CommitAndClose without an Open");
                    self.open = false;
                }
                Some(RingEffect::Close) => {
                    assert!(self.open, "step {step}: Close without an Open");
                    self.open = false;
                }
                None => {}
            }
        }
    }

    /// One machine call on `rig`, with the environment it saw and the invariants checked.
    fn call(
        tracker: &mut Tracker,
        rig: &mut Rig,
        step: usize,
        f: impl FnOnce(&mut Rig) -> CtrlTabOutcome,
    ) -> CtrlTabOutcome {
        let env = rig.env();
        let outcome = f(rig);
        tracker.see(step, env, outcome.effect);
        if tracker.open && !rig.machine.holding() && outcome.effect.is_none() {
            // the only way to end a hold silently is a ring that closed itself
            assert!(
                !env.ring_open,
                "step {step}: the hold ended with the ring still open"
            );
            tracker.open = false;
        }
        // every call re-reads the ring, so afterwards the machine agrees with the effects
        assert_eq!(rig.machine.holding(), tracker.open, "step {step}");
        if rig.machine.holding() {
            assert_eq!(rig.machine.deadline(), None, "step {step}");
        }
        outcome
    }

    /// Runs `events` random events; returns how many times the ring was opened.
    fn run_random_sequence(seed: u64, events: usize) -> usize {
        let mut rng = Lcg(seed);
        let mut rig = Rig::new();
        let mut tracker = Tracker {
            open: false,
            opens: 0,
        };
        let mut m = NONE;
        let mut now = 0u64;
        // a consumed Tab-down that has not had its up yet
        let mut tab_pending = false;

        for step in 0..events {
            now += rng.below(160);
            let at = if rng.chance(5) {
                now.saturating_sub(rng.below(400))
            } else {
                now
            };
            rig.composing = rng.chance(5);
            rig.ring.available.set(!rng.chance(15));

            match rng.below(100) {
                0..=34 => {
                    let synthetic = rng.chance(5);
                    let (down, repeat) = match rng.below(3) {
                        0 => (true, false),
                        1 => (true, true),
                        _ => (false, false),
                    };
                    let k = CtrlTabKey {
                        synthetic,
                        ..key(true, m, down, repeat)
                    };
                    let outcome = call(&mut tracker, &mut rig, step, |rig| rig.key(at, k));
                    if !synthetic && down && !repeat && outcome.consume {
                        tab_pending = true;
                    }
                    if !synthetic && !down && tab_pending {
                        assert!(
                            outcome.consume,
                            "step {step}: the up of a consumed Tab down passed"
                        );
                        tab_pending = false;
                    }
                }
                35..=49 => {
                    let k = CtrlTabKey {
                        modifier: rng.chance(30),
                        synthetic: rng.chance(5),
                        ..key(false, m, rng.chance(70), rng.chance(20))
                    };
                    call(&mut tracker, &mut rig, step, |rig| rig.key(at, k));
                }
                50..=69 => {
                    match rng.below(4) {
                        0 => m.control = !m.control,
                        1 => m.shift = !m.shift,
                        2 => m.command = rng.chance(30) && !m.command,
                        _ => m.alt = rng.chance(30) && !m.alt,
                    }
                    call(&mut tracker, &mut rig, step, |rig| rig.mods(at, m));
                }
                70..=79 => {
                    call(&mut tracker, &mut rig, step, |rig| rig.poll(at));
                }
                80..=84 => {
                    // reset has no environment: the hook's `close` must be harmless on a ring that
                    // closed itself, so the ring is left open here
                    let effect = rig.reset();
                    tracker.see(step, rig.env(), effect);
                    assert!(!rig.machine.holding());
                    m = NONE;
                    tab_pending = false;
                }
                85..=89 => {
                    // the ring closes itself; the hook's next call notices
                    rig.ring.open.set(false);
                    call(&mut tracker, &mut rig, step, |rig| rig.poll(at));
                }
                _ => {
                    // a deliberate ⌃⇥ press, so that holds are reached
                    m = CTRL;
                    call(&mut tracker, &mut rig, step, |rig| rig.mods(at, m));
                    call(&mut tracker, &mut rig, step, |rig| rig.key(at, tab_down(m)));
                }
            }
        }
        tracker.opens
    }

    #[test]
    fn the_random_sequences_reach_the_open_state_often_enough_to_mean_something() {
        let opens: usize = [1u64, 2, 3]
            .into_iter()
            .map(|seed| run_random_sequence(0x2545_f491_4f6c_dd1d ^ seed, 3_000))
            .sum();
        assert!(opens >= 30, "only {opens} opens");
    }
}
