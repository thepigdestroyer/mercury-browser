// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What only the engine knows about a node: native state, the current value of a form control and
//! the like. Every method has a "nothing known" default, so an implementation can start small and
//! a method can be added without breaking one.
//!
//! Owner: item 3 (`core-states`).

use a11y_dom::Document;

/// Native (not ARIA) state of an element, as the selectors `:checked`, `:disabled` and so on see it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeFlags {
    pub checked: bool,
    pub indeterminate: bool,
    pub disabled: bool,
    pub required: bool,
    pub read_only: bool,
    /// `:invalid` (constraint validation), not `aria-invalid`.
    pub invalid: bool,
    /// `<details open>`, an open `<select>` popup, an open `<dialog>`.
    pub open: bool,
    pub selected: bool,
    pub focusable: bool,
    /// A text control or `contenteditable` host the user can type into.
    pub editable: bool,
    pub multiline: bool,
    pub scroll_container: bool,
}

/// The value of a range-like control (`input type=range|number`, `progress`, `meter`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeValue {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: Option<f64>,
}

/// The engine-side half of a DOM. Implemented by the layout crate's adapter and by
/// [`crate::testing`].
pub trait HostState: Document {
    /// Native state flags of an element (default: nothing set).
    fn native_flags<'n>(&'n self, _node: Self::N<'n>) -> NativeFlags {
        NativeFlags::default()
    }

    /// The current value of a text control, `<select>` or `<output>` (not the `value` attribute).
    /// **Never the text of a password field**: an implementation returns `None` for one, and
    /// `states` checks the input type as well.
    fn control_value<'n>(&'n self, _node: Self::N<'n>) -> Option<String> {
        None
    }

    /// The numeric value of a range-like control.
    fn range<'n>(&'n self, _node: Self::N<'n>) -> Option<RangeValue> {
        None
    }

    /// Position (1-based) and size of the set a native element belongs to (list items, options,
    /// radio buttons of one group), if the engine knows it.
    fn position_in_set<'n>(&'n self, _node: Self::N<'n>) -> Option<(usize, usize)> {
        None
    }
}
