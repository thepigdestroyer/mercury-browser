// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `PdfiumBackend` against the real PDFium library.
//!
//! The tests run when `MERCURY_PDFIUM_LIBRARY` names the library or the cache that
//! `scripts/macos/pdfium.sh` fills has it; otherwise each prints `skipped: no PDFium library` and
//! passes. With `MERCURY_REQUIRE_PDFIUM=1` a missing library fails them.

mod support;

use std::time::Instant;

use mercury_pdf::backend::PdfBackend;
use mercury_pdf::error::PdfError;
use mercury_pdf::testing::{DocumentSpec, PageSpec, document_pdf, minimal_pdf};
use mercury_pdf::types::{
    FindOptions, LinkTarget, PageSize, Rect, Rotation, SearchHit, Tile, TileRequest,
};
use support::{
    Scratch, dark_pixels, encrypted_pdf, fixture, open, pixel, region_of, session, tile_around,
    whole_page,
};

fn near(actual: f32, wanted: f32, tolerance: f32) -> bool {
    (actual - wanted).abs() <= tolerance
}

fn assert_rect_near(rect: &Rect, wanted: [f32; 4], tolerance: f32, what: &str) {
    assert!(
        near(rect.left, wanted[0], tolerance)
            && near(rect.top, wanted[1], tolerance)
            && near(rect.right, wanted[2], tolerance)
            && near(rect.bottom, wanted[3], tolerance),
        "{what}: {rect:?} is not near {wanted:?}"
    );
}

fn single_page_pdf(page: PageSpec) -> Vec<u8> {
    document_pdf(&DocumentSpec {
        title: None,
        pages: vec![page],
        outline: Vec::new(),
    })
}

#[test]
fn the_pages_of_a_document_have_their_shown_sizes_and_title() {
    let Some(mut session) = session() else { return };
    let info = open(&mut session.backend, &fixture("three-pages.pdf"));
    assert_eq!(info.pages, vec![PageSize::new(612.0, 792.0); 3]);
    assert_eq!(info.title.as_deref(), Some("Mercury three-page test"));
}

#[test]
fn a_tile_is_dark_glyphs_on_opaque_white() {
    let Some(mut session) = session() else { return };
    let info = open(&mut session.backend, &fixture("three-pages.pdf"));
    let tile = whole_page(&mut session.backend, &info, 0, 1.0, Rotation::Deg0);
    assert_eq!((tile.width, tile.height), (612, 792));
    assert!(tile.is_consistent());
    assert_eq!(pixel(&tile, 0, 0), [255, 255, 255, 255]);
    assert!(
        tile.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 255),
        "every pixel is opaque"
    );
    assert!(
        dark_pixels(&tile, [36, 36, 400, 130]) > 300,
        "the text is drawn"
    );
    assert_eq!(
        dark_pixels(&tile, [0, 400, 612, 792]),
        0,
        "nothing is drawn below the text"
    );
}

#[test]
fn the_colour_channels_come_out_as_red_green_blue() {
    let Some(mut session) = session() else { return };
    let scratch = Scratch::new("channels");
    let mut page = PageSpec::letter(Vec::<String>::new());
    page.fills = vec![
        ([100.0, 100.0, 150.0, 150.0], [1.0, 0.0, 0.0]),
        ([200.0, 100.0, 250.0, 150.0], [0.0, 1.0, 0.0]),
        ([300.0, 100.0, 350.0, 150.0], [0.0, 0.0, 1.0]),
    ];
    let path = scratch.write("colours.pdf", &single_page_pdf(page));
    let info = open(&mut session.backend, &path);
    let tile = whole_page(&mut session.backend, &info, 0, 1.0, Rotation::Deg0);
    assert_eq!(pixel(&tile, 125, 125), [255, 0, 0, 255]);
    assert_eq!(pixel(&tile, 225, 125), [0, 255, 0, 255]);
    assert_eq!(pixel(&tile, 325, 125), [0, 0, 255, 255]);
    assert_eq!(pixel(&tile, 125, 300), [255, 255, 255, 255]);
}

/// The pixels of the window `(x, y, width, height)` of a whole-page tile.
fn crop(whole: &Tile, x: u32, y: u32, width: u32, height: u32) -> Vec<u8> {
    let mut rgba = Vec::new();
    for row in y..y + height {
        let start = ((row * whole.width + x) * 4) as usize;
        rgba.extend_from_slice(&whole.rgba[start..start + (width * 4) as usize]);
    }
    rgba
}

#[test]
fn a_tile_is_the_matching_window_of_the_whole_page() {
    let Some(mut session) = session() else { return };
    let info = open(&mut session.backend, &fixture("three-pages.pdf"));
    for scale in [1.0_f32, 2.0] {
        let whole = whole_page(&mut session.backend, &info, 0, scale, Rotation::Deg0);
        for (x, y, width, height) in [(0, 0, 64, 48), (100, 200, 64, 48), (333, 57, 200, 150)] {
            let (x, y) = ((x as f32 * scale) as u32, (y as f32 * scale) as u32);
            let request = TileRequest {
                page: 0,
                scale,
                rotation: Rotation::Deg0,
                x,
                y,
                width,
                height,
            };
            let tile = session.backend.render(&request).expect("the tile renders");
            assert_eq!(
                tile.rgba,
                crop(&whole, x, y, width, height),
                "scale {scale}: the tile at ({x}, {y}) differs from that window of the whole page"
            );
        }
    }
}

#[test]
fn the_whole_page_can_be_asked_for_in_pieces_that_fit_together() {
    let Some(mut session) = session() else { return };
    let info = open(&mut session.backend, &fixture("three-pages.pdf"));
    let whole = whole_page(&mut session.backend, &info, 1, 1.0, Rotation::Deg0);
    let mut rebuilt = vec![0_u8; whole.rgba.len()];
    for (x, width) in [(0, 512), (512, 100)] {
        for (y, height) in [(0, 512), (512, 280)] {
            let tile = session
                .backend
                .render(&TileRequest {
                    page: 1,
                    scale: 1.0,
                    rotation: Rotation::Deg0,
                    x,
                    y,
                    width,
                    height,
                })
                .unwrap();
            for row in 0..height {
                let from = (row * width * 4) as usize;
                let to = (((y + row) * whole.width + x) * 4) as usize;
                rebuilt[to..to + (width * 4) as usize]
                    .copy_from_slice(&tile.rgba[from..from + (width * 4) as usize]);
            }
        }
    }
    assert_eq!(rebuilt, whole.rgba);
}

#[test]
fn a_quarter_turn_moves_the_picture_with_the_page() {
    let Some(mut session) = session() else { return };
    let scratch = Scratch::new("turns");
    let mut page = PageSpec::letter(Vec::<String>::new());
    page.fills = vec![([100.0, 100.0, 150.0, 150.0], [1.0, 0.0, 0.0])];
    let path = scratch.write("red.pdf", &single_page_pdf(page));
    let info = open(&mut session.backend, &path);
    let red = [255, 0, 0, 255];
    // The fill's centre is (125, 125) on a 612 x 792 page.
    for (rotation, size, at) in [
        (Rotation::Deg0, (612, 792), (125, 125)),
        (Rotation::Deg90, (792, 612), (792 - 125, 125)),
        (Rotation::Deg180, (612, 792), (612 - 125, 792 - 125)),
        (Rotation::Deg270, (792, 612), (125, 612 - 125)),
    ] {
        let tile = whole_page(&mut session.backend, &info, 0, 1.0, rotation);
        assert_eq!((tile.width, tile.height), size, "{rotation:?}");
        assert_eq!(pixel(&tile, at.0, at.1), red, "{rotation:?}");
        // And not where it was before the turn (unless that is the same place).
        if at != (125, 125) {
            assert_ne!(pixel(&tile, 125, 125), red, "{rotation:?}");
        }
    }
}

#[test]
fn the_text_of_a_page_has_boxes_that_lie_on_its_glyphs() {
    let Some(mut session) = session() else { return };
    let info = open(&mut session.backend, &fixture("three-pages.pdf"));
    let text = session.backend.page_text(0).expect("the text");
    let lines: Vec<String> = text.text().lines().map(str::to_owned).collect();
    assert_eq!(
        lines,
        [
            "Mercury PDF test page 1",
            "The quick brown fox jumps over the lazy dog.",
            "The word fox appears on page 1.",
            "Visit the example site.",
            "Jump to the last page."
        ]
    );
    let first = text.chars[0];
    assert_eq!(first.ch, 'M');
    assert!(
        near(first.bounds.left, 36.0, 1.5)
            && first.bounds.top > 28.0
            && first.bounds.top < 42.0
            && first.bounds.bottom > 46.0
            && first.bounds.bottom < 56.0
            && first.bounds.width() > 7.0
            && first.bounds.width() < 12.0,
        "{:?}",
        first.bounds
    );
    // PDFium made up the line breaks; they have no place on the page.
    let breaks: Vec<_> = text
        .chars
        .iter()
        .filter(|character| matches!(character.ch, '\r' | '\n'))
        .collect();
    assert!(!breaks.is_empty());
    assert!(breaks.iter().all(|character| character.bounds.is_empty()));
    // Every visible letter has dark pixels inside its box.
    let scale = 3.0;
    let mut checked = 0;
    for character in &text.chars {
        if character.ch.is_whitespace() || character.bounds.is_empty() {
            continue;
        }
        let (tile, origin) = tile_around(
            &mut session.backend,
            &info,
            0,
            &character.bounds,
            scale,
            Rotation::Deg0,
            0,
        );
        let region = region_of(&character.bounds, origin, scale);
        assert!(
            dark_pixels(&tile, region) > 0,
            "'{}' at {:?} has no dark pixel inside its box",
            character.ch,
            character.bounds
        );
        checked += 1;
    }
    assert!(checked > 100, "{checked} letters were checked");
}

fn hits(
    session: &mut support::Session,
    page: u32,
    query: &str,
    options: FindOptions,
) -> Vec<SearchHit> {
    session
        .backend
        .find_in_page(page, query, options)
        .expect("the search runs")
}

#[test]
fn a_find_hit_is_a_rectangle_over_its_glyphs_and_the_range_of_its_characters() {
    let Some(mut session) = session() else { return };
    let info = open(&mut session.backend, &fixture("three-pages.pdf"));
    let text = session.backend.page_text(0).unwrap();
    let found = hits(&mut session, 0, "fox", FindOptions::default());
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[0].char_start < found[1].char_start,
        "in reading order"
    );
    assert!(found[0].rects[0].top < found[1].rects[0].top);
    for hit in &found {
        assert_eq!(hit.page, 0);
        assert_eq!(hit.rects.len(), 1);
        assert_eq!(
            text.slice(hit.char_start as usize..(hit.char_start + hit.char_len) as usize),
            "fox"
        );
        let rect = hit.rects[0];
        let (tile, origin) = tile_around(
            &mut session.backend,
            &info,
            0,
            &rect,
            4.0,
            Rotation::Deg0,
            2,
        );
        let dark = dark_pixels(&tile, region_of(&rect, origin, 4.0));
        assert!(
            dark > 30,
            "the hit rectangle {rect:?} holds only {dark} dark pixels"
        );
        // The rectangle is on the line it belongs to.
        assert!(rect.width() > 14.0 && rect.width() < 30.0, "{rect:?}");
    }
}

#[test]
fn find_options_and_phrases() {
    let Some(mut session) = session() else { return };
    open(&mut session.backend, &fixture("three-pages.pdf"));
    let text = session.backend.page_text(1).unwrap();
    let mut both = |options| hits(&mut session, 1, "FOX", options).len();
    assert_eq!(both(FindOptions::default()), 2);
    assert_eq!(
        both(FindOptions {
            match_case: true,
            whole_word: false
        }),
        0
    );
    assert_eq!(
        hits(
            &mut session,
            1,
            "fo",
            FindOptions {
                match_case: false,
                whole_word: true
            }
        )
        .len(),
        0
    );
    assert_eq!(
        hits(
            &mut session,
            1,
            "fox",
            FindOptions {
                match_case: false,
                whole_word: true
            }
        )
        .len(),
        2
    );
    let the = hits(&mut session, 1, "the", FindOptions::default());
    assert_eq!(the.len(), text.text().to_lowercase().matches("the").count());
    let phrase = hits(&mut session, 1, "quick brown", FindOptions::default());
    assert_eq!(phrase.len(), 1);
    assert_eq!(phrase[0].char_len, 11);
    assert_eq!(
        text.slice(
            phrase[0].char_start as usize..(phrase[0].char_start + phrase[0].char_len) as usize
        ),
        "quick brown"
    );
    assert!(hits(&mut session, 1, "", FindOptions::default()).is_empty());
    assert!(hits(&mut session, 1, "nothing like this", FindOptions::default()).is_empty());
}

/// The case the brief singles out: `/Rotate` and a crop box that does not start at the origin.
#[test]
fn on_rotated_and_cropped_pages_a_hit_lands_on_dark_pixels_and_text_reads_upright() {
    let Some(mut session) = session() else { return };
    let info = open(&mut session.backend, &fixture("rotated.pdf"));
    assert_eq!(
        info.pages,
        [
            PageSize::new(300.0, 400.0), // 400 x 300 crop, /Rotate 90
            PageSize::new(770.0, 555.0), // 555 x 770 crop, /Rotate 270
            PageSize::new(400.0, 300.0), // 400 x 300 crop, /Rotate 180
            PageSize::new(540.0, 648.0), // 540 x 648 crop, no rotation
        ]
    );
    for (index, page_size) in info.pages.clone().into_iter().enumerate() {
        let page = index as u32;
        let found = hits(&mut session, page, "FindMe", FindOptions::default());
        assert_eq!(found.len(), 1, "page {page}: {found:?}");
        let rect = found[0].rects[0];
        // Line 2 of the page: its baseline is 74 points from the top of the shown page, and the
        // text starts 36 points from the left.
        assert!(near(rect.left, 36.0, 2.0), "page {page}: {rect:?}");
        assert!(
            rect.top < 74.0 && rect.bottom > 74.0,
            "page {page}: {rect:?}"
        );
        assert!(
            rect.right < page_size.width && rect.bottom < page_size.height,
            "page {page}: {rect:?} is outside {page_size:?}"
        );
        let (tile, origin) = tile_around(
            &mut session.backend,
            &info,
            page,
            &rect,
            4.0,
            Rotation::Deg0,
            2,
        );
        let dark = dark_pixels(&tile, region_of(&rect, origin, 4.0));
        assert!(
            dark > 30,
            "page {page}: the hit rectangle {rect:?} holds only {dark} dark pixels"
        );
        // Where the hit is NOT (the mirrored place) there is nothing dark.
        let mirrored = Rect::new(
            page_size.width - rect.right,
            page_size.height - rect.bottom,
            page_size.width - rect.left,
            page_size.height - rect.top,
        );
        if (mirrored.left - rect.left).abs() > rect.width() {
            let (tile, origin) = tile_around(
                &mut session.backend,
                &info,
                page,
                &mirrored,
                4.0,
                Rotation::Deg0,
                0,
            );
            assert_eq!(
                dark_pixels(&tile, region_of(&mirrored, origin, 4.0)),
                0,
                "page {page}: dark pixels where nothing was written ({mirrored:?})"
            );
        }
        // The text reads left to right at one height: the letters of "FindMe" step to the right.
        let text = session.backend.page_text(page).unwrap();
        let start = found[0].char_start as usize;
        assert_eq!(text.slice(start..start + 6), "FindMe");
        let lefts: Vec<f32> = text.chars[start..start + 6]
            .iter()
            .map(|character| character.bounds.left)
            .collect();
        assert!(
            lefts.windows(2).all(|pair| pair[1] > pair[0]),
            "page {page}: {lefts:?}"
        );
        let tops: Vec<f32> = text.chars[start..start + 6]
            .iter()
            .map(|character| character.bounds.top)
            .collect();
        assert!(
            tops.windows(2).all(|pair| near(pair[0], pair[1], 3.5)),
            "page {page}: {tops:?}"
        );
    }
}

#[test]
fn links_come_with_their_targets_in_page_space() {
    let Some(mut session) = session() else { return };
    open(&mut session.backend, &fixture("three-pages.pdf"));
    let links = session.backend.links(0).unwrap();
    assert_eq!(links.len(), 2, "{links:?}");
    assert_rect_near(
        &links[0].rect,
        [36.0, 78.0, 166.0, 90.0],
        1.5,
        "the web link",
    );
    assert_eq!(
        links[0].target,
        LinkTarget::Uri("https://example.com/mercury".to_owned())
    );
    assert_rect_near(
        &links[1].rect,
        [36.0, 92.0, 166.0, 104.0],
        1.5,
        "the page link",
    );
    let LinkTarget::Page { page, y } = links[1].target.clone() else {
        panic!("expected a link to a page, got {:?}", links[1].target);
    };
    assert_eq!(page, 2);
    assert!(near(y.expect("a position"), 100.0, 1.5), "{y:?}");
    // A link with no position says so.
    let last = session.backend.links(2).unwrap();
    assert_eq!(last.len(), 1);
    assert_eq!(last[0].target, LinkTarget::Page { page: 0, y: None });
    // A page without links.
    assert!(session.backend.links(1).unwrap().is_empty());
}

#[test]
fn a_link_on_a_rotated_cropped_page_is_where_it_is_drawn() {
    let Some(mut session) = session() else { return };
    open(&mut session.backend, &fixture("rotated.pdf"));
    let links = session.backend.links(0).unwrap();
    assert_eq!(links.len(), 1, "{links:?}");
    assert_rect_near(&links[0].rect, [36.0, 50.0, 126.0, 62.0], 1.5, "the link");
    assert_eq!(
        links[0].target,
        LinkTarget::Uri("https://example.com/rotated".to_owned())
    );
}

#[test]
fn the_outline_is_a_tree_of_titles_and_pages() {
    let Some(mut session) = session() else { return };
    open(&mut session.backend, &fixture("three-pages.pdf"));
    let outline = session.backend.outline().unwrap();
    let shape: Vec<(&str, Option<LinkTarget>, usize)> = outline
        .iter()
        .map(|item| {
            (
                item.title.as_str(),
                item.target.clone(),
                item.children.len(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            ("Chapter 1", Some(LinkTarget::Page { page: 0, y: None }), 0),
            ("Chapter 2", Some(LinkTarget::Page { page: 1, y: None }), 1),
        ]
    );
    assert_eq!(outline[1].children[0].title, "Section 2.1");
    assert_eq!(
        outline[1].children[0].target,
        Some(LinkTarget::Page { page: 2, y: None })
    );
    // A document without bookmarks.
    let scratch = Scratch::new("no-outline");
    let path = scratch.write("plain.pdf", &minimal_pdf(&[PageSpec::letter(["x"])]));
    open(&mut session.backend, &path);
    assert!(session.backend.outline().unwrap().is_empty());
}

#[test]
fn a_password_is_asked_for_checked_and_accepted() {
    let Some(mut session) = session() else { return };
    let scratch = Scratch::new("password");
    let path = scratch.write("locked.pdf", &encrypted_pdf("open sesame"));
    assert_eq!(
        session.backend.open(&path, None),
        Err(PdfError::PasswordRequired)
    );
    assert_eq!(
        session.backend.open(&path, Some("wrong")),
        Err(PdfError::PasswordIncorrect)
    );
    let info = session
        .backend
        .open(&path, Some("open sesame"))
        .expect("the right password opens it");
    assert_eq!(info.pages, [PageSize::new(200.0, 100.0)]);
    let tile = whole_page(&mut session.backend, &info, 0, 2.0, Rotation::Deg0);
    assert!(
        dark_pixels(&tile, [0, 0, tile.width, tile.height]) > 100,
        "the decrypted text is drawn"
    );
    // A failed open leaves no document behind.
    assert_eq!(
        session.backend.open(&path, Some("wrong")),
        Err(PdfError::PasswordIncorrect)
    );
    assert_eq!(session.backend.page_text(0), Err(PdfError::NoDocument));
}

#[test]
fn files_that_are_not_documents_are_errors() {
    let Some(mut session) = session() else { return };
    let scratch = Scratch::new("bad-files");
    let text = scratch.write("text.pdf", b"this is not a PDF file at all\n");
    let empty = scratch.write("empty.pdf", b"");
    let truncated = scratch.write(
        "truncated.pdf",
        &single_page_pdf(PageSpec::letter(["x"]))[..90],
    );
    for (name, path) in [("text", text), ("empty", empty), ("truncated", truncated)] {
        let result = session.backend.open(&path, None);
        assert!(
            matches!(result, Err(PdfError::Corrupt(_) | PdfError::Io(_))),
            "{name}: {result:?}"
        );
    }
    assert!(matches!(
        session
            .backend
            .open(&scratch.path().join("missing.pdf"), None),
        Err(PdfError::Io(_))
    ));
    assert!(matches!(
        session.backend.open(scratch.path(), None),
        Err(PdfError::Io(_))
    ));
}

#[test]
fn a_document_with_too_many_pages_is_refused_and_20000_are_not() {
    let Some(mut session) = session() else { return };
    let scratch = Scratch::new("many-pages");
    let page = PageSpec::letter(Vec::<String>::new());
    let just_enough = scratch.write("max.pdf", &minimal_pdf(&vec![page.clone(); 20_000]));
    let started = Instant::now();
    let info = session
        .backend
        .open(&just_enough, None)
        .expect("20,000 pages open");
    assert_eq!(info.pages.len(), 20_000);
    eprintln!("opening 20,000 pages took {:?}", started.elapsed());
    let too_many = scratch.write("over.pdf", &minimal_pdf(&vec![page; 20_001]));
    assert!(matches!(
        session.backend.open(&too_many, None),
        Err(PdfError::TooLarge(_))
    ));
}

#[test]
fn requests_that_make_no_sense_are_errors_and_never_a_crash() {
    let Some(mut session) = session() else { return };
    let tile = TileRequest {
        page: 0,
        scale: 1.0,
        rotation: Rotation::Deg0,
        x: 0,
        y: 0,
        width: 16,
        height: 16,
    };
    assert_eq!(session.backend.render(&tile), Err(PdfError::NoDocument));
    assert_eq!(session.backend.outline(), Err(PdfError::NoDocument));
    session.backend.close(); // with nothing open
    open(&mut session.backend, &fixture("three-pages.pdf"));
    for broken in [
        TileRequest { page: 9, ..tile },
        TileRequest { scale: 0.0, ..tile },
        TileRequest { x: 10_000, ..tile },
        TileRequest { width: 0, ..tile },
    ] {
        assert!(
            matches!(
                session.backend.render(&broken),
                Err(PdfError::InvalidRequest(_))
            ),
            "{broken:?}"
        );
    }
    assert!(matches!(
        session.backend.page_text(9),
        Err(PdfError::InvalidRequest(_))
    ));
    assert!(matches!(
        session.backend.links(9),
        Err(PdfError::InvalidRequest(_))
    ));
    assert!(matches!(
        session.backend.find_in_page(9, "x", FindOptions::default()),
        Err(PdfError::InvalidRequest(_))
    ));
    session.backend.close();
    assert_eq!(session.backend.page_text(0), Err(PdfError::NoDocument));
    // Opening a second document replaces the first.
    open(&mut session.backend, &fixture("three-pages.pdf"));
    let info = open(&mut session.backend, &fixture("rotated.pdf"));
    assert_eq!(info.pages.len(), 4);
}

#[test]
fn a_missing_library_is_reported_not_hidden() {
    let result =
        mercury_pdfium::PdfiumBackend::load(std::path::Path::new("/nonexistent/libpdfium.dylib"));
    assert!(matches!(result, Err(PdfError::LibraryMissing(_))));
}

/// Not a check of speed: it prints how long tiles take, for the lane report.
#[test]
fn tile_render_times_are_printed() {
    let Some(mut session) = session() else { return };
    let info = open(&mut session.backend, &fixture("three-pages.pdf"));
    for scale in [1.0_f32, 2.0] {
        let (page_width, page_height) = info.pages[0].pixel_size(scale, Rotation::Deg0);
        let request = |index: u32| TileRequest {
            page: index % 3,
            scale,
            rotation: Rotation::Deg0,
            x: ((index * 37) % 100).min(page_width.saturating_sub(512)),
            y: ((index * 53) % 200).min(page_height.saturating_sub(512)),
            width: 512,
            height: 512,
        };
        let cold_start = Instant::now();
        session.backend.render(&request(0)).expect("a tile");
        let cold = cold_start.elapsed();
        let warm_start = Instant::now();
        for index in 1..=30 {
            session.backend.render(&request(index)).expect("a tile");
        }
        let warm = warm_start.elapsed() / 30;
        eprintln!("tile 512x512 at {scale}x: first {cold:?}, then {warm:?} each (30 tiles)");
        assert!(warm.as_secs_f32() < 2.0, "{warm:?}");
    }
}
