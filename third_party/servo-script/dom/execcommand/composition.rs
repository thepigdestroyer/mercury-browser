/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this whole file is Mercury's (EB11, the text lane). It lets an input method run the
// `inserttext` command of an editing host with the `beforeinput` and `input` events of a
// composition (`insertCompositionText`, `isComposing`, not cancelable) or of a replacement
// (`insertReplacementText`), and keeps the marked text of the composition as a live DOM range.
// The hook in `execcommands.rs` is one parameter of `exec_command_with_input_events()`; the
// command itself is not copied.

use std::cmp::Ordering;

use js::context::JSContext;
use script_bindings::codegen::GenericBindings::DocumentBinding::DocumentMethods;
use script_bindings::codegen::GenericBindings::SelectionBinding::SelectionMethods;
use script_bindings::inheritance::Castable;
use script_bindings::root::DomRoot;
use script_bindings::str::DOMString;

use crate::dom::abstractrange::bp_position;
use crate::dom::document::Document;
use crate::dom::event::inputevent::InputEventFlags;
use crate::dom::execcommand::execcommands::DocumentExecCommandSupport;
use crate::dom::iterators::ShadowIncluding;
use crate::dom::node::Node;
use crate::dom::range::Range;
use crate::dom::text::Text;

/// What an input method says about the `beforeinput` and `input` events an editing command fires.
#[derive(Clone, Debug)]
pub(crate) struct ImeInputEvents {
    /// `insertCompositionText`, `insertText` or `insertReplacementText`.
    pub(crate) input_type: &'static str,
    /// Whether the events are part of a composition (`isComposing`, `beforeinput` not cancelable).
    pub(crate) composing: bool,
    /// The text, as the `data` of both events.
    pub(crate) data: String,
}

impl ImeInputEvents {
    pub(crate) fn composition(text: &str) -> Self {
        Self {
            input_type: "insertCompositionText",
            composing: true,
            data: text.to_owned(),
        }
    }

    pub(crate) fn insertion(text: &str) -> Self {
        Self {
            input_type: "insertText",
            composing: false,
            data: text.to_owned(),
        }
    }

    pub(crate) fn replacement(text: &str) -> Self {
        Self {
            input_type: "insertReplacementText",
            composing: false,
            data: text.to_owned(),
        }
    }
}

/// The flags of the `beforeinput` event of an editing command.
pub(crate) fn before_input_flags(input: Option<&ImeInputEvents>) -> InputEventFlags {
    match input {
        None => InputEventFlags::BUBBLES | InputEventFlags::CANCELABLE,
        // > beforeinput: cancelable unless inputType is insertCompositionText
        Some(input) if input.composing => InputEventFlags::BUBBLES | InputEventFlags::COMPOSING,
        Some(_) => InputEventFlags::BUBBLES | InputEventFlags::CANCELABLE,
    }
}

/// The flags of the `input` event of an editing command.
pub(crate) fn input_flags(input: Option<&ImeInputEvents>) -> InputEventFlags {
    match input {
        Some(input) if input.composing => InputEventFlags::BUBBLES | InputEventFlags::COMPOSING,
        _ => InputEventFlags::BUBBLES,
    }
}

/// The `data` of the events of an editing command (null for the commands of the spec).
pub(crate) fn event_data(input: Option<&ImeInputEvents>) -> Option<DOMString> {
    input.map(|input| DOMString::from(input.data.as_str()))
}

/// The `inputType` of the events of an editing command: the input method's, or `mapped`, the
/// mapped value of the command.
pub(crate) fn event_input_type(input: Option<&ImeInputEvents>, mapped: DOMString) -> DOMString {
    input.map_or(mapped, |input| DOMString::from(input.input_type))
}

/// Run the `inserttext` command with the input events of `input`. Returns whether it ran.
pub(crate) fn insert_text(
    cx: &mut JSContext,
    document: &Document,
    text: &str,
    input: &ImeInputEvents,
) -> bool {
    document.exec_command_with_input_events(
        cx,
        DOMString::from_static("inserttext"),
        DOMString::from(text),
        Some(input),
    )
}

/// Replace `marked` (or, if there is none, the selection) with `text` in the editing host that has
/// the selection, with the input events of `input`, and return the range of the new text. The
/// selection is left collapsed after it, as `inserttext` does.
///
/// Returns `None` if there is no selection, the command did not run (`beforeinput` was canceled,
/// the selection is not editable) or it left no selection.
pub(crate) fn replace_marked_text(
    cx: &mut JSContext,
    document: &Document,
    marked: Option<&Range>,
    text: &str,
    input: &ImeInputEvents,
) -> Option<DomRoot<Range>> {
    let selection = document.GetSelection(cx)?;
    if let Some(marked) = marked {
        // The command replaces the selection: select the marked text.
        selection
            .SetBaseAndExtent(
                cx,
                &marked.start_container(),
                marked.start_offset(),
                &marked.end_container(),
                marked.end_offset(),
            )
            .ok()?;
    }

    let active_range = selection.active_range(cx)?;
    let start_container = active_range.start_container();
    let start_offset = active_range.start_offset();
    // A live range that stays where the new text starts: DOM mutations do not move a boundary
    // point that is at the offset text is inserted at.
    let start = Range::new(
        cx,
        document,
        &start_container,
        start_offset,
        &start_container,
        start_offset,
    );

    if !insert_text(cx, document, text, input) {
        return None;
    }

    let end_range = selection.active_range(cx)?;
    let end_container = end_range.end_container();
    let end_offset = end_range.end_offset();
    let (start_container, start_offset) = (start.start_container(), start.start_offset());
    if bp_position(
        cx.no_gc(),
        &start_container,
        start_offset,
        &end_container,
        end_offset,
    ) == Ordering::Greater
    {
        // Something moved the range past the end: the new text is empty.
        return Some(Range::new(
            cx,
            document,
            &end_container,
            end_offset,
            &end_container,
            end_offset,
        ));
    }
    Some(Range::new(
        cx,
        document,
        &start_container,
        start_offset,
        &end_container,
        end_offset,
    ))
}

/// The Text nodes of `range` in tree order, each with the span of its text that is inside the
/// range, in UTF-16 code units (the unit of DOM offsets).
pub(crate) fn text_spans(
    cx: &mut JSContext,
    range: &Range,
) -> Vec<(DomRoot<Node>, std::ops::Range<u32>)> {
    let start = range.start_container();
    let end = range.end_container();
    let mut nodes: Vec<DomRoot<Node>> = vec![start.clone()];
    if start != end {
        let root = start.owner_doc();
        let unrooted_end = end.as_unrooted(cx.no_gc());
        nodes.extend(
            start
                .following_nodes_unrooted(cx.no_gc(), root.upcast::<Node>(), ShadowIncluding::No)
                .take_while(|node| *node != *end)
                .chain(std::iter::once(unrooted_end))
                .map(|node| node.as_rooted()),
        );
    }

    nodes
        .into_iter()
        .filter(|node| node.is::<Text>())
        .map(|node| {
            let from = if node == start {
                range.start_offset()
            } else {
                0
            };
            let to = if node == end {
                range.end_offset()
            } else {
                node.len()
            };
            (node, from..to.max(from))
        })
        .collect()
}

/// The DOM position of the UTF-16 offset `offset` of the text of `range` (its Text nodes one after
/// the other). An offset between two nodes is at the end of the earlier one. `None` if the range
/// has no text or `offset` is past it.
pub(crate) fn position_in_range(
    cx: &mut JSContext,
    range: &Range,
    offset: u32,
) -> Option<(DomRoot<Node>, u32)> {
    let mut remaining = offset;
    for (node, span) in text_spans(cx, range) {
        let length = span.end - span.start;
        if remaining <= length {
            return Some((node, span.start + remaining));
        }
        remaining -= length;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_composition_is_not_cancelable_and_is_composing() {
        let composition = ImeInputEvents::composition("ni");
        let before = before_input_flags(Some(&composition));
        assert!(before.contains(InputEventFlags::COMPOSING));
        assert!(!before.contains(InputEventFlags::CANCELABLE));
        let after = input_flags(Some(&composition));
        assert!(after.contains(InputEventFlags::COMPOSING));
        assert_eq!(composition.input_type, "insertCompositionText");
    }

    #[test]
    fn the_events_of_the_spec_commands_are_unchanged() {
        let before = before_input_flags(None);
        assert!(before.contains(InputEventFlags::CANCELABLE));
        assert!(!before.contains(InputEventFlags::COMPOSING));
        assert!(input_flags(None) == InputEventFlags::BUBBLES);
        assert!(event_data(None).is_none());
    }

    #[test]
    fn a_replacement_is_cancelable_and_not_composing() {
        let replacement = ImeInputEvents::replacement("é");
        assert!(before_input_flags(Some(&replacement)).contains(InputEventFlags::CANCELABLE));
        assert!(!input_flags(Some(&replacement)).contains(InputEventFlags::COMPOSING));
        assert_eq!(replacement.input_type, "insertReplacementText");
        assert_eq!(ImeInputEvents::insertion("a").input_type, "insertText");
    }
}
