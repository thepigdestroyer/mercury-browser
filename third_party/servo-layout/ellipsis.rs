/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: the pure part of `text-overflow` (lane txt-1, item ED7).
//!
//! This file has no Stylo, Servo or font types in its signatures, only [`Au`], so that the arithmetic
//! can be tested without an engine (`tests/ellipsis.rs` includes this file with `#[path]`).
//!
//! **What it decides.** Given the advances of the glyph clusters of a line, the room the line has and the
//! advance of the overflow marker (the ellipsis, or the author's string), where does the line end?
//!
//! - A line that fits is left alone.
//! - Otherwise whole clusters are removed from the end of the line until the marker fits behind what is left.
//!   A cluster (a base letter with its combining marks, a ligature, a glyph that stands for several
//!   characters) is never split.
//! - If the marker does not fit even in front of nothing, nothing is kept and the marker is drawn clipped by
//!   the box. The marker is never made smaller.
//! - "End" is the end of the line in the writing direction of the block, which is a physical side: the right
//!   for a left-to-right block, the left for a right-to-left block. Content is hidden on that visual side
//!   whatever the direction of the text itself.
//!
//! **Where it comes from.** The rules follow CSS Overflow 4 and CSS UI 3 (`text-overflow`) and Gecko's
//! `layout/generic/TextOverflow.cpp` (MPL-2.0): the choice of the end side in [`end_side`] is a translation of
//! the constructor of `TextOverflow` there (`shouldToggleDirection`, `mIStart`, `mIEnd`), and the rule that
//! the marker is placed against the content that is left, which is also what the first draft of this
//! feature in Servo does, comes from `TextOverflow::ExamineLineFrames`. The layout half that uses this file
//! is a hand-port of servo/servo#42394 by RichardTjokroutomo onto Servo 0.8 (see `MERCURY.md`).

use app_units::Au;
use unicode_segmentation::UnicodeSegmentation;

/// Which physical end of a run of glyphs, laid out left to right, is the end of the line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    /// The block is left to right: the end of the line is the right side.
    Ltr,
    /// The block is right to left: the end of the line is the left side.
    Rtl,
}

/// Which of the two values of a `text-overflow` declaration is the one for the end of the line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    /// The first value (the left side when the declaration has two values).
    First,
    /// The second value (the right side when the declaration has two values, and the only one that
    /// counts when it has one).
    Second,
}

/// The value of `text-overflow` that applies to the end of the line.
///
/// With one value (`sides_are_logical`, Stylo's flag for it) the value is for the end whatever the
/// direction. With two values they are physical, left then right, so the end is the second for a
/// left-to-right block and the first for a right-to-left one. This is Gecko's `TextOverflow::TextOverflow`.
/// The value for the start side is not used: only the end of the line is elided.
pub fn end_side(sides_are_logical: bool, block_is_rtl: bool) -> Side {
    if sides_are_logical || !block_is_rtl {
        Side::Second
    } else {
        Side::First
    }
}

/// One glyph of a shaped text run, in the order the glyphs are painted (left to right).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlyphMetrics {
    /// The distance to the next glyph.
    pub advance: Au,
    /// The number of characters the glyph stands for. A glyph of a ligature stands for several; the
    /// extra glyph of a character that is drawn with two has none.
    pub character_count: u32,
}

/// Glyphs that must stay together: they are all kept or all hidden.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cluster {
    /// The sum of the advances of its glyphs.
    pub advance: Au,
    /// How many glyphs it has.
    pub glyph_count: usize,
}

/// Groups glyphs, given in painting order (left to right), into clusters, by what the shaper
/// reports about each glyph alone.
///
/// The text shaper reports one entry per character, so a base letter and its combining mark are two
/// glyphs. A glyph joins the cluster before it, in reading order, when it stands for no character (the
/// second glyph of a character drawn with two) or does not advance (a combining mark, a zero width
/// character); every other glyph starts a cluster. For a right-to-left run (`rtl_run`) the reading order is
/// the reverse of the painting order, so a mark that follows its base letter in the text is painted before
/// it, and the groups are made in reading order and then laid out again in painting order.
///
/// This is the fallback of [`group_clusters_in_text`], for a run whose text is not known: a font that does
/// not ligate leaves an emoji sequence, a flag or an Indic conjunct as several glyphs that advance.
// Mercury: `group_clusters_in_text` is what layout calls; this form is for the tests and as a fallback.
#[allow(dead_code)]
pub fn group_clusters(glyphs: &[GlyphMetrics], rtl_run: bool) -> Vec<Cluster> {
    group_clusters_in_text(glyphs, rtl_run, None)
}

/// Like [`group_clusters`], with the text of the run: glyphs also stay together when their characters
/// are in one extended grapheme cluster (UAX 29: an emoji ZWJ sequence, a flag of two regional
/// indicators, an emoji with a skin-tone modifier, a consonant, a virama and a consonant of an Indic
/// script, a decomposed Hangul syllable, CR LF). A glyph that stands for characters of several graphemes
/// (a ligature) keeps all of them together.
///
/// `text` is the text of the glyphs in reading order. If its number of characters is not the number of
/// characters the glyphs stand for (the two disagree, for a reason that is not known here), it is not
/// used and the result is that of [`group_clusters`].
pub fn group_clusters_in_text(
    glyphs: &[GlyphMetrics],
    rtl_run: bool,
    text: Option<&str>,
) -> Vec<Cluster> {
    let count = glyphs.len();
    let grapheme_of_char = text.map(grapheme_of_each_char).filter(|graphemes| {
        let stood_for: u64 = glyphs
            .iter()
            .map(|glyph| u64::from(glyph.character_count))
            .sum();
        graphemes.len() as u64 == stood_for
    });

    let mut cluster_of = vec![0usize; count];
    let mut started = 0usize;
    // The grapheme the last character of the current cluster is in.
    let mut last_grapheme = None::<usize>;
    let mut next_char = 0usize;
    for reading_index in 0..count {
        let index = if rtl_run {
            count - 1 - reading_index
        } else {
            reading_index
        };
        let glyph = glyphs[index];
        let characters = glyph.character_count as usize;
        let (first_grapheme, end_grapheme) = match &grapheme_of_char {
            Some(graphemes) if characters > 0 => (
                Some(graphemes[next_char]),
                Some(graphemes[next_char + characters - 1]),
            ),
            _ => (None, None),
        };
        next_char += characters;

        let joins_by_glyph = glyph.character_count == 0 || glyph.advance == Au(0);
        let joins_by_text = first_grapheme.is_some() && first_grapheme == last_grapheme;
        if started == 0 || !(joins_by_glyph || joins_by_text) {
            started += 1;
        }
        if end_grapheme.is_some() {
            last_grapheme = end_grapheme;
        }
        cluster_of[index] = started - 1;
    }

    let mut clusters: Vec<Cluster> = Vec::new();
    let mut previous = None;
    for (index, glyph) in glyphs.iter().enumerate() {
        if previous == Some(cluster_of[index]) {
            if let Some(last) = clusters.last_mut() {
                last.advance += glyph.advance;
                last.glyph_count += 1;
            }
        } else {
            clusters.push(Cluster {
                advance: glyph.advance,
                glyph_count: 1,
            });
            previous = Some(cluster_of[index]);
        }
    }
    clusters
}

/// For each character of `text`, the number of the extended grapheme cluster it is in.
fn grapheme_of_each_char(text: &str) -> Vec<usize> {
    let mut graphemes = Vec::with_capacity(text.len());
    for (grapheme_index, grapheme) in text.graphemes(true).enumerate() {
        graphemes.extend(grapheme.chars().map(|_| grapheme_index));
    }
    graphemes
}

fn sum(advances: &[Au]) -> i64 {
    advances.iter().map(|advance| i64::from(advance.0)).sum()
}

/// How many clusters at the end of the line have to go for the marker to fit, or `None` when nothing has
/// to go because the line fits.
///
/// `advances` are the advances of the clusters of the line in painting order, `available` is the room the
/// line has and `ellipsis` is the advance of the marker. The result is the index of the cut:
///
/// - [`Direction::Ltr`]: `advances[..cut]` are kept and `advances[cut..]` are elided;
/// - [`Direction::Rtl`]: `advances[..cut]` are elided and `advances[cut..]` are kept.
///
/// A line that fits (its total is at most `available`) is `None`, whatever the size of the marker. A line
/// that does not fit keeps the most clusters that fit in `available - ellipsis` and nothing when that is
/// negative.
// Layout calls `keep_within` for each text run of a line that is already known to overflow, so only
// the tests call this entry point, which is the one the lane's brief pins.
#[allow(dead_code)]
pub fn cut_point(
    advances: &[Au],
    available: Au,
    ellipsis: Au,
    direction: Direction,
) -> Option<usize> {
    if sum(advances) <= i64::from(available.0) {
        return None;
    }
    let budget = Au(available.0.saturating_sub(ellipsis.0));
    Some(keep_within(advances, budget, direction))
}

/// The index of the cut ([`cut_point`] says what it means) that keeps the most clusters whose advances add
/// up to at most `budget`, starting from the start of the line. A negative `budget` keeps nothing.
///
/// This is the part of [`cut_point`] for a line that is known not to fit; layout calls it for each text run
/// of such a line with what is left of the budget when the run starts.
pub fn keep_within(advances: &[Au], budget: Au, direction: Direction) -> usize {
    let budget = i64::from(budget.0);
    let mut kept_advance = 0i64;
    let mut kept = 0usize;
    match direction {
        Direction::Ltr => {
            for advance in advances {
                if kept_advance + i64::from(advance.0) > budget {
                    break;
                }
                kept_advance += i64::from(advance.0);
                kept += 1;
            }
            kept
        },
        Direction::Rtl => {
            for advance in advances.iter().rev() {
                if kept_advance + i64::from(advance.0) > budget {
                    break;
                }
                kept_advance += i64::from(advance.0);
                kept += 1;
            }
            advances.len() - kept
        },
    }
}

/// The advance and the number of glyphs that are kept when a run of `clusters` is cut at `cut` (the
/// result of [`cut_point`] or [`keep_within`] for the advances of these clusters).
pub fn kept_totals(clusters: &[Cluster], cut: usize, direction: Direction) -> (Au, usize) {
    let cut = cut.min(clusters.len());
    let kept = match direction {
        Direction::Ltr => &clusters[..cut],
        Direction::Rtl => &clusters[cut..],
    };
    let advance = kept
        .iter()
        .map(|cluster| i64::from(cluster.advance.0))
        .sum::<i64>();
    let advance = Au(advance.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32);
    (
        advance,
        kept.iter().map(|cluster| cluster.glyph_count).sum(),
    )
}
