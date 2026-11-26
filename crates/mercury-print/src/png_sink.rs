// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A `RowSink` that streams the rows of a full-page screenshot into a PNG file. The rows are
//! written as they arrive, so the page never has to fit in memory. The file is written under a
//! `.part` name and renamed into place only when every row has been written.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use image::RgbaImage;
use png::StreamWriter;

use crate::capture::{RowSink, SinkError};

/// Streams RGBA rows into a PNG file of a known size.
pub struct PngSink {
    target: PathBuf,
    part: Option<PathBuf>,
    width: u32,
    height: u32,
    written: u32,
    writer: Option<StreamWriter<'static, File>>,
}

impl PngSink {
    /// Starts the file `<target>.part` for an image of `width` by `height` pixels.
    pub fn create(target: &Path, width: u32, height: u32) -> Result<PngSink, SinkError> {
        let part = part_path(target);
        let file = File::create(&part)?;
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let writer = match encoder.write_header().and_then(|w| w.into_stream_writer()) {
            Ok(writer) => writer,
            Err(error) => {
                let _ = std::fs::remove_file(&part);
                return Err(error.into());
            }
        };
        Ok(PngSink {
            target: target.to_path_buf(),
            part: Some(part),
            width,
            height,
            written: 0,
            writer: Some(writer),
        })
    }
}

impl RowSink for PngSink {
    fn push_rows(&mut self, start_row: u32, rows: &RgbaImage) -> Result<(), SinkError> {
        if start_row != self.written {
            return Err(SinkError::OutOfOrder {
                expected: self.written,
                got: start_row,
            });
        }
        if rows.width() != self.width {
            return Err(SinkError::WidthChanged {
                expected: self.width,
                found: rows.width(),
            });
        }
        if self.written.saturating_add(rows.height()) > self.height {
            return Err(SinkError::TooManyRows {
                height: self.height,
            });
        }
        if let Some(writer) = self.writer.as_mut() {
            writer.write_all(rows.as_raw())?;
        }
        self.written += rows.height();
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<(), SinkError> {
        if self.written != self.height {
            return Err(SinkError::Incomplete {
                written: self.written,
                expected: self.height,
            });
        }
        if let Some(writer) = self.writer.take() {
            writer.finish()?;
        }
        if let Some(part) = self.part.take() {
            std::fs::rename(&part, &self.target)?;
        }
        Ok(())
    }
}

impl Drop for PngSink {
    fn drop(&mut self) {
        drop(self.writer.take());
        if let Some(part) = self.part.take() {
            let _ = std::fs::remove_file(part);
        }
    }
}

fn part_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    const WIDTH: u32 = 16;

    /// Rows `start..start + height` of a page where every pixel of row `r` encodes `r`.
    fn rows(start: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(WIDTH, height, |x, y| {
            let row = start + y;
            Rgba([row as u8, (row >> 8) as u8, x as u8, 255])
        })
    }

    #[test]
    fn png_sink_streams_rows_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("shot.png");
        let mut sink: Box<dyn RowSink> = Box::new(PngSink::create(&target, WIDTH, 120).unwrap());
        for start in [0, 40, 80] {
            sink.push_rows(start, &rows(start, 40)).unwrap();
        }
        sink.finish().unwrap();
        let decoded = image::open(&target).unwrap().to_rgba8();
        assert_eq!(decoded, rows(0, 120));
    }

    #[test]
    fn png_sink_rejects_rows_out_of_order() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("shot.png");
        let mut sink = PngSink::create(&target, WIDTH, 120).unwrap();
        sink.push_rows(0, &rows(0, 40)).unwrap();
        assert!(matches!(
            sink.push_rows(80, &rows(80, 40)),
            Err(SinkError::OutOfOrder {
                expected: 40,
                got: 80
            })
        ));
    }

    #[test]
    fn png_sink_rejects_a_width_change_and_extra_rows() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("shot.png");
        let mut sink = PngSink::create(&target, WIDTH, 40).unwrap();
        let wide = RgbaImage::from_pixel(WIDTH + 1, 10, Rgba([0, 0, 0, 255]));
        assert!(matches!(
            sink.push_rows(0, &wide),
            Err(SinkError::WidthChanged { .. })
        ));
        assert!(matches!(
            sink.push_rows(0, &rows(0, 50)),
            Err(SinkError::TooManyRows { height: 40 })
        ));
    }

    #[test]
    fn png_sink_finish_needs_every_row() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("shot.png");
        let mut sink: Box<dyn RowSink> = Box::new(PngSink::create(&target, WIDTH, 80).unwrap());
        sink.push_rows(0, &rows(0, 40)).unwrap();
        assert!(matches!(
            sink.finish(),
            Err(SinkError::Incomplete {
                written: 40,
                expected: 80
            })
        ));
        assert!(!target.exists());
    }

    #[test]
    fn png_sink_writes_to_part_then_renames() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("shot.png");
        let mut sink: Box<dyn RowSink> = Box::new(PngSink::create(&target, WIDTH, 40).unwrap());
        sink.push_rows(0, &rows(0, 40)).unwrap();
        assert!(!target.exists());
        assert!(part_path(&target).exists());
        sink.finish().unwrap();
        assert!(target.exists());
        assert!(!part_path(&target).exists());
    }

    #[test]
    fn dropped_png_sink_removes_its_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("shot.png");
        let mut sink = PngSink::create(&target, WIDTH, 40).unwrap();
        sink.push_rows(0, &rows(0, 20)).unwrap();
        assert!(part_path(&target).exists());
        drop(sink);
        assert!(!target.exists());
        assert!(!part_path(&target).exists());
    }
}
