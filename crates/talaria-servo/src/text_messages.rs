// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The text lane's side of the engine messages (txt-1): the IME events Mercury sends the page, and
//! the state of the focused editable element the engine sends back.
//!
//! The router (`mercury_messages.rs`) hands the lane's `TextMsg` to [`handle`]. Both directions
//! only translate between the engine's types (`servo::ImeEvent`, `servo::TextInputState`) and
//! `talaria_core`'s mirror of them, so that `talaria-core` stays free of the engine.
//!
//! Nothing here logs a payload: the text of a page, and what a user typed into it, must not reach
//! a log.

use std::ops::Range;

use servo::{
    ImeClause as ServoImeClause, ImeClauseKind as ServoImeClauseKind, ImeEvent as ServoImeEvent,
    ImeReplacement as ServoImeReplacement, TextInputState as ServoTextInputState, TextMsg,
};
use talaria_core::{
    BrowserEvent, ControlRect, ImeClause, ImeClauseKind, ImeReplacement, TextInputState, Utf16Range,
};

/// Handles one text lane message. `emit` sends an event to the tab the message came from; the
/// router gives a no-op to messages that name no tab.
pub(crate) fn handle(message: TextMsg, emit: &dyn Fn(BrowserEvent)) {
    match message {
        TextMsg::InputState(state) => {
            // The engine boxes the state (every engine message is as big as its largest variant).
            emit(BrowserEvent::TextInputState(
                state.map(|state| browser_text_state(*state)),
            ));
        }
    }
}

/// The engine's text state as the browser keeps it.
///
/// A password field carries no text in the message already; it is emptied again here, so a change
/// on the engine's side cannot put a password into the browser's state.
fn browser_text_state(state: ServoTextInputState) -> TextInputState {
    let ServoTextInputState {
        sequence,
        text,
        text_start,
        selection,
        marked,
        caret_rect,
        multiline,
        is_password,
    } = state;
    let (text, selection, marked) = if is_password {
        (String::new(), Utf16Range::collapsed(0), None)
    } else {
        (text, utf16_range(selection), marked.map(utf16_range))
    };
    TextInputState {
        sequence,
        text,
        text_start,
        selection,
        marked,
        caret_rect: caret_rect.and_then(control_rect),
        multiline,
        is_password,
    }
}

fn utf16_range(range: Range<u32>) -> Utf16Range {
    // A reversed range is a caret at its start.
    Utf16Range::new(range.start, range.end.max(range.start))
}

fn engine_range(range: Utf16Range) -> Range<u32> {
    range.start..range.end.max(range.start)
}

/// The caret rectangle `[x, y, width, height]` in CSS pixels, as the browser keeps rectangles of
/// the page (`ControlRect`, the convention of `embedder_controls::control_rect`: the numbers are
/// passed on in the frame's own pixels; the window converts with the page zoom). The rectangle is
/// rounded outwards, so a caret of a fraction of a pixel is at least one pixel wide. `None` for
/// numbers that are not finite or do not fit.
fn control_rect(rect: [f32; 4]) -> Option<ControlRect> {
    let [x, y, width, height] = rect;
    if !(x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite()) {
        return None;
    }
    let left = x.floor();
    let top = y.floor();
    let right = (x + width.max(0.0)).ceil();
    let bottom = (y + height.max(0.0)).ceil();
    // Far outside any page: not a caret.
    const LIMIT: f32 = 1_000_000.0;
    if [left, top, right, bottom]
        .iter()
        .any(|value| value.abs() > LIMIT)
    {
        return None;
    }
    // The limit above keeps every conversion exact.
    Some(ControlRect {
        x: left as i32,
        y: top as i32,
        width: ((right - left) as i32).max(1),
        height: ((bottom - top) as i32).max(1),
    })
}

fn servo_clause_kind(kind: ImeClauseKind) -> ServoImeClauseKind {
    match kind {
        ImeClauseKind::Raw => ServoImeClauseKind::Raw,
        ImeClauseKind::Converted => ServoImeClauseKind::Converted,
        ImeClauseKind::SelectedRaw => ServoImeClauseKind::SelectedRaw,
        ImeClauseKind::SelectedConverted => ServoImeClauseKind::SelectedConverted,
    }
}

fn servo_replacement(replacement: ImeReplacement) -> ServoImeReplacement {
    ServoImeReplacement {
        range: engine_range(replacement.range),
        sequence: replacement.sequence,
    }
}

/// `ImeInput::SetMarkedText` as the engine's event.
pub(crate) fn marked_text_event(
    text: String,
    selected: Option<Utf16Range>,
    clauses: Vec<ImeClause>,
    replacement: Option<ImeReplacement>,
) -> ServoImeEvent {
    ServoImeEvent::SetMarkedText {
        text,
        selected: selected.map(engine_range),
        clauses: clauses
            .into_iter()
            .map(|clause| ServoImeClause {
                range: engine_range(clause.range),
                kind: servo_clause_kind(clause.kind),
            })
            .collect(),
        replacement: replacement.map(servo_replacement),
    }
}

/// `ImeInput::InsertText` as the engine's event.
pub(crate) fn insert_text_event(
    text: String,
    replacement: Option<ImeReplacement>,
) -> ServoImeEvent {
    ServoImeEvent::InsertText {
        text,
        replacement: replacement.map(servo_replacement),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn engine_state(text: &str) -> ServoTextInputState {
        ServoTextInputState {
            sequence: 5,
            text: text.to_owned(),
            text_start: 10,
            selection: 2..3,
            marked: Some(1..3),
            caret_rect: Some([12.5, 40.25, 1.0, 17.5]),
            multiline: true,
            is_password: false,
        }
    }

    fn emitted(message: TextMsg) -> Vec<BrowserEvent> {
        let events = RefCell::new(Vec::new());
        handle(message, &|event| events.borrow_mut().push(event));
        events.into_inner()
    }

    #[test]
    fn a_state_becomes_a_browser_event_with_the_same_fields() {
        let events = emitted(TextMsg::InputState(Some(Box::new(engine_state("héllo")))));

        assert_eq!(
            events,
            vec![BrowserEvent::TextInputState(Some(TextInputState {
                sequence: 5,
                text: "héllo".to_owned(),
                text_start: 10,
                selection: Utf16Range::new(2, 3),
                marked: Some(Utf16Range::new(1, 3)),
                // 12.5 -> 12, 40.25 -> 40, right 13.5 -> 14, bottom 57.75 -> 58
                caret_rect: Some(ControlRect {
                    x: 12,
                    y: 40,
                    width: 2,
                    height: 18,
                }),
                multiline: true,
                is_password: false,
            }))]
        );
    }

    #[test]
    fn no_state_means_nothing_editable_has_focus() {
        assert_eq!(
            emitted(TextMsg::InputState(None)),
            vec![BrowserEvent::TextInputState(None)]
        );
    }

    #[test]
    fn a_password_state_never_carries_text_even_if_the_engine_sent_some() {
        let mut state = engine_state("hunter2");
        state.is_password = true;

        let events = emitted(TextMsg::InputState(Some(Box::new(state))));

        let [BrowserEvent::TextInputState(Some(state))] = events.as_slice() else {
            panic!("one state event expected, got {events:?}");
        };
        assert!(state.is_password);
        assert_eq!(state.text, "");
        assert_eq!(state.selection, Utf16Range::collapsed(0));
        assert_eq!(state.marked, None);
    }

    #[test]
    fn a_reversed_range_is_a_caret() {
        let mut state = engine_state("abc");
        state.selection = Range { start: 3, end: 1 };

        let browser = browser_text_state(state);

        assert_eq!(browser.selection, Utf16Range::collapsed(3));
    }

    #[test]
    fn caret_rectangles_are_rounded_outwards_and_bad_numbers_are_dropped() {
        assert_eq!(
            control_rect([10.0, 20.0, 0.0, 0.0]),
            Some(ControlRect {
                x: 10,
                y: 20,
                width: 1,
                height: 1,
            })
        );
        assert_eq!(
            control_rect([-3.5, 0.2, 1.2, 10.0]),
            Some(ControlRect {
                x: -4,
                y: 0,
                width: 2,
                height: 11,
            })
        );
        assert_eq!(control_rect([f32::NAN, 0.0, 1.0, 1.0]), None);
        assert_eq!(control_rect([0.0, f32::INFINITY, 1.0, 1.0]), None);
        assert_eq!(control_rect([0.0, 0.0, f32::MAX, 1.0]), None);
        assert_eq!(control_rect([2_000_000.0, 0.0, 1.0, 1.0]), None);
        // A negative size is no size.
        assert_eq!(
            control_rect([5.0, 5.0, -4.0, -2.0]),
            Some(ControlRect {
                x: 5,
                y: 5,
                width: 1,
                height: 1,
            })
        );
    }

    #[test]
    fn marked_text_keeps_its_ranges_clauses_and_replacement() {
        let event = marked_text_event(
            "にほん".to_owned(),
            Some(Utf16Range::new(1, 3)),
            vec![
                ImeClause {
                    range: Utf16Range::new(0, 1),
                    kind: ImeClauseKind::Converted,
                },
                ImeClause {
                    range: Utf16Range::new(1, 3),
                    kind: ImeClauseKind::SelectedConverted,
                },
            ],
            Some(ImeReplacement {
                range: Utf16Range::new(4, 6),
                sequence: 9,
            }),
        );

        let ServoImeEvent::SetMarkedText {
            text,
            selected,
            clauses,
            replacement,
        } = event
        else {
            panic!("a marked text event expected");
        };
        assert_eq!(text, "にほん");
        assert_eq!(selected, Some(1..3));
        assert_eq!(
            clauses,
            vec![
                ServoImeClause {
                    range: 0..1,
                    kind: ServoImeClauseKind::Converted,
                },
                ServoImeClause {
                    range: 1..3,
                    kind: ServoImeClauseKind::SelectedConverted,
                },
            ]
        );
        assert_eq!(
            replacement,
            Some(ServoImeReplacement {
                range: 4..6,
                sequence: 9,
            })
        );
    }

    #[test]
    fn every_clause_kind_maps_to_its_engine_kind() {
        assert_eq!(
            servo_clause_kind(ImeClauseKind::Raw),
            ServoImeClauseKind::Raw
        );
        assert_eq!(
            servo_clause_kind(ImeClauseKind::Converted),
            ServoImeClauseKind::Converted
        );
        assert_eq!(
            servo_clause_kind(ImeClauseKind::SelectedRaw),
            ServoImeClauseKind::SelectedRaw
        );
        assert_eq!(
            servo_clause_kind(ImeClauseKind::SelectedConverted),
            ServoImeClauseKind::SelectedConverted
        );
    }

    #[test]
    fn inserted_text_keeps_its_replacement() {
        let event = insert_text_event(
            "😀".to_owned(),
            Some(ImeReplacement {
                range: Utf16Range::new(0, 1),
                sequence: 2,
            }),
        );
        let ServoImeEvent::InsertText { text, replacement } = event else {
            panic!("an insert event expected");
        };
        assert_eq!(text, "😀");
        assert_eq!(
            replacement,
            Some(ServoImeReplacement {
                range: 0..1,
                sequence: 2,
            })
        );

        let ServoImeEvent::InsertText { replacement, .. } = insert_text_event("a".to_owned(), None)
        else {
            panic!("an insert event expected");
        };
        assert_eq!(replacement, None);
    }

    #[test]
    fn a_text_message_prints_only_its_variant() {
        let printed = format!(
            "{:?}",
            TextMsg::InputState(Some(Box::new(engine_state("secret"))))
        );
        assert!(!printed.contains("secret"), "{printed}");
    }
}
