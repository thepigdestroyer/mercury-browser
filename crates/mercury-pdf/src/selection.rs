// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Text selection over page text: hit-testing, ranges across pages, highlight rectangles and the
//! copied text. Pure logic: no I/O, no clock, no window.
//!
//! The text of a page arrives as a [`PageText`] in reading order (the order the PDF engine
//! reports, which for right-to-left text is the logical order). [`IndexedText`] adds a *line
//! index* to it, built once when the text arrives: a line ends at a line break character or where
//! the next character's box moves to another line (less than half of the smaller height shared).
//! The index is what makes a pointer event cheap (nearest line, then nearest character in it),
//! what a triple click selects, and what highlight rectangles are merged by.
//!
//! A selection is a pair of **carets** (between two characters of the whole document, `index`
//! 0 to the length of the page's text), so a click without a drag selects nothing, and a drag
//! from the middle of one letter to the middle of another selects from the caret nearer the first
//! to the caret nearer the second, like a browser does. The characters between the carets are
//! copied *by index*, so right-to-left text is never reordered; characters with an empty box
//! (a space or line break the engine made up) are never hit by the pointer but are copied.
//!
//! The pointer state is kept as page-space points, not content-space ones, so a zoom or a rotation
//! while the button is down does not move the selection, and a press on a page whose text has not
//! arrived is resolved when it does ([`Selection::update`] is idempotent: call it after every
//! change of the points and after every arrival of text).

use std::collections::HashMap;
use std::ops::Range;

use crate::cache::{PageStore, budget};
use crate::types::{PageText, Rect, TextChar};

/// A place between two characters: the page and how many characters of its text come before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Caret {
    pub page: u32,
    pub index: u32,
}

/// What a pointer press selects: a character position, a word (double click), a line (triple).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    Char,
    Word,
    Line,
}

impl Granularity {
    /// One click (or none) is a character, two a word, three or more a line.
    pub fn from_clicks(clicks: u8) -> Self {
        match clicks {
            0 | 1 => Self::Char,
            2 => Self::Word,
            _ => Self::Line,
        }
    }
}

/// A point on a page, in page space (points, y down, the page's own rotation applied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PagePoint {
    pub page: u32,
    pub x: f32,
    pub y: f32,
}

/// One line of a page's text.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// The first character of the line and one past its last (a line break belongs to the line
    /// it ends); lines are contiguous and cover the whole text.
    pub start: usize,
    pub end: usize,
    /// One past the last character that is text: the end without trailing line breaks and
    /// without trailing spaces the engine made up.
    pub text_end: usize,
    /// The union of the boxes of the line's characters; empty for a line with none.
    pub bounds: Rect,
    /// Whether the characters run from right to left on the page.
    pub rtl: bool,
}

/// A page's text with its line index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IndexedText {
    text: PageText,
    lines: Vec<Line>,
}

fn has_box(rect: &Rect) -> bool {
    rect.left.is_finite()
        && rect.top.is_finite()
        && rect.right.is_finite()
        && rect.bottom.is_finite()
        && !rect.is_empty()
}

fn is_break(ch: char) -> bool {
    ch == '\n' || ch == '\r'
}

fn same_line(a: &Rect, b: &Rect) -> bool {
    let shared = a.bottom.min(b.bottom) - a.top.max(b.top);
    shared > 0.5 * a.height().min(b.height())
}

fn centre_x(rect: &Rect) -> f32 {
    (rect.left + rect.right) / 2.0
}

fn centre_y(rect: &Rect) -> f32 {
    (rect.top + rect.bottom) / 2.0
}

fn gap_to(low: f32, high: f32, value: f32) -> f32 {
    (low - value).max(value - high).max(0.0)
}

fn distance_squared(rect: &Rect, x: f32, y: f32) -> f32 {
    let dx = gap_to(rect.left, rect.right, x);
    let dy = gap_to(rect.top, rect.bottom, y);
    dx * dx + dy * dy
}

fn make_line(chars: &[TextChar], start: usize, end: usize) -> Line {
    let mut bounds: Option<Rect> = None;
    let (mut first_x, mut last_x) = (None, None);
    for character in &chars[start..end] {
        if has_box(&character.bounds) {
            bounds = Some(bounds.map_or(character.bounds, |known| known.union(&character.bounds)));
            let x = centre_x(&character.bounds);
            first_x.get_or_insert(x);
            last_x = Some(x);
        }
    }
    let mut text_end = end;
    while text_end > start {
        let last = &chars[text_end - 1];
        if is_break(last.ch) || (last.ch.is_whitespace() && !has_box(&last.bounds)) {
            text_end -= 1;
        } else {
            break;
        }
    }
    Line {
        start,
        end,
        text_end,
        bounds: bounds.unwrap_or_default(),
        rtl: matches!((first_x, last_x), (Some(first), Some(last)) if first > last),
    }
}

fn build_lines(chars: &[TextChar]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut previous: Option<Rect> = None;
    let mut index = 0;
    while index < chars.len() {
        let character = &chars[index];
        if is_break(character.ch) {
            let mut end = index + 1;
            if character.ch == '\r' && chars.get(end).is_some_and(|next| next.ch == '\n') {
                end += 1;
            }
            lines.push(make_line(chars, start, end));
            start = end;
            previous = None;
            index = end;
            continue;
        }
        if has_box(&character.bounds) {
            if previous.is_some_and(|before| !same_line(&before, &character.bounds)) {
                lines.push(make_line(chars, start, index));
                start = index;
            }
            previous = Some(character.bounds);
        }
        index += 1;
    }
    if start < chars.len() {
        lines.push(make_line(chars, start, chars.len()));
    }
    lines
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Break,
    Space,
    Word,
    Other,
}

/// Combining marks the standard library does not call alphanumeric (an approximation: the common
/// blocks, not the whole of Unicode).
fn is_combining(ch: char) -> bool {
    matches!(u32::from(ch),
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05C7 | 0x064B..=0x065F
        | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

fn is_apostrophe(ch: char) -> bool {
    ch == '\'' || ch == '\u{2019}'
}

impl IndexedText {
    pub fn new(text: PageText) -> Self {
        let lines = build_lines(&text.chars);
        Self { text, lines }
    }

    pub fn text(&self) -> &PageText {
        &self.text
    }

    /// The number of characters.
    pub fn len(&self) -> usize {
        self.text.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.chars.is_empty()
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// The index of the line a character is on.
    pub fn line_of(&self, index: usize) -> Option<usize> {
        if index >= self.len() {
            return None;
        }
        let after = self.lines.partition_point(|line| line.start <= index);
        after.checked_sub(1)
    }

    fn nearest_line(&self, x: f32, y: f32) -> Option<usize> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| has_box(&line.bounds))
            .min_by(|(_, a), (_, b)| {
                distance_squared(&a.bounds, x, y)
                    .total_cmp(&distance_squared(&b.bounds, x, y))
                    .then_with(|| {
                        (centre_y(&a.bounds) - y)
                            .abs()
                            .total_cmp(&(centre_y(&b.bounds) - y).abs())
                    })
            })
            .map(|(index, _)| index)
    }

    fn nearest_in_line(&self, line: &Line, x: f32) -> Option<usize> {
        (line.start..line.end)
            .filter(|&index| has_box(&self.text.chars[index].bounds))
            .min_by(|&a, &b| {
                let (a_box, b_box) = (&self.text.chars[a].bounds, &self.text.chars[b].bounds);
                gap_to(a_box.left, a_box.right, x)
                    .total_cmp(&gap_to(b_box.left, b_box.right, x))
                    .then_with(|| {
                        (centre_x(a_box) - x)
                            .abs()
                            .total_cmp(&(centre_x(b_box) - x).abs())
                    })
            })
    }

    /// The character nearest to a point (page space) that has a box, or `None` when no character
    /// has one. A point past the end of a line, or above or below the text, is nearest to the
    /// end of the nearest line.
    pub fn nearest_char(&self, x: f32, y: f32) -> Option<usize> {
        let line = &self.lines[self.nearest_line(x, y)?];
        self.nearest_in_line(line, x)
    }

    /// The caret (0 to the length) a press at the point puts: before the nearest character if
    /// the point is in its first half along the line's direction, after it otherwise.
    pub fn caret_at(&self, x: f32, y: f32) -> Option<usize> {
        let line = &self.lines[self.nearest_line(x, y)?];
        let index = self.nearest_in_line(line, x)?;
        let middle = centre_x(&self.text.chars[index].bounds);
        let after = if line.rtl { x < middle } else { x > middle };
        Some(index + usize::from(after))
    }

    fn class_at(&self, index: usize, low: usize, high: usize) -> Class {
        let ch = self.text.chars[index].ch;
        if is_break(ch) {
            Class::Break
        } else if ch.is_whitespace() {
            Class::Space
        } else if ch.is_alphanumeric() || ch == '_' || is_combining(ch) {
            Class::Word
        } else if is_apostrophe(ch)
            && index > low
            && index + 1 < high
            && self.text.chars[index - 1].ch.is_alphanumeric()
            && self.text.chars[index + 1].ch.is_alphanumeric()
        {
            // "don't" is one word.
            Class::Word
        } else {
            Class::Other
        }
    }

    /// The word around a character: the run of letters and digits, of spaces or of punctuation
    /// it belongs to, within its line. An empty range for a line break or a bad index.
    pub fn word_range(&self, index: usize) -> Range<usize> {
        let Some(line) = self.line_of(index).map(|line| &self.lines[line]) else {
            return index..index;
        };
        let (low, high) = (line.start, line.text_end);
        if index >= high {
            return index..index;
        }
        let class = self.class_at(index, low, high);
        let mut start = index;
        while start > low && self.class_at(start - 1, low, high) == class {
            start -= 1;
        }
        let mut end = index + 1;
        while end < high && self.class_at(end, low, high) == class {
            end += 1;
        }
        start..end
    }

    /// The word under a point (see [`Self::word_range`]), if any character has a box.
    pub fn word_at(&self, x: f32, y: f32) -> Option<Range<usize>> {
        self.nearest_char(x, y).map(|index| self.word_range(index))
    }

    /// The text of the line nearest to a point, without its line break.
    pub fn line_at(&self, x: f32, y: f32) -> Option<Range<usize>> {
        let line = &self.lines[self.nearest_line(x, y)?];
        Some(line.start..line.text_end)
    }

    /// The rectangles that highlight the characters `range`: one per line, each the union of the
    /// boxes of the selected characters on that line (page space).
    pub fn highlight_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let end = range.end.min(self.len());
        if range.start >= end {
            return Vec::new();
        }
        let first = self.lines.partition_point(|line| line.end <= range.start);
        let mut rects = Vec::new();
        for line in &self.lines[first..] {
            if line.start >= end {
                break;
            }
            let from = range.start.max(line.start);
            let to = end.min(line.end);
            let mut merged: Option<Rect> = None;
            for character in &self.text.chars[from..to] {
                if has_box(&character.bounds) {
                    merged = Some(
                        merged.map_or(character.bounds, |known| known.union(&character.bounds)),
                    );
                }
            }
            rects.extend(merged);
        }
        rects
    }
}

/// Where the selection finds the text of a page.
pub trait TextSource {
    /// The text of a page, if it has arrived.
    fn page_text(&self, page: u32) -> Option<&IndexedText>;
}

impl TextSource for PageStore<IndexedText> {
    fn page_text(&self, page: u32) -> Option<&IndexedText> {
        self.get(page)
    }
}

impl TextSource for HashMap<u32, IndexedText> {
    fn page_text(&self, page: u32) -> Option<&IndexedText> {
        self.get(&page)
    }
}

/// The selection: where the pointer pressed and is now, and the characters between.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    press: Option<(PagePoint, Granularity)>,
    drag: Option<PagePoint>,
    held: bool,
    range: Option<(Caret, Caret)>,
}

/// The caret pair a point selects at a granularity on its page's text.
fn unit(text: &IndexedText, point: PagePoint, granularity: Granularity) -> Option<(Caret, Caret)> {
    let caret = |index: usize| Caret {
        page: point.page,
        index: u32::try_from(index).unwrap_or(u32::MAX),
    };
    match granularity {
        Granularity::Char => {
            let at = caret(text.caret_at(point.x, point.y)?);
            Some((at, at))
        }
        Granularity::Word => {
            let word = text.word_at(point.x, point.y)?;
            Some((caret(word.start), caret(word.end)))
        }
        Granularity::Line => {
            let line = text.line_at(point.x, point.y)?;
            Some((caret(line.start), caret(line.end)))
        }
    }
}

/// Appends `text` with `\r\n` and a lone `\r` turned into `\n`.
fn push_normalised(out: &mut String, text: &str) {
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                continue;
            }
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    /// A press: drops the old selection and starts a new one at the point; `clicks` is the
    /// number of clicks in a row (see [`Granularity::from_clicks`]). Call [`Self::update`] next.
    pub fn press(&mut self, point: PagePoint, clicks: u8) {
        *self = Self {
            press: Some((point, Granularity::from_clicks(clicks))),
            drag: None,
            held: true,
            range: None,
        };
    }

    /// The pointer moved with the button down; ignored otherwise.
    pub fn drag(&mut self, point: PagePoint) {
        if self.held && self.press.is_some() {
            self.drag = Some(point);
        }
    }

    /// The button went up. The selection stays.
    pub fn release(&mut self) {
        self.held = false;
    }

    /// Forgets everything: the selection and the pressed point.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Whether the button is down.
    pub fn is_held(&self) -> bool {
        self.held
    }

    /// The selected range as `(start, end)` carets, start before end; `None` when nothing (not
    /// even one character) is selected, or the text to resolve it has not arrived.
    pub fn range(&self) -> Option<(Caret, Caret)> {
        self.range
    }

    /// The pages whose text the selection needs, press page and pointer page first and then the
    /// pages between, at most [`budget::SELECTED_PAGES`] of them from the first; only the pages
    /// below `page_count`.
    pub fn pages_needed(&self, page_count: u32) -> Vec<u32> {
        let Some((press, _)) = self.press else {
            return Vec::new();
        };
        let head = self.drag.unwrap_or(press);
        let (low, high) = (press.page.min(head.page), press.page.max(head.page));
        let mut pages = vec![press.page];
        if head.page != press.page {
            pages.push(head.page);
        }
        let last = high.min(low.saturating_add(budget::SELECTED_PAGES - 1));
        pages.extend((low..=last).filter(|&page| page != press.page && page != head.page));
        pages.retain(|&page| page < page_count);
        pages
    }

    /// Works out the selected range from the pressed point and the pointer, with the text that
    /// has arrived. When the text of the page under the press or the pointer is missing, the
    /// range stays as it was until it arrives. Idempotent.
    pub fn update(&mut self, texts: &dyn TextSource) {
        let Some((press, granularity)) = self.press else {
            return;
        };
        let head = self.drag.unwrap_or(press);
        let (Some(press_text), Some(head_text)) =
            (texts.page_text(press.page), texts.page_text(head.page))
        else {
            return;
        };
        let (Some(first), Some(second)) = (
            unit(press_text, press, granularity),
            unit(head_text, head, granularity),
        ) else {
            self.range = None;
            return;
        };
        let start = first.0.min(second.0);
        let end = first.1.max(second.1);
        self.range = (start < end).then_some((start, end));
    }

    /// Whether the text of every page the selection spans (up to the copy limit) has arrived.
    pub fn is_complete(&self, texts: &dyn TextSource) -> bool {
        let Some((start, end)) = self.range else {
            return true;
        };
        let last = end
            .page
            .min(start.page.saturating_add(budget::SELECTED_PAGES - 1));
        (start.page..=last).all(|page| texts.page_text(page).is_some())
    }

    /// The selected text: the pages' characters between the carets joined by `\n`, with `\r\n`
    /// (and a lone `\r`) turned into `\n`, cut at [`budget::SELECTED_TEXT_BYTES`]. Pages whose
    /// text has not arrived are left out (see [`Self::is_complete`]). `None` when nothing is
    /// selected.
    pub fn text(&self, texts: &dyn TextSource) -> Option<String> {
        let (start, end) = self.range?;
        let last = end
            .page
            .min(start.page.saturating_add(budget::SELECTED_PAGES - 1));
        let mut out = String::new();
        let mut first = true;
        for page in start.page..=last {
            let Some(text) = texts.page_text(page) else {
                continue;
            };
            let from = if page == start.page {
                start.index as usize
            } else {
                0
            };
            let to = if page == end.page {
                end.index as usize
            } else {
                text.len()
            };
            if !first {
                out.push('\n');
            }
            first = false;
            push_normalised(&mut out, &text.text().slice(from..to));
            if out.len() >= budget::SELECTED_TEXT_BYTES {
                let mut cut = budget::SELECTED_TEXT_BYTES;
                while !out.is_char_boundary(cut) {
                    cut -= 1;
                }
                out.truncate(cut);
                break;
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// The highlight rectangles of the selection on one page (page space), one per line.
    pub fn rects_on_page(&self, page: u32, texts: &dyn TextSource) -> Vec<Rect> {
        let Some((start, end)) = self.range else {
            return Vec::new();
        };
        if page < start.page || page > end.page {
            return Vec::new();
        }
        let Some(text) = texts.page_text(page) else {
            return Vec::new();
        };
        let from = if page == start.page {
            start.index as usize
        } else {
            0
        };
        let to = if page == end.page {
            end.index as usize
        } else {
            text.len()
        };
        text.highlight_rects(from..to)
    }
}
