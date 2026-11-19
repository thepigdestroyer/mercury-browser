// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The vocabulary every part of the PDF viewer shares: the helper process that runs PDFium, the
//! client that talks to it, the view model and the shell.
//!
//! **Page space** is PDF points (1/72 inch), with the origin at the top-left corner of the page
//! as the reader sees it (the page's own `/Rotate` is already applied) and y growing downward.
//! Rectangles are normalised: `left <= right` and `top <= bottom`.
//!
//! **Device pixels** are what tiles are made of: a page of `w × h` points at `scale` device
//! pixels per point is [`PageSize::pixel_size`] pixels, and a tile is one rectangle of that.
//!
//! Everything a PDF supplies (sizes, text, link targets, titles) is untrusted; [`limits`] says
//! how much of it Mercury accepts.

use std::ops::Range;

use crate::error::PdfError;

/// What Mercury accepts from a PDF and from a request. The helper process enforces them; the
/// client and the view model stay inside them, so a request that breaks one is a bug.
pub mod limits {
    /// The most pages a document may have; a larger file is refused as too large.
    pub const MAX_PAGES: u32 = 20_000;
    /// The longest side of one tile, in device pixels.
    pub const MAX_TILE_SIDE: u32 = 2_048;
    /// The most pixels in one tile (4 mega-pixels: 16 MiB of RGBA).
    pub const MAX_TILE_PIXELS: u32 = 4_194_304;
    /// The smallest and largest `scale` of a tile request, in device pixels per point.
    pub const MIN_SCALE: f32 = 0.05;
    /// See [`MIN_SCALE`].
    pub const MAX_SCALE: f32 = 16.0;
    /// The most characters of text read from one page; the rest is dropped.
    pub const MAX_TEXT_CHARS: usize = 1_000_000;
    /// The most links read from one page.
    pub const MAX_LINKS_PER_PAGE: usize = 5_000;
    /// The most entries and the deepest nesting of the outline (bookmarks).
    pub const MAX_OUTLINE_ITEMS: usize = 5_000;
    /// See [`MAX_OUTLINE_ITEMS`].
    pub const MAX_OUTLINE_DEPTH: usize = 16;
    /// The longest find query, in characters.
    pub const MAX_QUERY_CHARS: usize = 256;
    /// The most hits reported for one page.
    pub const MAX_HITS_PER_PAGE: usize = 1_000;
    /// The largest file the viewer opens, in bytes.
    pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
    /// The longest title, outline entry or link address kept, in bytes.
    pub const MAX_STRING_BYTES: usize = 4_096;
}

/// A rectangle in page space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    /// True for a rectangle with no area (or NaN sides): a space PDFium made up has one.
    pub fn is_empty(&self) -> bool {
        !(self.right > self.left && self.bottom > self.top)
    }

    /// The same rectangle with its sides in order.
    pub fn normalized(self) -> Self {
        Self {
            left: self.left.min(self.right),
            top: self.top.min(self.bottom),
            right: self.left.max(self.right),
            bottom: self.top.max(self.bottom),
        }
    }

    /// Whether the point is inside; the left and top edges count, the right and bottom do not.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    pub fn intersects(&self, other: &Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    /// The smallest rectangle holding both.
    pub fn union(&self, other: &Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

/// A quarter-turn rotation of a page, clockwise, on top of the page's own `/Rotate`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rotation {
    #[default]
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl Rotation {
    pub const fn degrees(self) -> u16 {
        match self {
            Self::Deg0 => 0,
            Self::Deg90 => 90,
            Self::Deg180 => 180,
            Self::Deg270 => 270,
        }
    }

    /// Any number of quarter turns, clockwise (negative: counter-clockwise).
    pub const fn from_quarter_turns(turns: i32) -> Self {
        match turns.rem_euclid(4) {
            1 => Self::Deg90,
            2 => Self::Deg180,
            3 => Self::Deg270,
            _ => Self::Deg0,
        }
    }

    pub const fn quarter_turns(self) -> i32 {
        (self.degrees() / 90) as i32
    }

    pub const fn turned_clockwise(self) -> Self {
        Self::from_quarter_turns(self.quarter_turns() + 1)
    }

    pub const fn turned_counter_clockwise(self) -> Self {
        Self::from_quarter_turns(self.quarter_turns() - 1)
    }

    /// Whether width and height swap places.
    pub const fn swaps_axes(self) -> bool {
        matches!(self, Self::Deg90 | Self::Deg270)
    }
}

/// The size of one page in points, with its own `/Rotate` applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

impl PageSize {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// The size once `rotation` is applied.
    pub const fn rotated(self, rotation: Rotation) -> Self {
        if rotation.swaps_axes() {
            Self::new(self.height, self.width)
        } else {
            self
        }
    }

    /// The whole page in device pixels at `scale` pixels per point, rotated by `rotation`: each
    /// side rounded up, at least one. This is the one formula the helper, the layout and the
    /// tile grid all use, so their tiles agree.
    pub fn pixel_size(self, scale: f32, rotation: Rotation) -> (u32, u32) {
        let rotated = self.rotated(rotation);
        // `as` saturates for out-of-range and NaN values, which is the intent here.
        let pixels = |points: f32| ((points * scale).ceil() as u32).max(1);
        (pixels(rotated.width), pixels(rotated.height))
    }
}

/// What the helper process learns when it opens a document.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentInfo {
    /// One entry per page, in order; never more than [`limits::MAX_PAGES`], never empty.
    pub pages: Vec<PageSize>,
    /// The document's title from its information dictionary, if it has a non-empty one.
    pub title: Option<String>,
}

impl DocumentInfo {
    pub fn page_count(&self) -> u32 {
        u32::try_from(self.pages.len()).unwrap_or(u32::MAX)
    }
}

/// One rectangle of one page to render.
///
/// The page is rendered at `scale` device pixels per point, rotated clockwise by `rotation` on
/// top of its own `/Rotate`; the whole page is then `PageSize::pixel_size(scale, rotation)`
/// pixels, and this tile is the `width × height` area whose top-left pixel is `(x, y)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileRequest {
    pub page: u32,
    pub scale: f32,
    pub rotation: Rotation,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl TileRequest {
    /// Checks the request against the document's pages and [`limits`]; the helper process calls
    /// it before it passes a request to the backend.
    pub fn validate(&self, pages: &[PageSize]) -> Result<(), PdfError> {
        let invalid = |reason: &str| Err(PdfError::InvalidRequest(reason.to_owned()));
        let Some(size) = usize::try_from(self.page)
            .ok()
            .and_then(|index| pages.get(index))
        else {
            return invalid("no such page");
        };
        if !(limits::MIN_SCALE..=limits::MAX_SCALE).contains(&self.scale) {
            return invalid("scale out of range");
        }
        if self.width == 0 || self.height == 0 {
            return invalid("empty tile");
        }
        if self.width > limits::MAX_TILE_SIDE
            || self.height > limits::MAX_TILE_SIDE
            || u64::from(self.width) * u64::from(self.height) > u64::from(limits::MAX_TILE_PIXELS)
        {
            return invalid("tile too large");
        }
        let (page_width, page_height) = size.pixel_size(self.scale, self.rotation);
        let inside = |origin: u32, length: u32, page: u32| {
            u64::from(origin) + u64::from(length) <= u64::from(page)
        };
        if !inside(self.x, self.width, page_width) || !inside(self.y, self.height, page_height) {
            return invalid("tile outside its page");
        }
        Ok(())
    }
}

/// A rendered tile: opaque RGBA8 (alpha is always 255), rows top to bottom with no padding, so
/// `rgba.len() == width * height * 4`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Tile {
    /// Whether the pixel buffer matches the size.
    pub fn is_consistent(&self) -> bool {
        u64::try_from(self.rgba.len())
            .is_ok_and(|length| length == u64::from(self.width) * u64::from(self.height) * 4)
    }
}

/// One character of a page's text with where it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextChar {
    pub ch: char,
    /// Where the character is on the page; empty for a space or line break that PDFium made up
    /// (it has no place of its own but belongs in copied text).
    pub bounds: Rect,
}

/// The text of one page in reading order. Indexes into `chars` are what selections and find
/// hits use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageText {
    pub chars: Vec<TextChar>,
}

impl PageText {
    /// The whole text.
    pub fn text(&self) -> String {
        self.chars.iter().map(|character| character.ch).collect()
    }

    /// The text of a range of indexes; indexes past the end are ignored.
    pub fn slice(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.chars.len());
        let start = range.start.min(end);
        self.chars[start..end]
            .iter()
            .map(|character| character.ch)
            .collect()
    }
}

/// How a find query is matched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub match_case: bool,
    pub whole_word: bool,
}

/// One place a find query matches.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub page: u32,
    /// Index into the page's [`PageText::chars`] where the match starts, and how many it covers.
    pub char_start: u32,
    pub char_len: u32,
    /// Where to highlight: one rectangle per line the match covers.
    pub rects: Vec<Rect>,
}

/// Where a link goes.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    /// An address outside the document, as the PDF wrote it (never opened without the user
    /// clicking, and only as an ordinary navigation).
    Uri(String),
    /// A place in the document: the page (counted from 0) and, if given, the y position on it
    /// in points.
    Page { page: u32, y: Option<f32> },
}

/// A clickable area of a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub rect: Rect,
    pub target: LinkTarget,
}

/// One entry of the document outline (bookmarks).
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineItem {
    pub title: String,
    pub target: Option<LinkTarget>,
    pub children: Vec<OutlineItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages() -> Vec<PageSize> {
        vec![PageSize::new(612.0, 792.0), PageSize::new(842.0, 595.0)]
    }

    fn request() -> TileRequest {
        TileRequest {
            page: 0,
            scale: 1.0,
            rotation: Rotation::Deg0,
            x: 0,
            y: 0,
            width: 512,
            height: 512,
        }
    }

    #[test]
    fn rectangles_have_edges_like_pixels() {
        let rect = Rect::new(10.0, 20.0, 30.0, 40.0);
        assert!(rect.contains(10.0, 20.0));
        assert!(!rect.contains(30.0, 40.0));
        assert!(rect.intersects(&Rect::new(29.0, 39.0, 50.0, 50.0)));
        assert!(!rect.intersects(&Rect::new(30.0, 20.0, 50.0, 40.0)));
        assert_eq!(
            rect.union(&Rect::new(0.0, 25.0, 15.0, 60.0)),
            Rect::new(0.0, 20.0, 30.0, 60.0)
        );
        assert!(Rect::new(1.0, 1.0, 1.0, 5.0).is_empty());
        assert!(Rect::new(f32::NAN, 0.0, 1.0, 1.0).is_empty());
        assert_eq!(
            Rect::new(5.0, 9.0, 1.0, 2.0).normalized(),
            Rect::new(1.0, 2.0, 5.0, 9.0)
        );
    }

    #[test]
    fn quarter_turns_wrap_in_both_directions() {
        assert_eq!(Rotation::from_quarter_turns(-1), Rotation::Deg270);
        assert_eq!(Rotation::from_quarter_turns(5), Rotation::Deg90);
        assert_eq!(Rotation::Deg270.turned_clockwise(), Rotation::Deg0);
        assert_eq!(Rotation::Deg0.turned_counter_clockwise(), Rotation::Deg270);
        assert!(Rotation::Deg90.swaps_axes());
        assert!(!Rotation::Deg180.swaps_axes());
    }

    #[test]
    fn a_page_is_its_size_rounded_up_and_swapped_by_a_quarter_turn() {
        let letter = PageSize::new(612.0, 792.0);
        assert_eq!(letter.pixel_size(1.5, Rotation::Deg0), (918, 1188));
        assert_eq!(letter.pixel_size(1.5, Rotation::Deg90), (1188, 918));
        assert_eq!(
            PageSize::new(0.1, 0.1).pixel_size(1.0, Rotation::Deg0),
            (1, 1)
        );
        assert_eq!(
            PageSize::new(100.2, 50.0).pixel_size(1.0, Rotation::Deg0),
            (101, 50)
        );
    }

    #[test]
    fn a_tile_inside_its_page_is_valid() {
        assert_eq!(request().validate(&pages()), Ok(()));
        let last = TileRequest {
            x: 100,
            y: 280,
            width: 512,
            height: 512,
            ..request()
        };
        assert_eq!(last.validate(&pages()), Ok(()));
    }

    #[test]
    fn a_request_that_breaks_a_rule_is_refused() {
        let broken = [
            TileRequest {
                page: 2,
                ..request()
            },
            TileRequest {
                scale: 0.0,
                ..request()
            },
            TileRequest {
                scale: f32::NAN,
                ..request()
            },
            TileRequest {
                scale: 99.0,
                ..request()
            },
            TileRequest {
                width: 0,
                ..request()
            },
            TileRequest {
                width: 4096,
                scale: 8.0,
                ..request()
            },
            TileRequest {
                x: 101,
                ..request()
            },
            TileRequest {
                y: 281,
                ..request()
            },
        ];
        for request in broken {
            assert!(
                matches!(request.validate(&pages()), Err(PdfError::InvalidRequest(_))),
                "{request:?}"
            );
        }
    }

    #[test]
    fn the_grid_of_a_rotated_page_swaps_its_sides() {
        let sideways = TileRequest {
            rotation: Rotation::Deg90,
            width: 512,
            height: 100,
            y: 512,
            ..request()
        };
        // A letter page turned a quarter is 792 × 612 pixels at scale 1.
        assert_eq!(sideways.validate(&pages()), Ok(()));
        assert!(
            TileRequest { x: 281, ..sideways }
                .validate(&pages())
                .is_err()
        );
    }

    #[test]
    fn a_tile_must_hold_exactly_its_pixels() {
        let tile = Tile {
            width: 2,
            height: 1,
            rgba: vec![0; 8],
        };
        assert!(tile.is_consistent());
        assert!(
            !Tile {
                rgba: vec![0; 7],
                ..tile
            }
            .is_consistent()
        );
    }

    #[test]
    fn page_text_slices_stay_inside_the_page() {
        let text = PageText {
            chars: "héllo"
                .chars()
                .map(|ch| TextChar {
                    ch,
                    bounds: Rect::default(),
                })
                .collect(),
        };
        assert_eq!(text.text(), "héllo");
        assert_eq!(text.slice(1..3), "él");
        assert_eq!(text.slice(3..99), "lo");
        assert_eq!(text.slice(9..12), "");
    }
}
