// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What an AV1 track's `av1C` record (the WebM `CodecPrivate` of `V_AV1`, the `av1C` box of MP4) says:
//! the four fixed bytes, then the `configOBUs`, which hold the sequence header OBU.
//!
//! Why this exists: Apple's VideoToolbox only starts an AV1 decode session when the format description
//! carries an `av1C` that includes the sequence header (a community recipe, not an Apple statement). The
//! fixture `av1-low` was made with a hand-written four-byte `av1C` (`81000c00`, no sequence header), so
//! its decode failure on the Apple Silicon runner could have been the fixture and not the chip; the
//! fixture `av1-low-av1c` has the full record, and these functions let a test say so in code.

use crate::webm::parse_init;
use crate::{Segment, SplitError};

/// An `av1C` record, split into its fixed part and its `configOBUs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Av1Config {
    /// Byte 0: marker (1) and version (1).
    pub marker_version: u8,
    /// `seq_profile` (3 bits) and `seq_level_idx_0` (5 bits).
    pub profile_level: u8,
    /// `seq_tier_0`, `high_bitdepth`, `twelve_bit`, `monochrome`, `chroma_subsampling_x` and `y`,
    /// `chroma_sample_position` (2 bits).
    pub flags: u8,
    /// Byte 3: `initial_presentation_delay` fields.
    pub delay: u8,
    /// Everything after the fourth byte.
    pub config_obus: Vec<u8>,
}

impl Av1Config {
    /// Parses an `av1C` record; `None` when it is shorter than the four fixed bytes or its marker bit or
    /// version is wrong.
    pub fn parse(av1c: &[u8]) -> Option<Av1Config> {
        let [marker_version, profile_level, flags, delay, rest @ ..] = av1c else {
            return None;
        };
        if *marker_version != 0x81 {
            return None;
        }
        Some(Av1Config {
            marker_version: *marker_version,
            profile_level: *profile_level,
            flags: *flags,
            delay: *delay,
            config_obus: rest.to_vec(),
        })
    }

    /// The OBU types of the complete OBUs in `configOBUs`, in order (1 is a sequence header). A truncated
    /// OBU ends the list.
    pub fn obu_types(&self) -> Vec<u8> {
        obu_types(&self.config_obus)
    }

    /// Whether `configOBUs` holds a complete sequence header OBU (type 1).
    pub fn has_sequence_header(&self) -> bool {
        self.obu_types().contains(&1)
    }
}

/// The type of each complete OBU in `data`, for OBUs that carry a size field (as `configOBUs` must).
pub fn obu_types(data: &[u8]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut at = 0usize;
    while let Some(&header) = data.get(at) {
        let has_extension = header & 0x04 != 0;
        let has_size = header & 0x02 != 0;
        if header & 0x80 != 0 || !has_size {
            break;
        }
        let mut pos = at + 1 + usize::from(has_extension);
        let Some((size, used)) = leb128(data.get(pos..).unwrap_or_default()) else {
            break;
        };
        pos += used;
        let Some(end) = pos.checked_add(size).filter(|end| *end <= data.len()) else {
            break;
        };
        types.push((header >> 3) & 0x0f);
        at = end;
    }
    types
}

/// An unsigned LEB128 number (at most 8 bytes, as AV1 allows): (value, bytes used).
fn leb128(data: &[u8]) -> Option<(usize, usize)> {
    let mut value: u64 = 0;
    for (index, byte) in data.iter().take(8).enumerate() {
        value |= u64::from(byte & 0x7f) << (7 * index);
        if byte & 0x80 == 0 {
            return usize::try_from(value).ok().map(|v| (v, index + 1));
        }
    }
    None
}

/// The `CodecPrivate` of the first `V_AV1` track of a WebM initialisation segment, parsed as an `av1C`
/// record. `Ok(None)` when the file has no AV1 track or the track has no (or an invalid) `CodecPrivate`.
pub fn webm_av1_config(init: &Segment) -> Result<Option<Av1Config>, SplitError> {
    let info = parse_init(&init.bytes)?;
    Ok(info
        .tracks
        .iter()
        .find(|track| track.codec_id == "V_AV1")
        .and_then(|track| Av1Config::parse(&track.codec_private)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load;

    /// The record `gen.sh` wrote by hand into `av1-low`.
    const HAND_WRITTEN: [u8; 4] = [0x81, 0x00, 0x0c, 0x00];

    #[test]
    fn a_record_needs_four_bytes_and_the_marker() {
        assert_eq!(Av1Config::parse(&[]), None);
        assert_eq!(Av1Config::parse(&[0x81, 0x00, 0x0c]), None);
        assert_eq!(Av1Config::parse(&[0x01, 0x00, 0x0c, 0x00]), None);
        let config = Av1Config::parse(&HAND_WRITTEN).unwrap();
        assert!(config.config_obus.is_empty());
        assert!(!config.has_sequence_header());
    }

    #[test]
    fn leb128_reads_one_and_two_byte_numbers_and_refuses_an_unterminated_one() {
        assert_eq!(leb128(&[0x0a]), Some((10, 1)));
        assert_eq!(leb128(&[0xe5, 0x8e, 0x26]), Some((624_485, 3)));
        assert_eq!(leb128(&[0x80]), None);
        assert_eq!(leb128(&[]), None);
        assert_eq!(leb128(&[0x80; 9]), None);
    }

    #[test]
    fn obus_are_walked_by_their_size_fields() {
        // A sequence header (type 1, 3 payload bytes) then a padding OBU (type 15, 1 byte).
        let data = [0x0a, 0x03, 1, 2, 3, 0x7a, 0x01, 9];
        assert_eq!(obu_types(&data), [1, 15]);
        // The same cut short: the padding OBU is incomplete and is not listed.
        assert_eq!(obu_types(&data[..7]), [1]);
        // A header without a size field ends the walk; so does a set forbidden bit.
        assert!(obu_types(&[0x08, 1, 2]).is_empty());
        assert!(obu_types(&[0x8a, 0x00]).is_empty());
        // An extension byte is skipped before the size.
        assert_eq!(obu_types(&[0x0e, 0x00, 0x01, 7]), [1]);
    }

    #[test]
    fn a_sequence_header_that_is_cut_short_does_not_count() {
        let mut av1c = HAND_WRITTEN.to_vec();
        av1c.extend_from_slice(&[0x0a, 0x0b, 1, 2, 3]);
        assert!(!Av1Config::parse(&av1c).unwrap().has_sequence_header());
    }

    #[test]
    fn the_original_av1_fixture_has_the_four_byte_record_and_no_sequence_header() {
        let stream = load("av1-low").unwrap();
        let config = webm_av1_config(&stream.init).unwrap().unwrap();
        assert_eq!(
            [
                config.marker_version,
                config.profile_level,
                config.flags,
                config.delay
            ],
            HAND_WRITTEN
        );
        assert!(config.config_obus.is_empty());
        assert!(!config.has_sequence_header());
    }

    #[test]
    fn the_control_fixture_carries_the_full_record_with_the_sequence_header() {
        let stream = load("av1-low-av1c").unwrap();
        let config = webm_av1_config(&stream.init).unwrap().unwrap();
        // The same four fixed bytes as the original: the two fixtures differ only in the configOBUs.
        assert_eq!(
            [
                config.marker_version,
                config.profile_level,
                config.flags,
                config.delay
            ],
            HAND_WRITTEN
        );
        assert_eq!(
            config.obu_types(),
            [1],
            "exactly one OBU, a sequence header"
        );
        assert!(config.has_sequence_header());
        // The OBU is complete: its size field accounts for every byte after it.
        assert_eq!(config.config_obus[0], 0x0a, "type 1 with a size field");
        assert_eq!(
            usize::from(config.config_obus[1]) + 2,
            config.config_obus.len()
        );
    }

    #[test]
    fn a_stream_without_an_av1_track_has_no_record() {
        let stream = load("vp9-low").unwrap();
        assert_eq!(webm_av1_config(&stream.init).unwrap(), None);
    }

    #[test]
    fn the_control_fixture_repeats_the_sequence_header_in_its_first_key_frame() {
        // The record's OBU must be what the encoder wrote into the stream: it appears again, byte for
        // byte, inside the first media segment (every key frame's temporal unit starts with it).
        let stream = load("av1-low-av1c").unwrap();
        let config = webm_av1_config(&stream.init).unwrap().unwrap();
        let first = &stream.media[0].bytes;
        assert!(
            first
                .windows(config.config_obus.len())
                .any(|window| window == config.config_obus.as_slice()),
            "the sequence header of the record was not found in the first segment"
        );
    }
}
