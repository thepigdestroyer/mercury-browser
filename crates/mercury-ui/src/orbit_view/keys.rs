// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The keys Orbit reacts to, from the text of a Slint key event (moved here from the desktop glue so
//! that it is tested without a Servo build).

use super::Key;

/// A key of the view's `key-pressed` text as the machine's [`Key`]; `None` for anything Orbit has
/// no use for (function keys, white space, several characters at once).
pub fn key_from_text(text: &str) -> Option<Key> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let named = |key: slint::platform::Key| c == char::from(key);
    if named(slint::platform::Key::Escape) {
        return Some(Key::Escape);
    }
    if c == '\n' || c == '\r' || named(slint::platform::Key::Return) {
        return Some(Key::Enter);
    }
    if named(slint::platform::Key::UpArrow) {
        return Some(Key::Up);
    }
    if named(slint::platform::Key::DownArrow) {
        return Some(Key::Down);
    }
    if named(slint::platform::Key::LeftArrow) {
        return Some(Key::Left);
    }
    if named(slint::platform::Key::RightArrow) {
        return Some(Key::Right);
    }
    if named(slint::platform::Key::Tab) {
        return Some(Key::Tab);
    }
    match c {
        'n' | 'N' => Some(Key::N),
        '/' => Some(Key::Slash),
        // The private-use block holds Slint's other special keys (F1, Home, ...).
        c if c.is_control() || c.is_whitespace() || ('\u{e000}'..='\u{f8ff}').contains(&c) => None,
        c => Some(Key::Char(c)),
    }
}
