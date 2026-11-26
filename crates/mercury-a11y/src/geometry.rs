// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Node bounds: the union of rectangles, the bounding box of a transformed rectangle, and whether a
//! scroll moves a node.

use accesskit::{Affine, Rect};

fn is_finite_rect(rect: &Rect) -> bool {
    rect.x0.is_finite() && rect.y0.is_finite() && rect.x1.is_finite() && rect.y1.is_finite()
}

/// The smallest rectangle that holds every finite rectangle of `rects`, or `None` when there is
/// none.
pub fn union_rects(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    rects
        .into_iter()
        .filter(is_finite_rect)
        .reduce(|acc, rect| acc.union(rect))
}

/// The bounding box of `rect` after `transform`, or `None` when the result is not finite.
pub fn aabb_of_transformed(rect: Rect, transform: Affine) -> Option<Rect> {
    let bounds = transform.transform_rect_bbox(rect);
    is_finite_rect(&bounds).then_some(bounds)
}

/// One spatial node on the way from a node to the root of the scroll tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpatialKind {
    Scroll(u64),
    ReferenceFrame,
    Sticky,
    Other,
}

/// Whether a scroll of `scrolled` moves a node whose spatial chain (node first, root last) is
/// `chain_from_node_to_root`: the node scrolls with the scroller it sits inside.
pub fn scroll_applies(chain_from_node_to_root: &[SpatialKind], scrolled: u64) -> bool {
    chain_from_node_to_root.contains(&SpatialKind::Scroll(scrolled))
}
