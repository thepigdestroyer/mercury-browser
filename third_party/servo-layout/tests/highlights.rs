/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: new file (lane txt-1, EB7). Tests of the typed highlight arithmetic in
// `layout_api::highlights` (splitting overlaps) and `layout::highlights` (mapping to the laid-out
// text, putting segments on glyphs, decoration rectangles). Pure functions only: no window and
// no layout. To register: a `[[test]] name = "highlights"` entry in `Cargo.toml` and
// `pub mod highlights;` in `lib.rs` (the integrator, see the lane notes).

mod highlights {
    use std::ops::Range;

    use app_units::Au;
    use layout::flow::inline::text_transform::OffsetMap;
    use layout::highlights::{
        ACTIVE_FIND_MATCH_BACKGROUND, FIND_MATCH_BACKGROUND, GRAMMAR_ERROR_COLOR, GlyphSpan,
        PlacedSegment, SPELLING_ERROR_COLOR, TEXT_FRAGMENT_BACKGROUND, background_color,
        decoration_color, decoration_rects, highlights_in_window, map_highlights, place_segments,
        segments_for_fragment, snapped_thickness,
    };
    use layout_api::{
        Decoration, DecorationSet, HighlightKind, Segment, TextHighlight, resolve_segments,
    };
    use servo_base::text::{RangeAny, Utf16CodeUnits, Utf32CodeUnits};

    fn u(offset: u32) -> Utf32CodeUnits {
        Utf32CodeUnits(offset)
    }

    fn run(start: u32, end: u32) -> Range<Utf32CodeUnits> {
        u(start)..u(end)
    }

    fn highlight(kind: HighlightKind, start: u32, end: u32) -> TextHighlight {
        TextHighlight {
            kind,
            range: RangeAny::new(Some(u(start)), Some(u(end))),
        }
    }

    fn identity(range: RangeAny<Utf32CodeUnits>) -> Range<Utf32CodeUnits> {
        range.start().unwrap_or(u(0))..range.end().unwrap_or(u(u32::MAX))
    }

    fn px(value: i32) -> Au {
        Au::from_px(value)
    }

    /// One glyph per character, each 10 px wide.
    fn simple_glyphs(count: u32) -> Vec<GlyphSpan> {
        (0..count)
            .map(|_| GlyphSpan {
                characters: u(1),
                advance: px(10),
            })
            .collect()
    }

    fn decorations(of: &[Decoration]) -> DecorationSet {
        let mut set = DecorationSet::EMPTY;
        for decoration in of {
            set.insert(*decoration);
        }
        set
    }

    #[test]
    fn three_overlapping_highlights_are_split_and_placed() {
        // "0123456789", a find match over [0, 8), the active match over [2, 6) and a spelling
        // error over [4, 12).
        let highlights = [
            highlight(HighlightKind::FindMatch, 0, 8),
            highlight(HighlightKind::Spelling, 4, 12),
            highlight(HighlightKind::ActiveFindMatch, 2, 6),
        ];
        let segments = segments_for_fragment(&highlights, identity, run(0, 10));
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
                    decorations: decorations(&[Decoration::Spelling]),
                },
                Segment {
                    range: run(6, 8),
                    background: Some(HighlightKind::FindMatch),
                    decorations: decorations(&[Decoration::Spelling]),
                },
                Segment {
                    range: run(8, 10),
                    background: None,
                    decorations: decorations(&[Decoration::Spelling]),
                },
            ]
        );

        let placed = place_segments(simple_glyphs(10), run(0, 10), &segments);
        let spans: Vec<(i32, i32)> = placed
            .iter()
            .map(|placed| (placed.x.to_px(), placed.width.to_px()))
            .collect();
        assert_eq!(spans, vec![(0, 20), (20, 20), (40, 20), (60, 20), (80, 20)]);
    }

    #[test]
    fn a_highlight_outside_the_run_paints_nothing() {
        let highlights = [
            highlight(HighlightKind::FindMatch, 20, 25),
            highlight(HighlightKind::Spelling, 0, 5),
        ];
        // The fragment shows characters 5 to 10 of the run: neither highlight reaches it.
        let segments = segments_for_fragment(&highlights, identity, run(5, 10));
        assert!(segments.is_empty(), "{segments:?}");
        assert!(place_segments(simple_glyphs(5), run(5, 10), &segments).is_empty());
    }

    #[test]
    fn a_highlight_spanning_two_text_nodes_is_clipped_in_each() {
        // A range from offset 3 of the first node (5 characters) to offset 4 of the second:
        // layout gets an open end for the first node and an open start for the second.
        let in_first_node = [TextHighlight {
            kind: HighlightKind::FindMatch,
            range: RangeAny::new(Some(u(3)), None),
        }];
        let in_second_node = [TextHighlight {
            kind: HighlightKind::FindMatch,
            range: RangeAny::from_start_to(u(4)),
        }];

        let first = resolve_segments(run(0, 5), &in_first_node);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].range, run(3, 5));
        let second = resolve_segments(run(0, 7), &in_second_node);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].range, run(0, 4));
    }

    #[test]
    fn a_highlight_spanning_two_fragments_of_one_run_is_clipped_in_each() {
        // One text run broken into two lines: characters 0 to 6 and 6 to 12.
        let highlights = [highlight(HighlightKind::TextFragment, 4, 9)];
        let first_line = segments_for_fragment(&highlights, identity, run(0, 6));
        let second_line = segments_for_fragment(&highlights, identity, run(6, 12));
        assert_eq!(first_line.len(), 1);
        assert_eq!(first_line[0].range, run(4, 6));
        assert_eq!(second_line.len(), 1);
        assert_eq!(second_line[0].range, run(6, 9));

        // The first line is placed from its own start, the second from its own.
        let first = place_segments(simple_glyphs(6), run(0, 6), &first_line);
        assert_eq!((first[0].x, first[0].width), (px(40), px(20)));
        let second = place_segments(simple_glyphs(6), run(6, 12), &second_line);
        assert_eq!((second[0].x, second[0].width), (px(0), px(30)));
    }

    #[test]
    fn empty_ranges_paint_nothing() {
        let highlights = [
            highlight(HighlightKind::FindMatch, 3, 3),
            highlight(HighlightKind::Composition, 7, 2),
            highlight(HighlightKind::Spelling, 5, 5),
        ];
        assert!(segments_for_fragment(&highlights, identity, run(0, 10)).is_empty());
        // Segments with no width on the glyphs (zero-advance glyphs) are dropped too.
        let segments = [Segment {
            range: run(0, 2),
            background: Some(HighlightKind::FindMatch),
            decorations: DecorationSet::EMPTY,
        }];
        let zero_width = [
            GlyphSpan {
                characters: u(1),
                advance: Au(0),
            },
            GlyphSpan {
                characters: u(1),
                advance: Au(0),
            },
        ];
        assert!(place_segments(zero_width, run(0, 2), &segments).is_empty());
    }

    #[test]
    fn astral_characters_count_as_one_character() {
        // The DOM counts UTF-16 units, layout counts characters.
        let text = "a\u{1F600}b\u{1F600}c";
        let to_utf32 = |units: u32| Utf16CodeUnits(units).to_utf32_code_units_in(text);

        // [1, 3) in UTF-16 is the first emoji, [1, 2) in characters.
        let first_emoji = TextHighlight {
            kind: HighlightKind::FindMatch,
            range: RangeAny::new(Some(to_utf32(1)), Some(to_utf32(3))),
        };
        assert_eq!(first_emoji.range.start(), Some(u(1)));
        assert_eq!(first_emoji.range.end(), Some(u(2)));
        let segments = segments_for_fragment(&[first_emoji], identity, run(0, 5));
        let placed = place_segments(simple_glyphs(5), run(0, 5), &segments);
        assert_eq!((placed[0].x, placed[0].width), (px(10), px(10)));

        // A UTF-16 offset inside the pair rounds to before the pair: [1, 2) is empty.
        let half_emoji = TextHighlight {
            kind: HighlightKind::FindMatch,
            range: RangeAny::new(Some(to_utf32(1)), Some(to_utf32(2))),
        };
        assert_eq!(half_emoji.range.end(), Some(u(1)));
        assert!(segments_for_fragment(&[half_emoji], identity, run(0, 5)).is_empty());

        // The second emoji, UTF-16 [4, 6), is characters [3, 4).
        let second_emoji = TextHighlight {
            kind: HighlightKind::Grammar,
            range: RangeAny::new(Some(to_utf32(4)), Some(to_utf32(6))),
        };
        let segments = segments_for_fragment(&[second_emoji], identity, run(0, 5));
        assert_eq!(segments[0].range, run(3, 4));
    }

    #[test]
    fn background_order_and_colours() {
        let kinds = [
            HighlightKind::TextFragment,
            HighlightKind::ActiveFindMatch,
            HighlightKind::FindMatch,
        ];
        let highlights: Vec<TextHighlight> =
            kinds.iter().map(|kind| highlight(*kind, 0, 4)).collect();
        let segments = segments_for_fragment(&highlights, identity, run(0, 4));
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].background, Some(HighlightKind::ActiveFindMatch));

        // FindMatch < TextFragment < ActiveFindMatch, in whatever order they come.
        let find_and_fragment = [
            highlight(HighlightKind::TextFragment, 0, 4),
            highlight(HighlightKind::FindMatch, 0, 4),
        ];
        let segments = segments_for_fragment(&find_and_fragment, identity, run(0, 4));
        assert_eq!(segments[0].background, Some(HighlightKind::TextFragment));

        assert_eq!(
            background_color(HighlightKind::FindMatch),
            Some(FIND_MATCH_BACKGROUND)
        );
        assert_eq!(
            background_color(HighlightKind::ActiveFindMatch),
            Some(ACTIVE_FIND_MATCH_BACKGROUND)
        );
        assert_eq!(
            background_color(HighlightKind::TextFragment),
            Some(TEXT_FRAGMENT_BACKGROUND)
        );
        assert_eq!(background_color(HighlightKind::Custom(1)), None);
        assert_eq!(background_color(HighlightKind::Composition), None);
        assert_eq!(
            (
                FIND_MATCH_BACKGROUND.red,
                FIND_MATCH_BACKGROUND.green,
                FIND_MATCH_BACKGROUND.blue
            ),
            (0xFF, 0xFF, 0x00)
        );
        assert_eq!(
            (
                ACTIVE_FIND_MATCH_BACKGROUND.red,
                ACTIVE_FIND_MATCH_BACKGROUND.green,
                ACTIVE_FIND_MATCH_BACKGROUND.blue
            ),
            (0xFF, 0x96, 0x32)
        );
        assert_eq!(
            (
                TEXT_FRAGMENT_BACKGROUND.red,
                TEXT_FRAGMENT_BACKGROUND.green,
                TEXT_FRAGMENT_BACKGROUND.blue
            ),
            (0xFF, 0xE5, 0x00)
        );
    }

    #[test]
    fn decorations_stack_in_painting_order() {
        let highlights = [
            highlight(HighlightKind::Grammar, 0, 4),
            highlight(HighlightKind::Spelling, 0, 4),
            highlight(HighlightKind::CompositionSelected, 0, 4),
            highlight(HighlightKind::Composition, 0, 4),
        ];
        let segments = segments_for_fragment(&highlights, identity, run(0, 4));
        let placed = place_segments(simple_glyphs(4), run(0, 4), &segments);
        assert_eq!(placed.len(), 1);

        let rects = decoration_rects(&placed[0], px(12), 0.8, 1.0);
        let order: Vec<Decoration> = rects.iter().map(|rect| rect.decoration).collect();
        assert_eq!(
            order,
            vec![
                Decoration::Composition,
                Decoration::CompositionSelected,
                Decoration::Spelling,
                Decoration::Grammar,
            ]
        );
        for rect in &rects {
            assert_eq!((rect.x, rect.y, rect.width), (px(0), px(12), px(40)));
        }
        let heights: Vec<Au> = rects.iter().map(|rect| rect.height).collect();
        assert_eq!(heights, vec![px(1), px(2), px(1), px(1)]);

        assert_eq!(decoration_color(Decoration::Composition), None);
        assert_eq!(decoration_color(Decoration::CompositionSelected), None);
        assert_eq!(
            decoration_color(Decoration::Spelling),
            Some(SPELLING_ERROR_COLOR)
        );
        assert_eq!(
            decoration_color(Decoration::Grammar),
            Some(GRAMMAR_ERROR_COLOR)
        );
    }

    #[test]
    fn decoration_thickness_snaps_to_device_pixels() {
        // 1 and 2 CSS px at 1x and 2x are whole device pixels.
        assert_eq!(snapped_thickness(1.0, 1.0), px(1));
        assert_eq!(snapped_thickness(2.0, 1.0), px(2));
        assert_eq!(snapped_thickness(1.0, 2.0), px(1));
        assert_eq!(snapped_thickness(2.0, 2.0), px(2));
        // At 1.5x one CSS px is 1.5 device pixels: round down to one device pixel, 2/3 CSS px.
        assert_eq!(snapped_thickness(1.0, 1.5), Au::from_f32_px(1.0 / 1.5));
        assert_eq!(snapped_thickness(2.0, 1.5), px(2));
        // Never below one device pixel, and zero stays zero.
        assert_eq!(snapped_thickness(0.3, 1.0), px(1));
        assert_eq!(snapped_thickness(0.3, 2.0), Au::from_f32_px(0.5));
        assert_eq!(snapped_thickness(0.0, 2.0), Au(0));
        // A nonsense device pixel ratio does not divide by zero.
        assert_eq!(snapped_thickness(1.0, 0.0), px(1));
    }

    #[test]
    fn underline_rects_on_text_that_collapsed_white_space() {
        // DOM text "  ab   cd" laid out as "ab cd": two leading spaces collapse away, three
        // spaces in the middle become one.
        let mut offset_map = OffsetMap::default();
        offset_map.push_range(u(2), u(0));
        offset_map.push_range(u(2), u(2));
        offset_map.push_range(u(3), u(1));
        offset_map.push_range(u(2), u(2));
        assert_eq!(offset_map.map(u(0)), u(0));
        assert_eq!(offset_map.map(u(4)), u(2));
        assert_eq!(offset_map.map(u(9)), u(5));

        let map = |range: RangeAny<Utf32CodeUnits>| {
            let start = range.start().map_or(u(0), |start| offset_map.map(start));
            let end = range.end().map_or(u(5), |end| offset_map.map(end));
            start..end
        };

        // An IME composition over the DOM text "cd", DOM [7, 9), is [3, 5) in "ab cd".
        let composition = highlight(HighlightKind::Composition, 7, 9);
        let segments = segments_for_fragment(&[composition], &map, run(0, 5));
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].range, run(3, 5));
        let placed = place_segments(simple_glyphs(5), run(0, 5), &segments);
        assert_eq!(placed.len(), 1);
        let rects = decoration_rects(&placed[0], px(11), 0.8, 2.0);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].decoration, Decoration::Composition);
        assert_eq!(
            (rects[0].x, rects[0].y, rects[0].width, rects[0].height),
            (px(30), px(11), px(20), px(1))
        );

        // A highlight over the collapsed spaces only is empty after mapping and paints nothing.
        let leading_spaces = highlight(HighlightKind::FindMatch, 0, 2);
        assert!(segments_for_fragment(&[leading_spaces], &map, run(0, 5)).is_empty());
        let dropped = map_highlights(&[leading_spaces, composition], &map);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].kind, HighlightKind::Composition);

        // A find match over "ab" with its leading spaces, DOM [0, 4), is the glyphs "ab".
        let with_spaces = highlight(HighlightKind::FindMatch, 0, 4);
        let segments = segments_for_fragment(&[with_spaces], &map, run(0, 5));
        let placed = place_segments(simple_glyphs(5), run(0, 5), &segments);
        assert_eq!((placed[0].x, placed[0].width), (px(0), px(20)));
    }

    #[test]
    fn underline_rects_on_text_that_grew() {
        // `text-transform: uppercase` turns "aßb" into "ASSB": one character becomes two.
        let mut offset_map = OffsetMap::default();
        offset_map.push_range(u(1), u(1));
        offset_map.push_range(u(1), u(2));
        offset_map.push_range(u(1), u(1));
        let map = |range: RangeAny<Utf32CodeUnits>| {
            let start = range.start().map_or(u(0), |start| offset_map.map(start));
            let end = range.end().map_or(u(4), |end| offset_map.map(end));
            start..end
        };
        // The DOM "ß", [1, 2), is "SS", [1, 3).
        let segments =
            segments_for_fragment(&[highlight(HighlightKind::Spelling, 1, 2)], &map, run(0, 4));
        assert_eq!(segments[0].range, run(1, 3));
        let placed = place_segments(simple_glyphs(4), run(0, 4), &segments);
        assert_eq!((placed[0].x, placed[0].width), (px(10), px(20)));
    }

    #[test]
    fn segments_widen_to_whole_clusters() {
        // "fi" shaped as one ligature glyph for two characters, then "x".
        let glyphs = [
            GlyphSpan {
                characters: u(2),
                advance: px(14),
            },
            GlyphSpan {
                characters: u(1),
                advance: px(8),
            },
        ];
        // A highlight over just the "i" covers the whole ligature.
        let segments = [Segment {
            range: run(1, 2),
            background: Some(HighlightKind::FindMatch),
            decorations: DecorationSet::EMPTY,
        }];
        let placed = place_segments(glyphs, run(0, 3), &segments);
        assert_eq!((placed[0].x, placed[0].width), (px(0), px(14)));

        // A highlight from the "i" to the "x" starts at the ligature's start.
        let segments = [Segment {
            range: run(1, 3),
            background: Some(HighlightKind::FindMatch),
            decorations: DecorationSet::EMPTY,
        }];
        let placed = place_segments(glyphs, run(0, 3), &segments);
        assert_eq!((placed[0].x, placed[0].width), (px(0), px(22)));
    }

    #[test]
    fn glyphs_without_characters_belong_to_the_previous_cluster() {
        // "e" followed by a combining accent that was shaped as a glyph of its own with no
        // character count: the accent's advance belongs to the cluster of the "e".
        let glyphs = [
            GlyphSpan {
                characters: u(1),
                advance: px(10),
            },
            GlyphSpan {
                characters: u(0),
                advance: px(2),
            },
            GlyphSpan {
                characters: u(1),
                advance: px(10),
            },
        ];
        let over_e = [Segment {
            range: run(0, 1),
            background: Some(HighlightKind::FindMatch),
            decorations: DecorationSet::EMPTY,
        }];
        let placed = place_segments(glyphs, run(0, 2), &over_e);
        assert_eq!((placed[0].x, placed[0].width), (px(0), px(12)));

        let over_second = [Segment {
            range: run(1, 2),
            background: Some(HighlightKind::FindMatch),
            decorations: DecorationSet::EMPTY,
        }];
        let placed = place_segments(glyphs, run(0, 2), &over_second);
        assert_eq!((placed[0].x, placed[0].width), (px(12), px(10)));
    }

    #[test]
    fn segments_past_the_glyphs_are_clamped() {
        let segments = [Segment {
            range: run(2, 9),
            background: Some(HighlightKind::FindMatch),
            decorations: DecorationSet::EMPTY,
        }];
        let placed = place_segments(simple_glyphs(4), run(0, 4), &segments);
        assert_eq!(
            placed,
            vec![PlacedSegment {
                x: px(20),
                width: px(20),
                background: Some(HighlightKind::FindMatch),
                decorations: DecorationSet::EMPTY,
            }]
        );
    }

    #[test]
    fn first_letter_pieces_get_their_own_shifted_highlights() {
        // The text `"Hi" there` splits into the first letter `"Hi"` (characters 0 to 4) and the
        // rest. A find match over characters 2 to 8 reaches into both pieces.
        let highlights = [highlight(HighlightKind::FindMatch, 2, 8)];
        let first_letter = highlights_in_window(&highlights, RangeAny::new(Some(u(0)), Some(u(4))));
        assert_eq!(first_letter.len(), 1);
        assert_eq!(first_letter[0].range.start(), Some(u(2)));
        assert_eq!(first_letter[0].range.end(), Some(u(4)));

        let rest = highlights_in_window(&highlights, RangeAny::new(Some(u(4)), None));
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].range.start(), Some(u(0)));
        assert_eq!(rest[0].range.end(), Some(u(4)));

        // A highlight that stays in the first piece does not reach the rest.
        let early = [highlight(HighlightKind::Spelling, 0, 3)];
        assert!(highlights_in_window(&early, RangeAny::new(Some(u(4)), None)).is_empty());
        // A highlight with an open end covers the rest from its start.
        let open = [TextHighlight {
            kind: HighlightKind::Grammar,
            range: RangeAny::new(Some(u(1)), None),
        }];
        let rest = highlights_in_window(&open, RangeAny::new(Some(u(4)), None));
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].range.start(), Some(u(0)));
        assert_eq!(rest[0].range.end(), None);
    }

    #[test]
    fn no_highlights_cost_nothing() {
        assert!(segments_for_fragment(&[], identity, run(0, 10)).is_empty());
        assert!(map_highlights(&[], identity).is_empty());
        assert!(place_segments(simple_glyphs(3), run(0, 3), &[]).is_empty());
    }
}
