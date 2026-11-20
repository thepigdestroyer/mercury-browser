// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A deterministic [`PdfBackend`] for tests, with no PDF engine behind it.
//!
//! Everything it answers is a pure function of the request, so a test can check any tile or any
//! hit without a second implementation:
//!
//! - **Pixels.** [`fake_pixel`] is the colour of pixel `(x, y)` of a page at a scale and rotation;
//!   a tile is the matching rectangle of it, so a tile with the wrong origin, scale or rotation
//!   is noticed. [`expected_tile`] builds the whole expected tile.
//! - **Text.** [`fake_page_text`]: three lines on a grid of 6 × 12 point cells, `"Page N: the
//!   quick brown fox"`, `"jumps over the lazy dog"` and `"end of page N"`, with a generated
//!   `\r\n` (empty boxes, like PDFium's) between lines.
//! - **Find** over that text, case and whole-word options honoured.
//! - **Links**: one per page, to the next page; the last page links to
//!   `https://example.com/last`. **Outline**: Chapter 1 (with Section 1.1) and Chapter 2.
//!
//! Switches ([`FakeConfig`]) make it ask for a password, render slowly, panic after `K`
//! requests, or refuse the file as corrupt. [`FakeConfig::from_args`] reads them from command
//! line words, which is how the `pdf-fake-worker` test binary is steered.

use std::path::Path;
use std::time::Duration;

use crate::backend::PdfBackend;
use crate::error::PdfError;
use crate::types::{
    DocumentInfo, FindOptions, Link, LinkTarget, OutlineItem, PageSize, PageText, Rect, Rotation,
    SearchHit, TextChar, Tile, TileRequest, limits,
};

/// Width of one character cell of the fake text, in points.
pub const CELL_WIDTH: f32 = 6.0;
/// Height of one character cell, in points.
pub const CELL_HEIGHT: f32 = 12.0;
/// The distance between the tops of two lines, in points.
pub const LINE_PITCH: f32 = 14.0;
/// The left and top margin of the text, in points.
pub const TEXT_MARGIN: f32 = 36.0;

/// The size of the pages of a default fake document: US Letter.
pub const LETTER: PageSize = PageSize::new(612.0, 792.0);

/// How a [`FakeBackend`] behaves.
#[derive(Clone, Debug, PartialEq)]
pub struct FakeConfig {
    /// The pages of the document, in order.
    pub pages: Vec<PageSize>,
    pub title: Option<String>,
    /// The document is encrypted with this password.
    pub password: Option<String>,
    /// Every render sleeps this long first.
    pub render_delay: Duration,
    /// Every page of a find sleeps this long first.
    pub find_delay: Duration,
    /// The request after this many requests (renders, text, find, links, outline) panics.
    pub panic_after: Option<u32>,
    /// Opening fails as a damaged file.
    pub corrupt: bool,
}

impl Default for FakeConfig {
    fn default() -> Self {
        Self::with_pages(3)
    }
}

impl FakeConfig {
    /// `count` Letter pages and no switches.
    pub fn with_pages(count: usize) -> Self {
        Self {
            pages: vec![LETTER; count],
            title: Some("Fake document".to_owned()),
            password: None,
            render_delay: Duration::ZERO,
            find_delay: Duration::ZERO,
            panic_after: None,
            corrupt: false,
        }
    }

    /// Reads switches from command line words: `--pages=N`, `--title=T`, `--password=P`,
    /// `--render-delay-ms=N`, `--find-delay-ms=N`, `--panic-after=K`, `--corrupt`. An unknown
    /// word is an error naming it.
    pub fn from_args<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut config = Self::default();
        let number = |word: &str, value: &str| {
            value
                .parse::<u64>()
                .map_err(|_| format!("{word} needs a number, not {value:?}"))
        };
        for arg in args {
            let arg = arg.as_ref();
            let (name, value) = arg.split_once('=').unwrap_or((arg, ""));
            match name {
                "--pages" => {
                    let count = usize::try_from(number(name, value)?).unwrap_or(usize::MAX);
                    config.pages = vec![LETTER; count.min(limits::MAX_PAGES as usize)];
                }
                "--title" => config.title = Some(value.to_owned()),
                "--password" => config.password = Some(value.to_owned()),
                "--render-delay-ms" => {
                    config.render_delay = Duration::from_millis(number(name, value)?);
                }
                "--find-delay-ms" => {
                    config.find_delay = Duration::from_millis(number(name, value)?);
                }
                "--panic-after" => {
                    config.panic_after =
                        Some(u32::try_from(number(name, value)?).unwrap_or(u32::MAX));
                }
                "--corrupt" => config.corrupt = true,
                _ => return Err(format!("unknown switch {arg:?}")),
            }
        }
        Ok(config)
    }
}

/// The fake engine. See the module documentation.
#[derive(Debug, Default)]
pub struct FakeBackend {
    config: FakeConfig,
    open: bool,
    requests: u32,
}

impl FakeBackend {
    pub fn new(config: FakeConfig) -> Self {
        Self {
            config,
            open: false,
            requests: 0,
        }
    }

    /// A backend with `count` Letter pages.
    pub fn with_pages(count: usize) -> Self {
        Self::new(FakeConfig::with_pages(count))
    }

    /// Counts a request and panics when the configured number has been served.
    fn tick(&mut self) {
        if let Some(limit) = self.config.panic_after {
            assert!(
                self.requests < limit,
                "fake backend: asked to panic after {limit} requests"
            );
        }
        self.requests = self.requests.saturating_add(1);
    }

    fn check_page(&self, page: u32) -> Result<(), PdfError> {
        if !self.open {
            return Err(PdfError::NoDocument);
        }
        if (page as usize) < self.config.pages.len() {
            Ok(())
        } else {
            Err(PdfError::InvalidRequest("no such page".to_owned()))
        }
    }
}

impl PdfBackend for FakeBackend {
    fn open(&mut self, _path: &Path, password: Option<&str>) -> Result<DocumentInfo, PdfError> {
        if self.config.corrupt {
            return Err(PdfError::Corrupt("fake: damaged file".to_owned()));
        }
        if let Some(expected) = &self.config.password {
            match password {
                None => return Err(PdfError::PasswordRequired),
                Some(given) if given != expected => return Err(PdfError::PasswordIncorrect),
                Some(_) => {}
            }
        }
        if self.config.pages.is_empty() {
            return Err(PdfError::Corrupt("fake: no pages".to_owned()));
        }
        self.open = true;
        Ok(DocumentInfo {
            pages: self.config.pages.clone(),
            title: self.config.title.clone(),
        })
    }

    fn render(&mut self, request: &TileRequest) -> Result<Tile, PdfError> {
        self.tick();
        if !self.open {
            return Err(PdfError::NoDocument);
        }
        request.validate(&self.config.pages)?;
        if !self.config.render_delay.is_zero() {
            std::thread::sleep(self.config.render_delay);
        }
        Ok(expected_tile(request))
    }

    fn page_text(&mut self, page: u32) -> Result<PageText, PdfError> {
        self.tick();
        self.check_page(page)?;
        Ok(fake_page_text(page))
    }

    fn find_in_page(
        &mut self,
        page: u32,
        query: &str,
        options: FindOptions,
    ) -> Result<Vec<SearchHit>, PdfError> {
        self.tick();
        self.check_page(page)?;
        if !self.config.find_delay.is_zero() {
            std::thread::sleep(self.config.find_delay);
        }
        Ok(find_in_text(page, &fake_page_text(page), query, options))
    }

    fn links(&mut self, page: u32) -> Result<Vec<Link>, PdfError> {
        self.tick();
        self.check_page(page)?;
        let target = if (page as usize) + 1 < self.config.pages.len() {
            LinkTarget::Page {
                page: page + 1,
                y: Some(0.0),
            }
        } else {
            LinkTarget::Uri("https://example.com/last".to_owned())
        };
        Ok(vec![Link {
            rect: Rect::new(
                TEXT_MARGIN,
                TEXT_MARGIN,
                TEXT_MARGIN + 100.0,
                TEXT_MARGIN + CELL_HEIGHT,
            ),
            target,
        }])
    }

    fn outline(&mut self) -> Result<Vec<OutlineItem>, PdfError> {
        self.tick();
        if !self.open {
            return Err(PdfError::NoDocument);
        }
        let last = u32::try_from(self.config.pages.len().saturating_sub(1)).unwrap_or(0);
        let at = |page: u32| Some(LinkTarget::Page { page, y: None });
        Ok(vec![
            OutlineItem {
                title: "Chapter 1".to_owned(),
                target: at(0),
                children: vec![OutlineItem {
                    title: "Section 1.1".to_owned(),
                    target: at(1.min(last)),
                    children: Vec::new(),
                }],
            },
            OutlineItem {
                title: "Chapter 2".to_owned(),
                target: at(last),
                children: Vec::new(),
            },
        ])
    }

    fn close(&mut self) {
        self.open = false;
    }
}

/// The colour of pixel `(x, y)` of `page` rendered at `scale` and `rotation`: opaque, and a
/// different value for a different page, position, scale or rotation.
pub fn fake_pixel(page: u32, x: u32, y: u32, scale: f32, rotation: Rotation) -> [u8; 4] {
    // `as` saturates, which is fine for a test colour.
    let scale_thousandths = (scale * 1000.0).round() as u32;
    let mixed = x.wrapping_mul(73_856_093)
        ^ y.wrapping_mul(19_349_663)
        ^ page.wrapping_mul(83_492_791)
        ^ scale_thousandths.wrapping_mul(2_654_435_761)
        ^ u32::from(rotation.degrees()).wrapping_mul(40_503);
    [(mixed >> 16) as u8, (mixed >> 8) as u8, mixed as u8, 255]
}

/// The tile the fake backend renders for `request`.
pub fn expected_tile(request: &TileRequest) -> Tile {
    let mut rgba = Vec::with_capacity(request.width as usize * request.height as usize * 4);
    for row in 0..request.height {
        for column in 0..request.width {
            rgba.extend_from_slice(&fake_pixel(
                request.page,
                request.x + column,
                request.y + row,
                request.scale,
                request.rotation,
            ));
        }
    }
    Tile {
        width: request.width,
        height: request.height,
        rgba,
    }
}

/// The lines of text of page `page` (counted from 0).
pub fn fake_lines(page: u32) -> [String; 3] {
    [
        format!("Page {}: the quick brown fox", page + 1),
        "jumps over the lazy dog".to_owned(),
        format!("end of page {}", page + 1),
    ]
}

/// The text of page `page` with its boxes (see the module documentation).
pub fn fake_page_text(page: u32) -> PageText {
    let mut chars = Vec::new();
    for (line_index, line) in fake_lines(page).iter().enumerate() {
        if line_index > 0 {
            for generated in ['\r', '\n'] {
                chars.push(TextChar {
                    ch: generated,
                    bounds: Rect::default(),
                });
            }
        }
        let top = TEXT_MARGIN + line_index as f32 * LINE_PITCH;
        for (column, ch) in line.chars().enumerate() {
            let left = TEXT_MARGIN + column as f32 * CELL_WIDTH;
            chars.push(TextChar {
                ch,
                bounds: Rect::new(left, top, left + CELL_WIDTH, top + CELL_HEIGHT),
            });
        }
    }
    PageText { chars }
}

fn fold(ch: char, match_case: bool) -> char {
    if match_case {
        ch
    } else {
        ch.to_lowercase().next().unwrap_or(ch)
    }
}

/// Every non-overlapping match of `query` in `text`, with one rectangle per line it covers.
pub fn find_in_text(
    page: u32,
    text: &PageText,
    query: &str,
    options: FindOptions,
) -> Vec<SearchHit> {
    let wanted: Vec<char> = query
        .chars()
        .map(|ch| fold(ch, options.match_case))
        .collect();
    let mut hits = Vec::new();
    if wanted.is_empty() {
        return hits;
    }
    let haystack: Vec<char> = text
        .chars
        .iter()
        .map(|character| fold(character.ch, options.match_case))
        .collect();
    let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let mut start = 0;
    while start + wanted.len() <= haystack.len() && hits.len() < limits::MAX_HITS_PER_PAGE {
        let end = start + wanted.len();
        let mut matched = haystack[start..end] == wanted[..];
        if matched && options.whole_word {
            let before = start > 0 && is_word(haystack[start - 1]);
            let after = end < haystack.len() && is_word(haystack[end]);
            matched = !before && !after;
        }
        if !matched {
            start += 1;
            continue;
        }
        let mut rects: Vec<Rect> = Vec::new();
        for character in &text.chars[start..end] {
            if character.bounds.is_empty() {
                continue;
            }
            match rects.last_mut() {
                Some(last) if last.top == character.bounds.top => {
                    *last = last.union(&character.bounds);
                }
                _ => rects.push(character.bounds),
            }
        }
        hits.push(SearchHit {
            page,
            char_start: u32::try_from(start).unwrap_or(u32::MAX),
            char_len: u32::try_from(wanted.len()).unwrap_or(u32::MAX),
            rects,
        });
        start = end;
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened(config: FakeConfig) -> FakeBackend {
        let mut backend = FakeBackend::new(config);
        backend.open(Path::new("x.pdf"), None).expect("open");
        backend
    }

    fn tile_request(page: u32) -> TileRequest {
        TileRequest {
            page,
            scale: 1.0,
            rotation: Rotation::Deg0,
            x: 10,
            y: 20,
            width: 4,
            height: 3,
        }
    }

    #[test]
    fn a_tile_is_the_matching_rectangle_of_the_page_function() {
        let mut backend = opened(FakeConfig::default());
        let tile = backend.render(&tile_request(1)).unwrap();
        assert!(tile.is_consistent());
        assert_eq!((tile.width, tile.height), (4, 3));
        // Row 2, column 1 of the tile is page pixel (11, 22).
        let at = (2 * 4 + 1) * 4;
        assert_eq!(
            tile.rgba[at..at + 4],
            fake_pixel(1, 11, 22, 1.0, Rotation::Deg0)
        );
        assert!(
            tile.rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 255)
        );
    }

    #[test]
    fn the_pixels_depend_on_page_position_scale_and_rotation() {
        let base = fake_pixel(0, 5, 5, 1.0, Rotation::Deg0);
        assert_ne!(base, fake_pixel(1, 5, 5, 1.0, Rotation::Deg0));
        assert_ne!(base, fake_pixel(0, 6, 5, 1.0, Rotation::Deg0));
        assert_ne!(base, fake_pixel(0, 5, 6, 1.0, Rotation::Deg0));
        assert_ne!(base, fake_pixel(0, 5, 5, 2.0, Rotation::Deg0));
        assert_ne!(base, fake_pixel(0, 5, 5, 1.0, Rotation::Deg90));
        assert_eq!(base, fake_pixel(0, 5, 5, 1.0, Rotation::Deg0));
    }

    #[test]
    fn requests_that_break_a_rule_are_refused_not_panicked() {
        let mut backend = FakeBackend::default();
        assert_eq!(backend.render(&tile_request(0)), Err(PdfError::NoDocument));
        backend.open(Path::new("x.pdf"), None).unwrap();
        assert!(matches!(
            backend.render(&tile_request(9)),
            Err(PdfError::InvalidRequest(_))
        ));
        assert!(matches!(
            backend.page_text(9),
            Err(PdfError::InvalidRequest(_))
        ));
        backend.close();
        assert_eq!(backend.page_text(0), Err(PdfError::NoDocument));
    }

    #[test]
    fn the_text_has_real_boxes_and_generated_line_breaks() {
        let text = fake_page_text(0);
        assert_eq!(
            text.text(),
            "Page 1: the quick brown fox\r\njumps over the lazy dog\r\nend of page 1"
        );
        let first = text.chars[0].bounds;
        assert_eq!(first, Rect::new(36.0, 36.0, 42.0, 48.0));
        let generated = text.chars.iter().filter(|c| c.bounds.is_empty()).count();
        assert_eq!(generated, 4, "two line breaks of two characters");
        let second_line = text.text().find("jumps").unwrap();
        assert_eq!(text.chars[second_line].bounds.top, 36.0 + LINE_PITCH);
    }

    #[test]
    fn find_reports_ranges_and_one_rectangle_per_line() {
        let mut backend = opened(FakeConfig::default());
        let hits = backend
            .find_in_page(0, "FOX", FindOptions::default())
            .unwrap();
        assert_eq!(hits.len(), 1);
        let text = fake_page_text(0);
        let hit = &hits[0];
        assert_eq!(
            text.slice(hit.char_start as usize..(hit.char_start + hit.char_len) as usize),
            "fox"
        );
        assert_eq!(hit.rects.len(), 1);
        assert_eq!(hit.rects[0].width(), 3.0 * CELL_WIDTH);

        let exact = FindOptions {
            match_case: true,
            whole_word: false,
        };
        assert!(backend.find_in_page(0, "FOX", exact).unwrap().is_empty());
        let word = FindOptions {
            match_case: false,
            whole_word: true,
        };
        assert_eq!(backend.find_in_page(0, "he", word).unwrap().len(), 0);
        assert_eq!(backend.find_in_page(0, "the", word).unwrap().len(), 2);
        assert!(
            backend
                .find_in_page(0, "", FindOptions::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_match_across_a_line_break_has_a_rectangle_per_line() {
        let text = fake_page_text(0);
        let hits = find_in_text(0, &text, "fox\r\njumps", FindOptions::default());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rects.len(), 2, "{:?}", hits[0].rects);
    }

    #[test]
    fn links_and_outline_follow_the_page_count() {
        let mut backend = opened(FakeConfig::with_pages(2));
        assert_eq!(
            backend.links(0).unwrap()[0].target,
            LinkTarget::Page {
                page: 1,
                y: Some(0.0)
            }
        );
        assert_eq!(
            backend.links(1).unwrap()[0].target,
            LinkTarget::Uri("https://example.com/last".to_owned())
        );
        let outline = backend.outline().unwrap();
        assert_eq!(outline.len(), 2);
        assert_eq!(outline[0].children.len(), 1);
        assert_eq!(
            outline[1].target,
            Some(LinkTarget::Page { page: 1, y: None })
        );
    }

    #[test]
    fn the_switches_work() {
        let mut protected = FakeBackend::new(FakeConfig {
            password: Some("sesame".to_owned()),
            ..FakeConfig::default()
        });
        assert_eq!(
            protected.open(Path::new("x"), None),
            Err(PdfError::PasswordRequired)
        );
        assert_eq!(
            protected.open(Path::new("x"), Some("no")),
            Err(PdfError::PasswordIncorrect)
        );
        assert!(protected.open(Path::new("x"), Some("sesame")).is_ok());

        let mut corrupt = FakeBackend::new(FakeConfig {
            corrupt: true,
            ..FakeConfig::default()
        });
        assert!(matches!(
            corrupt.open(Path::new("x"), None),
            Err(PdfError::Corrupt(_))
        ));
    }

    #[test]
    #[should_panic(expected = "fake backend")]
    fn it_panics_after_the_configured_number_of_requests() {
        let mut backend = opened(FakeConfig {
            panic_after: Some(2),
            ..FakeConfig::default()
        });
        backend.page_text(0).unwrap();
        backend.page_text(1).unwrap();
        let _ = backend.page_text(2);
    }

    #[test]
    fn switches_parse_from_words_and_unknown_ones_are_named() {
        let config = FakeConfig::from_args([
            "--pages=5",
            "--password=pw",
            "--render-delay-ms=250",
            "--panic-after=3",
            "--corrupt",
            "--title=T",
        ])
        .unwrap();
        assert_eq!(config.pages.len(), 5);
        assert_eq!(config.password.as_deref(), Some("pw"));
        assert_eq!(config.render_delay, Duration::from_millis(250));
        assert_eq!(config.panic_after, Some(3));
        assert!(config.corrupt);
        assert_eq!(config.title.as_deref(), Some("T"));
        assert!(
            FakeConfig::from_args(["--nope"])
                .unwrap_err()
                .contains("--nope")
        );
        assert!(FakeConfig::from_args(["--pages=many"]).is_err());
    }
}
