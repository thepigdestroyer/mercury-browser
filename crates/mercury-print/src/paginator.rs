// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Streaming pagination: rows of a long capture come in tile by tile, and pages are cut as soon
//! as the search window after them is buffered. Only one page plus the search window is kept.

use image::RgbaImage;

use crate::breaks::{BreakCandidate, next_break};
use crate::raster::row_candidates;

/// One printed page: `rgba` is the page's rows, full width and at most the page height (the last
/// page is shorter, never padded). `index` is 0 for the first page.
#[derive(Debug, Clone)]
pub struct PageImage {
    pub rgba: RgbaImage,
    pub index: usize,
}

#[derive(Debug)]
pub struct Paginator {
    page_rows: u32,
    search_rows: u32,
    max_pages: usize,
    width: Option<u32>,
    /// Rows from `top` (inclusive) to `end` (exclusive), RGBA8, row-major.
    raw: Vec<u8>,
    top: u32,
    end: u32,
    candidates: Vec<BreakCandidate>,
    produced: usize,
    dropped: bool,
    ready: Vec<PageImage>,
}

impl Paginator {
    pub fn new(page_rows: u32, search_rows: u32, max_pages: usize) -> Self {
        Self {
            page_rows: page_rows.max(1),
            search_rows,
            max_pages: max_pages.max(1),
            width: None,
            raw: Vec::new(),
            top: 0,
            end: 0,
            candidates: Vec::new(),
            produced: 0,
            dropped: false,
            ready: Vec::new(),
        }
    }

    /// Appends the next rows of the document (contiguous with the rows pushed before).
    pub fn push_rows(&mut self, rows: &RgbaImage) {
        if rows.height() == 0 {
            return;
        }
        if self.produced >= self.max_pages {
            self.dropped = true;
            return;
        }
        match self.width {
            None => self.width = Some(rows.width()),
            Some(width) if width != rows.width() => {
                log::warn!(
                    "print paginator: rows are {} px wide, the page is {} px",
                    rows.width(),
                    width
                );
                return;
            }
            Some(_) => {}
        }
        self.candidates.extend(row_candidates(rows, self.end));
        self.raw.extend_from_slice(rows.as_raw());
        self.end = self.end.saturating_add(rows.height());
        self.cut_ready();
    }

    /// Pages cut since the last call. Their order is the document order.
    pub fn ready_pages(&mut self) -> Vec<PageImage> {
        std::mem::take(&mut self.ready)
    }

    /// Cuts the remaining rows into pages and returns every page not yet taken. A document with
    /// no rows gives one blank page.
    pub fn finish(mut self) -> Vec<PageImage> {
        if self.width.is_none() {
            let blank = RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
            return vec![PageImage {
                rgba: blank,
                index: 0,
            }];
        }
        while self.produced < self.max_pages && self.top < self.end {
            match next_break(
                &self.candidates,
                self.top,
                self.end,
                self.page_rows,
                self.search_rows,
            ) {
                Some(cut) => self.emit(cut),
                None => self.emit(self.end),
            }
        }
        std::mem::take(&mut self.ready)
    }

    /// True when the document has more rows than `max_pages` pages hold.
    pub fn truncated(&self) -> bool {
        self.dropped || (self.produced >= self.max_pages && self.top < self.end)
    }

    fn cut_ready(&mut self) {
        let threshold = self.page_rows.saturating_add(self.search_rows.max(1));
        while self.produced < self.max_pages && self.end.saturating_sub(self.top) >= threshold {
            let Some(cut) = next_break(
                &self.candidates,
                self.top,
                u32::MAX,
                self.page_rows,
                self.search_rows,
            ) else {
                break;
            };
            self.emit(cut);
        }
    }

    /// Cuts the page `top..cut` off the buffer.
    fn emit(&mut self, cut: u32) {
        let cut = cut.min(self.end);
        let width = self.width.unwrap_or(0);
        let rows = cut.saturating_sub(self.top);
        let bytes = rows as usize * width as usize * 4;
        let page_raw: Vec<u8> = self.raw.drain(..bytes.min(self.raw.len())).collect();
        if let Some(rgba) = RgbaImage::from_raw(width, rows, page_raw) {
            self.ready.push(PageImage {
                rgba,
                index: self.produced,
            });
        }
        self.produced += 1;
        self.top = cut;
        let done = self.candidates.partition_point(|c| c.at <= cut);
        self.candidates.drain(..done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
    const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);

    /// Rows where `quiet(y)` is white and every other row is a text-like black and white pattern.
    fn document(width: u32, height: u32, quiet: impl Fn(u32) -> bool) -> RgbaImage {
        let mut image = RgbaImage::from_pixel(width, height, WHITE);
        for y in 0..height {
            if quiet(y) {
                continue;
            }
            for x in 0..width {
                image.put_pixel(x, y, if x % 2 == 0 { BLACK } else { WHITE });
            }
        }
        image
    }

    fn rows(source: &RgbaImage, start: u32, len: u32) -> RgbaImage {
        image::imageops::crop_imm(source, 0, start, source.width(), len).to_image()
    }

    fn heights(pages: &[PageImage]) -> Vec<u32> {
        pages.iter().map(|p| p.rgba.height()).collect()
    }

    #[test]
    fn bands_every_quiet_gap_makes_pages_at_blank_rows() {
        let source = document(16, 2000, |y| (980..1000).contains(&y));
        let mut paginator = Paginator::new(1000, 100, 10);
        paginator.push_rows(&source);
        let pages = paginator.finish();
        assert_eq!(pages.len(), 3);
        assert!((980..=1000).contains(&pages[0].rgba.height()));
        assert_eq!(heights(&pages).iter().sum::<u32>(), 2000);
    }

    #[test]
    fn streaming_equals_one_shot() {
        let source = document(32, 2600, |y| y % 83 < 6);
        let mut one = Paginator::new(500, 120, 50);
        one.push_rows(&source);
        let one_shot = heights(&one.finish());

        let mut streamed = Paginator::new(500, 120, 50);
        let mut at = 0;
        for len in [1, 300, 7, 999, 500, 450, 343] {
            streamed.push_rows(&rows(&source, at, len));
            at += len;
        }
        assert_eq!(at, 2600);
        assert_eq!(heights(&streamed.finish()), one_shot);
    }

    #[test]
    fn empty_document_gives_one_blank_page() {
        let pages = Paginator::new(1000, 100, 10).finish();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].rgba.height(), 1);
        assert!(pages[0].rgba.pixels().all(|p| *p == WHITE));
    }

    #[test]
    fn max_pages_truncates() {
        let source = document(8, 1000, |_| true);
        let mut paginator = Paginator::new(100, 10, 3);
        paginator.push_rows(&source);
        assert!(paginator.truncated());
        assert_eq!(paginator.finish().len(), 3);
    }

    #[test]
    fn last_page_is_shorter_not_padded() {
        let source = document(8, 2500, |_| true);
        let mut paginator = Paginator::new(1000, 0, 10);
        paginator.push_rows(&source);
        assert!(!paginator.truncated());
        assert_eq!(heights(&paginator.finish()), vec![1000, 1000, 500]);
    }

    #[test]
    fn pages_cover_every_row_exactly_once() {
        let source = document(8, 2345, |y| y % 97 < 5);
        let mut paginator = Paginator::new(700, 150, 10);
        paginator.push_rows(&source);
        let pages = paginator.finish();
        let total: u32 = pages.iter().map(|p| p.rgba.height()).sum();
        assert_eq!(total, 2345);
        let joined: Vec<u8> = pages
            .iter()
            .flat_map(|p| p.rgba.as_raw().iter().copied())
            .collect();
        assert_eq!(joined.as_slice(), source.as_raw().as_slice());
        assert!(pages.iter().enumerate().all(|(i, p)| p.index == i));
    }

    #[test]
    fn ready_pages_are_taken_once() {
        let source = document(8, 1000, |_| true);
        let mut paginator = Paginator::new(300, 0, 10);
        paginator.push_rows(&source);
        let early = paginator.ready_pages();
        assert_eq!(heights(&early), vec![300, 300, 300]);
        assert!(paginator.ready_pages().is_empty());
        assert_eq!(heights(&paginator.finish()), vec![100]);
    }
}
