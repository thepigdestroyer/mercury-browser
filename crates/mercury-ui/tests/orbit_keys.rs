// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `orbit_view::keys::key_from_text`: the text of a Slint `key-pressed` event as the machine's
//! [`Key`]. Pure Rust: no Slint window.

use mercury_ui::orbit_view::Key;
use mercury_ui::orbit_view::keys::key_from_text;
use slint::platform::Key as SlintKey;

fn text(key: SlintKey) -> String {
    char::from(key).to_string()
}

#[test]
fn every_named_key_orbit_uses_maps_to_its_key() {
    for (slint_key, expected) in [
        (SlintKey::Escape, Key::Escape),
        (SlintKey::Return, Key::Enter),
        (SlintKey::UpArrow, Key::Up),
        (SlintKey::DownArrow, Key::Down),
        (SlintKey::LeftArrow, Key::Left),
        (SlintKey::RightArrow, Key::Right),
        (SlintKey::Tab, Key::Tab),
    ] {
        assert_eq!(
            key_from_text(&text(slint_key)),
            Some(expected),
            "{slint_key:?}"
        );
    }
}

#[test]
fn tab_is_the_tab_character_and_the_shift_state_is_the_machines_modifier() {
    assert_eq!(key_from_text("\t"), Some(Key::Tab));
    assert_eq!(text(SlintKey::Tab), "\t");
}

#[test]
fn a_new_line_and_a_carriage_return_are_enter() {
    assert_eq!(key_from_text("\n"), Some(Key::Enter));
    assert_eq!(key_from_text("\r"), Some(Key::Enter));
}

#[test]
fn n_in_both_cases_is_the_new_group_key() {
    assert_eq!(key_from_text("n"), Some(Key::N));
    assert_eq!(key_from_text("N"), Some(Key::N));
}

#[test]
fn slash_is_the_search_key() {
    assert_eq!(key_from_text("/"), Some(Key::Slash));
}

#[test]
fn letters_digits_and_symbols_are_typed_characters() {
    for c in [
        'a', 'x', 'Z', 'é', 'ß', 'ж', '日', '0', '7', '9', '-', '.', '?', '#', '😀',
    ] {
        assert_eq!(key_from_text(&c.to_string()), Some(Key::Char(c)), "{c:?}");
    }
}

#[test]
fn only_n_and_slash_are_named_among_the_ascii_printable_characters() {
    for byte in 0x20u8..0x7f {
        let c = char::from(byte);
        let got = key_from_text(&c.to_string());
        match c {
            'n' | 'N' => assert_eq!(got, Some(Key::N)),
            '/' => assert_eq!(got, Some(Key::Slash)),
            ' ' => assert_eq!(got, None, "a space is not a character"),
            c => assert_eq!(got, Some(Key::Char(c)), "{c:?}"),
        }
    }
}

#[test]
fn white_space_is_not_a_key() {
    for text in [" ", "\u{a0}", "\u{2003}", "\u{3000}", "\u{2028}"] {
        assert_eq!(key_from_text(text), None, "{text:?}");
    }
    assert_eq!(key_from_text(&text(SlintKey::Space)), None);
}

#[test]
fn control_characters_are_not_keys() {
    for code in (0u32..0x20).chain([0x7f]) {
        let Some(c) = char::from_u32(code) else {
            continue;
        };
        let expected = match c {
            '\n' | '\r' => Some(Key::Enter),
            '\t' => Some(Key::Tab),
            '\u{1b}' => Some(Key::Escape),
            _ => None,
        };
        assert_eq!(key_from_text(&c.to_string()), expected, "{code:#x}");
    }
}

#[test]
fn slints_other_special_keys_are_not_keys() {
    for slint_key in [
        SlintKey::Backspace,
        SlintKey::Backtab,
        SlintKey::Delete,
        SlintKey::Shift,
        SlintKey::Control,
        SlintKey::Alt,
        SlintKey::AltGr,
        SlintKey::CapsLock,
        SlintKey::Meta,
        SlintKey::Home,
        SlintKey::End,
        SlintKey::PageUp,
        SlintKey::PageDown,
        SlintKey::Insert,
        SlintKey::F1,
        SlintKey::F5,
        SlintKey::F12,
        SlintKey::F24,
    ] {
        assert_eq!(key_from_text(&text(slint_key)), None, "{slint_key:?}");
    }
}

#[test]
fn the_whole_private_use_block_is_not_a_key_except_the_arrows() {
    let arrows = [
        char::from(SlintKey::UpArrow),
        char::from(SlintKey::DownArrow),
        char::from(SlintKey::LeftArrow),
        char::from(SlintKey::RightArrow),
    ];
    for code in 0xe000u32..=0xf8ff {
        let Some(c) = char::from_u32(code) else {
            continue;
        };
        if arrows.contains(&c) {
            continue;
        }
        assert_eq!(key_from_text(&c.to_string()), None, "{code:#x}");
    }
}

#[test]
fn characters_just_outside_the_private_use_block_are_typed_characters() {
    for c in ['\u{d7ff}', '\u{f900}', '\u{fb00}'] {
        assert_eq!(key_from_text(&c.to_string()), Some(Key::Char(c)), "{c:?}");
    }
}

#[test]
fn an_empty_text_and_several_characters_are_not_a_key() {
    for text in [
        "",
        "ab",
        "nn",
        "//",
        "\t\t",
        "\n\n",
        "é\u{301}",
        "😀😀",
        " n",
        "n ",
    ] {
        assert_eq!(key_from_text(text), None, "{text:?}");
    }
}
