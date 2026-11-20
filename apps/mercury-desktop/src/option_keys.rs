// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The ⌥ group keys, desktop side (SPEC design/ui-final/SPEC.md §5.1; lane brief
//! `lane-key-switcher-brief.md` §6 item 5).
//!
//! The state machine is the crate `mercury-option-keys`: pure, with injected time, tested without
//! Servo. This module is the thin glue the `input-hooks` lane will call from `keyboard.rs`:
//! winit's plain enums to the crate's `Key`, `Mods` and `Event`, the clock, the hold timer's
//! delay, and the machine itself in a thread-local. Nothing calls it yet.

#![allow(dead_code)] // used by the input-hooks lane

use std::cell::RefCell;
use std::fmt::{self, Write as _};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use mercury_option_keys::{Config, Event, Key, LetterKey, Mods, OptionKeys};
use slint::winit_030::winit;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

// Nothing in this file registers a callback with Slint or hands it a closure, so no
// `InSlintCallback::enter()` guard is needed here. The hook in `keyboard.rs` that calls these
// functions runs inside one and holds its own. Everything below runs in that hook, so nothing
// outside the tests may unwrap, expect or panic.

/// The modifier keys that are down, as the machine takes them (⌘ is winit's Super).
pub(crate) fn mods_from_winit(state: ModifiersState) -> Mods {
    Mods {
        alt: state.alt_key(),
        shift: state.shift_key(),
        control: state.control_key(),
        command: state.super_key(),
    }
}

/// The machine's view of a physical key. The key is always the physical one: ⌥S types ß, and is
/// still `KeyS` on every layout.
pub(crate) fn key_from_winit(physical: &PhysicalKey) -> Key {
    let PhysicalKey::Code(code) = physical else {
        return Key::Other(key_id(physical));
    };
    match code {
        KeyCode::KeyA => Key::Letter(LetterKey::A),
        KeyCode::KeyB => Key::Letter(LetterKey::B),
        KeyCode::KeyC => Key::Letter(LetterKey::C),
        KeyCode::KeyD => Key::Letter(LetterKey::D),
        KeyCode::KeyE => Key::Letter(LetterKey::E),
        KeyCode::KeyF => Key::Letter(LetterKey::F),
        KeyCode::KeyG => Key::Letter(LetterKey::G),
        KeyCode::KeyH => Key::Letter(LetterKey::H),
        KeyCode::KeyI => Key::Letter(LetterKey::I),
        KeyCode::KeyJ => Key::Letter(LetterKey::J),
        KeyCode::KeyK => Key::Letter(LetterKey::K),
        KeyCode::KeyL => Key::Letter(LetterKey::L),
        KeyCode::KeyM => Key::Letter(LetterKey::M),
        KeyCode::KeyN => Key::Letter(LetterKey::N),
        KeyCode::KeyO => Key::Letter(LetterKey::O),
        KeyCode::KeyP => Key::Letter(LetterKey::P),
        KeyCode::KeyQ => Key::Letter(LetterKey::Q),
        KeyCode::KeyR => Key::Letter(LetterKey::R),
        KeyCode::KeyS => Key::Letter(LetterKey::S),
        KeyCode::KeyT => Key::Letter(LetterKey::T),
        KeyCode::KeyU => Key::Letter(LetterKey::U),
        KeyCode::KeyV => Key::Letter(LetterKey::V),
        KeyCode::KeyW => Key::Letter(LetterKey::W),
        KeyCode::KeyX => Key::Letter(LetterKey::X),
        KeyCode::KeyY => Key::Letter(LetterKey::Y),
        KeyCode::KeyZ => Key::Letter(LetterKey::Z),
        // 1 to 9 pick a tab; 0 is a key like any other
        KeyCode::Digit1 | KeyCode::Numpad1 => Key::Digit(1),
        KeyCode::Digit2 | KeyCode::Numpad2 => Key::Digit(2),
        KeyCode::Digit3 | KeyCode::Numpad3 => Key::Digit(3),
        KeyCode::Digit4 | KeyCode::Numpad4 => Key::Digit(4),
        KeyCode::Digit5 | KeyCode::Numpad5 => Key::Digit(5),
        KeyCode::Digit6 | KeyCode::Numpad6 => Key::Digit(6),
        KeyCode::Digit7 | KeyCode::Numpad7 => Key::Digit(7),
        KeyCode::Digit8 | KeyCode::Numpad8 => Key::Digit(8),
        KeyCode::Digit9 | KeyCode::Numpad9 => Key::Digit(9),
        KeyCode::Escape => Key::Escape,
        // ⌥ with an arrow moves by word: never captured
        KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight => {
            Key::Arrow
        }
        // the modifier keys themselves; their state arrives as `ModifiersChanged`
        KeyCode::AltLeft
        | KeyCode::AltRight
        | KeyCode::ShiftLeft
        | KeyCode::ShiftRight
        | KeyCode::ControlLeft
        | KeyCode::ControlRight
        | KeyCode::SuperLeft
        | KeyCode::SuperRight
        | KeyCode::CapsLock
        | KeyCode::Fn => Key::Modifier,
        // `KeyCode` is non-exhaustive: every other key, and `Unidentified` above
        _ => Key::Other(key_id(physical)),
    }
}

/// The hook's key event for the machine; `None` for a synthetic one. winit replays the keys that
/// are held when the window gains focus as synthetic events, and the machine must never arm on a
/// key the user is not pressing (brief D16).
pub(crate) fn event_from_parts(
    physical: &PhysicalKey,
    state: ElementState,
    repeat: bool,
    mods: ModifiersState,
    is_synthetic: bool,
) -> Option<Event> {
    if is_synthetic {
        return None;
    }
    Some(Event::Key {
        key: key_from_winit(physical),
        mods: mods_from_winit(mods),
        down: state.is_pressed(),
        repeat,
    })
}

/// [`event_from_parts`] for a winit `KeyEvent` (one cannot be built in a test, which is why the
/// logic takes its parts).
pub(crate) fn event_from_key_event(
    event: &KeyEvent,
    mods: ModifiersState,
    is_synthetic: bool,
) -> Option<Event> {
    event_from_parts(
        &event.physical_key,
        event.state,
        event.repeat,
        mods,
        is_synthetic,
    )
}

/// Time since the first call in this process: monotonic, the origin the machine's `now` counts
/// from (the same origin on every thread).
pub(crate) fn clock_now() -> Duration {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed()
}

/// How long until the machine's deadline, for the hold timer: zero when it has passed, `None`
/// when the machine has no deadline.
pub(crate) fn poll_delay(deadline: Option<Duration>, now: Duration) -> Option<Duration> {
    deadline.map(|deadline| deadline.saturating_sub(now))
}

thread_local! {
    static MACHINE: RefCell<OptionKeys> = RefCell::new(OptionKeys::new(Config::default()));
}

/// Runs `f` on this thread's machine (the pattern of `activation.rs`: a thread-local, as the
/// winit event hook and the Slint callbacks all run on the main thread).
///
/// `f` must not call `with_machine` again. If it does, the nested call gets a throwaway machine in
/// its starting state, which passes every key, instead of a panic inside the event hook: that is a
/// bug in the caller, and the safe answer to a bug here is to leave the key to the page.
pub(crate) fn with_machine<R>(f: impl FnOnce(&mut OptionKeys) -> R) -> R {
    MACHINE.with(|machine| match machine.try_borrow_mut() {
        Ok(mut machine) => f(&mut machine),
        Err(_) => f(&mut OptionKeys::new(Config::default())),
    })
}

/// The id of a key that has no variant of its own: the 32-bit FNV-1a hash of its `Debug` name
/// (`Code(F1)`, `Unidentified(MacOS(63))`). Two different keys get different ids, apart from the
/// rare collision of any 32-bit hash (the test lists 33 keys with none); the machine only needs
/// to tell keys apart, so the number has no other meaning and may change with winit's names.
fn key_id(physical: &PhysicalKey) -> u32 {
    let mut hash = Fnv1a::new();
    // Writing into the hasher cannot fail, and no string is built.
    let _ = write!(hash, "{physical:?}");
    hash.0
}

/// 32-bit FNV-1a over the bytes written to it, so `Debug` output can be hashed without being
/// collected into a `String` first.
struct Fnv1a(u32);

impl Fnv1a {
    const OFFSET_BASIS: u32 = 0x811c_9dc5;
    const PRIME: u32 = 0x0100_0193;

    fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }
}

impl fmt::Write for Fnv1a {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            self.0 = (self.0 ^ u32::from(byte)).wrapping_mul(Self::PRIME);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use mercury_option_keys::{CardChange, Focus, KeyedGroup, Phase, Target};
    use winit::keyboard::NativeKeyCode;

    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn code(code: KeyCode) -> PhysicalKey {
        PhysicalKey::Code(code)
    }

    /// The id of a key that the machine must see as `Key::Other`.
    fn other_id(code: KeyCode) -> u32 {
        match key_from_winit(&PhysicalKey::Code(code)) {
            Key::Other(id) => id,
            other => panic!("{code:?} should be Other, got {other:?}"),
        }
    }

    /// What the hook does with one key event: the winit parts to an `Event` (none when synthetic).
    fn hook_event(
        key: KeyCode,
        state: ElementState,
        mods: ModifiersState,
        synthetic: bool,
    ) -> Option<Event> {
        event_from_parts(&code(key), state, false, mods, synthetic)
    }

    /// The same for a real (not synthetic) event, which must produce one.
    fn real(key: KeyCode, state: ElementState, mods: ModifiersState) -> Event {
        match hook_event(key, state, mods, false) {
            Some(event) => event,
            None => panic!("a real {key:?} event must reach the machine"),
        }
    }

    // ---- key_from_winit ---------------------------------------------------------------

    #[test]
    fn all_26_letters_map_to_the_right_letter_key() {
        use KeyCode::*;
        let table = [
            (KeyA, LetterKey::A),
            (KeyB, LetterKey::B),
            (KeyC, LetterKey::C),
            (KeyD, LetterKey::D),
            (KeyE, LetterKey::E),
            (KeyF, LetterKey::F),
            (KeyG, LetterKey::G),
            (KeyH, LetterKey::H),
            (KeyI, LetterKey::I),
            (KeyJ, LetterKey::J),
            (KeyK, LetterKey::K),
            (KeyL, LetterKey::L),
            (KeyM, LetterKey::M),
            (KeyN, LetterKey::N),
            (KeyO, LetterKey::O),
            (KeyP, LetterKey::P),
            (KeyQ, LetterKey::Q),
            (KeyR, LetterKey::R),
            (KeyS, LetterKey::S),
            (KeyT, LetterKey::T),
            (KeyU, LetterKey::U),
            (KeyV, LetterKey::V),
            (KeyW, LetterKey::W),
            (KeyX, LetterKey::X),
            (KeyY, LetterKey::Y),
            (KeyZ, LetterKey::Z),
        ];
        assert_eq!(table.len(), 26);
        for (winit_code, letter) in table {
            assert_eq!(
                key_from_winit(&code(winit_code)),
                Key::Letter(letter),
                "{winit_code:?}"
            );
            // The names check the table, so a slip in both columns cannot pass: winit calls the
            // key "KeyS" and `LetterKey` calls it "S".
            assert_eq!(format!("{winit_code:?}"), format!("Key{letter:?}"));
        }
        // each of the 26 letters appears exactly once
        for letter in LetterKey::ALL {
            assert_eq!(table.iter().filter(|(_, l)| *l == letter).count(), 1);
        }
    }

    #[test]
    fn digits_one_to_nine_and_numpad_one_to_nine_map_to_digit() {
        use KeyCode::*;
        let row = [
            Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
        ];
        let pad = [
            Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
        ];
        for (n, (digit, numpad)) in (1u8..=9).zip(row.into_iter().zip(pad)) {
            assert_eq!(key_from_winit(&code(digit)), Key::Digit(n), "{digit:?}");
            assert_eq!(key_from_winit(&code(numpad)), Key::Digit(n), "{numpad:?}");
        }
    }

    #[test]
    fn digit_zero_and_numpad_zero_are_other_keys() {
        // 0 never picks a tab: the machine's digits are 1 to 9.
        assert_ne!(other_id(KeyCode::Digit0), other_id(KeyCode::Numpad0));
    }

    #[test]
    fn escape_maps_to_escape() {
        assert_eq!(key_from_winit(&code(KeyCode::Escape)), Key::Escape);
    }

    #[test]
    fn the_four_arrows_map_to_arrow() {
        for arrow in [
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
        ] {
            assert_eq!(key_from_winit(&code(arrow)), Key::Arrow, "{arrow:?}");
        }
    }

    #[test]
    fn every_modifier_key_maps_to_modifier() {
        for modifier in [
            KeyCode::AltLeft,
            KeyCode::AltRight,
            KeyCode::ShiftLeft,
            KeyCode::ShiftRight,
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
            KeyCode::CapsLock,
            KeyCode::Fn,
        ] {
            assert_eq!(
                key_from_winit(&code(modifier)),
                Key::Modifier,
                "{modifier:?}"
            );
        }
    }

    #[test]
    fn other_keys_get_different_ids() {
        use KeyCode::*;
        let keys = [
            F1,
            F2,
            F3,
            F12,
            Space,
            Enter,
            Backspace,
            Delete,
            Tab,
            Home,
            End,
            PageUp,
            PageDown,
            Insert,
            Backquote,
            Minus,
            Equal,
            BracketLeft,
            BracketRight,
            Backslash,
            Semicolon,
            Quote,
            Comma,
            Period,
            Slash,
            NumpadEnter,
            NumpadAdd,
            NumpadSubtract,
            NumLock,
            ScrollLock,
            ContextMenu,
            Digit0,
            Numpad0,
        ];
        let ids: HashSet<u32> = keys.iter().map(|k| other_id(*k)).collect();
        assert_eq!(ids.len(), keys.len(), "two keys share an id");
    }

    #[test]
    fn the_same_key_always_gets_the_same_id() {
        assert_eq!(other_id(KeyCode::F1), other_id(KeyCode::F1));
        assert_eq!(other_id(KeyCode::Space), other_id(KeyCode::Space));
        // the key-down and the key-up of one key must meet in the machine's key-up memory
        let key_of = |state| match real(KeyCode::Backspace, state, ModifiersState::ALT) {
            Event::Key { key, .. } => key,
            other => panic!("expected a key event, got {other:?}"),
        };
        assert_eq!(
            key_of(ElementState::Pressed),
            key_of(ElementState::Released)
        );
    }

    #[test]
    fn unidentified_keys_map_to_other() {
        let unknown = PhysicalKey::Unidentified(NativeKeyCode::Unidentified);
        let mac_a = PhysicalKey::Unidentified(NativeKeyCode::MacOS(0x3f));
        let mac_b = PhysicalKey::Unidentified(NativeKeyCode::MacOS(0x40));
        let known = code(KeyCode::F1);
        let keys = [unknown, mac_a, mac_b, known].map(|k| key_from_winit(&k));
        for key in keys {
            assert!(matches!(key, Key::Other(_)), "{key:?}");
        }
        // different native codes are different keys, and none collides with a known key
        for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // the same native code is the same key
        assert_eq!(key_from_winit(&mac_a), key_from_winit(&mac_a));
    }

    #[test]
    fn the_id_is_the_32_bit_fnv_1a_of_the_debug_name() {
        // the published FNV-1a test vectors
        fn hash(text: &str) -> u32 {
            let mut hasher = Fnv1a::new();
            write!(hasher, "{text}").unwrap();
            hasher.0
        }
        assert_eq!(hash(""), 0x811c_9dc5);
        assert_eq!(hash("a"), 0xe40c_292c);
        assert_eq!(hash("foobar"), 0xbf9c_f968);
        // chunking does not matter, so a Debug value written in pieces hashes the same
        let mut pieces = Fnv1a::new();
        write!(pieces, "foo").unwrap();
        write!(pieces, "bar").unwrap();
        assert_eq!(pieces.0, hash("foobar"));
        // and the id of a key is the hash of its Debug name
        let physical = code(KeyCode::F1);
        assert_eq!(
            key_from_winit(&physical),
            Key::Other(hash(&format!("{physical:?}")))
        );
    }

    // ---- mods_from_winit --------------------------------------------------------------

    #[test]
    fn mods_from_winit_maps_each_modifier() {
        let mods = |alt, shift, control, command| Mods {
            alt,
            shift,
            control,
            command,
        };
        assert_eq!(mods_from_winit(ModifiersState::empty()), Mods::NONE);
        assert_eq!(mods_from_winit(ModifiersState::ALT), Mods::ALT);
        assert_eq!(
            mods_from_winit(ModifiersState::ALT | ModifiersState::SHIFT),
            mods(true, true, false, false)
        );
        assert_eq!(
            mods_from_winit(ModifiersState::SUPER | ModifiersState::ALT),
            mods(true, false, false, true)
        );
        assert_eq!(
            mods_from_winit(ModifiersState::CONTROL),
            mods(false, false, true, false)
        );
        assert_eq!(
            mods_from_winit(ModifiersState::SHIFT),
            mods(false, true, false, false)
        );
        assert_eq!(
            mods_from_winit(ModifiersState::SUPER),
            mods(false, false, false, true)
        );
        assert_eq!(
            mods_from_winit(
                ModifiersState::ALT
                    | ModifiersState::SHIFT
                    | ModifiersState::CONTROL
                    | ModifiersState::SUPER
            ),
            mods(true, true, true, true)
        );
    }

    #[test]
    fn only_a_lone_option_is_alt_only() {
        assert!(mods_from_winit(ModifiersState::ALT).alt_only());
        assert!(!mods_from_winit(ModifiersState::ALT | ModifiersState::SHIFT).alt_only());
        assert!(!mods_from_winit(ModifiersState::SUPER | ModifiersState::ALT).alt_only());
        assert!(!mods_from_winit(ModifiersState::CONTROL).alt_only());
        assert!(!mods_from_winit(ModifiersState::empty()).alt_only());
    }

    // ---- event_from_parts -------------------------------------------------------------

    #[test]
    fn a_press_and_a_release_become_key_events() {
        let physical = code(KeyCode::KeyS);
        assert_eq!(
            event_from_parts(
                &physical,
                ElementState::Pressed,
                false,
                ModifiersState::ALT,
                false
            ),
            Some(Event::Key {
                key: Key::Letter(LetterKey::S),
                mods: Mods::ALT,
                down: true,
                repeat: false,
            })
        );
        assert_eq!(
            event_from_parts(
                &physical,
                ElementState::Released,
                false,
                ModifiersState::ALT,
                false
            ),
            Some(Event::Key {
                key: Key::Letter(LetterKey::S),
                mods: Mods::ALT,
                down: false,
                repeat: false,
            })
        );
    }

    #[test]
    fn the_repeat_flag_is_carried() {
        let event = event_from_parts(
            &code(KeyCode::Digit2),
            ElementState::Pressed,
            true,
            ModifiersState::ALT,
            false,
        );
        assert_eq!(
            event,
            Some(Event::Key {
                key: Key::Digit(2),
                mods: Mods::ALT,
                down: true,
                repeat: true,
            })
        );
    }

    #[test]
    fn a_synthetic_event_never_becomes_an_event() {
        // winit replays the held keys when the window gains focus: the machine must never arm on
        // a key the user is not pressing
        for state in [ElementState::Pressed, ElementState::Released] {
            for repeat in [false, true] {
                for key in [KeyCode::AltLeft, KeyCode::KeyS, KeyCode::Digit1] {
                    assert_eq!(
                        event_from_parts(&code(key), state, repeat, ModifiersState::ALT, true),
                        None,
                        "{key:?} {state:?} repeat={repeat}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_modifiers_of_the_event_are_carried() {
        let event = real(
            KeyCode::KeyS,
            ElementState::Pressed,
            ModifiersState::SUPER | ModifiersState::ALT,
        );
        let Event::Key { mods, .. } = event else {
            panic!("expected a key event, got {event:?}");
        };
        assert_eq!(
            mods,
            Mods {
                alt: true,
                command: true,
                ..Mods::NONE
            }
        );
        assert!(!mods.alt_only());
    }

    // ---- clock_now and poll_delay -----------------------------------------------------

    #[test]
    fn the_clock_never_goes_backwards() {
        let first = clock_now();
        let second = clock_now();
        assert!(second >= first);
        std::thread::sleep(ms(2));
        let third = clock_now();
        assert!(third > second, "{third:?} should be after {second:?}");
    }

    #[test]
    fn the_clock_has_one_origin_for_the_whole_process() {
        // a time taken on another thread is comparable with this thread's
        let before = clock_now();
        let on_thread = std::thread::spawn(clock_now).join().unwrap();
        let after = clock_now();
        assert!(before <= on_thread && on_thread <= after);
    }

    #[test]
    fn poll_delay_without_a_deadline_is_none() {
        assert_eq!(poll_delay(None, ms(0)), None);
        assert_eq!(poll_delay(None, ms(5_000)), None);
    }

    #[test]
    fn poll_delay_past_the_deadline_is_zero() {
        assert_eq!(poll_delay(Some(ms(400)), ms(500)), Some(Duration::ZERO));
        assert_eq!(poll_delay(Some(ms(400)), ms(400)), Some(Duration::ZERO));
        assert_eq!(
            poll_delay(Some(Duration::ZERO), Duration::MAX),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn poll_delay_before_the_deadline_is_the_difference() {
        assert_eq!(poll_delay(Some(ms(400)), ms(150)), Some(ms(250)));
        assert_eq!(poll_delay(Some(ms(400)), ms(0)), Some(ms(400)));
        assert_eq!(poll_delay(Some(ms(400)), ms(399)), Some(ms(1)));
    }

    // ---- with_machine -----------------------------------------------------------------

    /// A fresh machine on this thread, with the group of the tests below (the thread-local lives
    /// as long as its thread, and the test harness may reuse one).
    fn fresh_machine_with_group_s() {
        with_machine(|machine| {
            *machine = OptionKeys::new(Config::default());
            machine.set_groups(&[KeyedGroup {
                id: 7,
                key: LetterKey::S,
                tab_count: 3,
            }]);
        });
    }

    fn alt_down() -> Event {
        Event::Modifiers(mods_from_winit(ModifiersState::ALT))
    }

    fn alt_up() -> Event {
        Event::Modifiers(mods_from_winit(ModifiersState::empty()))
    }

    #[test]
    fn the_machine_keeps_its_state_between_calls() {
        fresh_machine_with_group_s();
        assert_eq!(with_machine(|machine| machine.phase()), Phase::Idle);
        // the modifiers arrive in one call, the key in another: the state carries over
        with_machine(|machine| machine.on_event(ms(10), alt_down()));
        assert_eq!(with_machine(|machine| machine.phase()), Phase::Armed);
        assert_eq!(
            with_machine(|machine| machine.deadline()),
            Some(ms(410)),
            "armed at 10 ms with the default 400 ms hold"
        );
        // and the same machine is seen again later
        with_machine(|machine| machine.on_event(ms(20), Event::Reset));
        assert_eq!(with_machine(|machine| machine.phase()), Phase::Idle);
    }

    #[test]
    fn the_machine_starts_with_the_default_config() {
        // a new thread has a new machine: on, outside text or hold, 400 ms
        let (phase, deadline) = std::thread::spawn(|| {
            with_machine(|machine| {
                machine.set_groups(&[KeyedGroup {
                    id: 7,
                    key: LetterKey::S,
                    tab_count: 3,
                }]);
                machine.on_event(ms(0), alt_down());
                (machine.phase(), machine.deadline())
            })
        })
        .join()
        .unwrap();
        assert_eq!(phase, Phase::Armed);
        assert_eq!(deadline, Some(ms(400)));
    }

    #[test]
    fn quick_option_s_outside_a_text_field_switches_group() {
        fresh_machine_with_group_s();
        with_machine(|machine| machine.set_focus(Focus::default()));

        // ⌥ down: winit's ModifiersChanged, then the KeyboardInput of the ⌥ key itself
        let outcome = with_machine(|machine| machine.on_event(ms(0), alt_down()));
        assert!(
            !outcome.consume,
            "modifier state must reach Servo and Slint"
        );
        let alt_key = real(KeyCode::AltLeft, ElementState::Pressed, ModifiersState::ALT);
        assert!(!with_machine(|machine| machine.on_event(ms(1), alt_key)).consume);

        // S goes down: the letter of a group, outside a text field, so Mercury's
        let s_down = real(KeyCode::KeyS, ElementState::Pressed, ModifiersState::ALT);
        let outcome = with_machine(|machine| machine.on_event(ms(80), s_down));
        assert!(outcome.consume);
        assert_eq!(outcome.activate, Some(Target::Group(7)));
        assert_eq!(outcome.card, CardChange::None);
        assert_eq!(with_machine(|machine| machine.phase()), Phase::Switched);

        // S goes up: the up of a consumed key is consumed too
        let s_up = real(KeyCode::KeyS, ElementState::Released, ModifiersState::ALT);
        let outcome = with_machine(|machine| machine.on_event(ms(120), s_up));
        assert!(outcome.consume);
        assert_eq!(outcome.activate, None);

        // 2 then opens the second tab of that group at once (the numpad digit works the same)
        let two = real(KeyCode::Numpad2, ElementState::Pressed, ModifiersState::ALT);
        let outcome = with_machine(|machine| machine.on_event(ms(200), two));
        assert!(outcome.consume);
        assert_eq!(outcome.activate, Some(Target::Tab { group: 7, index: 1 }));
    }

    #[test]
    fn quick_option_s_in_a_text_field_passes_through() {
        fresh_machine_with_group_s();
        with_machine(|machine| {
            machine.set_focus(Focus {
                text_field: true,
                composing: false,
            })
        });

        with_machine(|machine| machine.on_event(ms(0), alt_down()));
        let s_down = real(KeyCode::KeyS, ElementState::Pressed, ModifiersState::ALT);
        let outcome = with_machine(|machine| machine.on_event(ms(80), s_down));
        assert!(!outcome.consume, "⌥S types ß in a text field");
        assert_eq!(outcome.activate, None);
        assert_eq!(outcome.card, CardChange::None);
        assert_eq!(with_machine(|machine| machine.phase()), Phase::PassThrough);
    }

    #[test]
    fn a_hold_opens_the_card_and_poll_delay_says_when() {
        fresh_machine_with_group_s();
        with_machine(|machine| machine.on_event(ms(1000), alt_down()));

        // the hook arms its timer for poll_delay(deadline, now)
        let deadline = with_machine(|machine| machine.deadline());
        assert_eq!(poll_delay(deadline, ms(1100)), Some(ms(300)));
        let early = with_machine(|machine| machine.poll(ms(1399)));
        assert_eq!(early.card, CardChange::None);
        assert_eq!(poll_delay(deadline, ms(1399)), Some(ms(1)));
        let on_time = with_machine(|machine| machine.poll(ms(1400)));
        assert_eq!(on_time.card, CardChange::Show);
        assert_eq!(poll_delay(deadline, ms(1400)), Some(Duration::ZERO));
        assert!(with_machine(|machine| machine.view()).visible);
        assert_eq!(with_machine(|machine| machine.phase()), Phase::Overlay);

        // releasing ⌥ closes the card, and the modifier change itself is never swallowed
        let outcome = with_machine(|machine| machine.on_event(ms(1500), alt_up()));
        assert_eq!(outcome.card, CardChange::Hide);
        assert!(!outcome.consume);
    }

    #[test]
    fn a_synthetic_option_press_never_arms_the_machine() {
        fresh_machine_with_group_s();
        // winit replays ⌥ when the window gains focus: the hook gets None and sends nothing
        let replay = hook_event(
            KeyCode::AltLeft,
            ElementState::Pressed,
            ModifiersState::ALT,
            true,
        );
        assert_eq!(replay, None);
        assert_eq!(with_machine(|machine| machine.phase()), Phase::Idle);
        assert_eq!(with_machine(|machine| machine.deadline()), None);
    }

    #[test]
    fn a_reentrant_call_does_not_panic_and_leaves_the_outer_state_alone() {
        fresh_machine_with_group_s();
        with_machine(|machine| machine.on_event(ms(0), alt_down()));
        let outer_phase = with_machine(|outer| {
            // a closure that calls back into the glue is a bug in the caller; it must not take
            // the winit event hook down with it
            let inner_phase = with_machine(|inner| {
                inner.on_event(ms(5), Event::Reset);
                inner.phase()
            });
            assert_eq!(
                inner_phase,
                Phase::Idle,
                "the nested call got a spare machine"
            );
            outer.phase()
        });
        assert_eq!(outer_phase, Phase::Armed);
        assert_eq!(with_machine(|machine| machine.phase()), Phase::Armed);
    }
}
