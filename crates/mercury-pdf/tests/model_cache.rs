// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The viewer model's tile cache: eviction order, the on-screen exception and the budgets, through
//! the stores themselves and through a model that is fed more tiles than fit.

use std::sync::Arc;

use mercury_pdf::cache::{ByteLru, PageStore, PlaceholderKey, TileKey, budget, scale_to_milli};
use mercury_pdf::layout::placeholder_request;
use mercury_pdf::types::{DocumentInfo, PageSize, Rotation, Tile, TileRequest};
use mercury_pdf::viewer::{Plan, ViewerModel, Viewport};

fn tile_of(bytes: usize) -> Arc<Tile> {
    Arc::new(Tile {
        width: 1,
        height: (bytes / 4) as u32,
        rgba: vec![0; bytes],
    })
}

fn key(page: u32, col: u32, row: u32) -> TileKey {
    TileKey {
        page,
        scale_milli: 1000,
        rotation: Rotation::Deg0,
        col,
        row,
    }
}

#[test]
fn cache_evicts_in_the_order_the_tiles_were_last_used() {
    let mut cache = ByteLru::new(4_000);
    for page in 0..5 {
        cache.insert(key(page, 0, 0), tile_of(1_000));
    }
    // Used in this order: 3, 0, 4 (the others untouched, oldest 1).
    cache.touch(&key(3, 0, 0));
    cache.touch(&key(0, 0, 0));
    cache.get(&key(4, 0, 0));
    let evicted = cache.evict_to_budget(|_| false);
    assert_eq!(evicted, vec![key(1, 0, 0)]);
    cache.insert(key(7, 0, 0), tile_of(3_000));
    // 7 bytes... 4,000 + 3,000: the oldest go until it fits: 2, 3, 0 in that order.
    let evicted = cache.evict_to_budget(|_| false);
    assert_eq!(evicted, vec![key(2, 0, 0), key(3, 0, 0), key(0, 0, 0)]);
    assert_eq!(cache.bytes(), 4_000);
}

#[test]
fn cache_never_evicts_a_pinned_tile() {
    let mut cache = ByteLru::new(2_000);
    for page in 0..6 {
        cache.insert(key(page, 0, 0), tile_of(1_000));
    }
    // 0 and 1 are on screen: they are the oldest, and stay.
    let evicted = cache.evict_to_budget(|key| key.page < 2);
    assert_eq!(evicted.len(), 4);
    assert!(cache.contains(&key(0, 0, 0)) && cache.contains(&key(1, 0, 0)));
    assert_eq!(cache.bytes(), 2_000);
    // Everything pinned and over budget: nothing goes, nothing breaks.
    cache.insert(key(9, 0, 0), tile_of(1_000));
    assert!(cache.evict_to_budget(|_| true).is_empty());
    assert_eq!(cache.bytes(), 3_000);
    assert_eq!(cache.keys().count(), 3);
}

#[test]
fn cache_keys_are_page_scale_rotation_and_tile() {
    let request = TileRequest {
        page: 3,
        scale: 1.337,
        rotation: Rotation::Deg270,
        x: 512,
        y: 1024,
        width: 512,
        height: 512,
    };
    assert_eq!(
        TileKey::from_request(&request),
        TileKey {
            page: 3,
            scale_milli: 1337,
            rotation: Rotation::Deg270,
            col: 1,
            row: 2
        }
    );
    // The same tile at another zoom or rotation is another entry.
    let mut cache = ByteLru::new(10_000);
    cache.insert(TileKey::from_request(&request), tile_of(100));
    let other_scale = TileRequest {
        scale: 1.338,
        ..request
    };
    let other_rotation = TileRequest {
        rotation: Rotation::Deg0,
        ..request
    };
    assert!(!cache.contains(&TileKey::from_request(&other_scale)));
    assert!(!cache.contains(&TileKey::from_request(&other_rotation)));
    assert_eq!(scale_to_milli(0.05), 50);
    assert_eq!(scale_to_milli(16.0), 16_000);
}

#[test]
fn cache_the_page_store_keeps_what_is_wanted_and_drops_the_oldest() {
    let mut store: PageStore<Vec<u8>> = PageStore::new(100);
    for page in 0..10 {
        store.insert(page, vec![0; 3], 30);
    }
    assert_eq!(store.cost(), 300);
    let gone = store.evict(|page| page == 0);
    // 100 budget, 30 each: three stay, page 0 (wanted) among them, the newest two with it.
    assert_eq!(store.len(), 3);
    assert!(store.contains(0) && store.contains(8) && store.contains(9));
    assert_eq!(gone, vec![1, 2, 3, 4, 5, 6, 7]);
}

fn big_pages(count: usize, width: f32, height: f32) -> DocumentInfo {
    DocumentInfo {
        pages: vec![PageSize::new(width, height); count],
        title: None,
    }
}

fn tile_for(request: &TileRequest) -> Tile {
    Tile {
        width: request.width,
        height: request.height,
        rgba: vec![0; (request.width * request.height * 4) as usize],
    }
}

fn deliver(model: &mut ViewerModel, plan: &Plan) {
    for (request, _) in &plan.requests {
        model.tile_arrived(request, tile_for(request));
    }
}

/// Plans and delivers until the model asks for nothing more.
fn settle(model: &mut ViewerModel) {
    for _ in 0..30 {
        let plan = model.plan();
        if plan.requests.is_empty() {
            return;
        }
        deliver(model, &plan);
    }
    panic!("the model keeps asking for more");
}

#[test]
fn cache_scrolling_through_a_long_document_stays_inside_the_budget() {
    let mut model = ViewerModel::new(big_pages(120, 600.0, 800.0));
    model.set_viewport(Viewport {
        width: 616.0,
        height: 800.0,
        scale_factor: 1.0,
    });
    let mut most = 0;
    for step in 0..120 {
        model.go_to_page(step, None);
        settle(&mut model);
        let usage = model.cache_usage();
        most = most.max(usage.tile_bytes);
        assert!(
            usage.tile_bytes <= budget::TILE_BYTES,
            "page {step}: {} bytes",
            usage.tile_bytes
        );
        assert!(usage.placeholder_bytes <= budget::PLACEHOLDER_BYTES);
        // What shows is always there.
        let shown = model.visible_tiles();
        assert!(!shown.is_empty() && shown.iter().all(|tile| !tile.is_placeholder));
    }
    // It did fill the cache: the test would mean nothing if it never evicted.
    assert!(most > budget::TILE_BYTES * 7 / 10, "{most}");
}

#[test]
fn cache_what_is_on_screen_stays_even_when_it_is_more_than_the_budget() {
    // One page of 5,000 points in a 5,120 x 5,200 window: 10 x 10 tiles of 1 MiB on screen.
    let mut model = ViewerModel::new(big_pages(1, 5000.0, 5000.0));
    model.set_viewport(Viewport {
        width: 5120.0,
        height: 5200.0,
        scale_factor: 1.0,
    });
    settle(&mut model);
    let usage = model.cache_usage();
    assert!(usage.tile_bytes > budget::TILE_BYTES, "{usage:?}");
    let shown = model.visible_tiles();
    assert_eq!(shown.len(), 100);
    assert!(shown.iter().all(|tile| !tile.is_placeholder));
    // Nothing is asked for again, and nothing was lost to make room.
    assert!(model.plan().is_empty());
}

#[test]
fn cache_the_margin_never_asks_for_more_than_its_share_of_the_budget() {
    // Pages of 7.7 MB each on a screen that shows 4 of them: with the margin of one viewport on
    // each side the wanted tiles would be 11 pages, 86 MB, more than the 96 MiB budget leaves for
    // them. The margin is cut at 75 % of the budget, or fetching it would evict the screen.
    let mut model = ViewerModel::new(big_pages(200, 600.0, 800.0));
    model.set_viewport(Viewport {
        width: 616.0,
        height: 3000.0,
        scale_factor: 2.0,
    });
    model.go_to_page(10, None);
    let mut asked = 0_usize;
    for _ in 0..30 {
        let plan = model.plan();
        if plan.requests.is_empty() {
            break;
        }
        asked += plan
            .requests
            .iter()
            .map(|(request, _)| (request.width * request.height * 4) as usize)
            .sum::<usize>();
        deliver(&mut model, &plan);
    }
    // The model is quiet (no fetch-evict-fetch churn), nothing was lost to make room, and the
    // margin stopped at its share.
    let usage = model.cache_usage();
    let share = budget::TILE_BYTES / 100 * budget::PREFETCH_SHARE_PERCENT;
    assert!(model.plan().is_empty());
    assert!(usage.tile_bytes <= share, "{} > {share}", usage.tile_bytes);
    assert!(usage.tile_bytes > share / 2, "{usage:?}");
    assert!(asked >= usage.tile_bytes);
}

#[test]
fn cache_placeholders_have_their_own_small_budget_and_keep_the_visible_page() {
    let mut model = ViewerModel::new(big_pages(600, 600.0, 800.0));
    model.set_viewport(Viewport {
        width: 616.0,
        height: 600.0,
        scale_factor: 1.0,
    });
    let first = model.plan();
    // Page 0's placeholder is the last of the first plan's requests for page 0.
    let visible_placeholder = first
        .requests
        .iter()
        .map(|(request, _)| *request)
        .find(|request| request.page == 0 && request.width == 160)
        .expect("a placeholder for the page that shows");
    model.tile_arrived(&visible_placeholder, tile_for(&visible_placeholder));
    // 599 more pages' placeholders arrive, far more than 16 MiB.
    for page in 1..600 {
        let request = placeholder_request(PageSize::new(600.0, 800.0), page, Rotation::Deg0)
            .expect("a placeholder");
        model.tile_arrived(&request, tile_for(&request));
    }
    let usage = model.cache_usage();
    assert!(
        usage.placeholder_bytes <= budget::PLACEHOLDER_BYTES,
        "{usage:?}"
    );
    assert!(usage.placeholders > 100, "{usage:?}");
    // The page on screen kept its placeholder, so the paper is not blank while tiles load.
    let shown = model.visible_tiles();
    assert_eq!(shown.len(), 1);
    assert!(shown[0].is_placeholder && shown[0].page == 0);
}

#[test]
fn cache_a_placeholder_is_at_most_160_pixels_wide() {
    for (width, height) in [
        (612.0, 792.0),
        (842.0, 595.0),
        (100.0, 14_000.0),
        (20.0, 20.0),
    ] {
        let request = placeholder_request(PageSize::new(width, height), 0, Rotation::Deg0)
            .expect("a placeholder");
        assert!(
            request.width <= budget::PLACEHOLDER_MAX_WIDTH,
            "{request:?}"
        );
        let tile = tile_for(&request);
        assert!(tile.is_consistent());
    }
    let key: PlaceholderKey = (3, Rotation::Deg90);
    let mut store: ByteLru<PlaceholderKey> = ByteLru::new(budget::PLACEHOLDER_BYTES);
    store.insert(key, tile_of(400));
    assert!(store.peek(&key).is_some());
    assert!(store.peek(&(3, Rotation::Deg0)).is_none());
}

fn blank_text(chars: usize) -> mercury_pdf::types::PageText {
    mercury_pdf::types::PageText {
        chars: vec![
            mercury_pdf::types::TextChar {
                ch: 'a',
                bounds: mercury_pdf::types::Rect::default(),
            };
            chars
        ],
    }
}

#[test]
fn cache_page_text_is_bounded_and_the_selection_keeps_its_pages() {
    let mut model = ViewerModel::new(big_pages(8, 600.0, 800.0));
    model.set_viewport(Viewport {
        width: 616.0,
        height: 600.0,
        scale_factor: 1.0,
    });
    // Five pages of half a million characters: 2.5 million, over the 2 million budget.
    for page in 1..6 {
        model.set_page_text(page, blank_text(500_000));
    }
    let usage = model.cache_usage();
    assert!(
        usage.text_chars <= budget::TEXT_CHARS,
        "{}",
        usage.text_chars
    );
    assert!(usage.text_chars >= 1_000_000, "it keeps what fits");
    // A press on page 0 needs its text: it stays however many other pages come after it.
    model.pointer_down(18.5, 113.0, 1);
    model.set_page_text(0, hello_text());
    model.pointer_drag(8.0 + 200.0, 113.0);
    model.pointer_up();
    for page in 1..8 {
        model.set_page_text(page, blank_text(500_000));
    }
    assert!(model.cache_usage().text_chars <= budget::TEXT_CHARS + 500_001);
    assert_eq!(model.selected_text().as_deref(), Some("Hello world"));
}

fn hello_text() -> mercury_pdf::types::PageText {
    mercury_pdf::types::PageText {
        chars: "Hello world"
            .chars()
            .enumerate()
            .map(|(index, ch)| mercury_pdf::types::TextChar {
                ch,
                bounds: mercury_pdf::types::Rect::new(
                    10.0 + index as f32 * 6.0,
                    100.0,
                    16.0 + index as f32 * 6.0,
                    110.0,
                ),
            })
            .collect(),
    }
}

#[test]
fn cache_releasing_the_caches_frees_everything_fetched_and_keeps_the_view() {
    let mut model = ViewerModel::new(big_pages(40, 600.0, 800.0));
    model.set_viewport(Viewport {
        width: 616.0,
        height: 800.0,
        scale_factor: 1.0,
    });
    model.set_zoom_percent(150);
    model.go_to_page(20, None);
    settle(&mut model);
    model.set_page_text(20, blank_text(1_000));
    model.set_links(20, Vec::new());
    let before = model.cache_usage();
    assert!(before.tiles > 0 && before.tile_bytes > 0, "{before:?}");
    assert!(before.placeholders > 0, "{before:?}");
    assert!(before.text_chars > 0 && before.link_pages > 0, "{before:?}");
    let (scroll_x, scroll_y, zoom, page) = (
        model.scroll_x(),
        model.scroll_y(),
        model.zoom_percent(),
        model.current_page(),
    );

    model.release_caches();

    assert_eq!(model.cache_usage(), Default::default());
    assert_eq!(
        (
            model.scroll_x(),
            model.scroll_y(),
            model.zoom_percent(),
            model.current_page()
        ),
        (scroll_x, scroll_y, zoom, page),
        "the view stays"
    );
    // When the helper is back the model asks again for what the screen needs, tiles and links,
    // and fills its caches from the answers.
    let plan = model.plan();
    assert!(!plan.requests.is_empty(), "{plan:?}");
    // Text is asked for only when a selection needs it; links for the pages that show.
    assert!(plan.links.contains(&20), "{plan:?}");
    deliver(&mut model, &plan);
    settle(&mut model);
    assert!(model.cache_usage().tile_bytes > 0);
    assert!(!model.visible_tiles().is_empty());
}
