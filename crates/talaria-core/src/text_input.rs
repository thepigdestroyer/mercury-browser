// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Text input (txt-1 lane): what an input method sends the page, and what the page's focused
//! editable tells the browser back.
//!
//! The types mirror the engine's (`servo-embedder-traits`: `ImeClause`, `ImeClauseKind`,
//! `ImeReplacement` and, in `mercury/text.rs`, `TextInputState`) without depending on it: this
//! crate has no engine in it. All text offsets are UTF-16 code units, which is what macOS, the DOM
//! and JavaScript use.
//!
//! `Debug` of [`TextInputState`] never prints the text: what a page holds, and what a user typed,
//! must not reach a log.

use std::fmt;

use crate::ControlRect;

/// A range of UTF-16 code units, `start..end`. `start == end` is a caret.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Utf16Range {
    pub start: u32,
    pub end: u32,
}

impl Utf16Range {
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// A caret at `at`.
    pub const fn collapsed(at: u32) -> Self {
        Self { start: at, end: at }
    }

    pub const fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    pub const fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }
}

/// How a piece of marked text is to be drawn, as the input method describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeClauseKind {
    /// Text as typed, not yet converted (thin underline).
    Raw,
    /// Converted text that is not the clause being edited (thin underline).
    Converted,
    /// The clause being edited, still unconverted (thick underline).
    SelectedRaw,
    /// The clause being edited, converted (thick underline).
    SelectedConverted,
}

impl ImeClauseKind {
    /// Whether the clause is the one being edited.
    pub const fn is_selected(self) -> bool {
        matches!(self, Self::SelectedRaw | Self::SelectedConverted)
    }
}

/// One clause of marked text: a range of the marked text (UTF-16 units inside it) and its look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImeClause {
    pub range: Utf16Range,
    pub kind: ImeClauseKind,
}

/// A range of the field's text that an input method wants replaced (the accent menu, a dictation
/// correction, a reconversion). `range` is in the `text` of the [`TextInputState`] whose
/// `sequence` is given: the engine honours it only if that state is still the last one it sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImeReplacement {
    pub range: Utf16Range,
    pub sequence: u64,
}

/// What the focused editable element of a page holds, as the engine last reported it.
#[derive(Clone, PartialEq, Eq)]
pub struct TextInputState {
    /// The document (the engine's pipeline id) whose field this is. A page and an iframe each have
    /// their own sequence and their own messages, which can arrive in either order when the focus
    /// moves between them: the "nothing is focused" message of a document clears only this.
    pub document: u64,
    /// Grows by one with every message the engine sends.
    pub sequence: u64,
    /// A window of at most 4,096 UTF-16 units around the selection. Empty for a password field.
    pub text: String,
    /// The offset of the window in the whole text (0 unless it was cut).
    pub text_start: u32,
    /// The selection, relative to `text`.
    pub selection: Utf16Range,
    /// The marked text of a running composition, relative to `text`.
    pub marked: Option<Utf16Range>,
    /// The caret, widened by one character, in the same pixels and frame as `ControlRect` (the
    /// engine reports it in CSS pixels, relative to the web view, as fractions; they are rounded
    /// outwards here: a caret is a few pixels wide, and the candidate window only needs its
    /// corner).
    pub caret_rect: Option<ControlRect>,
    pub multiline: bool,
    pub is_password: bool,
}

impl fmt::Debug for TextInputState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // No text: only its size.
        formatter
            .debug_struct("TextInputState")
            .field("document", &self.document)
            .field("sequence", &self.sequence)
            .field("text_units", &utf16_len(&self.text))
            .field("text_start", &self.text_start)
            .field("selection", &self.selection)
            .field("marked", &self.marked)
            .field("caret_rect", &self.caret_rect)
            .field("multiline", &self.multiline)
            .field("is_password", &self.is_password)
            .finish()
    }
}

impl TextInputState {
    /// The end of the window in the whole text, in UTF-16 units.
    pub fn window_end(&self) -> u32 {
        self.text_start.saturating_add(utf16_len(&self.text))
    }

    /// The selection in the whole text (the window offset added).
    pub fn absolute_selection(&self) -> Utf16Range {
        Utf16Range::new(
            self.text_start.saturating_add(self.selection.start),
            self.text_start.saturating_add(self.selection.end),
        )
    }

    /// The part of the window that lies inside `absolute` (a range of the whole text), with the
    /// range actually covered. The ends move inwards, never splitting a surrogate pair, so the
    /// returned text is whole characters. `None` when `absolute` does not meet the window at all.
    pub fn text_in(&self, absolute: Utf16Range) -> Option<(Utf16Range, String)> {
        let units: Vec<u16> = self.text.encode_utf16().collect();
        let window_start = self.text_start;
        let window_end = self.window_end();
        if absolute.start > absolute.end
            || absolute.start > window_end
            || absolute.end < window_start
        {
            return None;
        }
        let start = absolute.start.max(window_start);
        let end = absolute.end.min(window_end);
        let mut from = (start - window_start) as usize;
        let mut to = (end - window_start) as usize;
        // A boundary inside a surrogate pair moves inwards.
        if from > 0 && from < units.len() && is_low_surrogate(units[from]) {
            from += 1;
        }
        if to > 0 && to < units.len() && is_low_surrogate(units[to]) {
            to -= 1;
        }
        if from > to {
            to = from;
        }
        let covered = Utf16Range::new(
            window_start.saturating_add(from as u32),
            window_start.saturating_add(to as u32),
        );
        Some((covered, String::from_utf16_lossy(&units[from..to])))
    }

    /// A replacement of `absolute` (a range of the whole text) that names this state, with the
    /// range moved to be relative to `text`. The range is cut to the window. `None` when it starts
    /// before the window or after its end: the engine cannot resolve what it never sent.
    pub fn replacement_for(&self, absolute: Utf16Range) -> Option<ImeReplacement> {
        let window_end = self.window_end();
        if absolute.start > absolute.end
            || absolute.start < self.text_start
            || absolute.start > window_end
        {
            return None;
        }
        let end = absolute.end.min(window_end);
        Some(ImeReplacement {
            range: Utf16Range::new(absolute.start - self.text_start, end - self.text_start),
            sequence: self.sequence,
        })
    }
}

fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&unit)
}

/// The length of `text` in UTF-16 code units (saturating at `u32::MAX`).
pub fn utf16_len(text: &str) -> u32 {
    u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX)
}

/// The UTF-16 offset of the UTF-8 byte offset `byte` in `text`, as an input method reports its
/// cursor (winit gives bytes, the page wants units). An offset past the end is the end; an offset
/// inside a character is the start of that character, so a character is never split.
pub fn utf16_offset_of_byte(text: &str, byte: usize) -> u32 {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    utf16_len(&text[..byte])
}

/// The UTF-8 byte offset of the UTF-16 offset `unit` in `text`: the inverse of
/// [`utf16_offset_of_byte`]. An offset past the end is the end; an offset inside a surrogate pair
/// is the start of that character.
pub fn utf8_offset_of_utf16(text: &str, unit: u32) -> usize {
    let mut units = 0_u32;
    for (byte, character) in text.char_indices() {
        let width = u32::try_from(character.len_utf16()).unwrap_or(2);
        if units + width > unit {
            return byte;
        }
        units += width;
    }
    text.len()
}
