/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the text lane. Add variants (and the payload types they need) here
// only, never in another lane's file or in a shared enum.

use std::fmt;
use std::ops::Range;

use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

/// Messages from the engine to Mercury for the `text` lane (IME, composition, editing).
///
/// `Debug` prints only the variant name, never a payload.
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum TextMsg {
    /// The text state of the focused editable (a text control or an editing host). Sent only when
    /// it changed since the last message of that document.
    ///
    /// The state is boxed so that this variant stays small: every `EmbedderMsg` is as big as its
    /// largest variant (see the size test in `mercury/mod.rs`).
    InputState(Box<TextInputState>),
    /// Nothing editable has focus in the document with this id any more (`TextInputState::document`).
    /// It carries the document because the messages of two documents (a page and an iframe) can
    /// reach the browser in either order when the focus moves between them: the browser clears
    /// its state only if it is the state of that document.
    InputCleared(u64),
}

impl fmt::Debug for TextMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}

/// What the input method (and the system services built on it: reconversion, the accent menu,
/// dictation corrections) may ask about the focused editable: a snapshot of its text.
///
/// Every offset is in UTF-16 code units, the unit AppKit's `NSRange` uses.
///
/// This holds the text a user types. It must never be logged: `Debug` prints sizes and flags only.
#[derive(Clone, Deserialize, PartialEq, Serialize)]
pub struct TextInputState {
    /// The document (its pipeline id) whose focused editable this describes; see
    /// [`TextMsg::InputCleared`].
    pub document: u64,
    /// Grows by one with every message the engine sends. An `ImeReplacement` is honoured only if
    /// it names the sequence of the last message sent.
    pub sequence: u64,
    /// A window of the editable's text of at most [`TextInputState::MAX_WINDOW`] UTF-16 code
    /// units around the selection. Empty for a password field.
    pub text: String,
    /// The UTF-16 offset of `text` in the whole text of the editable (0 unless it was truncated).
    pub text_start: u32,
    /// The selection (or the caret, when collapsed), relative to `text`.
    pub selection: Range<u32>,
    /// The marked text of an open composition, relative to `text`.
    pub marked: Option<Range<u32>>,
    /// Where the caret is: x, y, width and height in CSS pixels, relative to the web view.
    ///
    /// Until layout can measure a range of characters this is an estimate inside the box of the
    /// text node the caret is in (same line, horizontal position proportional to the offset).
    pub caret_rect: Option<[f32; 4]>,
    /// Whether the editable takes several lines.
    pub multiline: bool,
    /// Whether the editable is a password field (then `text` is empty and `selection` collapsed).
    pub is_password: bool,
}

impl TextInputState {
    /// The most UTF-16 code units of text one message carries.
    pub const MAX_WINDOW: u32 = 4096;

    /// Whether this state and `other` agree on everything but the sequence number and the caret
    /// rectangle.
    pub fn same_content_as(&self, other: &TextInputState) -> bool {
        self.text == other.text
            && self.text_start == other.text_start
            && self.selection == other.selection
            && self.marked == other.marked
            && self.multiline == other.multiline
            && self.is_password == other.is_password
    }
}

impl fmt::Debug for TextInputState {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        // Deliberately no `text`: what a user typed must not reach a log.
        formatter
            .debug_struct("TextInputState")
            .field("document", &self.document)
            .field("sequence", &self.sequence)
            .field("text_len_bytes", &self.text.len())
            .field("text_start", &self.text_start)
            .field("selection", &self.selection)
            .field("marked", &self.marked)
            .field("has_caret_rect", &self.caret_rect.is_some())
            .field("multiline", &self.multiline)
            .field("is_password", &self.is_password)
            .finish()
    }
}

// Mercury: `ImeEvent` has no derived `Debug` (see `input_events.rs`). Upstream logs whole messages
// at `debug` level (`script_thread.rs`: "Handling event: {event:?}"), and a derived `Debug` would
// print what the user typed. This one prints the variant and the lengths of its strings.
impl fmt::Debug for crate::ImeEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            crate::ImeEvent::Composition(event) => formatter
                .debug_struct("Composition")
                .field("state", &event.state)
                .field("data_len_bytes", &event.data.len())
                .finish(),
            crate::ImeEvent::Commit(text) => formatter
                .debug_struct("Commit")
                .field("text_len_bytes", &text.len())
                .finish(),
            crate::ImeEvent::SetMarkedText {
                text,
                selected,
                clauses,
                replacement,
            } => formatter
                .debug_struct("SetMarkedText")
                .field("text_len_bytes", &text.len())
                .field("selected", selected)
                .field("clauses", &clauses.len())
                .field("has_replacement", &replacement.is_some())
                .finish(),
            crate::ImeEvent::InsertText { text, replacement } => formatter
                .debug_struct("InsertText")
                .field("text_len_bytes", &text.len())
                .field("has_replacement", &replacement.is_some())
                .finish(),
            crate::ImeEvent::Dismissed => formatter.write_str("Dismissed"),
        }
    }
}

/// The window of a text that one [`TextInputState`] carries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextWindow {
    pub text: String,
    pub text_start: u32,
    pub selection: Range<u32>,
    pub marked: Option<Range<u32>>,
}

impl TextWindow {
    /// An estimate of the rectangle of the character at `caret` in a text node whose boxes (one
    /// per line it was broken into) are `rects`, as `[x, y, width, height]`. `total` is the length
    /// of the text in UTF-16 code units.
    ///
    /// Layout has no query for the box of a range of characters yet, so the caret is placed in
    /// the line boxes in proportion to the characters before it (exact for monospaced text, and
    /// close for CJK text, which input methods are mostly used with). A caret at the end of the
    /// text gets the box of the last character. The result is as wide as an average character
    /// and never leaves the box it is in. `None` if there is no box.
    pub fn estimate_caret_rect(rects: &[[f32; 4]], caret: u32, total: u32) -> Option<[f32; 4]> {
        let first = *rects.first()?;
        let width_sum: f32 = rects.iter().map(|rect| rect[2].max(0.0)).sum();
        if total == 0 || width_sum <= 0.0 {
            // Nothing to measure: the start of the first box, as wide as a caret.
            return Some([first[0], first[1], 1.0_f32.min(first[2].max(1.0)), first[3]]);
        }

        let average = (width_sum / total as f32).max(1.0);
        let at_end = caret >= total;
        let caret = caret.min(total);
        // How far into the boxes the left edge of the character is.
        let mut distance = if at_end {
            width_sum - average
        } else {
            width_sum * (caret as f32 / total as f32)
        }
        .max(0.0);

        for rect in rects {
            let width = rect[2].max(0.0);
            // A character at the end of this box belongs to the next box, except in the last one.
            if distance < width || std::ptr::eq(rect, &rects[rects.len() - 1]) {
                let width_of_character = average.min(width.max(1.0));
                let x = rect[0] + distance.min((width - width_of_character).max(0.0));
                return Some([x, rect[1], width_of_character, rect[3]]);
            }
            distance -= width;
        }
        None
    }

    /// The length of `text` in UTF-16 code units.
    pub fn utf16_len(text: &str) -> u32 {
        text.chars().map(|c| c.len_utf16() as u32).sum()
    }

    /// The byte offset in `text` of the UTF-16 offset `utf16`, or `None` when `utf16` is past the
    /// end of the text or in the middle of a surrogate pair (a character outside the Basic
    /// Multilingual Plane takes two UTF-16 code units and no offset may point between them).
    pub fn utf16_to_byte_offset(text: &str, utf16: u32) -> Option<usize> {
        let mut units = 0u32;
        for (byte_offset, character) in text.char_indices() {
            if units == utf16 {
                return Some(byte_offset);
            }
            if units > utf16 {
                // Passed it inside the previous character.
                return None;
            }
            units += character.len_utf16() as u32;
        }
        (units == utf16).then_some(text.len())
    }

    /// The UTF-16 offset in `text` of the byte offset `byte` (which should be on a character
    /// boundary; an offset past the end counts as the end of the text).
    pub fn byte_to_utf16_offset(text: &str, byte: usize) -> u32 {
        let mut units = 0u32;
        for (byte_offset, character) in text.char_indices() {
            if byte_offset >= byte {
                break;
            }
            units += character.len_utf16() as u32;
        }
        units
    }

    /// The window of `text` for a message: all of it when it has at most
    /// [`TextInputState::MAX_WINDOW`] UTF-16 code units, otherwise that many around the marked
    /// text or the selection. `selection` and `marked` are in UTF-16 code units of the whole
    /// text; the window's are relative to its start and clamped into it.
    pub fn new(text: &str, selection: Range<u32>, marked: Option<Range<u32>>) -> TextWindow {
        let total = Self::utf16_len(text);
        let max = TextInputState::MAX_WINDOW;
        if total <= max {
            return TextWindow {
                text: text.to_owned(),
                text_start: 0,
                selection: clamp_range(selection, 0, total),
                marked: marked.map(|marked| clamp_range(marked, 0, total)),
            };
        }

        // Centre the window on the marked text (the user is working there) or the selection.
        let anchor = marked
            .as_ref()
            .map_or(selection.start, |marked| marked.start);
        let mut begin = anchor.saturating_sub(max / 2).min(total - max);
        let mut end = begin + max;
        // Never cut a surrogate pair in two: move the edges inwards.
        if Self::utf16_to_byte_offset(text, begin).is_none() {
            begin += 1;
        }
        if Self::utf16_to_byte_offset(text, end).is_none() {
            end -= 1;
        }
        // The offsets were just checked, so this cannot fail; fall back to the empty window.
        let (Some(begin_byte), Some(end_byte)) = (
            Self::utf16_to_byte_offset(text, begin),
            Self::utf16_to_byte_offset(text, end),
        ) else {
            return TextWindow {
                text: String::new(),
                text_start: 0,
                selection: 0..0,
                marked: None,
            };
        };

        let selection = clamp_range(selection, begin, end);
        let marked = marked.map(|marked| clamp_range(marked, begin, end));
        TextWindow {
            text: text[begin_byte..end_byte].to_owned(),
            text_start: begin,
            selection: (selection.start - begin)..(selection.end - begin),
            marked: marked.map(|marked| (marked.start - begin)..(marked.end - begin)),
        }
    }
}

/// `range` limited to `min..max`, still ordered.
fn clamp_range(range: Range<u32>, min: u32, max: u32) -> Range<u32> {
    let start = range.start.clamp(min, max);
    let end = range.end.clamp(start, max);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(text: &str) -> TextInputState {
        TextInputState {
            document: 7,
            sequence: 1,
            text: text.to_owned(),
            text_start: 0,
            selection: 0..0,
            marked: None,
            caret_rect: None,
            multiline: false,
            is_password: false,
        }
    }

    #[test]
    fn debug_prints_only_the_variant_name() {
        let message = TextMsg::InputState(Box::new(state("secret text")));
        assert_eq!(format!("{message:?}"), "InputState");
    }

    #[test]
    fn debug_of_a_state_never_prints_its_text() {
        let mut with_text = state("hunter2 and more");
        with_text.marked = Some(0..3);
        with_text.caret_rect = Some([1.0, 2.0, 3.0, 4.0]);
        let printed = format!("{with_text:?}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(printed.contains("sequence"));
    }

    #[test]
    fn debug_of_an_ime_event_never_prints_the_typed_text() {
        use keyboard_types::{CompositionEvent, CompositionState};

        use crate::{ImeEvent, ImeReplacement};
        let events = [
            ImeEvent::Commit("hunter2 commit".to_owned()),
            ImeEvent::SetMarkedText {
                text: "hunter2 marked".to_owned(),
                selected: Some(0..2),
                clauses: vec![],
                replacement: Some(ImeReplacement {
                    range: 0..1,
                    sequence: 3,
                }),
            },
            ImeEvent::InsertText {
                text: "hunter2 insert".to_owned(),
                replacement: None,
            },
            ImeEvent::Composition(CompositionEvent {
                state: CompositionState::Update,
                data: "hunter2 composition".to_owned(),
            }),
        ];
        for event in events {
            let printed = format!("{event:?}");
            assert!(!printed.contains("hunter2"), "{printed}");
            assert!(printed.contains("len"), "{printed}");
        }
        assert_eq!(format!("{:?}", ImeEvent::Dismissed), "Dismissed");
    }

    #[test]
    fn the_message_types_cross_the_process_boundary() {
        fn assert_serde<T: Serialize + for<'de> Deserialize<'de>>() {}
        assert_serde::<TextMsg>();
        assert_serde::<TextInputState>();
    }

    #[test]
    fn same_content_ignores_the_sequence_and_the_rectangle() {
        let mut a = state("a");
        a.selection = 1..1;
        let mut b = a.clone();
        b.sequence = 2;
        b.caret_rect = Some([0.0, 0.0, 1.0, 1.0]);
        assert!(a.same_content_as(&b));
        a.selection = 0..0;
        assert!(!a.same_content_as(&b));
    }

    #[test]
    fn utf16_lengths_and_offsets_count_surrogate_pairs() {
        // "a" (1 unit) + the emoji (2 units) + "b" (1 unit)
        let text = "a😀b";
        assert_eq!(TextWindow::utf16_len(text), 4);
        assert_eq!(TextWindow::utf16_to_byte_offset(text, 0), Some(0));
        assert_eq!(TextWindow::utf16_to_byte_offset(text, 1), Some(1));
        // Inside the pair: refused.
        assert_eq!(TextWindow::utf16_to_byte_offset(text, 2), None);
        assert_eq!(TextWindow::utf16_to_byte_offset(text, 3), Some(5));
        assert_eq!(TextWindow::utf16_to_byte_offset(text, 4), Some(6));
        // Past the end: refused.
        assert_eq!(TextWindow::utf16_to_byte_offset(text, 5), None);
        assert_eq!(TextWindow::utf16_to_byte_offset(text, u32::MAX), None);
        assert_eq!(TextWindow::byte_to_utf16_offset(text, 5), 3);
        assert_eq!(TextWindow::byte_to_utf16_offset(text, 99), 4);
        assert_eq!(TextWindow::utf16_to_byte_offset("", 0), Some(0));
        assert_eq!(TextWindow::utf16_to_byte_offset("", 1), None);
    }

    #[test]
    fn a_short_text_is_sent_whole() {
        let window = TextWindow::new("hello", 5..5, Some(2..5));
        assert_eq!(window.text, "hello");
        assert_eq!(window.text_start, 0);
        assert_eq!(window.selection, 5..5);
        assert_eq!(window.marked, Some(2..5));
    }

    #[test]
    fn a_long_text_sends_a_window_of_4096_units() {
        let text = "x".repeat(10_000);
        let window = TextWindow::new(&text, 5_000..5_000, None);
        assert_eq!(
            TextWindow::utf16_len(&window.text),
            TextInputState::MAX_WINDOW
        );
        assert_eq!(window.text_start, 5_000 - 2_048);
        // The caret keeps its place relative to the window.
        assert_eq!(window.selection, 2_048..2_048);
    }

    #[test]
    fn a_window_at_the_start_and_at_the_end_stays_inside_the_text() {
        let text = "y".repeat(10_000);
        let start = TextWindow::new(&text, 0..0, None);
        assert_eq!(start.text_start, 0);
        assert_eq!(TextWindow::utf16_len(&start.text), 4_096);
        assert_eq!(start.selection, 0..0);

        let end = TextWindow::new(&text, 10_000..10_000, None);
        assert_eq!(end.text_start, 10_000 - 4_096);
        assert_eq!(TextWindow::utf16_len(&end.text), 4_096);
        assert_eq!(end.selection, 4_096..4_096);
    }

    #[test]
    fn a_window_never_cuts_a_surrogate_pair() {
        // Emoji only: every character is two units, so any odd edge is inside a pair.
        let text = "😀".repeat(5_000);
        let window = TextWindow::new(&text, 4_001..4_001, None);
        assert!(window.text.chars().all(|c| c == '😀'));
        assert_eq!(window.text_start % 2, 0);
        assert!(TextWindow::utf16_len(&window.text) <= TextInputState::MAX_WINDOW);
    }

    #[test]
    fn a_window_follows_the_marked_text_and_clamps_the_selection() {
        let text = "z".repeat(10_000);
        // The whole text is selected, the composition is near the end.
        let window = TextWindow::new(&text, 0..10_000, Some(9_000..9_002));
        assert_eq!(window.text_start, 10_000 - 4_096);
        assert_eq!(window.selection, 0..4_096);
        let marked = window.marked.expect("marked text is in the window");
        let start = 9_000 - window.text_start;
        assert_eq!(marked, start..(start + 2));
    }
    #[test]
    fn a_caret_in_a_single_box_is_placed_in_proportion() {
        // 10 characters in a 100 px wide box: 10 px each.
        let rect = [20.0, 5.0, 100.0, 16.0];
        assert_eq!(
            TextWindow::estimate_caret_rect(&[rect], 0, 10),
            Some([20.0, 5.0, 10.0, 16.0])
        );
        assert_eq!(
            TextWindow::estimate_caret_rect(&[rect], 3, 10),
            Some([50.0, 5.0, 10.0, 16.0])
        );
    }

    #[test]
    fn a_caret_at_the_end_gets_the_last_character() {
        let rect = [20.0, 5.0, 100.0, 16.0];
        assert_eq!(
            TextWindow::estimate_caret_rect(&[rect], 10, 10),
            Some([110.0, 5.0, 10.0, 16.0])
        );
        // Past the end counts as the end.
        assert_eq!(
            TextWindow::estimate_caret_rect(&[rect], 99, 10),
            Some([110.0, 5.0, 10.0, 16.0])
        );
    }

    #[test]
    fn a_caret_in_a_wrapped_text_picks_the_line() {
        // Two lines of 50 px for 10 characters: 10 px each, the sixth character starts line two.
        let lines = [[0.0, 0.0, 50.0, 16.0], [0.0, 16.0, 50.0, 16.0]];
        let sixth = TextWindow::estimate_caret_rect(&lines, 5, 10).expect("a rectangle");
        assert_eq!(sixth, [0.0, 16.0, 10.0, 16.0]);
        let third = TextWindow::estimate_caret_rect(&lines, 2, 10).expect("a rectangle");
        assert_eq!(third, [20.0, 0.0, 10.0, 16.0]);
        let last = TextWindow::estimate_caret_rect(&lines, 10, 10).expect("a rectangle");
        assert_eq!(last, [40.0, 16.0, 10.0, 16.0]);
    }

    #[test]
    fn an_empty_text_or_no_box_has_a_safe_answer() {
        assert_eq!(TextWindow::estimate_caret_rect(&[], 0, 0), None);
        let rect = [8.0, 4.0, 0.0, 14.0];
        assert_eq!(
            TextWindow::estimate_caret_rect(&[rect], 0, 0),
            Some([8.0, 4.0, 1.0, 14.0])
        );
        // A box with no width (a collapsed text) does not divide by zero.
        assert_eq!(
            TextWindow::estimate_caret_rect(&[rect], 2, 2),
            Some([8.0, 4.0, 1.0, 14.0])
        );
    }
}
