// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Cleaning what a backend or a helper process says: lengths, control characters, finite
//! rectangles, page targets that exist, and the counts and depths of lists.
//!
//! The helper cleans its answers before it sends them ([`crate::serve`]), and the browser cleans
//! them again when they arrive ([`crate::client`]), with the same functions: the helper is the
//! process that reads the hostile file, so a compromised one must not be able to hand the
//! browser a rectangle that is not a number, an outline of millions of entries or a title of
//! megabytes by skipping its own cleaning.

use crate::error::PdfError;
use crate::types::{DocumentInfo, Link, LinkTarget, OutlineItem, Rect, SearchHit, limits};

/// The longest detail text of an error that is sent, in bytes.
pub(crate) const DETAIL_BYTES: usize = 512;
/// The most rectangles kept for one search hit.
pub(crate) const MAX_RECTS_PER_HIT: usize = 256;

/// `text` cut to at most `max` bytes at a character boundary, with control characters (and so
/// line breaks) replaced by spaces.
pub(crate) fn clip(text: &str, max: usize) -> String {
    let mut clipped = String::with_capacity(text.len().min(max));
    for ch in text.chars() {
        let ch = if ch.is_control() { ' ' } else { ch };
        if clipped.len() + ch.len_utf8() > max {
            break;
        }
        clipped.push(ch);
    }
    clipped
}

pub(crate) fn clean_error(error: PdfError) -> PdfError {
    match error {
        PdfError::Corrupt(detail) => PdfError::Corrupt(clip(&detail, DETAIL_BYTES)),
        PdfError::Unsupported(detail) => PdfError::Unsupported(clip(&detail, DETAIL_BYTES)),
        PdfError::TooLarge(detail) => PdfError::TooLarge(clip(&detail, DETAIL_BYTES)),
        PdfError::Io(detail) => PdfError::Io(clip(&detail, DETAIL_BYTES)),
        PdfError::LibraryMissing(detail) => PdfError::LibraryMissing(clip(&detail, DETAIL_BYTES)),
        PdfError::InvalidRequest(detail) => PdfError::InvalidRequest(clip(&detail, DETAIL_BYTES)),
        PdfError::WorkerLost(detail) => PdfError::WorkerLost(clip(&detail, DETAIL_BYTES)),
        other => other,
    }
}

/// Accepts what a backend says about a freshly opened document, or refuses the document.
pub(crate) fn accept_document(info: DocumentInfo) -> Result<DocumentInfo, PdfError> {
    if info.pages.is_empty() {
        return Err(PdfError::Corrupt("the document has no pages".to_owned()));
    }
    if info.pages.len() > limits::MAX_PAGES as usize {
        return Err(PdfError::TooLarge("too many pages".to_owned()));
    }
    let usable = |side: f32| side.is_finite() && side > 0.0;
    if info
        .pages
        .iter()
        .any(|page| !usable(page.width) || !usable(page.height))
    {
        return Err(PdfError::Corrupt("a page has no usable size".to_owned()));
    }
    let title = info
        .title
        .map(|title| clip(&title, limits::MAX_STRING_BYTES).trim().to_owned())
        .filter(|title| !title.is_empty());
    Ok(DocumentInfo {
        pages: info.pages,
        title,
    })
}

pub(crate) fn finite_rect(rect: &Rect) -> bool {
    [rect.left, rect.top, rect.right, rect.bottom]
        .iter()
        .all(|side| side.is_finite())
}

/// The target as it may be sent, or `None` when the link is not usable: an address that is
/// too long or holds control characters, a page that does not exist.
pub(crate) fn clean_target(target: LinkTarget, page_count: usize) -> Option<LinkTarget> {
    match target {
        LinkTarget::Uri(uri) => {
            let usable = !uri.is_empty()
                && uri.len() <= limits::MAX_STRING_BYTES
                && !uri.chars().any(char::is_control);
            usable.then_some(LinkTarget::Uri(uri))
        }
        LinkTarget::Page { page, y } => {
            let y = y.filter(|y| y.is_finite());
            ((page as usize) < page_count).then_some(LinkTarget::Page { page, y })
        }
    }
}

pub(crate) fn clean_links(links: Vec<Link>, page_count: usize) -> Vec<Link> {
    links
        .into_iter()
        .filter(|link| finite_rect(&link.rect) && !link.rect.is_empty())
        .filter_map(|link| {
            clean_target(link.target, page_count).map(|target| Link {
                rect: link.rect,
                target,
            })
        })
        .take(limits::MAX_LINKS_PER_PAGE)
        .collect()
}

pub(crate) fn clean_outline(
    items: Vec<OutlineItem>,
    page_count: usize,
    budget: &mut usize,
    depth: usize,
) -> Vec<OutlineItem> {
    let mut cleaned = Vec::new();
    for item in items {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let children = if depth < limits::MAX_OUTLINE_DEPTH {
            clean_outline(item.children, page_count, budget, depth + 1)
        } else {
            Vec::new()
        };
        cleaned.push(OutlineItem {
            title: clip(&item.title, limits::MAX_STRING_BYTES)
                .trim()
                .to_owned(),
            target: item
                .target
                .and_then(|target| clean_target(target, page_count)),
            children,
        });
    }
    cleaned
}

pub(crate) fn clean_hits(hits: Vec<SearchHit>, page: u32) -> Vec<SearchHit> {
    hits.into_iter()
        .take(limits::MAX_HITS_PER_PAGE)
        .map(|mut hit| {
            hit.page = page;
            hit.rects
                .retain(|rect| finite_rect(rect) && !rect.is_empty());
            hit.rects.truncate(MAX_RECTS_PER_HIT);
            hit
        })
        .collect()
}
