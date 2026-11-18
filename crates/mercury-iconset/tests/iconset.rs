// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Behaviour of the iconset renderer behind `scripts/macos/make-icns.sh`.

use mercury_iconset::{Error, ICONSET, render_png, uses_small_artwork, write_iconset};
use resvg::tiny_skia::Pixmap;

const RED_SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#ff0000"/></svg>"##;
const BLUE_SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#0000ff"/></svg>"##;
const CIRCLE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="30" fill="#00ff00"/></svg>"##;

fn rgba(png: &[u8], x: u32, y: u32) -> (u8, u8, u8, u8) {
    let pixmap = Pixmap::decode_png(png).expect("a valid PNG");
    let pixel = pixmap.pixel(x, y).expect("a pixel inside the image").demultiply();
    (pixel.red(), pixel.green(), pixel.blue(), pixel.alpha())
}

fn dimensions(png: &[u8]) -> (u32, u32) {
    let pixmap = Pixmap::decode_png(png).expect("a valid PNG");
    (pixmap.width(), pixmap.height())
}

#[test]
fn iconset_lists_the_ten_files_iconutil_expects() {
    let names: Vec<(u32, &str)> = ICONSET.iter().map(|icon| (icon.pixels, icon.name)).collect();
    assert_eq!(
        names,
        [
            (16, "icon_16x16"),
            (32, "icon_16x16@2x"),
            (32, "icon_32x32"),
            (64, "icon_32x32@2x"),
            (128, "icon_128x128"),
            (256, "icon_128x128@2x"),
            (256, "icon_256x256"),
            (512, "icon_256x256@2x"),
            (512, "icon_512x512"),
            (1024, "icon_512x512@2x"),
        ]
    );
}

#[test]
fn small_artwork_is_used_up_to_32_pixels() {
    assert!(uses_small_artwork(16));
    assert!(uses_small_artwork(32));
    assert!(!uses_small_artwork(64));
    assert!(!uses_small_artwork(1024));
}

#[test]
fn renders_the_whole_canvas_at_the_requested_size() {
    for size in [16, 64, 1024] {
        let png = render_png(RED_SQUARE.as_bytes(), size).unwrap();
        assert_eq!(dimensions(&png), (size, size));
        assert_eq!(rgba(&png, size / 2, size / 2), (255, 0, 0, 255));
        assert_eq!(rgba(&png, 0, 0), (255, 0, 0, 255));
        assert_eq!(rgba(&png, size - 1, size - 1), (255, 0, 0, 255));
    }
}

#[test]
fn keeps_the_background_transparent() {
    let png = render_png(CIRCLE.as_bytes(), 100).unwrap();
    assert_eq!(rgba(&png, 2, 2).3, 0);
    assert_eq!(rgba(&png, 50, 50), (0, 255, 0, 255));
}

#[test]
fn rejects_an_svg_that_is_not_square() {
    let wide = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"/>"#;
    assert!(matches!(
        render_png(wide.as_bytes(), 32),
        Err(Error::NotSquare { .. })
    ));
}

#[test]
fn rejects_input_that_is_not_svg() {
    assert!(matches!(
        render_png(b"not an svg", 32),
        Err(Error::Svg(_))
    ));
}

#[test]
fn rejects_a_zero_size() {
    assert!(matches!(
        render_png(RED_SQUARE.as_bytes(), 0),
        Err(Error::ZeroSize)
    ));
}

#[test]
fn writes_ten_pngs_with_the_small_artwork_for_the_small_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let large = dir.path().join("large.svg");
    let small = dir.path().join("small.svg");
    std::fs::write(&large, BLUE_SQUARE).unwrap();
    std::fs::write(&small, RED_SQUARE).unwrap();
    let iconset = dir.path().join("out").join("AppIcon.iconset");

    write_iconset(&large, &small, &iconset).unwrap();

    let mut written: Vec<String> = std::fs::read_dir(&iconset)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    written.sort();
    let mut expected: Vec<String> = ICONSET.iter().map(|icon| format!("{}.png", icon.name)).collect();
    expected.sort();
    assert_eq!(written, expected);

    for icon in ICONSET {
        let png = std::fs::read(iconset.join(format!("{}.png", icon.name))).unwrap();
        assert_eq!(dimensions(&png), (icon.pixels, icon.pixels), "{}", icon.name);
        let colour = rgba(&png, icon.pixels / 2, icon.pixels / 2);
        let want = if icon.pixels <= 32 { (255, 0, 0, 255) } else { (0, 0, 255, 255) };
        assert_eq!(colour, want, "{}", icon.name);
    }
}

#[test]
fn reports_a_missing_source_file() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.svg");
    let result = write_iconset(&missing, &missing, &dir.path().join("AppIcon.iconset"));
    assert!(matches!(result, Err(Error::Read { .. })));
}
