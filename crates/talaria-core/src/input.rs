// SPDX-License-Identifier: MPL-2.0
use std::fmt;

use crate::{ImeClause, ImeReplacement, Utf16Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyLocation {
    Standard,
    Left,
    Right,
    Numpad,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyboardModifiers {
    pub alt: bool,
    pub control: bool,
    pub meta: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardInput {
    pub state: KeyState,
    pub key: String,
    pub code: String,
    pub location: KeyLocation,
    pub modifiers: KeyboardModifiers,
    pub repeat: bool,
    pub is_composing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionState {
    Start,
    Update,
    End,
}

/// What an input method (IME) does in the focused editable of the page.
///
/// `Composition` and `Dismissed` are the older pair: a composition is started, updated and ended
/// with the text as a whole. The other three carry what macOS reports (txt-1 lane): the part of
/// the marked text that is selected, its clauses, a replacement range, and text that arrives
/// without a composition (the emoji picker, dictation).
///
/// `Debug` prints the sizes of the text, never the text: what the user typed must not reach a log.
#[derive(Clone, PartialEq, Eq)]
pub enum ImeInput {
    Composition {
        state: CompositionState,
        data: String,
    },
    Dismissed,
    /// The marked text is `text` now (non-empty): a composition starts if none is open. Ranges
    /// are UTF-16 units. `selected` is what the input method has selected inside `text` (a caret
    /// when empty); `clauses` tell how its parts are drawn; `replacement` names text of the field
    /// that the marked text takes the place of.
    SetMarkedText {
        text: String,
        selected: Option<Utf16Range>,
        clauses: Vec<ImeClause>,
        replacement: Option<ImeReplacement>,
    },
    /// Text is final. With a composition open it replaces the marked text and ends the
    /// composition; an empty text cancels the composition. Without a composition it is inserted
    /// like typing.
    Commit(String),
    /// Text that comes without a composition, for example from the emoji picker or dictation. It
    /// replaces `replacement` when that is given and still current.
    InsertText {
        text: String,
        replacement: Option<ImeReplacement>,
    },
}

impl fmt::Debug for ImeInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Sizes only, never the text.
        match self {
            Self::Composition { state, data } => formatter
                .debug_struct("Composition")
                .field("state", state)
                .field("data_bytes", &data.len())
                .finish(),
            Self::Dismissed => formatter.write_str("Dismissed"),
            Self::SetMarkedText {
                text,
                selected,
                clauses,
                replacement,
            } => formatter
                .debug_struct("SetMarkedText")
                .field("text_bytes", &text.len())
                .field("selected", selected)
                .field("clauses", clauses)
                .field("replacement", replacement)
                .finish(),
            Self::Commit(text) => formatter
                .debug_struct("Commit")
                .field("text_bytes", &text.len())
                .finish(),
            Self::InsertText { text, replacement } => formatter
                .debug_struct("InsertText")
                .field("text_bytes", &text.len())
                .field("replacement", replacement)
                .finish(),
        }
    }
}

/// A clipboard edit performed on whatever has focus inside the page, as if its key had been
/// pressed: Servo runs the same editing code for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardCommand {
    Cut,
    Copy,
    Paste,
}
