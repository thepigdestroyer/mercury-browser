// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the "Switch group" card (`ui/groups/key-switcher.slint`, SPEC §4.7) with Slint's
//! software renderer at 1440 x 900, in the light and the dark chrome, and clicks it. No window is
//! opened. Set `MERCURY_UI_RENDER_DIR` to a directory to get one PNG per state and theme.
//!
//! The fixtures build the rows the way the card's Rust side does (`RingModel` with
//! `RingSpec::planet`, one `KeyRingSeg` per tab) with the tile measures of the mockup
//! (`design/ui-final/keys/keys-overlay-light.svg`): full tiles 108 x 168 with the ring centred at
//! (54, 46), compact tiles 72 x 134 from the seventh group on. The numbers asserted are the
//! mockup's; the colours asserted are read back from the window's own `ChromePalette`.
//!
//! The two parts of a card the renderer must get right are asserted by pixels: the halo (a circle
//! ring) and the tab ring (one filled `Path` of arcs per tab), at the middle of every segment.

use std::cell::{Cell, RefCell};
use std::f32::consts::PI;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use mercury_ui::ring_geometry::slint_models::RingModel;
use mercury_ui::ring_geometry::{Point, RingSpec};
use mercury_ui::{
    ChromePalette, ChromeTheme, GroupRow, KeyChip, KeyMetrics, KeyRingSeg, KeySwitcherPreview,
    KeyTile, TabRow,
};
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
const CARD_WIDTH: i32 = 760;

// ---------------------------------------------------------------------------------------------
// The platform: the software renderer, with a clock the tests move by hand (for the fade)
// ---------------------------------------------------------------------------------------------

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

/// Installs the platform on this test thread (each test runs on a thread of its own).
fn init() {
    READY.with(|ready| {
        if !ready.replace(true) {
            slint::platform::set_platform(Box::new(SoftwarePlatform))
                .expect("no platform is set on this thread yet");
        }
    });
}

/// Moves the clock forward and lets the animations see it.
fn advance(milliseconds: u64) {
    CLOCK.with(|clock| clock.set(clock.get() + Duration::from_millis(milliseconds)));
    update_timers_and_animations();
}

// ---------------------------------------------------------------------------------------------
// Fixtures: the groups of the mockup and what the card's Rust side would hand over
// ---------------------------------------------------------------------------------------------

fn hex(rgb: u32) -> Color {
    Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

#[derive(Clone)]
struct FixtureGroup {
    id: i32,
    key: String,
    name: String,
    /// Title and shade of each tab, in the group's order.
    tabs: Vec<(String, u32)>,
    /// The tint of the halo in the light and in the dark chrome.
    tint: (u32, u32),
    /// The tab that is the window's active one, which gets the mark.
    active: Option<usize>,
}

fn group(
    id: i32,
    key: &str,
    name: &str,
    tint: (u32, u32),
    active: Option<usize>,
    shades: &[u32],
) -> FixtureGroup {
    FixtureGroup {
        id,
        key: key.to_owned(),
        name: name.to_owned(),
        tabs: shades
            .iter()
            .enumerate()
            .map(|(index, shade)| (format!("{name} page {}", index + 1), *shade))
            .collect(),
        tint,
        active,
    }
}

/// The six groups of the mockup (the sixth is "Loose" there; the model has no key for that
/// pseudo-group, so the tests call it "Recipes"). Colours from `keys-overlay-light.svg` and
/// `-dark.svg`.
fn mockup_groups() -> Vec<FixtureGroup> {
    let mut research = group(
        1,
        "A",
        "Research",
        (0xDAE5F8, 0x223065),
        Some(0),
        &[0x2655A0, 0x2B62BB, 0x2F6FD6, 0x598CDE, 0x82A9E6],
    );
    research.tabs[0].0 = "Servo layout notes".to_owned();
    research.tabs[1].0 = "WebRender notes".to_owned();
    research.tabs[2].0 = "Slint rendering".to_owned();
    research.tabs[3].0 = "AccessKit".to_owned();
    research.tabs[4].0 = "Forum thread".to_owned();
    vec![
        research,
        group(
            2,
            "S",
            "Lisbon trip",
            (0xF9EBD9, 0x573A31),
            None,
            &[0xA76E23, 0xCD8528, 0xE4A046, 0xECBD7F],
        ),
        group(
            3,
            "D",
            "Reading",
            (0xE8E1F7, 0x392A63),
            None,
            &[0x60469B, 0x6C4FB0, 0x7857C5, 0x8868D4, 0x9D82DB, 0xB29DE3],
        ),
        group(
            4,
            "F",
            "Shopping",
            (0xF8DEE5, 0x552445),
            None,
            &[0xA23954, 0xD9486E, 0xE891A8],
        ),
        group(
            5,
            "G",
            "Work",
            (0xD9EEE3, 0x213E42),
            None,
            &[0x26774B, 0x82C5A1],
        ),
        group(
            6,
            "H",
            "Recipes",
            (0xE7E8EA, 0x39354D),
            None,
            &[0x5E6167, 0xB0B3B8],
        ),
    ]
}

/// `count` groups with the letters A to Z, for the compact tiles: one to five tabs each, shades
/// along the hue wheel.
fn many_groups(count: usize) -> Vec<FixtureGroup> {
    (0..count)
        .map(|index| {
            let letter = char::from(b'A' + (index % 26) as u8).to_string();
            let tab_count = 1 + index % 5;
            let hue = (index as f32 * 47.0) % 360.0;
            let shades: Vec<u32> = (0..tab_count)
                .map(|tab| hsl(hue, 0.55, 0.30 + 0.12 * tab as f32))
                .collect();
            group(
                index as i32 + 1,
                &letter,
                &format!("Group {letter}"),
                (hsl(hue, 0.60, 0.90), hsl(hue, 0.35, 0.22)),
                (index == 0).then_some(0),
                &shades,
            )
        })
        .collect()
}

/// An `0xRRGGBB` colour from hue (degrees), saturation and lightness.
fn hsl(hue: f32, saturation: f32, lightness: f32) -> u32 {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = (hue / 60.0) % 6.0;
    let x = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = lightness - chroma / 2.0;
    let channel = |value: f32| ((value + m) * 255.0).round() as u32;
    channel(r) << 16 | channel(g) << 8 | channel(b)
}

/// The inner measures of a tile and how tiles are laid out. Full tiles are the mockup's; the
/// compact ones are the lane brief's starting values (72 x 134, nine to a row).
#[derive(Clone, Copy)]
struct Measures {
    tile_width: f32,
    tile_height: f32,
    ring_size: f32,
    ring_cx: f32,
    ring_cy: f32,
    ring_radius: f32,
    ring_band: f32,
    halo_outer: f32,
    halo_width: f32,
    key_size: f32,
    key_y: f32,
    key_radius: f32,
    key_font: f32,
    name_y: f32,
    name_font: f32,
    count_y: f32,
    count_font: f32,
    gap_x: f32,
    gap_y: f32,
    columns: usize,
}

const FULL: Measures = Measures {
    tile_width: 108.0,
    tile_height: 168.0,
    ring_size: 48.0,
    ring_cx: 54.0,
    ring_cy: 46.0,
    ring_radius: 20.0,
    ring_band: 7.0,
    halo_outer: 47.5,
    halo_width: 22.0,
    key_size: 40.0,
    key_y: 88.0,
    key_radius: 9.0,
    key_font: 20.0,
    name_y: 136.0,
    name_font: 12.5,
    count_y: 152.0,
    count_font: 10.5,
    gap_x: 10.0,
    gap_y: 10.0,
    columns: 6,
};

const COMPACT: Measures = Measures {
    tile_width: 72.0,
    tile_height: 134.0,
    ring_size: 34.0,
    ring_cx: 36.0,
    ring_cy: 33.0,
    ring_radius: 14.0,
    ring_band: 5.0,
    halo_outer: 33.0,
    halo_width: 15.0,
    key_size: 28.0,
    key_y: 70.0,
    key_radius: 7.0,
    key_font: 14.0,
    name_y: 104.0,
    name_font: 11.0,
    count_y: 118.0,
    count_font: 9.5,
    gap_x: 6.0,
    gap_y: 8.0,
    columns: 9,
};

const TILES_LEFT: f32 = 30.0;
const TILES_TOP: f32 = 66.0;

/// Where the tiles go for a number of groups: from the tile area's top-left, and the card's
/// height (`tiles_bottom + 96`; a made-up 200 without tiles).
struct Layout {
    measures: Measures,
    positions: Vec<(f32, f32)>,
    tiles_bottom: f32,
    card_height: f32,
}

fn layout(count: usize) -> Layout {
    let measures = if count <= 6 { FULL } else { COMPACT };
    let positions: Vec<(f32, f32)> = (0..count)
        .map(|index| {
            (
                (index % measures.columns) as f32 * (measures.tile_width + measures.gap_x),
                (index / measures.columns) as f32 * (measures.tile_height + measures.gap_y),
            )
        })
        .collect();
    let rows = count.div_ceil(measures.columns);
    let tiles_bottom = TILES_TOP
        + if rows == 0 {
            0.0
        } else {
            rows as f32 * measures.tile_height + (rows - 1) as f32 * measures.gap_y
        };
    Layout {
        measures,
        positions,
        tiles_bottom,
        card_height: if count == 0 {
            200.0
        } else {
            tiles_bottom + 96.0
        },
    }
}

fn metrics_of(m: &Measures) -> KeyMetrics {
    KeyMetrics {
        tile_width: m.tile_width,
        tile_height: m.tile_height,
        ring_size: m.ring_size,
        ring_cx: m.ring_cx,
        ring_cy: m.ring_cy,
        halo_outer: m.halo_outer,
        halo_width: m.halo_width,
        key_size: m.key_size,
        key_y: m.key_y,
        key_radius: m.key_radius,
        key_font: m.key_font,
        name_y: m.name_y,
        name_font: m.name_font,
        count_y: m.count_y,
        count_font: m.count_font,
    }
}

/// A tile, and where the ring's segments are (ring-local middle of each, in ring order).
struct TileFixture {
    tile: KeyTile,
    middles: Vec<(f32, f32)>,
    shades: Vec<Color>,
}

fn count_text(tabs: usize) -> String {
    if tabs == 1 {
        "1 tab".to_owned()
    } else {
        format!("{tabs} tabs")
    }
}

fn tile_fixture(
    group: &FixtureGroup,
    position: (f32, f32),
    m: &Measures,
    dark: bool,
) -> TileFixture {
    let group_row = GroupRow {
        id: group.id,
        name: group.name.as_str().into(),
        key_label: group.key.as_str().into(),
        count: group.tabs.len() as i32,
        ..Default::default()
    };
    let tabs: Vec<TabRow> = group
        .tabs
        .iter()
        .enumerate()
        .map(|(index, (title, shade))| TabRow {
            id: group.id * 1000 + index as i32,
            group: group.id,
            index_in_group: index as i32,
            title: title.as_str().into(),
            shade: hex(*shade),
            active: group.active == Some(index),
            ..Default::default()
        })
        .collect();
    let centre = m.ring_size / 2.0;
    let model = RingModel::new(
        RingSpec::planet(Point::new(centre, centre), m.ring_radius, m.ring_band),
        &[group_row],
        &tabs,
    );
    let arcs = model.segments_for_slint();
    let shades: Vec<Color> = (0..arcs.len())
        .map(|flat| tabs[model.tab_rows[flat]].shade)
        .collect();
    let segments: Vec<KeyRingSeg> = arcs
        .iter()
        .zip(&shades)
        .map(|(arc, shade)| KeyRingSeg {
            commands: arc.commands.clone(),
            shade: *shade,
        })
        .collect();
    let middles: Vec<(f32, f32)> = arcs.iter().map(|arc| (arc.mid_x, arc.mid_y)).collect();
    // The mark: the active tab's middle, pulled in to about half the ring's radius (ring-local).
    let mark = group
        .active
        .and_then(|active| model.tab_rows.iter().position(|row| *row == active))
        .map(|flat| {
            let (mx, my) = middles[flat];
            (
                centre + (mx - centre) * 0.525,
                centre + (my - centre) * 0.525,
            )
        });
    TileFixture {
        tile: KeyTile {
            group: group.id,
            x: position.0,
            y: position.1,
            key_label: group.key.as_str().into(),
            name: group.name.as_str().into(),
            count_text: count_text(group.tabs.len()).into(),
            accessible_label: format!(
                "Option {}, {}, {}",
                group.key,
                group.name,
                count_text(group.tabs.len())
            )
            .into(),
            tint: hex(if dark { group.tint.1 } else { group.tint.0 }),
            segments: ModelRc::new(VecModel::from(segments)),
            has_mark: mark.is_some(),
            mark_x: mark.map_or(0.0, |m| m.0),
            mark_y: mark.map_or(0.0, |m| m.1),
        },
        middles,
        shades,
    }
}

const HINT_PRIMARY: &str =
    "Tap a letter for a group, then 1–9 for one of its tabs. Release ⌥ to go there.";
const HINT_SECONDARY: &str = "Opens after ⌥ is held for 0.4 s on its own, so quick ⌥ + letter still types å, ß, ∂ in text fields.";
const HINT_QUARTER: &str = "Opens after ⌥ is held for 0.25 s on its own, so quick ⌥ + letter still types å, ß, ∂ in text fields.";
const HINT_ALWAYS: &str = "⌥ + a group letter always switches groups in Mercury, so å, ß and ∂ cannot be typed here. Change this in Settings.";
const EMPTY_TEXT: &str = "No group has a key yet. Choose Change keys… to give each group a letter.";

/// Everything the card shows.
#[derive(Clone)]
struct Scene {
    groups: Vec<FixtureGroup>,
    selected_group: Option<i32>,
    /// The chosen digit, 1 to 9.
    selected_tab: Option<i32>,
    secondary: String,
    /// How many chips are handed over (the card's Rust side hands over at most nine).
    chip_limit: usize,
    reduce_motion: bool,
}

impl Scene {
    fn new(groups: Vec<FixtureGroup>) -> Self {
        Self {
            groups,
            selected_group: None,
            selected_tab: None,
            secondary: HINT_SECONDARY.to_owned(),
            chip_limit: 9,
            reduce_motion: true,
        }
    }

    fn chosen(mut self, group: i32, tab: Option<i32>) -> Self {
        self.selected_group = Some(group);
        self.selected_tab = tab;
        self
    }
}

// ---------------------------------------------------------------------------------------------
// The rig: a preview window with the scene applied, the callbacks logged
// ---------------------------------------------------------------------------------------------

struct Rig {
    window: KeySwitcherPreview,
    layout: Layout,
    scene: Scene,
    dark: bool,
    tiles: Vec<TileFixture>,
    log: Rc<RefCell<Vec<String>>>,
}

fn apply(window: &KeySwitcherPreview, scene: &Scene, dark: bool) -> (Layout, Vec<TileFixture>) {
    let layout = layout(scene.groups.len());
    let tiles: Vec<TileFixture> = scene
        .groups
        .iter()
        .zip(&layout.positions)
        .map(|(group, position)| tile_fixture(group, *position, &layout.measures, dark))
        .collect();
    let chosen = scene
        .selected_group
        .and_then(|id| scene.groups.iter().find(|group| group.id == id));
    let chips: Vec<KeyChip> = chosen
        .map(|group| {
            group
                .tabs
                .iter()
                .take(scene.chip_limit)
                .enumerate()
                .map(|(index, (title, shade))| KeyChip {
                    index: index as i32 + 1,
                    title: title.as_str().into(),
                    shade: hex(*shade),
                    accessible_label: format!("Tab {}, {title}", index + 1).into(),
                })
                .collect()
        })
        .unwrap_or_default();

    window.set_reduce_motion(scene.reduce_motion);
    window.set_tiles(ModelRc::new(VecModel::from(
        tiles.iter().map(|t| t.tile.clone()).collect::<Vec<_>>(),
    )));
    window.set_chips(ModelRc::new(VecModel::from(chips)));
    window.set_metrics(metrics_of(&layout.measures));
    window.set_selected_group(scene.selected_group.unwrap_or(-1));
    window.set_selected_tab(scene.selected_tab.unwrap_or(-1));
    window.set_chosen_name(chosen.map_or("", |group| group.name.as_str()).into());
    window.set_hint_primary(HINT_PRIMARY.into());
    window.set_hint_secondary(scene.secondary.as_str().into());
    window.set_empty_text(EMPTY_TEXT.into());
    window.set_card_width(CARD_WIDTH as f32);
    window.set_card_height(layout.card_height);
    window.set_tiles_left(TILES_LEFT);
    window.set_tiles_top(TILES_TOP);
    (layout, tiles)
}

/// Opens the preview with the scene (not yet shown), in the light or the dark chrome.
fn open(scene: &Scene, dark: bool) -> Rig {
    init();
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
    window.on_page_clicked({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("page".to_owned())
    });
    let (layout, tiles) = apply(&window, scene, dark);
    window.show().expect("the preview window can be shown");
    Rig {
        window,
        layout,
        scene: scene.clone(),
        dark,
        tiles,
        log,
    }
}

impl Rig {
    fn snapshot(&self) -> Pixels {
        self.window
            .window()
            .take_snapshot()
            .expect("the window renders")
    }

    /// Shows the card and renders it once the fade is over.
    fn show(&self) -> Pixels {
        self.window.set_shown(true);
        let _ = self.snapshot();
        advance(300);
        self.snapshot()
    }

    fn palette(&self) -> Colours {
        let palette = self.window.global::<ChromePalette>();
        Colours {
            sheet: palette.get_sheet_background(),
            border: palette.get_popup_border(),
            tile: palette.get_toolbar_background(),
            chosen_tile: palette.get_item_hover(),
            accent: palette.get_accent(),
            keycap: palette.get_field_background(),
            keycap_border: palette.get_text_secondary(),
            text: palette.get_text(),
            scrim: palette.get_scrim(),
            page: palette.get_content_background(),
        }
    }

    /// The card's rectangle in the window: centred, `(x, y, width, height)`.
    fn card(&self) -> (i32, i32, i32, i32) {
        let height = self.layout.card_height.round() as i32;
        (
            ((WINDOW_WIDTH - CARD_WIDTH) as f32 / 2.0).round() as i32,
            ((WINDOW_HEIGHT - height) as f32 / 2.0).round() as i32,
            CARD_WIDTH,
            height,
        )
    }

    /// The top-left of tile `index` in the window.
    fn tile_origin(&self, index: usize) -> (i32, i32) {
        let (cx, cy, ..) = self.card();
        let (x, y) = self.layout.positions[index];
        (
            (cx as f32 + TILES_LEFT + x) as i32,
            (cy as f32 + TILES_TOP + y) as i32,
        )
    }

    fn save(&self, pixels: &Pixels, state: &str) {
        save(
            pixels,
            &format!("key-switcher-{state}-{}.png", theme_name(self.dark)),
        );
    }

    fn click(&self, x: f32, y: f32) {
        click(&self.window, x, y);
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }
}

fn theme_name(dark: bool) -> &'static str {
    if dark { "dark" } else { "light" }
}

struct Colours {
    sheet: Color,
    border: Color,
    tile: Color,
    chosen_tile: Color,
    accent: Color,
    keycap: Color,
    keycap_border: Color,
    text: Color,
    scrim: Color,
    page: Color,
}

/// Writes the PNG only when `MERCURY_UI_RENDER_DIR` names a directory.
fn save(pixels: &Pixels, name: &str) {
    let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR").map(PathBuf::from) else {
        return;
    };
    image::save_buffer_with_format(
        directory.join(name),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .expect("the rendering can be saved");
}

fn click(window: &KeySwitcherPreview, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}

// ---------------------------------------------------------------------------------------------
// Pixels
// ---------------------------------------------------------------------------------------------

fn at(pixels: &Pixels, x: i32, y: i32) -> Rgba8Pixel {
    pixels.as_slice()[(y as u32 * pixels.width() + x as u32) as usize]
}

fn is_near(pixel: Rgba8Pixel, colour: Color, tolerance: u8) -> bool {
    pixel.r.abs_diff(colour.red()) <= tolerance
        && pixel.g.abs_diff(colour.green()) <= tolerance
        && pixel.b.abs_diff(colour.blue()) <= tolerance
}

fn show_pixel(pixel: Rgba8Pixel) -> String {
    format!("#{:02x}{:02x}{:02x}", pixel.r, pixel.g, pixel.b)
}

fn show_colour(colour: Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        colour.red(),
        colour.green(),
        colour.blue()
    )
}

/// Asserts that the pixel is within `tolerance` of the colour.
#[track_caller]
fn assert_pixel(pixels: &Pixels, (x, y): (i32, i32), colour: Color, tolerance: u8, what: &str) {
    let pixel = at(pixels, x, y);
    assert!(
        is_near(pixel, colour, tolerance),
        "{what}: the pixel at ({x}, {y}) is {}, expected about {}",
        show_pixel(pixel),
        show_colour(colour)
    );
}

/// Pixels of a box `(x0, y0, x1, y1)` (end exclusive) that differ from `background`.
fn ink(pixels: &Pixels, (x0, y0, x1, y1): (i32, i32, i32, i32), background: Color) -> usize {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| !is_near(at(pixels, *x, *y), background, 6))
        .count()
}

/// Pixels of a box that are within `tolerance` of the colour.
fn count_near(
    pixels: &Pixels,
    (x0, y0, x1, y1): (i32, i32, i32, i32),
    colour: Color,
    tolerance: u8,
) -> usize {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| is_near(at(pixels, *x, *y), colour, tolerance))
        .count()
}

/// What the scrim makes of a page colour: the scrim laid over it.
fn under_scrim(page: Color, scrim: Color) -> Color {
    let alpha = scrim.alpha() as f32 / 255.0;
    let mix =
        |under: u8, over: u8| (under as f32 * (1.0 - alpha) + over as f32 * alpha).round() as u8;
    Color::from_rgb_u8(
        mix(page.red(), scrim.red()),
        mix(page.green(), scrim.green()),
        mix(page.blue(), scrim.blue()),
    )
}

/// The card's rectangle found in the picture: the outermost pixels of the border colour, seen
/// from the middle column of the top and bottom edges and from a row near the bottom.
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
// What every shown state must satisfy
// ---------------------------------------------------------------------------------------------

/// The card where the layout puts it, the scrim over the page, every tile where the model says,
/// its halo, its ring, its keycap and (if it has one) its mark, and the margins free of ink.
fn check_card(rig: &Rig, pixels: &Pixels, state: &str) {
    let what = format!("{state} ({})", theme_name(rig.dark));
    let colours = rig.palette();
    let (card_x, card_y, card_width, card_height) = rig.card();

    // The card is the centred rectangle of the layout, to the pixel.
    assert_eq!(
        find_card(pixels, colours.border),
        (card_x, card_y, card_width, card_height),
        "{what}: the card's rectangle"
    );
    assert!(
        card_x >= 0 && card_y >= 0 && card_y + card_height <= WINDOW_HEIGHT,
        "{what}: the card lies inside the window"
    );
    assert_pixel(
        pixels,
        (card_x + 380, card_y + 3),
        colours.sheet,
        2,
        &format!("{what}: the card's own background"),
    );

    // The scrim covers the page.
    let dimmed = under_scrim(colours.page, colours.scrim);
    for point in [(20, 500), (1420, 880), (1400, 300), (100, 700)] {
        assert_pixel(
            pixels,
            point,
            dimmed,
            3,
            &format!("{what}: the scrim over the page"),
        );
    }

    // The tiles.
    let m = rig.layout.measures;
    for (index, fixture) in rig.tiles.iter().enumerate() {
        let (ox, oy) = rig.tile_origin(index);
        let chosen = rig.scene.selected_group == Some(fixture.tile.group);
        let tile_colour = if chosen {
            colours.chosen_tile
        } else {
            colours.tile
        };
        let name = &rig.scene.groups[index].name;
        let which = format!("{what}: tile {index} ({name})");

        // Its left edge, 1 px (2 px when chosen), and the card's background just outside it.
        let edge_y = oy + (m.tile_height * 0.75) as i32;
        let edge = if chosen {
            colours.accent
        } else {
            colours.border
        };
        assert_pixel(pixels, (ox, edge_y), edge, 3, &format!("{which}: border"));
        assert_pixel(
            pixels,
            (ox + 1, edge_y),
            if chosen { colours.accent } else { tile_colour },
            3,
            &format!("{which}: second pixel"),
        );
        assert_pixel(
            pixels,
            (ox - 2, edge_y),
            colours.sheet,
            2,
            &format!("{which}: the card's background outside the tile"),
        );
        let bare = (ox + 5, oy + (m.key_y + m.key_size / 2.0) as i32);
        assert_pixel(pixels, bare, tile_colour, 3, &format!("{which}: fill"));

        // The halo: the group's tint at the middle of its band, the tile's fill in the gap
        // between the halo and the ring.
        let cx = ox + m.ring_cx as i32;
        let cy = oy + m.ring_cy as i32;
        let halo_mid = m.halo_outer - m.halo_width / 2.0;
        assert_pixel(
            pixels,
            (cx + halo_mid as i32, cy),
            fixture.tile.tint,
            3,
            &format!("{which}: halo"),
        );
        assert_pixel(
            pixels,
            (cx - halo_mid as i32 - 1, cy),
            fixture.tile.tint,
            3,
            &format!("{which}: halo, left"),
        );
        let gap = m.halo_outer - m.halo_width - 1.0;
        assert_pixel(
            pixels,
            (cx + gap as i32, cy),
            tile_colour,
            8,
            &format!("{which}: the gap between ring and halo"),
        );

        // The ring: every segment is filled in its shade at its middle (it is a `Path` of arcs).
        let ring_left = ox as f32 + m.ring_cx - m.ring_size / 2.0;
        let ring_top = oy as f32 + m.ring_cy - m.ring_size / 2.0;
        for (flat, (mid_x, mid_y)) in fixture.middles.iter().enumerate() {
            let point = ((ring_left + mid_x) as i32, (ring_top + mid_y) as i32);
            assert_pixel(
                pixels,
                point,
                fixture.shades[flat],
                4,
                &format!("{which}: segment {flat} at its middle"),
            );
        }
        // And its area: a segment of n has the ring's area over n, less its gap.
        let n = fixture.middles.len() as f32;
        let (outer, inner) = (
            m.ring_radius + m.ring_band / 2.0,
            m.ring_radius - m.ring_band / 2.0,
        );
        let sweep = 360.0 / n - 3.2;
        let want = PI / 360.0 * (outer * outer - inner * inner) * sweep;
        let ring_box = (
            cx - outer as i32 - 2,
            cy - outer as i32 - 2,
            cx + outer as i32 + 3,
            cy + outer as i32 + 3,
        );
        for (flat, shade) in fixture.shades.iter().enumerate() {
            // Skip a shade shared with a neighbour (then the count is of both).
            if fixture
                .shades
                .iter()
                .filter(|other| *other == shade)
                .count()
                > 1
            {
                continue;
            }
            let got = count_near(pixels, ring_box, *shade, 2) as f32;
            assert!(
                got > want * 0.65 && got < want * 1.05,
                "{which}: segment {flat} covers {got} px, a segment of {n} should cover about {want:.0}"
            );
        }

        // The mark of the active tab: an ink dot, at the place the model gives.
        if fixture.tile.has_mark {
            let point = (
                (ring_left + fixture.tile.mark_x) as i32,
                (ring_top + fixture.tile.mark_y) as i32,
            );
            assert_pixel(
                pixels,
                point,
                colours.text,
                8,
                &format!("{which}: the mark"),
            );
        } else {
            // Without a mark the middle of the ring's hole is the tile's fill.
            assert_pixel(
                pixels,
                (cx, cy),
                tile_colour,
                3,
                &format!("{which}: no mark"),
            );
        }

        // The keycap: its border where the metrics say, a letter inside it and nothing of the
        // letter outside.
        let key_x = ox + ((m.tile_width - m.key_size) / 2.0) as i32;
        let key_y = oy + m.key_y as i32;
        let key = m.key_size as i32;
        assert_pixel(
            pixels,
            (key_x, key_y + key / 2),
            colours.keycap_border,
            4,
            &format!("{which}: keycap border"),
        );
        assert_pixel(
            pixels,
            (key_x + 4, key_y + 4),
            colours.keycap,
            3,
            &format!("{which}: keycap fill"),
        );
        let inside = (key_x + 3, key_y + 3, key_x + key - 3, key_y + key - 3);
        let letter = ink(pixels, inside, colours.keycap);
        assert!(
            letter >= 15,
            "{which}: the keycap shows its letter ({letter} px of ink)"
        );

        // Name and count stay inside the tile: nothing in its 4 px side padding or in the gap.
        let text_rows = (oy + m.name_y as i32 - 1, oy + m.tile_height as i32 - 1);
        // (The padding strips stop above the tile's rounded corners, 12 px from its bottom.)
        let padding_end = text_rows.1 - 12;
        for strip in [
            (ox + 2, text_rows.0, ox + 4, padding_end),
            (
                ox + m.tile_width as i32 - 4,
                text_rows.0,
                ox + m.tile_width as i32 - 2,
                padding_end,
            ),
        ] {
            assert_eq!(
                ink(pixels, strip, tile_colour),
                0,
                "{which}: no text in the tile's padding {strip:?}"
            );
        }
        let right = ox + m.tile_width as i32;
        let gap_to_next = (
            right + 1,
            text_rows.0,
            right + m.gap_x as i32 - 1,
            text_rows.1,
        );
        if (index + 1) % m.columns != 0 && index + 1 < rig.tiles.len() {
            assert_eq!(
                ink(pixels, gap_to_next, colours.sheet),
                0,
                "{which}: nothing between this tile and the next {gap_to_next:?}"
            );
        }
    }

    // The margins of the card: no ink left of the tiles or right of them, from the tiles down.
    let margin_top = card_y + TILES_TOP as i32 + 40;
    let margin_bottom = card_y + card_height - 13;
    if !rig.tiles.is_empty() {
        for strip in [
            (
                card_x + 2,
                margin_top,
                card_x + TILES_LEFT as i32 - 2,
                margin_bottom,
            ),
            (
                card_x + CARD_WIDTH - 28,
                margin_top,
                card_x + CARD_WIDTH - 2,
                margin_bottom,
            ),
        ] {
            assert_eq!(
                ink(pixels, strip, colours.sheet),
                0,
                "{what}: nothing in the card's margin {strip:?}"
            );
        }
        // The first hint is there, under the last row of tiles.
        let hint = (
            card_x + 30,
            card_y + rig.layout.tiles_bottom as i32 + 14,
            card_x + 730,
            card_y + rig.layout.tiles_bottom as i32 + 40,
        );
        assert!(
            ink(pixels, hint, colours.sheet) > 150,
            "{what}: the first hint is drawn under the tiles"
        );
        // The second hint or the chips are there, and nothing lies below them but padding.
        let bottom_rows = (
            card_x + 14,
            card_y + rig.layout.tiles_bottom as i32 + 80,
            card_x + CARD_WIDTH - 14,
            card_y + card_height - 2,
        );
        assert_eq!(
            ink(pixels, bottom_rows, colours.sheet),
            0,
            "{what}: nothing below the hints and chips"
        );
    }

    // The header: the title, the chip and the link, each with ink in its place.
    let header = |x0: i32, x1: i32| (card_x + x0, card_y + 20, card_x + x1, card_y + 50);
    assert!(
        ink(pixels, header(30, 160), colours.sheet) > 100,
        "{what}: the title"
    );
    assert!(
        ink(pixels, header(170, 270), colours.sheet) > 100,
        "{what}: the held chip"
    );
    assert!(
        ink(pixels, header(620, 735), colours.sheet) > 80,
        "{what}: Change keys"
    );
    assert_eq!(
        ink(pixels, header(738, 756), colours.sheet),
        0,
        "{what}: the link ends 30 px from the card's edge"
    );
}

/// The chips row: chip `index` (0-based) as a rectangle in the window, `(x, y, width, height)`.
fn chip_box(rig: &Rig, index: usize) -> (i32, i32, i32, i32) {
    let (card_x, card_y, ..) = rig.card();
    (
        card_x + 30 + index as i32 * 78,
        card_y + rig.layout.tiles_bottom as i32 + 42,
        72,
        24,
    )
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn six_groups_at_rest_are_drawn_to_the_numbers_of_the_mockup() {
    for dark in [false, true] {
        let rig = open(&Scene::new(mockup_groups()), dark);
        let pixels = rig.show();
        rig.save(&pixels, "rest");
        check_card(&rig, &pixels, "six groups at rest");

        // The card is 760 x 330 at (340, 285) here, and the tiles end at x 728 of it.
        assert_eq!(rig.card(), (340, 285, 760, 330));
        let colours = rig.palette();
        let (card_x, card_y, ..) = rig.card();
        let last = rig.tile_origin(5);
        assert_eq!(last.0 - card_x + 108, 728, "six tiles end at x 728");
        // Only the first group has an active tab: one mark in the six rings.
        let marks: Vec<bool> = rig.tiles.iter().map(|t| t.tile.has_mark).collect();
        assert_eq!(marks, [true, false, false, false, false, false]);
        // The held chip is the primary button's colour; the link is the accent.
        let palette = rig.window.global::<ChromePalette>();
        assert_pixel(
            &pixels,
            (card_x + 180 + 6, card_y + 36),
            palette.get_primary_button_background(),
            3,
            "the held chip's fill",
        );
        assert!(
            count_near(
                &pixels,
                (card_x + 600, card_y + 20, card_x + 740, card_y + 50),
                colours.accent,
                40
            ) > 40,
            "the link is drawn in the accent colour"
        );
    }
}

#[test]
fn a_chosen_group_has_the_accent_border_and_shows_its_numbered_tabs() {
    for dark in [false, true] {
        let rig = open(&Scene::new(mockup_groups()).chosen(1, None), dark);
        let pixels = rig.show();
        rig.save(&pixels, "group-chosen");
        check_card(&rig, &pixels, "a group chosen");
        let colours = rig.palette();

        // Five chips, not the second hint: ink in the chips' row, and none of the hint's text
        // past the fifth chip.
        let (card_x, card_y, ..) = rig.card();
        let row_y = card_y + rig.layout.tiles_bottom as i32 + 54;
        for index in 0..5 {
            let (x, y, width, height) = chip_box(&rig, index);
            // The unchosen chip's 1 px border on its top edge and its tab's shade as a dot.
            assert_pixel(
                &pixels,
                (x + 36, y),
                colours.border,
                3,
                &format!("chip {index}: border"),
            );
            let shade = rig.tiles[0].shades[index];
            assert_pixel(
                &pixels,
                (x + 26, y + height / 2),
                shade,
                4,
                &format!("chip {index}: shade dot"),
            );
            assert!(
                ink(
                    &pixels,
                    (x + 34, y + 6, x + width - 6, y + height - 6),
                    colours.tile
                ) > 12,
                "chip {index}: its title"
            );
        }
        assert_eq!(
            ink(
                &pixels,
                (chip_box(&rig, 5).0, row_y - 8, card_x + 730, row_y + 8),
                colours.sheet
            ),
            0,
            "no chip and no hint after the fifth chip"
        );
        // Only the chosen tile has the accent border (checked tile by tile in `check_card`).
        let chosen = rig.tile_origin(0);
        assert_pixel(
            &pixels,
            (chosen.0, chosen.1 + 120),
            colours.accent,
            3,
            "the chosen tile's border",
        );
        let other = rig.tile_origin(1);
        assert_pixel(
            &pixels,
            (other.0, other.1 + 120),
            colours.border,
            3,
            "another tile's border",
        );
    }
}

#[test]
fn a_chosen_tab_has_the_accent_border_on_its_chip_alone() {
    for dark in [false, true] {
        let rig = open(&Scene::new(mockup_groups()).chosen(1, Some(3)), dark);
        let pixels = rig.show();
        rig.save(&pixels, "tab-chosen");
        check_card(&rig, &pixels, "a tab chosen");
        let colours = rig.palette();
        for index in 0..5 {
            let (x, y, ..) = chip_box(&rig, index);
            let chosen = index == 2;
            let want = if chosen {
                colours.accent
            } else {
                colours.border
            };
            assert_pixel(
                &pixels,
                (x + 36, y),
                want,
                3,
                &format!("chip {index}: top border (chosen: {chosen})"),
            );
            assert_pixel(
                &pixels,
                (x + 36, y + 1),
                if chosen { colours.accent } else { colours.tile },
                3,
                &format!("chip {index}: second row (chosen: {chosen})"),
            );
            assert_pixel(
                &pixels,
                (x + 36, y + 3),
                if chosen {
                    colours.chosen_tile
                } else {
                    colours.tile
                },
                3,
                &format!("chip {index}: fill (chosen: {chosen})"),
            );
        }
    }
}

#[test]
fn nine_groups_use_compact_tiles_in_one_row() {
    for dark in [false, true] {
        let rig = open(&Scene::new(many_groups(9)), dark);
        let pixels = rig.show();
        rig.save(&pixels, "nine-groups");
        check_card(&rig, &pixels, "nine groups");
        assert_eq!(rig.card(), (340, 302, 760, 296), "one compact row: 296 px");
        let first = rig.tile_origin(0);
        let last = rig.tile_origin(8);
        assert_eq!(last.1, first.1, "one row");
        assert_eq!(
            last.0 - first.0,
            8 * 78,
            "compact tiles are 72 wide with 6 between"
        );
    }
}

#[test]
fn twenty_six_groups_use_three_rows_and_the_card_stays_inside_the_window() {
    for dark in [false, true] {
        let rig = open(&Scene::new(many_groups(26)), dark);
        let pixels = rig.show();
        rig.save(&pixels, "twenty-six-groups");
        check_card(&rig, &pixels, "twenty-six groups");
        assert_eq!(
            rig.card(),
            (340, 160, 760, 580),
            "three compact rows: 580 px"
        );
        let rows: Vec<i32> = (0..26).map(|index| rig.tile_origin(index).1).collect();
        let mut distinct = rows.clone();
        distinct.dedup();
        assert_eq!(distinct.len(), 3, "three rows of tiles");
        assert!(rig.card().1 >= 0 && rig.card().1 + rig.card().3 <= WINDOW_HEIGHT);
    }
}

#[test]
fn one_group_with_one_tab_draws_a_nearly_full_ring_and_its_halo() {
    for dark in [false, true] {
        let single = group(7, "Q", "Solo", (0xE7E8EA, 0x39354D), Some(0), &[0x2F6FD6]);
        let rig = open(&Scene::new(vec![single.clone()]), dark);
        let pixels = rig.show();
        rig.save(&pixels, "one-group-one-tab");
        check_card(&rig, &pixels, "one group with one tab");

        // The ring is one arc of 356.8 degrees: all but the gap at 12 o'clock is the shade.
        let m = rig.layout.measures;
        let (ox, oy) = rig.tile_origin(0);
        let (cx, cy) = (ox as f32 + m.ring_cx, oy as f32 + m.ring_cy);
        let shade = hex(single.tabs[0].1);
        let samples = 72;
        let drawn = (0..samples)
            .filter(|step| {
                let angle = (*step as f32 + 0.5) * 360.0 / samples as f32;
                let (sin, cos) = angle.to_radians().sin_cos();
                let x = (cx + m.ring_radius * sin).round() as i32;
                let y = (cy - m.ring_radius * cos).round() as i32;
                is_near(at(&pixels, x, y), shade, 6)
            })
            .count();
        assert!(
            drawn >= 70,
            "the one-tab ring is drawn all the way round ({drawn} of {samples} samples)"
        );
        // The halo is a full circle too: the tint all round at the middle of its band.
        let tint = rig.tiles[0].tile.tint;
        let halo_mid = m.halo_outer - m.halo_width / 2.0;
        let halo = (0..samples)
            .filter(|step| {
                let angle = (*step as f32 + 0.5) * 360.0 / samples as f32;
                let (sin, cos) = angle.to_radians().sin_cos();
                let x = (cx + halo_mid * sin).round() as i32;
                let y = (cy - halo_mid * cos).round() as i32;
                // The keycap covers the halo's bottom: count only where it does not.
                let in_keycap = y >= oy + m.key_y as i32 - 2;
                in_keycap || is_near(at(&pixels, x, y), tint, 6)
            })
            .count();
        assert_eq!(
            halo, samples,
            "the halo is a full circle of the group's tint"
        );
        // One tab: the chip row of the chosen group has one chip.
        let chosen = open(&Scene::new(vec![single]).chosen(7, None), dark);
        let chosen_pixels = chosen.show();
        chosen.save(&chosen_pixels, "one-group-one-tab-chosen");
        check_card(&chosen, &chosen_pixels, "one group with one tab, chosen");
        let (x, y, width, height) = chip_box(&chosen, 0);
        assert!(
            ink(
                &chosen_pixels,
                (x + 34, y + 6, x + width - 6, y + height - 6),
                chosen.palette().tile
            ) > 6,
            "the single chip shows its title"
        );
        assert_eq!(
            ink(
                &chosen_pixels,
                (
                    chip_box(&chosen, 1).0,
                    y - 2,
                    chip_box(&chosen, 1).0 + 200,
                    y + height + 2
                ),
                chosen.palette().sheet
            ),
            0,
            "no second chip"
        );
    }
}

#[test]
fn no_keyed_groups_show_the_empty_text_and_no_tiles() {
    for dark in [false, true] {
        let rig = open(&Scene::new(Vec::new()), dark);
        let pixels = rig.show();
        rig.save(&pixels, "no-groups");
        check_card(&rig, &pixels, "no keyed groups");
        let colours = rig.palette();
        let (card_x, card_y, _, card_height) = rig.card();
        // The words are in the tile area, and neither hint is drawn.
        let area = (
            card_x + 30,
            card_y + 66,
            card_x + 730,
            card_y + card_height - 30,
        );
        assert!(
            ink(&pixels, area, colours.sheet) > 400,
            "the empty text is drawn"
        );
        assert_eq!(
            ink(
                &pixels,
                (
                    card_x + 14,
                    card_y + card_height - 28,
                    card_x + CARD_WIDTH - 14,
                    card_y + card_height - 2
                ),
                colours.sheet
            ),
            0,
            "nothing below the empty text"
        );
        // A click in the tile area is a click on the card: it is kept.
        rig.click(card_x as f32 + 380.0, card_y as f32 + 100.0);
        assert!(rig.take_log().is_empty());
    }
}

#[test]
fn long_names_cjk_and_emoji_are_elided_inside_their_tiles() {
    let long = "A group whose name is far too long to fit in a tile of the card";
    let sixty_four = "x".repeat(64);
    let cjk = "日本語のとても長いグループの名前をここに書きます";
    let emoji = "🚀🔥🎉 Launch 🚀🔥🎉 plans 🎉🔥🚀";
    let rtl = "مجموعة طويلة جدا باللغة العربية";
    let mut groups = vec![
        group(
            1,
            "A",
            long,
            (0xDAE5F8, 0x223065),
            Some(0),
            &[0x2655A0, 0x2B62BB],
        ),
        group(2, "B", &sixty_four, (0xF9EBD9, 0x573A31), None, &[0xA76E23]),
        group(
            3,
            "C",
            cjk,
            (0xE8E1F7, 0x392A63),
            None,
            &[0x60469B, 0x6C4FB0, 0x7857C5],
        ),
        group(
            4,
            "D",
            emoji,
            (0xF8DEE5, 0x552445),
            None,
            &[0xA23954, 0xD9486E],
        ),
        group(
            5,
            "E",
            rtl,
            (0xD9EEE3, 0x213E42),
            None,
            &[0x26774B, 0x82C5A1],
        ),
        group(6, "F", "Short", (0xE7E8EA, 0x39354D), None, &[0x5E6167]),
    ];
    // The chosen group's tabs have long titles too.
    for (title, _) in &mut groups[0].tabs {
        *title =
            "A page title that is much too long for a chip, with 日本語 and 🚀 in it".to_owned();
    }
    for dark in [false, true] {
        let rig = open(&Scene::new(groups.clone()).chosen(1, Some(2)), dark);
        let pixels = rig.show();
        rig.save(&pixels, "long-names");
        check_card(&rig, &pixels, "long names, CJK and emoji");
        // The chips' titles stay inside their chips too: nothing in the 6 px between two chips.
        let colours = rig.palette();
        let (x, y, width, height) = chip_box(&rig, 0);
        assert_eq!(
            ink(
                &pixels,
                (x + width + 1, y + 2, x + width + 5, y + height - 2),
                colours.sheet
            ),
            0,
            "a chip's title does not run into the next chip"
        );

        // The same in compact tiles.
        let compact: Vec<FixtureGroup> = (0..9)
            .map(|index| {
                let mut g = groups[index % groups.len()].clone();
                g.id = index as i32 + 1;
                g.key = char::from(b'A' + index as u8).to_string();
                g
            })
            .collect();
        let rig = open(&Scene::new(compact), dark);
        let pixels = rig.show();
        rig.save(&pixels, "long-names-compact");
        check_card(&rig, &pixels, "long names in compact tiles");
    }
}

#[test]
fn the_always_hint_and_a_quarter_second_hold_fit_the_card() {
    for (secondary, state) in [
        (HINT_ALWAYS, "hint-always"),
        (HINT_QUARTER, "hint-quarter-second"),
    ] {
        for dark in [false, true] {
            let mut scene = Scene::new(mockup_groups());
            scene.secondary = secondary.to_owned();
            let rig = open(&scene, dark);
            let pixels = rig.show();
            rig.save(&pixels, state);
            check_card(&rig, &pixels, state);
            // The second hint's line is there (it may take a second line, not more).
            let colours = rig.palette();
            let (card_x, card_y, ..) = rig.card();
            let line = (
                card_x + 30,
                card_y + rig.layout.tiles_bottom as i32 + 44,
                card_x + 730,
                card_y + rig.layout.tiles_bottom as i32 + 60,
            );
            assert!(
                ink(&pixels, line, colours.sheet) > 200,
                "{state}: the second hint is drawn"
            );
        }
    }
}

#[test]
fn a_hidden_card_draws_nothing_and_lets_every_click_through() {
    for dark in [false, true] {
        let scene = Scene::new(mockup_groups()).chosen(1, Some(2));
        let rig = open(&scene, dark);
        // Never shown: the same picture as the page alone.
        let hidden = rig.snapshot();
        advance(300);
        let hidden_later = rig.snapshot();
        let bare = open(&Scene::new(Vec::new()), dark);
        let page = bare.snapshot();
        assert_eq!(
            hidden.as_bytes(),
            page.as_bytes(),
            "a card that was never shown draws nothing"
        );
        assert_eq!(hidden_later.as_bytes(), page.as_bytes());
        rig.save(&hidden, "hidden");

        // Clicks on a tile, on a chip, on the link and on the backdrop all reach the page.
        let (card_x, card_y, ..) = rig.card();
        let tile = rig.tile_origin(0);
        let chip = chip_box(&rig, 1);
        for (x, y) in [
            (tile.0 as f32 + 50.0, tile.1 as f32 + 100.0),
            (chip.0 as f32 + 36.0, chip.1 as f32 + 12.0),
            (card_x as f32 + 700.0, card_y as f32 + 35.0),
            (20.0, 500.0),
        ] {
            rig.click(x, y);
        }
        assert_eq!(rig.take_log(), ["page", "page", "page", "page"]);

        // Shown and hidden again: invisible at once, and clicks go through again.
        rig.window.set_shown(true);
        let _ = rig.snapshot();
        advance(300);
        let shown = rig.snapshot();
        assert_ne!(shown.as_bytes(), page.as_bytes());
        rig.window.set_shown(false);
        let gone = rig.snapshot();
        assert_eq!(
            gone.as_bytes(),
            page.as_bytes(),
            "hiding the card removes it at once"
        );
        rig.click(tile.0 as f32 + 50.0, tile.1 as f32 + 100.0);
        assert_eq!(rig.take_log(), ["page"]);
    }
}

#[test]
fn a_click_calls_the_matching_callback_once_with_the_right_argument() {
    for dark in [false, true] {
        let rig = open(&Scene::new(mockup_groups()).chosen(1, None), dark);
        let _ = rig.show();
        let (card_x, card_y, ..) = rig.card();

        // The centre of each tile: its group's id.
        for index in 0..6 {
            let (ox, oy) = rig.tile_origin(index);
            rig.click(ox as f32 + 54.0, oy as f32 + 84.0);
            assert_eq!(
                rig.take_log(),
                [format!("tile:{}", index + 1)],
                "tile {index}"
            );
        }
        // The edges of a tile too (its corner, its top left inside the rounding).
        let (ox, oy) = rig.tile_origin(2);
        rig.click(ox as f32 + 6.0, oy as f32 + 100.0);
        assert_eq!(rig.take_log(), ["tile:3"]);

        // The centre of each chip: its digit.
        for index in 0..5 {
            let (x, y, width, height) = chip_box(&rig, index);
            rig.click((x + width / 2) as f32, (y + height / 2) as f32);
            assert_eq!(
                rig.take_log(),
                [format!("chip:{}", index + 1)],
                "chip {index}"
            );
        }

        // "Change keys…", near the right end of the text, which ends 30 px from the card's edge.
        rig.click(card_x as f32 + 715.0, card_y as f32 + 35.0);
        assert_eq!(rig.take_log(), ["change-keys"]);

        // The backdrop: the page around the card, above it and below it.
        for (x, y) in [
            (20.0, 500.0),
            (720.0, 20.0),
            (card_x as f32 - 5.0, card_y as f32 + 100.0),
            (1300.0, 700.0),
        ] {
            rig.click(x, y);
            assert_eq!(rig.take_log(), ["dismissed"], "backdrop at ({x}, {y})");
        }

        // The card's own padding and the title: kept, nothing is called.
        for (x, y) in [
            (card_x as f32 + 380.0, card_y as f32 + 10.0),
            (card_x as f32 + 60.0, card_y as f32 + 32.0),
            (card_x as f32 + 380.0, card_y as f32 + 300.0),
            (card_x as f32 + 750.0, card_y as f32 + 150.0),
        ] {
            rig.click(x, y);
            assert!(rig.take_log().is_empty(), "inside the card at ({x}, {y})");
        }
    }
}

#[test]
fn a_pointer_over_a_tile_changes_nothing() {
    for dark in [false, true] {
        let rig = open(&Scene::new(mockup_groups()), dark);
        let before = rig.show();
        let (ox, oy) = rig.tile_origin(3);
        for (x, y) in [(54.0, 84.0), (20.0, 140.0), (100.0, 10.0)] {
            rig.window
                .window()
                .dispatch_event(WindowEvent::PointerMoved {
                    position: LogicalPosition::new(ox as f32 + x, oy as f32 + y),
                });
            let after = rig.snapshot();
            assert_eq!(
                before.as_bytes(),
                after.as_bytes(),
                "the pointer over the tile at ({x}, {y}) changes the picture"
            );
        }
        assert!(rig.take_log().is_empty());
    }
}

#[test]
fn the_card_fades_in_over_100_ms_and_at_once_with_reduce_motion() {
    for dark in [false, true] {
        let mut scene = Scene::new(mockup_groups());
        scene.reduce_motion = false;
        let rig = open(&scene, dark);
        let colours = rig.palette();
        let dimmed = under_scrim(colours.page, colours.scrim);
        let probe = (20, 500);
        let brightness = |pixels: &Pixels| i32::from(at(pixels, probe.0, probe.1).b);
        let page = i32::from(colours.page.blue());
        let full = i32::from(dimmed.blue());

        rig.window.set_shown(true);
        let start = rig.snapshot();
        assert!(
            (brightness(&start) - page).abs() <= 2,
            "{}: at 0 ms the page is not dimmed yet ({} instead of {page})",
            theme_name(dark),
            brightness(&start)
        );
        advance(50);
        let middle = rig.snapshot();
        let partway = brightness(&middle);
        assert!(
            (partway - page).abs() > 2 && (partway - full).abs() > 2,
            "{}: at 50 ms the scrim is partly there ({partway}, between {page} and {full})",
            theme_name(dark)
        );
        advance(100);
        let done = rig.snapshot();
        assert!(
            (brightness(&done) - full).abs() <= 2,
            "{}: after 150 ms the scrim is whole ({} instead of {full})",
            theme_name(dark),
            brightness(&done)
        );
        check_card(&rig, &done, "after the fade");

        // With Reduce Motion the first picture is the whole card.
        let still = open(&Scene::new(mockup_groups()), dark);
        still.window.set_shown(true);
        let at_once = still.snapshot();
        assert!(
            (brightness(&at_once) - full).abs() <= 2,
            "{}: with Reduce Motion the card is there at 0 ms ({})",
            theme_name(dark),
            brightness(&at_once)
        );
        check_card(&still, &at_once, "with Reduce Motion, at 0 ms");
    }
}

#[test]
fn the_card_draws_at_most_nine_chips_whatever_it_is_given() {
    for dark in [false, true] {
        let mut many = group(
            1,
            "A",
            "Many tabs",
            (0xDAE5F8, 0x223065),
            Some(0),
            &[
                0x2655A0, 0x2B62BB, 0x2F6FD6, 0x598CDE, 0x82A9E6, 0x60469B, 0x6C4FB0, 0x7857C5,
                0x8868D4, 0x9D82DB, 0xB29DE3, 0xA76E23,
            ],
        );
        for (index, (title, _)) in many.tabs.iter_mut().enumerate() {
            *title = format!("Tab title {}", index + 1);
        }
        let mut scene = Scene::new(vec![many, mockup_groups().remove(1)]).chosen(1, Some(9));
        scene.chip_limit = 12;
        let rig = open(&scene, dark);
        let pixels = rig.show();
        rig.save(&pixels, "twelve-chips-given");
        check_card(&rig, &pixels, "twelve chips handed over");
        let (x, y, width, height) = chip_box(&rig, 8);
        // The ninth is the chosen one, the last that is drawn.
        assert_pixel(
            &pixels,
            (x + 36, y),
            rig.palette().accent,
            3,
            "the ninth chip is chosen",
        );
        assert!(width == 72 && height == 24);
        rig.click(x as f32 + 36.0, y as f32 + 12.0);
        assert_eq!(rig.take_log(), ["chip:9"]);
    }
}

/// Lines of the file that are not comments.
fn code_lines(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect()
}

#[test]
fn the_card_never_takes_keyboard_focus_and_names_no_colour_of_its_own() {
    let source = include_str!("../ui/groups/key-switcher.slint");
    let code = code_lines(source).join("\n");
    for forbidden in [
        "FocusScope",
        "forward-focus",
        "TextInput",
        "LineEdit",
        ".focus(",
        "focus-policy",
    ] {
        assert!(
            !code.contains(forbidden),
            "the card is never a keyboard focus target: `{forbidden}` found"
        );
    }
    // Colours come from `ChromePalette` and from the model; the only literal is `transparent`.
    let bytes = code.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'#' {
            let digits = bytes[index + 1..]
                .iter()
                .take_while(|b| b.is_ascii_hexdigit())
                .count();
            // `#MadeWithSlint`-style names do not occur here; a colour literal has 3 to 8 digits.
            assert!(
                !(3..=8).contains(&digits),
                "a colour literal in the card at byte {index}: {}",
                &code[index..(index + 9).min(code.len())]
            );
        }
    }
}

/// WCAG relative luminance of an opaque colour.
fn luminance(colour: Color) -> f64 {
    let linear = |value: u8| {
        let s = f64::from(value) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(colour.red()) + 0.7152 * linear(colour.green()) + 0.0722 * linear(colour.blue())
}

/// WCAG contrast ratio of two opaque colours.
fn contrast(a: Color, b: Color) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn the_cards_text_and_borders_are_readable_on_the_surfaces_they_are_drawn_on() {
    for dark in [false, true] {
        let rig = open(&Scene::new(mockup_groups()), dark);
        let colours = rig.palette();
        let palette = rig.window.global::<ChromePalette>();
        let theme = theme_name(dark);
        // Text (4.5:1): names, letters and chip titles in the text colour; counts and hints in
        // the secondary colour; the link in the accent (and its hover colour); the held chip.
        let text: [(&str, Color, Color); 12] = [
            ("title on the card", colours.text, colours.sheet),
            ("name on a tile", colours.text, colours.tile),
            ("name on the chosen tile", colours.text, colours.chosen_tile),
            ("letter on a keycap", colours.text, colours.keycap),
            ("count on a tile", colours.keycap_border, colours.tile),
            (
                "count on the chosen tile",
                colours.keycap_border,
                colours.chosen_tile,
            ),
            ("hints on the card", colours.keycap_border, colours.sheet),
            ("Change keys", colours.accent, colours.sheet),
            (
                "Change keys, pointed at",
                palette.get_button_label_hover(),
                colours.sheet,
            ),
            (
                "⌥ held",
                palette.get_primary_button_text(),
                palette.get_primary_button_background(),
            ),
            ("chip title on a chip", colours.text, colours.tile),
            (
                "chip title on the chosen chip",
                colours.text,
                colours.chosen_tile,
            ),
        ];
        for (what, foreground, background) in text {
            let ratio = contrast(foreground, background);
            assert!(
                ratio >= 4.5,
                "{theme}: {what} is {ratio:.2}:1 ({} on {}), under 4.5:1",
                show_colour(foreground),
                show_colour(background)
            );
        }
        // Shapes that carry meaning (3:1): the keycap's border, and the chosen tile's border.
        for (what, foreground, background) in [
            ("keycap border", colours.keycap_border, colours.keycap),
            (
                "chosen tile border on the card",
                colours.accent,
                colours.sheet,
            ),
            (
                "chosen tile border on its fill",
                colours.accent,
                colours.chosen_tile,
            ),
        ] {
            let ratio = contrast(foreground, background);
            assert!(ratio >= 3.0, "{theme}: {what} is {ratio:.2}:1, under 3:1");
        }
    }
}
