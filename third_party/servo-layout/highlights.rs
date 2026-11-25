/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: new file (lane txt-1, EB7). The arithmetic behind painting typed highlights: which
// part of a text fragment a highlight covers, where it sits and how thick its decoration is. It
// has no Stylo, WebRender or display list types in its signatures, so it is tested by
// `tests/highlights.rs` and in a scratch crate without a Servo build. `display_list/mod.rs`
// only turns the results into display items.

//! Geometry and colours for painting typed highlights ([`layout_api::TextHighlight`]).
//!
//! The data model and the splitting of overlapping highlights are in `layout_api`
//! ([`layout_api::resolve_segments`]). This module maps highlights from the offsets of the DOM
//! text node to the offsets of the laid-out (transformed) text, puts the resulting segments on
//! the glyphs of one text fragment and computes the rectangles of the decorations.

use std::ops::Range;

use app_units::Au;
use layout_api::{
    Decoration, DecorationSet, HighlightKind, Segment, TextHighlight, TextHighlights,
    resolve_segments,
};
use servo_base::text::{RangeAny, Utf32CodeUnits};

/// An opaque sRGB colour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HighlightColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl HighlightColor {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

/// The background of a find-in-page match (Chromium's colour).
pub const FIND_MATCH_BACKGROUND: HighlightColor = HighlightColor::new(0xFF, 0xFF, 0x00);
/// The background of the active find-in-page match (Chromium's colour).
pub const ACTIVE_FIND_MATCH_BACKGROUND: HighlightColor = HighlightColor::new(0xFF, 0x96, 0x32);
/// The background of a text fragment (`#:~:text=`) match.
pub const TEXT_FRAGMENT_BACKGROUND: HighlightColor = HighlightColor::new(0xFF, 0xE5, 0x00);
/// The wavy underline of a spelling error. The same red as `text-decoration-line:
/// spelling-error` in `display_list`.
pub const SPELLING_ERROR_COLOR: HighlightColor = HighlightColor::new(255, 0, 0);
/// The wavy underline of a grammar error. The same green as `text-decoration-line:
/// grammar-error` in `display_list`.
pub const GRAMMAR_ERROR_COLOR: HighlightColor = HighlightColor::new(0, 128, 0);

/// The thickness, in CSS px, of the underline of the marked text of an IME composition.
pub const COMPOSITION_UNDERLINE_CSS_PX: f32 = 1.0;
/// The thickness, in CSS px, of the underline of the selected clause of an IME composition.
pub const COMPOSITION_SELECTED_UNDERLINE_CSS_PX: f32 = 2.0;

/// The colour of the background that a highlight kind paints, if it paints one with a colour of
/// its own. `Custom` has no colour yet (the `::highlight()` pseudo-element would give it one).
pub fn background_color(kind: HighlightKind) -> Option<HighlightColor> {
    match kind {
        HighlightKind::FindMatch => Some(FIND_MATCH_BACKGROUND),
        HighlightKind::ActiveFindMatch => Some(ACTIVE_FIND_MATCH_BACKGROUND),
        HighlightKind::TextFragment => Some(TEXT_FRAGMENT_BACKGROUND),
        HighlightKind::Custom(_) |
        HighlightKind::Composition |
        HighlightKind::CompositionSelected |
        HighlightKind::Spelling |
        HighlightKind::Grammar => None,
    }
}

/// The colour of a decoration, or `None` for "the colour of the text".
pub fn decoration_color(decoration: Decoration) -> Option<HighlightColor> {
    match decoration {
        Decoration::Composition | Decoration::CompositionSelected => None,
        Decoration::Spelling => Some(SPELLING_ERROR_COLOR),
        Decoration::Grammar => Some(GRAMMAR_ERROR_COLOR),
    }
}

/// Round `css_px` down to a whole number of device pixels, but never below one device pixel, as
/// `display_list` does for the thickness of text decoration lines. See
/// <https://drafts.csswg.org/css-values-4/#snap-as-a-line-width>.
pub fn snapped_thickness(css_px: f32, device_pixel_ratio: f32) -> Au {
    if css_px <= 0.0 {
        return Au(0);
    }
    let device_pixel_ratio = if device_pixel_ratio > 0.0 {
        device_pixel_ratio
    } else {
        1.0
    };
    Au::from_f32_px((css_px * device_pixel_ratio).floor().max(1.0) / device_pixel_ratio)
}

/// Map the DOM-text-node ranges of `highlights` to ranges of the laid-out text of one text run
/// with `map` (`SharedTextRunData::map_dom_range_to_transformed_range`). White space collapsing
/// and `text-transform` can make a range empty (a collapsed space): those highlights are
/// dropped. The result has closed ranges only.
pub fn map_highlights(
    highlights: &[TextHighlight],
    map: impl Fn(RangeAny<Utf32CodeUnits>) -> Range<Utf32CodeUnits>,
) -> TextHighlights {
    highlights
        .iter()
        .filter_map(|highlight| {
            let mapped = map(highlight.range);
            (mapped.start < mapped.end).then_some(TextHighlight {
                kind: highlight.kind,
                range: RangeAny::from(mapped),
            })
        })
        .collect()
}

/// The highlights of a part of a text node: `highlights` clipped to `window` and shifted so that
/// their offsets are relative to the start of the window. This is what the text run of one
/// piece of a node that `::first-letter` splits needs, as it is for the selection (`push_text`
/// in `construct.rs`). A highlight that does not reach into the window is dropped.
pub fn highlights_in_window(
    highlights: &[TextHighlight],
    window: RangeAny<Utf32CodeUnits>,
) -> TextHighlights {
    let window_start = window.start().unwrap_or(Utf32CodeUnits(0));
    highlights
        .iter()
        .filter_map(|highlight| {
            highlight
                .range
                .intersect(window)
                .map(|range| TextHighlight {
                    kind: highlight.kind,
                    range: range.map(|offset| offset - window_start),
                })
        })
        .collect()
}

/// One glyph (or cluster) of a text fragment, as far as highlights are concerned. Any
/// justification of the glyph is already part of `advance`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlyphSpan {
    /// How many characters of the text the glyph stands for. Glyphs that continue the cluster
    /// of the previous glyph (a combining mark shaped on its own) have none.
    pub characters: Utf32CodeUnits,
    /// The inline advance of the glyph.
    pub advance: Au,
}

/// A [`Segment`] placed on a text fragment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlacedSegment {
    /// The inline offset of the start of the segment from the start of the fragment.
    pub x: Au,
    /// The inline size of the segment.
    pub width: Au,
    pub background: Option<HighlightKind>,
    pub decorations: DecorationSet,
}

/// The segments of the highlights of a text run that fall in one text fragment: map the
/// highlights to the text of the run, clip them to the fragment's characters
/// (`fragment_characters`, in the offsets of the run's text) and split the overlaps.
pub fn segments_for_fragment(
    highlights: &[TextHighlight],
    map: impl Fn(RangeAny<Utf32CodeUnits>) -> Range<Utf32CodeUnits>,
    fragment_characters: Range<Utf32CodeUnits>,
) -> Vec<Segment> {
    if highlights.is_empty() {
        return Vec::new();
    }
    resolve_segments(fragment_characters, &map_highlights(highlights, map))
}

/// Put `segments` (as [`segments_for_fragment`] returns them) on the glyphs of the fragment.
///
/// A segment that starts or ends inside a cluster (a ligature, a base character with its
/// combining marks) is widened to the whole cluster, so an IME underline or a find match never
/// stops in the middle of a glyph. (The document selection, which paints beside this, rounds
/// the other way. That code is unchanged.) Segments with no width are dropped.
pub fn place_segments(
    glyphs: impl IntoIterator<Item = GlyphSpan>,
    fragment_characters: Range<Utf32CodeUnits>,
    segments: &[Segment],
) -> Vec<PlacedSegment> {
    if segments.is_empty() {
        return Vec::new();
    }

    // `(position in the run's text, inline offset)` at every glyph boundary. Where a position
    // repeats (glyphs that have no character of their own), the later offset is the end of the
    // whole cluster, so it replaces the earlier one.
    let mut boundaries: Vec<(u32, Au)> = Vec::new();
    let mut position = fragment_characters.start.0;
    let mut offset = Au(0);
    boundaries.push((position, offset));
    for glyph in glyphs {
        position += glyph.characters.0;
        offset += glyph.advance;
        match boundaries.last_mut() {
            Some(last) if last.0 == position => last.1 = offset,
            _ => boundaries.push((position, offset)),
        }
    }

    let offset_at_or_before = |position: u32| {
        let index = boundaries.partition_point(|(boundary, _)| *boundary <= position);
        boundaries[index.saturating_sub(1)].1
    };
    let offset_at_or_after = |position: u32| {
        let index = boundaries.partition_point(|(boundary, _)| *boundary < position);
        boundaries[index.min(boundaries.len() - 1)].1
    };

    segments
        .iter()
        .filter_map(|segment| {
            let start = segment.range.start.0.max(fragment_characters.start.0);
            let end = segment.range.end.0.min(fragment_characters.end.0);
            if start >= end {
                return None;
            }
            let x = offset_at_or_before(start);
            let end_x = offset_at_or_after(end);
            (end_x > x).then_some(PlacedSegment {
                x,
                width: end_x - x,
                background: segment.background,
                decorations: segment.decorations,
            })
        })
        .collect()
}

/// A rectangle of one decoration, relative to the top-left corner of the text fragment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecorationRect {
    pub decoration: Decoration,
    pub x: Au,
    pub y: Au,
    pub width: Au,
    pub height: Au,
}

/// The rectangles of the decorations of a placed segment, in painting order.
///
/// `underline_y` is the distance from the top of the fragment to the top of an underline
/// (`ascent - underline_offset` of the font, as for `text-decoration-line: underline`),
/// `wave_thickness_css_px` the thickness of spelling and grammar waves (the font's underline
/// size). The composition underlines are 1 and 2 CSS px thick, rounded down to whole device
/// pixels but never below one.
pub fn decoration_rects(
    placed: &PlacedSegment,
    underline_y: Au,
    wave_thickness_css_px: f32,
    device_pixel_ratio: f32,
) -> Vec<DecorationRect> {
    placed
        .decorations
        .iter()
        .map(|decoration| {
            let thickness_css_px = match decoration {
                Decoration::Composition => COMPOSITION_UNDERLINE_CSS_PX,
                Decoration::CompositionSelected => COMPOSITION_SELECTED_UNDERLINE_CSS_PX,
                Decoration::Spelling | Decoration::Grammar => wave_thickness_css_px,
            };
            DecorationRect {
                decoration,
                x: placed.x,
                y: underline_y,
                width: placed.width,
                height: snapped_thickness(thickness_css_px, device_pixel_ratio),
            }
        })
        .collect()
}
