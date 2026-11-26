// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The group dock through Slint's software renderer (no display): pixel checks of the open, the
//! magnified and the shrunk dock in light and dark. Set `MERCURY_UI_RENDER_DIR` to keep the PNGs.
//! (`examples/render_group_dock.rs` writes the full set of pictures for looking at.)

mod group_dock_common;

use std::path::PathBuf;

use group_dock_common::{Host, init_software, near, pixel_at, snapshot, snapshot_of};
use mercury_ui::group_dock::DockOptions;
use mercury_ui::ring_bands::fixtures::{mockup_groups, mockup_tabs};
use mercury_ui::{ChromePalette, ChromeTheme, DockStateDotPreview, DockStateKind};
use slint::{ComponentHandle, Model, Rgba8Pixel, SharedPixelBuffer};

fn scene(
    dark: bool,
    pointer_pill: Option<usize>,
    shrunk: bool,
) -> (Host, SharedPixelBuffer<Rgba8Pixel>) {
    init_software();
    let host = Host::new(
        mockup_groups(dark),
        mockup_tabs(dark),
        DockOptions::default(),
    );
    host.window.global::<ChromePalette>().set_theme(if dark {
        ChromeTheme::Dark
    } else {
        ChromeTheme::Light
    });
    host.open();
    if let Some(index) = pointer_pill {
        host.settle_on_pill(index);
    }
    host.window.set_shrunk(shrunk);
    let picture = snapshot(&host.window);
    (host, picture)
}

fn save(picture: &SharedPixelBuffer<Rgba8Pixel>, name: &str) {
    let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&directory).expect("the render directory can be made");
    image::save_buffer_with_format(
        directory.join(name),
        picture.as_bytes(),
        picture.width(),
        picture.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .expect("the rendering can be saved");
}

fn rgb(color: slint::Color) -> (u8, u8, u8) {
    (color.red(), color.green(), color.blue())
}

#[test]
fn the_open_dock_is_drawn_with_the_rows_colours() {
    for (dark, name) in [(false, "light"), (true, "dark")] {
        let (host, picture) = scene(dark, None, false);
        save(&picture, &format!("dock-open-{name}.png"));
        let palette = host.window.global::<ChromePalette>();
        let frame = host.frame.borrow();
        let y = Host::mid_y();

        // The bar's padding is the popup colour.
        let padding = pixel_at(&picture, frame.dock_x + 6.0, y);
        assert!(
            near(padding, rgb(palette.get_popup_background()), 2),
            "{name}: bar padding {padding:?}"
        );

        // A folded pill's left end is the group's tint, and a sliver is the tab's shade.
        let pill = &frame.pills[1];
        let tint = pixel_at(&picture, pill.x + 8.0, y);
        assert!(
            near(tint, rgb(pill.group.tint), 2),
            "{name}: pill tint {tint:?} for {:?}",
            rgb(pill.group.tint)
        );
        let sliver = pill.slivers.row_data(1).expect("a sliver");
        let drawn = pixel_at(&picture, pill.x + sliver.x + sliver.width / 2.0, y);
        assert!(
            near(drawn, rgb(sliver.shade), 2),
            "{name}: sliver {drawn:?} for {:?}",
            rgb(sliver.shade)
        );

        // The current pill's own "+" is drawn at its right end: its middle is not the pill's tint.
        let current = &frame.pills[0];
        assert!(current.expanded);
        let plus = pixel_at(&picture, current.x + current.width - 17.0, y);
        assert!(
            !near(plus, rgb(current.group.tint), 20),
            "{name}: nothing is drawn where the pill's + is: {plus:?}"
        );

        // The page above the band is untouched.
        let page = pixel_at(&picture, 720.0, 300.0);
        assert!(
            near(page, rgb(palette.get_content_background()), 1),
            "{name}: page {page:?}"
        );
    }
}

#[test]
fn the_dark_dock_differs_from_the_light_one() {
    let (light, light_picture) = scene(false, None, false);
    let (dark, dark_picture) = scene(true, None, false);
    let y = Host::mid_y();
    let a = pixel_at(&light_picture, light.frame.borrow().dock_x + 6.0, y);
    let b = pixel_at(&dark_picture, dark.frame.borrow().dock_x + 6.0, y);
    assert!(
        !near(a, b, 8),
        "light {a:?} and dark {b:?} bars must differ"
    );
}

#[test]
fn the_magnified_pill_is_taller_and_the_bar_grows_upward_only() {
    let (host, picture) = scene(false, Some(2), false);
    save(&picture, "dock-magnified-light.png");
    let frame = host.frame.borrow();
    let rest = host.data.borrow().frame(None);
    assert!((rest.dock_height - 52.0).abs() < 1e-3);
    assert!(
        (frame.dock_height - rest.dock_height - 13.5).abs() < 0.1,
        "{} against {}",
        frame.dock_height,
        rest.dock_height
    );
    let big = &frame.pills[2];
    assert!((big.height - 43.5).abs() < 0.1, "{}", big.height);
    assert!((frame.pills[0].height - 30.0).abs() < 0.1);
    // The bottom edge did not move: the bar's bottom row is still drawn 12 px above the window's
    // bottom, wherever the pills are.
    // (24 px in from the left end, 5 px above the bottom edge: below the pills, inside the bar.)
    let low = pixel_at(&picture, frame.dock_x + 24.0, 900.0 - 12.0 - 5.0);
    let palette = host.window.global::<ChromePalette>();
    assert!(near(low, rgb(palette.get_popup_background()), 2), "{low:?}");
}

#[test]
fn the_shrunk_dock_is_the_capsule_with_an_accent_outline() {
    for (dark, name) in [(false, "light"), (true, "dark")] {
        let (host, picture) = scene(dark, None, true);
        save(&picture, &format!("dock-shrunk-{name}.png"));
        let palette = host.window.global::<ChromePalette>();
        // The capsule is 236 x 28, centred, 12 px above the window's bottom; its outline is 1.5 px.
        let outline = pixel_at(&picture, 602.5, 900.0 - 12.0 - 14.0);
        assert!(
            near(outline, rgb(palette.get_accent()), 8),
            "{name}: outline {outline:?} against {:?}",
            rgb(palette.get_accent())
        );
        let inside = pixel_at(&picture, 607.0, 900.0 - 12.0 - 14.0);
        assert!(
            near(inside, rgb(palette.get_popup_background()), 4),
            "{name}: capsule inside {inside:?}"
        );
        // The count is drawn beside the slivers (the box starts 182 px into the capsule).
        assert_eq!(host.window.get_count_text().as_str(), "22");
        let background = rgb(palette.get_popup_background());
        let ink = (0..52).any(|i| {
            (0..32).any(|j| {
                let probe = pixel_at(&picture, 784.0 + i as f32 * 0.5, 866.0 + j as f32 * 0.5);
                !near(probe, background, 20)
            })
        });
        assert!(ink, "{name}: the tab count is drawn in the capsule");
        // The open dock's bar is not drawn: where its padding was, the page shows.
        let page = pixel_at(&picture, 100.0, 900.0 - 12.0 - 14.0);
        assert!(
            near(page, rgb(palette.get_content_background()), 2),
            "{name}: the page beside the capsule {page:?}"
        );
    }
}

#[test]
fn every_state_marker_has_a_shape_of_its_own() {
    init_software();
    let kinds = [
        ("unread", DockStateKind::Disc),
        ("crashed", DockStateKind::Triangle),
        ("capture", DockStateKind::RingDot),
        ("audible", DockStateKind::Bars),
        ("muted", DockStateKind::RingSlash),
        ("loading", DockStateKind::Ring),
    ];
    // The same colour for every kind: what tells them apart is the shape.
    let masks: Vec<Vec<bool>> = kinds
        .iter()
        .map(|(name, kind)| {
            let window = DockStateDotPreview::new().expect("the marker preview");
            window.set_kind(*kind);
            window.show().expect("the window can be shown");
            let picture = snapshot_of(window.window(), 24.0, 24.0);
            let background = picture.as_slice()[0];
            let mask: Vec<bool> = picture
                .as_slice()
                .iter()
                .map(|p| {
                    !near(
                        (p.r, p.g, p.b),
                        (background.r, background.g, background.b),
                        40,
                    )
                })
                .collect();
            let ink = mask.iter().filter(|on| **on).count();
            assert!(ink > 60, "{name} draws something ({ink} px)");
            window.hide().expect("the window can be hidden");
            mask
        })
        .collect();
    for i in 0..kinds.len() {
        for j in i + 1..kinds.len() {
            let differing = masks[i]
                .iter()
                .zip(&masks[j])
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                differing >= 60,
                "{} and {} differ in {differing} px only",
                kinds[i].0,
                kinds[j].0
            );
        }
    }
}
