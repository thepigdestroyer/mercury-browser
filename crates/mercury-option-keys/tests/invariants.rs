// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A deterministic fuzz of the ⌥ state machine against the invariants of the lane brief
//! (`lane-key-switcher-brief.md`, section 6, item 2), written from the brief and from `types.rs`
//! without looking at `machine.rs`.
//!
//! Each seed drives one machine through 20,000 events (and the setters and polls between them)
//! made by a small xorshift generator: random and chorded ⌥ presses, letters of the groups, digits
//! (including 0 and 10), Escape, arrows, modifier keys, auto-repeat, pointer and card events,
//! `Reset`, random `Focus` changes, `set_groups` of 0 to 30 groups with duplicated letters and
//! odd tab counts, `set_config` over the three capture modes (and off), and a clock that mostly
//! creeps, often jumps past the hold, hits the hold boundary exactly, and sometimes goes
//! backwards. After every call the checker below tests:
//!
//! | | |
//! | --- | --- |
//! | I1 | a key-up is consumed iff its key-down was consumed (the checker keeps its own record, cleared by `Reset`) |
//! | I2 | `Event::Modifiers` is never consumed |
//! | I3 | `CardChange` is what `view()` before and after the call say |
//! | I4 | `activate` names a group of the last `set_groups` and a tab below its `tab_count` and below 9 |
//! | I5 | after `Reset` the phase is Idle and the view is hidden and empty |
//! | I6 | a key down whose modifiers are not ⌥ alone is never consumed |
//! | I7 | with `enabled == false`: Idle, hidden, nothing activated, nothing consumed |
//! | I8 | while `Focus.composing`, no key down is consumed |
//! | I9 | `deadline()` is `Some(t0 + hold)` in Armed with a hold path, and `None` otherwise |
//! | I10 | the same actions on a second machine give the same answers |
//! | I11 | `selected_group` and `selected_tab` are `None` unless the card is visible |
//! | I12 | arrows and `Modifier` keys are never consumed (the brief's section 5.1 and 5.4) |
//!
//! Two readings the brief leaves implicit:
//!
//! - A key-up is consumed iff its down was (I1), which is the brief's explicit rule for keys that
//!   outlive the phase they were pressed in. So the key-up of a consumed down is excused from I6
//!   (the brief says so itself) and from I7 (the brief's "nothing consumed" cannot mean a release
//!   whose press the page never saw).
//! - Time that goes backwards "counts as not having moved" (5.3), so the checker's clock is the
//!   largest `now` seen, and `t0` is that clock when Armed was entered.
//!
//! Every failure prints the seed, the index of the action, the invariant and the last ten actions.
//! The first failure of each invariant is reported (with how many there were), so one run shows
//! every kind of disagreement with the brief.

use std::fmt::{self, Write as _};
use std::time::Duration;

use mercury_option_keys::{
    CardChange, CardView, Config, Event, Focus, Key, KeyCapture, KeyedGroup, LetterKey, MAX_DIGIT,
    Mods, OptionKeys, Outcome, Phase, Target,
};

/// The brief contradicts itself on one point, and this switch is where the integrator settles it.
///
/// Section 5.4, "Any phase": "Auto-repeat of a captured key is consumed with no effect". Section 6
/// item 2: I6 (a key down with modifiers other than ⌥ alone is never consumed), I7 (disabled
/// means nothing consumed) and I8 (no key down consumed while composing). For the auto-repeat of
/// a key whose press was consumed, and is still held, the two cannot both hold: ⌘ pressed while a
/// captured letter is held, a composition opened, or the keys switched off.
///
/// `false` (the default) checks I6 to I8 as stated, for every key down, and reports the
/// auto-repeats as their own invariants (`I6 (auto-repeat ...)` and so on) apart from the
/// fresh presses. `true` excuses exactly that class, as the brief already excuses a key-up whose
/// press was consumed.
const EXCUSE_CAPTURED_REPEATS: bool = false;

const I6_REPEAT: &str = "I6 (auto-repeat of a captured key)";
const I7_REPEAT: &str = "I7 (auto-repeat of a captured key)";
const I8_REPEAT: &str = "I8 (auto-repeat of a captured key)";

/// `on_event` calls per seed.
const EVENTS_PER_SEED: usize = 20_000;
/// The brief asks for at least this many events over all seeds.
const MIN_TOTAL_EVENTS: usize = 200_000;

#[derive(Clone, Copy)]
struct Seed {
    seed: u64,
    /// The capture mode the run starts in (the run changes it at random).
    capture: KeyCapture,
    /// The hold the run uses, before the clamp to 250..=1000 ms.
    hold_ms: u64,
}

const fn seed(seed: u64, capture: KeyCapture, hold_ms: u64) -> Seed {
    Seed {
        seed,
        capture,
        hold_ms,
    }
}

const SEEDS: [Seed; 12] = [
    seed(0x9E37_79B9_7F4A_7C15, KeyCapture::OutsideTextOrHold, 400),
    seed(0xD1B5_4A32_D192_ED03, KeyCapture::OutsideText, 400),
    seed(0x8CB9_2BA7_2F3D_8DD7, KeyCapture::Always, 400),
    seed(0x2545_F491_4F6C_DD1D, KeyCapture::OutsideTextOrHold, 250),
    seed(0x1405_7B7E_F767_814F, KeyCapture::OutsideText, 1000),
    seed(0xC2B2_AE3D_27D4_EB4F, KeyCapture::Always, 100),
    seed(0x1656_67B1_9E37_79F9, KeyCapture::OutsideTextOrHold, 5000),
    seed(0x27D4_EB2F_1656_67C5, KeyCapture::OutsideText, 650),
    seed(0x94D0_49BB_1331_11EB, KeyCapture::Always, 399),
    seed(0xBF58_476D_1CE4_E5B9, KeyCapture::OutsideTextOrHold, 400),
    seed(0x0123_4567_89AB_CDEF, KeyCapture::Always, 400),
    seed(0xFEDC_BA98_7654_3210, KeyCapture::OutsideText, 250),
];

// ------------------------------------------------------------------------------------------
// The random source and the actions
// ------------------------------------------------------------------------------------------

/// xorshift64 (13, 7, 17).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// 0 to `n - 1`; `n` is never 0.
    fn below(&mut self, n: u64) -> u64 {
        (self.next_u64() >> 11) % n
    }

    fn index(&mut self, len: usize) -> usize {
        self.below(len as u64) as usize
    }

    fn percent(&mut self, p: u64) -> bool {
        self.below(100) < p
    }
}

/// One call on the machine.
#[derive(Clone, Debug)]
enum Action {
    Config(Config),
    Groups(Vec<KeyedGroup>),
    Focus(Focus),
    Event(Duration, Event),
    Poll(Duration),
}

fn mods_str(m: Mods) -> String {
    let mut s = String::new();
    for (on, c) in [
        (m.alt, 'A'),
        (m.shift, 'S'),
        (m.control, 'C'),
        (m.command, 'M'),
    ] {
        if on {
            s.push(c);
        }
    }
    if s.is_empty() {
        s.push('-');
    }
    s
}

fn key_str(key: Key) -> String {
    match key {
        Key::Letter(l) => l.letter().to_string(),
        Key::Digit(n) => format!("digit{n}"),
        Key::Escape => "Esc".to_owned(),
        Key::Arrow => "Arrow".to_owned(),
        Key::Modifier => "ModKey".to_owned(),
        Key::Other(n) => format!("Other{n}"),
    }
}

fn describe(action: &Action) -> String {
    match action {
        Action::Config(c) => format!(
            "set_config(enabled {}, {:?}, hold {} ms)",
            c.enabled,
            c.capture,
            c.hold.as_millis()
        ),
        Action::Groups(groups) => {
            let list: Vec<String> = groups
                .iter()
                .map(|g| format!("{}:{}x{}", g.id, g.key, g.tab_count))
                .collect();
            format!("set_groups({} [{}])", groups.len(), list.join(" "))
        }
        Action::Focus(f) => format!(
            "set_focus(text_field {}, composing {})",
            f.text_field, f.composing
        ),
        Action::Event(now, event) => {
            let what = match event {
                Event::Modifiers(m) => format!("Modifiers[{}]", mods_str(*m)),
                Event::Key {
                    key,
                    mods,
                    down,
                    repeat,
                } => format!(
                    "Key {} {}{} [{}]",
                    key_str(*key),
                    if *down { "down" } else { "up" },
                    if *repeat { " repeat" } else { "" },
                    mods_str(*mods)
                ),
                other => format!("{other:?}"),
            };
            format!("on_event({} ms, {what})", now.as_millis())
        }
        Action::Poll(now) => format!("poll({} ms)", now.as_millis()),
    }
}

// ------------------------------------------------------------------------------------------
// The generator: a pure function of the seed (it never looks at the machine)
// ------------------------------------------------------------------------------------------

struct Gen {
    rng: Rng,
    /// The frontier of the clock, in milliseconds; stamps are this, or sometimes earlier.
    now: u64,
    /// The time of the last time-stamped action.
    stamp: u64,
    /// The modifiers the generator believes are physically down.
    mods: Mods,
    /// Keys pressed and not yet released (since the last `Reset`).
    held: Vec<Key>,
    /// The last `set_groups` the generator made.
    groups: Vec<KeyedGroup>,
    capture: KeyCapture,
    /// The hold after `Config::new`'s clamp.
    hold: u64,
    hold_given: u64,
    actions: Vec<Action>,
    events: usize,
}

impl Gen {
    fn new(s: Seed) -> Gen {
        let hold = Config::new(true, s.capture, Duration::from_millis(s.hold_ms))
            .hold
            .as_millis() as u64;
        let mut g = Gen {
            rng: Rng::new(s.seed),
            now: 1000,
            stamp: 1000,
            mods: Mods::NONE,
            held: Vec::new(),
            groups: Vec::new(),
            capture: s.capture,
            hold,
            hold_given: s.hold_ms,
            actions: Vec::new(),
            events: 0,
        };
        let groups = g.random_groups(true);
        g.set_groups(groups);
        let focus = g.random_focus();
        g.actions.push(Action::Focus(focus));
        g
    }

    fn initial_config(&self) -> Config {
        Config::new(true, self.capture, Duration::from_millis(self.hold_given))
    }

    fn run(mut self) -> (Config, Vec<Action>) {
        let initial = self.initial_config();
        while self.events < EVENTS_PER_SEED {
            if self.rng.percent(30) {
                self.chord();
            } else {
                self.single();
            }
        }
        (initial, self.actions)
    }

    /// Advance the clock a little (mostly), a lot (often), to the hold boundary or past it, or
    /// back; the result is the time of the next call.
    fn tick(&mut self) -> Duration {
        let hold = self.hold;
        let mut back = None;
        match self.rng.below(100) {
            0..=69 => self.now += self.rng.below(51),
            70..=84 => self.now += 51 + self.rng.below(250),
            85..=91 => self.now += hold - 1 + self.rng.below(4),
            92..=96 => self.now += hold + self.rng.below(600),
            _ => back = Some(1 + self.rng.below(700)),
        }
        self.stamp = back.map_or(self.now, |b| self.now.saturating_sub(b));
        Duration::from_millis(self.stamp)
    }

    fn event(&mut self, event: Event) {
        let at = self.tick();
        self.actions.push(Action::Event(at, event));
        self.events += 1;
    }

    fn poll(&mut self) {
        let at = self.tick();
        self.actions.push(Action::Poll(at));
    }

    fn set_mods(&mut self, mods: Mods) {
        self.mods = mods;
        self.event(Event::Modifiers(mods));
    }

    fn random_mods(&mut self) -> Mods {
        let bits = self.rng.below(16);
        Mods {
            alt: bits & 1 != 0,
            shift: bits & 2 != 0,
            control: bits & 4 != 0,
            command: bits & 8 != 0,
        }
    }

    fn random_focus(&mut self) -> Focus {
        Focus {
            text_field: self.rng.percent(45),
            composing: self.rng.percent(12),
        }
    }

    fn random_letter(&mut self) -> LetterKey {
        LetterKey::ALL[self.rng.index(LetterKey::ALL.len())]
    }

    /// 0 to 30 groups (at least three when `some`), distinct ids, letters that repeat often when
    /// the pool is small, 0 to 14 tabs.
    fn random_groups(&mut self, some: bool) -> Vec<KeyedGroup> {
        let wanted: u64 = if some {
            3 + self.rng.below(8)
        } else {
            match self.rng.below(100) {
                0..=7 => 0,
                8..=67 => 1 + self.rng.below(9),
                68..=89 => 10 + self.rng.below(17),
                _ => 27 + self.rng.below(4),
            }
        };
        let count = wanted as usize;
        let mut ids: Vec<u64> = (1..=60).collect();
        // The largest id a `GroupId` can have.
        ids.push(i32::MAX as u64);
        let pool = [3, 8, 26][self.rng.index(3)];
        let mut groups = Vec::new();
        for i in 0..count {
            let pick = i + self.rng.index(ids.len() - i);
            ids.swap(i, pick);
            groups.push(KeyedGroup {
                id: ids[i],
                key: LetterKey::ALL[self.rng.index(pool)],
                tab_count: self.rng.index(15),
            });
        }
        groups
    }

    fn set_groups(&mut self, groups: Vec<KeyedGroup>) {
        self.groups.clone_from(&groups);
        self.actions.push(Action::Groups(groups));
    }

    fn random_key(&mut self, assigned_bias: u64) -> Key {
        let roll = self.rng.below(100);
        if roll < assigned_bias && !self.groups.is_empty() {
            let i = self.rng.index(self.groups.len());
            return Key::Letter(self.groups[i].key);
        }
        match self.rng.below(100) {
            0..=14 => Key::Letter(self.random_letter()),
            15..=64 => {
                if self.rng.percent(88) {
                    Key::Digit(1 + self.rng.below(u64::from(MAX_DIGIT)) as u8)
                } else {
                    Key::Digit([0, 10, 255][self.rng.index(3)])
                }
            }
            65..=74 => Key::Escape,
            75..=82 => Key::Arrow,
            83..=89 => Key::Modifier,
            _ => Key::Other(100 + self.rng.below(4) as u32),
        }
    }

    /// A press, a release or an auto-repeat of `key`, like a keyboard would send it: a key that
    /// is down is released or repeats, a key that is up is pressed (rarely: released).
    fn key_event(&mut self, key: Key) {
        let mods = if self.rng.percent(92) {
            self.mods
        } else {
            self.random_mods()
        };
        let down_now = self.held.iter().position(|k| *k == key);
        let (down, repeat) = match down_now {
            Some(pos) if self.rng.percent(60) => {
                self.held.remove(pos);
                (false, false)
            }
            Some(_) => (true, true),
            None if self.rng.percent(95) => {
                self.held.push(key);
                (true, false)
            }
            None => (false, false),
        };
        self.event(Event::Key {
            key,
            mods,
            down,
            repeat,
        });
    }

    /// ⌥ goes down (sometimes after another modifier), the hold is waited out (or not), keys are
    /// pressed, ⌥ goes up (or not).
    fn chord(&mut self) {
        let mut mods = Mods::NONE;
        if self.rng.percent(15) {
            match self.rng.below(3) {
                0 => mods.shift = true,
                1 => mods.control = true,
                _ => mods.command = true,
            }
            self.set_mods(mods);
        }
        mods.alt = true;
        self.set_mods(mods);
        let armed_at = self.stamp;
        if self.rng.percent(30) {
            let focus = self.random_focus();
            self.actions.push(Action::Focus(focus));
        }
        match self.rng.below(5) {
            1 => self.now += self.rng.below(self.hold.saturating_sub(2).max(1)),
            2 => {
                // The boundary: hold - 1, hold or hold + 1 after ⌥ went down.
                let target = armed_at + self.hold - 1 + self.rng.below(3);
                self.now = self.now.max(target);
                let at = Duration::from_millis(target);
                self.stamp = target;
                if self.rng.percent(50) {
                    self.actions.push(Action::Poll(at));
                } else {
                    let key = self.random_key(60);
                    self.key_event_at(key, at);
                }
            }
            3 => self.now += self.hold + self.rng.below(400),
            _ => {}
        }
        if self.rng.percent(30) {
            self.poll();
        }
        let keys = self.rng.below(5);
        for i in 0..keys {
            let key = self.random_key(if i == 0 { 75 } else { 50 });
            self.key_event(key);
        }
        match self.rng.below(7) {
            0 => {}
            1..=4 => self.set_mods(Mods::NONE),
            5 => {
                let without_alt = Mods {
                    alt: false,
                    ..self.mods
                };
                self.set_mods(without_alt);
            }
            _ => self.single(),
        }
    }

    /// A key event at an exact time (the hold boundary).
    fn key_event_at(&mut self, key: Key, at: Duration) {
        let before = self.actions.len();
        self.key_event(key);
        if let Some(Action::Event(time, _)) = self.actions.get_mut(before) {
            *time = at;
        }
    }

    /// One action of any kind.
    fn single(&mut self) {
        match self.rng.below(100) {
            0..=29 => {
                let key = self.random_key(35);
                self.key_event(key);
            }
            30..=44 => {
                let mut mods = self.mods;
                match self.rng.below(8) {
                    0..=3 => mods.alt = !mods.alt,
                    4 => mods.shift = !mods.shift,
                    5 => mods.control = !mods.control,
                    6 => mods.command = !mods.command,
                    _ => mods = self.random_mods(),
                }
                self.set_mods(mods);
            }
            45..=47 => self.event(Event::PointerDown),
            48..=51 => {
                let known = !self.groups.is_empty() && self.rng.percent(75);
                let group = if known {
                    self.groups[self.rng.index(self.groups.len())].id
                } else {
                    1000 + self.rng.below(5)
                };
                self.event(Event::TileClicked { group });
            }
            52..=55 => {
                let index = self.rng.index(13);
                self.event(Event::ChipClicked { index });
            }
            56..=57 => self.event(Event::Dismiss),
            58..=59 => {
                self.held.clear();
                self.event(Event::Reset);
            }
            60..=67 => self.poll(),
            68..=74 => {
                let focus = self.random_focus();
                self.actions.push(Action::Focus(focus));
            }
            75..=78 => {
                let groups = self.random_groups(false);
                self.set_groups(groups);
            }
            79..=82 => {
                let capture = [
                    KeyCapture::OutsideTextOrHold,
                    KeyCapture::OutsideText,
                    KeyCapture::Always,
                ][self.rng.index(3)];
                self.capture = capture;
                let enabled = self.rng.percent(88);
                let config = Config::new(enabled, capture, Duration::from_millis(self.hold_given));
                self.actions.push(Action::Config(config));
            }
            _ => {
                let key = self.random_key(40);
                self.key_event(key);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------
// The checker
// ------------------------------------------------------------------------------------------

/// What one call gave, as the machine's `phase`, `view` and `deadline` say afterwards.
type Step = (Outcome, Phase, CardView, Option<Duration>);

struct Failure {
    invariant: &'static str,
    text: String,
    count: usize,
}

#[derive(Default)]
struct Coverage {
    phases: [u64; 5],
    shows: u64,
    hides: u64,
    updates: u64,
    group_switches: u64,
    tab_switches: u64,
    consumed_downs: u64,
    consumed_ups: u64,
    selected_tabs: u64,
    composing_calls: u64,
    disabled_calls: u64,
    resets: u64,
    deadlines: u64,
    by_mode: [u64; 3],
    events: usize,
}

struct Checker {
    seed: u64,
    cfg: Config,
    groups: Vec<KeyedGroup>,
    focus: Focus,
    clock: Duration,
    /// Keys whose down was consumed and whose up has not arrived (I1).
    recorded: Vec<Key>,
    armed_t0: Option<Duration>,
    failures: Vec<Failure>,
    cov: Coverage,
}

/// What a call gave, printed only when an invariant fails.
struct State {
    out: Outcome,
    was: Phase,
    phase: Phase,
    view: CardView,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?}, phase {:?} (was {:?}), view {:?}",
            self.out, self.phase, self.was, self.view
        )
    }
}

/// What the checker needs to say where a call is.
struct Call<'a> {
    index: usize,
    actions: &'a [Action],
}

fn expected_card(before: CardView, after: CardView) -> CardChange {
    match (before.visible, after.visible) {
        (false, true) => CardChange::Show,
        (true, false) => CardChange::Hide,
        (true, true) if before != after => CardChange::Update,
        _ => CardChange::None,
    }
}

impl Checker {
    fn new(seed: u64, cfg: Config) -> Checker {
        Checker {
            seed,
            cfg,
            groups: Vec::new(),
            focus: Focus::default(),
            clock: Duration::ZERO,
            recorded: Vec::new(),
            armed_t0: None,
            failures: Vec::new(),
            cov: Coverage::default(),
        }
    }

    /// The model's view of the world after the call's own input is applied.
    fn pre(&mut self, action: &Action) {
        match action {
            Action::Config(c) => self.cfg = *c,
            Action::Groups(g) => self.groups.clone_from(g),
            Action::Focus(f) => self.focus = *f,
            Action::Event(now, _) | Action::Poll(now) => self.clock = self.clock.max(*now),
        }
    }

    fn fail(&mut self, invariant: &'static str, call: &Call<'_>, what: String) {
        if let Some(known) = self.failures.iter_mut().find(|f| f.invariant == invariant) {
            known.count += 1;
            return;
        }
        let from = call.index.saturating_sub(9);
        let mut text = format!(
            "{invariant} violated, seed {:#x}, action {} ({} events in): {what}\n  last ten actions:\n",
            self.seed, call.index, self.cov.events
        );
        for (i, action) in call.actions[from..=call.index].iter().enumerate() {
            let mark = if from + i == call.index { ">>" } else { "  " };
            let _ = writeln!(text, "   {mark} {:>6}  {}", from + i, describe(action));
        }
        self.failures.push(Failure {
            invariant,
            text,
            count: 1,
        });
    }

    fn check_target(&self, target: Target) -> Result<(), String> {
        match target {
            Target::Group(id) => {
                if self.groups.iter().any(|g| g.id == id) {
                    Ok(())
                } else {
                    Err(format!("Group({id}) is not in the last set_groups"))
                }
            }
            Target::Tab { group, index } => match self.groups.iter().find(|g| g.id == group) {
                None => Err(format!(
                    "Tab {{ group {group}, index {index} }}: no such group in the last set_groups"
                )),
                Some(g) if index >= g.tab_count => Err(format!(
                    "Tab {{ group {group}, index {index} }}: the group has {} tabs",
                    g.tab_count
                )),
                Some(_) if index >= usize::from(MAX_DIGIT) => Err(format!(
                    "Tab {{ group {group}, index {index} }}: digits reach only {MAX_DIGIT}"
                )),
                Some(_) => Ok(()),
            },
        }
    }

    fn observe(
        &mut self,
        call: &Call<'_>,
        action: &Action,
        before: (Phase, CardView),
        out: Outcome,
        after: (Phase, CardView, Option<Duration>),
    ) {
        let (phase, view, deadline) = after;
        let state = State {
            out,
            was: before.0,
            phase,
            view,
        };

        // Coverage.
        self.cov.phases[phase as usize] += 1;
        match out.card {
            CardChange::Show => self.cov.shows += 1,
            CardChange::Hide => self.cov.hides += 1,
            CardChange::Update => self.cov.updates += 1,
            CardChange::None => {}
        }
        match out.activate {
            Some(Target::Group(_)) => self.cov.group_switches += 1,
            Some(Target::Tab { .. }) => self.cov.tab_switches += 1,
            None => {}
        }
        if view.selected_tab.is_some() {
            self.cov.selected_tabs += 1;
        }
        if self.focus.composing {
            self.cov.composing_calls += 1;
        }
        if !self.cfg.enabled {
            self.cov.disabled_calls += 1;
        }
        if deadline.is_some() {
            self.cov.deadlines += 1;
        }
        self.cov.by_mode[match self.cfg.capture {
            KeyCapture::OutsideTextOrHold => 0,
            KeyCapture::OutsideText => 1,
            KeyCapture::Always => 2,
        }] += 1;
        if matches!(action, Action::Event(..)) {
            self.cov.events += 1;
        }

        // I3: the card change is what the views say.
        let want = expected_card(before.1, view);
        if out.card != want {
            self.fail(
                "I3",
                call,
                format!("the card change is {:?}, the views before ({:?}) and after ({view:?}) say {want:?}", out.card, before.1),
            );
        }

        // I11: no selection while hidden.
        if !view.visible && (view.selected_group.is_some() || view.selected_tab.is_some()) {
            self.fail(
                "I11",
                call,
                format!("a hidden card has a selection: {view:?}"),
            );
        }

        // I4: activations name real groups and tabs.
        if let Some(target) = out.activate
            && let Err(why) = self.check_target(target)
        {
            self.fail("I4", call, format!("{why} ({state})"));
        }

        let mut excused_up = false;
        // An auto-repeat of a key whose press was consumed (and not yet released).
        let repeat_of_captured = match action {
            Action::Event(
                _,
                Event::Key {
                    key,
                    down: true,
                    repeat: true,
                    ..
                },
            ) => self.recorded.contains(key),
            _ => false,
        };
        match action {
            Action::Event(_, Event::Modifiers(_)) => {
                // I2.
                if out.consume {
                    self.fail("I2", call, format!("Modifiers consumed ({state})"));
                }
            }
            Action::Event(
                _,
                Event::Key {
                    key, mods, down, ..
                },
            ) => {
                if *down {
                    if out.consume {
                        self.cov.consumed_downs += 1;
                        if !self.recorded.contains(key) {
                            self.recorded.push(*key);
                        }
                        // I6 and I8, strictly as the brief states them; see `EXCUSE_CAPTURED_REPEATS`.
                        if !(EXCUSE_CAPTURED_REPEATS && repeat_of_captured) {
                            let (i6, i8) = if repeat_of_captured {
                                (I6_REPEAT, I8_REPEAT)
                            } else {
                                ("I6", "I8")
                            };
                            let what = if repeat_of_captured {
                                format!("an auto-repeat of {}", key_str(*key))
                            } else {
                                format!("the key down of {}", key_str(*key))
                            };
                            if !mods.alt_only() {
                                self.fail(
                                    i6,
                                    call,
                                    format!("{what} with [{}] consumed ({state})", mods_str(*mods)),
                                );
                            }
                            if self.focus.composing {
                                self.fail(
                                    i8,
                                    call,
                                    format!("{what} consumed while composing ({state})"),
                                );
                            }
                        }
                    }
                } else {
                    // I1.
                    let was_consumed = self.recorded.iter().position(|k| k == key);
                    if let Some(pos) = was_consumed {
                        self.recorded.remove(pos);
                        excused_up = true;
                    }
                    if out.consume != was_consumed.is_some() {
                        self.fail(
                            "I1",
                            call,
                            format!(
                                "the key-up of {} is {} but its down was {}consumed ({state})",
                                key_str(*key),
                                if out.consume { "consumed" } else { "passed" },
                                if was_consumed.is_some() { "" } else { "not " }
                            ),
                        );
                    }
                    if out.consume {
                        self.cov.consumed_ups += 1;
                    }
                }
                // I12.
                if matches!(key, Key::Arrow | Key::Modifier) && out.consume {
                    self.fail("I12", call, format!("{} consumed ({state})", key_str(*key)));
                }
            }
            Action::Event(_, Event::Reset) => {
                self.cov.resets += 1;
                // I5.
                if phase != Phase::Idle || view != CardView::default() {
                    self.fail("I5", call, format!("after Reset: {state}"));
                }
                self.recorded.clear();
            }
            _ => {}
        }

        // I7.
        if !self.cfg.enabled {
            let consumed = out.consume && !excused_up;
            let repeat_only = consumed && repeat_of_captured;
            if consumed && !repeat_only {
                self.fail(
                    "I7",
                    call,
                    format!("the keys are off but a key is consumed: {state}"),
                );
            }
            if repeat_only && !EXCUSE_CAPTURED_REPEATS {
                self.fail(
                    I7_REPEAT,
                    call,
                    format!("the keys are off but an auto-repeat is consumed: {state}"),
                );
            }
            if out.activate.is_some() || view.visible || phase != Phase::Idle {
                self.fail("I7", call, format!("the keys are off but: {state}"));
            }
        }

        // I9.
        if phase == Phase::Armed {
            if before.0 != Phase::Armed {
                self.armed_t0 = Some(self.clock);
            }
        } else {
            self.armed_t0 = None;
        }
        let hold_path = self.cfg.enabled
            && self.cfg.capture != KeyCapture::OutsideText
            && !self.groups.is_empty();
        let want_deadline = match self.armed_t0 {
            Some(t0) if phase == Phase::Armed && hold_path => Some(t0 + self.cfg.hold),
            _ => None,
        };
        if deadline != want_deadline {
            self.fail(
                "I9",
                call,
                format!(
                    "deadline() is {deadline:?}, expected {want_deadline:?} (phase {phase:?}, t0 {:?}, hold {:?}, hold path {hold_path}, clock {:?})",
                    self.armed_t0, self.cfg.hold, self.clock
                ),
            );
        }
    }
}

// ------------------------------------------------------------------------------------------
// Running a seed
// ------------------------------------------------------------------------------------------

fn call_machine(keys: &mut OptionKeys, action: &Action) -> Outcome {
    match action {
        Action::Config(c) => keys.set_config(*c),
        Action::Groups(g) => keys.set_groups(g),
        Action::Focus(f) => keys.set_focus(*f),
        Action::Event(now, event) => keys.on_event(*now, *event),
        Action::Poll(now) => keys.poll(*now),
    }
}

/// Run the actions on a new machine; check every call (I1 to I9, I11, I12) as it happens.
fn execute(seed: u64, initial: Config, actions: &[Action]) -> (Vec<Step>, Checker) {
    let mut keys = OptionKeys::new(initial);
    let mut checker = Checker::new(seed, initial);
    let mut transcript = Vec::with_capacity(actions.len());
    for (index, action) in actions.iter().enumerate() {
        let before = (keys.phase(), keys.view());
        checker.pre(action);
        let out = call_machine(&mut keys, action);
        let after = (keys.phase(), keys.view(), keys.deadline());
        transcript.push((out, after.0, after.1, after.2));
        let call = Call { index, actions };
        checker.observe(&call, action, before, out, after);
    }
    (transcript, checker)
}

/// The same actions on another new machine (I10).
fn replay(initial: Config, actions: &[Action]) -> Vec<Step> {
    let mut keys = OptionKeys::new(initial);
    actions
        .iter()
        .map(|action| {
            let out = call_machine(&mut keys, action);
            (out, keys.phase(), keys.view(), keys.deadline())
        })
        .collect()
}

fn run_seed(s: Seed) {
    let (initial, actions) = Gen::new(s).run();
    let events = actions
        .iter()
        .filter(|a| matches!(a, Action::Event(..)))
        .count();
    assert!(
        events >= EVENTS_PER_SEED,
        "the generator made {events} events"
    );

    let (first, mut checker) = execute(s.seed, initial, &actions);

    // I10.
    let second = replay(initial, &actions);
    if let Some(index) = first.iter().zip(&second).position(|(a, b)| a != b) {
        let call = Call {
            index,
            actions: &actions,
        };
        checker.fail(
            "I10",
            &call,
            format!(
                "the first run gave {:?}, the second {:?}",
                first[index], second[index]
            ),
        );
    } else if first.len() != second.len() {
        let call = Call {
            index: actions.len() - 1,
            actions: &actions,
        };
        checker.fail("I10", &call, "the runs have different lengths".to_owned());
    }

    // The generator must reach what the invariants are about (a fuzz that never gets there
    // proves nothing).
    let cov = &checker.cov;
    let mut thin = Vec::new();
    let mut need = |name: &str, got: u64, min: u64| {
        if got < min {
            thin.push(format!("{name}: {got} (wanted at least {min})"));
        }
    };
    for (phase, name) in [
        (Phase::Idle, "Idle"),
        (Phase::Armed, "Armed"),
        (Phase::PassThrough, "PassThrough"),
        (Phase::Switched, "Switched"),
        (Phase::Overlay, "Overlay"),
    ] {
        need(name, cov.phases[phase as usize], 100);
    }
    need("Show", cov.shows, 20);
    need("Hide", cov.hides, 20);
    need("Update", cov.updates, 20);
    need("group switches", cov.group_switches, 20);
    need("tab switches", cov.tab_switches, 20);
    need("consumed downs", cov.consumed_downs, 200);
    need("consumed ups", cov.consumed_ups, 200);
    need("calls with a tab selected", cov.selected_tabs, 20);
    need("calls while composing", cov.composing_calls, 200);
    need("calls while disabled", cov.disabled_calls, 200);
    need("resets", cov.resets, 50);
    need("calls with a deadline", cov.deadlines, 50);
    for (mode, name) in ["OutsideTextOrHold", "OutsideText", "Always"]
        .iter()
        .enumerate()
    {
        need(name, cov.by_mode[mode], 1000);
    }

    let mut report = String::new();
    for failure in &checker.failures {
        let _ = writeln!(
            report,
            "{} ({} times in this run)\n{}",
            failure.invariant, failure.count, failure.text
        );
    }
    for line in &thin {
        let _ = writeln!(
            report,
            "COVERAGE (the generator is too thin to prove anything): {line}"
        );
    }
    assert!(report.is_empty(), "\nseed {:#x}:\n{report}", s.seed);
}

macro_rules! fuzz_seeds {
    ($($name:ident => $index:expr),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run_seed(SEEDS[$index]);
            }
        )*
    };
}

fuzz_seeds! {
    fuzz_seed_01_outside_text_or_hold_400 => 0,
    fuzz_seed_02_outside_text_400 => 1,
    fuzz_seed_03_always_400 => 2,
    fuzz_seed_04_outside_text_or_hold_250 => 3,
    fuzz_seed_05_outside_text_1000 => 4,
    fuzz_seed_06_always_clamped_up_to_250 => 5,
    fuzz_seed_07_outside_text_or_hold_clamped_down_to_1000 => 6,
    fuzz_seed_08_outside_text_650 => 7,
    fuzz_seed_09_always_399 => 8,
    fuzz_seed_10_outside_text_or_hold_400 => 9,
    fuzz_seed_11_always_400 => 10,
    fuzz_seed_12_outside_text_250 => 11,
}

/// The brief asks for 200,000 events or more over several seeds and the three capture modes.
#[test]
fn the_seeds_add_up_to_at_least_200_000_events_in_all_three_modes() {
    assert!(SEEDS.len() * EVENTS_PER_SEED >= MIN_TOTAL_EVENTS);
    for capture in [
        KeyCapture::OutsideTextOrHold,
        KeyCapture::OutsideText,
        KeyCapture::Always,
    ] {
        let starts = SEEDS.iter().filter(|s| s.capture == capture).count();
        assert!(starts >= 3, "{capture:?} starts only {starts} seeds");
    }
    // The holds include both clamps and the hold boundary of the SPEC.
    assert!(SEEDS.iter().any(|s| s.hold_ms < 250));
    assert!(SEEDS.iter().any(|s| s.hold_ms > 1000));
    assert!(SEEDS.iter().any(|s| s.hold_ms == 399 || s.hold_ms == 400));
}

/// The generator is a pure function of the seed: two generations are the same list of actions.
#[test]
fn the_generator_is_deterministic() {
    let (config_a, actions_a) = Gen::new(SEEDS[0]).run();
    let (config_b, actions_b) = Gen::new(SEEDS[0]).run();
    assert_eq!(config_a, config_b);
    let text = |actions: &[Action]| actions.iter().map(describe).collect::<Vec<_>>();
    assert_eq!(text(&actions_a), text(&actions_b));
    let (_, other) = Gen::new(SEEDS[1]).run();
    assert_ne!(text(&actions_a), text(&other));
}
