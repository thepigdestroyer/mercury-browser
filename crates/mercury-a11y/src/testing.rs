// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Test support: a document the pure logic can run on, built from `a11y_dom::Arena`.
//!
//! Owner: item 2 (`core-names`). Item 3 builds on it, so it is pushed first. `TestDom` is a
//! newtype because the orphan rule forbids implementing `Rendering` for `Arena` itself.

use a11y_dom::{Arena, ArenaNode, ComputedStyle, Document, Rect, Rendering};

use crate::host::HostState;

/// An `Arena` as a document with no layout and no engine state.
pub struct TestDom(pub Arena);

impl Document for TestDom {
    type N<'a> = ArenaNode<'a>;

    fn root(&self) -> ArenaNode<'_> {
        self.0.root()
    }

    fn node_count(&self) -> Option<usize> {
        Some(self.0.len())
    }
}

impl Rendering for TestDom {
    fn computed_style<'n>(&'n self, _node: ArenaNode<'n>) -> Option<ComputedStyle> {
        None
    }

    fn bounds<'n>(&'n self, _node: ArenaNode<'n>) -> Option<Rect> {
        None
    }
}

impl HostState for TestDom {}
