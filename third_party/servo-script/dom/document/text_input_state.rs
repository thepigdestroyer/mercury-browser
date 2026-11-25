/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this whole file is Mercury's (EB11, the text lane). It is the IME state of a document:
// the composition that an `ImeEvent` opened, the replacement of text the system asks for, and the
// text state message that tells the browser what the focused editable holds. The hooks in
// upstream files are in `document_event_handler.rs` (the events, a flush after each input event),
// `editing.rs` (the old `Composition` events in editing hosts) and the two text controls (a flush
// when they gain or lose focus).

//! The IME state of a document.
//!
//! * [`ImeState::set_marked_text`], [`ImeState::commit`] and [`ImeState::insert_text`] handle the
//!   `ImeEvent`s of the same names, for text controls and editing hosts alike, and fire the events
//!   of a composition in the order Chromium does: `compositionstart` (cancelable), then for each
//!   update `compositionupdate`, `beforeinput` (not cancelable) and `input`
//!   (`insertCompositionText`, `isComposing`), and at the end `compositionend` exactly once.
//! * [`ImeState::flush`] builds a [`TextInputState`] for the focused editable, and sends it to
//!   the browser when it differs from the last one sent.
//!
//! Neither logs text: what a user types must not reach a log.

use std::cell::Cell;
use std::ops::Range as UnitRange;

use embedder_traits::{
    EditingAction, EmbedderMsg, ImeClause, ImeClauseKind, ImeReplacement, InputEventResult,
    MercuryEmbedderMsg, TextInputState, TextMsg, TextWindow,
};
use js::context::JSContext;
use keyboard_types::CompositionState;
use layout_api::HighlightKind;
use script_bindings::cell::DomRefCell;
use script_bindings::codegen::GenericBindings::CharacterDataBinding::CharacterDataMethods;
use script_bindings::codegen::GenericBindings::DocumentBinding::DocumentMethods;
use script_bindings::codegen::GenericBindings::NodeBinding::NodeMethods;
use script_bindings::codegen::GenericBindings::SelectionBinding::SelectionMethods;
use script_bindings::inheritance::Castable;
use script_bindings::root::DomRoot;
use script_bindings::str::DOMString;

use crate::dom::bindings::root::MutNullableDom;
use crate::dom::characterdata::CharacterData;
use crate::dom::document::Document;
use crate::dom::document::editing::EditingContext;
use crate::dom::event::EventFlags;
use crate::dom::execcommand::execcommands::composition::{self, ImeInputEvents};
use crate::dom::html::htmlbrelement::HTMLBRElement;
use crate::dom::iterators::ShadowIncluding;
use crate::dom::node::{Node, NodeTraits};
use crate::dom::range::Range;
use crate::dom::text::Text;
use crate::dom::text_input::{Lines, TextInputStamp};
use crate::dom::types::{CompositionEvent, Element, Event, EventTarget};

/// When the text state is being flushed, which decides whether layout is asked for the caret
/// rectangle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FlushMode {
    /// After an input event, or a focus change: the caret rectangle is measured only for a newly
    /// focused editable and while a composition is open (the candidate window follows it). At
    /// most one layout is forced, by that measurement.
    AfterInput,
    /// After a reflow, when layout is up to date and measuring costs nothing: the caret
    /// rectangle is brought up to date.
    AfterReflow,
}

/// The IME state of a [`Document`], kept in its `DocumentEventHandler`.
#[derive(Default, JSTraceable, MallocSizeOf)]
#[cfg_attr(crown, crown::unrooted_must_root_lint::must_root)]
pub(crate) struct ImeState {
    /// The element a composition opened by an `ImeEvent` belongs to.
    composition_target: MutNullableDom<Element>,
    /// The marked text of an editing host, as a live range.
    host_marked: MutNullableDom<Range>,
    /// The text of the marked text at its last update, for the `compositionend` of a composition
    /// that lost its element.
    marked_text: DomRefCell<String>,
    /// The editable the last state message described.
    last_target: MutNullableDom<Element>,
    /// The last state message sent (`None` for "nothing editable has focus" and before the first).
    #[no_trace]
    #[ignore_malloc_size_of = "a message that was sent already"]
    last_sent: DomRefCell<Option<TextInputState>>,
    /// The sequence number of the last message sent.
    sequence: Cell<u64>,
    /// Whether the caret of the editable moved since its rectangle was measured.
    caret_rect_stale: Cell<bool>,
    /// What the last state was made of, so that a flush can skip the rebuild (and its copy of
    /// the text) when nothing changed.
    #[no_trace]
    #[ignore_malloc_size_of = "a few numbers"]
    last_stamp: DomRefCell<Option<Stamp>>,
}

/// A boundary point of a range: the node's address, its offset, for both ends.
type RangeStamp = (usize, u32, usize, u32);

/// Everything the text state of the focused editable is made of, except the caret rectangle and
/// the text itself. Equal stamps mean an equal state; making one copies nothing (a text control
/// keeps a counter of its changes, an editing host's block has a version number).
#[derive(Clone, Debug, PartialEq)]
enum Stamp {
    Control {
        input: TextInputStamp,
        password: bool,
    },
    Host {
        version: u64,
        selection: Option<RangeStamp>,
        marked: Option<RangeStamp>,
    },
}

fn range_stamp(range: &Range) -> RangeStamp {
    let (start, end) = (range.start_container(), range.end_container());
    (
        &*start as *const Node as usize,
        range.start_offset(),
        &*end as *const Node as usize,
        range.end_offset(),
    )
}

/// Whether a flush can return before it builds the state: it is the same editable as the last
/// message described, a message was sent, nothing it is made of changed, and the caret rectangle
/// does not need a new measurement after this reflow.
fn can_skip_rebuild(
    target_changed: bool,
    sent_before: bool,
    stamp_unchanged: bool,
    mode: FlushMode,
    caret_rect_stale: bool,
) -> bool {
    !target_changed &&
        sent_before &&
        stamp_unchanged &&
        !(mode == FlushMode::AfterReflow && caret_rect_stale)
}

/// The focused element, if there is one.
fn focused_element(document: &Document) -> Option<DomRoot<Element>> {
    let focused_area = document.focus_handler().focused_area();
    focused_area.element().map(DomRoot::from_ref)
}

/// Whether `a` and `b` are the same element.
fn same_element(a: &Element, b: &Element) -> bool {
    a.upcast::<Node>() == b.upcast::<Node>()
}

/// The focused editable: a text control, or an element in an editing host.
fn focused_editable(
    cx: &mut JSContext,
    document: &Document,
) -> Option<(DomRoot<Element>, EditingContext)> {
    let element = focused_element(document)?;
    let context = document.editing_context(cx.no_gc(), element.upcast());
    if matches!(context, EditingContext::TextControl(_)) {
        return Some((element, context));
    }
    // Editing hosts exist only with `execCommand` (Mercury turns it on).
    if !servo_config::pref!(dom_exec_command_enabled) {
        return None;
    }
    let editable = element.upcast::<Node>().is_editable_or_editing_host();
    editable.then_some((element, context))
}

/// Fire a composition event at `target`. Returns the flags of the event, so that the caller can
/// tell whether a page canceled it or handled it.
fn fire_composition_event(
    cx: &mut JSContext,
    document: &Document,
    target: &Element,
    state: CompositionState,
    data: &str,
) -> EventFlags {
    let window = document.window();
    let cancelable = state == CompositionState::Start;
    let event = CompositionEvent::new(
        cx,
        window,
        state.event_type().into(),
        true,
        cancelable,
        Some(window),
        0,
        DOMString::from(data),
    );
    let event = event.upcast::<Event>();
    event.fire(cx, target.upcast::<EventTarget>());
    event.flags()
}

/// The caret and the selected clause the input method asked for, both as UTF-16 offsets into the
/// marked text: the caret is where its own selection starts (the end of the marked text if it
/// said nothing); the selected clause is its selection if that is not empty, otherwise the
/// clause it marked as selected.
fn caret_and_selected_clause(
    selected: Option<&UnitRange<u32>>,
    clauses: &[ImeClause],
) -> (Option<u32>, Option<UnitRange<u32>>) {
    let caret = selected.map(|selected| selected.start);
    let selected_clause = selected
        .filter(|selected| selected.start < selected.end)
        .cloned()
        .or_else(|| {
            clauses
                .iter()
                .find(|clause| {
                    matches!(
                        clause.kind,
                        ImeClauseKind::SelectedRaw | ImeClauseKind::SelectedConverted
                    ) && clause.range.start < clause.range.end
                })
                .map(|clause| clause.range.clone())
        });
    (caret, selected_clause)
}

impl ImeState {
    /// Whether the focused editable has an open composition, whoever opened it.
    pub(crate) fn has_open_composition(&self, cx: &mut JSContext, document: &Document) -> bool {
        focused_editable(cx, document)
            .is_some_and(|(element, context)| self.composition_is_open(&element, &context))
    }

    /// Whether the editable of `context` has an open composition, whoever opened it.
    fn composition_is_open(&self, element: &Element, context: &EditingContext) -> bool {
        match context {
            EditingContext::TextControl(control) => {
                control.text_control_element().text_input().is_composing()
            }
            EditingContext::Document(_) => self
                .composition_target
                .get()
                .is_some_and(|target| same_element(&target, element)),
        }
    }

    // ----- the events -----------------------------------------------------------------------

    /// `ImeEvent::SetMarkedText`: the input method set its marked text.
    pub(crate) fn set_marked_text(
        &self,
        cx: &mut JSContext,
        document: &Document,
        text: String,
        selected: Option<UnitRange<u32>>,
        clauses: Vec<ImeClause>,
        replacement: Option<ImeReplacement>,
    ) -> InputEventResult {
        let Some((element, context)) = focused_editable(cx, document) else {
            return Default::default();
        };
        self.end_stale_composition(cx, document, &element);
        if !is_still_rendered(cx, &element) {
            return Default::default();
        }

        let composing = self.composition_is_open(&element, &context);
        // An empty text ends the composition, or means nothing if there was none.
        if text.is_empty() {
            return if composing {
                self.finish(cx, document, &element, &context, String::new())
            } else {
                Default::default()
            };
        }

        let mut result = InputEventResult::default();
        if !composing {
            // The marked text starts by replacing the text the system named, if it still can.
            if let Some(replacement) = &replacement {
                self.select_replacement(cx, document, &element, &context, replacement);
            }

            let flags = fire_composition_event(cx, document, &element, CompositionState::Start, "");
            result |= flags.into();
            if flags.intersects(EventFlags::Canceled | EventFlags::Handled) {
                return result;
            }
            self.begin(&element, &context);
        }

        // `compositionupdate`, then the text and its `beforeinput` and `input` events.
        let flags = fire_composition_event(cx, document, &element, CompositionState::Update, &text);
        result |= flags.into();
        if flags.intersects(EventFlags::Canceled | EventFlags::Handled) {
            return result;
        }
        *self.marked_text.borrow_mut() = text.clone();
        let (caret, selected_clause) = caret_and_selected_clause(selected.as_ref(), &clauses);
        let changed = match &context {
            EditingContext::TextControl(control) => control
                .text_control_element()
                .ime_update_marked_text(cx, &text, caret, selected_clause),
            EditingContext::Document(_) => {
                self.update_host(cx, document, &text, caret, selected_clause)
            }
        };
        if changed {
            result |= InputEventResult::Consumed;
        }
        result
    }

    /// `ImeEvent::Commit`, with a composition open: the text replaces the marked text and ends
    /// the composition. (Without a composition it is an insertion: `handle_ime_commit()`.)
    pub(crate) fn commit(
        &self,
        cx: &mut JSContext,
        document: &Document,
        text: String,
    ) -> InputEventResult {
        let Some((element, context)) = focused_editable(cx, document) else {
            return Default::default();
        };
        if !self.composition_is_open(&element, &context) {
            return self.insert_text(cx, document, text, None);
        }
        self.finish(cx, document, &element, &context, text)
    }

    /// `ImeEvent::InsertText`: text the system inserts without composing it, perhaps in place of
    /// text it names. With a composition open, the text ends it, as `Commit` does.
    pub(crate) fn insert_text(
        &self,
        cx: &mut JSContext,
        document: &Document,
        text: String,
        replacement: Option<ImeReplacement>,
    ) -> InputEventResult {
        let Some((element, context)) = focused_editable(cx, document) else {
            return Default::default();
        };
        self.end_stale_composition(cx, document, &element);
        if self.composition_is_open(&element, &context) {
            return self.finish(cx, document, &element, &context, text);
        }
        if text.is_empty() || !is_still_rendered(cx, &element) {
            return Default::default();
        }

        let replacing = replacement.as_ref().is_some_and(|replacement| {
            self.select_replacement(cx, document, &element, &context, replacement)
        });
        let inserted = match &context {
            EditingContext::TextControl(control) => control
                .text_control_element()
                .ime_insert_text(cx, &text, replacing),
            EditingContext::Document(_) => {
                let events = if replacing {
                    ImeInputEvents::replacement(&text)
                } else {
                    ImeInputEvents::insertion(&text)
                };
                composition::insert_text(cx, document, &text, &events)
            }
        };
        if inserted {
            InputEventResult::Consumed
        } else {
            Default::default()
        }
    }

    /// `ImeEvent::Dismissed` with a composition open: it ends, the marked text stays as it is
    /// (nothing the user typed is lost), before the caller moves focus away.
    pub(crate) fn dismiss(&self, cx: &mut JSContext, document: &Document) {
        let Some(target) = self.composition_target.get() else {
            return;
        };
        self.end_lost_composition(cx, document, &target);
    }

    // ----- composition ----------------------------------------------------------------------

    /// Open a composition on `element` (its `compositionstart` was fired).
    fn begin(&self, element: &Element, context: &EditingContext) {
        self.composition_target.set(Some(element));
        self.host_marked.set(None);
        if let EditingContext::TextControl(control) = context {
            control
                .text_control_element()
                .text_input_mut()
                .begin_composition();
        }
    }

    /// End the composition with `text` as its final text, and fire `compositionend` (once).
    fn finish(
        &self,
        cx: &mut JSContext,
        document: &Document,
        element: &Element,
        context: &EditingContext,
        text: String,
    ) -> InputEventResult {
        match context {
            EditingContext::TextControl(control) => {
                control
                    .text_control_element()
                    .ime_commit_composition(cx, &text);
            }
            EditingContext::Document(_) => {
                // The same events as for an update, even when the text did not change.
                self.update_host(cx, document, &text, None, None);
                self.end_host(cx, document, None, true);
            }
        }
        // The composition is closed before the event, so that a page that reacts to it, or an
        // event that arrives meanwhile, cannot end it a second time.
        self.composition_target.set(None);
        self.host_marked.set(None);
        self.marked_text.borrow_mut().clear();
        let flags = fire_composition_event(cx, document, element, CompositionState::End, &text);
        InputEventResult::from(flags) | InputEventResult::Consumed
    }

    /// A composition that belongs to another element than the focused editable (the focus moved
    /// between two IME events, before any flush noticed): it ends, with its `compositionend`,
    /// before the event for the new element is handled.
    fn end_stale_composition(&self, cx: &mut JSContext, document: &Document, focused: &Element) {
        if let Some(target) = self.composition_target.get() {
            if !same_element(&target, focused) {
                self.end_lost_composition(cx, document, &target);
            }
        }
    }

    /// A composition whose element is not the focused editable any more: end it where it is and
    /// fire `compositionend` with the marked text it had.
    fn end_lost_composition(&self, cx: &mut JSContext, document: &Document, target: &Element) {
        let text = std::mem::take(&mut *self.marked_text.borrow_mut());
        let context = document.editing_context(cx.no_gc(), target.upcast());
        match &context {
            EditingContext::TextControl(control) => {
                let control = control.text_control_element();
                control.text_input_mut().end_composition(None);
                control.maybe_update_composition_highlights();
            }
            EditingContext::Document(_) => self.end_host(cx, document, None, false),
        }
        self.composition_target.set(None);
        self.host_marked.set(None);
        fire_composition_event(cx, document, target, CompositionState::End, &text);
    }

    /// The old `Composition` events in an editing host (`EditingAction`s of the document):
    /// `StartComposition`, `InsertCompositionText` and `EndComposition`.
    pub(crate) fn perform_composition_action(
        &self,
        cx: &mut JSContext,
        document: &Document,
        action: &EditingAction,
    ) -> bool {
        match action {
            EditingAction::StartComposition => {
                self.host_marked.set(None);
                false
            }
            EditingAction::InsertCompositionText(text) => {
                self.update_host(cx, document, text, None, None)
            }
            EditingAction::EndComposition(text) => {
                let changed =
                    self.host_marked.get().is_some() && *self.marked_text.borrow() != *text;
                self.end_host(cx, document, Some(text), true);
                changed
            }
            _ => false,
        }
    }

    // ----- editing hosts --------------------------------------------------------------------

    /// Replace the marked text of the editing host (or the selection, when a composition just
    /// began) with `text`, with the events of a composition, and put the caret where the input
    /// method wants it. Returns whether the text was changed.
    fn update_host(
        &self,
        cx: &mut JSContext,
        document: &Document,
        text: &str,
        caret: Option<u32>,
        selected: Option<UnitRange<u32>>,
    ) -> bool {
        let events = ImeInputEvents::composition(text);
        let marked = self.host_marked.get();
        let Some(range) =
            composition::replace_marked_text(cx, document, marked.as_deref(), text, &events)
        else {
            return false;
        };
        self.host_marked.set(Some(&range));
        *self.marked_text.borrow_mut() = text.to_owned();
        self.place_host_caret(cx, document, &range, caret, selected);
        true
    }

    /// Put the selection (the caret) at `caret` in the marked text of an editing host, and show
    /// the marked text and the selected clause as highlights.
    fn place_host_caret(
        &self,
        cx: &mut JSContext,
        document: &Document,
        marked: &Range,
        caret: Option<u32>,
        selected: Option<UnitRange<u32>>,
    ) {
        if let Some(caret) = caret
            && let Some((node, offset)) = composition::position_in_range(cx, marked, caret)
            && let Some(selection) = document.GetSelection(cx)
        {
            let _ = selection.Collapse(cx, Some(&node), offset);
        }

        let highlights = document.highlights();
        if marked.collapsed() {
            highlights.clear(HighlightKind::Composition);
        } else {
            highlights.set(HighlightKind::Composition, vec![DomRoot::from_ref(marked)]);
        }
        let clause_range = selected.and_then(|selected| {
            let (start, start_offset) = composition::position_in_range(cx, marked, selected.start)?;
            let (end, end_offset) = composition::position_in_range(cx, marked, selected.end)?;
            Some(Range::new(
                cx,
                document,
                &start,
                start_offset,
                &end,
                end_offset,
            ))
        });
        match clause_range {
            Some(range) if !range.collapsed() => {
                highlights.set(HighlightKind::CompositionSelected, vec![range]);
            }
            _ => highlights.clear(HighlightKind::CompositionSelected),
        }
    }

    /// End the composition of an editing host: with `final_text`, the marked text first becomes
    /// that (events as for an update, unless it already is); then the highlights go and the
    /// caret is after the text.
    ///
    /// `move_caret` is false for a composition that lost its element (the focus, and with it the
    /// selection, is somewhere else by now: putting the caret back would steal it).
    fn end_host(
        &self,
        cx: &mut JSContext,
        document: &Document,
        final_text: Option<&str>,
        move_caret: bool,
    ) {
        if let Some(final_text) = final_text
            && self.host_marked.get().is_some()
            && *self.marked_text.borrow() != *final_text
        {
            self.update_host(cx, document, final_text, None, None);
        }

        if move_caret
            && let Some(marked) = self.host_marked.get()
            && let Some(selection) = document.GetSelection(cx)
        {
            let _ = selection.Collapse(cx, Some(&marked.end_container()), marked.end_offset());
        }
        self.host_marked.set(None);
        let highlights = document.highlights();
        highlights.clear(HighlightKind::Composition);
        highlights.clear(HighlightKind::CompositionSelected);
    }

    // ----- replacement ----------------------------------------------------------------------

    /// Select the text of the focused editable that `replacement` names, so that the text that
    /// comes next replaces it. It is honoured only while the last state message sent is the one
    /// `replacement` was made from and the editable still holds the text that message carried;
    /// otherwise nothing is selected and the text goes in at the selection. Never panics and
    /// never touches text outside the editable. Returns whether the selection was set.
    fn select_replacement(
        &self,
        cx: &mut JSContext,
        document: &Document,
        element: &Element,
        context: &EditingContext,
        replacement: &ImeReplacement,
    ) -> bool {
        let last = self.last_sent.borrow().clone();
        let Some(last) = last else {
            return false;
        };
        if last.sequence != replacement.sequence
            || last.is_password
            || replacement.range.start > replacement.range.end
            || replacement.range.end > TextWindow::utf16_len(&last.text)
            || !self
                .last_target
                .get()
                .is_some_and(|target| same_element(&target, element))
        {
            return false;
        }
        let start = last.text_start.saturating_add(replacement.range.start);
        let end = last.text_start.saturating_add(replacement.range.end);

        match context {
            EditingContext::TextControl(control) => {
                let control = control.text_control_element();
                let whole = String::from(control.text_input().get_content());
                if !window_is_unchanged(&whole, &last) {
                    return false;
                }
                let selected = control
                    .text_input_mut()
                    .select_utf16_range_checked(start, end);
                if selected {
                    control.maybe_update_shared_selection();
                }
                selected
            }
            EditingContext::Document(_) => {
                let Some(host_text) = HostText::new(cx, document, element) else {
                    return false;
                };
                if !window_is_unchanged(&host_text.text, &last) {
                    return false;
                }
                let (Some((start_node, start_offset)), Some((end_node, end_offset))) =
                    (host_text.position_at(start), host_text.position_at(end))
                else {
                    return false;
                };
                document.GetSelection(cx).is_some_and(|selection| {
                    selection
                        .SetBaseAndExtent(cx, &start_node, start_offset, &end_node, end_offset)
                        .is_ok()
                })
            }
        }
    }

    // ----- the text state message -----------------------------------------------------------

    /// Tell the browser what the focused editable holds, if that changed since the last time.
    /// Cheap when nothing is editable and nothing was sent.
    pub(crate) fn flush(&self, cx: &mut JSContext, document: &Document, mode: FlushMode) {
        // A composition whose element lost focus ends here.
        if let Some(target) = self.composition_target.get() {
            let still_focused =
                focused_element(document).is_some_and(|focused| same_element(&focused, &target));
            if !still_focused {
                self.end_lost_composition(cx, document, &target);
            }
        }

        let Some((element, context)) = focused_editable(cx, document) else {
            self.last_target.set(None);
            self.last_stamp.borrow_mut().take();
            if self.last_sent.borrow().is_some() {
                self.send(document, None);
            }
            return;
        };

        // Cheap first: a stamp copies nothing. A reflow that changed nothing about the editable
        // (an animation elsewhere on the page) ends here, whatever the size of the text.
        let target_changed = self
            .last_target
            .get()
            .is_none_or(|target| !same_element(&target, &element));
        let stamp = self.stamp_of(cx, document, &element, &context);
        if can_skip_rebuild(
            target_changed,
            self.last_sent.borrow().is_some(),
            stamp.is_some() && *self.last_stamp.borrow() == stamp,
            mode,
            self.caret_rect_stale.get(),
        ) {
            return;
        }

        let Some(mut state) = self.build_state(cx, document, &element, &context) else {
            return;
        };
        *self.last_stamp.borrow_mut() = stamp;

        let previous = self.last_sent.borrow().clone();
        let content_changed = previous
            .as_ref()
            .is_none_or(|previous| !previous.same_content_as(&state));
        let composing = state.marked.is_some();

        let measure = target_changed
            || composing
            || (mode == FlushMode::AfterReflow
                && (content_changed || self.caret_rect_stale.get() || previous.is_none()));
        if measure {
            state.caret_rect = self.measure_caret_rect(cx, document, &element, &context);
            self.caret_rect_stale.set(false);
        } else {
            // The caret may have moved; its rectangle is measured after the next reflow.
            state.caret_rect = previous.as_ref().and_then(|previous| previous.caret_rect);
            if content_changed {
                self.caret_rect_stale.set(true);
            }
        }

        // Nothing to send if it is the state last sent (the sequence number apart).
        let unchanged = !target_changed
            && previous.as_ref().is_some_and(|previous| {
                previous.same_content_as(&state) && previous.caret_rect == state.caret_rect
            });
        if unchanged {
            return;
        }

        self.last_target.set(Some(&element));
        self.send(document, Some(state));
    }

    /// What the state of the focused editable is made of, without copying its text.
    fn stamp_of(
        &self,
        cx: &mut JSContext,
        document: &Document,
        element: &Element,
        context: &EditingContext,
    ) -> Option<Stamp> {
        match context {
            EditingContext::TextControl(control) => {
                let control = control.text_control_element();
                Some(Stamp::Control {
                    input: control.text_input().change_stamp(),
                    password: control.is_password_field(),
                })
            },
            EditingContext::Document(_) => {
                let host = element.upcast::<Node>().editing_host_of()?;
                let selection = document
                    .selection()
                    .and_then(|selection| selection.active_range(cx))
                    .map(|range| range_stamp(&range));
                Some(Stamp::Host {
                    version: host.inclusive_descendants_version(),
                    selection,
                    marked: self.host_marked.get().map(|marked| range_stamp(&marked)),
                })
            },
        }
    }

    /// Send a message and remember it. Every message, `None` included, takes the next sequence
    /// number.
    fn send(&self, document: &Document, state: Option<TextInputState>) {
        let sequence = self.sequence.get() + 1;
        self.sequence.set(sequence);
        let document_id = u64::from(document.window().pipeline_id());
        let state = state.map(|mut state| {
            state.sequence = sequence;
            state.document = document_id;
            state
        });
        *self.last_sent.borrow_mut() = state.clone();
        // The "nothing is focused" message names its document: a late one from a page must not
        // hide the field of the frame the focus moved to.
        let message = match state {
            Some(state) => TextMsg::InputState(Box::new(state)),
            None => TextMsg::InputCleared(document_id),
        };
        document.send_to_embedder(EmbedderMsg::Mercury(
            Some(document.webview_id()),
            MercuryEmbedderMsg::Text(message),
        ));
    }

    /// The state of the focused editable, without a caret rectangle and a sequence number.
    fn build_state(
        &self,
        cx: &mut JSContext,
        document: &Document,
        element: &Element,
        context: &EditingContext,
    ) -> Option<TextInputState> {
        match context {
            EditingContext::TextControl(control) => {
                let control = control.text_control_element();
                let is_password = control.is_password_field();
                let multiline = control.text_input().mode() == Lines::Multiple;
                let window = if is_password {
                    // The text of a password field never leaves the engine.
                    TextWindow::new("", 0..0, None)
                } else {
                    let text_input = control.text_input();
                    let whole = String::from(text_input.get_content());
                    let selection =
                        text_input.selection_start_utf16().0..text_input.selection_end_utf16().0;
                    TextWindow::new(&whole, selection, text_input.composition_utf16_range())
                };
                Some(state_from_window(window, multiline, is_password))
            }
            EditingContext::Document(_) => {
                let host_text = HostText::new(cx, document, element)?;
                let total = TextWindow::utf16_len(&host_text.text);
                let range = document
                    .selection()
                    .and_then(|selection| selection.active_range(cx));
                let selection = range
                    .and_then(|range| {
                        let start =
                            host_text.offset_of(&range.start_container(), range.start_offset())?;
                        let end =
                            host_text.offset_of(&range.end_container(), range.end_offset())?;
                        Some(start..end.max(start))
                    })
                    .unwrap_or(total..total);
                let marked = self.host_marked.get().and_then(|marked| {
                    let start =
                        host_text.offset_of(&marked.start_container(), marked.start_offset())?;
                    let end = host_text.offset_of(&marked.end_container(), marked.end_offset())?;
                    Some(start..end.max(start))
                });
                let window = TextWindow::new(&host_text.text, selection, marked);
                Some(state_from_window(window, true, false))
            }
        }
    }

    /// The rectangle of the caret of the focused editable, in CSS pixels of its frame. This asks
    /// layout (forcing it if it is out of date), so it is done sparingly: see [`FlushMode`].
    fn measure_caret_rect(
        &self,
        cx: &mut JSContext,
        document: &Document,
        element: &Element,
        context: &EditingContext,
    ) -> Option<[f32; 4]> {
        // The text node the caret is in and its offset (UTF-16 code units).
        let located: Option<(DomRoot<Node>, u32)> = match context {
            EditingContext::TextControl(control) => {
                let control = control.text_control_element();
                let node = control.value_text_node()?;
                let caret = control.text_input().selection_end_utf16().0;
                Some((DomRoot::from_ref(node.upcast::<Node>()), caret))
            }
            EditingContext::Document(_) => document
                .selection()
                .and_then(|selection| selection.active_range(cx))
                .and_then(|range| {
                    let (node, offset) = (range.end_container(), range.end_offset());
                    node.is::<Text>().then_some((node, offset))
                }),
        };

        if let Some((node, caret)) = located {
            let rects: Vec<[f32; 4]> = node
                .border_boxes(cx)
                .iter()
                .map(|rect| {
                    [
                        rect.origin.x.to_f64_px() as f32,
                        rect.origin.y.to_f64_px() as f32,
                        rect.size.width.to_f64_px() as f32,
                        rect.size.height.to_f64_px() as f32,
                    ]
                })
                .collect();
            if let Some(rect) = TextWindow::estimate_caret_rect(&rects, caret, node.len()) {
                return Some(rect);
            }
        }

        // No box for the text (it is empty, or not rendered): the box of the editable itself.
        element.upcast::<Node>().border_box(cx).map(|rect| {
            [
                rect.origin.x.to_f64_px() as f32,
                rect.origin.y.to_f64_px() as f32,
                rect.size.width.to_f64_px() as f32,
                rect.size.height.to_f64_px() as f32,
            ]
        })
    }
}

fn state_from_window(window: TextWindow, multiline: bool, is_password: bool) -> TextInputState {
    TextInputState {
        document: 0,
        sequence: 0,
        text: window.text,
        text_start: window.text_start,
        selection: window.selection,
        marked: window.marked,
        caret_rect: None,
        multiline,
        is_password,
    }
}

/// Whether `whole` still has, where the last message put it, the text that message carried.
fn window_is_unchanged(whole: &str, last: &TextInputState) -> bool {
    let start = last.text_start;
    let end = start.saturating_add(TextWindow::utf16_len(&last.text));
    let (Some(start), Some(end)) = (
        TextWindow::utf16_to_byte_offset(whole, start),
        TextWindow::utf16_to_byte_offset(whole, end),
    ) else {
        return false;
    };
    start <= end && whole[start..end] == *last.text
}

/// Whether an event listener left the element in the rendered tree (it may have hidden it).
fn is_still_rendered(cx: &mut JSContext, element: &Element) -> bool {
    let node = element.upcast::<Node>();
    node.owner_window()
        .layout_reflow(cx, layout_api::QueryMsg::StyleQuery);
    node.is_being_rendered_or_delegates_rendering(None)
}

// ----- the plain text of an editing host ------------------------------------------------------

/// What the plain text of an editing host's block is made of.
struct HostPiece {
    /// A Text node, or the `<br>` (or block) that stands for a line break.
    node: DomRoot<Node>,
    /// Where the piece starts in the text, in UTF-16 code units.
    start: u32,
    /// How long it is, in UTF-16 code units.
    length: u32,
    is_text: bool,
}

/// The plain text of the block of an editing host that the selection is in (the whole host when
/// it has no blocks), with a map between offsets in that text and DOM positions. Text nodes
/// contribute their text, `<br>` and block boundaries a newline.
struct HostText {
    text: String,
    pieces: Vec<HostPiece>,
}

impl HostText {
    fn new(cx: &mut JSContext, document: &Document, element: &Element) -> Option<Self> {
        let host = element.upcast::<Node>().editing_host_of()?;
        // The block the selection ends in: the part of the host an input method asks about.
        let focus = document
            .selection()
            .and_then(|selection| selection.active_range(cx))
            .map(|range| range.end_container())
            .filter(|node| host.is_inclusive_ancestor_of(node))
            .unwrap_or_else(|| host.clone());
        let mut block = host.clone();
        for ancestor in focus.inclusive_ancestors(ShadowIncluding::No) {
            if *ancestor == *host {
                break;
            }
            if ancestor.is::<Element>() && ancestor.is_block_node() {
                block = ancestor;
                break;
            }
        }

        let mut built = HostText {
            text: String::new(),
            pieces: Vec::new(),
        };
        for node in block.traverse_preorder(ShadowIncluding::No) {
            if let Some(character_data) = node.downcast::<Text>() {
                let data = String::from(character_data.upcast::<CharacterData>().Data());
                built.push(&node, &data, true);
            } else if node.is::<HTMLBRElement>() {
                built.push(&node, "\n", false);
            } else if *node != *block
                && node.is::<Element>()
                && node.is_block_node()
                && !built.text.is_empty()
                && !built.text.ends_with('\n')
            {
                // The start of a block inside the block: a new line.
                built.push(&node, "\n", false);
            }
        }
        Some(built)
    }

    /// Append a piece of the text.
    fn push(&mut self, node: &DomRoot<Node>, piece_text: &str, is_text: bool) {
        let start = self
            .pieces
            .last()
            .map_or(0, |piece| piece.start + piece.length);
        let length = TextWindow::utf16_len(piece_text);
        self.text.push_str(piece_text);
        self.pieces.push(HostPiece {
            node: node.clone(),
            start,
            length,
            is_text,
        });
    }

    /// The offset in the text of the boundary point (`container`, `offset`), or `None` if it is
    /// not in this block.
    fn offset_of(&self, container: &Node, offset: u32) -> Option<u32> {
        if container.is::<Text>() {
            let piece = self
                .pieces
                .iter()
                .find(|piece| piece.is_text && &*piece.node == container)?;
            return Some(piece.start + offset.min(piece.length));
        }
        // A position between the children of an element.
        let inside = |piece: &&HostPiece| container.is_inclusive_ancestor_of(&piece.node);
        match container.children().nth(offset as usize) {
            Some(child) => self
                .pieces
                .iter()
                .find(|piece| child.is_inclusive_ancestor_of(&piece.node))
                .map(|piece| piece.start)
                .or_else(|| {
                    // Nothing inside the child: the end of what comes before it.
                    self.pieces
                        .iter()
                        .filter(inside)
                        .last()
                        .map(|piece| piece.start + piece.length)
                }),
            None => self
                .pieces
                .iter()
                .filter(inside)
                .last()
                .map(|piece| piece.start + piece.length),
        }
    }

    /// The DOM position of the offset `offset` in the text; an offset between two pieces is at
    /// the end of the earlier one. `None` past the end of the text.
    fn position_at(&self, offset: u32) -> Option<(DomRoot<Node>, u32)> {
        let piece = self
            .pieces
            .iter()
            .find(|piece| offset <= piece.start + piece.length)?;
        if piece.is_text {
            return Some((piece.node.clone(), offset - piece.start));
        }
        // A line break: before it or after it, in its parent.
        let parent = piece.node.GetParentNode()?;
        let index = piece.node.index();
        Some((
            parent,
            if offset <= piece.start {
                index
            } else {
                index + 1
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::text_input::{ClipboardProvider, SelectionDirection, TextInput};

    struct NoClipboard;

    impl ClipboardProvider for NoClipboard {
        fn get_text(&mut self) -> Result<String, String> {
            Err("no clipboard in tests".to_owned())
        }
        fn set_text(&mut self, _: String) {}
    }

    fn input(text: &str) -> TextInput<NoClipboard> {
        TextInput::new(Lines::Single, DOMString::from(text), NoClipboard)
    }

    #[test]
    fn a_flush_that_changed_nothing_skips_the_rebuild() {
        assert!(can_skip_rebuild(false, true, true, FlushMode::AfterReflow, false));
        assert!(can_skip_rebuild(false, true, true, FlushMode::AfterInput, true));
    }

    #[test]
    fn a_changed_target_stamp_or_a_first_message_rebuilds() {
        assert!(!can_skip_rebuild(true, true, true, FlushMode::AfterReflow, false));
        assert!(!can_skip_rebuild(false, false, true, FlushMode::AfterReflow, false));
        assert!(!can_skip_rebuild(false, true, false, FlushMode::AfterReflow, false));
    }

    #[test]
    fn a_stale_caret_rectangle_is_measured_after_a_reflow_only() {
        assert!(!can_skip_rebuild(false, true, true, FlushMode::AfterReflow, true));
        assert!(can_skip_rebuild(false, true, true, FlushMode::AfterInput, true));
    }

    #[test]
    fn the_stamp_of_an_untouched_text_input_does_not_change() {
        let mut text = input("hello");
        let before = text.change_stamp();
        // Reading and moving nothing: the same stamp, however often it is asked for.
        assert_eq!(before, text.change_stamp());
        text.clear_selection();
        assert_eq!(before, text.change_stamp());
    }

    #[test]
    fn an_edit_changes_the_stamp_even_when_the_length_does_not() {
        let mut text = input("hello");
        let before = text.change_stamp();

        text.set_content(DOMString::from("jello"));

        assert_ne!(before, text.change_stamp());
    }

    #[test]
    fn a_selection_change_changes_the_stamp() {
        let mut text = input("hello");
        let before = text.change_stamp();

        text.set_selection_range_utf16(
            servo_base::text::Utf16CodeUnits(1),
            servo_base::text::Utf16CodeUnits(3),
            SelectionDirection::None,
        );

        assert_ne!(before, text.change_stamp());
    }

    #[test]
    fn a_composition_changes_the_stamp() {
        let mut text = input("hello");
        let before = text.change_stamp();

        text.begin_composition();
        text.replace_composition("ni");
        let composing = text.change_stamp();
        assert_ne!(before, composing);

        text.replace_composition("nihao");
        assert_ne!(composing, text.change_stamp());
    }
}
