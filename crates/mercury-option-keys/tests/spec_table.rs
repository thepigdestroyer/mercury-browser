// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The ⌥ state machine read from the lane brief (`lane-key-switcher-brief.md`, sections 5.1 to 5.4
//! and 7) and from `types.rs`, one short script per line of the transition table.
//!
//! This file was written without looking at `machine.rs` or `tests/machine.rs`: it is a second,
//! independent reading of the specification. Every script is a list of steps; a step is the time
//! (`now`, in milliseconds since an origin the test chooses), the event or setter, the exact
//! `Outcome` and what `phase()` and `view()` say afterwards. Where the brief says nothing, the
//! script says nothing either (`ANY`, or a check on one field only) instead of guessing.
//!
//! The first five tests are the sentence of `design/ui-final/SPEC.md` §8, lane 11, by name.

use std::time::Duration;

use mercury_option_keys::{
    CardChange, CardView, Config, Event, Focus, Key, KeyCapture, KeyedGroup, LetterKey, Mods,
    OptionKeys, Outcome, Phase, Target,
};

// ------------------------------------------------------------------------------------------
// Vocabulary
// ------------------------------------------------------------------------------------------

const fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

const NONE: Mods = Mods::NONE;
const ALT: Mods = Mods::ALT;
const SHIFT: Mods = Mods {
    alt: false,
    shift: true,
    control: false,
    command: false,
};
const CONTROL: Mods = Mods {
    alt: false,
    shift: false,
    control: true,
    command: false,
};
const COMMAND: Mods = Mods {
    alt: false,
    shift: false,
    control: false,
    command: true,
};

/// `other` with ⌥ added.
fn with_alt(other: Mods) -> Mods {
    Mods { alt: true, ..other }
}

const S_KEY: Key = Key::Letter(LetterKey::S);
const A_KEY: Key = Key::Letter(LetterKey::A);
const E_KEY: Key = Key::Letter(LetterKey::E);
/// A letter no test group owns.
const Q_KEY: Key = Key::Letter(LetterKey::Q);

const fn digit(n: u8) -> Key {
    Key::Digit(n)
}

fn mods(m: Mods) -> Event {
    Event::Modifiers(m)
}

fn down(key: Key, m: Mods) -> Event {
    Event::Key {
        key,
        mods: m,
        down: true,
        repeat: false,
    }
}

fn up(key: Key, m: Mods) -> Event {
    Event::Key {
        key,
        mods: m,
        down: false,
        repeat: false,
    }
}

/// An auto-repeat of a key that is held.
fn again(key: Key, m: Mods) -> Event {
    Event::Key {
        key,
        mods: m,
        down: true,
        repeat: true,
    }
}

// The groups of the scripts: three with a key and a tab count that matters, and one with no tabs.
/// Key S, 5 tabs.
const RESEARCH: u64 = 11;
/// Key A, 2 tabs.
const TRIP: u64 = 12;
/// Key E (⌥E is a dead key), 12 tabs: more than the nine digits reach.
const READING: u64 = 13;
/// Key D, no tab at all.
const EMPTY: u64 = 14;
/// Not a keyed group of any script.
const UNKNOWN: u64 = 99;

fn keyed(id: u64, key: LetterKey, tab_count: usize) -> KeyedGroup {
    KeyedGroup { id, key, tab_count }
}

fn keyed_groups() -> Vec<KeyedGroup> {
    vec![
        keyed(RESEARCH, LetterKey::S, 5),
        keyed(TRIP, LetterKey::A, 2),
        keyed(READING, LetterKey::E, 12),
        keyed(EMPTY, LetterKey::D, 0),
    ]
}

const fn group(id: u64) -> Target {
    Target::Group(id)
}

const fn tab(id: u64, index: usize) -> Target {
    Target::Tab { group: id, index }
}

const PLAIN: Focus = Focus {
    text_field: false,
    composing: false,
};
const TEXT: Focus = Focus {
    text_field: true,
    composing: false,
};
/// An IME composition in a text field.
const IME: Focus = Focus {
    text_field: true,
    composing: true,
};

const CAPTURES: [KeyCapture; 3] = [
    KeyCapture::OutsideTextOrHold,
    KeyCapture::OutsideText,
    KeyCapture::Always,
];

fn config(capture: KeyCapture) -> Config {
    Config::new(true, capture, ms(400))
}

fn default_config() -> Config {
    config(KeyCapture::OutsideTextOrHold)
}

fn disabled(capture: KeyCapture) -> Config {
    Config::new(false, capture, ms(400))
}

// Outcomes.
const PASS: Outcome = Outcome::PASS;
/// The key is consumed and nothing else happens.
const EAT: Outcome = Outcome {
    consume: true,
    activate: None,
    card: CardChange::None,
};
const EAT_AND_SHOW: Outcome = Outcome {
    consume: true,
    activate: None,
    card: CardChange::Show,
};
const EAT_AND_UPDATE: Outcome = Outcome {
    consume: true,
    activate: None,
    card: CardChange::Update,
};
const EAT_AND_HIDE: Outcome = Outcome {
    consume: true,
    activate: None,
    card: CardChange::Hide,
};
const SHOW: Outcome = Outcome {
    consume: false,
    activate: None,
    card: CardChange::Show,
};
const UPDATE: Outcome = Outcome {
    consume: false,
    activate: None,
    card: CardChange::Update,
};
const HIDE: Outcome = Outcome {
    consume: false,
    activate: None,
    card: CardChange::Hide,
};

/// A consumed key that switches at once.
const fn goto(target: Target) -> Outcome {
    Outcome {
        consume: true,
        activate: Some(target),
        card: CardChange::None,
    }
}

/// ⌥ came up with the card showing: it closes, and switches if something is selected.
const fn commit(target: Option<Target>) -> Outcome {
    Outcome {
        consume: false,
        activate: target,
        card: CardChange::Hide,
    }
}

// Views.
const HIDDEN: CardView = CardView {
    visible: false,
    selected_group: None,
    selected_tab: None,
};
/// The card is up and nothing is selected.
const BARE: CardView = CardView {
    visible: true,
    selected_group: None,
    selected_tab: None,
};

const fn card(group: u64, tab: Option<u8>) -> CardView {
    CardView {
        visible: true,
        selected_group: Some(group),
        selected_tab: tab,
    }
}

// What a step may say about the phase afterwards.
trait Phases: Copy {
    fn allows(self, got: Phase) -> bool;
    fn describe(self) -> String;
}

impl Phases for Phase {
    fn allows(self, got: Phase) -> bool {
        self == got
    }
    fn describe(self) -> String {
        format!("{self:?}")
    }
}

/// Two phases the brief allows ("Idle (or PassThrough while ⌥ is down)").
impl Phases for [Phase; 2] {
    fn allows(self, got: Phase) -> bool {
        self.contains(&got)
    }
    fn describe(self) -> String {
        format!("{:?} or {:?}", self[0], self[1])
    }
}

/// The brief does not say which phase follows.
#[derive(Clone, Copy)]
struct AnyPhase;
const ANY: AnyPhase = AnyPhase;

impl Phases for AnyPhase {
    fn allows(self, _: Phase) -> bool {
        true
    }
    fn describe(self) -> String {
        "any phase (the brief is silent)".to_owned()
    }
}

const IDLE_OR_PASS_THROUGH: [Phase; 2] = [Phase::Idle, Phase::PassThrough];

/// A machine and the script that is running on it.
struct Rig {
    keys: OptionKeys,
    name: String,
}

impl Rig {
    fn new(config: Config, focus: Focus) -> Rig {
        Rig::with_groups(config, focus, &keyed_groups())
    }

    fn with_groups(config: Config, focus: Focus, groups: &[KeyedGroup]) -> Rig {
        let mut keys = OptionKeys::new(config);
        assert_eq!(keys.set_groups(groups), PASS, "set_groups on a new machine");
        assert_eq!(keys.set_focus(focus), PASS, "set_focus on a new machine");
        let rig = Rig {
            keys,
            name: "rig".to_owned(),
        };
        rig.check_state("a new machine", Phase::Idle, HIDDEN);
        rig
    }

    fn named(mut self, name: &str) -> Rig {
        self.name = name.to_owned();
        self
    }

    /// The card is up (text field focus, the default hold): armed at 0, shown at 400.
    fn overlay(config: Config, focus: Focus) -> Rig {
        let mut rig = Rig::new(config, focus);
        rig.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        rig.poll(400, SHOW, Phase::Overlay, BARE);
        rig
    }

    #[track_caller]
    fn check_state(&self, what: &str, phase: impl Phases, view: CardView) {
        let got = self.keys.phase();
        assert!(
            phase.allows(got),
            "{}: after {what}: phase() is {got:?}, the brief says {}",
            self.name,
            phase.describe()
        );
        assert_eq!(
            self.keys.view(),
            view,
            "{}: after {what}: view()",
            self.name
        );
    }

    #[track_caller]
    fn verify(&self, what: &str, got: Outcome, want: Outcome, phase: impl Phases, view: CardView) {
        assert_eq!(got, want, "{}: {what}: the Outcome", self.name);
        self.check_state(what, phase, view);
    }

    /// One call of `on_event` and everything the brief says about it.
    #[track_caller]
    fn step(&mut self, at: u64, event: Event, want: Outcome, phase: impl Phases, view: CardView) {
        let got = self.keys.on_event(ms(at), event);
        self.verify(&format!("{at} ms {event:?}"), got, want, phase, view);
    }

    /// A key pressed (`want` is the answer to the press) and released five milliseconds later: the
    /// release is consumed if and only if the press was, and changes nothing.
    #[track_caller]
    fn tap(
        &mut self,
        at: u64,
        key: Key,
        m: Mods,
        want: Outcome,
        phase: impl Phases,
        view: CardView,
    ) {
        self.step(at, down(key, m), want, phase, view);
        let release = Outcome {
            consume: want.consume,
            ..PASS
        };
        self.step(at + 5, up(key, m), release, phase, view);
    }

    /// A pointer or card event: the brief does not say whether it is `consume`d, so only the
    /// activation, the card change, the phase and the view are checked.
    #[track_caller]
    fn click(
        &mut self,
        at: u64,
        event: Event,
        activate: Option<Target>,
        change: CardChange,
        phase: impl Phases,
        view: CardView,
    ) {
        let got = self.keys.on_event(ms(at), event);
        assert_eq!(
            got.activate, activate,
            "{}: {at} ms {event:?}: activate ({got:?})",
            self.name
        );
        assert_eq!(
            got.card, change,
            "{}: {at} ms {event:?}: card ({got:?})",
            self.name
        );
        self.check_state(&format!("{at} ms {event:?}"), phase, view);
    }

    #[track_caller]
    fn poll(&mut self, at: u64, want: Outcome, phase: impl Phases, view: CardView) {
        let got = self.keys.poll(ms(at));
        self.verify(&format!("poll({at} ms)"), got, want, phase, view);
    }

    #[track_caller]
    fn set_config(&mut self, config: Config, want: Outcome, phase: impl Phases, view: CardView) {
        let got = self.keys.set_config(config);
        self.verify(&format!("set_config({config:?})"), got, want, phase, view);
    }

    #[track_caller]
    fn set_groups(
        &mut self,
        groups: &[KeyedGroup],
        want: Outcome,
        phase: impl Phases,
        view: CardView,
    ) {
        let got = self.keys.set_groups(groups);
        self.verify(&format!("set_groups({groups:?})"), got, want, phase, view);
    }

    #[track_caller]
    fn set_focus(&mut self, focus: Focus, want: Outcome, phase: impl Phases, view: CardView) {
        let got = self.keys.set_focus(focus);
        self.verify(&format!("set_focus({focus:?})"), got, want, phase, view);
    }

    #[track_caller]
    fn deadline_is(&self, at: Option<u64>) {
        assert_eq!(
            self.keys.deadline(),
            at.map(ms),
            "{}: deadline()",
            self.name
        );
    }
}

const ALL_PHASES: [Phase; 5] = [
    Phase::Idle,
    Phase::Armed,
    Phase::PassThrough,
    Phase::Switched,
    Phase::Overlay,
];

/// A machine that is in `phase`, and a time shortly after it got there (before any hold deadline).
fn rig_in(phase: Phase) -> (Rig, u64) {
    let mut rig = Rig::new(default_config(), PLAIN).named(&format!("rig in {phase:?}"));
    match phase {
        Phase::Idle => {}
        Phase::Armed => rig.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN),
        Phase::PassThrough => {
            rig.set_focus(TEXT, PASS, Phase::Idle, HIDDEN);
            rig.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
            rig.tap(1010, S_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
        }
        Phase::Switched => {
            rig.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
            rig.tap(
                1010,
                S_KEY,
                ALT,
                goto(group(RESEARCH)),
                Phase::Switched,
                HIDDEN,
            );
        }
        Phase::Overlay => {
            rig.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
            rig.poll(1400, SHOW, Phase::Overlay, BARE);
            return (rig, 1500);
        }
    }
    (rig, 1100)
}

/// The modifiers that are down in `phase` of `rig_in`.
fn held(phase: Phase) -> Mods {
    if phase == Phase::Idle { NONE } else { ALT }
}

/// What the card shows in `phase` of `rig_in`.
fn shown(phase: Phase) -> CardView {
    if phase == Phase::Overlay {
        BARE
    } else {
        HIDDEN
    }
}

// ------------------------------------------------------------------------------------------
// SPEC §8, lane 11: the sentence, by name
// ------------------------------------------------------------------------------------------

/// "quick ⌥S in a text field passes through" (brief 5.4, Armed: a letter of a group in a text
/// field, capture is not Always: PassThrough, pass).
#[test]
fn quick_option_s_in_a_text_field_passes_through() {
    let mut r = Rig::new(default_config(), TEXT);
    r.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.deadline_is(Some(1400));
    r.step(1100, down(S_KEY, ALT), PASS, Phase::PassThrough, HIDDEN);
    r.step(1150, up(S_KEY, ALT), PASS, Phase::PassThrough, HIDDEN);
    r.deadline_is(None);
    // ⌥ is still down when the hold would have been over: this hold belongs to the text field.
    r.poll(1500, PASS, Phase::PassThrough, HIDDEN);
    r.step(1600, mods(NONE), PASS, Phase::Idle, HIDDEN);
    r.deadline_is(None);
}

/// "a 400 ms hold opens the card" (Armed: the deadline is reached with the hold path).
#[test]
fn a_400_ms_hold_opens_the_card() {
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.deadline_is(Some(400));
    r.poll(399, PASS, Phase::Armed, HIDDEN);
    r.poll(400, SHOW, Phase::Overlay, BARE);
    r.deadline_is(None);
    // `poll` shows it once; and there is no timeout, the card stays until ⌥ comes up.
    r.poll(401, PASS, Phase::Overlay, BARE);
    r.poll(60_000, PASS, Phase::Overlay, BARE);
    r.step(61_000, mods(NONE), HIDE, Phase::Idle, HIDDEN);
}

/// "letter then digit then ⌥-up commits" (Overlay: select a group, select a tab, ⌥ up).
#[test]
fn letter_then_digit_then_option_up_commits() {
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.poll(400, SHOW, Phase::Overlay, BARE);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        600,
        digit(3),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(3)),
    );
    r.step(
        800,
        mods(NONE),
        commit(Some(tab(RESEARCH, 2))),
        Phase::Idle,
        HIDDEN,
    );
    r.deadline_is(None);

    // The same without the digit: the group's most recently shown tab.
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.poll(400, SHOW, Phase::Overlay, BARE);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        800,
        mods(NONE),
        commit(Some(group(RESEARCH))),
        Phase::Idle,
        HIDDEN,
    );
}

/// "⌘⌥ ignored": ⌘ down first, then ⌥, then ⌘ up does not arm; ⌥ then ⌘ is the page's.
#[test]
fn command_option_is_ignored() {
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(COMMAND), PASS, Phase::Idle, HIDDEN);
    r.step(10, mods(with_alt(COMMAND)), PASS, Phase::Idle, HIDDEN);
    r.deadline_is(None);
    r.tap(20, S_KEY, with_alt(COMMAND), PASS, Phase::Idle, HIDDEN);
    r.poll(1000, PASS, Phase::Idle, HIDDEN);
    // ⌘ comes up while ⌥ stays down: that is not a new ⌥ press.
    r.step(1100, mods(ALT), PASS, Phase::Idle, HIDDEN);
    r.deadline_is(None);
    r.tap(1150, S_KEY, ALT, PASS, Phase::Idle, HIDDEN);
    r.poll(3000, PASS, Phase::Idle, HIDDEN);
    r.step(3100, mods(NONE), PASS, Phase::Idle, HIDDEN);
    r.step(3200, mods(ALT), PASS, Phase::Armed, HIDDEN);

    // ⌥ first, then ⌘: the hold is the page's from there on, even when ⌘ is released again.
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(
        50,
        mods(with_alt(COMMAND)),
        PASS,
        Phase::PassThrough,
        HIDDEN,
    );
    r.deadline_is(None);
    r.tap(
        100,
        S_KEY,
        with_alt(COMMAND),
        PASS,
        Phase::PassThrough,
        HIDDEN,
    );
    r.step(200, mods(ALT), PASS, Phase::PassThrough, HIDDEN);
    r.tap(250, S_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
    r.poll(2000, PASS, Phase::PassThrough, HIDDEN);
    r.step(2100, mods(NONE), PASS, Phase::Idle, HIDDEN);
}

/// "IME composition blocks capture": composing when ⌥ goes down, in every capture mode, and
/// composing that starts while ⌥ is already down.
#[test]
fn ime_composition_blocks_capture() {
    for capture in CAPTURES {
        // Composing before ⌥ goes down: the press is not armed, it is the IME's.
        let mut r = Rig::new(config(capture), IME).named(&format!("composing first, {capture:?}"));
        r.step(0, mods(ALT), PASS, Phase::PassThrough, HIDDEN);
        r.deadline_is(None);
        r.tap(100, S_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
        r.poll(2000, PASS, Phase::PassThrough, HIDDEN);
        r.step(2100, mods(NONE), PASS, Phase::Idle, HIDDEN);

        // The same outside a text field (a composition can be open in the address bar).
        let outside = Focus {
            text_field: false,
            composing: true,
        };
        let mut r =
            Rig::new(config(capture), outside).named(&format!("composing, no field, {capture:?}"));
        r.step(0, mods(ALT), PASS, Phase::PassThrough, HIDDEN);
        r.tap(100, S_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
        r.step(200, mods(NONE), PASS, Phase::Idle, HIDDEN);

        // Composing that opens while ⌥ is down (Armed) cancels the arming.
        let mut r =
            Rig::new(config(capture), TEXT).named(&format!("composing while armed, {capture:?}"));
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.set_focus(IME, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
        r.deadline_is(None);
        r.poll(400, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
        r.tap(500, S_KEY, ALT, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
        r.step(600, mods(NONE), PASS, Phase::Idle, HIDDEN);
    }
}

// ------------------------------------------------------------------------------------------
// Idle and Armed
// ------------------------------------------------------------------------------------------

/// Armed: `Modifiers` with ⌥ up gives Idle ("a lone ⌥ press does nothing").
#[test]
fn lone_option_press_does_nothing() {
    for focus in [PLAIN, TEXT] {
        let mut r = Rig::new(default_config(), focus);
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.step(100, mods(NONE), PASS, Phase::Idle, HIDDEN);
        r.deadline_is(None);
        r.poll(2000, PASS, Phase::Idle, HIDDEN);
        // And again: the second press arms afresh from its own time.
        r.step(3000, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.deadline_is(Some(3400));
    }
}

/// Armed becomes Overlay before the event is processed: a release after the deadline is seen as a
/// release of the card (hidden to visible to hidden is no change at all).
#[test]
fn a_late_lone_release_reports_no_card_change() {
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(900, mods(NONE), PASS, Phase::Idle, HIDDEN);
    r.deadline_is(None);
}

/// Armed: ⌘, ⌃ or ⇧ added: PassThrough, until ⌥ comes up.
#[test]
fn option_then_another_modifier_passes_through() {
    for (name, other) in [("shift", SHIFT), ("control", CONTROL), ("command", COMMAND)] {
        let both = with_alt(other);
        let mut r = Rig::new(default_config(), PLAIN).named(name);
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.step(50, mods(both), PASS, Phase::PassThrough, HIDDEN);
        r.deadline_is(None);
        r.poll(1000, PASS, Phase::PassThrough, HIDDEN);
        r.tap(1100, S_KEY, both, PASS, Phase::PassThrough, HIDDEN);
        // The other modifier is released, ⌥ is still down: still not Mercury's.
        r.step(1200, mods(ALT), PASS, Phase::PassThrough, HIDDEN);
        r.tap(1300, A_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
        r.step(1400, mods(NONE), PASS, Phase::Idle, HIDDEN);
    }
}

/// Idle: ⌥ newly down with ⌘, ⌃ or ⇧ already down stays Idle; releasing the other modifier later
/// is not a new ⌥ press (the edge is ⌥ going from up to down).
#[test]
fn another_modifier_down_first_then_option_does_not_arm() {
    for (name, other) in [("shift", SHIFT), ("control", CONTROL), ("command", COMMAND)] {
        let both = with_alt(other);
        let mut r = Rig::new(default_config(), PLAIN).named(name);
        r.step(0, mods(other), PASS, Phase::Idle, HIDDEN);
        r.step(10, mods(both), PASS, Phase::Idle, HIDDEN);
        r.deadline_is(None);
        r.tap(20, S_KEY, both, PASS, Phase::Idle, HIDDEN);
        r.poll(1000, PASS, Phase::Idle, HIDDEN);
        r.step(1100, mods(ALT), PASS, Phase::Idle, HIDDEN);
        r.deadline_is(None);
        r.tap(1150, S_KEY, ALT, PASS, Phase::Idle, HIDDEN);
        r.poll(3000, PASS, Phase::Idle, HIDDEN);
        r.step(3100, mods(NONE), PASS, Phase::Idle, HIDDEN);
        // A fresh press of ⌥ alone does arm.
        r.step(3200, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.deadline_is(Some(3600));
    }
}

/// "A key event first applies its own `mods` as an `Event::Modifiers` if they differ from the
/// tracked ones."
#[test]
fn a_key_event_applies_its_own_modifiers_first() {
    // A key that arrives with ⌥ and no `Modifiers` before it arms, then acts.
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(
        0,
        down(S_KEY, ALT),
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );

    // A key that arrives without ⌥ while Armed means ⌥ came up: a lone press, then an ordinary key.
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(100, down(S_KEY, NONE), PASS, Phase::Idle, HIDDEN);

    // With ⌘ in its own mods an Armed key is the page's.
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(
        100,
        down(S_KEY, with_alt(COMMAND)),
        PASS,
        Phase::PassThrough,
        HIDDEN,
    );

    // With the card up: a key that carries ⇧ closes it and is not consumed (it is the page's).
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        600,
        down(A_KEY, with_alt(SHIFT)),
        HIDE,
        Phase::PassThrough,
        HIDDEN,
    );

    // A key that carries no ⌥ commits the card (⌥ came up), and is then an ordinary key.
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        600,
        down(Q_KEY, NONE),
        commit(Some(group(RESEARCH))),
        Phase::Idle,
        HIDDEN,
    );

    // Not tested, brief ambiguous: a `Key::Modifier` event whose own `mods` differ from the
    // tracked ones ("a key event first applies its own mods" against "Modifier: never changes
    // the state"). The ⌥ key's release reaches the machine as `Event::Modifiers`.
}

// The three capture modes, in and out of a text field (Armed, a letter of a group).

#[test]
fn outside_text_or_hold_outside_a_text_field_switches_at_once() {
    let mut r = Rig::new(config(KeyCapture::OutsideTextOrHold), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(
        100,
        S_KEY,
        ALT,
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
    r.deadline_is(None);
    r.step(200, mods(NONE), PASS, Phase::Idle, HIDDEN);

    // The hold opens the card here too.
    r.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.poll(1400, SHOW, Phase::Overlay, BARE);
}

#[test]
fn outside_text_or_hold_in_a_text_field_passes_and_the_hold_opens_the_card() {
    let mut r = Rig::new(config(KeyCapture::OutsideTextOrHold), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(100, S_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
    r.step(200, mods(NONE), PASS, Phase::Idle, HIDDEN);

    r.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.poll(1400, SHOW, Phase::Overlay, BARE);
    r.tap(
        1500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        1600,
        mods(NONE),
        commit(Some(group(RESEARCH))),
        Phase::Idle,
        HIDDEN,
    );
}

#[test]
fn outside_text_outside_a_text_field_switches_at_once_even_after_a_long_hold() {
    let mut r = Rig::new(config(KeyCapture::OutsideText), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(
        100,
        S_KEY,
        ALT,
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
    r.step(200, mods(NONE), PASS, Phase::Idle, HIDDEN);

    // No hold path in this mode: no deadline, no card, however long ⌥ is held.
    r.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.deadline_is(None);
    r.poll(5000, PASS, Phase::Armed, HIDDEN);
    r.tap(
        5100,
        S_KEY,
        ALT,
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
}

#[test]
fn outside_text_in_a_text_field_never_captures_and_never_shows_the_card() {
    let mut r = Rig::new(config(KeyCapture::OutsideText), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.deadline_is(None);
    r.tap(100, S_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
    r.step(200, mods(NONE), PASS, Phase::Idle, HIDDEN);

    r.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.deadline_is(None);
    r.poll(9000, PASS, Phase::Armed, HIDDEN);
    r.tap(9100, S_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
    r.step(9200, mods(NONE), PASS, Phase::Idle, HIDDEN);
}

#[test]
fn always_outside_a_text_field_switches_at_once_and_the_hold_opens_the_card() {
    let mut r = Rig::new(config(KeyCapture::Always), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(
        100,
        S_KEY,
        ALT,
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
    r.step(200, mods(NONE), PASS, Phase::Idle, HIDDEN);

    r.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.deadline_is(Some(1400));
    r.poll(1400, SHOW, Phase::Overlay, BARE);
}

#[test]
fn always_in_a_text_field_captures_too() {
    let mut r = Rig::new(config(KeyCapture::Always), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(
        100,
        S_KEY,
        ALT,
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
    r.step(200, mods(NONE), PASS, Phase::Idle, HIDDEN);

    r.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.poll(1400, SHOW, Phase::Overlay, BARE);
    r.tap(
        1500,
        A_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(TRIP, None),
    );
}

/// ⌥E starts an accent: a group on a dead-key letter is the text field's, and so is the vowel
/// that follows.
#[test]
fn an_assigned_dead_key_letter_in_a_text_field_passes_through() {
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(100, E_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
    // ⌥ released, then the vowel: ordinary typing.
    r.step(200, mods(NONE), PASS, Phase::Idle, HIDDEN);
    r.tap(300, E_KEY, NONE, PASS, Phase::Idle, HIDDEN);

    // ⌥ kept down for the vowel: the vowel is still the text field's, and the card never opens.
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(100, E_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
    r.tap(200, A_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
    r.poll(2000, PASS, Phase::PassThrough, HIDDEN);
    r.step(2100, mods(NONE), PASS, Phase::Idle, HIDDEN);

    // The same letter outside a text field is Mercury's.
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(
        100,
        E_KEY,
        ALT,
        goto(group(READING)),
        Phase::Switched,
        HIDDEN,
    );
}

/// Armed: any other key is the page's, and so is everything after it.
#[test]
fn an_unassigned_letter_passes_through() {
    for focus in [PLAIN, TEXT] {
        let mut r = Rig::new(default_config(), focus);
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.tap(50, Q_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
        r.deadline_is(None);
        // PassThrough: a letter that is assigned is the page's now.
        r.tap(100, S_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
        r.poll(2000, PASS, Phase::PassThrough, HIDDEN);
        r.step(2100, mods(NONE), PASS, Phase::Idle, HIDDEN);
    }
}

/// Armed: a digit, Escape, an arrow or any other key passes (a digit before any group is chosen
/// is not Mercury's); `Digit` outside 1..=9 behaves as `Other`.
#[test]
fn a_digit_escape_arrow_or_other_key_in_armed_passes_through() {
    let keys = [
        digit(1),
        digit(3),
        digit(9),
        digit(0),
        digit(10),
        Key::Escape,
        Key::Arrow,
        Key::Other(7),
    ];
    for key in keys {
        let mut r = Rig::new(default_config(), PLAIN).named(&format!("{key:?}"));
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.step(50, down(key, ALT), PASS, Phase::PassThrough, HIDDEN);
        r.deadline_is(None);
        r.step(60, up(key, ALT), PASS, Phase::PassThrough, HIDDEN);
        r.step(100, mods(NONE), PASS, Phase::Idle, HIDDEN);
    }
}

/// Armed: the modifier keys themselves, auto-repeat included, pass and change nothing (not even
/// the time the hold started).
#[test]
fn a_modifier_key_in_armed_changes_nothing() {
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(50, down(Key::Modifier, ALT), PASS, Phase::Armed, HIDDEN);
    r.step(100, again(Key::Modifier, ALT), PASS, Phase::Armed, HIDDEN);
    r.step(150, up(Key::Modifier, ALT), PASS, Phase::Armed, HIDDEN);
    r.deadline_is(Some(400));
    r.poll(399, PASS, Phase::Armed, HIDDEN);
    r.poll(400, SHOW, Phase::Overlay, BARE);
}

/// Armed: `PointerDown` gives PassThrough; `Dismiss`, `TileClicked` and `ChipClicked` are ignored.
#[test]
fn pointer_events_in_armed() {
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.click(
        50,
        Event::PointerDown,
        None,
        CardChange::None,
        Phase::PassThrough,
        HIDDEN,
    );
    r.poll(1000, PASS, Phase::PassThrough, HIDDEN);

    let ignored = [
        Event::Dismiss,
        Event::TileClicked { group: RESEARCH },
        Event::ChipClicked { index: 0 },
    ];
    for event in ignored {
        let mut r = Rig::new(default_config(), PLAIN).named(&format!("{event:?}"));
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.click(50, event, None, CardChange::None, Phase::Armed, HIDDEN);
        r.deadline_is(Some(400));
        r.poll(400, SHOW, Phase::Overlay, BARE);
    }
}

/// PassThrough: everything passes; only ⌥ up gives Idle.
#[test]
fn pass_through_ends_only_when_option_comes_up() {
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(10, S_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
    for key in [
        Key::Escape,
        Key::Arrow,
        digit(2),
        Q_KEY,
        A_KEY,
        Key::Other(3),
        Key::Modifier,
    ] {
        r.tap(100, key, ALT, PASS, Phase::PassThrough, HIDDEN);
    }
    r.click(
        150,
        Event::PointerDown,
        None,
        CardChange::None,
        Phase::PassThrough,
        HIDDEN,
    );
    r.click(
        160,
        Event::Dismiss,
        None,
        CardChange::None,
        Phase::PassThrough,
        HIDDEN,
    );
    r.step(200, mods(with_alt(SHIFT)), PASS, Phase::PassThrough, HIDDEN);
    r.step(250, mods(ALT), PASS, Phase::PassThrough, HIDDEN);
    r.poll(5000, PASS, Phase::PassThrough, HIDDEN);
    r.step(5100, mods(NONE), PASS, Phase::Idle, HIDDEN);
    r.deadline_is(None);
}

/// "No hold path with no keyed group": nothing to pick, so no card and no deadline.
#[test]
fn no_hold_path_with_no_keyed_group() {
    for focus in [PLAIN, TEXT] {
        for capture in [KeyCapture::OutsideTextOrHold, KeyCapture::Always] {
            let mut r = Rig::with_groups(config(capture), focus, &[])
                .named(&format!("{capture:?}, {focus:?}"));
            r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
            r.deadline_is(None);
            r.poll(10_000, PASS, Phase::Armed, HIDDEN);
            // No group owns S: an unassigned letter.
            r.tap(10_100, S_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);
            r.step(10_200, mods(NONE), PASS, Phase::Idle, HIDDEN);
        }
    }
}

// ------------------------------------------------------------------------------------------
// The hold: boundaries, clamps, the lazy deadline, time
// ------------------------------------------------------------------------------------------

#[test]
fn hold_boundary_399_nothing_400_shows() {
    let mut r = Rig::new(default_config(), TEXT);
    r.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.deadline_is(Some(1400));
    r.poll(1399, PASS, Phase::Armed, HIDDEN);
    r.poll(1400, SHOW, Phase::Overlay, BARE);

    // Through an event instead of a poll: 399 ms is a quick tap, 400 ms is a hold.
    let mut r = Rig::new(default_config(), TEXT);
    r.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(1399, down(S_KEY, ALT), PASS, Phase::PassThrough, HIDDEN);

    let mut r = Rig::new(default_config(), TEXT);
    r.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(
        1400,
        down(S_KEY, ALT),
        EAT_AND_SHOW,
        Phase::Overlay,
        card(RESEARCH, None),
    );
}

/// The hold of the settings is 250 to 1000 ms; `Config::new` clamps what is outside.
#[test]
fn hold_boundaries_at_the_clamps_250_and_1000() {
    for (given, effective) in [(0, 250), (100, 250), (250, 250), (1000, 1000), (5000, 1000)] {
        let wanted = Config::new(true, KeyCapture::OutsideTextOrHold, ms(given));

        // Through `new`.
        let mut r = Rig::new(wanted, TEXT).named(&format!("new, hold {given}"));
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.deadline_is(Some(effective));
        r.poll(effective - 1, PASS, Phase::Armed, HIDDEN);
        r.poll(effective, SHOW, Phase::Overlay, BARE);

        // Through `set_config`.
        let mut r = Rig::new(default_config(), TEXT).named(&format!("set_config, hold {given}"));
        r.set_config(wanted, PASS, Phase::Idle, HIDDEN);
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.deadline_is(Some(effective));
        r.poll(effective - 1, PASS, Phase::Armed, HIDDEN);
        r.poll(effective, SHOW, Phase::Overlay, BARE);
    }
}

/// "Armed and `now >= t0 + hold` becomes Overlay before the event is processed, so a timer that
/// fires late never changes what a key does": a key after the deadline acts as in the Overlay
/// although no `poll` ran.
#[test]
fn the_deadline_is_applied_lazily_before_a_key() {
    for focus in [TEXT, PLAIN] {
        // A letter: a selection, not a quick tap and not an immediate switch.
        let mut r = Rig::new(default_config(), focus).named(&format!("letter, {focus:?}"));
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.step(
            500,
            down(S_KEY, ALT),
            EAT_AND_SHOW,
            Phase::Overlay,
            card(RESEARCH, None),
        );
        r.deadline_is(None);
        r.step(
            520,
            up(S_KEY, ALT),
            EAT,
            Phase::Overlay,
            card(RESEARCH, None),
        );
        r.tap(
            600,
            digit(2),
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(RESEARCH, Some(2)),
        );
        r.step(
            700,
            mods(NONE),
            commit(Some(tab(RESEARCH, 1))),
            Phase::Idle,
            HIDDEN,
        );

        // Escape closes the card the lazy open has just shown: consumed, and not a net change.
        let mut r = Rig::new(default_config(), focus).named(&format!("escape, {focus:?}"));
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.step(500, down(Key::Escape, ALT), EAT, Phase::Idle, HIDDEN);

        // An unassigned letter is consumed in the Overlay (before the deadline it would pass).
        let mut r = Rig::new(default_config(), focus).named(&format!("unassigned, {focus:?}"));
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.step(500, down(Q_KEY, ALT), EAT_AND_SHOW, Phase::Overlay, BARE);

        // A poll after the late key finds nothing more to do.
        r.poll(600, PASS, Phase::Overlay, BARE);
    }

    // Without a hold path the deadline does not exist, so the same key is an immediate switch.
    let mut r = Rig::new(config(KeyCapture::OutsideText), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(
        500,
        down(S_KEY, ALT),
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
}

/// "Time that goes backwards counts as not having moved."
#[test]
fn time_going_backwards_counts_as_not_having_moved() {
    let mut r = Rig::new(default_config(), TEXT);
    r.step(1000, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.poll(300, PASS, Phase::Armed, HIDDEN);
    r.deadline_is(Some(1400));
    r.poll(1399, PASS, Phase::Armed, HIDDEN);
    r.poll(900, PASS, Phase::Armed, HIDDEN);
    r.poll(1400, SHOW, Phase::Overlay, BARE);
    // An earlier time with the card up: the card does not mind.
    r.tap(
        10,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );

    // ⌥ goes down "before" the last call: the clock is where it was, so the hold starts there.
    let mut r = Rig::new(default_config(), TEXT);
    r.poll(1000, PASS, Phase::Idle, HIDDEN);
    r.step(600, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.deadline_is(Some(1400));
    r.poll(1399, PASS, Phase::Armed, HIDDEN);
    r.poll(1400, SHOW, Phase::Overlay, BARE);
}

// ------------------------------------------------------------------------------------------
// Switched
// ------------------------------------------------------------------------------------------

/// D5: after an immediate switch a digit opens that group's tab at once, another group's letter
/// switches again, nothing shows.
#[test]
fn the_immediate_switch_then_a_digit_is_switched() {
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(
        50,
        S_KEY,
        ALT,
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
    r.deadline_is(None);
    r.tap(
        150,
        digit(2),
        ALT,
        goto(tab(RESEARCH, 1)),
        Phase::Switched,
        HIDDEN,
    );
    r.tap(
        250,
        digit(5),
        ALT,
        goto(tab(RESEARCH, 4)),
        Phase::Switched,
        HIDDEN,
    );
    // RESEARCH has five tabs: 6 is consumed and goes nowhere.
    r.tap(350, digit(6), ALT, EAT, Phase::Switched, HIDDEN);
    // Another group's letter switches again, and the digits are that group's now.
    r.tap(450, A_KEY, ALT, goto(group(TRIP)), Phase::Switched, HIDDEN);
    r.tap(
        550,
        digit(2),
        ALT,
        goto(tab(TRIP, 1)),
        Phase::Switched,
        HIDDEN,
    );
    r.tap(650, digit(3), ALT, EAT, Phase::Switched, HIDDEN);
    // However long ⌥ is held: no card, no deadline.
    r.poll(5000, PASS, Phase::Switched, HIDDEN);
    r.deadline_is(None);
    r.step(5100, mods(NONE), PASS, Phase::Idle, HIDDEN);
    r.deadline_is(None);
}

/// Switched: a group with no tab consumes the digit and goes nowhere.
#[test]
fn a_digit_after_a_group_with_no_tab_goes_nowhere() {
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(
        50,
        Key::Letter(LetterKey::D),
        ALT,
        goto(group(EMPTY)),
        Phase::Switched,
        HIDDEN,
    );
    r.tap(150, digit(1), ALT, EAT, Phase::Switched, HIDDEN);
}

/// Switched: unassigned letters, Escape, arrows and other keys pass and change nothing; a digit
/// outside 1..=9 is an `Other`.
#[test]
fn switched_passes_the_keys_it_does_not_own() {
    let keys = [
        Q_KEY,
        Key::Escape,
        Key::Arrow,
        Key::Other(7),
        digit(0),
        digit(10),
        Key::Modifier,
    ];
    for key in keys {
        let mut r = Rig::new(default_config(), PLAIN).named(&format!("{key:?}"));
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.tap(
            50,
            S_KEY,
            ALT,
            goto(group(RESEARCH)),
            Phase::Switched,
            HIDDEN,
        );
        r.tap(100, key, ALT, PASS, Phase::Switched, HIDDEN);
        // Still Switched(RESEARCH): the digit still opens its tab.
        r.tap(
            150,
            digit(1),
            ALT,
            goto(tab(RESEARCH, 0)),
            Phase::Switched,
            HIDDEN,
        );
    }
}

/// Switched: ⌥ up gives Idle; ⌘, ⌃ or ⇧ added gives PassThrough.
#[test]
fn switched_ends_when_option_comes_up_or_another_modifier_joins() {
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(
        50,
        S_KEY,
        ALT,
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
    r.step(100, mods(NONE), PASS, Phase::Idle, HIDDEN);
    r.tap(150, digit(2), NONE, PASS, Phase::Idle, HIDDEN);

    for (name, other) in [("shift", SHIFT), ("control", CONTROL), ("command", COMMAND)] {
        let mut r = Rig::new(default_config(), PLAIN).named(name);
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.tap(
            50,
            S_KEY,
            ALT,
            goto(group(RESEARCH)),
            Phase::Switched,
            HIDDEN,
        );
        r.step(100, mods(with_alt(other)), PASS, Phase::PassThrough, HIDDEN);
        r.tap(
            150,
            digit(2),
            with_alt(other),
            PASS,
            Phase::PassThrough,
            HIDDEN,
        );
        r.step(200, mods(NONE), PASS, Phase::Idle, HIDDEN);
    }
}

/// Digit `n` (1..=9) is the tab with index `n - 1`: through the immediate switch and through the
/// card, with a group of 12 tabs (so the nine digits are all in range).
#[test]
fn digits_one_to_nine_map_to_tab_index_n_minus_one() {
    for n in 1..=9u8 {
        let index = usize::from(n) - 1;

        let mut r = Rig::new(default_config(), PLAIN).named(&format!("immediate, digit {n}"));
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.tap(
            10,
            E_KEY,
            ALT,
            goto(group(READING)),
            Phase::Switched,
            HIDDEN,
        );
        r.tap(
            30,
            digit(n),
            ALT,
            goto(tab(READING, index)),
            Phase::Switched,
            HIDDEN,
        );

        let mut r = Rig::overlay(default_config(), TEXT).named(&format!("card, digit {n}"));
        r.tap(
            500,
            E_KEY,
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(READING, None),
        );
        r.tap(
            520,
            digit(n),
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(READING, Some(n)),
        );
        r.step(
            600,
            mods(NONE),
            commit(Some(tab(READING, index))),
            Phase::Idle,
            HIDDEN,
        );
    }
}

// ------------------------------------------------------------------------------------------
// Overlay
// ------------------------------------------------------------------------------------------

/// Overlay: a letter of a group selects it and clears the tab; the same letter again clears the tab
/// and, with nothing to clear, changes nothing.
#[test]
fn in_the_card_a_letter_selects_the_group_and_clears_the_tab() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        600,
        digit(3),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(3)),
    );
    r.tap(
        700,
        A_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(TRIP, None),
    );
    r.tap(
        800,
        digit(2),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(TRIP, Some(2)),
    );
    // The same letter again: the group stays, the tab goes.
    r.tap(
        900,
        A_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(TRIP, None),
    );
    // And again: the view is the same, so no update.
    r.tap(1000, A_KEY, ALT, EAT, Phase::Overlay, card(TRIP, None));
    r.step(
        1100,
        mods(NONE),
        commit(Some(group(TRIP))),
        Phase::Idle,
        HIDDEN,
    );
}

/// "A `Digit` before any group is chosen": consumed, nothing selected.
#[test]
fn in_the_card_a_digit_before_any_group_is_chosen_is_consumed_without_effect() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(500, digit(3), ALT, EAT, Phase::Overlay, BARE);
    r.tap(600, digit(1), ALT, EAT, Phase::Overlay, BARE);
    r.step(700, mods(NONE), HIDE, Phase::Idle, HIDDEN);
}

/// "A digit above the group's `tab_count`": consumed, and the selection stays as it was.
#[test]
fn in_the_card_a_digit_above_the_tab_count_has_no_effect() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        A_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(TRIP, None),
    );
    // TRIP has two tabs.
    r.tap(600, digit(3), ALT, EAT, Phase::Overlay, card(TRIP, None));
    r.tap(700, digit(9), ALT, EAT, Phase::Overlay, card(TRIP, None));
    r.tap(
        800,
        digit(2),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(TRIP, Some(2)),
    );
    // A digit that is too high does not clear the tab that is selected.
    r.tap(900, digit(3), ALT, EAT, Phase::Overlay, card(TRIP, Some(2)));
    r.step(
        1000,
        mods(NONE),
        commit(Some(tab(TRIP, 1))),
        Phase::Idle,
        HIDDEN,
    );

    // A group with no tab: every digit is too high; ⌥ up still picks the group.
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        Key::Letter(LetterKey::D),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(EMPTY, None),
    );
    r.tap(600, digit(1), ALT, EAT, Phase::Overlay, card(EMPTY, None));
    r.step(
        700,
        mods(NONE),
        commit(Some(group(EMPTY))),
        Phase::Idle,
        HIDDEN,
    );

    // A group of more than nine tabs: digits stop at 9.
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        E_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(READING, None),
    );
    r.tap(
        600,
        digit(9),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(READING, Some(9)),
    );
    r.tap(
        700,
        digit(10),
        ALT,
        EAT,
        Phase::Overlay,
        card(READING, Some(9)),
    );
}

/// Overlay: an unassigned letter, `Other` and a `Digit` outside 1..=9 are consumed with no effect
/// (the page behind must not get å or a word delete).
#[test]
fn in_the_card_unassigned_keys_are_consumed_without_effect() {
    let keys = [
        Q_KEY,
        Key::Other(7),
        Key::Other(0),
        digit(0),
        digit(10),
        digit(255),
    ];
    for key in keys {
        let mut r = Rig::overlay(default_config(), TEXT).named(&format!("{key:?}"));
        r.tap(500, key, ALT, EAT, Phase::Overlay, BARE);
        r.tap(
            600,
            S_KEY,
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(RESEARCH, None),
        );
        r.tap(
            700,
            digit(3),
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(RESEARCH, Some(3)),
        );
        r.tap(800, key, ALT, EAT, Phase::Overlay, card(RESEARCH, Some(3)));
        r.step(
            900,
            mods(NONE),
            commit(Some(tab(RESEARCH, 2))),
            Phase::Idle,
            HIDDEN,
        );
    }
}

/// Overlay: arrows and the modifier keys pass (never captured) and change nothing.
#[test]
fn in_the_card_arrows_and_modifier_keys_pass() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        600,
        Key::Arrow,
        ALT,
        PASS,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        650,
        again(Key::Arrow, ALT),
        PASS,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        700,
        Key::Modifier,
        ALT,
        PASS,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        750,
        again(Key::Modifier, ALT),
        PASS,
        Phase::Overlay,
        card(RESEARCH, None),
    );
}

/// Overlay: Escape is consumed, gives Idle and `Hide`, and nothing changes: no switch.
#[test]
fn escape_closes_the_card_and_changes_nothing() {
    for selection in [None, Some(None), Some(Some(3))] {
        let mut r = Rig::overlay(default_config(), TEXT).named(&format!("{selection:?}"));
        if let Some(tab_digit) = selection {
            r.tap(
                500,
                S_KEY,
                ALT,
                EAT_AND_UPDATE,
                Phase::Overlay,
                card(RESEARCH, None),
            );
            if let Some(n) = tab_digit {
                r.tap(
                    520,
                    digit(n),
                    ALT,
                    EAT_AND_UPDATE,
                    Phase::Overlay,
                    card(RESEARCH, Some(n)),
                );
            }
        }
        r.step(
            600,
            down(Key::Escape, ALT),
            EAT_AND_HIDE,
            Phase::Idle,
            HIDDEN,
        );
        r.step(610, up(Key::Escape, ALT), EAT, Phase::Idle, HIDDEN);
        r.deadline_is(None);
        // ⌥ comes up: nothing to commit.
        r.step(700, mods(NONE), PASS, Phase::Idle, HIDDEN);
    }
}

/// D10: after Escape (or a click) the machine is Idle with ⌥ still down; nothing re-arms until ⌥
/// goes up and down again.
#[test]
fn after_escape_nothing_arms_again_until_option_is_pressed_again() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.step(
        600,
        down(Key::Escape, ALT),
        EAT_AND_HIDE,
        Phase::Idle,
        HIDDEN,
    );
    r.step(610, up(Key::Escape, ALT), EAT, Phase::Idle, HIDDEN);
    // The same state again is no press; and a long hold is no card.
    r.step(700, mods(ALT), PASS, Phase::Idle, HIDDEN);
    r.deadline_is(None);
    r.poll(5000, PASS, Phase::Idle, HIDDEN);
    r.tap(5100, S_KEY, ALT, PASS, Phase::Idle, HIDDEN);
    // A new press does arm.
    r.step(6000, mods(NONE), PASS, Phase::Idle, HIDDEN);
    r.step(6100, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.deadline_is(Some(6500));
}

/// Overlay: ⌥ up commits what is selected: a tab, else the group, else nothing.
#[test]
fn in_the_card_option_up_commits_the_tab_else_the_group_else_nothing() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.step(600, mods(NONE), commit(None), Phase::Idle, HIDDEN);

    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        600,
        mods(NONE),
        commit(Some(group(RESEARCH))),
        Phase::Idle,
        HIDDEN,
    );

    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        520,
        digit(5),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(5)),
    );
    r.step(
        600,
        mods(NONE),
        commit(Some(tab(RESEARCH, 4))),
        Phase::Idle,
        HIDDEN,
    );
    r.deadline_is(None);
}

/// Overlay: ⌥ up "wins over any other change in the same event": ⌥ released while ⌘, ⌃ or ⇧ is
/// pressed in the same `Modifiers` event still commits.
#[test]
fn option_up_wins_over_another_modifier_changing_in_the_same_event() {
    for (name, other) in [("shift", SHIFT), ("control", CONTROL), ("command", COMMAND)] {
        let mut r = Rig::overlay(default_config(), TEXT).named(name);
        r.tap(
            500,
            S_KEY,
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(RESEARCH, None),
        );
        r.tap(
            520,
            digit(3),
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(RESEARCH, Some(3)),
        );
        r.step(
            600,
            mods(other),
            commit(Some(tab(RESEARCH, 2))),
            Phase::Idle,
            HIDDEN,
        );
    }
}

/// Overlay: ⌘, ⌃ or ⇧ added with ⌥ still down: PassThrough, `Hide`, nothing switches.
#[test]
fn another_modifier_added_while_the_card_is_up_hides_it_without_acting() {
    for (name, other) in [("shift", SHIFT), ("control", CONTROL), ("command", COMMAND)] {
        let both = with_alt(other);
        let mut r = Rig::overlay(default_config(), TEXT).named(name);
        r.tap(
            500,
            S_KEY,
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(RESEARCH, None),
        );
        r.tap(
            520,
            digit(3),
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(RESEARCH, Some(3)),
        );
        r.step(600, mods(both), HIDE, Phase::PassThrough, HIDDEN);
        r.deadline_is(None);
        r.tap(650, A_KEY, both, PASS, Phase::PassThrough, HIDDEN);
        r.step(700, mods(ALT), PASS, Phase::PassThrough, HIDDEN);
        r.step(800, mods(NONE), PASS, Phase::Idle, HIDDEN);
    }
}

/// Overlay: `PointerDown` and `Dismiss` close the card and change nothing; ⌥ up afterwards
/// switches nowhere.
#[test]
fn pointer_down_and_dismiss_close_the_card_without_acting() {
    for event in [Event::PointerDown, Event::Dismiss] {
        let mut r = Rig::overlay(default_config(), TEXT).named(&format!("{event:?}"));
        r.tap(
            500,
            S_KEY,
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(RESEARCH, None),
        );
        r.tap(
            520,
            digit(3),
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(RESEARCH, Some(3)),
        );
        r.click(600, event, None, CardChange::Hide, Phase::Idle, HIDDEN);
        r.deadline_is(None);
        r.step(700, mods(NONE), PASS, Phase::Idle, HIDDEN);
    }
}

/// Overlay: `TileClicked` of a keyed group switches to the group (not a selected tab), closes the
/// card; an unknown group is ignored.
#[test]
fn a_tile_click_switches_to_that_group() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        520,
        digit(3),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(3)),
    );
    r.click(
        600,
        Event::TileClicked { group: TRIP },
        Some(group(TRIP)),
        CardChange::Hide,
        Phase::Idle,
        HIDDEN,
    );
    // ⌥ up afterwards has nothing left to commit.
    r.step(700, mods(NONE), PASS, Phase::Idle, HIDDEN);

    // A group with no tab is a group all the same.
    let mut r = Rig::overlay(default_config(), TEXT);
    r.click(
        500,
        Event::TileClicked { group: EMPTY },
        Some(group(EMPTY)),
        CardChange::Hide,
        Phase::Idle,
        HIDDEN,
    );
}

#[test]
fn a_tile_click_on_an_unknown_group_is_ignored() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.click(
        500,
        Event::TileClicked { group: UNKNOWN },
        None,
        CardChange::None,
        Phase::Overlay,
        BARE,
    );
    r.tap(
        550,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.click(
        600,
        Event::TileClicked { group: UNKNOWN },
        None,
        CardChange::None,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        700,
        mods(NONE),
        commit(Some(group(RESEARCH))),
        Phase::Idle,
        HIDDEN,
    );
}

/// Overlay: `ChipClicked` with a group selected switches to its tab.
#[test]
fn a_chip_click_switches_to_that_tab() {
    for index in [0, 1, 4] {
        let mut r = Rig::overlay(default_config(), TEXT).named(&format!("chip {index}"));
        r.tap(
            500,
            S_KEY,
            ALT,
            EAT_AND_UPDATE,
            Phase::Overlay,
            card(RESEARCH, None),
        );
        r.click(
            600,
            Event::ChipClicked { index },
            Some(tab(RESEARCH, index)),
            CardChange::Hide,
            Phase::Idle,
            HIDDEN,
        );
        r.step(700, mods(NONE), PASS, Phase::Idle, HIDDEN);
    }

    // The selected digit does not matter: the chip says which tab.
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        520,
        digit(2),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(2)),
    );
    r.click(
        600,
        Event::ChipClicked { index: 3 },
        Some(tab(RESEARCH, 3)),
        CardChange::Hide,
        Phase::Idle,
        HIDDEN,
    );

    // The ninth chip is the last one, in a group with more than nine tabs.
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        E_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(READING, None),
    );
    r.click(
        600,
        Event::ChipClicked { index: 8 },
        Some(tab(READING, 8)),
        CardChange::Hide,
        Phase::Idle,
        HIDDEN,
    );
}

/// `ChipClicked` above `tab_count` or at 9 or more is ignored; with no group selected there is
/// no tab to switch to.
#[test]
fn a_chip_click_above_the_tab_count_or_nine_is_ignored() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    // RESEARCH has five tabs: index 5 is the sixth.
    for index in [5, 6, 100, usize::MAX] {
        r.click(
            600,
            Event::ChipClicked { index },
            None,
            CardChange::None,
            Phase::Overlay,
            card(RESEARCH, None),
        );
    }
    r.step(
        700,
        mods(NONE),
        commit(Some(group(RESEARCH))),
        Phase::Idle,
        HIDDEN,
    );

    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        E_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(READING, None),
    );
    // READING has twelve tabs, but only nine can be picked: index 9 is the tenth.
    for index in [9, 10, 11] {
        r.click(
            600,
            Event::ChipClicked { index },
            None,
            CardChange::None,
            Phase::Overlay,
            card(READING, None),
        );
    }

    // A group with no tab: no chip can be right.
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        Key::Letter(LetterKey::D),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(EMPTY, None),
    );
    r.click(
        600,
        Event::ChipClicked { index: 0 },
        None,
        CardChange::None,
        Phase::Overlay,
        card(EMPTY, None),
    );

    // No group selected: nothing to switch to.
    let mut r = Rig::overlay(default_config(), TEXT);
    let got = r.keys.on_event(ms(500), Event::ChipClicked { index: 0 });
    assert_eq!(got.activate, None, "a chip with no group selected");
}

/// Overlay: `set_groups` clears a selected group that no longer exists, or a selected digit above
/// the new `tab_count`.
#[test]
fn set_groups_clears_a_selection_that_no_longer_exists() {
    let without_research = [
        keyed(TRIP, LetterKey::A, 2),
        keyed(READING, LetterKey::E, 12),
        keyed(EMPTY, LetterKey::D, 0),
    ];
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        520,
        digit(3),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(3)),
    );
    r.set_groups(&without_research, UPDATE, Phase::Overlay, BARE);
    r.step(700, mods(NONE), commit(None), Phase::Idle, HIDDEN);

    // The digit is too high for the new tab count: the tab goes, the group stays.
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        520,
        digit(3),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(3)),
    );
    let shorter = [
        keyed(RESEARCH, LetterKey::S, 2),
        keyed(TRIP, LetterKey::A, 2),
    ];
    r.set_groups(&shorter, UPDATE, Phase::Overlay, card(RESEARCH, None));
    r.step(
        700,
        mods(NONE),
        commit(Some(group(RESEARCH))),
        Phase::Idle,
        HIDDEN,
    );

    // A change that leaves the selection valid is no update: same list, a longer group, a
    // group exactly as long as the digit, another group gone, a group added.
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        520,
        digit(3),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(3)),
    );
    let selected = card(RESEARCH, Some(3));
    r.set_groups(&keyed_groups(), PASS, Phase::Overlay, selected);
    r.set_groups(
        &[keyed(RESEARCH, LetterKey::S, 9)],
        PASS,
        Phase::Overlay,
        selected,
    );
    r.set_groups(
        &[keyed(RESEARCH, LetterKey::S, 3)],
        PASS,
        Phase::Overlay,
        selected,
    );
    r.set_groups(
        &[
            keyed(RESEARCH, LetterKey::S, 3),
            keyed(TRIP, LetterKey::A, 2),
        ],
        PASS,
        Phase::Overlay,
        selected,
    );
    r.set_groups(
        &[
            keyed(RESEARCH, LetterKey::S, 3),
            keyed(TRIP, LetterKey::A, 2),
            keyed(30, LetterKey::Z, 4),
        ],
        PASS,
        Phase::Overlay,
        selected,
    );
    r.step(
        700,
        mods(NONE),
        commit(Some(tab(RESEARCH, 2))),
        Phase::Idle,
        HIDDEN,
    );
}

/// "`set_groups` keeps the first group for a duplicated letter and at most 26 groups."
#[test]
fn set_groups_keeps_the_first_group_of_a_duplicated_letter() {
    let groups = [
        keyed(21, LetterKey::S, 3),
        keyed(22, LetterKey::S, 7),
        keyed(23, LetterKey::A, 2),
    ];
    let mut r = Rig::with_groups(default_config(), PLAIN, &groups);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.tap(50, S_KEY, ALT, goto(group(21)), Phase::Switched, HIDDEN);
    // The first group has three tabs (the second would have seven).
    r.tap(100, digit(7), ALT, EAT, Phase::Switched, HIDDEN);
    r.tap(
        150,
        digit(3),
        ALT,
        goto(tab(21, 2)),
        Phase::Switched,
        HIDDEN,
    );

    let mut r = Rig::with_groups(default_config(), TEXT, &groups);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.poll(400, SHOW, Phase::Overlay, BARE);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(21, None),
    );
    r.tap(600, digit(7), ALT, EAT, Phase::Overlay, card(21, None));

    // One group per letter, 26 of them, and four more that repeat a letter: each letter is the
    // first group's.
    let mut many: Vec<KeyedGroup> = LetterKey::ALL
        .iter()
        .enumerate()
        .map(|(i, &letter)| keyed(100 + i as u64, letter, 3))
        .collect();
    for (i, &letter) in LetterKey::ALL.iter().take(4).enumerate() {
        many.push(keyed(200 + i as u64, letter, 9));
    }
    for (i, &letter) in LetterKey::ALL.iter().enumerate() {
        let mut r = Rig::with_groups(default_config(), PLAIN, &many).named(&format!("{letter:?}"));
        r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.tap(
            50,
            Key::Letter(letter),
            ALT,
            goto(group(100 + i as u64)),
            Phase::Switched,
            HIDDEN,
        );
    }
}

// ------------------------------------------------------------------------------------------
// Any phase
// ------------------------------------------------------------------------------------------

/// `Reset` in each phase: Idle, nothing selected, hidden, `Hide` only if it was visible, the
/// tracked modifiers forgotten, the hold forgotten.
#[test]
fn reset_in_each_phase() {
    for phase in ALL_PHASES {
        let (mut r, t) = rig_in(phase);
        let want = if phase == Phase::Overlay { HIDE } else { PASS };
        r.step(t, Event::Reset, want, Phase::Idle, HIDDEN);
        r.deadline_is(None);
        r.poll(t + 5000, PASS, Phase::Idle, HIDDEN);

        // The tracked modifiers are forgotten: a ⌥ that is still down is a new press.
        r.step(t + 5100, mods(ALT), PASS, Phase::Armed, HIDDEN);
        r.deadline_is(Some(t + 5500));
    }
}

/// `Reset` in the Overlay with a selection: no switch; the key-up memory is forgotten too.
#[test]
fn reset_forgets_the_card_and_the_keys_it_consumed() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.step(
        500,
        down(S_KEY, ALT),
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        520,
        down(digit(3), ALT),
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(3)),
    );
    r.step(600, Event::Reset, HIDE, Phase::Idle, HIDDEN);
    r.step(610, up(S_KEY, NONE), PASS, Phase::Idle, HIDDEN);
    r.step(620, up(digit(3), NONE), PASS, Phase::Idle, HIDDEN);
}

/// "A key-up is consumed if and only if its key-down was consumed."
#[test]
fn a_key_up_is_consumed_if_and_only_if_its_down_was() {
    // Consumed down, consumed up (the card, and the immediate switch).
    let mut r = Rig::overlay(default_config(), TEXT);
    r.step(
        500,
        down(S_KEY, ALT),
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        520,
        up(S_KEY, ALT),
        EAT,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    // The second up of the same key has no consumed down behind it any more.
    r.step(
        530,
        up(S_KEY, ALT),
        PASS,
        Phase::Overlay,
        card(RESEARCH, None),
    );

    // Passed down, passed up: the page saw the press, so it sees the release.
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(50, down(S_KEY, ALT), PASS, Phase::PassThrough, HIDDEN);
    r.step(80, up(S_KEY, ALT), PASS, Phase::PassThrough, HIDDEN);

    // A key pressed before ⌥ (so passed) and released with the card up: passed, though an
    // unassigned key would be consumed there as a press.
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, down(Q_KEY, NONE), PASS, Phase::Idle, HIDDEN);
    r.step(10, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.poll(410, SHOW, Phase::Overlay, BARE);
    r.step(500, up(Q_KEY, ALT), PASS, Phase::Overlay, BARE);

    // A release with no press at all.
    r.step(510, up(digit(4), ALT), PASS, Phase::Overlay, BARE);

    // Escape is consumed, so its release is, in the phase it left behind (Idle).
    let mut r = Rig::overlay(default_config(), TEXT);
    r.step(
        500,
        down(Key::Escape, ALT),
        EAT_AND_HIDE,
        Phase::Idle,
        HIDDEN,
    );
    r.step(520, up(Key::Escape, ALT), EAT, Phase::Idle, HIDDEN);
}

/// "This holds after ⌥ came up first": the card commits on ⌥ up, and the release of the letter
/// that follows is still consumed. The same after an immediate switch.
#[test]
fn a_key_up_after_option_came_up_first_is_still_consumed() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.step(
        500,
        down(S_KEY, ALT),
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        520,
        down(digit(3), ALT),
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(3)),
    );
    r.step(
        600,
        mods(NONE),
        commit(Some(tab(RESEARCH, 2))),
        Phase::Idle,
        HIDDEN,
    );
    r.step(610, up(digit(3), NONE), EAT, Phase::Idle, HIDDEN);
    r.step(620, up(S_KEY, NONE), EAT, Phase::Idle, HIDDEN);
    // They are forgotten once released.
    r.step(630, up(S_KEY, NONE), PASS, Phase::Idle, HIDDEN);

    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(
        50,
        down(S_KEY, ALT),
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
    r.step(100, mods(NONE), PASS, Phase::Idle, HIDDEN);
    r.step(110, up(S_KEY, NONE), EAT, Phase::Idle, HIDDEN);

    // The release carrying no ⌥ itself (it is the key event that says ⌥ is up).
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(
        50,
        down(S_KEY, ALT),
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
    r.step(100, up(S_KEY, NONE), EAT, Phase::Idle, HIDDEN);
    r.step(110, mods(NONE), PASS, Phase::Idle, HIDDEN);
}

/// "Auto-repeat of a captured key is consumed with no effect; of a passed key it passes."
#[test]
fn auto_repeat_of_a_captured_key_is_consumed_and_of_a_passed_key_passes() {
    // In the card: the repeat of a letter changes nothing, even when another group is selected.
    let mut r = Rig::overlay(default_config(), TEXT);
    r.step(
        500,
        down(S_KEY, ALT),
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        520,
        again(S_KEY, ALT),
        EAT,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        540,
        again(S_KEY, ALT),
        EAT,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.step(
        600,
        down(A_KEY, ALT),
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(TRIP, None),
    );
    r.step(
        620,
        again(S_KEY, ALT),
        EAT,
        Phase::Overlay,
        card(TRIP, None),
    );
    // A digit, and an unassigned key.
    r.step(
        700,
        down(digit(1), ALT),
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(TRIP, Some(1)),
    );
    r.step(
        720,
        again(digit(1), ALT),
        EAT,
        Phase::Overlay,
        card(TRIP, Some(1)),
    );
    r.step(
        800,
        down(Q_KEY, ALT),
        EAT,
        Phase::Overlay,
        card(TRIP, Some(1)),
    );
    r.step(
        820,
        again(Q_KEY, ALT),
        EAT,
        Phase::Overlay,
        card(TRIP, Some(1)),
    );
    r.step(
        900,
        down(Key::Arrow, ALT),
        PASS,
        Phase::Overlay,
        card(TRIP, Some(1)),
    );
    r.step(
        920,
        again(Key::Arrow, ALT),
        PASS,
        Phase::Overlay,
        card(TRIP, Some(1)),
    );

    // After an immediate switch: no second switch, and a digit's repeat is no second tab.
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(
        50,
        down(S_KEY, ALT),
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
    r.step(70, again(S_KEY, ALT), EAT, Phase::Switched, HIDDEN);
    r.step(90, again(S_KEY, ALT), EAT, Phase::Switched, HIDDEN);
    r.step(100, up(S_KEY, ALT), EAT, Phase::Switched, HIDDEN);
    r.step(
        150,
        down(digit(2), ALT),
        goto(tab(RESEARCH, 1)),
        Phase::Switched,
        HIDDEN,
    );
    r.step(170, again(digit(2), ALT), EAT, Phase::Switched, HIDDEN);
    r.step(190, up(digit(2), ALT), EAT, Phase::Switched, HIDDEN);
    // A passed key repeats as a passed key.
    r.step(250, down(Q_KEY, ALT), PASS, Phase::Switched, HIDDEN);
    r.step(270, again(Q_KEY, ALT), PASS, Phase::Switched, HIDDEN);
    r.step(290, up(Q_KEY, ALT), PASS, Phase::Switched, HIDDEN);

    // In a text field the held letter that the page types keeps being the page's.
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.step(50, down(S_KEY, ALT), PASS, Phase::PassThrough, HIDDEN);
    r.step(70, again(S_KEY, ALT), PASS, Phase::PassThrough, HIDDEN);
    r.step(90, up(S_KEY, ALT), PASS, Phase::PassThrough, HIDDEN);

    // And in Idle.
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, down(S_KEY, NONE), PASS, Phase::Idle, HIDDEN);
    r.step(20, again(S_KEY, NONE), PASS, Phase::Idle, HIDDEN);
    r.step(40, up(S_KEY, NONE), PASS, Phase::Idle, HIDDEN);
}

/// "Arrows are never consumed", in any phase (⌥ with an arrow moves by word), and an arrow's
/// release is never consumed either.
#[test]
fn arrows_are_never_consumed_in_any_phase() {
    for phase in ALL_PHASES {
        let (mut r, t) = rig_in(phase);
        let m = held(phase);
        // Armed: "any other key" is PassThrough; in every other phase the arrow changes nothing.
        let after = if phase == Phase::Armed {
            Phase::PassThrough
        } else {
            phase
        };
        r.tap(t, Key::Arrow, m, PASS, after, shown(phase));
        r.step(t + 20, again(Key::Arrow, m), PASS, after, shown(phase));
        r.tap(t + 40, Key::Arrow, m, PASS, after, shown(phase));
    }
}

/// "`Modifier` keys: never captured, never change the state", in any phase.
#[test]
fn modifier_keys_are_never_consumed_in_any_phase() {
    for phase in ALL_PHASES {
        let (mut r, t) = rig_in(phase);
        let m = held(phase);
        r.tap(t, Key::Modifier, m, PASS, phase, shown(phase));
        r.step(t + 20, again(Key::Modifier, m), PASS, phase, shown(phase));
        r.tap(t + 40, Key::Modifier, m, PASS, phase, shown(phase));
    }
}

/// "`Event::Modifiers` is never consumed": every combination, in every phase.
#[test]
fn modifiers_events_are_never_consumed() {
    for phase in ALL_PHASES {
        for bits in 0u8..16 {
            let m = Mods {
                alt: bits & 1 != 0,
                shift: bits & 2 != 0,
                control: bits & 4 != 0,
                command: bits & 8 != 0,
            };
            let (mut r, t) = rig_in(phase);
            let got = r.keys.on_event(ms(t), mods(m));
            assert!(
                !got.consume,
                "{phase:?}, {m:?}: Modifiers consumed ({got:?})"
            );
        }
    }
}

/// Escape in each phase: the page's, except in the card.
#[test]
fn escape_in_each_phase() {
    for phase in ALL_PHASES {
        let (mut r, t) = rig_in(phase);
        let m = held(phase);
        match phase {
            Phase::Overlay => r.tap(t, Key::Escape, m, EAT_AND_HIDE, Phase::Idle, HIDDEN),
            Phase::Armed => r.tap(t, Key::Escape, m, PASS, Phase::PassThrough, HIDDEN),
            _ => r.tap(t, Key::Escape, m, PASS, phase, HIDDEN),
        }
    }
}

/// `enabled == false`: Idle, hidden, nothing consumed or activated, forever; turning it on again
/// brings ⌥ back.
#[test]
fn a_disabled_config_passes_everything() {
    for capture in CAPTURES {
        for focus in [PLAIN, TEXT, IME] {
            let mut r =
                Rig::new(disabled(capture), focus).named(&format!("{capture:?}, {focus:?}"));
            r.step(0, mods(ALT), PASS, Phase::Idle, HIDDEN);
            r.deadline_is(None);
            r.tap(50, S_KEY, ALT, PASS, Phase::Idle, HIDDEN);
            r.poll(2000, PASS, Phase::Idle, HIDDEN);
            for key in [
                digit(2),
                Key::Escape,
                Key::Arrow,
                Key::Other(7),
                A_KEY,
                Q_KEY,
                Key::Modifier,
            ] {
                r.tap(2100, key, ALT, PASS, Phase::Idle, HIDDEN);
            }
            let ignored = [
                Event::PointerDown,
                Event::Dismiss,
                Event::TileClicked { group: RESEARCH },
                Event::ChipClicked { index: 0 },
            ];
            for event in ignored {
                r.click(2200, event, None, CardChange::None, Phase::Idle, HIDDEN);
            }
            r.step(2300, mods(with_alt(COMMAND)), PASS, Phase::Idle, HIDDEN);
            r.step(2400, mods(NONE), PASS, Phase::Idle, HIDDEN);
            r.step(2500, Event::Reset, PASS, Phase::Idle, HIDDEN);

            // On again: a new ⌥ press arms (not when composing: that is PassThrough).
            r.set_config(config(capture), PASS, Phase::Idle, HIDDEN);
            let armed = if focus.composing {
                Phase::PassThrough
            } else {
                Phase::Armed
            };
            r.step(3000, mods(ALT), PASS, armed, HIDDEN);
        }
    }
}

/// `set_config` that disables the keys while the card is up closes it: Idle, hidden, no switch.
#[test]
fn set_config_disabling_while_the_card_is_up_closes_it() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        520,
        digit(3),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(3)),
    );
    r.set_config(
        disabled(KeyCapture::OutsideTextOrHold),
        HIDE,
        Phase::Idle,
        HIDDEN,
    );
    r.deadline_is(None);
    // Nothing is captured now, and ⌥ up has nothing to commit.
    r.tap(600, A_KEY, ALT, PASS, Phase::Idle, HIDDEN);
    r.step(700, mods(NONE), PASS, Phase::Idle, HIDDEN);
}

/// `enabled == false` through `set_config` in the other phases is Idle too.
#[test]
fn set_config_disabling_in_any_phase_gives_idle() {
    for phase in ALL_PHASES {
        let (mut r, t) = rig_in(phase);
        let want = if phase == Phase::Overlay { HIDE } else { PASS };
        r.set_config(
            disabled(KeyCapture::OutsideTextOrHold),
            want,
            Phase::Idle,
            HIDDEN,
        );
        r.deadline_is(None);
        r.poll(t + 2000, PASS, Phase::Idle, HIDDEN);
        r.tap(t + 2100, digit(2), ALT, PASS, Phase::Idle, HIDDEN);
        r.tap(t + 2200, S_KEY, ALT, PASS, Phase::Idle, HIDDEN);
    }
}

/// `set_config` that removes the hold path (`OutsideText`) while the card is up closes it; ⌥ is
/// still down, so the rest of the hold belongs to the page.
#[test]
fn set_config_removing_the_hold_path_while_the_card_is_up_closes_it() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.set_config(
        config(KeyCapture::OutsideText),
        HIDE,
        IDLE_OR_PASS_THROUGH,
        HIDDEN,
    );
    r.deadline_is(None);
    r.tap(600, A_KEY, ALT, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
    r.step(700, mods(NONE), PASS, Phase::Idle, HIDDEN);
}

/// `set_config` that keeps the hold path (a change to `Always`) leaves the card as it is.
#[test]
fn set_config_keeping_the_hold_path_leaves_the_card_alone() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        520,
        digit(5),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(5)),
    );
    r.set_config(
        config(KeyCapture::Always),
        PASS,
        Phase::Overlay,
        card(RESEARCH, Some(5)),
    );
    r.set_config(
        default_config(),
        PASS,
        Phase::Overlay,
        card(RESEARCH, Some(5)),
    );
    r.step(
        700,
        mods(NONE),
        commit(Some(tab(RESEARCH, 4))),
        Phase::Idle,
        HIDDEN,
    );
}

/// `set_config` that removes the hold path while Armed: no deadline any more, no card.
#[test]
fn set_config_removing_the_hold_path_while_armed_cancels_the_deadline() {
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.deadline_is(Some(400));
    r.set_config(config(KeyCapture::OutsideText), PASS, ANY, HIDDEN);
    r.deadline_is(None);
    r.poll(1000, PASS, ANY, HIDDEN);
    r.step(1100, mods(NONE), PASS, Phase::Idle, HIDDEN);
}

/// `set_focus` with `composing` cancels the card (Idle, or PassThrough while ⌥ is down) and a
/// later composition end does not give the page's hold back to Mercury.
#[test]
fn set_focus_composing_while_the_card_is_up_closes_it() {
    let mut r = Rig::overlay(default_config(), TEXT);
    r.tap(
        500,
        S_KEY,
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, None),
    );
    r.tap(
        520,
        digit(3),
        ALT,
        EAT_AND_UPDATE,
        Phase::Overlay,
        card(RESEARCH, Some(3)),
    );
    r.set_focus(IME, HIDE, IDLE_OR_PASS_THROUGH, HIDDEN);
    r.deadline_is(None);
    // Nothing is captured while composing, and ⌥ up has nothing to commit.
    r.tap(600, A_KEY, ALT, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
    r.step(700, mods(NONE), PASS, Phase::Idle, HIDDEN);

    // The composition ends with ⌥ still down: the keys are still not Mercury's.
    let mut r = Rig::overlay(default_config(), TEXT);
    r.set_focus(IME, HIDE, IDLE_OR_PASS_THROUGH, HIDDEN);
    r.set_focus(TEXT, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
    r.tap(600, S_KEY, ALT, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
    r.poll(5000, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
}

/// `set_focus` with `composing` while Armed: no deadline, no card, the next letter is the IME's;
/// the composition ending does not bring the hold back.
#[test]
fn set_focus_composing_while_armed_cancels_the_hold() {
    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.set_focus(IME, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
    r.deadline_is(None);
    r.set_focus(TEXT, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
    r.deadline_is(None);
    r.poll(2000, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
    r.tap(2100, S_KEY, ALT, PASS, IDLE_OR_PASS_THROUGH, HIDDEN);
    r.step(2200, mods(NONE), PASS, Phase::Idle, HIDDEN);
}

/// Armed reads the focus when the letter arrives: a text field that gains focus after ⌥ is the
/// text field's, and one that loses it is not.
#[test]
fn the_focus_is_read_when_the_letter_arrives() {
    let mut r = Rig::new(default_config(), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.set_focus(TEXT, PASS, Phase::Armed, HIDDEN);
    r.deadline_is(Some(400));
    r.tap(100, S_KEY, ALT, PASS, Phase::PassThrough, HIDDEN);

    let mut r = Rig::new(default_config(), TEXT);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.set_focus(PLAIN, PASS, Phase::Armed, HIDDEN);
    r.tap(
        100,
        S_KEY,
        ALT,
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );

    // `Always` does not care about the text field.
    let mut r = Rig::new(config(KeyCapture::Always), PLAIN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.set_focus(TEXT, PASS, Phase::Armed, HIDDEN);
    r.tap(
        100,
        S_KEY,
        ALT,
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
}

/// `set_groups` outside the card changes nothing that shows; the groups of the moment are the
/// ones a key finds.
#[test]
fn set_groups_outside_the_card_is_silent() {
    let mut r = Rig::new(default_config(), PLAIN);
    r.set_groups(&[], PASS, Phase::Idle, HIDDEN);
    r.set_groups(&keyed_groups(), PASS, Phase::Idle, HIDDEN);
    r.step(0, mods(ALT), PASS, Phase::Armed, HIDDEN);
    r.set_groups(
        &[keyed(RESEARCH, LetterKey::S, 1)],
        PASS,
        Phase::Armed,
        HIDDEN,
    );
    r.tap(
        100,
        S_KEY,
        ALT,
        goto(group(RESEARCH)),
        Phase::Switched,
        HIDDEN,
    );
    r.tap(
        200,
        digit(1),
        ALT,
        goto(tab(RESEARCH, 0)),
        Phase::Switched,
        HIDDEN,
    );
    r.tap(300, digit(2), ALT, EAT, Phase::Switched, HIDDEN);
}
