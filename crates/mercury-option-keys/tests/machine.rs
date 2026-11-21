// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Tests of the ⌥ state machine, one per line of the transition table of the lane brief (§5.4)
//! and one per decision around it (D2, D3, D5, D9, D10, D16). Time is injected: every call says
//! what time it is, in milliseconds since an origin of the test's choosing.
//!
//! The groups of the fixture: `S` is "Search" (3 tabs), `A` is "Art" (5), `E` is "Mail" (2) and
//! `R` is "Reading" (12). `Q` has no group, so it is the "unassigned letter".

use std::time::Duration;

use mercury_option_keys::{
    CardChange, CardView, Config, Event, Focus, Key, KeyCapture, KeyedGroup, LetterKey, Mods,
    OptionKeys, Outcome, Phase, Target,
};

// ---------------------------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------------------------

const NONE: Mods = Mods::NONE;
const ALT: Mods = Mods::ALT;
const SHIFT: Mods = Mods {
    shift: true,
    ..Mods::NONE
};
const CTRL: Mods = Mods {
    control: true,
    ..Mods::NONE
};
const CMD: Mods = Mods {
    command: true,
    ..Mods::NONE
};
const ALT_SHIFT: Mods = Mods { alt: true, ..SHIFT };
const ALT_CTRL: Mods = Mods { alt: true, ..CTRL };
const ALT_CMD: Mods = Mods { alt: true, ..CMD };

const S: Key = Key::Letter(LetterKey::S);
const A: Key = Key::Letter(LetterKey::A);
const E: Key = Key::Letter(LetterKey::E);
const R: Key = Key::Letter(LetterKey::R);
const Q: Key = Key::Letter(LetterKey::Q);

const SEARCH: u64 = 10;
const ART: u64 = 20;
const MAIL: u64 = 30;
const READ: u64 = 40;

const PASS: Outcome = Outcome::PASS;
const HIDDEN: CardView = CardView {
    visible: false,
    selected_group: None,
    selected_tab: None,
};
const THREE_MODES: [KeyCapture; 3] = [
    KeyCapture::OutsideTextOrHold,
    KeyCapture::OutsideText,
    KeyCapture::Always,
];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn digit(n: u8) -> Key {
    Key::Digit(n)
}

fn kg(id: u64, key: LetterKey, tab_count: usize) -> KeyedGroup {
    KeyedGroup { id, key, tab_count }
}

fn groups() -> [KeyedGroup; 4] {
    [
        kg(SEARCH, LetterKey::S, 3),
        kg(ART, LetterKey::A, 5),
        kg(MAIL, LetterKey::E, 2),
        kg(READ, LetterKey::R, 12),
    ]
}

fn config(capture: KeyCapture) -> Config {
    Config::new(true, capture, ms(400))
}

fn machine_in(capture: KeyCapture) -> OptionKeys {
    let mut m = OptionKeys::new(config(capture));
    m.set_groups(&groups());
    m
}

fn machine() -> OptionKeys {
    machine_in(KeyCapture::OutsideTextOrHold)
}

fn typing() -> Focus {
    Focus {
        text_field: true,
        composing: false,
    }
}

fn composing() -> Focus {
    Focus {
        text_field: true,
        composing: true,
    }
}

fn event(m: &mut OptionKeys, t: u64, event: Event) -> Outcome {
    m.on_event(ms(t), event)
}

fn modifiers(m: &mut OptionKeys, t: u64, mods: Mods) -> Outcome {
    event(m, t, Event::Modifiers(mods))
}

fn key_event(
    m: &mut OptionKeys,
    t: u64,
    key: Key,
    mods: Mods,
    down: bool,
    repeat: bool,
) -> Outcome {
    event(
        m,
        t,
        Event::Key {
            key,
            mods,
            down,
            repeat,
        },
    )
}

fn press(m: &mut OptionKeys, t: u64, key: Key, mods: Mods) -> Outcome {
    key_event(m, t, key, mods, true, false)
}

fn autorepeat(m: &mut OptionKeys, t: u64, key: Key, mods: Mods) -> Outcome {
    key_event(m, t, key, mods, true, true)
}

fn release(m: &mut OptionKeys, t: u64, key: Key, mods: Mods) -> Outcome {
    key_event(m, t, key, mods, false, false)
}

fn card(change: CardChange) -> Outcome {
    Outcome {
        card: change,
        ..PASS
    }
}

fn swallowed() -> Outcome {
    Outcome {
        consume: true,
        ..PASS
    }
}

fn swallowed_with(card: CardChange) -> Outcome {
    Outcome {
        consume: true,
        activate: None,
        card,
    }
}

fn switch_to(target: Target) -> Outcome {
    Outcome {
        consume: true,
        activate: Some(target),
        card: CardChange::None,
    }
}

fn activates(target: Target, card: CardChange) -> Outcome {
    Outcome {
        consume: false,
        activate: Some(target),
        card,
    }
}

fn group(id: u64) -> Target {
    Target::Group(id)
}

fn tab(group: u64, index: usize) -> Target {
    Target::Tab { group, index }
}

fn shown(selected_group: Option<u64>, selected_tab: Option<u8>) -> CardView {
    CardView {
        visible: true,
        selected_group,
        selected_tab,
    }
}

/// ⌥ went down alone at 0 ms.
fn armed_in(capture: KeyCapture) -> OptionKeys {
    let mut m = machine_in(capture);
    assert_eq!(modifiers(&mut m, 0, ALT), PASS);
    assert_eq!(m.phase(), Phase::Armed);
    m
}

fn armed() -> OptionKeys {
    armed_in(KeyCapture::OutsideTextOrHold)
}

/// ⌥ went down at 0 ms and the hold timer fired at 400 ms.
fn overlay_in(capture: KeyCapture) -> OptionKeys {
    let mut m = armed_in(capture);
    assert_eq!(m.poll(ms(400)), card(CardChange::Show));
    assert_eq!(m.phase(), Phase::Overlay);
    m
}

fn overlay() -> OptionKeys {
    overlay_in(KeyCapture::OutsideTextOrHold)
}

/// ⌥ then S outside a text field: an immediate switch to Search.
fn switched() -> OptionKeys {
    let mut m = armed();
    assert_eq!(press(&mut m, 50, S, ALT), switch_to(group(SEARCH)));
    assert_eq!(m.phase(), Phase::Switched);
    m
}

/// ⌥ then an unassigned letter: this hold belongs to the page.
fn pass_through() -> OptionKeys {
    let mut m = armed();
    assert_eq!(press(&mut m, 50, Q, ALT), PASS);
    assert_eq!(m.phase(), Phase::PassThrough);
    m
}

/// One machine in each phase, ⌥ down in all but the first.
fn every_phase() -> Vec<(Phase, OptionKeys)> {
    vec![
        (Phase::Idle, machine()),
        (Phase::Armed, armed()),
        (Phase::PassThrough, pass_through()),
        (Phase::Switched, switched()),
        (Phase::Overlay, overlay()),
    ]
}

// ---------------------------------------------------------------------------------------------
// The sentence of SPEC section 8, lane 11
// ---------------------------------------------------------------------------------------------

mod sentence {
    use super::*;

    #[test]
    fn quick_option_s_in_a_text_field_passes_through() {
        let mut m = machine();
        m.set_focus(typing());
        assert_eq!(modifiers(&mut m, 0, ALT), PASS);
        assert_eq!(press(&mut m, 90, S, ALT), PASS);
        assert_eq!(m.phase(), Phase::PassThrough);
        assert_eq!(release(&mut m, 140, S, ALT), PASS);
        assert_eq!(modifiers(&mut m, 200, NONE), PASS);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
    }

    #[test]
    fn a_400_ms_hold_in_a_text_field_opens_the_card() {
        let mut m = machine();
        m.set_focus(typing());
        modifiers(&mut m, 1_000, ALT);
        assert_eq!(m.deadline(), Some(ms(1_400)));
        assert_eq!(m.poll(ms(1_399)), PASS);
        assert_eq!(m.poll(ms(1_400)), card(CardChange::Show));
        assert_eq!(m.view(), shown(None, None));
    }

    #[test]
    fn letter_then_digit_then_alt_up_commits_the_tab() {
        let mut m = overlay();
        assert_eq!(
            press(&mut m, 500, S, ALT),
            swallowed_with(CardChange::Update)
        );
        assert_eq!(
            press(&mut m, 600, digit(2), ALT),
            swallowed_with(CardChange::Update)
        );
        assert_eq!(m.view(), shown(Some(SEARCH), Some(2)));
        assert_eq!(
            modifiers(&mut m, 700, NONE),
            activates(tab(SEARCH, 1), CardChange::Hide)
        );
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
    }

    #[test]
    fn letter_then_alt_up_commits_the_group() {
        let mut m = overlay();
        press(&mut m, 500, A, ALT);
        assert_eq!(
            modifiers(&mut m, 600, NONE),
            activates(group(ART), CardChange::Hide)
        );
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn escape_pointer_down_and_dismiss_hide_with_nothing_activated() {
        for closer in 0..3 {
            let mut m = overlay();
            press(&mut m, 500, S, ALT);
            press(&mut m, 550, digit(2), ALT);
            let closed = match closer {
                0 => press(&mut m, 600, Key::Escape, ALT),
                1 => event(&mut m, 600, Event::PointerDown),
                _ => event(&mut m, 600, Event::Dismiss),
            };
            assert_eq!(closed.activate, None, "closer {closer}");
            assert_eq!(closed.card, CardChange::Hide, "closer {closer}");
            assert_eq!(closed.consume, closer == 0, "closer {closer}");
            assert_eq!(m.phase(), Phase::Idle);
            assert_eq!(m.view(), HIDDEN);
            // Letting go of ⌥ afterwards activates nothing: the selection died with the card.
            assert_eq!(modifiers(&mut m, 700, NONE), PASS);
        }
    }

    #[test]
    fn command_option_is_ignored() {
        let mut m = machine();
        assert_eq!(modifiers(&mut m, 0, ALT_CMD), PASS);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.deadline(), None);
        assert_eq!(press(&mut m, 50, S, ALT_CMD), PASS);
        assert_eq!(m.poll(ms(10_000)), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn ime_composition_blocks_capture_in_all_three_modes() {
        for capture in THREE_MODES {
            for text_field in [false, true] {
                let focus = Focus {
                    text_field,
                    composing: true,
                };
                // The composition was already open when ⌥ went down.
                let mut m = machine_in(capture);
                m.set_focus(focus);
                assert_eq!(modifiers(&mut m, 0, ALT), PASS, "{capture:?}");
                assert_eq!(m.phase(), Phase::PassThrough, "{capture:?}");
                assert_eq!(press(&mut m, 50, S, ALT), PASS, "{capture:?}");
                // The composition opened while ⌥ was down and armed.
                let mut m = armed_in(capture);
                assert_eq!(m.set_focus(focus), PASS, "{capture:?}");
                assert_eq!(m.phase(), Phase::PassThrough, "{capture:?}");
                assert_eq!(press(&mut m, 50, S, ALT), PASS, "{capture:?}");
                assert_eq!(m.phase(), Phase::PassThrough, "{capture:?}");
            }
        }
    }

    #[test]
    fn a_dead_key_letter_is_captured_outside_a_text_field_and_passed_inside_one_unless_always() {
        for capture in THREE_MODES {
            let mut outside = armed_in(capture);
            assert_eq!(
                press(&mut outside, 50, E, ALT),
                switch_to(group(MAIL)),
                "{capture:?} outside a text field"
            );
            let mut inside = machine_in(capture);
            inside.set_focus(typing());
            modifiers(&mut inside, 0, ALT);
            let answer = press(&mut inside, 50, E, ALT);
            if capture == KeyCapture::Always {
                assert_eq!(answer, switch_to(group(MAIL)), "always");
            } else {
                assert_eq!(answer, PASS, "{capture:?} inside a text field");
                assert_eq!(inside.phase(), Phase::PassThrough);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Idle
// ---------------------------------------------------------------------------------------------

mod idle {
    use super::*;

    #[test]
    fn a_new_machine_is_idle_with_nothing_to_show() {
        let m = machine();
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
        assert_eq!(m.deadline(), None);
        let bare = OptionKeys::new(Config::default());
        assert_eq!(bare.phase(), Phase::Idle);
        assert_eq!(bare.view(), HIDDEN);
    }

    #[test]
    fn alt_going_down_alone_arms_at_that_time() {
        let mut m = machine();
        assert_eq!(modifiers(&mut m, 100, ALT), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        assert_eq!(m.deadline(), Some(ms(500)));
    }

    #[test]
    fn alt_going_down_during_a_composition_goes_to_pass_through() {
        let mut m = machine();
        m.set_focus(composing());
        assert_eq!(modifiers(&mut m, 0, ALT), PASS);
        assert_eq!(m.phase(), Phase::PassThrough);
        assert_eq!(m.deadline(), None);
    }

    #[test]
    fn alt_going_down_with_command_control_or_shift_already_down_stays_idle() {
        for other in [CMD, CTRL, SHIFT] {
            let mut m = machine();
            assert_eq!(modifiers(&mut m, 0, other), PASS);
            assert_eq!(modifiers(&mut m, 10, Mods { alt: true, ..other }), PASS);
            assert_eq!(m.phase(), Phase::Idle, "{other:?}");
            assert_eq!(m.deadline(), None, "{other:?}");
            assert_eq!(m.poll(ms(5_000)), PASS, "{other:?}");
        }
    }

    #[test]
    fn releasing_the_other_modifier_later_does_not_arm() {
        let mut m = machine();
        modifiers(&mut m, 0, ALT_CMD);
        assert_eq!(modifiers(&mut m, 100, ALT), PASS);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.deadline(), None);
        // ⌥ is still down and was never captured: S is the page's.
        assert_eq!(press(&mut m, 150, S, ALT), PASS);
        assert_eq!(m.poll(ms(5_000)), PASS);
        // A new ⌥ press is a new edge.
        modifiers(&mut m, 6_000, NONE);
        modifiers(&mut m, 6_100, ALT);
        assert_eq!(m.phase(), Phase::Armed);
    }

    #[test]
    fn alt_with_shift_or_control_already_down_is_ignored_like_command() {
        for other in [ALT_SHIFT, ALT_CTRL] {
            let mut m = machine();
            assert_eq!(modifiers(&mut m, 0, other), PASS);
            assert_eq!(m.phase(), Phase::Idle, "{other:?}");
        }
    }

    #[test]
    fn keys_pass_in_idle() {
        let mut m = machine();
        for (i, key) in [
            S,
            Q,
            digit(1),
            digit(0),
            Key::Escape,
            Key::Arrow,
            Key::Modifier,
            Key::Other(7),
        ]
        .into_iter()
        .enumerate()
        {
            let t = 10 * i as u64;
            assert_eq!(press(&mut m, t, key, NONE), PASS, "{key:?}");
            assert_eq!(autorepeat(&mut m, t + 1, key, NONE), PASS, "{key:?}");
            assert_eq!(release(&mut m, t + 2, key, NONE), PASS, "{key:?}");
        }
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn pointer_tile_chip_and_dismiss_events_are_ignored_in_idle() {
        let mut m = machine();
        for e in [
            Event::PointerDown,
            Event::TileClicked { group: SEARCH },
            Event::ChipClicked { index: 0 },
            Event::Dismiss,
        ] {
            assert_eq!(event(&mut m, 0, e), PASS, "{e:?}");
        }
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn disabled_keys_pass_everything_forever() {
        let mut m = OptionKeys::new(Config::new(false, KeyCapture::Always, ms(400)));
        m.set_groups(&groups());
        assert_eq!(modifiers(&mut m, 0, ALT), PASS);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.deadline(), None);
        assert_eq!(press(&mut m, 50, S, ALT), PASS);
        assert_eq!(press(&mut m, 60, digit(1), ALT), PASS);
        assert_eq!(m.poll(ms(60_000)), PASS);
        assert_eq!(modifiers(&mut m, 70_000, NONE), PASS);
        assert_eq!(modifiers(&mut m, 71_000, ALT), PASS);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
    }

    #[test]
    fn a_key_event_with_alt_in_its_modifiers_arms_and_is_then_handled_as_armed() {
        let mut m = machine();
        // No ModifiersChanged was seen; the key carries ⌥.
        assert_eq!(press(&mut m, 40, S, ALT), switch_to(group(SEARCH)));
        assert_eq!(m.phase(), Phase::Switched);
    }

    #[test]
    fn a_key_event_with_alt_and_another_modifier_does_not_arm() {
        let mut m = machine();
        assert_eq!(press(&mut m, 40, S, ALT_SHIFT), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }
}

// ---------------------------------------------------------------------------------------------
// Armed
// ---------------------------------------------------------------------------------------------

mod armed {
    use super::*;

    #[test]
    fn a_lone_alt_press_and_release_does_nothing() {
        let mut m = armed();
        assert_eq!(modifiers(&mut m, 100, NONE), PASS);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.deadline(), None);
        assert_eq!(m.poll(ms(5_000)), PASS);
        assert_eq!(m.view(), HIDDEN);
    }

    #[test]
    fn alt_released_with_another_modifier_pressed_goes_idle() {
        let mut m = armed();
        assert_eq!(modifiers(&mut m, 100, CMD), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn adding_command_control_or_shift_goes_to_pass_through() {
        for added in [ALT_CMD, ALT_CTRL, ALT_SHIFT] {
            let mut m = armed();
            assert_eq!(modifiers(&mut m, 100, added), PASS);
            assert_eq!(m.phase(), Phase::PassThrough, "{added:?}");
            assert_eq!(m.deadline(), None, "{added:?}");
            assert_eq!(press(&mut m, 150, S, added), PASS, "{added:?}");
            assert_eq!(m.poll(ms(5_000)), PASS, "{added:?}");
        }
    }

    #[test]
    fn a_modifier_key_passes_and_changes_nothing() {
        let mut m = armed();
        assert_eq!(press(&mut m, 10, Key::Modifier, ALT), PASS);
        assert_eq!(autorepeat(&mut m, 20, Key::Modifier, ALT), PASS);
        assert_eq!(release(&mut m, 30, Key::Modifier, ALT), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        assert_eq!(m.deadline(), Some(ms(400)));
    }

    #[test]
    fn a_modifier_key_event_never_changes_the_state_even_with_other_modifiers() {
        // The modifier state arrives as `Event::Modifiers`; the key of a modifier itself
        // (Key::Modifier) is never an edge.
        let mut m = armed();
        assert_eq!(press(&mut m, 10, Key::Modifier, ALT_CMD), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        assert_eq!(press(&mut m, 20, Key::Modifier, NONE), PASS);
        assert_eq!(m.phase(), Phase::Armed);
    }

    #[test]
    fn an_assigned_letter_outside_a_text_field_switches_at_once() {
        for capture in THREE_MODES {
            let mut m = armed_in(capture);
            assert_eq!(
                press(&mut m, 120, S, ALT),
                switch_to(group(SEARCH)),
                "{capture:?}"
            );
            assert_eq!(m.phase(), Phase::Switched, "{capture:?}");
            assert_eq!(m.view(), HIDDEN, "{capture:?}");
            assert_eq!(m.deadline(), None, "{capture:?}");
        }
    }

    #[test]
    fn an_assigned_letter_in_a_text_field_passes_unless_capture_is_always() {
        for capture in [KeyCapture::OutsideTextOrHold, KeyCapture::OutsideText] {
            let mut m = armed_in(capture);
            m.set_focus(typing());
            assert_eq!(press(&mut m, 120, S, ALT), PASS, "{capture:?}");
            assert_eq!(m.phase(), Phase::PassThrough, "{capture:?}");
        }
        let mut m = armed_in(KeyCapture::Always);
        m.set_focus(typing());
        assert_eq!(press(&mut m, 120, S, ALT), switch_to(group(SEARCH)));
        assert_eq!(m.phase(), Phase::Switched);
    }

    #[test]
    fn the_focus_is_read_when_the_letter_arrives() {
        // ⌥ went down in a text field and the focus moved out of it before the letter.
        let mut m = machine();
        m.set_focus(typing());
        modifiers(&mut m, 0, ALT);
        assert_eq!(m.set_focus(Focus::default()), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        assert_eq!(press(&mut m, 50, A, ALT), switch_to(group(ART)));
    }

    #[test]
    fn any_other_key_goes_to_pass_through_and_passes() {
        for key in [
            Q,
            digit(1),
            digit(0),
            digit(10),
            Key::Escape,
            Key::Arrow,
            Key::Other(3),
        ] {
            let mut m = armed();
            assert_eq!(press(&mut m, 50, key, ALT), PASS, "{key:?}");
            assert_eq!(m.phase(), Phase::PassThrough, "{key:?}");
            assert_eq!(m.deadline(), None, "{key:?}");
        }
    }

    #[test]
    fn escape_is_not_consumed_while_armed() {
        let mut m = armed();
        let answer = press(&mut m, 50, Key::Escape, ALT);
        assert!(!answer.consume);
        assert_eq!(release(&mut m, 80, Key::Escape, ALT), PASS);
    }

    #[test]
    fn a_pointer_press_goes_to_pass_through() {
        let mut m = armed();
        assert_eq!(event(&mut m, 50, Event::PointerDown), PASS);
        assert_eq!(m.phase(), Phase::PassThrough);
        assert_eq!(m.deadline(), None);
    }

    #[test]
    fn dismiss_tile_and_chip_events_are_ignored() {
        let mut m = armed();
        for e in [
            Event::Dismiss,
            Event::TileClicked { group: SEARCH },
            Event::ChipClicked { index: 0 },
        ] {
            assert_eq!(event(&mut m, 50, e), PASS, "{e:?}");
            assert_eq!(m.phase(), Phase::Armed, "{e:?}");
        }
        assert_eq!(m.deadline(), Some(ms(400)));
    }

    #[test]
    fn a_key_event_with_alt_released_in_its_modifiers_goes_idle_first_and_passes() {
        let mut m = armed();
        assert_eq!(press(&mut m, 50, S, NONE), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn a_key_event_with_another_modifier_added_goes_to_pass_through_first() {
        let mut m = armed();
        assert_eq!(press(&mut m, 50, S, ALT_CMD), PASS);
        assert_eq!(m.phase(), Phase::PassThrough);
    }

    #[test]
    fn the_up_of_a_key_pressed_before_alt_changes_nothing() {
        // S went down before ⌥ and comes up while Armed.
        let mut m = armed();
        assert_eq!(release(&mut m, 50, S, ALT), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        assert_eq!(m.deadline(), Some(ms(400)));
    }

    #[test]
    fn with_no_hold_path_there_is_no_deadline_and_the_card_never_opens() {
        // OutsideText has no hold path.
        let mut m = armed_in(KeyCapture::OutsideText);
        assert_eq!(m.deadline(), None);
        assert_eq!(m.poll(ms(60_000)), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        // Still armed: a letter outside a text field switches.
        assert_eq!(press(&mut m, 61_000, S, ALT), switch_to(group(SEARCH)));
    }

    #[test]
    fn with_no_keyed_group_there_is_no_hold_path() {
        // D2: nothing to pick, so no card.
        let mut m = OptionKeys::new(config(KeyCapture::OutsideTextOrHold));
        modifiers(&mut m, 0, ALT);
        assert_eq!(m.phase(), Phase::Armed);
        assert_eq!(m.deadline(), None);
        assert_eq!(m.poll(ms(60_000)), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        // A group that appears while ⌥ is still down gives the hold path back.
        assert_eq!(m.set_groups(&groups()), PASS);
        assert_eq!(m.deadline(), Some(ms(400)));
        assert_eq!(m.poll(ms(60_000)), card(CardChange::Show));
    }

    #[test]
    fn the_last_group_going_away_while_armed_takes_the_deadline_away() {
        let mut m = armed();
        assert_eq!(m.deadline(), Some(ms(400)));
        assert_eq!(m.set_groups(&[]), PASS);
        assert_eq!(m.deadline(), None);
        assert_eq!(m.poll(ms(60_000)), PASS);
        assert_eq!(m.phase(), Phase::Armed);
    }

    #[test]
    fn the_deadline_with_the_hold_path_opens_the_card() {
        for capture in [KeyCapture::OutsideTextOrHold, KeyCapture::Always] {
            let mut m = armed_in(capture);
            assert_eq!(m.deadline(), Some(ms(400)), "{capture:?}");
            assert_eq!(m.poll(ms(400)), card(CardChange::Show), "{capture:?}");
            assert_eq!(m.phase(), Phase::Overlay, "{capture:?}");
        }
    }

    #[test]
    fn a_composition_opening_before_the_deadline_cancels_it() {
        let mut m = armed();
        m.set_focus(composing());
        assert_eq!(m.deadline(), None);
        assert_eq!(m.poll(ms(5_000)), PASS);
        assert_eq!(m.phase(), Phase::PassThrough);
        assert_eq!(m.view(), HIDDEN);
    }
}

// ---------------------------------------------------------------------------------------------
// PassThrough
// ---------------------------------------------------------------------------------------------

mod pass_through {
    use super::*;

    #[test]
    fn everything_passes() {
        let mut m = pass_through();
        for (i, key) in [
            S,
            A,
            digit(1),
            digit(9),
            Key::Escape,
            Key::Arrow,
            Key::Modifier,
            Key::Other(2),
        ]
        .into_iter()
        .enumerate()
        {
            let t = 100 + 10 * i as u64;
            assert_eq!(press(&mut m, t, key, ALT), PASS, "{key:?}");
            assert_eq!(autorepeat(&mut m, t + 1, key, ALT), PASS, "{key:?}");
            assert_eq!(release(&mut m, t + 2, key, ALT), PASS, "{key:?}");
        }
        for e in [
            Event::PointerDown,
            Event::TileClicked { group: SEARCH },
            Event::ChipClicked { index: 0 },
            Event::Dismiss,
        ] {
            assert_eq!(event(&mut m, 500, e), PASS, "{e:?}");
        }
        assert_eq!(m.phase(), Phase::PassThrough);
        assert_eq!(m.view(), HIDDEN);
    }

    #[test]
    fn there_is_no_deadline_and_no_card() {
        let mut m = pass_through();
        assert_eq!(m.deadline(), None);
        assert_eq!(m.poll(ms(60_000)), PASS);
        assert_eq!(m.phase(), Phase::PassThrough);
    }

    #[test]
    fn only_alt_up_gives_idle() {
        let mut m = pass_through();
        // More modifiers, fewer modifiers, ⌥ still down: nothing changes.
        for mods in [ALT_CMD, ALT, ALT_SHIFT, ALT] {
            assert_eq!(modifiers(&mut m, 100, mods), PASS);
            assert_eq!(m.phase(), Phase::PassThrough, "{mods:?}");
        }
        assert_eq!(modifiers(&mut m, 200, NONE), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn alt_released_together_with_a_new_modifier_gives_idle() {
        let mut m = pass_through();
        assert_eq!(modifiers(&mut m, 200, CMD), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn a_new_alt_press_arms_again_after_the_release() {
        let mut m = pass_through();
        modifiers(&mut m, 200, NONE);
        assert_eq!(modifiers(&mut m, 300, ALT), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        assert_eq!(m.deadline(), Some(ms(700)));
    }
}

// ---------------------------------------------------------------------------------------------
// Switched
// ---------------------------------------------------------------------------------------------

mod switched {
    use super::*;

    #[test]
    fn another_assigned_letter_switches_again() {
        let mut m = switched();
        assert_eq!(press(&mut m, 100, A, ALT), switch_to(group(ART)));
        assert_eq!(m.phase(), Phase::Switched);
        // Digits now belong to Art, which has five tabs.
        assert_eq!(press(&mut m, 150, digit(5), ALT), switch_to(tab(ART, 4)));
        assert_eq!(
            press(&mut m, 160, digit(5), ALT).activate,
            Some(tab(ART, 4))
        );
    }

    #[test]
    fn the_same_letter_again_switches_again() {
        let mut m = switched();
        assert_eq!(release(&mut m, 80, S, ALT), swallowed());
        assert_eq!(press(&mut m, 100, S, ALT), switch_to(group(SEARCH)));
        assert_eq!(m.phase(), Phase::Switched);
    }

    #[test]
    fn an_auto_repeat_of_an_assigned_letter_is_consumed_with_no_effect() {
        let mut m = switched();
        // The held S itself repeats.
        assert_eq!(autorepeat(&mut m, 600, S, ALT), swallowed());
        // A letter that was held before and repeats is consumed as well, and switches nothing.
        assert_eq!(autorepeat(&mut m, 650, A, ALT), swallowed());
        assert_eq!(m.phase(), Phase::Switched);
        assert_eq!(press(&mut m, 700, digit(1), ALT), switch_to(tab(SEARCH, 0)));
    }

    #[test]
    fn a_digit_picks_that_tab_of_the_group() {
        let mut m = switched();
        assert_eq!(press(&mut m, 100, digit(1), ALT), switch_to(tab(SEARCH, 0)));
        assert_eq!(press(&mut m, 120, digit(2), ALT), switch_to(tab(SEARCH, 1)));
        assert_eq!(press(&mut m, 140, digit(3), ALT), switch_to(tab(SEARCH, 2)));
        assert_eq!(m.phase(), Phase::Switched);
        assert_eq!(m.view(), HIDDEN);
    }

    #[test]
    fn digit_nine_reaches_the_ninth_tab_of_a_long_group() {
        let mut m = armed();
        press(&mut m, 50, R, ALT);
        assert_eq!(press(&mut m, 100, digit(9), ALT), switch_to(tab(READ, 8)));
        assert_eq!(press(&mut m, 120, digit(1), ALT), switch_to(tab(READ, 0)));
    }

    #[test]
    fn a_digit_above_the_tab_count_is_consumed_and_activates_nothing() {
        let mut m = switched();
        assert_eq!(press(&mut m, 100, digit(4), ALT), swallowed());
        assert_eq!(press(&mut m, 110, digit(9), ALT), swallowed());
        assert_eq!(m.phase(), Phase::Switched);
    }

    #[test]
    fn a_repeated_digit_is_consumed_and_activates_nothing_more() {
        let mut m = switched();
        assert_eq!(press(&mut m, 100, digit(2), ALT), switch_to(tab(SEARCH, 1)));
        assert_eq!(autorepeat(&mut m, 150, digit(2), ALT), swallowed());
        assert_eq!(autorepeat(&mut m, 200, digit(3), ALT), swallowed());
    }

    #[test]
    fn a_digit_outside_one_to_nine_passes_like_any_other_key() {
        let mut m = switched();
        assert_eq!(press(&mut m, 100, digit(0), ALT), PASS);
        assert_eq!(press(&mut m, 110, digit(10), ALT), PASS);
        assert_eq!(m.phase(), Phase::Switched);
    }

    #[test]
    fn an_unassigned_letter_escape_arrows_and_other_keys_pass_and_stay() {
        let mut m = switched();
        for key in [Q, Key::Escape, Key::Arrow, Key::Modifier, Key::Other(9)] {
            assert_eq!(press(&mut m, 100, key, ALT), PASS, "{key:?}");
            assert_eq!(m.phase(), Phase::Switched, "{key:?}");
        }
        // Still Switched to Search.
        assert_eq!(press(&mut m, 200, digit(1), ALT), switch_to(tab(SEARCH, 0)));
    }

    #[test]
    fn alt_up_gives_idle() {
        let mut m = switched();
        assert_eq!(modifiers(&mut m, 200, NONE), PASS);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
    }

    #[test]
    fn adding_command_control_or_shift_gives_pass_through() {
        for added in [ALT_CMD, ALT_CTRL, ALT_SHIFT] {
            let mut m = switched();
            assert_eq!(modifiers(&mut m, 200, added), PASS);
            assert_eq!(m.phase(), Phase::PassThrough, "{added:?}");
            assert_eq!(press(&mut m, 250, digit(1), added), PASS, "{added:?}");
        }
    }

    #[test]
    fn there_is_no_card_and_no_deadline() {
        let mut m = switched();
        assert_eq!(m.deadline(), None);
        assert_eq!(m.poll(ms(60_000)), PASS);
        assert_eq!(m.phase(), Phase::Switched);
        assert_eq!(m.view(), HIDDEN);
    }

    #[test]
    fn card_events_are_ignored() {
        let mut m = switched();
        for e in [
            Event::PointerDown,
            Event::TileClicked { group: ART },
            Event::ChipClicked { index: 0 },
            Event::Dismiss,
        ] {
            assert_eq!(event(&mut m, 100, e), PASS, "{e:?}");
            assert_eq!(m.phase(), Phase::Switched, "{e:?}");
        }
    }

    #[test]
    fn the_tab_count_follows_the_groups() {
        let mut m = switched();
        assert_eq!(m.set_groups(&[kg(SEARCH, LetterKey::S, 1)]), PASS);
        assert_eq!(press(&mut m, 100, digit(2), ALT), swallowed());
        assert_eq!(press(&mut m, 110, digit(1), ALT), switch_to(tab(SEARCH, 0)));
        // The group is gone: digits are consumed and go nowhere.
        assert_eq!(m.set_groups(&[kg(ART, LetterKey::A, 5)]), PASS);
        assert_eq!(press(&mut m, 120, digit(1), ALT), swallowed());
        assert_eq!(m.phase(), Phase::Switched);
    }

    #[test]
    fn a_composition_opening_ends_the_capture_and_the_page_gets_the_keys() {
        let mut m = switched();
        assert_eq!(m.set_focus(composing()), PASS);
        assert_eq!(m.phase(), Phase::PassThrough);
        // Neither a digit, nor a group letter, nor a repeat of the held S is captured any more.
        assert_eq!(press(&mut m, 100, digit(2), ALT), PASS);
        assert_eq!(press(&mut m, 110, A, ALT), PASS);
        // The S that was captured keeps its key-up in step with its down.
        assert_eq!(release(&mut m, 120, S, ALT), swallowed());
        assert_eq!(modifiers(&mut m, 130, NONE), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }
}

// ---------------------------------------------------------------------------------------------
// Overlay
// ---------------------------------------------------------------------------------------------

mod overlay {
    use super::*;

    #[test]
    fn the_card_starts_with_nothing_selected() {
        let m = overlay();
        assert_eq!(m.view(), shown(None, None));
    }

    #[test]
    fn an_assigned_letter_selects_its_group() {
        let mut m = overlay();
        assert_eq!(
            press(&mut m, 500, S, ALT),
            swallowed_with(CardChange::Update)
        );
        assert_eq!(m.view(), shown(Some(SEARCH), None));
        assert_eq!(m.phase(), Phase::Overlay);
    }

    #[test]
    fn another_letter_changes_the_group_and_clears_the_tab() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        press(&mut m, 510, digit(2), ALT);
        assert_eq!(m.view(), shown(Some(SEARCH), Some(2)));
        assert_eq!(
            press(&mut m, 520, A, ALT),
            swallowed_with(CardChange::Update)
        );
        assert_eq!(m.view(), shown(Some(ART), None));
    }

    #[test]
    fn the_chosen_letter_again_clears_the_tab_and_is_otherwise_no_change() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(press(&mut m, 510, S, ALT), swallowed());
        press(&mut m, 520, digit(2), ALT);
        assert_eq!(
            press(&mut m, 530, S, ALT),
            swallowed_with(CardChange::Update)
        );
        assert_eq!(m.view(), shown(Some(SEARCH), None));
    }

    #[test]
    fn an_auto_repeat_of_an_assigned_letter_is_consumed_with_no_effect() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(autorepeat(&mut m, 700, S, ALT), swallowed());
        // A letter that was held before the card opened and repeats does not select anything.
        assert_eq!(autorepeat(&mut m, 710, A, ALT), swallowed());
        assert_eq!(m.view(), shown(Some(SEARCH), None));
    }

    #[test]
    fn a_digit_selects_that_tab_of_the_chosen_group() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(
            press(&mut m, 510, digit(2), ALT),
            swallowed_with(CardChange::Update)
        );
        assert_eq!(m.view(), shown(Some(SEARCH), Some(2)));
        assert_eq!(
            press(&mut m, 520, digit(3), ALT),
            swallowed_with(CardChange::Update)
        );
        assert_eq!(m.view(), shown(Some(SEARCH), Some(3)));
    }

    #[test]
    fn the_same_digit_again_is_no_change() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        press(&mut m, 510, digit(2), ALT);
        assert_eq!(press(&mut m, 520, digit(2), ALT), swallowed());
        assert_eq!(autorepeat(&mut m, 530, digit(2), ALT), swallowed());
        assert_eq!(m.view(), shown(Some(SEARCH), Some(2)));
    }

    #[test]
    fn a_digit_above_the_tab_count_has_no_effect() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(press(&mut m, 510, digit(4), ALT), swallowed());
        assert_eq!(m.view(), shown(Some(SEARCH), None));
        press(&mut m, 520, digit(3), ALT);
        assert_eq!(press(&mut m, 530, digit(4), ALT), swallowed());
        assert_eq!(m.view(), shown(Some(SEARCH), Some(3)));
    }

    #[test]
    fn a_digit_with_no_group_selected_has_no_effect() {
        let mut m = overlay();
        assert_eq!(press(&mut m, 500, digit(1), ALT), swallowed());
        assert_eq!(m.view(), shown(None, None));
    }

    #[test]
    fn digit_nine_selects_the_ninth_tab_of_a_long_group() {
        let mut m = overlay();
        press(&mut m, 500, R, ALT);
        assert_eq!(
            press(&mut m, 510, digit(9), ALT),
            swallowed_with(CardChange::Update)
        );
        assert_eq!(m.view(), shown(Some(READ), Some(9)));
        assert_eq!(
            modifiers(&mut m, 600, NONE),
            activates(tab(READ, 8), CardChange::Hide)
        );
    }

    #[test]
    fn an_unassigned_letter_other_keys_and_odd_digits_are_consumed_with_no_effect() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        for key in [Q, Key::Other(77), digit(0), digit(10), digit(200)] {
            assert_eq!(press(&mut m, 600, key, ALT), swallowed(), "{key:?}");
            assert_eq!(autorepeat(&mut m, 610, key, ALT), swallowed(), "{key:?}");
            assert_eq!(m.view(), shown(Some(SEARCH), None), "{key:?}");
        }
        assert_eq!(m.phase(), Phase::Overlay);
    }

    #[test]
    fn arrows_and_modifier_keys_pass_and_change_nothing() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        for key in [Key::Arrow, Key::Modifier] {
            assert_eq!(press(&mut m, 600, key, ALT), PASS, "{key:?}");
            assert_eq!(autorepeat(&mut m, 610, key, ALT), PASS, "{key:?}");
            assert_eq!(release(&mut m, 620, key, ALT), PASS, "{key:?}");
        }
        assert_eq!(m.view(), shown(Some(SEARCH), None));
    }

    #[test]
    fn escape_is_consumed_hides_the_card_and_changes_nothing() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        press(&mut m, 510, digit(1), ALT);
        assert_eq!(
            press(&mut m, 600, Key::Escape, ALT),
            swallowed_with(CardChange::Hide)
        );
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
        // The key-up of Escape is the card's too.
        assert_eq!(release(&mut m, 650, Key::Escape, ALT), swallowed());
    }

    #[test]
    fn after_escape_the_hold_is_over_and_nothing_re_arms_by_itself() {
        // D10: ⌥ is still down, the machine is Idle, and a letter is the page's.
        let mut m = overlay();
        press(&mut m, 500, Key::Escape, ALT);
        assert_eq!(press(&mut m, 600, S, ALT), PASS);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.poll(ms(60_000)), PASS);
        // Another ⌥ press, after the release, arms again.
        modifiers(&mut m, 700, NONE);
        modifiers(&mut m, 800, ALT);
        assert_eq!(m.phase(), Phase::Armed);
    }

    #[test]
    fn alt_up_commits_the_tab_when_one_is_chosen() {
        let mut m = overlay();
        press(&mut m, 500, A, ALT);
        press(&mut m, 510, digit(4), ALT);
        assert_eq!(
            modifiers(&mut m, 600, NONE),
            activates(tab(ART, 3), CardChange::Hide)
        );
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
    }

    #[test]
    fn alt_up_commits_the_group_when_only_a_group_is_chosen() {
        let mut m = overlay();
        press(&mut m, 500, E, ALT);
        assert_eq!(
            modifiers(&mut m, 600, NONE),
            activates(group(MAIL), CardChange::Hide)
        );
    }

    #[test]
    fn alt_up_with_nothing_chosen_activates_nothing_and_hides() {
        let mut m = overlay();
        assert_eq!(modifiers(&mut m, 600, NONE), card(CardChange::Hide));
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn alt_up_wins_over_another_modifier_changing_in_the_same_event() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(
            modifiers(&mut m, 600, SHIFT),
            activates(group(SEARCH), CardChange::Hide)
        );
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn alt_up_carried_by_a_key_event_commits_and_the_key_passes() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        press(&mut m, 510, digit(1), ALT);
        // The page's key arrives with ⌥ already up, before any ModifiersChanged.
        assert_eq!(
            press(&mut m, 600, Q, NONE),
            activates(tab(SEARCH, 0), CardChange::Hide)
        );
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn adding_command_control_or_shift_with_alt_still_down_hides_and_activates_nothing() {
        for added in [ALT_CMD, ALT_CTRL, ALT_SHIFT] {
            let mut m = overlay();
            press(&mut m, 500, S, ALT);
            press(&mut m, 510, digit(2), ALT);
            assert_eq!(
                modifiers(&mut m, 600, added),
                card(CardChange::Hide),
                "{added:?}"
            );
            assert_eq!(m.phase(), Phase::PassThrough, "{added:?}");
            assert_eq!(m.view(), HIDDEN, "{added:?}");
            // The rest of the hold belongs to the page, and ⌥ up activates nothing.
            assert_eq!(press(&mut m, 650, A, added), PASS, "{added:?}");
            assert_eq!(modifiers(&mut m, 700, NONE), PASS, "{added:?}");
        }
    }

    #[test]
    fn a_pointer_press_or_dismiss_hides_and_activates_nothing() {
        for e in [Event::PointerDown, Event::Dismiss] {
            let mut m = overlay();
            press(&mut m, 500, S, ALT);
            assert_eq!(event(&mut m, 600, e), card(CardChange::Hide), "{e:?}");
            assert_eq!(m.phase(), Phase::Idle, "{e:?}");
            assert_eq!(m.view(), HIDDEN, "{e:?}");
        }
    }

    #[test]
    fn a_tile_click_activates_that_group_and_hides() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(
            event(&mut m, 600, Event::TileClicked { group: ART }),
            activates(group(ART), CardChange::Hide)
        );
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
        // ⌥ coming up afterwards activates nothing more.
        assert_eq!(modifiers(&mut m, 700, NONE), PASS);
    }

    #[test]
    fn a_tile_click_with_nothing_selected_works_too() {
        let mut m = overlay();
        assert_eq!(
            event(&mut m, 600, Event::TileClicked { group: READ }),
            activates(group(READ), CardChange::Hide)
        );
    }

    #[test]
    fn a_tile_click_on_an_unknown_group_is_ignored() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(event(&mut m, 600, Event::TileClicked { group: 999 }), PASS);
        assert_eq!(m.phase(), Phase::Overlay);
        assert_eq!(m.view(), shown(Some(SEARCH), None));
    }

    #[test]
    fn a_chip_click_opens_that_tab_of_the_chosen_group() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(
            event(&mut m, 600, Event::ChipClicked { index: 2 }),
            activates(tab(SEARCH, 2), CardChange::Hide)
        );
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
    }

    #[test]
    fn a_chip_click_without_a_group_or_beyond_the_tabs_is_ignored() {
        let mut m = overlay();
        assert_eq!(event(&mut m, 500, Event::ChipClicked { index: 0 }), PASS);
        press(&mut m, 510, S, ALT);
        // Search has three tabs: index 3 does not exist.
        assert_eq!(event(&mut m, 520, Event::ChipClicked { index: 3 }), PASS);
        assert_eq!(
            event(&mut m, 530, Event::ChipClicked { index: usize::MAX }),
            PASS
        );
        assert_eq!(m.phase(), Phase::Overlay);
        assert_eq!(m.view(), shown(Some(SEARCH), None));
    }

    #[test]
    fn only_the_first_nine_chips_exist() {
        let mut m = overlay();
        press(&mut m, 500, R, ALT);
        // Reading has twelve tabs, and a digit reaches only 1..=9.
        assert_eq!(event(&mut m, 510, Event::ChipClicked { index: 9 }), PASS);
        assert_eq!(m.phase(), Phase::Overlay);
        assert_eq!(
            event(&mut m, 520, Event::ChipClicked { index: 8 }),
            activates(tab(READ, 8), CardChange::Hide)
        );
    }

    #[test]
    fn a_composition_cancels_the_card() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(m.set_focus(composing()), card(CardChange::Hide));
        assert_eq!(m.phase(), Phase::PassThrough);
        assert_eq!(m.view(), HIDDEN);
        assert_eq!(press(&mut m, 600, A, ALT), PASS);
        assert_eq!(modifiers(&mut m, 700, NONE), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn moving_into_a_text_field_does_not_close_the_card() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(m.set_focus(typing()), PASS);
        assert_eq!(m.phase(), Phase::Overlay);
        assert_eq!(m.view(), shown(Some(SEARCH), None));
    }

    #[test]
    fn disabling_the_keys_closes_the_card_and_goes_idle() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        let off = Config::new(false, KeyCapture::OutsideTextOrHold, ms(400));
        assert_eq!(m.set_config(off), card(CardChange::Hide));
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
        assert_eq!(modifiers(&mut m, 700, NONE), PASS);
    }

    #[test]
    fn a_mode_without_a_hold_path_closes_the_card_and_goes_to_pass_through() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(
            m.set_config(config(KeyCapture::OutsideText)),
            card(CardChange::Hide)
        );
        assert_eq!(m.phase(), Phase::PassThrough);
        assert_eq!(press(&mut m, 600, S, ALT), PASS);
        assert_eq!(modifiers(&mut m, 700, NONE), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn a_config_change_that_keeps_the_hold_path_keeps_the_card() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        assert_eq!(m.set_config(config(KeyCapture::Always)), PASS);
        assert_eq!(
            m.set_config(Config::new(true, KeyCapture::Always, ms(900))),
            PASS
        );
        assert_eq!(m.phase(), Phase::Overlay);
        assert_eq!(m.view(), shown(Some(SEARCH), None));
    }

    #[test]
    fn a_selected_group_that_goes_away_is_cleared() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        press(&mut m, 510, digit(2), ALT);
        let without_search = [kg(ART, LetterKey::A, 5)];
        assert_eq!(m.set_groups(&without_search), card(CardChange::Update));
        assert_eq!(m.view(), shown(None, None));
        assert_eq!(m.phase(), Phase::Overlay);
        // The card stays up; ⌥ up has nothing to commit.
        assert_eq!(modifiers(&mut m, 600, NONE), card(CardChange::Hide));
    }

    #[test]
    fn a_selected_digit_above_the_new_tab_count_is_cleared() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        press(&mut m, 510, digit(3), ALT);
        let shrunk = [kg(SEARCH, LetterKey::S, 2), kg(ART, LetterKey::A, 5)];
        assert_eq!(m.set_groups(&shrunk), card(CardChange::Update));
        assert_eq!(m.view(), shown(Some(SEARCH), None));
    }

    #[test]
    fn a_selection_that_still_exists_is_kept() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        press(&mut m, 510, digit(2), ALT);
        let grown = [kg(ART, LetterKey::A, 1), kg(SEARCH, LetterKey::S, 9)];
        assert_eq!(m.set_groups(&grown), PASS);
        assert_eq!(m.view(), shown(Some(SEARCH), Some(2)));
        // A selected digit equal to the new tab count is still valid.
        let exact = [kg(SEARCH, LetterKey::S, 2)];
        assert_eq!(m.set_groups(&exact), PASS);
        assert_eq!(m.view(), shown(Some(SEARCH), Some(2)));
    }

    #[test]
    fn there_is_no_timeout() {
        let mut m = overlay();
        assert_eq!(m.poll(ms(3_600_000)), PASS);
        assert_eq!(m.view(), shown(None, None));
        assert_eq!(m.phase(), Phase::Overlay);
        assert_eq!(m.deadline(), None);
    }

    #[test]
    fn the_card_opens_again_for_the_next_hold() {
        let mut m = overlay();
        modifiers(&mut m, 600, NONE);
        modifiers(&mut m, 1_000, ALT);
        assert_eq!(m.poll(ms(1_400)), card(CardChange::Show));
        assert_eq!(m.view(), shown(None, None));
    }
}

// ---------------------------------------------------------------------------------------------
// The hold timer
// ---------------------------------------------------------------------------------------------

mod hold {
    use super::*;

    #[test]
    fn nothing_at_399_ms_and_the_card_at_400_ms() {
        let mut m = armed();
        assert_eq!(m.poll(ms(399)), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        assert_eq!(m.view(), HIDDEN);
        assert_eq!(m.poll(ms(400)), card(CardChange::Show));
        assert_eq!(m.phase(), Phase::Overlay);
        assert_eq!(m.view(), shown(None, None));
    }

    #[test]
    fn the_hold_counts_from_when_alt_went_down() {
        let mut m = machine();
        modifiers(&mut m, 5_000, ALT);
        assert_eq!(m.deadline(), Some(ms(5_400)));
        assert_eq!(m.poll(ms(5_399)), PASS);
        assert_eq!(m.poll(ms(5_400)), card(CardChange::Show));
    }

    #[test]
    fn a_shorter_hold_after_set_config_opens_sooner() {
        let mut m = armed();
        assert_eq!(
            m.set_config(Config::new(true, KeyCapture::OutsideTextOrHold, ms(250))),
            PASS
        );
        assert_eq!(m.deadline(), Some(ms(250)));
        assert_eq!(m.poll(ms(249)), PASS);
        assert_eq!(m.poll(ms(250)), card(CardChange::Show));
    }

    #[test]
    fn a_longer_hold_after_set_config_opens_later() {
        let mut m = armed();
        m.set_config(Config::new(true, KeyCapture::OutsideTextOrHold, ms(1_000)));
        assert_eq!(m.deadline(), Some(ms(1_000)));
        assert_eq!(m.poll(ms(400)), PASS);
        assert_eq!(m.poll(ms(999)), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        assert_eq!(m.poll(ms(1_000)), card(CardChange::Show));
    }

    #[test]
    fn the_hold_is_the_one_in_the_config_from_the_start() {
        for (hold, early, due) in [(250, 249, 250), (700, 699, 700), (1_000, 999, 1_000)] {
            let mut m = OptionKeys::new(Config::new(true, KeyCapture::Always, ms(hold)));
            m.set_groups(&groups());
            modifiers(&mut m, 0, ALT);
            assert_eq!(m.poll(ms(early)), PASS, "hold {hold}");
            assert_eq!(m.poll(ms(due)), card(CardChange::Show), "hold {hold}");
        }
    }

    #[test]
    fn poll_shows_once_and_then_nothing() {
        let mut m = armed();
        assert_eq!(m.poll(ms(400)), card(CardChange::Show));
        assert_eq!(m.poll(ms(401)), PASS);
        assert_eq!(m.poll(ms(10_000)), PASS);
        assert_eq!(m.phase(), Phase::Overlay);
    }

    #[test]
    fn poll_never_consumes_or_activates() {
        let mut m = armed();
        let shown = m.poll(ms(400));
        assert!(!shown.consume);
        assert_eq!(shown.activate, None);
    }

    #[test]
    fn poll_in_idle_does_nothing() {
        let mut m = machine();
        assert_eq!(m.poll(ms(0)), PASS);
        assert_eq!(m.poll(ms(60_000)), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn a_poll_that_comes_after_the_alt_release_shows_nothing() {
        let mut m = armed();
        modifiers(&mut m, 200, NONE);
        assert_eq!(m.poll(ms(400)), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }

    // The deadline is lazy: no timer is needed for the next event to see the card.

    #[test]
    fn a_key_after_the_deadline_with_no_poll_behaves_as_in_overlay() {
        // In a text field, where a quick ⌥S would have passed.
        let mut m = machine();
        m.set_focus(typing());
        modifiers(&mut m, 0, ALT);
        assert_eq!(
            press(&mut m, 500, S, ALT),
            swallowed_with(CardChange::Show),
            "the card opens and S selects Search in the same call"
        );
        assert_eq!(m.phase(), Phase::Overlay);
        assert_eq!(m.view(), shown(Some(SEARCH), None));
    }

    #[test]
    fn a_key_exactly_at_the_deadline_behaves_as_in_overlay() {
        let mut m = armed();
        assert_eq!(press(&mut m, 400, S, ALT), swallowed_with(CardChange::Show));
        assert_eq!(m.phase(), Phase::Overlay);
    }

    #[test]
    fn a_key_just_before_the_deadline_behaves_as_in_armed() {
        let mut m = armed();
        assert_eq!(press(&mut m, 399, S, ALT), switch_to(group(SEARCH)));
        assert_eq!(m.phase(), Phase::Switched);
    }

    #[test]
    fn an_unassigned_key_after_the_deadline_is_swallowed_by_the_card() {
        let mut m = armed();
        // In Armed this would have passed (to the page: å or a word delete).
        assert_eq!(
            press(&mut m, 500, Key::Other(51), ALT),
            swallowed_with(CardChange::Show)
        );
        assert_eq!(m.phase(), Phase::Overlay);
    }

    #[test]
    fn escape_after_the_deadline_closes_a_card_the_caller_never_saw() {
        let mut m = armed();
        // Hidden before and hidden after: the whole call compares to no change.
        assert_eq!(press(&mut m, 500, Key::Escape, ALT), swallowed());
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn alt_up_after_the_deadline_with_no_poll_commits_nothing_and_shows_nothing() {
        let mut m = armed();
        assert_eq!(modifiers(&mut m, 500, NONE), PASS);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
    }

    #[test]
    fn a_selection_made_after_a_late_timer_is_committed_by_alt_up() {
        let mut m = machine();
        m.set_focus(typing());
        modifiers(&mut m, 0, ALT);
        press(&mut m, 500, S, ALT);
        press(&mut m, 520, digit(3), ALT);
        assert_eq!(
            modifiers(&mut m, 600, NONE),
            activates(tab(SEARCH, 2), CardChange::Hide)
        );
    }

    #[test]
    fn a_late_poll_after_a_lazy_open_does_not_show_again() {
        let mut m = armed();
        press(&mut m, 500, S, ALT);
        assert_eq!(m.poll(ms(600)), PASS);
        assert_eq!(m.view(), shown(Some(SEARCH), None));
    }

    #[test]
    fn a_pointer_press_after_the_deadline_closes_the_card_the_caller_never_saw() {
        let mut m = armed();
        assert_eq!(event(&mut m, 500, Event::PointerDown), PASS);
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn time_going_backwards_counts_as_not_moving() {
        let mut m = machine();
        modifiers(&mut m, 1_000, NONE);
        // The next event claims an earlier time: the clock stays at 1000 ms.
        modifiers(&mut m, 200, ALT);
        assert_eq!(m.deadline(), Some(ms(1_400)));
        assert_eq!(m.poll(ms(600)), PASS, "200 + 400 is not the deadline");
        assert_eq!(m.poll(ms(300)), PASS);
        assert_eq!(m.poll(ms(1_399)), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        assert_eq!(m.poll(ms(1_400)), card(CardChange::Show));
    }

    #[test]
    fn a_backwards_event_does_not_open_the_card_early_either() {
        let mut m = machine();
        modifiers(&mut m, 1_000, ALT);
        assert_eq!(press(&mut m, 100, S, ALT), switch_to(group(SEARCH)));
        assert_eq!(m.phase(), Phase::Switched);
    }

    #[test]
    fn huge_times_do_not_panic() {
        let mut m = machine();
        modifiers(&mut m, 0, NONE);
        let forever = Duration::MAX;
        assert_eq!(m.on_event(forever, Event::Modifiers(ALT)), PASS);
        assert_eq!(m.deadline(), Some(forever));
        let _ = m.poll(forever);
        let _ = m.on_event(forever, Event::Modifiers(NONE));
        assert_eq!(m.phase(), Phase::Idle);
    }
}

// ---------------------------------------------------------------------------------------------
// deadline() and view()
// ---------------------------------------------------------------------------------------------

mod deadline_and_view {
    use super::*;

    #[test]
    fn the_deadline_exists_only_in_armed_with_a_hold_path() {
        for (phase, m) in every_phase() {
            let expected = (phase == Phase::Armed).then(|| ms(400));
            assert_eq!(m.deadline(), expected, "{phase:?}");
        }
    }

    #[test]
    fn the_deadline_needs_the_keys_on_a_mode_with_a_hold_path_and_a_keyed_group() {
        let on = |capture| Config::new(true, capture, ms(400));
        let off = Config::new(false, KeyCapture::Always, ms(400));
        let mut m = armed();
        assert_eq!(m.deadline(), Some(ms(400)));
        m.set_config(on(KeyCapture::Always));
        assert_eq!(m.deadline(), Some(ms(400)));
        m.set_config(on(KeyCapture::OutsideText));
        assert_eq!(m.deadline(), None);
        m.set_config(on(KeyCapture::OutsideTextOrHold));
        assert_eq!(m.deadline(), Some(ms(400)));
        m.set_groups(&[]);
        assert_eq!(m.deadline(), None);
        m.set_groups(&groups());
        assert_eq!(m.deadline(), Some(ms(400)));
        // Disabling also ends the capture: the machine is Idle and has no deadline.
        m.set_config(off);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.deadline(), None);
    }

    #[test]
    fn the_view_is_visible_only_in_overlay() {
        for (phase, m) in every_phase() {
            let expected = if phase == Phase::Overlay {
                shown(None, None)
            } else {
                HIDDEN
            };
            assert_eq!(m.view(), expected, "{phase:?}");
        }
    }

    #[test]
    fn the_phase_names_the_state() {
        for (phase, m) in every_phase() {
            assert_eq!(m.phase(), phase);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Any phase
// ---------------------------------------------------------------------------------------------

mod any_phase {
    use super::*;

    #[test]
    fn reset_goes_idle_and_hides_a_visible_card() {
        for (phase, mut m) in every_phase() {
            let expected = if phase == Phase::Overlay {
                card(CardChange::Hide)
            } else {
                PASS
            };
            assert_eq!(event(&mut m, 900, Event::Reset), expected, "{phase:?}");
            assert_eq!(m.phase(), Phase::Idle, "{phase:?}");
            assert_eq!(m.view(), HIDDEN, "{phase:?}");
            assert_eq!(m.deadline(), None, "{phase:?}");
        }
    }

    #[test]
    fn reset_clears_the_selection() {
        let mut m = overlay();
        press(&mut m, 500, S, ALT);
        press(&mut m, 510, digit(2), ALT);
        event(&mut m, 600, Event::Reset);
        // A fresh hold starts with nothing selected and commits nothing.
        modifiers(&mut m, 700, NONE);
        modifiers(&mut m, 800, ALT);
        assert_eq!(m.poll(ms(1_200)), card(CardChange::Show));
        assert_eq!(m.view(), shown(None, None));
    }

    #[test]
    fn reset_forgets_the_tracked_modifiers() {
        // ⌥ was down before the reset. After it the next ⌥ event is a new edge.
        let mut m = overlay();
        event(&mut m, 600, Event::Reset);
        assert_eq!(modifiers(&mut m, 700, ALT), PASS);
        assert_eq!(m.phase(), Phase::Armed);
        assert_eq!(m.deadline(), Some(ms(1_100)));
    }

    #[test]
    fn reset_keeps_the_groups_the_config_and_the_focus() {
        let mut m = machine();
        m.set_focus(typing());
        event(&mut m, 0, Event::Reset);
        modifiers(&mut m, 10, ALT);
        // Still in a text field (the focus is kept): a quick ⌥S passes.
        assert_eq!(press(&mut m, 50, S, ALT), PASS);
        // Still keyed and still on: outside a text field the letter switches.
        let mut m = machine();
        event(&mut m, 0, Event::Reset);
        modifiers(&mut m, 10, ALT);
        assert_eq!(press(&mut m, 50, S, ALT), switch_to(group(SEARCH)));
        // Still the configured hold: 400 ms, not a default of the reset.
        let mut m = OptionKeys::new(Config::new(true, KeyCapture::Always, ms(700)));
        m.set_groups(&groups());
        event(&mut m, 0, Event::Reset);
        modifiers(&mut m, 10, ALT);
        assert_eq!(m.deadline(), Some(ms(710)));
    }

    #[test]
    fn a_key_up_is_consumed_if_and_only_if_its_down_was() {
        let mut m = armed();
        // S is captured, Q passes.
        assert_eq!(press(&mut m, 50, S, ALT), switch_to(group(SEARCH)));
        assert_eq!(press(&mut m, 60, Q, ALT), PASS);
        assert_eq!(release(&mut m, 100, S, ALT), swallowed());
        assert_eq!(release(&mut m, 110, Q, ALT), PASS);
        // Each captured down is answered once.
        assert_eq!(release(&mut m, 120, S, ALT), PASS);
    }

    #[test]
    fn a_key_up_never_seen_down_passes() {
        let mut m = overlay();
        assert_eq!(release(&mut m, 500, S, ALT), PASS);
        assert_eq!(release(&mut m, 510, Key::Other(8), ALT), PASS);
        assert_eq!(m.phase(), Phase::Overlay);
    }

    #[test]
    fn the_memory_outlives_the_alt_release() {
        let mut m = armed();
        press(&mut m, 50, S, ALT);
        press(&mut m, 60, digit(2), ALT);
        // ⌥ comes up first, then the letters.
        assert_eq!(modifiers(&mut m, 100, NONE), PASS);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(release(&mut m, 110, S, NONE), swallowed());
        assert_eq!(release(&mut m, 120, digit(2), NONE), swallowed());
        assert_eq!(release(&mut m, 130, S, NONE), PASS);
    }

    #[test]
    fn the_memory_outlives_the_alt_release_announced_by_the_key_event_itself() {
        // No ModifiersChanged before the up: the key-up carries the new modifiers.
        let mut m = armed();
        press(&mut m, 50, S, ALT);
        assert_eq!(release(&mut m, 100, S, NONE), swallowed());
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn the_memory_covers_every_key_the_card_swallowed() {
        let mut m = overlay();
        let swallowed_keys = [
            S,
            digit(1),
            Q,
            Key::Other(5),
            digit(0),
            digit(10),
            Key::Escape,
        ];
        for (i, key) in swallowed_keys.into_iter().enumerate() {
            let t = 500 + i as u64;
            // Escape closes the card, which is why it comes last.
            assert!(press(&mut m, t, key, ALT).consume, "{key:?}");
        }
        modifiers(&mut m, 600, NONE);
        for key in swallowed_keys {
            assert_eq!(release(&mut m, 700, key, NONE), swallowed(), "{key:?}");
        }
    }

    #[test]
    fn arrows_and_modifier_keys_are_never_remembered() {
        let mut m = overlay();
        press(&mut m, 500, Key::Arrow, ALT);
        press(&mut m, 501, Key::Modifier, ALT);
        modifiers(&mut m, 600, NONE);
        assert_eq!(release(&mut m, 700, Key::Arrow, NONE), PASS);
        assert_eq!(release(&mut m, 701, Key::Modifier, NONE), PASS);
    }

    #[test]
    fn a_passed_down_is_not_remembered() {
        let mut m = pass_through();
        press(&mut m, 100, S, ALT);
        modifiers(&mut m, 200, NONE);
        assert_eq!(release(&mut m, 300, S, NONE), PASS);
        assert_eq!(release(&mut m, 310, Q, NONE), PASS);
    }

    #[test]
    fn reset_clears_the_key_up_memory() {
        let mut m = armed();
        press(&mut m, 50, S, ALT);
        event(&mut m, 60, Event::Reset);
        assert_eq!(release(&mut m, 100, S, NONE), PASS);
    }

    #[test]
    fn many_distinct_swallowed_keys_are_all_remembered() {
        let mut m = overlay();
        for n in 0..300 {
            assert!(press(&mut m, 500, Key::Other(1_000 + n), ALT).consume);
        }
        modifiers(&mut m, 600, NONE);
        for n in 0..300 {
            assert!(release(&mut m, 700, Key::Other(1_000 + n), NONE).consume);
            assert!(!release(&mut m, 710, Key::Other(1_000 + n), NONE).consume);
        }
    }

    #[test]
    fn an_auto_repeat_of_a_captured_key_is_consumed_with_no_effect_after_alt_came_up() {
        let mut m = armed();
        press(&mut m, 50, S, ALT);
        modifiers(&mut m, 100, NONE);
        assert_eq!(autorepeat(&mut m, 150, S, NONE), swallowed());
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(release(&mut m, 200, S, NONE), swallowed());
        // Forgotten after the up: a repeat that arrives later passes.
        assert_eq!(autorepeat(&mut m, 250, S, NONE), PASS);
    }

    #[test]
    fn an_auto_repeat_of_a_passed_key_passes() {
        let mut m = pass_through();
        assert_eq!(autorepeat(&mut m, 100, Q, ALT), PASS);
        assert_eq!(autorepeat(&mut m, 110, S, ALT), PASS);
        let mut idle = machine();
        assert_eq!(autorepeat(&mut idle, 100, S, NONE), PASS);
    }

    #[test]
    fn modifier_events_are_never_consumed() {
        let all = [
            NONE, ALT, SHIFT, CTRL, CMD, ALT_SHIFT, ALT_CTRL, ALT_CMD, ALT, NONE,
        ];
        for (phase, mut m) in every_phase() {
            for (i, mods) in all.into_iter().enumerate() {
                let answer = modifiers(&mut m, 800 + i as u64, mods);
                assert!(!answer.consume, "{phase:?} {mods:?}");
            }
        }
    }

    #[test]
    fn disabling_through_set_config_goes_idle_hides_and_consumes_nothing() {
        let off = Config::new(false, KeyCapture::OutsideTextOrHold, ms(400));
        for (phase, mut m) in every_phase() {
            let expected = if phase == Phase::Overlay {
                card(CardChange::Hide)
            } else {
                PASS
            };
            assert_eq!(m.set_config(off), expected, "{phase:?}");
            assert_eq!(m.phase(), Phase::Idle, "{phase:?}");
            assert_eq!(m.view(), HIDDEN, "{phase:?}");
            assert_eq!(m.deadline(), None, "{phase:?}");
            assert_eq!(press(&mut m, 900, S, ALT), PASS, "{phase:?}");
            assert_eq!(press(&mut m, 910, digit(1), ALT), PASS, "{phase:?}");
            assert_eq!(m.poll(ms(60_000)), PASS, "{phase:?}");
        }
    }

    #[test]
    fn keys_work_again_after_the_keys_are_switched_back_on() {
        let mut m = machine();
        m.set_config(Config::new(false, KeyCapture::OutsideTextOrHold, ms(400)));
        m.set_config(config(KeyCapture::OutsideTextOrHold));
        modifiers(&mut m, 0, ALT);
        assert_eq!(press(&mut m, 50, S, ALT), switch_to(group(SEARCH)));
    }

    #[test]
    fn set_calls_answer_with_only_the_card_set() {
        for (phase, mut m) in every_phase() {
            for answer in [
                m.set_groups(&groups()),
                m.set_focus(Focus::default()),
                m.set_config(config(KeyCapture::OutsideTextOrHold)),
            ] {
                assert_eq!(answer, PASS, "{phase:?}");
            }
        }
    }

    #[test]
    fn a_duplicated_letter_keeps_the_first_group() {
        let mut m = OptionKeys::new(config(KeyCapture::OutsideTextOrHold));
        m.set_groups(&[
            kg(1, LetterKey::S, 2),
            kg(2, LetterKey::S, 9),
            kg(3, LetterKey::A, 1),
        ]);
        modifiers(&mut m, 0, ALT);
        assert_eq!(press(&mut m, 50, S, ALT), switch_to(group(1)));
        // The second group is gone altogether, not just unreachable by its letter.
        assert_eq!(press(&mut m, 60, digit(3), ALT), swallowed());
        assert_eq!(press(&mut m, 70, digit(2), ALT), switch_to(tab(1, 1)));
    }

    #[test]
    fn at_most_twenty_six_groups_are_kept() {
        let mut m = OptionKeys::new(config(KeyCapture::OutsideTextOrHold));
        let many: Vec<KeyedGroup> = (0..30)
            .map(|i| kg(100 + i as u64, LetterKey::ALL[i % 26], 1))
            .collect();
        m.set_groups(&many);
        modifiers(&mut m, 0, ALT);
        m.poll(ms(400));
        assert_eq!(m.phase(), Phase::Overlay);
        // The first 26 are keyed; the four that repeat a letter are not.
        assert_eq!(event(&mut m, 500, Event::TileClicked { group: 126 }), PASS);
        assert_eq!(event(&mut m, 510, Event::TileClicked { group: 129 }), PASS);
        assert_eq!(
            event(&mut m, 520, Event::TileClicked { group: 125 }),
            activates(group(125), CardChange::Hide)
        );
    }

    #[test]
    fn a_duplicated_letter_does_not_use_up_one_of_the_twenty_six_places() {
        // 27 entries: S twice at the front, then the 25 other letters. Only the second S is
        // dropped, so all 26 letters are keyed and the very last entry is among them.
        let mut entries = vec![kg(1, LetterKey::S, 1), kg(2, LetterKey::S, 1)];
        let others = LetterKey::ALL.iter().filter(|&&l| l != LetterKey::S);
        entries.extend(others.enumerate().map(|(i, &l)| kg(3 + i as u64, l, 1)));
        assert_eq!(entries.len(), 27);
        let mut m = OptionKeys::new(config(KeyCapture::OutsideTextOrHold));
        m.set_groups(&entries);
        modifiers(&mut m, 0, ALT);
        m.poll(ms(400));
        assert_eq!(event(&mut m, 500, Event::TileClicked { group: 2 }), PASS);
        assert_eq!(
            event(&mut m, 510, Event::TileClicked { group: 27 }),
            activates(group(27), CardChange::Hide)
        );
    }

    #[test]
    fn every_letter_can_be_assigned() {
        let all: Vec<KeyedGroup> = LetterKey::ALL
            .iter()
            .enumerate()
            .map(|(i, &letter)| kg(i as u64 + 1, letter, 1))
            .collect();
        for (i, &letter) in LetterKey::ALL.iter().enumerate() {
            let mut fresh = OptionKeys::new(config(KeyCapture::OutsideTextOrHold));
            fresh.set_groups(&all);
            modifiers(&mut fresh, 0, ALT);
            assert_eq!(
                press(&mut fresh, 10, Key::Letter(letter), ALT),
                switch_to(group(i as u64 + 1)),
                "{letter:?}"
            );
        }
    }

    #[test]
    fn a_digit_key_outside_one_to_nine_never_picks_a_tab() {
        let mut m = switched();
        for n in [0u8, 10, 99, 255] {
            assert_eq!(press(&mut m, 100, digit(n), ALT), PASS, "digit {n}");
        }
    }

    #[test]
    fn a_whole_session_with_no_hold_ends_clean() {
        // ⌥ S 2 outside a text field, then released in the usual order.
        let mut m = machine();
        assert_eq!(modifiers(&mut m, 0, ALT), PASS);
        assert_eq!(press(&mut m, 40, S, ALT), switch_to(group(SEARCH)));
        assert_eq!(release(&mut m, 90, S, ALT), swallowed());
        assert_eq!(press(&mut m, 130, digit(2), ALT), switch_to(tab(SEARCH, 1)));
        assert_eq!(release(&mut m, 180, digit(2), ALT), swallowed());
        assert_eq!(modifiers(&mut m, 250, NONE), PASS);
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.view(), HIDDEN);
        assert_eq!(m.deadline(), None);
    }
}
