// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! ISO BMFF box reading (sizes, 64-bit sizes, nesting, bounded reads).
//!
//! Written from ISO/IEC 14496-12 (box header: 32-bit size, four-character type, a 64-bit size after the
//! type when the 32-bit size is 1, "to the end of the data" when it is 0), starting from the box walker
//! of the spike's `mse-segments` (Mercury's own). Every read is checked: a short or inconsistent box is a
//! `ParseError::Malformed`, never a panic or an out-of-range slice.

use super::{PResult, malformed};

/// A checked big-endian cursor over a byte slice.
pub(crate) struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(d: &'a [u8]) -> Self {
        Reader { d, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.d.len() - self.pos
    }

    /// Everything not read yet.
    pub fn rest(&self) -> &'a [u8] {
        self.d.get(self.pos..).unwrap_or(&[])
    }

    pub fn take(&mut self, n: usize) -> PResult<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| malformed("a length overflows"))?;
        let s = self
            .d
            .get(self.pos..end)
            .ok_or_else(|| malformed("a box or descriptor is shorter than its fields"))?;
        self.pos = end;
        Ok(s)
    }

    pub fn skip(&mut self, n: usize) -> PResult<()> {
        self.take(n).map(|_| ())
    }

    pub fn u8(&mut self) -> PResult<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> PResult<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> PResult<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> PResult<i32> {
        self.u32().map(|v| v as i32)
    }

    pub fn u64(&mut self) -> PResult<u64> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
}

/// The header of a box: its type, how long the header is (8, or 16 with a 64-bit size) and its total
/// size including the header (`None` for "to the end of the data", which only the last box may use).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoxHeader {
    pub typ: [u8; 4],
    pub header_len: usize,
    pub size: Option<u64>,
}

/// The header at the start of `d`; `None` when more bytes are needed to read it; an error when the size
/// is smaller than the header.
pub(crate) fn peek_header(d: &[u8]) -> PResult<Option<BoxHeader>> {
    if d.len() < 8 {
        return Ok(None);
    }
    let size32 = u32::from_be_bytes([d[0], d[1], d[2], d[3]]);
    let typ = [d[4], d[5], d[6], d[7]];
    let (header_len, size) = match size32 {
        0 => (8usize, None),
        1 => {
            if d.len() < 16 {
                return Ok(None);
            }
            let s = u64::from_be_bytes([d[8], d[9], d[10], d[11], d[12], d[13], d[14], d[15]]);
            (16usize, Some(s))
        }
        n => (8usize, Some(u64::from(n))),
    };
    if let Some(s) = size
        && s < header_len as u64
    {
        return Err(malformed(format!(
            "box '{}' declares {s} bytes, fewer than its {header_len}-byte header",
            type_str(&typ)
        )));
    }
    Ok(Some(BoxHeader {
        typ,
        header_len,
        size,
    }))
}

/// A box found inside an in-memory payload.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BoxRef<'a> {
    pub typ: [u8; 4],
    pub header_len: usize,
    /// The whole box, header included.
    pub whole: &'a [u8],
}

impl<'a> BoxRef<'a> {
    pub fn is(&self, typ: &[u8; 4]) -> bool {
        &self.typ == typ
    }

    /// What follows the header.
    pub fn payload(&self) -> &'a [u8] {
        self.whole.get(self.header_len..).unwrap_or(&[])
    }
}

/// The boxes of `d` in order; stops at the first one that is malformed or runs past `d`.
pub(crate) fn boxes(d: &[u8]) -> BoxIter<'_> {
    BoxIter { d, failed: false }
}

pub(crate) struct BoxIter<'a> {
    d: &'a [u8],
    failed: bool,
}

impl<'a> Iterator for BoxIter<'a> {
    type Item = PResult<BoxRef<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.d.is_empty() {
            return None;
        }
        let header = match peek_header(self.d) {
            Ok(Some(h)) => h,
            Ok(None) => {
                self.failed = true;
                return Some(Err(malformed("a box header is cut short")));
            }
            Err(e) => {
                self.failed = true;
                return Some(Err(e));
            }
        };
        let total = match header.size {
            None => self.d.len() as u64,
            Some(s) => s,
        };
        if total > self.d.len() as u64 {
            self.failed = true;
            return Some(Err(malformed(format!(
                "box '{}' of {total} bytes runs past the end of its parent",
                type_str(&header.typ)
            ))));
        }
        let (whole, rest) = self.d.split_at(total as usize);
        self.d = rest;
        Some(Ok(BoxRef {
            typ: header.typ,
            header_len: header.header_len,
            whole,
        }))
    }
}

/// The first box of type `typ` in `d`.
pub(crate) fn find<'a>(d: &'a [u8], typ: &[u8; 4]) -> PResult<Option<BoxRef<'a>>> {
    for b in boxes(d) {
        let b = b?;
        if b.is(typ) {
            return Ok(Some(b));
        }
    }
    Ok(None)
}

/// Follows a path of box types from `d`; `None` when a step is missing.
pub(crate) fn find_path<'a>(d: &'a [u8], path: &[&[u8; 4]]) -> PResult<Option<BoxRef<'a>>> {
    let mut cur = d;
    let mut found = None;
    for step in path {
        match find(cur, step)? {
            Some(b) => {
                cur = b.payload();
                found = Some(b);
            }
            None => return Ok(None),
        }
    }
    Ok(found)
}

/// The version, the 24 flag bits and the rest of a full box's payload.
pub(crate) fn full_box(payload: &[u8]) -> PResult<(u8, u32, &[u8])> {
    let head = payload
        .get(..4)
        .ok_or_else(|| malformed("a full box is shorter than its version and flags"))?;
    let version = head[0];
    let flags = u32::from_be_bytes([0, head[1], head[2], head[3]]);
    Ok((version, flags, &payload[4..]))
}

/// A box type for messages: printable bytes as they are, others as `?`.
pub(crate) fn type_str(typ: &[u8; 4]) -> String {
    typ.iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '?'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ParseError;

    fn is_malformed(e: &ParseError) -> bool {
        matches!(e, ParseError::Malformed(_))
    }

    fn make(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((8 + payload.len()) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(typ);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn headers_need_their_bytes_and_their_sizes_make_sense() {
        assert_eq!(peek_header(&[0, 0, 0]).unwrap(), None);
        let h = peek_header(&[0, 0, 0, 8, b'f', b'r', b'e', b'e'])
            .unwrap()
            .unwrap();
        assert_eq!((h.typ, h.header_len, h.size), (*b"free", 8, Some(8)));
        // 64-bit size: needs 16 bytes.
        let mut big = vec![0, 0, 0, 1, b'm', b'd', b'a', b't'];
        assert_eq!(peek_header(&big).unwrap(), None);
        big.extend_from_slice(&40u64.to_be_bytes());
        let h = peek_header(&big).unwrap().unwrap();
        assert_eq!((h.header_len, h.size), (16, Some(40)));
        // A size smaller than the header is malformed.
        assert!(is_malformed(
            &peek_header(&[0, 0, 0, 4, b'f', b'r', b'e', b'e']).unwrap_err()
        ));
        let mut tiny = vec![0, 0, 0, 1, b'm', b'd', b'a', b't'];
        tiny.extend_from_slice(&15u64.to_be_bytes());
        assert!(peek_header(&tiny).is_err());
        // Size 0 means "to the end".
        let h = peek_header(&[0, 0, 0, 0, b'm', b'd', b'a', b't'])
            .unwrap()
            .unwrap();
        assert_eq!(h.size, None);
    }

    #[test]
    fn nested_boxes_are_walked_and_bounded() {
        let inner = make(b"tfhd", &[1, 2, 3]);
        let mut parent = make(b"free", &[0; 5]);
        parent.extend_from_slice(&inner);
        let all: Vec<_> = boxes(&parent).collect::<Result<_, _>>().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[1].is(b"tfhd"));
        assert_eq!(all[1].payload(), &[1, 2, 3]);
        assert!(find(&parent, b"tfhd").unwrap().is_some());
        assert!(find(&parent, b"trun").unwrap().is_none());
        // A child that claims more than its parent holds is an error, not a slice panic.
        let mut bad = make(b"free", &[0; 5]);
        bad[3] = 200;
        assert!(boxes(&bad).next().unwrap().is_err());
        // A cut header is an error too.
        assert!(boxes(&[0, 0, 0, 12, b'a']).next().unwrap().is_err());
        // find_path follows nesting.
        let moov = make(b"moov", &make(b"trak", &make(b"tkhd", &[7])));
        let tkhd = find_path(&moov, &[b"moov", b"trak", b"tkhd"])
            .unwrap()
            .unwrap();
        assert_eq!(tkhd.payload(), &[7]);
        assert!(
            find_path(&moov, &[b"moov", b"mvex"]).unwrap().is_none(),
            "a missing step is None"
        );
    }

    #[test]
    fn reader_refuses_short_data() {
        let mut r = Reader::new(&[1, 2, 3, 4, 5]);
        assert_eq!(r.u16().unwrap(), 0x0102);
        assert_eq!(r.u8().unwrap(), 3);
        assert!(r.u32().is_err());
        assert_eq!(r.remaining(), 2);
        assert_eq!(r.rest(), &[4, 5]);
        assert!(r.take(usize::MAX).is_err());
        let (v, f, rest) = full_box(&[1, 0, 0, 5, 9]).unwrap();
        assert_eq!((v, f, rest), (1, 5, &[9u8][..]));
        assert!(full_box(&[1, 2]).is_err());
    }
}
