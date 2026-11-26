// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Text runs for AccessKit: graphemes, words, the split of a line into runs, and the mapping between
//! DOM offsets, run positions and selections.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// AccessKit slices are `u8`, so a run holds at most this many characters.
pub const MAX_RUN_CHARACTERS: usize = 255;
/// Runs per text node, at most.
pub const MAX_RUNS_PER_TEXT_NODE: usize = 400;
/// Metric graphemes per update, at most.
pub const MAX_METRIC_GRAPHEMES_PER_UPDATE: usize = 20_000;

/// The inline direction of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
}

/// One line-box fragment of text, as layout hands it over.
pub struct LineInput<'a> {
    /// Rendered text of one line-box fragment.
    pub text: &'a str,
    /// One per Unicode scalar; may be empty or the wrong length (never a panic).
    pub advances: &'a [f32],
    /// UTF-32 DOM offset of each scalar, plus one past the end; may be empty.
    pub dom_offsets: &'a [usize],
    pub direction: Direction,
    pub ends_with_hard_break: bool,
}

/// A run of text ready for AccessKit.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSpec {
    pub text: String,
    /// UTF-8 bytes per grapheme; sum equals `text.len()`.
    pub character_lengths: Vec<u8>,
    /// Grapheme indices, sorted, each below 255.
    pub word_starts: Vec<u8>,
    /// x of each grapheme from the run's left edge (right edge for RTL).
    pub positions: Option<Vec<f32>>,
    pub widths: Option<Vec<f32>>,
    pub direction: Direction,
    pub dom_start: usize,
    pub dom_end: usize,
    /// DOM offset of each grapheme, plus one past the end; one more entry than graphemes.
    pub dom_offsets: Vec<usize>,
}

/// The runs of one line, and what was dropped to stay within the limits.
#[derive(Debug, PartialEq, Default)]
pub struct RunBuild {
    pub runs: Vec<RunSpec>,
    pub metrics_dropped: bool,
    pub advances_mismatched: bool,
}

/// A character position inside one run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextPositionSpec {
    pub run: usize,
    pub character_index: usize,
}

/// The anchor and focus of a selection, in run positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionSpec {
    pub anchor: TextPositionSpec,
    pub focus: TextPositionSpec,
}

/// Byte range of each extended grapheme cluster in `text`. A cluster longer than 255 bytes is cut at
/// character boundaries, because a character length is a `u8`.
pub fn graphemes(text: &str) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    for (start, cluster) in text.grapheme_indices(true) {
        let mut piece_start = start;
        for (offset, character) in cluster.char_indices() {
            let end = start + offset + character.len_utf8();
            if end - piece_start > MAX_CHARACTER_BYTES {
                spans.push(piece_start..start + offset);
                piece_start = start + offset;
            }
        }
        spans.push(piece_start..start + cluster.len());
    }
    spans
}

const MAX_CHARACTER_BYTES: usize = 255;

/// Grapheme indices where a UAX #29 word starts. A word's trailing space belongs to that word, so
/// it does not start a new one.
pub fn word_starts_in(text: &str, graphemes: &[Range<usize>]) -> Vec<usize> {
    let mut starts = Vec::new();
    for (byte_start, word) in text.split_word_bound_indices() {
        if word.chars().all(char::is_whitespace) {
            continue;
        }
        if let Some(index) = graphemes.iter().position(|span| span.start == byte_start) {
            starts.push(index);
        }
    }
    starts
}

/// Split a line into runs of at most 255 characters, cutting at a word boundary where one exists.
pub fn build_runs(line: &LineInput<'_>) -> RunBuild {
    let mut build = RunBuild::default();
    if line.text.is_empty() {
        return build;
    }
    let spans = graphemes(line.text);
    let advances_mismatched =
        !line.advances.is_empty() && line.advances.len() != line.text.chars().count();
    build.advances_mismatched = advances_mismatched;
    let positions_for_scalars = !advances_mismatched && !line.advances.is_empty();
    let word_starts = word_starts_in(line.text, &spans);

    let mut run_start = 0;
    while run_start < spans.len() {
        let mut run_end = (run_start + MAX_RUN_CHARACTERS).min(spans.len());
        if run_end < spans.len()
            && let Some(&boundary) = word_starts
                .iter()
                .rev()
                .find(|&&start| start > run_start && start < run_end)
        {
            run_end = boundary;
        }
        let byte_start = spans[run_start].start;
        let byte_end = spans[run_end - 1].end;
        let text = line.text[byte_start..byte_end].to_string();
        let character_lengths = spans[run_start..run_end]
            .iter()
            .map(|span| (span.end - span.start) as u8)
            .collect();
        let local_word_starts = word_starts
            .iter()
            .filter(|&&start| start >= run_start && start < run_end)
            .map(|&start| (start - run_start) as u8)
            .collect();
        let (positions, widths) = if positions_for_scalars {
            advances_for(line, &spans[run_start..run_end])
        } else {
            (None, None)
        };
        let dom_offsets: Vec<usize> = (run_start..=run_end)
            .map(|index| line.dom_offsets.get(index).copied().unwrap_or(index))
            .collect();
        let dom_start = dom_offsets[0];
        let dom_end = *dom_offsets.last().unwrap_or(&dom_start);
        build.runs.push(RunSpec {
            text,
            character_lengths,
            word_starts: local_word_starts,
            positions,
            widths,
            direction: line.direction,
            dom_start,
            dom_end,
            dom_offsets,
        });
        run_start = run_end;
    }
    build
}

/// Positions and widths for the graphemes of one run, from the per-scalar advances.
fn advances_for(
    line: &LineInput<'_>,
    spans: &[Range<usize>],
) -> (Option<Vec<f32>>, Option<Vec<f32>>) {
    let mut widths = Vec::with_capacity(spans.len());
    let mut scalar_index = line.text[..spans[0].start].chars().count();
    for span in spans {
        let scalars = line.text[span.clone()].chars().count();
        let width: f32 = line.advances[scalar_index..scalar_index + scalars]
            .iter()
            .sum();
        widths.push(width);
        scalar_index += scalars;
    }
    let mut positions = Vec::with_capacity(widths.len());
    let mut x = 0.0_f32;
    for width in &widths {
        positions.push(x);
        x += width;
    }
    (Some(positions), Some(widths))
}

/// The run and character index for a DOM offset. An offset on a boundary between two runs belongs to
/// the earlier run, at its end, unless the earlier run ends a line.
pub fn dom_offset_to_position(runs: &[RunSpec], dom_offset: usize) -> Option<TextPositionSpec> {
    let containing = runs.iter().enumerate().find(|(index, run)| {
        let next_starts_here = runs
            .get(index + 1)
            .is_some_and(|next| next.dom_start == dom_offset);
        dom_offset >= run.dom_start
            && dom_offset <= run.dom_end
            && !(dom_offset == run.dom_end && next_starts_here)
    });
    if let Some((run_index, run)) = containing {
        let character_index = run
            .dom_offsets
            .iter()
            .position(|&offset| offset >= dom_offset)
            .unwrap_or(run.character_lengths.len());
        return Some(TextPositionSpec {
            run: run_index,
            character_index,
        });
    }
    runs.iter()
        .position(|run| run.dom_start > dom_offset)
        .map(|run| TextPositionSpec {
            run,
            character_index: 0,
        })
}

/// The DOM offset for a run position, the inverse of [`dom_offset_to_position`].
pub fn position_to_dom_offset(runs: &[RunSpec], pos: TextPositionSpec) -> Option<usize> {
    let run = runs.get(pos.run)?;
    run.dom_offsets.get(pos.character_index).copied()
}

/// The selection between two DOM offsets, clipped to the runs. Returns `None` when neither offset
/// lies in a run. A backward selection keeps its direction: anchor is the earlier offset when the
/// selection runs forward, and the later one when it runs backward.
pub fn clip_selection(
    runs: &[RunSpec],
    dom_anchor: usize,
    dom_focus: usize,
) -> Option<SelectionSpec> {
    let lo = dom_anchor.min(dom_focus);
    let hi = dom_anchor.max(dom_focus);
    if !runs
        .iter()
        .any(|run| run.dom_end >= lo && run.dom_start <= hi)
    {
        return None;
    }
    let anchor = clip_end(runs, dom_anchor)?;
    let focus = clip_end(runs, dom_focus)?;
    Some(SelectionSpec { anchor, focus })
}

/// One end of a selection, moved into the nearest run when it lies outside all of them.
fn clip_end(runs: &[RunSpec], dom_offset: usize) -> Option<TextPositionSpec> {
    let first = runs.first()?;
    let last = runs.last()?;
    let clipped = dom_offset.clamp(first.dom_start, last.dom_end);
    dom_offset_to_position(runs, clipped)
}
