/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: tests of the pure part of `text-overflow` (lane txt-1, item ED7).
//!
//! `ellipsis.rs` has nothing in it but `app_units`, so this test includes the file itself with `#[path]`
//! instead of reaching it through the crate, which keeps it private to the crate. The same file is
//! compiled and tested in a scratch crate when no engine can be built.

#[allow(dead_code)]
#[path = "../ellipsis.rs"]
mod ellipsis;

mod ellipsis_tests {
    use app_units::Au;

    use super::ellipsis::{
        Cluster, Direction, GlyphMetrics, Side, cut_point, end_side, group_clusters,
        group_clusters_in_text, keep_within, kept_totals,
    };
    use unicode_segmentation::UnicodeSegmentation;

    fn px(values: &[i32]) -> Vec<Au> {
        values.iter().map(|value| Au::from_px(*value)).collect()
    }

    fn glyph(advance_px: i32, character_count: u32) -> GlyphMetrics {
        GlyphMetrics {
            advance: Au::from_px(advance_px),
            character_count,
        }
    }

    fn advances(clusters: &[Cluster]) -> Vec<Au> {
        clusters.iter().map(|cluster| cluster.advance).collect()
    }

    #[test]
    fn a_line_that_fits_is_untouched() {
        let line = px(&[10, 10, 10]);
        // Exactly the room, and more than the room.
        assert_eq!(
            cut_point(&line, Au::from_px(30), Au::from_px(10), Direction::Ltr),
            None
        );
        assert_eq!(
            cut_point(&line, Au::from_px(100), Au::from_px(10), Direction::Ltr),
            None
        );
        assert_eq!(
            cut_point(&line, Au::from_px(30), Au::from_px(10), Direction::Rtl),
            None
        );
        // A marker wider than the room does not matter when nothing is cut.
        assert_eq!(
            cut_point(&line, Au::from_px(30), Au::from_px(500), Direction::Ltr),
            None
        );
        // Nothing to cut in an empty line, or a line of no width.
        assert_eq!(
            cut_point(&[], Au::from_px(0), Au::from_px(10), Direction::Ltr),
            None
        );
        assert_eq!(
            cut_point(
                &px(&[0, 0]),
                Au::from_px(0),
                Au::from_px(10),
                Direction::Ltr
            ),
            None
        );
    }

    #[test]
    fn one_glyph_too_wide_loses_two_because_the_marker_needs_room() {
        // Three glyphs of 10 in 25: the third does not fit, and the marker (10) does not fit behind
        // the second either, so only the first is kept.
        let line = px(&[10, 10, 10]);
        assert_eq!(
            cut_point(&line, Au::from_px(25), Au::from_px(10), Direction::Ltr),
            Some(1)
        );
        // With a narrow marker the second glyph stays: the budget is 25 - 4 = 21 and two glyphs are 20.
        assert_eq!(
            cut_point(&line, Au::from_px(25), Au::from_px(4), Direction::Ltr),
            Some(2)
        );
        // One pixel over: 31 in 30. The marker is 10, so 20 is the budget.
        let line = px(&[10, 10, 10, 1]);
        assert_eq!(
            cut_point(&line, Au::from_px(30), Au::from_px(10), Direction::Ltr),
            Some(2)
        );
    }

    #[test]
    fn the_budget_is_inclusive() {
        let line = px(&[10, 10, 10, 10]);
        // 40 in 35 with a marker of 5: the budget is 30, and three glyphs are exactly 30.
        assert_eq!(
            cut_point(&line, Au::from_px(35), Au::from_px(5), Direction::Ltr),
            Some(3)
        );
        assert_eq!(keep_within(&line, Au::from_px(30), Direction::Ltr), 3);
        assert_eq!(keep_within(&line, Au::from_px(29), Direction::Ltr), 2);
        assert_eq!(keep_within(&line, Au::from_px(40), Direction::Ltr), 4);
    }

    #[test]
    fn a_ligature_is_never_split() {
        // "a", the ligature "fi" (one glyph, two characters), "b".
        let glyphs = [glyph(10, 1), glyph(20, 2), glyph(10, 1)];
        let clusters = group_clusters(&glyphs, false);
        assert_eq!(advances(&clusters), px(&[10, 20, 10]));
        // 40 in 35 with a marker of 10: the budget is 25. "a" fits, "a" and "fi" (30) do not.
        let cut = cut_point(
            &advances(&clusters),
            Au::from_px(35),
            Au::from_px(10),
            Direction::Ltr,
        );
        assert_eq!(cut, Some(1));
        assert_eq!(
            kept_totals(&clusters, 1, Direction::Ltr),
            (Au::from_px(10), 1)
        );
        // The same cut with a budget that would reach into the middle of the ligature if it could.
        assert_eq!(
            keep_within(&advances(&clusters), Au::from_px(20), Direction::Ltr),
            1
        );
    }

    #[test]
    fn a_combining_mark_stays_with_its_base() {
        // "e", U+0301 (no advance), "x", "y".
        let glyphs = [glyph(10, 1), glyph(0, 1), glyph(10, 1), glyph(10, 1)];
        let clusters = group_clusters(&glyphs, false);
        assert_eq!(advances(&clusters), px(&[10, 10, 10]));
        assert_eq!(
            clusters
                .iter()
                .map(|cluster| cluster.glyph_count)
                .collect::<Vec<_>>(),
            vec![2, 1, 1]
        );
        // 30 in 25, marker 5: the budget is 20, so "e" with its mark and "x".
        let cut = cut_point(
            &advances(&clusters),
            Au::from_px(25),
            Au::from_px(5),
            Direction::Ltr,
        );
        assert_eq!(cut, Some(2));
        assert_eq!(
            kept_totals(&clusters, 2, Direction::Ltr),
            (Au::from_px(20), 3)
        );
        // A budget for one glyph keeps "e" and its mark, not "e" alone and not nothing.
        let cut = keep_within(&advances(&clusters), Au::from_px(10), Direction::Ltr);
        assert_eq!(cut, 1);
        assert_eq!(
            kept_totals(&clusters, cut, Direction::Ltr),
            (Au::from_px(10), 2)
        );
        // A budget that cannot hold the base keeps neither.
        assert_eq!(
            keep_within(&advances(&clusters), Au::from_px(9), Direction::Ltr),
            0
        );
    }

    #[test]
    fn the_second_glyph_of_a_character_stays_with_the_first() {
        // A character drawn with two glyphs: the second stands for no character but has an advance.
        let glyphs = [glyph(10, 1), glyph(10, 0), glyph(10, 1)];
        let clusters = group_clusters(&glyphs, false);
        assert_eq!(advances(&clusters), px(&[20, 10]));
        assert_eq!(
            keep_within(&advances(&clusters), Au::from_px(15), Direction::Ltr),
            0
        );
        assert_eq!(
            keep_within(&advances(&clusters), Au::from_px(20), Direction::Ltr),
            1
        );
    }

    #[test]
    fn a_zero_width_glyph_first_in_a_run_starts_a_cluster() {
        let glyphs = [glyph(0, 1), glyph(10, 1)];
        let clusters = group_clusters(&glyphs, false);
        assert_eq!(advances(&clusters), px(&[0, 10]));
        assert!(group_clusters(&[], false).is_empty());
        assert!(group_clusters(&[], true).is_empty());
    }

    #[test]
    fn a_right_to_left_block_loses_its_left_side() {
        // Painted left to right, four glyphs of 10 in 30, marker 10: the budget is 20, so the two
        // on the right stay (the start of the line is on the right) and the two on the left go.
        let line = px(&[10, 10, 10, 10]);
        let cut = cut_point(&line, Au::from_px(30), Au::from_px(10), Direction::Rtl);
        assert_eq!(cut, Some(2));
        let clusters = vec![
            Cluster {
                advance: Au::from_px(10),
                glyph_count: 1
            };
            4
        ];
        assert_eq!(
            kept_totals(&clusters, 2, Direction::Rtl),
            (Au::from_px(20), 2)
        );
        // Uneven advances: 70 in 50 with a marker of 10 is a budget of 40, which holds the three
        // glyphs of 10 on the right (30) but not the 40 on the left as well.
        let line = px(&[40, 10, 10, 10]);
        assert_eq!(
            cut_point(&line, Au::from_px(50), Au::from_px(10), Direction::Rtl),
            Some(1)
        );
        assert_eq!(keep_within(&line, Au::from_px(30), Direction::Rtl), 1);
        assert_eq!(keep_within(&line, Au::from_px(70), Direction::Rtl), 0);
    }

    #[test]
    fn a_right_to_left_run_keeps_a_mark_with_its_base() {
        // Painted left to right: the mark U+0651 comes before its base letter, because the text
        // runs from right to left, then two more letters.
        let glyphs = [glyph(0, 1), glyph(10, 1), glyph(10, 1), glyph(10, 1)];
        let clusters = group_clusters(&glyphs, true);
        assert_eq!(advances(&clusters), px(&[10, 10, 10]));
        assert_eq!(
            clusters
                .iter()
                .map(|cluster| cluster.glyph_count)
                .collect::<Vec<_>>(),
            vec![2, 1, 1]
        );
        // Cutting from the left of a right-to-left block: room for two clusters keeps the two on
        // the right (a cut at 1), room for three keeps all of them (a cut at 0), and the base is
        // never kept without its mark.
        assert_eq!(
            keep_within(&advances(&clusters), Au::from_px(20), Direction::Rtl),
            1
        );
        assert_eq!(
            kept_totals(&clusters, 1, Direction::Rtl),
            (Au::from_px(20), 2)
        );
        assert_eq!(
            keep_within(&advances(&clusters), Au::from_px(30), Direction::Rtl),
            0
        );
        // Room for one cluster keeps only the rightmost; the mark and its base go together.
        assert_eq!(
            keep_within(&advances(&clusters), Au::from_px(10), Direction::Rtl),
            2
        );
    }

    #[test]
    fn a_marker_wider_than_the_line_keeps_nothing() {
        let line = px(&[10, 10]);
        // 20 in 5, marker 8: the marker cannot fit even in front of nothing.
        assert_eq!(
            cut_point(&line, Au::from_px(5), Au::from_px(8), Direction::Ltr),
            Some(0)
        );
        // For a right-to-left block "nothing kept" is a cut at the end.
        assert_eq!(
            cut_point(&line, Au::from_px(5), Au::from_px(8), Direction::Rtl),
            Some(2)
        );
        // A marker exactly as wide as the room leaves a budget of zero: nothing is kept.
        assert_eq!(
            cut_point(&line, Au::from_px(8), Au::from_px(8), Direction::Ltr),
            Some(0)
        );
    }

    #[test]
    fn a_line_of_no_width_keeps_nothing() {
        assert_eq!(
            cut_point(&px(&[10]), Au::from_px(0), Au::from_px(10), Direction::Ltr),
            Some(0)
        );
        assert_eq!(
            cut_point(&px(&[10]), Au::from_px(0), Au::from_px(0), Direction::Ltr),
            Some(0)
        );
        assert_eq!(
            cut_point(&px(&[10]), Au::from_px(0), Au::from_px(0), Direction::Rtl),
            Some(1)
        );
        // A negative room (a float took more than the line) behaves like none.
        assert_eq!(
            cut_point(&px(&[10]), Au::from_px(-4), Au::from_px(2), Direction::Ltr),
            Some(0)
        );
    }

    #[test]
    fn a_string_wider_than_the_available_room_keeps_nothing() {
        // A string value such as "[...]" is one marker of its own width, however many characters.
        let line = px(&[10, 10, 10, 10]);
        assert_eq!(
            cut_point(&line, Au::from_px(30), Au::from_px(50), Direction::Ltr),
            Some(0)
        );
        // A short string behaves like a short ellipsis.
        assert_eq!(
            cut_point(&line, Au::from_px(30), Au::from_px(5), Direction::Ltr),
            Some(2)
        );
    }

    #[test]
    fn kept_totals_clamps_the_cut_and_sums_glyphs() {
        let clusters = [
            Cluster {
                advance: Au::from_px(10),
                glyph_count: 2,
            },
            Cluster {
                advance: Au::from_px(5),
                glyph_count: 1,
            },
        ];
        assert_eq!(
            kept_totals(&clusters, 0, Direction::Ltr),
            (Au::from_px(0), 0)
        );
        assert_eq!(
            kept_totals(&clusters, 99, Direction::Ltr),
            (Au::from_px(15), 3)
        );
        assert_eq!(
            kept_totals(&clusters, 99, Direction::Rtl),
            (Au::from_px(0), 0)
        );
        assert_eq!(
            kept_totals(&clusters, 1, Direction::Rtl),
            (Au::from_px(5), 1)
        );
    }

    #[test]
    fn the_end_side_follows_gecko() {
        // One value: always for the end.
        assert_eq!(end_side(true, false), Side::Second);
        assert_eq!(end_side(true, true), Side::Second);
        // Two values are physical: left then right.
        assert_eq!(end_side(false, false), Side::Second);
        assert_eq!(end_side(false, true), Side::First);
    }

    // ---- grapheme clusters: a font that does not ligate leaves one glyph per character, each with
    // an advance (even the zero width joiner, in the worst case), and the cut must still fall
    // between user-perceived characters.

    /// One glyph per character, each as wide as `advance_px`.
    fn glyph_per_char(text: &str, advance_px: i32) -> Vec<GlyphMetrics> {
        text.chars().map(|_| glyph(advance_px, 1)).collect()
    }

    fn cluster_advances_px(text: &str) -> Vec<i32> {
        let glyphs = glyph_per_char(text, 10);
        group_clusters_in_text(&glyphs, false, Some(text))
            .iter()
            .map(|cluster| cluster.advance.to_px())
            .collect()
    }

    #[test]
    fn an_emoji_zwj_sequence_is_one_cluster() {
        // a, man ZWJ woman ZWJ girl (5 characters), b
        assert_eq!(
            cluster_advances_px("a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}b"),
            vec![10, 50, 10]
        );
    }

    #[test]
    fn without_the_text_the_glyphs_alone_would_cut_the_sequence() {
        // The reason for reading the text: with glyphs that all advance, the old rule makes one
        // cluster per character.
        let text = "a\u{1F468}\u{200D}\u{1F469}b";
        let glyphs = glyph_per_char(text, 10);
        assert_eq!(group_clusters(&glyphs, false).len(), 5);
        assert_eq!(group_clusters_in_text(&glyphs, false, Some(text)).len(), 3);
    }

    #[test]
    fn a_flag_of_two_regional_indicators_is_one_cluster() {
        assert_eq!(
            cluster_advances_px("\u{1F1FA}\u{1F1F8}x\u{1F1EB}\u{1F1F7}"),
            vec![20, 10, 20]
        );
    }

    #[test]
    fn an_emoji_with_a_skin_tone_modifier_is_one_cluster() {
        assert_eq!(cluster_advances_px("\u{1F44D}\u{1F3FD}!"), vec![20, 10]);
    }

    #[test]
    fn an_indic_conjunct_is_one_cluster() {
        // Devanagari ksha: KA, VIRAMA, SSA (one grapheme since Unicode 15.1), then KA alone.
        assert_eq!(
            cluster_advances_px("\u{0915}\u{094D}\u{0937}\u{0915}"),
            vec![30, 10]
        );
    }

    #[test]
    fn a_decomposed_hangul_syllable_and_a_crlf_and_a_combining_mark_are_one_cluster() {
        // Hangul jamo HIEUH, A, NIEUN; CR LF; e and a combining acute accent.
        assert_eq!(
            cluster_advances_px("\u{1112}\u{1161}\u{11AB}\r\ne\u{0301}"),
            vec![30, 20, 20]
        );
    }

    #[test]
    fn a_ligature_over_several_graphemes_stays_whole() {
        // "fi" is one glyph for two characters, then x.
        let glyphs = [glyph(12, 2), glyph(10, 1)];
        let clusters = group_clusters_in_text(&glyphs, false, Some("fix"));
        assert_eq!(advances(&clusters), px(&[12, 10]));
    }

    #[test]
    fn text_that_does_not_match_the_glyphs_is_ignored() {
        // Three characters of text for two glyphs of one character each: the glyph rule applies.
        let glyphs = [glyph(10, 1), glyph(10, 1)];
        let clusters = group_clusters_in_text(&glyphs, false, Some("abc"));
        assert_eq!(advances(&clusters), px(&[10, 10]));
        assert_eq!(group_clusters_in_text(&glyphs, false, None), clusters);
    }

    #[test]
    fn a_right_to_left_run_groups_in_reading_order() {
        // Text in reading order: aleph, a combining mark (two characters, one grapheme), bet.
        // Painted right to left, the glyphs are bet, the mark, aleph.
        let text = "\u{05D0}\u{05B0}\u{05D1}";
        let glyphs = glyph_per_char(text, 10);
        let clusters = group_clusters_in_text(&glyphs, true, Some(text));
        assert_eq!(advances(&clusters), px(&[10, 20]));
    }

    #[test]
    fn no_cut_lands_inside_a_grapheme_cluster() {
        let samples = [
            "ab\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}cd",
            "\u{1F1FA}\u{1F1F8}\u{1F1EB}\u{1F1F7}ab",
            "x\u{1F44D}\u{1F3FD}y",
            "\u{0915}\u{094D}\u{0937}\u{0915}\u{094D}\u{0937}",
            "\u{1112}\u{1161}\u{11AB}\u{1100}\u{1161}",
            "e\u{0301}e\u{0301}\u{0301}z",
        ];
        for text in samples {
            // Where the graphemes end, counted in characters (one glyph per character).
            let mut boundaries = vec![0usize];
            for grapheme in text.graphemes(true) {
                let last = *boundaries.last().unwrap();
                boundaries.push(last + grapheme.chars().count());
            }
            let glyphs = glyph_per_char(text, 10);
            let clusters = group_clusters_in_text(&glyphs, false, Some(text));
            let line: Vec<Au> = clusters.iter().map(|cluster| cluster.advance).collect();
            for budget_px in 0..=(10 * glyphs.len() as i32) {
                let cut = keep_within(&line, Au::from_px(budget_px), Direction::Ltr);
                let (_, kept_glyphs) = kept_totals(&clusters, cut, Direction::Ltr);
                assert!(
                    boundaries.contains(&kept_glyphs),
                    "{text:?}: a budget of {budget_px} keeps {kept_glyphs} characters"
                );
            }
        }
    }
}
