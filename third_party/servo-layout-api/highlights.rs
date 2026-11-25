/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: new file (lane txt-1, EB7). Typed highlight ranges for text runs. Nothing here depends
// on Servo beyond `servo_base::text`, so the logic is tested in a scratch crate and in
// `servo-layout/tests/highlights.rs`.

//! Typed highlight ranges for text runs.
//!
//! Layout used to know exactly one range per text run, the document selection (or the selection
//! of a text control). This module adds a small set of *typed* highlights per text run, beside
//! that selection fast path (which is unchanged): the marked text of an IME composition, find
//! matches, the active find match, text fragments, spelling and grammar errors. Script owns the
//! ranges (`Document::highlights()` in `script`); layout only receives them as
//! [`TextHighlights`], in offsets of the DOM text node (counted in `char`s, like the selection),
//! and paints them without a new layout.
//!
//! [`resolve_segments`] turns the highlights of one text run into non-overlapping
//! [`Segment`]s, each with the one background that wins and the full stack of decorations, so
//! that painting never has to think about overlaps.

use std::ops::Range;

use servo_base::text::{RangeAny, Utf32CodeUnits};

/// What a highlight is for. The kind decides how it paints: a background under the glyphs
/// (`FindMatch`, `ActiveFindMatch`, `TextFragment`, `Custom`) or a decoration over them
/// (`Composition`, `CompositionSelected`, `Spelling`, `Grammar`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HighlightKind {
    /// The marked text of an IME composition: a thin underline.
    Composition,
    /// The clause of an IME composition that the input method has selected: a thick underline.
    CompositionSelected,
    /// A find-in-page match.
    FindMatch,
    /// The find-in-page match that is currently active.
    ActiveFindMatch,
    /// A text fragment directive (`#:~:text=`) match.
    TextFragment,
    /// A spelling error: a red wavy underline.
    Spelling,
    /// A grammar error: a green wavy underline.
    Grammar,
    /// A highlight with a producer-defined identifier (the CSS Custom Highlight API later). It
    /// paints as a background, below every other background, and has no colour of its own yet.
    Custom(u32),
}

/// A decoration that paints over the glyphs of a highlighted range. The order of the variants is
/// the order of painting: a later decoration paints over an earlier one.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Decoration {
    /// A 1 px underline in the text colour.
    Composition,
    /// A 2 px underline in the text colour.
    CompositionSelected,
    /// A red wavy underline.
    Spelling,
    /// A green wavy underline.
    Grammar,
}

impl Decoration {
    /// Every decoration, in painting order.
    pub const ALL: [Decoration; 4] = [
        Decoration::Composition,
        Decoration::CompositionSelected,
        Decoration::Spelling,
        Decoration::Grammar,
    ];

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// A set of [`Decoration`]s: the stack of decorations over one segment of text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DecorationSet(u8);

impl DecorationSet {
    /// The empty set.
    pub const EMPTY: DecorationSet = DecorationSet(0);

    /// Whether `decoration` is in the set.
    pub fn contains(self, decoration: Decoration) -> bool {
        self.0 & decoration.bit() != 0
    }

    /// Add `decoration` to the set.
    pub fn insert(&mut self, decoration: Decoration) {
        self.0 |= decoration.bit();
    }

    /// Whether the set has no decoration.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The decorations in the set, in painting order.
    pub fn iter(self) -> impl Iterator<Item = Decoration> {
        Decoration::ALL
            .into_iter()
            .filter(move |decoration| self.contains(*decoration))
    }
}

impl HighlightKind {
    /// For a kind that paints a background, its rank among backgrounds: where backgrounds
    /// overlap, the highest rank wins. `Custom` ranks below everything else (and among
    /// `Custom`s the higher identifier wins), then `FindMatch < TextFragment < ActiveFindMatch`.
    pub fn background_rank(self) -> Option<(u8, u32)> {
        match self {
            HighlightKind::Custom(id) => Some((0, id)),
            HighlightKind::FindMatch => Some((1, 0)),
            HighlightKind::TextFragment => Some((2, 0)),
            HighlightKind::ActiveFindMatch => Some((3, 0)),
            HighlightKind::Composition |
            HighlightKind::CompositionSelected |
            HighlightKind::Spelling |
            HighlightKind::Grammar => None,
        }
    }

    /// Whether this kind paints a background under the glyphs.
    pub fn paints_background(self) -> bool {
        self.background_rank().is_some()
    }

    /// The decoration this kind paints over the glyphs, if it is a decoration kind.
    pub fn decoration(self) -> Option<Decoration> {
        match self {
            HighlightKind::Composition => Some(Decoration::Composition),
            HighlightKind::CompositionSelected => Some(Decoration::CompositionSelected),
            HighlightKind::Spelling => Some(Decoration::Spelling),
            HighlightKind::Grammar => Some(Decoration::Grammar),
            HighlightKind::FindMatch |
            HighlightKind::ActiveFindMatch |
            HighlightKind::TextFragment |
            HighlightKind::Custom(_) => None,
        }
    }
}

/// One highlighted range of one text node.
///
/// The range is in `char`s (UTF-32 code units) of the DOM text node, like the selection of a text
/// run; an unset bound means "to the start/end of the node". It is not clipped to the node and
/// may be empty: [`resolve_segments`] deals with both.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextHighlight {
    pub kind: HighlightKind,
    pub range: RangeAny<Utf32CodeUnits>,
}

/// The highlights of one text node. No `SmallVec` here (this crate has no such dependency and an
/// empty `Vec` does not allocate): the common case is no highlight at all.
pub type TextHighlights = Vec<TextHighlight>;

/// A part of a text run, between two consecutive highlight boundaries, with everything that
/// paints on it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Segment {
    /// The characters this segment covers, in the coordinates the highlights were given in.
    pub range: Range<Utf32CodeUnits>,
    /// The background that wins on this segment, if any highlight paints one here.
    pub background: Option<HighlightKind>,
    /// Every decoration that applies on this segment. Decorations stack: they are all painted.
    pub decorations: DecorationSet,
}

/// Split the highlights that fall inside `run_range` into non-overlapping [`Segment`]s, sorted by
/// position.
///
/// `run_range` and the ranges of `highlights` must be in the same coordinates (DOM offsets of one
/// text node, or offsets of the transformed text of one run: the arithmetic does not care). Each
/// highlight is clipped to `run_range`; highlights that end up empty (outside the run, or empty
/// to begin with) are dropped. Where highlights overlap, the segment gets the background of the
/// highest-ranked background kind ([`HighlightKind::background_rank`]) and the union of the
/// decorations. Adjacent segments that would paint identically are merged, and a segment on
/// which nothing paints is not returned.
pub fn resolve_segments(
    run_range: Range<Utf32CodeUnits>,
    highlights: &[TextHighlight],
) -> Vec<Segment> {
    let run_start = run_range.start.0;
    let run_end = run_range.end.0;
    if highlights.is_empty() || run_start >= run_end {
        return Vec::new();
    }

    // Clip to the run and drop what is empty.
    let mut clipped: Vec<(u32, u32, HighlightKind)> = highlights
        .iter()
        .filter_map(|highlight| {
            let start = highlight
                .range
                .start()
                .map_or(run_start, |start| start.0.max(run_start));
            let end = highlight
                .range
                .end()
                .map_or(run_end, |end| end.0.min(run_end));
            (start < end).then_some((start, end, highlight.kind))
        })
        .collect();
    if clipped.is_empty() {
        return Vec::new();
    }
    clipped.sort_by_key(|(start, ..)| *start);

    // Every boundary: between two consecutive ones the set of active highlights is constant.
    let mut boundaries: Vec<u32> = clipped
        .iter()
        .flat_map(|(start, end, _)| [*start, *end])
        .collect();
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut segments: Vec<Segment> = Vec::new();
    let mut next_to_activate = 0;
    let mut active: Vec<(u32, HighlightKind)> = Vec::new();
    for pair in boundaries.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        while next_to_activate < clipped.len() && clipped[next_to_activate].0 <= from {
            let (_, end, kind) = clipped[next_to_activate];
            active.push((end, kind));
            next_to_activate += 1;
        }
        // `end > from` implies `end >= to`, as every end is a boundary.
        active.retain(|(end, _)| *end > from);

        let background = active
            .iter()
            .filter_map(|(_, kind)| kind.background_rank().map(|rank| (rank, *kind)))
            .max_by_key(|(rank, _)| *rank)
            .map(|(_, kind)| kind);
        let mut decorations = DecorationSet::EMPTY;
        for (_, kind) in &active {
            if let Some(decoration) = kind.decoration() {
                decorations.insert(decoration);
            }
        }
        if background.is_none() && decorations.is_empty() {
            continue;
        }

        if let Some(previous) = segments.last_mut() &&
            previous.range.end.0 == from &&
            previous.background == background &&
            previous.decorations == decorations
        {
            previous.range.end = Utf32CodeUnits(to);
            continue;
        }
        segments.push(Segment {
            range: Utf32CodeUnits(from)..Utf32CodeUnits(to),
            background,
            decorations,
        });
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u32, end: u32) -> RangeAny<Utf32CodeUnits> {
        RangeAny::new(Some(Utf32CodeUnits(start)), Some(Utf32CodeUnits(end)))
    }

    fn highlight(kind: HighlightKind, start: u32, end: u32) -> TextHighlight {
        TextHighlight {
            kind,
            range: range(start, end),
        }
    }

    fn run(start: u32, end: u32) -> Range<Utf32CodeUnits> {
        Utf32CodeUnits(start)..Utf32CodeUnits(end)
    }

    fn decorations(of: &[Decoration]) -> DecorationSet {
        let mut set = DecorationSet::EMPTY;
        for decoration in of {
            set.insert(*decoration);
        }
        set
    }

    #[test]
    fn no_highlights_no_segments() {
        assert!(resolve_segments(run(0, 10), &[]).is_empty());
        assert!(
            resolve_segments(run(5, 5), &[highlight(HighlightKind::FindMatch, 0, 10)]).is_empty()
        );
    }

    #[test]
    fn one_background_is_clipped_to_the_run() {
        let segments = resolve_segments(run(2, 8), &[highlight(HighlightKind::FindMatch, 0, 5)]);
        assert_eq!(
            segments,
            vec![Segment {
                range: run(2, 5),
                background: Some(HighlightKind::FindMatch),
                decorations: DecorationSet::EMPTY,
            }]
        );
    }

    #[test]
    fn open_bounds_mean_the_whole_run() {
        let whole = TextHighlight {
            kind: HighlightKind::Spelling,
            range: RangeAny::full(),
        };
        let from_start = TextHighlight {
            kind: HighlightKind::TextFragment,
            range: RangeAny::from_start_to(Utf32CodeUnits(3)),
        };
        let to_end = TextHighlight {
            kind: HighlightKind::Grammar,
            range: RangeAny::new(Some(Utf32CodeUnits(7)), None),
        };
        let segments = resolve_segments(run(0, 10), &[whole, from_start, to_end]);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].range, run(0, 3));
        assert_eq!(segments[0].background, Some(HighlightKind::TextFragment));
        assert_eq!(
            segments[0].decorations,
            decorations(&[Decoration::Spelling])
        );
        assert_eq!(segments[1].range, run(3, 7));
        assert_eq!(segments[1].background, None);
        assert_eq!(
            segments[1].decorations,
            decorations(&[Decoration::Spelling])
        );
        assert_eq!(segments[2].range, run(7, 10));
        assert_eq!(
            segments[2].decorations,
            decorations(&[Decoration::Spelling, Decoration::Grammar])
        );
    }

    #[test]
    fn a_highlight_outside_the_run_is_dropped() {
        let highlights = [
            highlight(HighlightKind::FindMatch, 0, 4),
            highlight(HighlightKind::Spelling, 10, 12),
            // Touching the run is not overlapping it.
            highlight(HighlightKind::Grammar, 8, 9),
            highlight(HighlightKind::Composition, 4, 5),
        ];
        let segments = resolve_segments(run(5, 8), &highlights);
        assert!(segments.is_empty(), "{segments:?}");
    }

    #[test]
    fn empty_ranges_are_dropped() {
        let highlights = [
            highlight(HighlightKind::FindMatch, 3, 3),
            // A backwards range is empty too.
            highlight(HighlightKind::Spelling, 6, 2),
            highlight(HighlightKind::Composition, 5, 6),
        ];
        let segments = resolve_segments(run(0, 10), &highlights);
        assert_eq!(
            segments,
            vec![Segment {
                range: run(5, 6),
                background: None,
                decorations: decorations(&[Decoration::Composition]),
            }]
        );
    }

    #[test]
    fn three_overlapping_highlights_are_split() {
        // 0         10
        //  FindMatch  [0.......8)
        //  Spelling        [4.........12)
        //  ActiveFind   [2.....6)
        let highlights = [
            highlight(HighlightKind::FindMatch, 0, 8),
            highlight(HighlightKind::Spelling, 4, 12),
            highlight(HighlightKind::ActiveFindMatch, 2, 6),
        ];
        let segments = resolve_segments(run(0, 10), &highlights);
        let spelling = decorations(&[Decoration::Spelling]);
        assert_eq!(
            segments,
            vec![
                Segment {
                    range: run(0, 2),
                    background: Some(HighlightKind::FindMatch),
                    decorations: DecorationSet::EMPTY,
                },
                Segment {
                    range: run(2, 4),
                    background: Some(HighlightKind::ActiveFindMatch),
                    decorations: DecorationSet::EMPTY,
                },
                Segment {
                    range: run(4, 6),
                    background: Some(HighlightKind::ActiveFindMatch),
                    decorations: spelling,
                },
                Segment {
                    range: run(6, 8),
                    background: Some(HighlightKind::FindMatch),
                    decorations: spelling,
                },
                Segment {
                    range: run(8, 10),
                    background: None,
                    decorations: spelling,
                },
            ]
        );
    }

    #[test]
    fn background_order_is_find_then_fragment_then_active() {
        let order = [
            HighlightKind::FindMatch,
            HighlightKind::TextFragment,
            HighlightKind::ActiveFindMatch,
        ];
        // Whatever the order they are given in, the highest rank wins.
        for first in 0..3 {
            for second in 0..3 {
                if first == second {
                    continue;
                }
                let highlights = [
                    highlight(order[first], 0, 4),
                    highlight(order[second], 0, 4),
                ];
                let segments = resolve_segments(run(0, 4), &highlights);
                assert_eq!(segments.len(), 1);
                assert_eq!(segments[0].background, Some(order[first.max(second)]));
            }
        }
    }

    #[test]
    fn custom_ranks_below_every_other_background() {
        let highlights = [
            highlight(HighlightKind::Custom(9), 0, 4),
            highlight(HighlightKind::FindMatch, 0, 4),
            highlight(HighlightKind::Custom(3), 0, 4),
        ];
        let segments = resolve_segments(run(0, 4), &highlights);
        assert_eq!(segments[0].background, Some(HighlightKind::FindMatch));

        let customs = [
            highlight(HighlightKind::Custom(3), 0, 4),
            highlight(HighlightKind::Custom(9), 0, 4),
        ];
        let segments = resolve_segments(run(0, 4), &customs);
        assert_eq!(segments[0].background, Some(HighlightKind::Custom(9)));
    }

    #[test]
    fn decorations_stack_and_paint_in_order() {
        let highlights = [
            highlight(HighlightKind::Grammar, 0, 5),
            highlight(HighlightKind::CompositionSelected, 0, 5),
            highlight(HighlightKind::Spelling, 0, 5),
            highlight(HighlightKind::Composition, 0, 5),
        ];
        let segments = resolve_segments(run(0, 5), &highlights);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].background, None);
        assert_eq!(
            segments[0].decorations.iter().collect::<Vec<_>>(),
            vec![
                Decoration::Composition,
                Decoration::CompositionSelected,
                Decoration::Spelling,
                Decoration::Grammar,
            ]
        );
    }

    #[test]
    fn the_same_kind_twice_does_not_split_a_segment() {
        let highlights = [
            highlight(HighlightKind::FindMatch, 0, 3),
            highlight(HighlightKind::FindMatch, 3, 6),
            highlight(HighlightKind::FindMatch, 2, 4),
        ];
        let segments = resolve_segments(run(0, 10), &highlights);
        assert_eq!(
            segments,
            vec![Segment {
                range: run(0, 6),
                background: Some(HighlightKind::FindMatch),
                decorations: DecorationSet::EMPTY,
            }]
        );
    }

    #[test]
    fn a_gap_between_highlights_is_not_a_segment() {
        let highlights = [
            highlight(HighlightKind::FindMatch, 0, 2),
            highlight(HighlightKind::FindMatch, 4, 6),
        ];
        let segments = resolve_segments(run(0, 10), &highlights);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].range, run(0, 2));
        assert_eq!(segments[1].range, run(4, 6));
    }

    #[test]
    fn many_highlights_stay_sorted_and_disjoint() {
        // 500 find matches and a spelling error over all of them: boundaries are the point.
        let mut highlights: Vec<TextHighlight> = (0..500)
            .rev()
            .map(|i| highlight(HighlightKind::FindMatch, i * 4, i * 4 + 2))
            .collect();
        highlights.push(highlight(HighlightKind::Spelling, 0, 2000));
        let segments = resolve_segments(run(0, 2000), &highlights);
        assert_eq!(segments.len(), 1000);
        for pair in segments.windows(2) {
            assert!(pair[0].range.end <= pair[1].range.start);
        }
        assert_eq!(segments[0].range, run(0, 2));
        assert_eq!(segments[1].range, run(2, 4));
        assert_eq!(segments[1].background, None);
        assert!(segments[1].decorations.contains(Decoration::Spelling));
    }

    #[test]
    fn kinds_know_how_they_paint() {
        for kind in [
            HighlightKind::FindMatch,
            HighlightKind::ActiveFindMatch,
            HighlightKind::TextFragment,
            HighlightKind::Custom(1),
        ] {
            assert!(kind.paints_background());
            assert_eq!(kind.decoration(), None);
        }
        for kind in [
            HighlightKind::Composition,
            HighlightKind::CompositionSelected,
            HighlightKind::Spelling,
            HighlightKind::Grammar,
        ] {
            assert!(!kind.paints_background());
            assert!(kind.decoration().is_some());
        }
    }
}
