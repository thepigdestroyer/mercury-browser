// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Reading screenshots with the `image` crate (the workspace's PNG decoder): any PNG it
//! decodes (every colour type and bit depth, interlaced or not), its size checked against
//! limits well above any screenshot before the pixels are decoded, so that a small crafted
//! file cannot make it allocate without bound.

use std::io::Cursor;

#[cfg(test)]
use image::ImageEncoder;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbImage};

pub const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Limits well above any screenshot (a 900x700 window at 3x is 2700x2100).
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_SIDE: u32 = 16_384;
pub const MAX_PIXELS: u64 = 40 * 1000 * 1000;

/// Why a file is not a usable PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngError(pub String);

impl std::fmt::Display for PngError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PngError {}

/// A decoded image as 8-bit RGB (alpha dropped, grey expanded, deeper samples scaled).
#[derive(Debug, Clone)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    rgb: RgbImage,
}

impl Image {
    /// (r, g, b) at integer image coordinates.
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        self.rgb.get_pixel(x as u32, y as u32).0
    }

    /// Mean (r, g, b) of the (2 * radius + 1)² block centred on (x, y), clipped to the image.
    pub fn average(&self, x: usize, y: usize, radius: usize) -> [u8; 3] {
        let mut totals = [0u64; 3];
        let mut count = 0u64;
        for yy in y.saturating_sub(radius)..self.height.min(y + radius + 1) {
            for xx in x.saturating_sub(radius)..self.width.min(x + radius + 1) {
                for (total, value) in totals.iter_mut().zip(self.pixel(xx, yy)) {
                    *total += u64::from(value);
                }
                count += 1;
            }
        }
        totals.map(|total| (total / count) as u8)
    }
}

/// Decodes a whole PNG file held in memory.
pub fn decode(data: &[u8]) -> Result<Image, PngError> {
    if !data.starts_with(SIGNATURE) {
        return Err(PngError("not a PNG file (bad signature)".into()));
    }
    let failed = |error: image::ImageError| PngError(error.to_string());
    // Reads the header only; the pixels are decoded below, once the size is known to be sane.
    let decoder = ImageReader::with_format(Cursor::new(data), ImageFormat::Png)
        .into_decoder()
        .map_err(failed)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(PngError("empty image".into()));
    }
    if width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(PngError(format!(
            "{width}x{height} is larger than any screenshot this checks"
        )));
    }
    let rgb = DynamicImage::from_decoder(decoder)
        .map_err(failed)?
        .into_rgb8();
    Ok(Image {
        width: width as usize,
        height: height as usize,
        rgb,
    })
}

/// Encodes 8-bit RGB (3 bytes a pixel) or RGBA (4) pixels as a PNG. Test builds only: the
/// tools read screenshots and never write them.
#[cfg(test)]
pub(crate) fn encode(width: u32, height: u32, channels: usize, pixels: &[u8]) -> Vec<u8> {
    let colour = match channels {
        3 => image::ExtendedColorType::Rgb8,
        4 => image::ExtendedColorType::Rgba8,
        _ => panic!("encode takes 3 or 4 channels, not {channels}"),
    };
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(pixels, width, height, colour)
        .expect("encodes in memory");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32, channels: usize) -> Vec<u8> {
        (0..width as usize * height as usize * channels)
            .map(|i| (i * 7 % 251) as u8)
            .collect()
    }

    fn message(data: &[u8]) -> String {
        decode(data).expect_err("must fail").0
    }

    #[test]
    fn round_trips_rgb_and_rgba() {
        for channels in [3, 4] {
            let pixels = gradient(13, 11, channels);
            let image = decode(&encode(13, 11, channels, &pixels)).expect("decodes");
            assert_eq!((image.width, image.height), (13, 11));
            for y in 0..11 {
                for x in 0..13 {
                    let at = (y * 13 + x) * channels;
                    assert_eq!(
                        image.pixel(x, y),
                        [pixels[at], pixels[at + 1], pixels[at + 2]]
                    );
                }
            }
        }
    }

    #[test]
    fn reads_grey_as_grey_rgb() {
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&[0, 200], 2, 1, image::ExtendedColorType::L8)
            .expect("encodes");
        let image = decode(&out).expect("decodes");
        assert_eq!(image.pixel(0, 0), [0, 0, 0]);
        assert_eq!(image.pixel(1, 0), [200, 200, 200]);
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(body);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }

    /// A file whose header says `width`x`height` (8-bit RGB), with a few bytes of image data:
    /// far too few for that size, so only refusing it from the header passes.
    fn with_ihdr(width: u32, height: u32) -> Vec<u8> {
        let mut header = Vec::new();
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, 2, 0, 0, 0]);
        let mut out = SIGNATURE.to_vec();
        push_chunk(&mut out, b"IHDR", &header);
        push_chunk(&mut out, b"IDAT", &[0x78, 0x9c, 0x03, 0x00]);
        push_chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn refuses_malformed_files() {
        let good = encode(4, 4, 3, &gradient(4, 4, 3));
        assert!(decode(&good).is_ok());
        assert_eq!(message(b"GIF89a"), "not a PNG file (bad signature)");
        // Cut before the last pixel, or corrupted: an error with a message, never a panic.
        // (A file cut in its IEND chunk, after every pixel, still reads.)
        for end in [8, 10, 20, 33, 45] {
            assert!(!message(&good[..end]).is_empty(), "cut at {end}");
        }
        let mut corrupt = good.clone();
        corrupt[20] ^= 1;
        assert!(!message(&corrupt).is_empty());
        // Refused from the header alone, before any pixel is decoded.
        assert_eq!(
            message(&with_ihdr(20_000, 1)),
            "20000x1 is larger than any screenshot this checks"
        );
        assert_eq!(
            message(&with_ihdr(10_000, 10_000)),
            "10000x10000 is larger than any screenshot this checks"
        );
    }
}
