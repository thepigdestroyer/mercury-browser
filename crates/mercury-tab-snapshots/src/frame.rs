// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The one picture type of this crate.

use std::fmt;

/// The largest width or height accepted, in pixels. No screen is bigger, and the bound keeps the
/// size arithmetic far from overflow.
pub const MAX_DIMENSION: u32 = 16_384;

/// A tightly packed picture: 4 bytes per pixel (red, green, blue, alpha), rows from the top to the
/// bottom, no padding between rows. The only way to make one is [`RgbaFrame::new`], which checks
/// the size, so every `RgbaFrame` has `width * height * 4` bytes and no zero side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// Why a buffer is not a picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The width or the height is zero.
    Empty,
    /// The width or the height is above [`MAX_DIMENSION`].
    TooLarge { width: u32, height: u32 },
    /// The buffer is not `width * height * 4` bytes long.
    LengthMismatch { expected: usize, actual: usize },
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("the picture has no pixels"),
            Self::TooLarge { width, height } => write!(
                formatter,
                "a picture of {width}x{height} pixels is larger than {MAX_DIMENSION} on a side"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "the pixel buffer holds {actual} bytes, but the size needs {expected}"
            ),
        }
    }
}

impl std::error::Error for FrameError {}

impl RgbaFrame {
    /// Wraps `pixels` (RGBA8, top row first) as a picture of `width` by `height` pixels.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::Empty);
        }
        if width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(FrameError::TooLarge { width, height });
        }
        // At most 16384 * 16384 * 4 = 2^30, which fits a `usize` on every supported target.
        let expected = width as usize * height as usize * 4;
        if pixels.len() != expected {
            return Err(FrameError::LengthMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels: RGBA8, top row first, `width * height * 4` bytes.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Gives the pixel buffer back, for example to hand it to an image library.
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }

    /// The size of the pixel buffer in bytes.
    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_of_the_right_length_is_a_picture() {
        let frame = RgbaFrame::new(3, 2, vec![7; 24]).expect("3x2 needs 24 bytes");

        assert_eq!((frame.width(), frame.height()), (3, 2));
        assert_eq!(frame.byte_len(), 24);
        assert_eq!(frame.pixels(), &[7; 24][..]);
        assert_eq!(frame.into_pixels(), vec![7; 24]);
    }

    #[test]
    fn a_picture_with_no_pixels_is_refused() {
        assert_eq!(RgbaFrame::new(0, 5, Vec::new()), Err(FrameError::Empty));
        assert_eq!(RgbaFrame::new(5, 0, Vec::new()), Err(FrameError::Empty));
    }

    #[test]
    fn a_picture_above_the_largest_side_is_refused_before_its_buffer_is_measured() {
        assert_eq!(
            RgbaFrame::new(MAX_DIMENSION + 1, 1, Vec::new()),
            Err(FrameError::TooLarge {
                width: MAX_DIMENSION + 1,
                height: 1
            })
        );
        assert_eq!(
            RgbaFrame::new(1, u32::MAX, Vec::new()),
            Err(FrameError::TooLarge {
                width: 1,
                height: u32::MAX
            })
        );
    }

    #[test]
    fn a_buffer_of_the_wrong_length_is_refused() {
        assert_eq!(
            RgbaFrame::new(2, 2, vec![0; 15]),
            Err(FrameError::LengthMismatch {
                expected: 16,
                actual: 15
            })
        );
        assert_eq!(
            RgbaFrame::new(2, 2, vec![0; 17]),
            Err(FrameError::LengthMismatch {
                expected: 16,
                actual: 17
            })
        );
    }

    #[test]
    fn errors_describe_themselves() {
        assert_eq!(FrameError::Empty.to_string(), "the picture has no pixels");
        assert!(
            FrameError::LengthMismatch {
                expected: 16,
                actual: 15
            }
            .to_string()
            .contains("15 bytes")
        );
    }
}
