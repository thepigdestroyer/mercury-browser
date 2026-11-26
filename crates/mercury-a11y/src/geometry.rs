// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Node bounds: the union of rectangles, the bounding box of a transformed rectangle, and whether a
//! scroll moves a node.

use accesskit::{Affine, Rect};

// stub
pub fn union_rects(_rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    None
}

// stub
pub fn aabb_of_transformed(_rect: Rect, _transform: Affine) -> Option<Rect> {
    None
}

/// One spatial node on the way from a node to the root of the scroll tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpatialKind {
    Scroll(u64),
    ReferenceFrame,
    Sticky,
    Other,
}

// stub
pub fn scroll_applies(_chain_from_node_to_root: &[SpatialKind], _scrolled: u64) -> bool {
    false
}
