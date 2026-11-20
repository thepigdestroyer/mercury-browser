// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! [`PdfiumBackend`]: the [`PdfBackend`] of the PDF helper process, on PDFium through
//! `pdfium-render`.
//!
//! - **One library, one thread.** The library is loaded once per process ([`PdfiumBackend::load`]
//!   keeps it for the life of the process) and every call comes from the helper's one backend
//!   thread. A backend holds at most one document, opened **by path** (PDFium reads what it needs
//!   from the file; a 300 MB file is not read into memory).
//! - **Tiles.** A page is drawn at `scale` device pixels per point into a tile-sized bitmap with
//!   the page's origin moved to `(-x, -y)`, form fields and annotations on, on opaque white; PDFium
//!   writes BGRx and the helper turns it into RGBA with alpha 255, once.
//! - **Geometry.** Characters, links and search rectangles come from PDFium in user space and
//!   are converted by [`PageGeometry`] to the viewer's page space (`/Rotate`, crop box, y down).
//! - **Text.** `chars()` with loose boxes (so the boxes of a line touch and a selection has no
//!   holes); a character PDFium made up (a space or a line break it inserted) has an empty box.
//!   The index of a character is PDFium's own, which is what a search hit's range counts.
//! - **Find.** `PdfPageText::search`. PDFium gives the rectangles of a hit but not the range of
//!   characters, so the range is found by looking for the characters whose boxes the rectangles
//!   cover (see [`char_range`]).
//! - **Limits.** [`limits`] are enforced here as well as in the request loop: file size, page
//!   count, text length, links, outline size and depth, hit count.
//! - **Pages** are cached (the last few), because a tile, the text and the links of one page
//!   are asked for one after the other, and parsing a page is the expensive part.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use mercury_pdf::backend::PdfBackend;
use mercury_pdf::error::PdfError;
use mercury_pdf::types::{
    DocumentInfo, FindOptions, Link, LinkTarget, OutlineItem, PageSize, PageText, Rect, Rotation,
    SearchHit, TextChar, Tile, TileRequest, limits,
};
use pdfium_render::prelude::*;

use crate::geometry::PageGeometry;

/// How many parsed pages are kept.
const CACHED_PAGES: usize = 4;
/// The smallest and largest side of a page, in points; PDFium would report anything.
const MIN_PAGE_SIDE: f32 = 1.0;
const MAX_PAGE_SIDE: f32 = 100_000.0;
/// The size of a page whose size makes no sense (US Letter).
const FALLBACK_PAGE: PageSize = PageSize::new(612.0, 792.0);

/// The library, loaded once for the whole process: `Pdfium` is a handle to global state.
static PDFIUM: Mutex<Option<(PathBuf, &'static Pdfium)>> = Mutex::new(None);

fn load_library(library: &Path) -> Result<&'static Pdfium, PdfError> {
    if !library.is_file() {
        return Err(PdfError::LibraryMissing(format!(
            "{} is not a file",
            library.display()
        )));
    }
    let mut slot = PDFIUM.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((loaded, pdfium)) = slot.as_ref() {
        if loaded != library {
            log::debug!(
                "pdfium: {} is already loaded; ignoring {}",
                loaded.display(),
                library.display()
            );
        }
        return Ok(pdfium);
    }
    let bindings = Pdfium::bind_to_library(library).map_err(|error| {
        PdfError::LibraryMissing(format!(
            "{} could not be loaded: {error}",
            library.display()
        ))
    })?;
    // One `Pdfium` for the life of the process: it owns nothing that needs to be freed earlier,
    // and documents borrow it.
    let pdfium: &'static Pdfium = Box::leak(Box::new(Pdfium::new(bindings)));
    *slot = Some((library.to_path_buf(), pdfium));
    Ok(pdfium)
}

/// A parsed page with its geometry.
struct CachedPage {
    index: u32,
    page: PdfPage<'static>,
    geometry: PageGeometry,
}

/// The open document. Field order matters: the pages are dropped before the document.
struct Loaded {
    cache: Vec<CachedPage>,
    document: PdfDocument<'static>,
    sizes: Vec<PageSize>,
}

impl Loaded {
    /// The page, parsed now or taken from the cache.
    fn page(&mut self, index: u32) -> Result<&CachedPage, PdfError> {
        if (index as usize) >= self.sizes.len() {
            return Err(PdfError::InvalidRequest("no such page".to_owned()));
        }
        if let Some(position) = self.cache.iter().position(|entry| entry.index == index) {
            let entry = self.cache.remove(position);
            self.cache.push(entry);
        } else {
            let number = PdfPageIndex::try_from(index)
                .map_err(|_| PdfError::InvalidRequest("no such page".to_owned()))?;
            let page = self
                .document
                .pages()
                .get(number)
                .map_err(|error| page_error(&error))?;
            let geometry = geometry_of(&page)?;
            if self.cache.len() >= CACHED_PAGES {
                self.cache.remove(0);
            }
            self.cache.push(CachedPage {
                index,
                page,
                geometry,
            });
        }
        self.cache
            .last()
            .ok_or_else(|| PdfError::InvalidRequest("no such page".to_owned()))
    }
}

fn geometry_of(page: &PdfPage<'static>) -> Result<PageGeometry, PdfError> {
    let bounds = page
        .boundaries()
        .bounding()
        .map_err(|error| page_error(&error))?
        .bounds;
    let rotate = page.rotation().map_or(0, |rotation| match rotation {
        PdfPageRenderRotation::None => 0,
        PdfPageRenderRotation::Degrees90 => 90,
        PdfPageRenderRotation::Degrees180 => 180,
        PdfPageRenderRotation::Degrees270 => 270,
    });
    Ok(PageGeometry::new(
        [
            bounds.left().value,
            bounds.bottom().value,
            bounds.right().value,
            bounds.top().value,
        ],
        rotate,
    ))
}

fn page_error(error: &PdfiumError) -> PdfError {
    PdfError::Corrupt(format!("a page could not be read: {error}"))
}

fn load_error(error: PdfiumError, password_given: bool) -> PdfError {
    match error {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
            if password_given {
                PdfError::PasswordIncorrect
            } else {
                PdfError::PasswordRequired
            }
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FormatError) => {
            PdfError::Corrupt("not a PDF file, or damaged".to_owned())
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FileError) => {
            PdfError::Io("the file could not be read".to_owned())
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::SecurityError) => {
            PdfError::Unsupported("an unsupported security handler".to_owned())
        }
        PdfiumError::IoError(error) => PdfError::Io(error.to_string()),
        other => PdfError::Corrupt(format!("the file could not be opened: {other}")),
    }
}

/// The page sizes PDFium reports, made safe: a side that is not a finite number is the US Letter
/// side, and every side is kept between 1 and 100,000 points.
fn safe_size(width: f32, height: f32) -> PageSize {
    let side = |value: f32, fallback: f32| {
        if value.is_finite() && value > 0.0 {
            value.clamp(MIN_PAGE_SIDE, MAX_PAGE_SIDE)
        } else {
            fallback
        }
    };
    PageSize::new(
        side(width, FALLBACK_PAGE.width),
        side(height, FALLBACK_PAGE.height),
    )
}

/// The PDFium backend. See the module documentation.
pub struct PdfiumBackend {
    pdfium: &'static Pdfium,
    loaded: Option<Loaded>,
}

impl PdfiumBackend {
    /// Loads the PDFium library at `library` (once per process; later calls reuse it).
    pub fn load(library: &Path) -> Result<Self, PdfError> {
        Ok(Self {
            pdfium: load_library(library)?,
            loaded: None,
        })
    }

    fn loaded(&mut self) -> Result<&mut Loaded, PdfError> {
        self.loaded.as_mut().ok_or(PdfError::NoDocument)
    }
}

impl PdfBackend for PdfiumBackend {
    fn open(&mut self, path: &Path, password: Option<&str>) -> Result<DocumentInfo, PdfError> {
        self.close();
        let metadata = std::fs::metadata(path)
            .map_err(|error| PdfError::Io(format!("the file cannot be read: {error}")))?;
        if !metadata.is_file() {
            return Err(PdfError::Io("not a regular file".to_owned()));
        }
        if metadata.len() > limits::MAX_FILE_BYTES {
            return Err(PdfError::TooLarge(format!("{} bytes", metadata.len())));
        }
        let document = self
            .pdfium
            .load_pdf_from_file(path, password)
            .map_err(|error| load_error(error, password.is_some()))?;
        let count = usize::try_from(document.pages().len()).unwrap_or(0);
        if count == 0 {
            return Err(PdfError::Corrupt("the document has no pages".to_owned()));
        }
        if count > limits::MAX_PAGES as usize {
            return Err(PdfError::TooLarge(format!("{count} pages")));
        }
        let mut sizes = Vec::with_capacity(count);
        for index in 0..document.pages().len() {
            let rect = document
                .pages()
                .page_size(index)
                .map_err(|error| page_error(&error))?;
            sizes.push(safe_size(rect.width().value, rect.height().value));
        }
        let title = document
            .metadata()
            .get(PdfDocumentMetadataTagType::Title)
            .map(|tag| tag.value().to_owned())
            .filter(|title| !title.trim().is_empty());
        let info = DocumentInfo {
            pages: sizes.clone(),
            title,
        };
        self.loaded = Some(Loaded {
            cache: Vec::new(),
            document,
            sizes,
        });
        Ok(info)
    }

    fn render(&mut self, request: &TileRequest) -> Result<Tile, PdfError> {
        let loaded = self.loaded()?;
        request.validate(&loaded.sizes)?;
        let size = loaded.sizes[request.page as usize];
        let cached = loaded.page(request.page)?;
        let (page_width, page_height) = size.pixel_size(request.scale, request.rotation);
        let to_pixels =
            |value: u32| i32::try_from(value).map_err(|_| PdfError::TooLarge("tile".to_owned()));
        let mut config = PdfRenderConfig::new()
            .set_fixed_size(to_pixels(page_width)?, to_pixels(page_height)?)
            .set_origin(-to_pixels(request.x)?, -to_pixels(request.y)?)
            .set_format(PdfBitmapFormat::BGRx)
            .set_reverse_byte_order(false)
            .clear_before_rendering(true)
            .set_clear_color(PdfColor::WHITE)
            .render_form_data(true)
            .render_annotations(true);
        let turn = match request.rotation {
            Rotation::Deg0 => None,
            Rotation::Deg90 => Some(PdfPageRenderRotation::Degrees90),
            Rotation::Deg180 => Some(PdfPageRenderRotation::Degrees180),
            Rotation::Deg270 => Some(PdfPageRenderRotation::Degrees270),
        };
        if let Some(turn) = turn {
            config = config.rotate(turn, false);
        }
        let bytes = request.width as usize * request.height as usize * 4;
        let mut pixels = vec![0_u8; bytes];
        {
            let mut bitmap = PdfBitmap::from_bytes(
                to_pixels(request.width)?,
                to_pixels(request.height)?,
                PdfBitmapFormat::BGRx,
                &mut pixels,
            )
            .map_err(|error| PdfError::Corrupt(format!("no bitmap: {error}")))?;
            cached
                .page
                .render_into_bitmap_with_config(&mut bitmap, &config)
                .map_err(|error| PdfError::Corrupt(format!("the page did not render: {error}")))?;
        }
        // BGRx to RGBA, opaque.
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        Ok(Tile {
            width: request.width,
            height: request.height,
            rgba: pixels,
        })
    }

    fn page_text(&mut self, page: u32) -> Result<PageText, PdfError> {
        let cached = self.loaded()?.page(page)?;
        let text = cached
            .page
            .text()
            .map_err(|error| PdfError::Corrupt(format!("the text could not be read: {error}")))?;
        Ok(read_text(&text, &cached.geometry))
    }

    fn find_in_page(
        &mut self,
        page: u32,
        query: &str,
        options: FindOptions,
    ) -> Result<Vec<SearchHit>, PdfError> {
        let cached = self.loaded()?.page(page)?;
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let text = cached
            .page
            .text()
            .map_err(|error| PdfError::Corrupt(format!("the text could not be read: {error}")))?;
        let search_options = PdfSearchOptions::new()
            .match_case(options.match_case)
            .match_whole_word(options.whole_word);
        let search = text
            .search(query, &search_options)
            .map_err(|error| PdfError::Corrupt(format!("the search failed: {error}")))?;
        // The characters are read once, to turn each hit's rectangles into a character range.
        let characters = read_text(&text, &cached.geometry);
        let mut cursor = 0;
        let mut ranges_work = true;
        let mut hits = Vec::new();
        for segments in search.iter(PdfSearchDirection::SearchForward) {
            if hits.len() >= limits::MAX_HITS_PER_PAGE {
                break;
            }
            let mut rects = Vec::new();
            for index in 0..segments.len() {
                if let Ok(segment) = segments.get(index) {
                    let bounds = segment.bounds();
                    rects.push(cached.geometry.rect(
                        bounds.left().value,
                        bounds.bottom().value,
                        bounds.right().value,
                        bounds.top().value,
                    ));
                }
            }
            rects.retain(|rect| !rect.is_empty());
            if rects.is_empty() {
                continue;
            }
            let (char_start, char_len) = if ranges_work {
                match char_range(&characters, &rects, cursor) {
                    Some((start, len)) => {
                        cursor = start + len;
                        (start, len)
                    }
                    None => {
                        // Do not scan the page again for every later hit.
                        ranges_work = false;
                        (0, 0)
                    }
                }
            } else {
                (0, 0)
            };
            hits.push(SearchHit {
                page,
                char_start: u32::try_from(char_start).unwrap_or(u32::MAX),
                char_len: u32::try_from(char_len).unwrap_or(u32::MAX),
                rects,
            });
        }
        Ok(hits)
    }

    fn links(&mut self, page: u32) -> Result<Vec<Link>, PdfError> {
        // First, what the page says, with the page borrowed; then the y positions of the
        // targets, which may need other pages.
        let mut raw: Vec<(Rect, RawTarget)> = Vec::new();
        {
            let page_count = self.loaded()?.sizes.len();
            let cached = self.loaded()?.page(page)?;
            for link in cached.page.links().iter() {
                if raw.len() >= limits::MAX_LINKS_PER_PAGE {
                    break;
                }
                let Ok(rect) = link.rect() else { continue };
                let rect = cached.geometry.rect(
                    rect.left().value,
                    rect.bottom().value,
                    rect.right().value,
                    rect.top().value,
                );
                if rect.is_empty() {
                    continue;
                }
                let target = match link.action() {
                    Some(PdfAction::Uri(action)) => action.uri().ok().map(RawTarget::Uri),
                    Some(PdfAction::LocalDestination(action)) => action
                        .destination()
                        .ok()
                        .and_then(|destination| raw_destination(&destination, page_count)),
                    _ => link
                        .destination()
                        .and_then(|destination| raw_destination(&destination, page_count)),
                };
                if let Some(target) = target {
                    raw.push((rect, target));
                }
            }
        }
        let mut links = Vec::with_capacity(raw.len());
        for (rect, target) in raw {
            let target = match target {
                RawTarget::Uri(uri) => LinkTarget::Uri(uri),
                RawTarget::Page { index, view } => {
                    let y = match view {
                        Some((x, y)) => self.loaded()?.page(index).ok().map(|target_page| {
                            let x = x.unwrap_or(target_page.geometry.left);
                            target_page.geometry.point(x, y).1
                        }),
                        None => None,
                    };
                    LinkTarget::Page { page: index, y }
                }
            };
            links.push(Link { rect, target });
        }
        Ok(links)
    }

    fn outline(&mut self) -> Result<Vec<OutlineItem>, PdfError> {
        let loaded = self.loaded()?;
        let page_count = loaded.sizes.len();
        let mut budget = limits::MAX_OUTLINE_ITEMS;
        Ok(outline_level(
            loaded.document.bookmarks().root(),
            page_count,
            &mut budget,
            1,
        ))
    }

    fn close(&mut self) {
        // Pages first, then the document (see `Loaded`).
        self.loaded = None;
    }
}

/// Reads every character of a page's text: its character, and its box in page space (empty for a
/// character PDFium made up).
fn read_text(text: &PdfPageText<'_>, geometry: &PageGeometry) -> PageText {
    let total = usize::try_from(text.len())
        .unwrap_or(0)
        .min(limits::MAX_TEXT_CHARS);
    let characters = text.chars();
    let mut chars = Vec::with_capacity(total);
    for index in 0..total {
        let Ok(character) = characters.get(index) else {
            chars.push(TextChar {
                ch: '\u{FFFD}',
                bounds: Rect::default(),
            });
            continue;
        };
        let ch = match char::from_u32(character.unicode_value()) {
            Some('\0') => ' ',
            Some(ch) => ch,
            None => '\u{FFFD}',
        };
        let generated = character.is_generated().unwrap_or(false);
        let bounds = if generated {
            Rect::default()
        } else {
            character
                .loose_bounds()
                .map(|bounds| {
                    geometry.rect(
                        bounds.left().value,
                        bounds.bottom().value,
                        bounds.right().value,
                        bounds.top().value,
                    )
                })
                .unwrap_or_default()
        };
        chars.push(TextChar { ch, bounds });
    }
    PageText { chars }
}

/// The range of characters (start, count) that a search hit's rectangles cover, looking from
/// `from` on (hits come in order, so the next hit starts after the last).
///
/// A character belongs to the hit when its box, horizontally, has its centre inside one of the
/// rectangles and, vertically, contains the middle of that rectangle: a hit made of full stops has
/// a very flat rectangle that a loose character box still contains, and the line above or below,
/// which has its own boxes, does not. Characters without a box (made-up spaces and line breaks)
/// between two that belong to the hit belong to it. Returns `None` when no character matches.
pub fn char_range(text: &PageText, rects: &[Rect], from: usize) -> Option<(usize, usize)> {
    let mut first = None;
    let mut last = None;
    for (index, character) in text.chars.iter().enumerate().skip(from) {
        let bounds = &character.bounds;
        if bounds.is_empty() {
            continue;
        }
        let centre_x = (bounds.left + bounds.right) / 2.0;
        let inside = rects.iter().any(|rect| {
            let middle_y = (rect.top + rect.bottom) / 2.0;
            centre_x >= rect.left
                && centre_x < rect.right
                && middle_y >= bounds.top
                && middle_y <= bounds.bottom
        });
        if inside {
            first.get_or_insert(index);
            last = Some(index);
        } else if first.is_some() {
            // The hit is over once a character with a box falls outside it.
            break;
        }
    }
    match (first, last) {
        (Some(first), Some(last)) => Some((first, last - first + 1)),
        _ => None,
    }
}

/// Where a destination points on its page, in that page's user space: the x (if it names one)
/// and the y.
type View = (Option<f32>, f32);

/// A link target as PDFium states it, before the position on the target page is resolved: an
/// address, or a page with (if the destination names one) a position on it.
enum RawTarget {
    Uri(String),
    Page { index: u32, view: Option<View> },
}

/// A destination as the page it leads to and, if it names a position, where (the x and the y in
/// the target page's user space).
fn raw_destination_parts(destination: &PdfDestination<'_>) -> Option<(u32, Option<View>)> {
    let index = u32::try_from(destination.page_index().ok()?).ok()?;
    let view = match destination.view_settings().ok()? {
        PdfDestinationViewSettings::SpecificCoordinatesAndZoom(x, Some(y), _) => {
            Some((x.map(|x| x.value), y.value))
        }
        PdfDestinationViewSettings::FitPageHorizontallyToWindow(Some(y))
        | PdfDestinationViewSettings::FitBoundsHorizontallyToWindow(Some(y)) => {
            Some((None, y.value))
        }
        PdfDestinationViewSettings::FitPageToRectangle(rect) => {
            Some((Some(rect.left().value), rect.top().value))
        }
        _ => None,
    };
    Some((
        index,
        view.filter(|(x, y)| y.is_finite() && x.is_none_or(f32::is_finite)),
    ))
}

fn raw_destination(destination: &PdfDestination<'_>, page_count: usize) -> Option<RawTarget> {
    let (index, view) = raw_destination_parts(destination)?;
    ((index as usize) < page_count).then_some(RawTarget::Page { index, view })
}

/// The bookmarks of one level and, below each, its children.
fn outline_level(
    first: Option<PdfBookmark<'_>>,
    page_count: usize,
    budget: &mut usize,
    depth: usize,
) -> Vec<OutlineItem> {
    let mut items = Vec::new();
    let mut next = first;
    while let Some(bookmark) = next {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let target = bookmark_target(&bookmark, page_count);
        let children = if depth < limits::MAX_OUTLINE_DEPTH {
            outline_level(bookmark.first_child(), page_count, budget, depth + 1)
        } else {
            Vec::new()
        };
        items.push(OutlineItem {
            title: bookmark.title().unwrap_or_default(),
            target,
            children,
        });
        next = bookmark.next_sibling();
    }
    items
}

fn bookmark_target(bookmark: &PdfBookmark<'_>, page_count: usize) -> Option<LinkTarget> {
    let page_target = |destination: &PdfDestination<'_>| {
        let (index, _) = raw_destination_parts(destination)?;
        // Where on the page is left out: it would need every target page parsed.
        ((index as usize) < page_count).then_some(LinkTarget::Page {
            page: index,
            y: None,
        })
    };
    if let Some(destination) = bookmark.destination() {
        return page_target(&destination);
    }
    match bookmark.action()? {
        PdfAction::LocalDestination(action) => action
            .destination()
            .ok()
            .and_then(|destination| page_target(&destination)),
        PdfAction::Uri(action) => action.uri().ok().map(LinkTarget::Uri),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(boxes: &[(char, [f32; 4])]) -> PageText {
        PageText {
            chars: boxes
                .iter()
                .map(|(ch, [left, top, right, bottom])| TextChar {
                    ch: *ch,
                    bounds: Rect::new(*left, *top, *right, *bottom),
                })
                .collect(),
        }
    }

    #[test]
    fn page_sizes_that_make_no_sense_are_replaced_or_clamped() {
        assert_eq!(safe_size(200.0, 300.0), PageSize::new(200.0, 300.0));
        assert_eq!(safe_size(f32::NAN, -5.0), FALLBACK_PAGE);
        assert_eq!(safe_size(0.0, f32::INFINITY), FALLBACK_PAGE);
        assert_eq!(safe_size(0.2, 9.0e9), PageSize::new(1.0, 100_000.0));
    }

    #[test]
    fn the_range_of_a_hit_is_the_characters_its_rectangle_covers() {
        // "ab fox cd": boxes 6 wide, 12 tall.
        let line: Vec<(char, [f32; 4])> = "ab fox cd"
            .chars()
            .enumerate()
            .map(|(index, ch)| {
                let left = 10.0 + index as f32 * 6.0;
                (ch, [left, 20.0, left + 6.0, 32.0])
            })
            .collect();
        let page = text(&line);
        let rect = Rect::new(10.0 + 3.0 * 6.0, 22.0, 10.0 + 6.0 * 6.0, 30.0);
        assert_eq!(char_range(&page, &[rect], 0), Some((3, 3)));
        // Starting after the hit finds nothing.
        assert_eq!(char_range(&page, &[rect], 6), None);
    }

    #[test]
    fn a_flat_rectangle_still_finds_its_characters_and_the_next_line_is_not_taken() {
        // Two lines with loose boxes 12 tall, 14 apart; the hit "..." is a flat rectangle at the
        // baseline of the first.
        let mut boxes = Vec::new();
        for (line, top) in [(0, 20.0), (1, 34.0)] {
            for column in 0..3 {
                let left = 10.0 + column as f32 * 6.0;
                boxes.push(('.', [left, top, left + 6.0, top + 12.0]));
            }
            let _ = line;
        }
        let page = text(&boxes);
        let flat = Rect::new(10.0, 29.0, 28.0, 30.0);
        assert_eq!(char_range(&page, &[flat], 0), Some((0, 3)));
    }

    #[test]
    fn a_hit_over_two_lines_includes_the_breaks_between() {
        let boxes = [
            ('a', [10.0, 20.0, 16.0, 32.0]),
            ('\r', [0.0, 0.0, 0.0, 0.0]),
            ('\n', [0.0, 0.0, 0.0, 0.0]),
            ('b', [10.0, 40.0, 16.0, 52.0]),
            ('c', [16.0, 40.0, 22.0, 52.0]),
        ];
        let page = text(&boxes);
        let first = Rect::new(10.0, 22.0, 16.0, 30.0);
        let second = Rect::new(10.0, 42.0, 16.0, 50.0);
        assert_eq!(char_range(&page, &[first, second], 0), Some((0, 4)));
    }
}
