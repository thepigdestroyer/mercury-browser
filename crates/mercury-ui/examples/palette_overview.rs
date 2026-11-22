// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Contact sheets for the palettes a user can choose: one `overview.png` per palette
//! folder (scene 2, light and dark side by side, with the colours) and one combined
//! `overview.png` with all of them. Reads the pictures `tests/palette_gallery.rs` saved.
//!
//! `cargo run --release -p mercury-ui --example palette_overview -- <palettes directory>`

use std::cell::Cell;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use mercury_appearance::{Palette, Role, Scheme, palettes};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::{
    Color, ComponentHandle, Image, LogicalSize, ModelRc, Rgba8Pixel, SharedPixelBuffer,
    SharedString, VecModel,
};

slint::slint! {
    struct Swatch { label: string, hex: string, colour: color }
    struct CardData {
        title: string,
        mood: string,
        light: image,
        dark: image,
        swatches: [Swatch],
    }

    component Card inherits Rectangle {
        in property <CardData> data;
        in property <length> shot-w;
        in property <length> clip-h;
        in property <float> clip-src-h;
        in property <bool> big;
        background: #1c1d22;
        border-radius: 12px;
        property <length> pad: 14px;

        Text {
            x: root.pad; y: 10px;
            text: root.data.title;
            font-size: root.big ? 30px : 22px;
            font-weight: 700;
            color: #ffffff;
        }
        Text {
            x: root.pad; y: root.big ? 48px : 38px;
            width: root.width - 2 * root.pad;
            text: root.data.mood;
            font-size: root.big ? 14px : 11px;
            color: #b9bcc6;
            overflow: elide;
        }
        // swatches, right-aligned on the title line
        HorizontalLayout {
            x: root.width - self.preferred-width - root.pad;
            y: 10px;
            spacing: 10px;
            for sw in root.data.swatches : VerticalLayout {
                spacing: 2px;
                Rectangle {
                    width: root.big ? 34px : 26px; height: root.big ? 34px : 26px;
                    border-radius: 6px;
                    background: sw.colour;
                    border-width: 1px;
                    border-color: #ffffff40;
                }
                Text {
                    text: sw.label;
                    font-size: 8px; color: #8d909b;
                    horizontal-alignment: center;
                }
                Text {
                    text: sw.hex;
                    font-size: 8px; color: #b9bcc6;
                    horizontal-alignment: center;
                }
            }
        }
        Image {
            x: root.pad;
            y: root.big ? 88px : 70px;
            width: root.shot-w; height: root.clip-h;
            source: root.data.light;
            image-fit: fill;
            source-clip-x: 0; source-clip-y: 0;
            source-clip-width: root.data.light.width;
            source-clip-height: root.clip-src-h;
        }
        Image {
            x: 2 * root.pad + root.shot-w;
            y: root.big ? 88px : 70px;
            width: root.shot-w; height: root.clip-h;
            source: root.data.dark;
            image-fit: fill;
            source-clip-x: 0; source-clip-y: 0;
            source-clip-width: root.data.dark.width;
            source-clip-height: root.clip-src-h;
        }
    }

    export component Sheet inherits Window {
        in property <[CardData]> cards;
        in property <int> columns;
        in property <length> shot-w;
        in property <length> clip-h;
        in property <float> clip-src-h;
        in property <length> card-w;
        in property <length> card-h;
        in property <bool> big;
        background: #0d0e11;
        for card[i] in root.cards : Card {
            x: 12px + mod(i, root.columns) * (root.card-w + 12px);
            y: 12px + floor(i / root.columns) * (root.card-h + 12px);
            width: root.card-w; height: root.card-h;
            data: card;
            shot-w: root.shot-w;
            clip-h: root.clip-h;
            clip-src-h: root.clip-src-h;
            big: root.big;
        }
    }
}

struct HeadlessPlatform;

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }

    fn duration_since_start(&self) -> Duration {
        CLOCK.with(Cell::get)
    }
}

thread_local! {
    static CLOCK: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

fn mood(id: &str) -> &'static str {
    match id {
        "abyss" => {
            "Deep ocean: sea-glass white by day, a trench of blue-green by night, bright cyan-turquoise accent."
        }
        "glacier" => {
            "Arctic ice: frosted blue-white chrome, vivid cyan accent; steel-blue polar night in the dark."
        }
        "graphite" => "Studio graphite: true neutral greys, no tint, one electric-violet accent.",
        "midnight" => {
            "Midnight indigo with an aurora-green accent: periwinkle paper by day, deep saturated blue by night."
        }
        "forest" => {
            "Moss and mint: sage-green paper by day, a dark forest floor by night, fresh lime-mint accent."
        }
        "signal" => {
            "Bold and stark: paper white and ink black, hard hairlines, one hot electric-pink accent."
        }
        _ => "",
    }
}

fn load(path: &Path) -> Result<Image, Box<dyn Error>> {
    let picture = image::open(path)?.to_rgba8();
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        picture.as_raw(),
        picture.width(),
        picture.height(),
    );
    Ok(Image::from_rgba8(buffer))
}

fn colour(palette: &Palette, scheme: Scheme, role: Role) -> (Color, String) {
    let value = palette.colour(scheme, role);
    (
        Color::from_rgb_u8(value.r, value.g, value.b),
        value.to_hex(),
    )
}

fn swatch(palette: &Palette, label: &str, scheme: Scheme, role: Role) -> Swatch {
    let (colour, hex) = colour(palette, scheme, role);
    Swatch {
        label: SharedString::from(label),
        hex: SharedString::from(hex),
        colour,
    }
}

fn card(root: &Path, palette: &Palette) -> Result<CardData, Box<dyn Error>> {
    let folder = root.join(palette.id);
    let swatches = vec![
        swatch(
            palette,
            "L accent",
            Scheme::Light,
            Role::PrimaryButtonBackground,
        ),
        swatch(palette, "L frame", Scheme::Light, Role::WindowBackground),
        swatch(palette, "L bar", Scheme::Light, Role::ToolbarBackground),
        swatch(palette, "L text", Scheme::Light, Role::Text),
        swatch(
            palette,
            "D accent",
            Scheme::Dark,
            Role::PrimaryButtonBackground,
        ),
        swatch(palette, "D frame", Scheme::Dark, Role::WindowBackground),
        swatch(palette, "D bar", Scheme::Dark, Role::ToolbarBackground),
        swatch(palette, "D text", Scheme::Dark, Role::Text),
    ];
    Ok(CardData {
        title: SharedString::from(palette.name),
        mood: SharedString::from(mood(palette.id)),
        light: load(&folder.join("2-main-window-tabs-active-light.png"))?,
        dark: load(&folder.join("2-main-window-tabs-active-dark.png"))?,
        swatches: ModelRc::new(VecModel::from(swatches)),
    })
}

#[allow(clippy::too_many_arguments)]
fn sheet(
    cards: Vec<CardData>,
    columns: i32,
    scale: f32,
    clip_src_h: f32,
    big: bool,
    path: &Path,
) -> Result<(), Box<dyn Error>> {
    let shot_w = 900.0 * scale;
    let clip_h = clip_src_h * scale;
    let card_w = 3.0 * 14.0 + 2.0 * shot_w;
    let top = if big { 88.0 } else { 70.0 };
    let card_h = top + clip_h + 14.0;
    let rows = (cards.len() as f32 / columns as f32).ceil();
    let width = 12.0 + columns as f32 * (card_w + 12.0);
    let height = 12.0 + rows * (card_h + 12.0);
    let window = Sheet::new()?;
    window.set_cards(ModelRc::new(VecModel::from(cards)));
    window.set_columns(columns);
    window.set_shot_w(shot_w);
    window.set_clip_h(clip_h);
    window.set_clip_src_h(clip_src_h);
    window.set_card_w(card_w);
    window.set_card_h(card_h);
    window.set_big(big);
    window
        .window()
        .set_size(LogicalSize::new(width.ceil(), height.ceil()));
    window.show()?;
    for _ in 0..3 {
        CLOCK.with(|clock| clock.set(clock.get() + Duration::from_millis(100)));
        slint::platform::update_timers_and_animations();
        window.window().request_redraw();
        let _ = window.window().take_snapshot();
    }
    let snapshot = window.window().take_snapshot()?;
    image::save_buffer_with_format(
        path,
        snapshot.as_bytes(),
        snapshot.width(),
        snapshot.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )?;
    println!(
        "{} ({} x {})",
        path.display(),
        snapshot.width(),
        snapshot.height()
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let root: PathBuf = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: palette_overview <palettes directory>")?;
    slint::platform::set_platform(Box::new(HeadlessPlatform))
        .map_err(|error| format!("platform: {error}"))?;
    for palette in palettes::selectable() {
        // One palette: the whole picture of scene 2, both schemes, at full size.
        sheet(
            vec![card(&root, palette)?],
            1,
            0.75,
            560.0,
            true,
            &root.join(palette.id).join("overview.png"),
        )?;
    }
    // All of them: the top of the window (strip, toolbar, a little page), scaled down.
    let cards = palettes::selectable()
        .map(|palette| card(&root, palette))
        .collect::<Result<Vec<_>, _>>()?;
    sheet(cards, 2, 0.62, 250.0, false, &root.join("overview.png"))?;
    Ok(())
}
