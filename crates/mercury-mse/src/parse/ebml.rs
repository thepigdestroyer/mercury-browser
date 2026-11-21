// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! EBML element reading (variable-length ids and sizes, unknown sizes).
//!
//! Written from RFC 8794 (EBML) and the Matroska element list, starting from the element reader of the
//! spike's `mse-segments` (Mercury's own). Every read is checked: bad input is a
//! `ParseError::Malformed`, never a panic or an out-of-range slice.

use super::{PResult, malformed};

pub(crate) const ID_EBML: u32 = 0x1A45_DFA3;
pub(crate) const ID_DOC_TYPE: u32 = 0x4282;
pub(crate) const ID_SEGMENT: u32 = 0x1853_8067;
pub(crate) const ID_SEEK_HEAD: u32 = 0x114D_9B74;
pub(crate) const ID_INFO: u32 = 0x1549_A966;
pub(crate) const ID_TRACKS: u32 = 0x1654_AE6B;
pub(crate) const ID_CLUSTER: u32 = 0x1F43_B675;
pub(crate) const ID_CUES: u32 = 0x1C53_BB6B;
pub(crate) const ID_TAGS: u32 = 0x1254_C367;
pub(crate) const ID_CHAPTERS: u32 = 0x1043_A770;
pub(crate) const ID_ATTACHMENTS: u32 = 0x1941_A469;
pub(crate) const ID_TIMECODE_SCALE: u32 = 0x2A_D7B1;
pub(crate) const ID_DURATION: u32 = 0x4489;
pub(crate) const ID_TRACK_ENTRY: u32 = 0xAE;
pub(crate) const ID_TRACK_NUMBER: u32 = 0xD7;
pub(crate) const ID_TRACK_TYPE: u32 = 0x83;
pub(crate) const ID_FLAG_ENABLED: u32 = 0xB9;
pub(crate) const ID_FLAG_DEFAULT: u32 = 0x88;
pub(crate) const ID_CODEC_ID: u32 = 0x86;
pub(crate) const ID_CODEC_PRIVATE: u32 = 0x63A2;
pub(crate) const ID_DEFAULT_DURATION: u32 = 0x23_E383;
pub(crate) const ID_LANGUAGE: u32 = 0x22_B59C;
pub(crate) const ID_VIDEO: u32 = 0xE0;
pub(crate) const ID_PIXEL_WIDTH: u32 = 0xB0;
pub(crate) const ID_PIXEL_HEIGHT: u32 = 0xBA;
pub(crate) const ID_AUDIO: u32 = 0xE1;
pub(crate) const ID_SAMPLING_FREQUENCY: u32 = 0xB5;
pub(crate) const ID_CHANNELS: u32 = 0x9F;
pub(crate) const ID_TIMECODE: u32 = 0xE7;
pub(crate) const ID_SIMPLE_BLOCK: u32 = 0xA3;
pub(crate) const ID_BLOCK_GROUP: u32 = 0xA0;
pub(crate) const ID_BLOCK: u32 = 0xA1;
pub(crate) const ID_BLOCK_DURATION: u32 = 0x9B;
pub(crate) const ID_REFERENCE_BLOCK: u32 = 0xFB;

/// The IDs that can only be children of the Segment: they end an unknown-size Cluster.
pub(crate) fn is_level1(id: u32) -> bool {
    matches!(
        id,
        ID_SEEK_HEAD
            | ID_INFO
            | ID_TRACKS
            | ID_CLUSTER
            | ID_CUES
            | ID_TAGS
            | ID_CHAPTERS
            | ID_ATTACHMENTS
    )
}

/// An element header: its ID (with the length marker bits, as the Matroska tables write it), its size
/// (`None` when every value bit is 1: "unknown", which only a Segment or a Cluster may use) and the
/// lengths of the two fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ElemHeader {
    pub id: u32,
    pub id_len: usize,
    pub size: Option<u64>,
    pub size_len: usize,
}

impl ElemHeader {
    pub fn header_len(&self) -> usize {
        self.id_len + self.size_len
    }
}

/// The header at the start of `d`; `None` when more bytes are needed to read it. An ID or size whose
/// first byte is 0 (longer than the 4 and 8 bytes EBML allows) is an error at once.
pub(crate) fn peek_header(d: &[u8]) -> PResult<Option<ElemHeader>> {
    let Some(&first) = d.first() else {
        return Ok(None);
    };
    let id_len = first.leading_zeros() as usize + 1;
    if first == 0 || id_len > 4 {
        return Err(malformed("an invalid EBML element ID"));
    }
    let Some(id_bytes) = d.get(..id_len) else {
        return Ok(None);
    };
    let id = id_bytes.iter().fold(0u32, |a, &b| (a << 8) | u32::from(b));
    let Some(&size_first) = d.get(id_len) else {
        return Ok(None);
    };
    if size_first == 0 {
        return Err(malformed("an EBML size of more than 8 bytes"));
    }
    let size_len = size_first.leading_zeros() as usize + 1;
    let Some(size_bytes) = d.get(id_len..id_len + size_len) else {
        return Ok(None);
    };
    let (value, all_ones) = vint_value(size_bytes);
    Ok(Some(ElemHeader {
        id,
        id_len,
        size: if all_ones { None } else { Some(value) },
        size_len,
    }))
}

/// The value of a variable-length integer whose bytes are all there (1 to 8), and whether every value
/// bit is 1.
fn vint_value(b: &[u8]) -> (u64, bool) {
    let len = b.len();
    let mask = if len >= 8 { 0 } else { 0xffu8 >> len };
    let mut v = u64::from(b[0] & mask);
    for &x in &b[1..] {
        v = (v << 8) | u64::from(x);
    }
    let ones = (1u64 << (7 * len)) - 1;
    (v, v == ones)
}

/// A variable-length number at the start of `d` (a Block's track number): (value, length). The reserved
/// all-ones value is refused.
pub(crate) fn read_vint(d: &[u8]) -> PResult<(u64, usize)> {
    let first = *d
        .first()
        .ok_or_else(|| malformed("a block ends before its track number"))?;
    if first == 0 {
        return Err(malformed("a track number longer than 8 bytes"));
    }
    let len = first.leading_zeros() as usize + 1;
    let b = d
        .get(..len)
        .ok_or_else(|| malformed("a block ends inside its track number"))?;
    let (v, ones) = vint_value(b);
    if ones {
        return Err(malformed("a reserved track number"));
    }
    Ok((v, len))
}

/// A child element found inside a payload that is in memory.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Elem<'a> {
    pub id: u32,
    pub payload: &'a [u8],
}

/// The child elements of `d` in order, each with a known size that fits in `d`.
pub(crate) fn elements(d: &[u8]) -> ElemIter<'_> {
    ElemIter { d, failed: false }
}

pub(crate) struct ElemIter<'a> {
    d: &'a [u8],
    failed: bool,
}

impl<'a> Iterator for ElemIter<'a> {
    type Item = PResult<Elem<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.d.is_empty() {
            return None;
        }
        let fail = |this: &mut Self, msg: &str| {
            this.failed = true;
            Some(Err(malformed(msg.to_string())))
        };
        let h = match peek_header(self.d) {
            Ok(Some(h)) => h,
            Ok(None) => return fail(self, "an element header is cut short"),
            Err(e) => {
                self.failed = true;
                return Some(Err(e));
            }
        };
        let Some(size) = h.size else {
            return fail(
                self,
                "an element with an unknown size inside a sized element",
            );
        };
        let start = h.header_len();
        let Some(end) = usize::try_from(size)
            .ok()
            .and_then(|s| start.checked_add(s))
            .filter(|&e| e <= self.d.len())
        else {
            return fail(self, "an element runs past the end of its parent");
        };
        let payload = &self.d[start..end];
        self.d = &self.d[end..];
        Some(Ok(Elem { id: h.id, payload }))
    }
}

/// A big-endian unsigned integer payload (0 to 8 bytes).
pub(crate) fn read_uint(p: &[u8]) -> PResult<u64> {
    if p.len() > 8 {
        return Err(malformed("an integer element longer than 8 bytes"));
    }
    Ok(p.iter().fold(0u64, |a, &b| (a << 8) | u64::from(b)))
}

/// A 4- or 8-byte big-endian float payload (an empty one is 0).
pub(crate) fn read_float(p: &[u8]) -> PResult<f64> {
    match p.len() {
        0 => Ok(0.0),
        4 => Ok(f64::from(f32::from_be_bytes([p[0], p[1], p[2], p[3]]))),
        8 => Ok(f64::from_be_bytes([
            p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7],
        ])),
        n => Err(malformed(format!("a float element of {n} bytes"))),
    }
}

/// A string payload; muxers pad with NUL bytes, which are dropped.
pub(crate) fn read_string(p: &[u8]) -> String {
    String::from_utf8_lossy(p)
        .trim_end_matches('\0')
        .to_string()
}

/// The fixed part of a `SimpleBlock` or `Block`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlockHeader {
    pub track: u64,
    /// Time relative to the Cluster timecode, in timecode units.
    pub timecode: i16,
    pub flags: u8,
    /// Where the frame data starts in the block payload.
    pub data_start: usize,
}

impl BlockHeader {
    pub fn keyframe(&self) -> bool {
        self.flags & 0x80 != 0
    }

    /// 0 none, 1 Xiph, 2 fixed-size, 3 EBML.
    pub fn lacing(&self) -> u8 {
        (self.flags >> 1) & 0x03
    }
}

pub(crate) fn parse_block_header(p: &[u8]) -> PResult<BlockHeader> {
    let (track, len) = read_vint(p)?;
    let rest = p
        .get(len..len + 3)
        .ok_or_else(|| malformed("a block ends inside its header"))?;
    Ok(BlockHeader {
        track,
        timecode: i16::from_be_bytes([rest[0], rest[1]]),
        flags: rest[2],
        data_start: len + 3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_read_ids_and_sizes_of_every_length() {
        // EBML header: 4-byte ID, 8-byte size 0x10.
        let d = [0x1a, 0x45, 0xdf, 0xa3, 0x01, 0, 0, 0, 0, 0, 0, 0x10];
        let h = peek_header(&d).unwrap().unwrap();
        assert_eq!(
            (h.id, h.id_len, h.size, h.size_len),
            (ID_EBML, 4, Some(16), 8)
        );
        assert_eq!(h.header_len(), 12);
        // A one-byte ID and size: Timecode (0xE7) of 2 bytes.
        let h = peek_header(&[0xe7, 0x82]).unwrap().unwrap();
        assert_eq!((h.id, h.size), (ID_TIMECODE, Some(2)));
        // Unknown sizes, of every length.
        assert_eq!(peek_header(&[0xa3, 0xff]).unwrap().unwrap().size, None);
        assert_eq!(
            peek_header(&[
                0x1f, 0x43, 0xb6, 0x75, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff
            ])
            .unwrap()
            .unwrap()
            .size,
            None
        );
        // Needs more bytes, at every cut.
        for cut in 0..d.len() {
            assert_eq!(peek_header(&d[..cut]).unwrap(), None, "cut {cut}");
        }
        // Invalid first bytes are errors at once.
        assert!(peek_header(&[0x00]).is_err());
        assert!(peek_header(&[0x0f, 0, 0, 0, 0]).is_err());
        assert!(peek_header(&[0xe7, 0x00]).is_err());
    }

    #[test]
    fn children_are_bounded_by_their_parent() {
        let d = [0xe7, 0x82, 0x12, 0x34, 0xa3, 0x81, 0x77];
        let all: Vec<_> = elements(&d).collect::<Result<_, _>>().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            (all[0].id, all[0].payload),
            (ID_TIMECODE, &[0x12, 0x34][..])
        );
        assert_eq!((all[1].id, all[1].payload), (ID_SIMPLE_BLOCK, &[0x77][..]));
        // A child that claims more than its parent holds.
        assert!(elements(&[0xe7, 0x85, 1]).next().unwrap().is_err());
        // An unknown-size child inside a sized element.
        assert!(elements(&[0xa3, 0xff, 1]).next().unwrap().is_err());
        // A size that does not fit in usize arithmetic.
        let huge = [0xe7, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe];
        assert!(elements(&huge).next().unwrap().is_err());
    }

    #[test]
    fn values() {
        assert_eq!(read_uint(&[]).unwrap(), 0);
        assert_eq!(read_uint(&[1, 0]).unwrap(), 256);
        assert!(read_uint(&[0; 9]).is_err());
        assert_eq!(read_float(&[0x40, 0x59, 0, 0, 0, 0, 0, 0]).unwrap(), 100.0);
        assert_eq!(read_float(&[0x42, 0xc8, 0, 0]).unwrap(), 100.0);
        assert!(read_float(&[1, 2, 3]).is_err());
        assert_eq!(read_string(b"V_VP9\0"), "V_VP9");
    }

    #[test]
    fn block_headers() {
        // Track 1, relative timecode -2, keyframe flag.
        let b = parse_block_header(&[0x81, 0xff, 0xfe, 0x80, 9, 9]).unwrap();
        assert_eq!((b.track, b.timecode, b.data_start), (1, -2, 4));
        assert!(b.keyframe());
        assert_eq!(b.lacing(), 0);
        // A two-byte track number.
        let b = parse_block_header(&[0x40, 0x02, 0, 5, 0x06]).unwrap();
        assert_eq!(
            (b.track, b.timecode, b.data_start, b.lacing()),
            (2, 5, 5, 3)
        );
        assert!(parse_block_header(&[0x81, 0, 0]).is_err());
        assert!(parse_block_header(&[]).is_err());
        assert!(parse_block_header(&[0x00, 1, 2, 3]).is_err());
        assert!(parse_block_header(&[0xff, 0, 0, 0]).is_err());
    }
}
