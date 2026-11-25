// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the Mercury Browser app icon, a square SVG drawn on Apple's macOS icon grid, into the
//! ten PNGs of a macOS `.iconset`. `scripts/macos/make-icns.sh` runs it, then `iconutil`.

#![warn(missing_docs)]

mod tint;

use std::path::{Path, PathBuf};

pub use tint::{APP_ICON_PIXELS, tint_app_icon};

/// One file of a macOS iconset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconSize {
    /// Width and height in pixels.
    pub pixels: u32,
    /// File name without `.png`, as `iconutil` expects it.
    pub name: &'static str,
}

/// The ten files of a macOS iconset.
pub const ICONSET: [IconSize; 10] = [
    IconSize {
        pixels: 16,
        name: "icon_16x16",
    },
    IconSize {
        pixels: 32,
        name: "icon_16x16@2x",
    },
    IconSize {
        pixels: 32,
        name: "icon_32x32",
    },
    IconSize {
        pixels: 64,
        name: "icon_32x32@2x",
    },
    IconSize {
        pixels: 128,
        name: "icon_128x128",
    },
    IconSize {
        pixels: 256,
        name: "icon_128x128@2x",
    },
    IconSize {
        pixels: 256,
        name: "icon_256x256",
    },
    IconSize {
        pixels: 512,
        name: "icon_256x256@2x",
    },
    IconSize {
        pixels: 512,
        name: "icon_512x512",
    },
    IconSize {
        pixels: 1024,
        name: "icon_512x512@2x",
    },
];

/// Why rendering failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A source file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// An output file or folder could not be written.
    #[error("cannot write {path}: {source}")]
    Write {
        /// The file or folder.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The input is not a valid SVG.
    #[error("not a valid SVG: {0}")]
    Svg(String),
    /// The SVG canvas is not square, so it would be distorted.
    #[error("the SVG canvas is {width} x {height}; an app icon must be square")]
    NotSquare {
        /// Canvas width in SVG units.
        width: f32,
        /// Canvas height in SVG units.
        height: f32,
    },
    /// A size of zero pixels was requested.
    #[error("the requested size is zero")]
    ZeroSize,
    /// The PNG could not be encoded.
    #[error("cannot encode the PNG: {0}")]
    Png(String),
    /// The icon artwork lacks a colour the tint replaces, so it was edited without the tint.
    #[error("the icon artwork has no {0}")]
    Template(String),
}

/// The largest size drawn from the small artwork. At 16 and 32 px the full icon's detail turns to
/// mush, so a simplified drawing is used there, as Apple's own small icons do.
pub const SMALL_ARTWORK_MAX_PIXELS: u32 = 32;

/// Whether an icon of `pixels` uses the simplified small artwork.
pub fn uses_small_artwork(pixels: u32) -> bool {
    pixels <= SMALL_ARTWORK_MAX_PIXELS
}

/// Renders the whole canvas of a square SVG to a `size` x `size` PNG with a transparent background.
pub fn render_png(svg: &[u8], size: u32) -> Result<Vec<u8>, Error> {
    if size == 0 {
        return Err(Error::ZeroSize);
    }
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .map_err(|error| Error::Svg(error.to_string()))?;
    let canvas = tree.size();
    let (width, height) = (canvas.width(), canvas.height());
    if (width - height).abs() > width.max(height) * 1e-4 {
        return Err(Error::NotSquare { width, height });
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).ok_or(Error::ZeroSize)?;
    let scale = size as f32 / width;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|error| Error::Png(error.to_string()))
}

/// Writes the ten PNGs of an iconset into `dir`, creating it if needed: sizes up to 32 px from
/// `small`, the rest from `large`.
pub fn write_iconset(large: &Path, small: &Path, dir: &Path) -> Result<(), Error> {
    let read = |path: &Path| {
        std::fs::read(path).map_err(|source| Error::Read {
            path: path.to_owned(),
            source,
        })
    };
    let (large, small) = (read(large)?, read(small)?);
    std::fs::create_dir_all(dir).map_err(|source| Error::Write {
        path: dir.to_owned(),
        source,
    })?;
    for icon in ICONSET {
        let artwork = if uses_small_artwork(icon.pixels) {
            &small
        } else {
            &large
        };
        let png = render_png(artwork, icon.pixels)?;
        let path = dir.join(format!("{}.png", icon.name));
        std::fs::write(&path, png).map_err(|source| Error::Write { path, source })?;
    }
    Ok(())
}
