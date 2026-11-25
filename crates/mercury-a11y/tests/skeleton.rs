// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The pipeline runs end to end on a `TestDom` (the skeleton's smoke test).

use a11y_dom::{Arena, Document, Node, elements};
use accname::IdIndex;
use mercury_a11y::compute;
use mercury_a11y::testing::TestDom;

#[test]
fn compute_runs_on_a_test_dom() {
    let doc = TestDom(Arena::builder().open("p").text("hello").close().build());
    let ids = IdIndex::build(doc.root());
    let paragraph = elements(&doc)
        .find(|node| node.local_name() == "p")
        .expect("the paragraph exists");
    let semantics = compute(&doc, paragraph, &ids);
    assert!(!semantics.hidden);
}
