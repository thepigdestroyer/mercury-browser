// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! From the view model to what the window shows: the rows of pages, tiles and highlights of
//! `PdfViewerModel` (the Slint global in `ui/pdf/pdf-model.slint`).
//!
//! Only what changed is written, so that scrolling does not rebuild the page area: the scroll
//! position is written every time (a change of one number), the rows only when the set of visible
//! tiles, pages or highlights changed. A tile's Slint image is made once per rendered tile and
//! kept while the tile shows.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use mercury_pdf::types::Tile;
use mercury_pdf::viewer::{PlacedTile, ViewerModel};
use mercury_ui::{PdfFitMode, PdfPageRow, PdfRectRow, PdfTileRow, PdfViewState, PdfViewerModel};
use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

/// A tile's identity while it shows: its page, where its pixels live (the model hands out the
/// same `Arc` for as long as it caches the tile) and whether it is the page's placeholder.
type ImageKey = (u32, usize, bool);

/// Slint images of the tiles on show.
#[derive(Default)]
pub(super) struct TileImages {
    // The `Arc` stays in the value so that its address cannot be reused while the entry lives.
    images: HashMap<ImageKey, (Arc<Tile>, Image)>,
}

impl TileImages {
    /// The rows for `tiles`, in order. Images of tiles no longer shown are dropped.
    pub(super) fn rows(&mut self, tiles: &[PlacedTile]) -> Vec<PdfTileRow> {
        let mut used = HashSet::new();
        let mut rows = Vec::with_capacity(tiles.len());
        for placed in tiles {
            let key = key_of(placed);
            used.insert(key);
            let image = match self.images.get(&key) {
                Some((_, image)) => image.clone(),
                None => {
                    let Some(image) = image_of(&placed.pixels) else {
                        continue;
                    };
                    self.images
                        .insert(key, (Arc::clone(&placed.pixels), image.clone()));
                    image
                }
            };
            rows.push(PdfTileRow {
                x: placed.x,
                y: placed.y,
                width: placed.width,
                height: placed.height,
                image,
            });
        }
        self.images.retain(|key, _| used.contains(key));
        rows
    }

    /// How many images are kept.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.images.len()
    }
}

fn key_of(placed: &PlacedTile) -> ImageKey {
    (
        placed.page,
        Arc::as_ptr(&placed.pixels) as usize,
        placed.is_placeholder,
    )
}

/// The Slint image of a rendered tile; `None` for a tile whose buffer does not match its size.
pub(super) fn image_of(tile: &Tile) -> Option<Image> {
    if tile.width == 0 || tile.height == 0 || !tile.is_consistent() {
        return None;
    }
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(tile.width, tile.height);
    buffer.make_mut_bytes().copy_from_slice(&tile.rgba);
    Some(Image::from_rgba8(buffer))
}

/// What the toolbar and the panels show besides the pages.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Facts {
    pub(super) state: ViewState,
    pub(super) title: String,
    /// The error state's sentence, or the password prompt's error line.
    pub(super) message: String,
    pub(super) zoom_percent: u16,
    pub(super) fit: FitButton,
    pub(super) can_save_copy: bool,
    pub(super) can_open_externally: bool,
    pub(super) find_open: bool,
    pub(super) over_link: bool,
}

/// The viewer's state, apart from Slint's enum so that tests need no window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewState {
    Loading,
    Password,
    Error,
    Ready,
}

/// Which "fit" button is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FitButton {
    None,
    Width,
    Page,
}

/// The rows last written to the window, so that unchanged rows are not written again.
#[derive(Default)]
pub(super) struct ViewCache {
    last_state: Option<ViewState>,
    images: TileImages,
    tile_signature: Vec<(ImageKey, [u32; 4])>,
    pages: Vec<PdfPageRow>,
    selection: Vec<PdfRectRow>,
    hits: Vec<PdfRectRow>,
}

/// Where the pages are and what the page indicator says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Content {
    pub(super) page: u32,
    pub(super) page_count: u32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) scroll_x: f32,
    pub(super) scroll_y: f32,
}

/// What to write to the window. Made while the document's state is borrowed, written after the
/// borrow is released: a Slint setter may run callbacks that borrow it again.
pub(super) struct Update {
    pub(super) facts: Facts,
    /// The state differs from the one written last: the panel or toolbar before it is gone, and
    /// whatever field of it had the keyboard cannot say so.
    pub(super) state_changed: bool,
    /// `None`: no pages are showing and the page area is empty.
    pub(super) content: Option<Content>,
    /// Each is `Some` only when it differs from what was written last.
    pub(super) pages: Option<Vec<PdfPageRow>>,
    pub(super) tiles: Option<Vec<PdfTileRow>>,
    pub(super) selection: Option<Vec<PdfRectRow>>,
    pub(super) hits: Option<Vec<PdfRectRow>>,
}

impl ViewCache {
    /// What the window should show for `model` (shown only in the ready state), against what it
    /// shows now.
    pub(super) fn prepare(&mut self, facts: Facts, model: Option<&ViewerModel>) -> Update {
        let state_changed = self.last_state.replace(facts.state) != Some(facts.state);
        let Some(model) = model.filter(|_| facts.state == ViewState::Ready) else {
            let cleared = self.clear();
            return Update {
                facts,
                state_changed,
                content: None,
                pages: cleared.then(Vec::new),
                tiles: cleared.then(Vec::new),
                selection: cleared.then(Vec::new),
                hits: cleared.then(Vec::new),
            };
        };
        let (width, height) = model.content_size();
        let content = Content {
            page: model.current_page() + 1,
            page_count: model.page_count(),
            width,
            height,
            scroll_x: model.scroll_x(),
            scroll_y: model.scroll_y(),
        };

        let pages = model
            .page_rects()
            .into_iter()
            .map(|(_, x, y, width, height)| PdfPageRow {
                x,
                y,
                width,
                height,
            })
            .collect::<Vec<_>>();
        let pages = (pages != self.pages).then(|| {
            self.pages.clone_from(&pages);
            pages
        });

        let placed = model.visible_tiles();
        let signature = placed
            .iter()
            .map(|tile| {
                (
                    key_of(tile),
                    [
                        tile.x.to_bits(),
                        tile.y.to_bits(),
                        tile.width.to_bits(),
                        tile.height.to_bits(),
                    ],
                )
            })
            .collect::<Vec<_>>();
        let tiles = (signature != self.tile_signature).then(|| {
            self.tile_signature = signature;
            self.images.rows(&placed)
        });

        let selection = model
            .selection_rects()
            .into_iter()
            .map(|(x, y, width, height)| rect_row(x, y, width, height, false))
            .collect::<Vec<_>>();
        let selection = (selection != self.selection).then(|| {
            self.selection.clone_from(&selection);
            selection
        });

        let hits = model
            .find_rects()
            .into_iter()
            .map(|(x, y, width, height, current)| rect_row(x, y, width, height, current))
            .collect::<Vec<_>>();
        let hits = (hits != self.hits).then(|| {
            self.hits.clone_from(&hits);
            hits
        });

        Update {
            facts,
            state_changed,
            content: Some(content),
            pages,
            tiles,
            selection,
            hits,
        }
    }

    /// Forgets what was written; true if there was something to empty.
    fn clear(&mut self) -> bool {
        let had_rows = !(self.pages.is_empty()
            && self.tile_signature.is_empty()
            && self.selection.is_empty()
            && self.hits.is_empty());
        self.images = TileImages::default();
        self.tile_signature.clear();
        self.pages.clear();
        self.selection.clear();
        self.hits.clear();
        had_rows
    }
}

impl Update {
    /// Writes the update to the window's viewer.
    pub(super) fn write(self, global: &PdfViewerModel<'_>) {
        let facts = &self.facts;
        if self.state_changed {
            global.set_text_focused(false);
        }
        global.set_state(match facts.state {
            ViewState::Loading => PdfViewState::Loading,
            ViewState::Password => PdfViewState::Password,
            ViewState::Error => PdfViewState::Error,
            ViewState::Ready => PdfViewState::Ready,
        });
        global.set_title(facts.title.as_str().into());
        global.set_message(facts.message.as_str().into());
        global.set_zoom_percent(i32::from(facts.zoom_percent));
        global.set_fit(match facts.fit {
            FitButton::None => PdfFitMode::None,
            FitButton::Width => PdfFitMode::Width,
            FitButton::Page => PdfFitMode::Page,
        });
        global.set_can_zoom_in(facts.zoom_percent < ZOOM_MAX);
        global.set_can_zoom_out(facts.zoom_percent > ZOOM_MIN);
        global.set_can_save_copy(facts.can_save_copy);
        global.set_can_open_externally(facts.can_open_externally);
        global.set_find_open(facts.find_open);
        global.set_over_link(facts.over_link);
        match self.content {
            Some(content) => {
                global.set_page(i32::try_from(content.page).unwrap_or(i32::MAX));
                global.set_page_count(i32::try_from(content.page_count).unwrap_or(i32::MAX));
                global.set_content_width(content.width);
                global.set_content_height(content.height);
                global.set_scroll_x(content.scroll_x);
                global.set_scroll_y(content.scroll_y);
            }
            None => {
                global.set_page_count(0);
                global.set_content_width(0.0);
                global.set_content_height(0.0);
                global.set_scroll_x(0.0);
                global.set_scroll_y(0.0);
            }
        }
        if let Some(pages) = self.pages {
            global.set_pages(ModelRc::new(VecModel::from(pages)));
        }
        if let Some(tiles) = self.tiles {
            global.set_tiles(ModelRc::new(VecModel::from(tiles)));
        }
        if let Some(selection) = self.selection {
            global.set_selection(ModelRc::new(VecModel::from(selection)));
        }
        if let Some(hits) = self.hits {
            global.set_hits(ModelRc::new(VecModel::from(hits)));
        }
    }
}

/// The smallest and largest tab zoom (the ends of `talaria_core::ZOOM_STEPS`).
const ZOOM_MIN: u16 = talaria_core::ZOOM_STEPS[0];
const ZOOM_MAX: u16 = talaria_core::ZOOM_STEPS[talaria_core::ZOOM_STEPS.len() - 1];

fn rect_row(x: f32, y: f32, width: f32, height: f32, current: bool) -> PdfRectRow {
    PdfRectRow {
        x,
        y,
        width,
        height,
        current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(width: u32, height: u32, shade: u8) -> Arc<Tile> {
        Arc::new(Tile {
            width,
            height,
            rgba: vec![shade; (width * height * 4) as usize],
        })
    }

    fn placed(page: u32, pixels: &Arc<Tile>, x: f32) -> PlacedTile {
        PlacedTile {
            x,
            y: 8.0,
            width: 100.0,
            height: 100.0,
            page,
            pixels: Arc::clone(pixels),
            is_placeholder: false,
        }
    }

    #[test]
    fn a_tile_becomes_an_image_of_its_size_with_its_pixels() {
        let image = image_of(&Tile {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
        })
        .expect("an image");
        let pixels = image.to_rgba8().expect("pixels the CPU can read");
        assert_eq!((pixels.width(), pixels.height()), (2, 1));
        assert_eq!(pixels.as_bytes(), &[1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn a_tile_whose_buffer_does_not_fit_its_size_is_not_drawn() {
        assert!(
            image_of(&Tile {
                width: 4,
                height: 4,
                rgba: vec![0; 10],
            })
            .is_none()
        );
        assert!(
            image_of(&Tile {
                width: 0,
                height: 4,
                rgba: Vec::new(),
            })
            .is_none()
        );
    }

    #[test]
    fn images_are_kept_while_their_tile_shows_and_dropped_after() {
        let first = tile(4, 4, 10);
        let second = tile(4, 4, 20);
        let mut images = TileImages::default();

        let rows = images.rows(&[placed(0, &first, 0.0), placed(0, &second, 100.0)]);
        assert_eq!(rows.len(), 2);
        assert_eq!(images.len(), 2);
        assert_eq!(rows[1].x, 100.0);

        // The same tiles again: no new images.
        let again = images.rows(&[placed(0, &first, 0.0), placed(0, &second, 100.0)]);
        assert_eq!(again.len(), 2);
        assert_eq!(images.len(), 2);

        // One scrolled out of view.
        let rows = images.rows(&[placed(0, &second, 100.0)]);
        assert_eq!(rows.len(), 1);
        assert_eq!(images.len(), 1);
    }

    fn facts(state: ViewState) -> Facts {
        Facts {
            state,
            title: "Report".to_owned(),
            message: String::new(),
            zoom_percent: 100,
            fit: FitButton::Width,
            can_save_copy: true,
            can_open_externally: true,
            find_open: false,
            over_link: false,
        }
    }

    fn model_of(pages: usize) -> ViewerModel {
        ViewerModel::new(mercury_pdf::types::DocumentInfo {
            pages: vec![mercury_pdf::types::PageSize::new(612.0, 792.0); pages],
            title: None,
        })
    }

    #[test]
    fn only_what_changed_is_written_again() {
        let model = model_of(3);
        let mut cache = ViewCache::default();

        let first = cache.prepare(facts(ViewState::Ready), Some(&model));
        assert!(first.state_changed);
        let content = first.content.expect("pages show");
        assert_eq!((content.page, content.page_count), (1, 3));
        assert!(content.height > content.width, "pages are stacked");
        assert!(!first.pages.expect("the pages are new").is_empty());
        // No tile has arrived: the paper shows alone.
        assert!(first.tiles.is_none_or(|tiles| tiles.is_empty()));

        // The same again: the numbers are written, no row is.
        let again = cache.prepare(facts(ViewState::Ready), Some(&model));
        assert!(!again.state_changed);
        assert!(again.pages.is_none() && again.tiles.is_none());
        assert!(again.selection.is_none() && again.hits.is_none());
    }

    #[test]
    fn leaving_the_ready_state_empties_the_page_area_once() {
        let model = model_of(2);
        let mut cache = ViewCache::default();
        let _ = cache.prepare(facts(ViewState::Ready), Some(&model));

        let password = cache.prepare(facts(ViewState::Password), Some(&model));
        assert!(password.state_changed, "the panel before it is gone");
        assert!(password.content.is_none());
        assert_eq!(password.pages, Some(Vec::new()));
        assert_eq!(password.hits, Some(Vec::new()));

        let still = cache.prepare(facts(ViewState::Password), None);
        assert!(!still.state_changed);
        assert!(still.pages.is_none(), "already empty");

        let back = cache.prepare(facts(ViewState::Ready), Some(&model));
        assert!(back.state_changed);
        assert!(back.pages.is_some(), "the pages are written again");
    }

    #[test]
    fn a_broken_tile_is_left_out_and_the_others_still_show() {
        let good = tile(2, 2, 7);
        let broken = Arc::new(Tile {
            width: 3,
            height: 3,
            rgba: vec![0; 5],
        });
        let mut images = TileImages::default();
        let rows = images.rows(&[placed(0, &broken, 0.0), placed(0, &good, 50.0)]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].x, 50.0);
    }
}
