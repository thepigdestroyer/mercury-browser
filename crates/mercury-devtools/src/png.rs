// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A small, strict PNG reader for screenshots: 8-bit, non-interlaced greyscale, grey with
//! alpha, RGB, palette or RGBA images. Every malformed input is an error with its own message,
//! and the limits keep a small crafted file from making it allocate without bound.

use miniz_oxide::inflate::{TINFLStatus, decompress_to_vec_zlib_with_limit};

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

fn error<T>(message: impl Into<String>) -> Result<T, PngError> {
    Err(PngError(message.into()))
}

/// A decoded image: rows of `channels` bytes per pixel (1 grey, 2 grey and alpha, 3 RGB,
/// 4 RGBA; palette images are expanded to RGB).
#[derive(Debug, Clone)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub rows: Vec<Vec<u8>>,
}

impl Image {
    /// (r, g, b) at integer image coordinates.
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let row = &self.rows[y];
        let offset = x * self.channels;
        if self.channels <= 2 {
            let value = row[offset];
            [value, value, value]
        } else {
            [row[offset], row[offset + 1], row[offset + 2]]
        }
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

/// The IEEE CRC-32 that PNG chunks carry.
pub fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for part in parts {
        for &byte in *part {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
    }
    !crc
}

fn be32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let pa = (p - i16::from(a)).abs();
    let pb = (p - i16::from(b)).abs();
    let pc = (p - i16::from(c)).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

struct Header {
    width: u32,
    height: u32,
    depth: u8,
    colour_type: u8,
    compression: u8,
    filter_method: u8,
    interlace: u8,
}

/// Decodes a whole PNG file held in memory.
pub fn decode(data: &[u8]) -> Result<Image, PngError> {
    if !data.starts_with(SIGNATURE) {
        return error("not a PNG file (bad signature)");
    }
    let mut position = SIGNATURE.len();
    let mut header: Option<Header> = None;
    let mut palette: Option<Vec<[u8; 3]>> = None;
    let mut compressed = Vec::new();
    let mut seen_end = false;
    while position < data.len() {
        if position + 8 > data.len() {
            return error(format!("truncated chunk header at byte {position}"));
        }
        let length = be32(&data[position..]) as usize;
        let kind = &data[position + 4..position + 8];
        let kind_name: String = kind.iter().map(|&byte| char::from(byte)).collect();
        let body_start = position + 8;
        let body_end = body_start + length;
        if body_end + 4 > data.len() {
            return error(format!("truncated {kind_name} chunk at byte {position}"));
        }
        let body = &data[body_start..body_end];
        if crc32(&[kind, body]) != be32(&data[body_end..]) {
            return error(format!("CRC mismatch in {kind_name} chunk"));
        }
        position = body_end + 4;
        if header.is_none() && kind != b"IHDR" {
            return error("first chunk is not IHDR");
        }
        match kind {
            b"IHDR" => {
                if header.is_some() {
                    return error("more than one IHDR chunk");
                }
                if length != 13 {
                    return error(format!("IHDR chunk is {length} bytes long, not 13"));
                }
                header = Some(Header {
                    width: be32(body),
                    height: be32(&body[4..]),
                    depth: body[8],
                    colour_type: body[9],
                    compression: body[10],
                    filter_method: body[11],
                    interlace: body[12],
                });
            }
            b"PLTE" => {
                if palette.is_some() {
                    return error("more than one PLTE chunk");
                }
                if length == 0 || !length.is_multiple_of(3) || length > 3 * 256 {
                    return error(format!(
                        "PLTE chunk is {length} bytes long, not a multiple of 3 up to 768"
                    ));
                }
                palette = Some(body.as_chunks::<3>().0.to_vec());
            }
            b"IDAT" => compressed.extend_from_slice(body),
            b"IEND" => {
                seen_end = true;
                break;
            }
            _ => {}
        }
    }
    let Some(header) = header else {
        return error("no IHDR chunk");
    };
    if !seen_end {
        return error("no IEND chunk (file truncated?)");
    }
    let channels = match header.colour_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        other => return error(format!("unsupported colour type {other}")),
    };
    if header.depth != 8 {
        return error(format!(
            "unsupported bit depth {} (only 8 is handled)",
            header.depth
        ));
    }
    if header.compression != 0 || header.filter_method != 0 {
        return error("unknown compression or filter method in IHDR");
    }
    if header.interlace != 0 {
        return error("interlaced PNGs are not handled");
    }
    if header.colour_type == 3 && palette.is_none() {
        return error("palette image without a PLTE chunk");
    }
    let (width, height) = (header.width, header.height);
    if width == 0 || height == 0 {
        return error("empty image");
    }
    if width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return error(format!(
            "{width}x{height} is larger than any screenshot this checks"
        ));
    }
    let (width, height) = (width as usize, height as usize);
    let stride = width * channels;
    let expected = height * (stride + 1);

    // One byte more than the header allows is enough to tell that the stream is too large;
    // the limit keeps a small stream that would expand without bound from doing so.
    let raw = match decompress_to_vec_zlib_with_limit(&compressed, expected + 1) {
        Ok(raw) => raw,
        Err(failure) => {
            return match failure.status {
                TINFLStatus::HasMoreOutput => error(format!(
                    "image data is larger than the {width}x{height} header allows"
                )),
                TINFLStatus::FailedCannotMakeProgress | TINFLStatus::NeedsMoreInput => {
                    error("image data stream is incomplete")
                }
                status => error(format!("image data does not decompress: {status:?}")),
            };
        }
    };
    if raw.len() > expected {
        return error(format!(
            "image data is larger than the {width}x{height} header allows"
        ));
    }
    if raw.len() != expected {
        return error(format!(
            "image data has {} bytes, expected {expected}",
            raw.len()
        ));
    }

    let mut rows: Vec<Vec<u8>> = Vec::with_capacity(height);
    let mut previous = vec![0u8; stride];
    let bpp = channels;
    for y in 0..height {
        let start = y * (stride + 1);
        let filter = raw[start];
        let mut line = raw[start + 1..start + 1 + stride].to_vec();
        match filter {
            0 => {}
            1 => {
                for i in bpp..stride {
                    line[i] = line[i].wrapping_add(line[i - bpp]);
                }
            }
            2 => {
                for i in 0..stride {
                    line[i] = line[i].wrapping_add(previous[i]);
                }
            }
            3 => {
                for i in 0..stride {
                    let left = if i >= bpp { line[i - bpp] } else { 0 };
                    let mean = ((u16::from(left) + u16::from(previous[i])) >> 1) as u8;
                    line[i] = line[i].wrapping_add(mean);
                }
            }
            4 => {
                for i in 0..stride {
                    let left = if i >= bpp { line[i - bpp] } else { 0 };
                    let upper_left = if i >= bpp { previous[i - bpp] } else { 0 };
                    line[i] = line[i].wrapping_add(paeth(left, previous[i], upper_left));
                }
            }
            other => return error(format!("unknown filter type {other} on row {y}")),
        }
        previous.clone_from(&line);
        rows.push(line);
    }

    if header.colour_type == 3 {
        let palette = palette.unwrap_or_default();
        let mut expanded = Vec::with_capacity(height);
        for line in &rows {
            let mut out = Vec::with_capacity(width * 3);
            for &index in line {
                let Some(entry) = palette.get(usize::from(index)) else {
                    return error("palette index out of range");
                };
                out.extend_from_slice(entry);
            }
            expanded.push(out);
        }
        return Ok(Image {
            width,
            height,
            channels: 3,
            rows: expanded,
        });
    }
    Ok(Image {
        width,
        height,
        channels,
        rows,
    })
}

/// Encodes 8-bit pixels as a PNG, cycling through the five row filters so that a decoder's
/// handling of each one is exercised (for tests).
pub fn encode(
    width: u32,
    height: u32,
    colour_type: u8,
    pixels: &[u8],
    palette: Option<&[[u8; 3]]>,
) -> Vec<u8> {
    let channels = match colour_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        _ => 4,
    };
    let stride = width as usize * channels;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    let mut previous = vec![0u8; stride];
    for (y, line) in pixels.chunks_exact(stride).enumerate() {
        let filter = (y % 5) as u8;
        raw.push(filter);
        for i in 0..stride {
            let left = if i >= channels { line[i - channels] } else { 0 };
            let up = previous[i];
            let upper_left = if i >= channels {
                previous[i - channels]
            } else {
                0
            };
            let predictor = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) >> 1) as u8,
                _ => paeth(left, up, upper_left),
            };
            raw.push(line[i].wrapping_sub(predictor));
        }
        previous.copy_from_slice(line);
    }
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, colour_type, 0, 0, 0]);
    let mut out = SIGNATURE.to_vec();
    push_chunk(&mut out, b"IHDR", &ihdr);
    if let Some(palette) = palette {
        let body: Vec<u8> = palette.iter().flatten().copied().collect();
        push_chunk(&mut out, b"PLTE", &body);
    }
    push_chunk(
        &mut out,
        b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
    );
    push_chunk(&mut out, b"IEND", &[]);
    out
}

/// Appends a chunk with its length and CRC.
pub fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out.extend_from_slice(&crc32(&[kind, body]).to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32, channels: usize) -> Vec<u8> {
        (0..width as usize * height as usize * channels)
            .map(|i| (i * 7 % 251) as u8)
            .collect()
    }

    #[test]
    fn crc_matches_the_standard_check_value() {
        assert_eq!(crc32(&[b"123456789"]), 0xcbf4_3926);
        assert_eq!(crc32(&[b"IEND"]), 0xae42_6082);
    }

    #[test]
    fn round_trips_every_colour_type_and_filter() {
        for (colour_type, channels) in [(0u8, 1usize), (2, 3), (4, 2), (6, 4)] {
            let pixels = gradient(13, 11, channels);
            let image = decode(&encode(13, 11, colour_type, &pixels, None)).expect("decodes");
            assert_eq!(
                (image.width, image.height, image.channels),
                (13, 11, channels)
            );
            assert_eq!(image.rows.concat(), pixels, "colour type {colour_type}");
        }
    }

    #[test]
    fn expands_palettes() {
        let palette = [[255, 0, 0], [0, 0, 255]];
        let image = decode(&encode(2, 1, 3, &[1, 0], Some(&palette))).expect("decodes");
        assert_eq!(image.pixel(0, 0), [0, 0, 255]);
        assert_eq!(image.pixel(1, 0), [255, 0, 0]);
        let bad = encode(1, 1, 3, &[2], Some(&palette));
        assert_eq!(decode(&bad).unwrap_err().0, "palette index out of range");
        let missing = encode(1, 1, 3, &[0], None);
        assert_eq!(
            decode(&missing).unwrap_err().0,
            "palette image without a PLTE chunk"
        );
    }

    fn with_ihdr(width: u32, height: u32, rest: [u8; 5]) -> Vec<u8> {
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&rest);
        let mut out = SIGNATURE.to_vec();
        push_chunk(&mut out, b"IHDR", &ihdr);
        out
    }

    fn message(data: &[u8]) -> String {
        decode(data).expect_err("must fail").0
    }

    #[test]
    fn refuses_malformed_files() {
        let good = encode(4, 4, 2, &gradient(4, 4, 3), None);
        assert_eq!(message(b"GIF89a"), "not a PNG file (bad signature)");
        assert_eq!(
            message(&good[..good.len() - 2]),
            format!("truncated IEND chunk at byte {}", good.len() - 12)
        );
        assert_eq!(message(&good[..10]), "truncated chunk header at byte 8");
        let mut corrupt = good.clone();
        corrupt[20] ^= 1;
        assert_eq!(message(&corrupt), "CRC mismatch in IHDR chunk");
        assert_eq!(
            message(&good[..good.len() - 12]),
            "no IEND chunk (file truncated?)"
        );
        assert_eq!(message(SIGNATURE), "no IHDR chunk");

        let mut first_not_ihdr = SIGNATURE.to_vec();
        push_chunk(&mut first_not_ihdr, b"IEND", &[]);
        assert_eq!(message(&first_not_ihdr), "first chunk is not IHDR");

        let finish = |mut data: Vec<u8>| {
            push_chunk(&mut data, b"IEND", &[]);
            data
        };
        assert_eq!(
            message(&finish(with_ihdr(1, 1, [16, 2, 0, 0, 0]))),
            "unsupported bit depth 16 (only 8 is handled)"
        );
        assert_eq!(
            message(&finish(with_ihdr(1, 1, [8, 5, 0, 0, 0]))),
            "unsupported colour type 5"
        );
        assert_eq!(
            message(&finish(with_ihdr(1, 1, [8, 2, 1, 0, 0]))),
            "unknown compression or filter method in IHDR"
        );
        assert_eq!(
            message(&finish(with_ihdr(1, 1, [8, 2, 0, 0, 1]))),
            "interlaced PNGs are not handled"
        );
        assert_eq!(
            message(&finish(with_ihdr(0, 1, [8, 2, 0, 0, 0]))),
            "empty image"
        );
        assert_eq!(
            message(&finish(with_ihdr(20_000, 1, [8, 2, 0, 0, 0]))),
            "20000x1 is larger than any screenshot this checks"
        );
        assert_eq!(
            message(&finish(with_ihdr(10_000, 10_000, [8, 2, 0, 0, 0]))),
            "10000x10000 is larger than any screenshot this checks"
        );

        let mut two_headers = with_ihdr(1, 1, [8, 2, 0, 0, 0]);
        push_chunk(&mut two_headers, b"IHDR", &[0; 13]);
        assert_eq!(message(&finish(two_headers)), "more than one IHDR chunk");
        let mut short_header = SIGNATURE.to_vec();
        push_chunk(&mut short_header, b"IHDR", &[0; 12]);
        assert_eq!(
            message(&finish(short_header)),
            "IHDR chunk is 12 bytes long, not 13"
        );
        let mut bad_palette = with_ihdr(1, 1, [8, 3, 0, 0, 0]);
        push_chunk(&mut bad_palette, b"PLTE", &[0; 4]);
        assert_eq!(
            message(&finish(bad_palette)),
            "PLTE chunk is 4 bytes long, not a multiple of 3 up to 768"
        );
    }

    fn with_data(width: u32, height: u32, raw: &[u8], compress: bool) -> Vec<u8> {
        let mut data = with_ihdr(width, height, [8, 2, 0, 0, 0]);
        let body = if compress {
            miniz_oxide::deflate::compress_to_vec_zlib(raw, 6)
        } else {
            raw.to_vec()
        };
        push_chunk(&mut data, b"IDAT", &body);
        push_chunk(&mut data, b"IEND", &[]);
        data
    }

    #[test]
    fn checks_the_image_data_against_the_header() {
        // A 1x1 RGB image needs 4 bytes: the filter type and one pixel.
        assert_eq!(
            message(&with_data(1, 1, &[0; 1_000_000], true)),
            "image data is larger than the 1x1 header allows"
        );
        assert_eq!(
            message(&with_data(1, 1, &[0; 3], true)),
            "image data has 3 bytes, expected 4"
        );
        let stream = miniz_oxide::deflate::compress_to_vec_zlib(&[0; 4], 6);
        assert_eq!(
            message(&with_data(1, 1, &stream[..stream.len() - 5], false)),
            "image data stream is incomplete"
        );
        assert!(
            message(&with_data(1, 1, b"not zlib at all", false))
                .starts_with("image data does not decompress")
        );
        assert_eq!(
            message(&with_data(1, 1, &[9, 0, 0, 0], true)),
            "unknown filter type 9 on row 0"
        );
    }
}
