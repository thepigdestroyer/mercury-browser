// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Text runs for AccessKit: graphemes, words, the split of a line into runs, and the mapping between
//! DOM offsets, run positions and selections.

use std::ops::Range;

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

// stub
pub fn graphemes(_text: &str) -> Vec<Range<usize>> {
    Vec::new()
}

// stub
pub fn word_starts_in(_text: &str, _graphemes: &[Range<usize>]) -> Vec<usize> {
    Vec::new()
}

// stub
pub fn build_runs(_line: &LineInput<'_>) -> RunBuild {
    RunBuild::default()
}

// stub
pub fn dom_offset_to_position(_runs: &[RunSpec], _dom_offset: usize) -> Option<TextPositionSpec> {
    None
}

// stub
pub fn position_to_dom_offset(_runs: &[RunSpec], _pos: TextPositionSpec) -> Option<usize> {
    None
}

// stub
pub fn clip_selection(
    _runs: &[RunSpec],
    _dom_anchor: usize,
    _dom_focus: usize,
) -> Option<SelectionSpec> {
    None
}
