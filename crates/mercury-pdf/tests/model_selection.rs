// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The viewer model's text selection on synthetic page text, through the public API only.

use std::collections::HashMap;

use mercury_pdf::cache::budget;
use mercury_pdf::selection::{Caret, Granularity, IndexedText, PagePoint, Selection, TextSource};
use mercury_pdf::types::{PageText, Rect, TextChar};

/// Every character is 6 points wide and 10 tall.
const WIDTH: f32 = 6.0;
const HEIGHT: f32 = 10.0;

fn glyph(ch: char, x: f32, top: f32) -> TextChar {
    TextChar {
        ch,
        bounds: Rect::new(x, top, x + WIDTH, top + HEIGHT),
    }
}

/// A character the engine made up: no place on the page.
fn made_up(ch: char) -> TextChar {
    TextChar {
        ch,
        bounds: Rect::default(),
    }
}

/// One line of glyphs from `x0`, spaces included (with boxes, as a PDF's real spaces have).
fn line(text: &str, x0: f32, top: f32) -> Vec<TextChar> {
    text.chars()
        .enumerate()
        .map(|(index, ch)| glyph(ch, x0 + index as f32 * WIDTH, top))
        .collect()
}

/// Lines one under the other, 20 points apart from `top`, each ended by the engine's `\r\n`.
fn paragraph(lines: &[&str], x0: f32, top: f32) -> PageText {
    let mut chars = Vec::new();
    for (index, text) in lines.iter().enumerate() {
        chars.extend(line(text, x0, top + index as f32 * 20.0));
        if index + 1 < lines.len() {
            chars.push(made_up('\r'));
            chars.push(made_up('\n'));
        }
    }
    PageText { chars }
}

fn one_line(text: &str) -> PageText {
    PageText {
        chars: line(text, 10.0, 100.0),
    }
}

fn pages(entries: Vec<(u32, PageText)>) -> HashMap<u32, IndexedText> {
    entries
        .into_iter()
        .map(|(page, text)| (page, IndexedText::new(text)))
        .collect()
}

fn at(page: u32, x: f32, y: f32) -> PagePoint {
    PagePoint { page, x, y }
}

/// A selection by a press at one point and a drag to another.
fn drag(texts: &dyn TextSource, from: PagePoint, to: PagePoint, clicks: u8) -> Selection {
    let mut selection = Selection::new();
    selection.press(from, clicks);
    selection.update(texts);
    selection.drag(to);
    selection.update(texts);
    selection.release();
    selection
}

#[test]
fn selection_in_one_line_runs_from_caret_to_caret() {
    let texts = pages(vec![(0, one_line("Hello world"))]);
    // 'H' is x 10..16, centre 13; the space is 40..46, centre 43.
    let cases = [
        // press, drag, expected text
        ((10.5, 105.0), (40.5, 105.0), Some("Hello")),
        ((10.5, 105.0), (43.5, 105.0), Some("Hello ")),
        // From the right half of 'H' the 'H' is not selected.
        ((14.0, 105.0), (40.5, 105.0), Some("ello")),
        // Dragging backwards gives the same selection.
        ((40.5, 105.0), (10.5, 105.0), Some("Hello")),
        // Beyond the end of the line and before its start.
        ((10.5, 105.0), (500.0, 105.0), Some("Hello world")),
        ((-50.0, 105.0), (500.0, 105.0), Some("Hello world")),
        // Above and below the text: the nearest line is still this one.
        ((10.5, 0.0), (500.0, 900.0), Some("Hello world")),
        // A press and a release in the same place selects nothing.
        ((20.0, 105.0), (20.0, 105.0), None),
        ((20.0, 105.0), (21.0, 105.0), None),
    ];
    for (press, release, expected) in cases {
        let selection = drag(
            &texts,
            at(0, press.0, press.1),
            at(0, release.0, release.1),
            1,
        );
        assert_eq!(
            selection.text(&texts).as_deref(),
            expected,
            "{press:?} -> {release:?}"
        );
        assert_eq!(selection.range().is_some(), expected.is_some());
    }
}

#[test]
fn selection_a_click_selects_nothing_and_clears_the_old_selection() {
    let texts = pages(vec![(0, one_line("Hello world"))]);
    let mut selection = drag(&texts, at(0, 10.5, 105.0), at(0, 70.0, 105.0), 1);
    assert!(selection.text(&texts).is_some());
    selection.press(at(0, 30.0, 105.0), 1);
    assert_eq!(
        selection.range(),
        None,
        "a new press drops the old selection"
    );
    selection.update(&texts);
    selection.release();
    assert_eq!(selection.text(&texts), None);
}

#[test]
fn selection_across_two_lines_joins_them_with_a_newline() {
    let text = paragraph(&["First line", "Second line"], 10.0, 100.0);
    let texts = pages(vec![(0, text)]);
    // "First line" is x 10..70; 'l' of "line" is index 6 (x 46..52). "Second line": 'S' 10..16,
    // 'e' 16..22, 'c' 22..28.
    let selection = drag(&texts, at(0, 46.5, 105.0), at(0, 27.0, 125.0), 1);
    assert_eq!(selection.text(&texts).as_deref(), Some("line\nSec"));
    // Backwards.
    let backwards = drag(&texts, at(0, 27.0, 125.0), at(0, 46.5, 105.0), 1);
    assert_eq!(backwards.text(&texts).as_deref(), Some("line\nSec"));
}

#[test]
fn selection_normalises_line_endings_to_newlines() {
    let mut chars = line("ab", 10.0, 100.0);
    chars.push(made_up('\r'));
    chars.push(made_up('\n'));
    chars.extend(line("cd", 10.0, 120.0));
    chars.push(made_up('\r'));
    chars.extend(line("ef", 10.0, 140.0));
    chars.push(made_up('\n'));
    chars.extend(line("gh", 10.0, 160.0));
    let texts = pages(vec![(0, PageText { chars })]);
    let selection = drag(&texts, at(0, -5.0, 100.0), at(0, 500.0, 170.0), 1);
    assert_eq!(selection.text(&texts).as_deref(), Some("ab\ncd\nef\ngh"));
}

#[test]
fn selection_across_a_page_break_joins_the_pages_with_a_newline() {
    let texts = pages(vec![
        (0, paragraph(&["Page one text"], 10.0, 100.0)),
        (1, paragraph(&["Page two text"], 10.0, 100.0)),
        (2, paragraph(&["Page three"], 10.0, 100.0)),
    ]);
    // From "one" ('o' is index 5, x 40..46) on page 0 to "tw" of page 1 (index 5..7).
    let selection = drag(&texts, at(0, 40.5, 105.0), at(1, 51.0, 105.0), 1);
    assert_eq!(selection.text(&texts).as_deref(), Some("one text\nPage tw"));
    let (start, end) = selection.range().expect("a range");
    assert_eq!(start, Caret { page: 0, index: 5 });
    assert_eq!(end, Caret { page: 1, index: 7 });
    // Across three pages: the middle one is whole.
    let all = drag(&texts, at(0, 10.5, 105.0), at(2, 500.0, 105.0), 1);
    assert_eq!(
        all.text(&texts).as_deref(),
        Some("Page one text\nPage two text\nPage three")
    );
    // Dragging up from page 2 to page 0 gives the same.
    let up = drag(&texts, at(2, 500.0, 105.0), at(0, 10.5, 105.0), 1);
    assert_eq!(up.text(&texts), all.text(&texts));
}

#[test]
fn selection_copies_made_up_characters_but_never_hits_them() {
    // "ab cd" where the space was made up and has no box.
    let mut chars = line("ab", 10.0, 100.0);
    chars.push(made_up(' '));
    chars.extend(line("cd", 22.0, 100.0));
    let text = IndexedText::new(PageText { chars });
    // The pointer in the "gap" where the space would be lands on a real neighbour.
    let nearest = text.nearest_char(21.0, 105.0).expect("a character");
    assert!(nearest == 1 || nearest == 3, "{nearest}");
    let texts = pages(vec![(0, text.text().clone())]);
    let selection = drag(&texts, at(0, 10.5, 105.0), at(0, 500.0, 105.0), 1);
    assert_eq!(selection.text(&texts).as_deref(), Some("ab cd"));
    // The highlight covers the glyphs only: one rectangle for the line, 10..34.
    assert_eq!(
        selection.rects_on_page(0, &texts),
        vec![Rect::new(10.0, 100.0, 34.0, 110.0)]
    );
}

#[test]
fn selection_a_page_with_no_boxes_cannot_be_hit() {
    let texts = pages(vec![(
        0,
        PageText {
            chars: "invisible".chars().map(made_up).collect(),
        },
    )]);
    let selection = drag(&texts, at(0, 1.0, 1.0), at(0, 50.0, 50.0), 1);
    assert_eq!(selection.range(), None);
    assert_eq!(selection.text(&texts), None);
    let empty = pages(vec![(0, PageText::default())]);
    assert_eq!(
        drag(&empty, at(0, 1.0, 1.0), at(0, 5.0, 5.0), 1).range(),
        None
    );
}

#[test]
fn selection_a_double_click_selects_a_word() {
    let texts = pages(vec![(0, one_line("Hello world, don't stop"))]);
    // Indexes: "Hello"0..5, ' '5, "world"6..11, ','11, ' '12, "don't"13..18, ' '18, "stop"19..23.
    let cases = [
        (13.0_f32, "Hello"),
        (10.5, "Hello"),
        (10.0 + 6.0 * 8.0 + 1.0, "world"),
        // The comma is a run of punctuation of its own.
        (10.0 + 6.0 * 11.0 + 1.0, ","),
        // "don't" is one word, with its apostrophe, whichever letter is hit.
        (10.0 + 6.0 * 13.0 + 1.0, "don't"),
        (10.0 + 6.0 * 16.0 + 1.0, "don't"),
        (10.0 + 6.0 * 21.0 + 1.0, "stop"),
        // A space is a run of spaces.
        (10.0 + 6.0 * 5.0 + 1.0, " "),
    ];
    for (x, word) in cases {
        let mut selection = Selection::new();
        selection.press(at(0, x, 105.0), 2);
        selection.update(&texts);
        selection.release();
        assert_eq!(selection.text(&texts).as_deref(), Some(word), "x = {x}");
    }
    assert_eq!(Granularity::from_clicks(2), Granularity::Word);
}

#[test]
fn selection_a_double_click_and_drag_grows_by_words() {
    let texts = pages(vec![(0, one_line("Hello world, stop"))]);
    // Double click in "Hello", drag into "world": whole words from "Hello" to "world".
    let selection = drag(
        &texts,
        at(0, 12.0, 105.0),
        at(0, 10.0 + 6.0 * 7.0 + 1.0, 105.0),
        2,
    );
    assert_eq!(selection.text(&texts).as_deref(), Some("Hello world"));
    // Dragging back to the left of the press word extends to its start.
    let back = drag(
        &texts,
        at(0, 10.0 + 6.0 * 7.0 + 1.0, 105.0),
        at(0, 12.0, 105.0),
        2,
    );
    assert_eq!(back.text(&texts).as_deref(), Some("Hello world"));
}

#[test]
fn selection_a_word_stays_on_its_line() {
    // "abc" ends line one and "def" starts line two, with no separator between them in the text.
    let mut chars = line("abc", 10.0, 100.0);
    chars.extend(line("def", 10.0, 120.0));
    let texts = pages(vec![(0, PageText { chars })]);
    let mut selection = Selection::new();
    selection.press(at(0, 12.0, 105.0), 2);
    selection.update(&texts);
    assert_eq!(selection.text(&texts).as_deref(), Some("abc"));
}

#[test]
fn selection_a_triple_click_selects_the_line_without_its_break() {
    let texts = pages(vec![(
        0,
        paragraph(&["First line", "Second line", "Third"], 10.0, 100.0),
    )]);
    let mut selection = Selection::new();
    selection.press(at(0, 30.0, 125.0), 3);
    selection.update(&texts);
    selection.release();
    assert_eq!(selection.text(&texts).as_deref(), Some("Second line"));
    // Clicking more than three times is still a line.
    let mut many = Selection::new();
    many.press(at(0, 30.0, 125.0), 7);
    many.update(&texts);
    assert_eq!(many.text(&texts).as_deref(), Some("Second line"));
    // Triple click and drag to the next line: two whole lines.
    let two = drag(&texts, at(0, 30.0, 105.0), at(0, 20.0, 125.0), 3);
    assert_eq!(two.text(&texts).as_deref(), Some("First line\nSecond line"));
}

#[test]
fn selection_right_to_left_text_is_copied_in_index_order() {
    // Logical order a, b, c; on the page they run from right to left: a is rightmost.
    let chars = vec![
        TextChar {
            ch: 'a',
            bounds: Rect::new(34.0, 100.0, 40.0, 110.0),
        },
        TextChar {
            ch: 'b',
            bounds: Rect::new(28.0, 100.0, 34.0, 110.0),
        },
        TextChar {
            ch: 'c',
            bounds: Rect::new(22.0, 100.0, 28.0, 110.0),
        },
    ];
    let text = IndexedText::new(PageText { chars });
    assert!(text.lines()[0].rtl);
    let texts = pages(vec![(0, text.text().clone())]);
    // From the right half of nothing (right of 'a', before it in reading order) to the left of
    // 'c' (after it): everything, in the order a, b, c.
    let everything = drag(&texts, at(0, 39.0, 105.0), at(0, 22.5, 105.0), 1);
    assert_eq!(everything.text(&texts).as_deref(), Some("abc"));
    assert_eq!(
        everything.rects_on_page(0, &texts),
        vec![Rect::new(22.0, 100.0, 40.0, 110.0)]
    );
    // From 'b' to 'c': the later two, not reordered, one rectangle over their boxes.
    let tail = drag(&texts, at(0, 33.0, 105.0), at(0, 22.5, 105.0), 1);
    assert_eq!(tail.text(&texts).as_deref(), Some("bc"));
    assert_eq!(
        tail.rects_on_page(0, &texts),
        vec![Rect::new(22.0, 100.0, 34.0, 110.0)]
    );
    // Dragging the other way gives the same characters.
    let reverse = drag(&texts, at(0, 22.5, 105.0), at(0, 33.0, 105.0), 1);
    assert_eq!(reverse.text(&texts).as_deref(), Some("bc"));
}

#[test]
fn selection_highlights_are_merged_one_per_line() {
    let texts = pages(vec![(
        0,
        paragraph(&["First line", "Second line", "Third"], 10.0, 100.0),
    )]);
    // From "line" of the first line to "Thi" of the third.
    let selection = drag(&texts, at(0, 46.5, 105.0), at(0, 29.0, 145.0), 1);
    let rects = selection.rects_on_page(0, &texts);
    assert_eq!(
        rects,
        vec![
            // "line": index 6..10, x 46..70
            Rect::new(46.0, 100.0, 70.0, 110.0),
            // the whole second line: 11 characters, x 10..76
            Rect::new(10.0, 120.0, 76.0, 130.0),
            // "Thi": x 10..28
            Rect::new(10.0, 140.0, 28.0, 150.0),
        ]
    );
    assert!(selection.rects_on_page(1, &texts).is_empty());
}

#[test]
fn selection_rectangles_on_a_middle_page_cover_every_line() {
    let texts = pages(vec![
        (0, paragraph(&["a"], 10.0, 100.0)),
        (1, paragraph(&["one", "two"], 10.0, 100.0)),
        (2, paragraph(&["z"], 10.0, 100.0)),
    ]);
    let selection = drag(&texts, at(0, 10.5, 105.0), at(2, 500.0, 105.0), 1);
    assert_eq!(selection.rects_on_page(1, &texts).len(), 2);
    assert_eq!(selection.rects_on_page(0, &texts).len(), 1);
    assert_eq!(selection.rects_on_page(2, &texts).len(), 1);
    assert!(selection.rects_on_page(3, &texts).is_empty());
}

#[test]
fn selection_waits_for_the_text_of_the_page_it_needs() {
    let mut texts: HashMap<u32, IndexedText> = HashMap::new();
    let mut selection = Selection::new();
    selection.press(at(0, 10.5, 105.0), 1);
    selection.update(&texts);
    assert_eq!(selection.range(), None, "no text yet");
    assert_eq!(selection.pages_needed(10), vec![0]);
    texts.insert(0, IndexedText::new(one_line("Hello world")));
    selection.drag(at(2, 500.0, 105.0));
    selection.update(&texts);
    assert_eq!(
        selection.range(),
        None,
        "the page under the pointer has no text yet"
    );
    assert_eq!(selection.pages_needed(10), vec![0, 2, 1]);
    texts.insert(2, IndexedText::new(one_line("far")));
    selection.update(&texts);
    selection.release();
    assert!(selection.range().is_some());
    // The page between is missing: the selection is not complete and its text leaves it out.
    assert!(!selection.is_complete(&texts));
    assert_eq!(selection.text(&texts).as_deref(), Some("Hello world\nfar"));
    texts.insert(1, IndexedText::new(one_line("mid")));
    assert!(selection.is_complete(&texts));
    assert_eq!(
        selection.text(&texts).as_deref(),
        Some("Hello world\nmid\nfar")
    );
    // update is idempotent.
    let range = selection.range();
    selection.update(&texts);
    selection.update(&texts);
    assert_eq!(selection.range(), range);
}

#[test]
fn selection_drag_only_counts_while_the_button_is_down() {
    let texts = pages(vec![(0, one_line("Hello world"))]);
    let mut selection = Selection::new();
    selection.drag(at(0, 50.0, 105.0));
    selection.update(&texts);
    assert_eq!(selection.range(), None, "a drag without a press");
    selection.press(at(0, 10.5, 105.0), 1);
    selection.update(&texts);
    selection.release();
    selection.drag(at(0, 50.0, 105.0));
    selection.update(&texts);
    assert_eq!(selection.range(), None, "a drag after the release");
    assert!(!selection.is_held());
}

#[test]
fn selection_pages_needed_are_capped_at_the_copy_limit() {
    let mut selection = Selection::new();
    selection.press(at(0, 1.0, 1.0), 1);
    selection.drag(at(1000, 1.0, 1.0));
    let pages = selection.pages_needed(5000);
    assert_eq!(
        &pages[..2],
        &[0, 1000],
        "the press and the pointer come first"
    );
    assert_eq!(pages.len(), 2 + (budget::SELECTED_PAGES as usize - 1));
    assert!(pages.contains(&255) && !pages.contains(&256));
    // Pages that do not exist are not asked for.
    assert_eq!(selection.pages_needed(3), vec![0, 1, 2]);
    selection.clear();
    assert!(selection.pages_needed(10).is_empty());
}

#[test]
fn selection_copied_text_is_cut_at_one_mebibyte_on_a_character_boundary() {
    // 400,000 three-byte characters in one very long line: 1.2 MB.
    let chars: Vec<TextChar> = (0..400_000)
        .map(|index| glyph('\u{20AC}', index as f32 * 2.0, 100.0))
        .collect();
    let texts = pages(vec![(0, PageText { chars })]);
    let selection = drag(&texts, at(0, -10.0, 105.0), at(0, 1_000_000.0, 105.0), 1);
    let text = selection.text(&texts).expect("text");
    assert_eq!(budget::SELECTED_TEXT_BYTES, 1024 * 1024);
    assert_eq!(
        text.len(),
        1_048_575,
        "the largest whole number of characters"
    );
    assert!(text.chars().all(|ch| ch == '\u{20AC}'));
}

#[test]
fn selection_lines_follow_breaks_and_the_geometry() {
    // Two lines with no break character between them, split by position; the first has a
    // trailing made-up space and the second ends with a CRLF.
    let mut chars = line("one", 10.0, 100.0);
    chars.push(made_up(' '));
    chars.extend(line("two", 10.0, 120.0));
    chars.push(made_up('\r'));
    chars.push(made_up('\n'));
    chars.extend(line("three", 10.0, 140.0));
    let text = IndexedText::new(PageText { chars });
    let lines = text.lines();
    assert_eq!(lines.len(), 3);
    assert_eq!((lines[0].start, lines[0].end, lines[0].text_end), (0, 4, 3));
    assert_eq!((lines[1].start, lines[1].end, lines[1].text_end), (4, 9, 7));
    assert_eq!(
        (lines[2].start, lines[2].end, lines[2].text_end),
        (9, 14, 14)
    );
    assert_eq!(lines[0].bounds, Rect::new(10.0, 100.0, 28.0, 110.0));
    assert_eq!(text.line_of(0), Some(0));
    assert_eq!(text.line_of(3), Some(0));
    assert_eq!(text.line_of(4), Some(1));
    assert_eq!(text.line_of(13), Some(2));
    assert_eq!(text.line_of(14), None);
    assert!(lines.iter().all(|line| !line.rtl));
}

#[test]
fn selection_text_of_a_blank_line_is_kept() {
    let mut chars = line("a", 10.0, 100.0);
    chars.extend([made_up('\n'), made_up('\n')]);
    chars.extend(line("b", 10.0, 140.0));
    let texts = pages(vec![(0, PageText { chars })]);
    let selection = drag(&texts, at(0, -5.0, 100.0), at(0, 500.0, 145.0), 1);
    assert_eq!(selection.text(&texts).as_deref(), Some("a\n\nb"));
}

#[test]
fn selection_hostile_boxes_do_not_panic() {
    let chars = vec![
        TextChar {
            ch: 'a',
            bounds: Rect::new(f32::NAN, 0.0, 1.0, 1.0),
        },
        TextChar {
            ch: 'b',
            bounds: Rect::new(0.0, f32::INFINITY, 5.0, f32::INFINITY),
        },
        glyph('c', 10.0, 10.0),
    ];
    let texts = pages(vec![(0, PageText { chars })]);
    let selection = drag(
        &texts,
        at(0, f32::NAN, 0.0),
        at(0, f32::INFINITY, f32::NEG_INFINITY),
        1,
    );
    // Whatever it selects, it does not panic, and a range stays inside the text.
    if let Some((start, end)) = selection.range() {
        assert!(start < end && end.index <= 3);
    }
    let _ = selection.text(&texts);
}
