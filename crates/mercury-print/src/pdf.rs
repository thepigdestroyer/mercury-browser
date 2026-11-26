// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Writing the printed pages as a PDF: each captured page is an image, placed at the margin of a
//! paper-sized PDF page, with the URL and the time in the document's metadata.

use std::time::{SystemTime, UNIX_EPOCH};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use krilla::geom::{Size, Transform};
use krilla::image::Image;
use krilla::metadata::{DateTime, Metadata};
use krilla::page::PageSettings;
use krilla::{Data, Document, SerializeSettings};

use crate::names::civil_from_days;
use crate::setup::PageSetup;

/// The JPEG quality used for pages that are too large as PNG.
const JPEG_QUALITY: u8 = 92;

/// A page is encoded as JPEG once its PNG is larger than this share of its raw RGBA size.
const PNG_SHARE_LIMIT_PERCENT: u64 = 35;

/// The producer written into every PDF this crate makes.
const PRODUCER: &str = "Mercury Browser (image print)";

/// What is drawn on a PDF page.
pub enum PageContent {
    /// A captured page, as an encoded picture.
    Image(EncodedPage),
}

/// A captured page, encoded once so the PDF writer and the thumbnails never re-encode it.
pub struct EncodedPage {
    pub width_px: u32,
    pub height_px: u32,
    pub data: EncodedData,
}

/// The bytes of an encoded page.
pub enum EncodedData {
    Png(Vec<u8>),
    Jpeg(Vec<u8>),
}

/// The document information written into the PDF.
pub struct PdfMetadata {
    pub title: String,
    pub author: Option<String>,
    pub url: String,
    pub created: SystemTime,
}

/// Why a PDF could not be written.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("there are no pages to write")]
    NoPages,
    #[error("the paper size does not give a valid PDF page")]
    PageSize,
    #[error("a page could not be read back for the PDF: {0}")]
    Image(String),
    #[error("the PDF could not be written: {0}")]
    Write(String),
}

/// Encodes a captured page as PNG, or as JPEG (quality 92, over white) when the PNG is more than
/// 35 % of the raw RGBA size, so photographs stay small and text stays sharp.
pub fn encode_page(page: &RgbaImage) -> EncodedPage {
    let (width_px, height_px) = page.dimensions();
    let png = encode_png(page);
    let raw = u64::from(width_px) * u64::from(height_px) * 4;
    let too_large_as_png = (png.len() as u64) * 100 > raw * PNG_SHARE_LIMIT_PERCENT;

    let data = match encode_jpeg(page) {
        Some(jpeg) if too_large_as_png => EncodedData::Jpeg(jpeg),
        _ => EncodedData::Png(png),
    };
    EncodedPage {
        width_px,
        height_px,
        data,
    }
}

/// The page scaled to `max_width` pixels wide, keeping its aspect ratio; a page that is already
/// narrower is returned unchanged.
pub fn thumbnail(page: &RgbaImage, max_width: u32) -> RgbaImage {
    let (width, height) = page.dimensions();
    let max_width = max_width.max(1);
    if width <= max_width {
        return page.clone();
    }
    let scaled_height =
        (u64::from(height) * u64::from(max_width) + u64::from(width) / 2) / u64::from(width);
    let scaled_height = u32::try_from(scaled_height).unwrap_or(u32::MAX).max(1);
    imageops::resize(page, max_width, scaled_height, FilterType::Triangle)
}

/// Writes one PDF page per encoded page. Each page has the paper size of `setup`; the picture is
/// placed at the margin, as wide as the content box, and scaled down if it is taller than the box.
/// Pages with no pixels are skipped.
///
/// `content_dpr` is the device pixels per CSS pixel the pages were captured at. Pictures keep
/// their own proportions, so the writer does not need it; it is kept for the vector pages of EB3.
pub fn write_pdf(
    pages: &[&EncodedPage],
    setup: &PageSetup,
    _content_dpr: f32,
    meta: &PdfMetadata,
) -> Result<Vec<u8>, PdfError> {
    let drawable: Vec<&EncodedPage> = pages
        .iter()
        .copied()
        .filter(|page| page.width_px > 0 && page.height_px > 0)
        .collect();
    if drawable.is_empty() {
        return Err(PdfError::NoPages);
    }

    let (page_width, page_height) = setup.page_points();
    if PageSettings::from_wh(page_width, page_height).is_none() {
        return Err(PdfError::PageSize);
    }
    let margin = setup.margin_points();
    let (content_width, content_height) = setup.content_points();

    // Content streams stay uncompressed: each is one `cm` and one `Do`, and the pictures are
    // already compressed, so the tests can read the placement directly.
    let mut document = Document::new_with(SerializeSettings {
        compress_content_streams: false,
        ..SerializeSettings::default()
    });
    document.set_metadata(metadata(meta));

    for page in drawable {
        let (width_pt, height_pt) = fitted_size(page, content_width, content_height);
        let size = Size::from_wh(width_pt, height_pt).ok_or(PdfError::PageSize)?;
        let image = image_for(page)?;

        let settings = PageSettings::from_wh(page_width, page_height).ok_or(PdfError::PageSize)?;
        let mut pdf_page = document.start_page_with(settings);
        let mut surface = pdf_page.surface();
        surface.push_transform(&Transform::from_translate(margin, margin));
        surface.draw_image(image, size);
        surface.pop();
        surface.finish();
        pdf_page.finish();
    }

    document
        .finish()
        .map_err(|error| PdfError::Write(error.to_string()))
}

fn encode_png(page: &RgbaImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    let encoder =
        PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, PngFilter::Adaptive);
    match encoder.write_image(
        page.as_raw(),
        page.width(),
        page.height(),
        ExtendedColorType::Rgba8,
    ) {
        Ok(()) => bytes,
        Err(_) => Vec::new(),
    }
}

fn encode_jpeg(page: &RgbaImage) -> Option<Vec<u8>> {
    let mut rgb = Vec::with_capacity(page.as_raw().len() / 4 * 3);
    for pixel in page.pixels() {
        let alpha = u32::from(pixel[3]);
        for channel in &pixel.0[..3] {
            let over_white = (u32::from(*channel) * alpha + 255 * (255 - alpha)) / 255;
            rgb.push(over_white as u8);
        }
    }
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY)
        .encode(&rgb, page.width(), page.height(), ExtendedColorType::Rgb8)
        .ok()?;
    Some(bytes)
}

fn image_for(page: &EncodedPage) -> Result<Image, PdfError> {
    match &page.data {
        EncodedData::Png(bytes) => Image::from_png(Data::from(bytes.clone()), false),
        EncodedData::Jpeg(bytes) => Image::from_jpeg(Data::from(bytes.clone()), false),
    }
    .map_err(PdfError::Image)
}

/// The picture's size in points: as wide as the content box, or narrower when that would make it
/// taller than the box.
fn fitted_size(page: &EncodedPage, content_width: f32, content_height: f32) -> (f32, f32) {
    let width = content_width;
    let height = content_width * page.height_px as f32 / page.width_px as f32;
    if height <= content_height {
        (width, height)
    } else {
        let scale = content_height / height;
        (width * scale, content_height)
    }
}

fn metadata(meta: &PdfMetadata) -> Metadata {
    let authors = meta.author.iter().cloned().collect();
    Metadata::new()
        .title(meta.title.clone())
        .producer(PRODUCER.to_owned())
        .description(format!("Printed as pictures from {}", meta.url))
        .authors(authors)
        .creation_date(utc_date_time(meta.created))
}

fn utc_date_time(time: SystemTime) -> DateTime {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let in_day = seconds.rem_euclid(86_400);
    DateTime::new(u16::try_from(year).unwrap_or(9999))
        .month(month as u8)
        .day(day as u8)
        .hour((in_day / 3_600) as u8)
        .minute((in_day % 3_600 / 60) as u8)
        .second((in_day % 60) as u8)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use image::Rgba;

    use super::*;
    use crate::paper::{MarginPreset, Orientation, PaperSize};

    fn a4_portrait() -> PageSetup {
        PageSetup {
            paper: PaperSize::A4,
            orientation: Orientation::Portrait,
            margins: MarginPreset::Default,
            scale_percent: 100,
        }
    }

    fn text_page() -> RgbaImage {
        RgbaImage::from_fn(1440, 2000, |_, y| {
            if (y / 20) % 2 == 0 {
                Rgba([255, 255, 255, 255])
            } else {
                Rgba([0, 0, 0, 255])
            }
        })
    }

    fn noise_page(width: u32, height: u32) -> RgbaImage {
        let mut state: u32 = 0x1234_5678;
        RgbaImage::from_fn(width, height, |_, _| {
            let mut px = [255u8; 4];
            for channel in px.iter_mut().take(3) {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *channel = (state >> 24) as u8;
            }
            Rgba(px)
        })
    }

    fn meta() -> PdfMetadata {
        PdfMetadata {
            title: "My page".to_owned(),
            author: Some("Ada".to_owned()),
            url: "https://example.com/a".to_owned(),
            created: UNIX_EPOCH + Duration::from_secs(1_795_356_189),
        }
    }

    fn page_count(pdf: &[u8]) -> usize {
        let text = String::from_utf8_lossy(pdf);
        text.match_indices("/Type")
            .filter(|(index, _)| {
                let rest = text[index + "/Type".len()..].trim_start();
                rest.starts_with("/Page") && !rest["/Page".len()..].starts_with('s')
            })
            .count()
    }

    fn media_boxes(pdf: &[u8]) -> Vec<[f32; 4]> {
        let text = String::from_utf8_lossy(pdf);
        text.match_indices("/MediaBox")
            .map(|(index, _)| {
                let rest = &text[index..];
                let open = rest.find('[').expect("MediaBox has an array");
                let close = rest[open..].find(']').expect("MediaBox array closes") + open;
                let numbers: Vec<f32> = rest[open + 1..close]
                    .split_whitespace()
                    .map(|n| n.parse().expect("MediaBox number"))
                    .collect();
                [numbers[0], numbers[1], numbers[2], numbers[3]]
            })
            .collect()
    }

    /// The six numbers before each `cm` operator, as `[a, b, c, d, e, f]`.
    fn cm_matrices(pdf: &[u8]) -> Vec<[f32; 6]> {
        let text = String::from_utf8_lossy(pdf);
        text.match_indices(" cm")
            .filter_map(|(index, _)| {
                let tokens: Vec<&str> = text[..index].split_whitespace().collect();
                let tail = tokens.get(tokens.len().checked_sub(6)?..)?;
                let mut matrix = [0.0f32; 6];
                for (slot, token) in matrix.iter_mut().zip(tail) {
                    *slot = token.parse().ok()?;
                }
                Some(matrix)
            })
            .collect()
    }

    fn assert_close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected} ± {tolerance}, got {actual}"
        );
    }

    #[test]
    fn text_page_is_encoded_as_png() {
        let page = text_page();
        let encoded = encode_page(&page);
        let raw = 1440u64 * 2000 * 4;
        match &encoded.data {
            EncodedData::Png(bytes) => assert!(
                (bytes.len() as u64) * 100 < raw * 35,
                "PNG of {} bytes is not under 35% of {raw}",
                bytes.len()
            ),
            EncodedData::Jpeg(_) => panic!("text page must stay PNG"),
        }
        assert_eq!((encoded.width_px, encoded.height_px), (1440, 2000));
    }

    #[test]
    fn photo_page_is_encoded_as_jpeg() {
        let page = noise_page(300, 200);
        let encoded = encode_page(&page);
        let jpeg = match &encoded.data {
            EncodedData::Jpeg(bytes) => bytes,
            EncodedData::Png(_) => panic!("noise page must become JPEG"),
        };
        let decoded = image::load_from_memory(jpeg).expect("JPEG decodes");
        assert_eq!((decoded.width(), decoded.height()), (300, 200));
    }

    #[test]
    fn thumbnail_keeps_aspect() {
        let page = RgbaImage::new(1440, 2000);
        let small = thumbnail(&page, 200);
        assert_eq!(small.dimensions(), (200, 278));

        let narrow = RgbaImage::new(100, 50);
        assert_eq!(thumbnail(&narrow, 200).dimensions(), (100, 50));
    }

    #[test]
    fn pdf_has_one_page_per_image_and_the_paper_box() {
        let encoded = encode_page(&text_page());
        let pages = [&encoded, &encoded];
        let pdf = write_pdf(&pages, &a4_portrait(), 2.0, &meta()).expect("PDF writes");

        assert!(pdf.starts_with(b"%PDF-"));
        assert_eq!(page_count(&pdf), 2);
        let boxes = media_boxes(&pdf);
        assert_eq!(boxes.len(), 2);
        for box_ in boxes {
            assert_close(box_[2], 595.28, 0.01);
            assert_close(box_[3], 841.89, 0.01);
        }
    }

    #[test]
    fn landscape_swaps_the_box() {
        let encoded = encode_page(&text_page());
        let mut setup = a4_portrait();
        setup.orientation = Orientation::Landscape;
        let pdf = write_pdf(&[&encoded], &setup, 2.0, &meta()).expect("PDF writes");

        let boxes = media_boxes(&pdf);
        assert_eq!(boxes.len(), 1);
        assert_close(boxes[0][2], 841.89, 0.01);
        assert_close(boxes[0][3], 595.28, 0.01);
    }

    #[test]
    fn image_is_placed_at_the_margin_and_content_width() {
        let encoded = encode_page(&text_page());
        let pdf = write_pdf(&[&encoded], &a4_portrait(), 2.0, &meta()).expect("PDF writes");

        let matrices = cm_matrices(&pdf);
        let placed = matrices
            .iter()
            .find(|m| (m[0] - 538.58).abs() <= 0.1)
            .expect("a cm with the content width");
        assert_close(placed[4], 28.35, 0.1);
    }

    #[test]
    fn metadata_is_written() {
        let encoded = encode_page(&text_page());
        let pdf = write_pdf(&[&encoded], &a4_portrait(), 2.0, &meta()).expect("PDF writes");
        let text = String::from_utf8_lossy(&pdf);

        assert!(text.contains("My page"), "Title is written");
        assert!(
            text.contains("https://example.com/a"),
            "Subject carries the URL"
        );
        assert!(text.contains(PRODUCER), "Producer is Mercury Browser");
    }

    #[test]
    fn zero_pages_is_an_error() {
        let result = write_pdf(&[], &a4_portrait(), 2.0, &meta());
        assert!(matches!(result, Err(PdfError::NoPages)));
    }

    #[test]
    fn a_zero_sized_page_is_skipped_not_panicked() {
        let empty = EncodedPage {
            width_px: 0,
            height_px: 0,
            data: EncodedData::Png(Vec::new()),
        };
        let good = encode_page(&text_page());
        let pdf = write_pdf(&[&empty, &good], &a4_portrait(), 2.0, &meta()).expect("PDF writes");
        assert_eq!(page_count(&pdf), 1);

        let only_empty = write_pdf(&[&empty], &a4_portrait(), 2.0, &meta());
        assert!(matches!(only_empty, Err(PdfError::NoPages)));
    }
}
