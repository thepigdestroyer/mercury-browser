// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! EBML (the container syntax of Matroska and WebM): checked readers for element IDs, sizes and a
//! walker over the children of an element. Written from RFC 8794.

use crate::SplitError;
use crate::bytes::slice;

/// One element: where it starts, where its payload starts, where it ends (`None` for an
/// unknown-size element: only Segment and Cluster may have that).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Elem {
    pub id: u32,
    pub start: usize,
    pub data: usize,
    pub end: Option<usize>,
    /// Length in bytes of the size field (to rewrite the element with the same shape).
    pub size_len: usize,
}

pub(crate) const ID_EBML: u32 = 0x1A45_DFA3;
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
pub(crate) const ID_CODEC_ID: u32 = 0x86;
pub(crate) const ID_CODEC_PRIVATE: u32 = 0x63A2;
pub(crate) const ID_DEFAULT_DURATION: u32 = 0x23_E383;
pub(crate) const ID_VIDEO: u32 = 0xE0;
pub(crate) const ID_PIXEL_WIDTH: u32 = 0xB0;
pub(crate) const ID_PIXEL_HEIGHT: u32 = 0xBA;
pub(crate) const ID_TIMECODE: u32 = 0xE7;
pub(crate) const ID_SIMPLE_BLOCK: u32 = 0xA3;
pub(crate) const ID_BLOCK_GROUP: u32 = 0xA0;
pub(crate) const ID_BLOCK: u32 = 0xA1;
pub(crate) const ID_BLOCK_DURATION: u32 = 0x9B;

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

/// An element ID at `pos`: (id with its length marker bits, length in bytes).
pub(crate) fn read_id(d: &[u8], pos: usize) -> Result<(u32, usize), SplitError> {
    let first = *d.get(pos).ok_or(SplitError::Truncated)?;
    let len = first.leading_zeros() as usize + 1;
    if first == 0 || len > 4 {
        return Err(SplitError::Malformed(format!(
            "invalid EBML element ID at {pos}"
        )));
    }
    let b = slice(d, pos, pos + len)?;
    let mut id = 0u32;
    for &x in b {
        id = (id << 8) | u32::from(x);
    }
    Ok((id, len))
}

/// A size at `pos`: (value, or `None` when every value bit is 1 = unknown; length in bytes).
pub(crate) fn read_size(d: &[u8], pos: usize) -> Result<(Option<u64>, usize), SplitError> {
    let first = *d.get(pos).ok_or(SplitError::Truncated)?;
    if first == 0 {
        return Err(SplitError::Malformed(format!(
            "invalid EBML size at {pos} (more than 8 bytes)"
        )));
    }
    let len = first.leading_zeros() as usize + 1;
    let b = slice(d, pos, pos + len)?;
    let mut v = u64::from(b[0]) & (0xff >> len);
    for &x in &b[1..] {
        v = (v << 8) | u64::from(x);
    }
    let all_ones = (1u64 << (7 * len)) - 1;
    Ok((if v == all_ones { None } else { Some(v) }, len))
}

/// Reads the element header at `pos`; a known size must fit before `limit`.
pub(crate) fn read_elem(d: &[u8], pos: usize, limit: usize) -> Result<Elem, SplitError> {
    let limit = limit.min(d.len());
    if pos >= limit {
        return Err(SplitError::Truncated);
    }
    let (id, id_len) = read_id(d, pos)?;
    let (size, size_len) = read_size(d, pos + id_len)?;
    let data = pos + id_len + size_len;
    if data > limit {
        return Err(SplitError::Truncated);
    }
    let end = match size {
        Some(s) => {
            let end = usize::try_from(s)
                .ok()
                .and_then(|s| data.checked_add(s))
                .ok_or(SplitError::Truncated)?;
            if end > limit {
                return Err(SplitError::Truncated);
            }
            Some(end)
        }
        None => None,
    };
    Ok(Elem {
        id,
        start: pos,
        data,
        end,
        size_len,
    })
}

/// As `read_elem`, but a known size may reach beyond `d`: the header of a Segment whose data is only
/// partly there (an initialisation segment, or a stream that is still arriving).
pub(crate) fn read_elem_header(d: &[u8], pos: usize) -> Result<Elem, SplitError> {
    let (id, id_len) = read_id(d, pos)?;
    let (size, size_len) = read_size(d, pos + id_len)?;
    let data = pos + id_len + size_len;
    if data > d.len() {
        return Err(SplitError::Truncated);
    }
    let end = match size {
        Some(s) => Some(
            usize::try_from(s)
                .ok()
                .and_then(|s| data.checked_add(s))
                .ok_or(SplitError::Truncated)?,
        ),
        None => None,
    };
    Ok(Elem {
        id,
        start: pos,
        data,
        end,
        size_len,
    })
}

/// The known-size children found in `d[start..end]`, in order; an unknown-size child is an error.
pub(crate) fn children(d: &[u8], start: usize, end: usize) -> Result<Vec<Elem>, SplitError> {
    let mut out = Vec::new();
    let mut pos = start;
    while pos < end {
        let e = read_elem(d, pos, end)?;
        let e_end = e.end.ok_or_else(|| {
            SplitError::Malformed(format!("element {:#x} at {pos} has an unknown size", e.id))
        })?;
        out.push(e);
        pos = e_end;
    }
    Ok(out)
}

/// Big-endian unsigned integer payload (0 to 8 bytes).
pub(crate) fn read_uint(d: &[u8], e: &Elem) -> Result<u64, SplitError> {
    let end = e.end.ok_or(SplitError::Truncated)?;
    let b = slice(d, e.data, end)?;
    if b.len() > 8 {
        return Err(SplitError::Malformed("integer longer than 8 bytes".into()));
    }
    Ok(b.iter().fold(0u64, |acc, &x| (acc << 8) | u64::from(x)))
}

/// 4- or 8-byte big-endian float payload.
pub(crate) fn read_float(d: &[u8], e: &Elem) -> Result<f64, SplitError> {
    let end = e.end.ok_or(SplitError::Truncated)?;
    let b = slice(d, e.data, end)?;
    match b.len() {
        4 => Ok(f64::from(f32::from_be_bytes([b[0], b[1], b[2], b[3]]))),
        8 => Ok(f64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ])),
        0 => Ok(0.0),
        n => Err(SplitError::Malformed(format!("float of {n} bytes"))),
    }
}

/// Payload bytes of a known-size element.
pub(crate) fn payload<'a>(d: &'a [u8], e: &Elem) -> Result<&'a [u8], SplitError> {
    slice(d, e.data, e.end.ok_or(SplitError::Truncated)?)
}

/// A size field of `value` in at least `min_len` bytes (an all-ones value is reserved for "unknown").
pub(crate) fn write_size(value: u64, min_len: usize) -> Vec<u8> {
    let mut len = min_len.clamp(1, 8);
    while len < 8 && value >= (1u64 << (7 * len)) - 1 {
        len += 1;
    }
    let v = value | (1u64 << (7 * len));
    v.to_be_bytes()[8 - len..].to_vec()
}

/// An unsigned integer element with ID `id`, in at least `min_len` payload bytes.
pub(crate) fn write_uint_elem(id: u32, value: u64, min_len: usize) -> Vec<u8> {
    let needed = (8 - (value.leading_zeros() as usize / 8)).max(1);
    let len = needed.max(min_len).min(8);
    let mut out = Vec::new();
    let id_bytes = id.to_be_bytes();
    let skip = id_bytes.iter().take_while(|&&b| b == 0).count();
    out.extend_from_slice(&id_bytes[skip..]);
    out.extend_from_slice(&write_size(len as u64, 1));
    out.extend_from_slice(&value.to_be_bytes()[8 - len..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_round_trips() {
        for v in [
            0u64,
            1,
            126,
            127,
            16382,
            16383,
            1 << 21,
            (1 << 28) - 2,
            1 << 40,
        ] {
            for min in 1..=8 {
                let w = write_size(v, min);
                assert!(w.len() >= min.min(8));
                let (r, len) = read_size(&w, 0).unwrap();
                assert_eq!(r, Some(v), "value {v} min {min}");
                assert_eq!(len, w.len());
            }
        }
    }

    #[test]
    fn unknown_size_is_none() {
        assert_eq!(read_size(&[0xff], 0).unwrap().0, None);
        assert_eq!(
            read_size(&[0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], 0)
                .unwrap()
                .0,
            None
        );
        assert!(read_size(&[0x00], 0).is_err());
    }

    #[test]
    fn ids_have_their_marker() {
        assert_eq!(read_id(&[0x1a, 0x45, 0xdf, 0xa3], 0).unwrap(), (ID_EBML, 4));
        assert_eq!(read_id(&[0xe7], 0).unwrap(), (ID_TIMECODE, 1));
        assert!(read_id(&[0x00, 0x01], 0).is_err());
        assert!(read_id(&[0x1a, 0x45], 0).is_err());
    }

    #[test]
    fn uint_elements() {
        let e = write_uint_elem(ID_TIMECODE, 0x1234, 1);
        assert_eq!(e, vec![0xe7, 0x82, 0x12, 0x34]);
        let el = read_elem(&e, 0, e.len()).unwrap();
        assert_eq!(read_uint(&e, &el).unwrap(), 0x1234);
        assert_eq!(write_uint_elem(ID_TIMECODE, 0, 1), vec![0xe7, 0x81, 0x00]);
        assert_eq!(
            write_uint_elem(ID_TIMECODE, 5, 3),
            vec![0xe7, 0x83, 0x00, 0x00, 0x05]
        );
    }
}
