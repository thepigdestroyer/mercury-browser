// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The cross test of the "Switch group" card: the real model from `build_card` (the card's Rust
//! side) drawn by the real component (`KeySwitcherPreview`), with Slint's software renderer at
//! 1440 x 900 in the light and the dark chrome. `key_switcher_tiles.rs` tests the model and
//! `key_switcher_render.rs` draws hand-made fixtures; this file checks that the two meet: the card
//! has the size `card_layout` computed, every tile is where the model put it, the rings carry the
//! tabs' shades, and a click on a tile or a chip reports the id the model gave it.
//!
//! No window is opened. Set `MERCURY_UI_RENDER_DIR` to a directory to get the PNGs.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use mercury_ui::key_switcher::{CardInput, CardModel, HintMode, build_card};
use mercury_ui::{ChromePalette, ChromeTheme, GroupRow, KeySwitcherPreview, KeyTile, TabRow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{
    Platform, PointerEventButton, WindowAdapter, WindowEvent, update_timers_and_animations,
};
use slint::{
    Color, ComponentHandle, LogicalPosition, LogicalSize, ModelRc, Rgba8Pixel, SharedPixelBuffer,
    VecModel,
};

type Pixels = SharedPixelBuffer<Rgba8Pixel>;

const WINDOW_WIDTH: i32 = 1440;
const WINDOW_HEIGHT: i32 = 900;

thread_local! {
    static CLOCK: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    static READY: Cell<bool> = const { Cell::new(false) };
}

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }

    fn duration_since_start(&self) -> Duration {
        CLOCK.with(Cell::get)
    }
}

fn init() {
    READY.with(|ready| {
        if !ready.replace(true) {
            slint::platform::set_platform(Box::new(SoftwarePlatform))
                .expect("no platform is set on this thread yet");
        }
    });
}

fn advance(milliseconds: u64) {
    CLOCK.with(|clock| clock.set(clock.get() + Duration::from_millis(milliseconds)));
    update_timers_and_animations();
}

// ---------------------------------------------------------------------------------------------
// The six groups of the mockup, as the rows the groups model hands over
// ---------------------------------------------------------------------------------------------

fn hex(rgb: u32) -> Color {
    Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// (id, key label, name, light tint, dark tint, active tab, tab shades); colours from
/// `keys-overlay-light.svg` and `-dark.svg`. The mockup's sixth group is "Loose", which has no key
/// in the model, so it is "Recipes" here.
#[allow(clippy::type_complexity)]
const MOCKUP: [(i32, &str, &str, u32, u32, Option<usize>, &[u32]); 6] = [
    (
        1,
        "A",
        "Research",
        0xDAE5F8,
        0x223065,
        Some(0),
        &[0x2655A0, 0x2B62BB, 0x2F6FD6, 0x598CDE, 0x82A9E6],
    ),
    (
        2,
        "S",
        "Lisbon trip",
        0xF9EBD9,
        0x573A31,
        None,
        &[0xA76E23, 0xCD8528, 0xE4A046, 0xECBD7F],
    ),
    (
        3,
        "D",
        "Reading",
        0xE8E1F7,
        0x392A63,
        None,
        &[0x60469B, 0x6C4FB0, 0x7857C5, 0x8868D4, 0x9D82DB, 0xB29DE3],
    ),
    (
        4,
        "F",
        "Shopping",
        0xF8DEE5,
        0x552445,
        None,
        &[0xA23954, 0xD9486E, 0xE891A8],
    ),
    (
        5,
        "G",
        "Work",
        0xD9EEE3,
        0x213E42,
        None,
        &[0x26774B, 0x82C5A1],
    ),
    (
        6,
        "H",
        "Recipes",
        0xE7E8EA,
        0x39354D,
        None,
        &[0x5E6167, 0xB0B3B8],
    ),
];

fn rows(dark: bool) -> (Vec<GroupRow>, Vec<TabRow>) {
    let mut groups = Vec::new();
    let mut tabs = Vec::new();
    for (id, key, name, light_tint, dark_tint, active, shades) in MOCKUP {
        groups.push(GroupRow {
            id,
            name: name.into(),
            key_label: key.into(),
            count: shades.len() as i32,
            tint: hex(if dark { dark_tint } else { light_tint }),
            ..Default::default()
        });
        for (index, shade) in shades.iter().enumerate() {
            tabs.push(TabRow {
                id: id * 1000 + index as i32,
                group: id,
                index_in_group: index as i32,
                title: format!("{name} page {}", index + 1).into(),
                shade: hex(*shade),
                active: active == Some(index),
                ..Default::default()
            });
        }
    }
    (groups, tabs)
}

// ---------------------------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------------------------

struct Rig {
    window: KeySwitcherPreview,
    model: CardModel,
    dark: bool,
    log: Rc<RefCell<Vec<String>>>,
}

/// Builds the card for the six groups with the given selection and applies the whole model.
fn open(dark: bool, selected_group: Option<i32>, selected_tab: Option<u8>) -> Rig {
    init();
    let (groups, tabs) = rows(dark);
    let model = build_card(&CardInput {
        groups: &groups,
        tabs: &tabs,
        hold_ms: 400,
        mode: HintMode::OutsideTextOrHold,
        selected_group,
        selected_tab,
    });

    let window = KeySwitcherPreview::new().expect("the preview window can be created");
    window.global::<ChromePalette>().set_theme(if dark {
        ChromeTheme::Dark
    } else {
        ChromeTheme::Light
    });
    window
        .window()
        .set_size(LogicalSize::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32));
    let log = Rc::new(RefCell::new(Vec::new()));
    window.on_tile_clicked({
        let log = Rc::clone(&log);
        move |group| log.borrow_mut().push(format!("tile:{group}"))
    });
    window.on_chip_clicked({
        let log = Rc::clone(&log);
        move |digit| log.borrow_mut().push(format!("chip:{digit}"))
    });
    window.on_change_keys({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("change-keys".to_owned())
    });
    window.on_dismissed({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("dismissed".to_owned())
    });

    window.set_reduce_motion(true);
    window.set_tiles(ModelRc::new(VecModel::from(model.tiles.clone())));
    window.set_chips(ModelRc::new(VecModel::from(model.chips.clone())));
    window.set_metrics(model.metrics.clone());
    window.set_selected_group(model.selected_group);
    window.set_selected_tab(model.selected_tab);
    window.set_chosen_name(model.chosen_name.as_str().into());
    window.set_hint_primary(model.hint_primary.as_str().into());
    window.set_hint_secondary(model.hint_secondary.as_str().into());
    window.set_empty_text(model.empty_text.as_str().into());
    window.set_card_width(model.layout.card_width);
    window.set_card_height(model.layout.card_height);
    window.set_tiles_left(model.layout.tiles_left);
    window.set_tiles_top(model.layout.tiles_top);
    window.show().expect("the preview window can be shown");
    Rig {
        window,
        model,
        dark,
        log,
    }
}

impl Rig {
    fn show(&self) -> Pixels {
        self.window.set_shown(true);
        let _ = self.snapshot();
        advance(300);
        self.snapshot()
    }

    fn snapshot(&self) -> Pixels {
        self.window
            .window()
            .take_snapshot()
            .expect("the window renders")
    }

    /// The card's rectangle in the window: centred, as the component centres it.
    fn card(&self) -> (i32, i32, i32, i32) {
        let layout = &self.model.layout;
        let (width, height) = (layout.card_width.round(), layout.card_height.round());
        (
            ((WINDOW_WIDTH as f32 - width) / 2.0).round() as i32,
            ((WINDOW_HEIGHT as f32 - height) / 2.0).round() as i32,
            width as i32,
            height as i32,
        )
    }

    /// Tile `index`'s box in the window: `(x, y, width, height)`.
    fn tile_box(&self, index: usize) -> (i32, i32, i32, i32) {
        let (card_x, card_y, ..) = self.card();
        let layout = &self.model.layout;
        let tile: &KeyTile = &self.model.tiles[index];
        (
            (card_x as f32 + layout.tiles_left + tile.x).round() as i32,
            (card_y as f32 + layout.tiles_top + tile.y).round() as i32,
            layout.metrics.tile_width.round() as i32,
            layout.metrics.tile_height.round() as i32,
        )
    }

    fn click(&self, x: i32, y: i32) {
        let position = LogicalPosition::new(x as f32, y as f32);
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    fn save(&self, pixels: &Pixels, state: &str) {
        let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR").map(PathBuf::from) else {
            return;
        };
        let theme = if self.dark { "dark" } else { "light" };
        image::save_buffer_with_format(
            directory.join(format!("key-switcher-card-{state}-{theme}.png")),
            pixels.as_bytes(),
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .expect("the rendering can be saved");
    }
}

fn at(pixels: &Pixels, x: i32, y: i32) -> Rgba8Pixel {
    pixels.as_slice()[(y as u32 * pixels.width() + x as u32) as usize]
}

fn is_near(pixel: Rgba8Pixel, colour: Color, tolerance: u8) -> bool {
    pixel.r.abs_diff(colour.red()) <= tolerance
        && pixel.g.abs_diff(colour.green()) <= tolerance
        && pixel.b.abs_diff(colour.blue()) <= tolerance
}

/// Pixels of a box `(x, y, width, height)` that are within `tolerance` of the colour.
fn count_near(
    pixels: &Pixels,
    (x, y, width, height): (i32, i32, i32, i32),
    colour: Color,
    tolerance: u8,
) -> usize {
    (y..y + height)
        .flat_map(|row| (x..x + width).map(move |column| (column, row)))
        .filter(|(column, row)| is_near(at(pixels, *column, *row), colour, tolerance))
        .count()
}

/// The card's rectangle as found in the picture: the outermost pixels of the border colour, seen
/// from the middle column and from a row near the bottom.
fn find_card(pixels: &Pixels, border: Color) -> (i32, i32, i32, i32) {
    let near = |x: i32, y: i32| is_near(at(pixels, x, y), border, 2);
    let column = WINDOW_WIDTH / 2;
    let top = (0..WINDOW_HEIGHT)
        .find(|y| near(column, *y))
        .expect("the card has a top border");
    let bottom = (0..WINDOW_HEIGHT)
        .rev()
        .find(|y| near(column, *y))
        .expect("the card has a bottom border");
    let row = bottom - 30;
    let left = (0..WINDOW_WIDTH)
        .find(|x| near(*x, row))
        .expect("the card has a left border");
    let right = (0..WINDOW_WIDTH)
        .rev()
        .find(|x| near(*x, row))
        .expect("the card has a right border");
    (left, top, right - left + 1, bottom - top + 1)
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn the_model_of_the_six_mockup_groups_is_drawn_to_the_size_and_places_it_computed() {
    for dark in [false, true] {
        let rig = open(dark, None, None);
        let theme = if dark { "dark" } else { "light" };
        let layout = &rig.model.layout;
        assert_eq!(rig.model.tiles.len(), 6, "{theme}");
        assert_eq!(
            (layout.card_width, layout.card_height),
            (760.0, 330.0),
            "{theme}: six full tiles make the mockup's card"
        );

        let pixels = rig.show();
        rig.save(&pixels, "rest");
        let palette = rig.window.global::<ChromePalette>();

        // The card is where and as big as `card_layout` said.
        assert_eq!(
            find_card(&pixels, palette.get_popup_border()),
            rig.card(),
            "{theme}: the card's rectangle"
        );

        for (index, tile) in rig.model.tiles.iter().enumerate() {
            let tile_box = rig.tile_box(index);
            // The tile's own fill is there: the tile is where its model said.
            let fill = count_near(&pixels, tile_box, palette.get_toolbar_background(), 2);
            assert!(
                fill * 3 > (tile_box.2 * tile_box.3) as usize,
                "{theme}: tile {index} is not drawn at {tile_box:?} (only {fill} px of its fill)"
            );
            // Every tab of the group has its shade in the ring: the model's segments reach the
            // screen, in the colours of the rows.
            let (_, _, _, _, _, _, shades) = MOCKUP[index];
            for shade in shades {
                let found = count_near(&pixels, tile_box, hex(*shade), 2);
                assert!(
                    found >= 20,
                    "{theme}: tile {index} ({}) shows {found} px of the shade {shade:#08X}",
                    tile.name
                );
            }
            // The halo is the group's tint.
            let halo = count_near(&pixels, tile_box, tile.tint, 2);
            assert!(
                halo >= 200,
                "{theme}: tile {index} shows {halo} px of its tint"
            );
        }

        // The mark of the active tab is on the first tile only, in the ink colour.
        let with_mark: Vec<usize> = rig
            .model
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, tile)| tile.has_mark)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(with_mark, [0], "{theme}: only Research has an active tab");
        let metrics = &layout.metrics;
        let (tile_x, tile_y, ..) = rig.tile_box(0);
        let mark = &rig.model.tiles[0];
        let ring_left = metrics.ring_cx - metrics.ring_size / 2.0;
        let ring_top = metrics.ring_cy - metrics.ring_size / 2.0;
        let (mark_x, mark_y) = (
            tile_x + (ring_left + mark.mark_x).round() as i32,
            tile_y + (ring_top + mark.mark_y).round() as i32,
        );
        // ... and it sits inside the ring, about 10.5 px from its centre (the mockup's dot).
        let distance = ((mark_x - tile_x) as f32 - metrics.ring_cx)
            .hypot((mark_y - tile_y) as f32 - metrics.ring_cy);
        assert!(
            (distance - 10.5).abs() < 1.5,
            "{theme}: the mark is {distance} px from the ring's centre"
        );
        let ink = palette.get_text();
        assert!(
            is_near(at(&pixels, mark_x, mark_y), ink, 12),
            "{theme}: the mark at ({mark_x}, {mark_y}) is not the ink colour"
        );
    }
}

#[test]
fn a_click_on_a_tile_reports_the_id_the_model_gave_it_and_nothing_else() {
    let rig = open(false, None, None);
    rig.show();
    for (index, tile) in rig.model.tiles.iter().enumerate() {
        let (x, y, width, _) = rig.tile_box(index);
        // The keycap's middle: inside the tile, away from the ring.
        let metrics = &rig.model.layout.metrics;
        rig.click(
            x + width / 2,
            y + (metrics.key_y + metrics.key_size / 2.0) as i32,
        );
        assert_eq!(
            rig.take_log(),
            [format!("tile:{}", tile.group)],
            "tile {index}"
        );
    }
    // The backdrop is outside the card: it dismisses.
    rig.click(100, 100);
    assert_eq!(rig.take_log(), ["dismissed"]);
}

#[test]
fn a_chosen_group_shows_the_numbered_tabs_of_the_model_and_a_click_on_one_reports_its_digit() {
    for dark in [false, true] {
        // Research has five tabs; the third is the chosen one.
        let rig = open(dark, Some(1), Some(3));
        let theme = if dark { "dark" } else { "light" };
        assert_eq!(rig.model.chips.len(), 5, "{theme}");
        assert_eq!(rig.model.chosen_name, "Research", "{theme}");

        let pixels = rig.show();
        rig.save(&pixels, "tab-chosen");
        let palette = rig.window.global::<ChromePalette>();

        // The chosen tile and the chosen chip have the accent border.
        let (tile_x, tile_y, tile_w, tile_h) = rig.tile_box(0);
        assert!(
            count_near(
                &pixels,
                (tile_x, tile_y, tile_w, tile_h),
                palette.get_accent(),
                2
            ) > 100,
            "{theme}: the chosen tile has the accent border"
        );
        let (card_x, card_y, ..) = rig.card();
        let chips_y = card_y + (rig.model.layout.tiles_bottom + 42.0).round() as i32;
        let chip_x = |index: i32| card_x + rig.model.layout.tiles_left.round() as i32 + index * 78;
        assert!(
            count_near(
                &pixels,
                (chip_x(2), chips_y, 72, 24),
                palette.get_accent(),
                2
            ) > 40,
            "{theme}: the chosen chip has the accent border"
        );
        assert_eq!(
            count_near(
                &pixels,
                (chip_x(1), chips_y, 72, 24),
                palette.get_accent(),
                2
            ),
            0,
            "{theme}: another chip has none"
        );

        // A click on each chip reports its digit, 1 to 5; the sixth place is empty.
        for digit in 1..=5 {
            rig.click(chip_x(digit - 1) + 36, chips_y + 12);
            assert_eq!(rig.take_log(), [format!("chip:{digit}")], "{theme}");
        }
        rig.click(chip_x(5) + 36, chips_y + 12);
        assert_eq!(
            rig.take_log(),
            Vec::<String>::new(),
            "{theme}: no sixth chip"
        );
    }
}
