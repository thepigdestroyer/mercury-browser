// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! List item positions (EE9): the `li` siblings of a list. Item 3 (`a11y-2`).

use mercury_a11y::lists::{ListItemInfo, list_item_info};
use mercury_a11y::testing::TestDom;

fn info(html: &str, id: &str) -> Option<ListItemInfo> {
    let dom = TestDom::from_html(html);
    list_item_info(&dom, dom.by_id(id))
}

#[test]
fn li_position_and_size() {
    let html = "<ul><li id=a>one</li><li id=b>two</li><li id=c>three</li></ul>";
    let size = |id| info(html, id).map(|i| (i.position, i.size));
    assert_eq!(size("a"), Some((1, 3)));
    assert_eq!(size("b"), Some((2, 3)));
    assert_eq!(size("c"), Some((3, 3)));
}

#[test]
fn nested_lists_count_separately() {
    let html = "<ul><li id=outer1>a<ol><li id=inner1>x</li><li id=inner2>y</li></ol></li>\
                <li id=outer2>b</li></ul>";
    let inner = info(html, "inner2").expect("a list item");
    assert_eq!((inner.position, inner.size), (2, 2));
    let outer = info(html, "outer2").expect("a list item");
    assert_eq!((outer.position, outer.size), (2, 2));
}

#[test]
fn an_ol_start_and_reversed_do_not_change_position() {
    let html = "<ol start=5 reversed><li id=a>one</li><li id=b value=9>two</li></ol>";
    let size = |id| info(html, id).map(|i| (i.position, i.size));
    assert_eq!(size("a"), Some((1, 2)));
    assert_eq!(size("b"), Some((2, 2)));
}

#[test]
fn list_style_none_keeps_the_list() {
    let html = "<ul style=\"list-style: none\"><li id=a>one</li><li id=b>two</li></ul>";
    assert_eq!(info(html, "b").map(|i| (i.position, i.size)), Some((2, 2)));
}
