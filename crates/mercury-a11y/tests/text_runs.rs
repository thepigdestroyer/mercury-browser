// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Text runs: grapheme and word boundaries, run splitting, and the limits AccessKit imposes.

use mercury_a11y::text::{
    Direction, LineInput, MAX_RUN_CHARACTERS, MAX_RUNS_PER_TEXT_NODE, RunBuild, build_runs,
    graphemes, word_starts_in,
};

fn line<'a>(text: &'a str, advances: &'a [f32], dom_offsets: &'a [usize]) -> LineInput<'a> {
    LineInput {
        text,
        advances,
        dom_offsets,
        direction: Direction::LeftToRight,
        ends_with_hard_break: false,
    }
}

fn run_text(built: &RunBuild) -> String {
    built.runs.iter().map(|run| run.text.as_str()).collect()
}

fn offsets_for(text: &str) -> Vec<usize> {
    (0..=text.chars().count()).collect()
}

#[test]
fn a_family_emoji_is_one_character() {
    let text = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let spans = graphemes(text);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0], 0..text.len());
}

#[test]
fn crlf_is_one_character() {
    assert_eq!(graphemes("a\r\nb").len(), 3);
}

#[test]
fn character_lengths_sum_to_the_text_length() {
    let samples = [
        "plain",
        "e\u{301}",
        "a\u{301}\u{302}b",
        "\u{1100}\u{1161}\u{11A8}",
        "\u{1F1EF}\u{1F1F5}\u{1F1FA}\u{1F1F8}",
        "\u{1F468}\u{200D}\u{1F469}",
        "naïve café",
        "\u{0939}\u{093F}\u{0928}\u{094D}\u{0926}\u{0940}",
        "\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        "a\r\nb\n",
        "\t\u{00A0}x",
        "\u{1F44D}\u{1F3FD}",
    ];
    for text in samples {
        let advances = vec![1.0_f32; text.chars().count()];
        let offsets = offsets_for(text);
        let built = build_runs(&line(text, &advances, &offsets));
        assert_eq!(run_text(&built), text, "text round-trips for {text:?}");
        for run in &built.runs {
            let total: usize = run.character_lengths.iter().map(|&len| len as usize).sum();
            assert_eq!(total, run.text.len(), "lengths sum for {text:?}");
        }
    }
}

#[test]
fn word_starts_follow_uax29() {
    let text = "Hello, world";
    let spans = graphemes(text);
    assert_eq!(word_starts_in(text, &spans), vec![0, 5, 7]);
}

#[test]
fn a_300_character_line_is_split_in_runs_of_at_most_255() {
    let text: String = std::iter::repeat_n('a', 300).collect();
    let advances = vec![1.0_f32; 300];
    let offsets = offsets_for(&text);
    let built = build_runs(&line(&text, &advances, &offsets));
    assert!(built.runs.len() >= 2);
    for run in &built.runs {
        assert!(run.character_lengths.len() <= MAX_RUN_CHARACTERS);
        assert!(
            run.word_starts
                .iter()
                .all(|&i| (i as usize) < MAX_RUN_CHARACTERS)
        );
    }
    assert_eq!(run_text(&built), text);
}

#[test]
fn the_split_prefers_a_word_boundary() {
    let text = format!("{} {}", "a".repeat(200), "b".repeat(100));
    let advances = vec![1.0_f32; text.len()];
    let offsets = offsets_for(&text);
    let built = build_runs(&line(&text, &advances, &offsets));
    assert_eq!(built.runs[0].text, format!("{} ", "a".repeat(200)));
}

#[test]
fn a_cluster_over_255_bytes_is_cut() {
    let mut text = String::from("a");
    for _ in 0..400 {
        text.push('\u{301}');
    }
    let advances = vec![1.0_f32; text.chars().count()];
    let offsets = offsets_for(&text);
    let built = build_runs(&line(&text, &advances, &offsets));
    for run in &built.runs {
        assert!(run.character_lengths.iter().all(|&len| len as usize <= 255));
        let total: usize = run.character_lengths.iter().map(|&len| len as usize).sum();
        assert_eq!(total, run.text.len());
    }
    assert_eq!(run_text(&built), text);
}

#[test]
fn a_hard_break_is_one_character_in_the_last_run() {
    let text = "abc\n";
    let advances = [1.0_f32; 4];
    let offsets = offsets_for(text);
    let mut input = line(text, &advances, &offsets);
    input.ends_with_hard_break = true;
    let built = build_runs(&input);
    let last = built.runs.last().expect("a run");
    assert_eq!(*last.character_lengths.last().expect("a length"), 1);
}

#[test]
fn positions_and_widths_from_advances() {
    let text = "abc";
    let advances = [10.0_f32, 5.0, 5.0];
    let offsets = offsets_for(text);
    let built = build_runs(&line(text, &advances, &offsets));
    let run = &built.runs[0];
    assert_eq!(run.positions.as_deref(), Some(&[0.0_f32, 10.0, 15.0][..]));
    assert_eq!(run.widths.as_deref(), Some(&[10.0_f32, 5.0, 5.0][..]));
}

#[test]
fn rtl_positions_mirror_from_the_right_edge() {
    let text = "abc";
    let advances = [10.0_f32, 5.0, 5.0];
    let offsets = offsets_for(text);
    let mut input = line(text, &advances, &offsets);
    input.direction = Direction::RightToLeft;
    let built = build_runs(&input);
    let run = &built.runs[0];
    assert_eq!(run.direction, Direction::RightToLeft);
    assert_eq!(run.positions.as_deref(), Some(&[0.0_f32, 10.0, 15.0][..]));
}

#[test]
fn wrong_length_advances_never_panic() {
    let text = "abc";
    let advances = [1.0_f32];
    let offsets = offsets_for(text);
    let built = build_runs(&line(text, &advances, &offsets));
    assert!(built.advances_mismatched);
    let run = &built.runs[0];
    assert!(run.positions.is_none());
    assert_eq!(run.text, "abc");
}

#[test]
fn a_10000_grapheme_line_completes_within_limits() {
    let text: String = std::iter::repeat_n('x', 10_000).collect();
    let advances = vec![1.0_f32; 10_000];
    let offsets = offsets_for(&text);
    let built = build_runs(&line(&text, &advances, &offsets));
    assert!(built.runs.len() <= MAX_RUNS_PER_TEXT_NODE || built.metrics_dropped);
    assert_eq!(run_text(&built), text);
}

#[test]
fn empty_text_gives_no_run() {
    let built = build_runs(&line("", &[], &[]));
    assert!(built.runs.is_empty());
}
