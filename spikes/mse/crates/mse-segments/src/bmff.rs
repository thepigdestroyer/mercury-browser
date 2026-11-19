// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! ISO base media file format boxes: a checked walker over a byte range. Written from ISO/IEC
//! 14496-12 (box header: 32-bit size, four-character type, optional 64-bit size when the 32-bit size is
//! 1, "to the end of the data" when it is 0, and a 16-byte user type after a `uuid` type).

use crate::SplitError;
use crate::bytes::{slice, u32_at, u64_at};

/// One box: where it starts, where its payload starts, where it ends (all offsets into the data
/// that was walked).
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoxRef {
    pub typ: [u8; 4],
    pub start: usize,
    /// First byte after the header (after the 64-bit size and the `uuid` user type, if any).
    pub payload: usize,
    pub end: usize,
}

impl BoxRef {
    pub fn is(&self, typ: &[u8; 4]) -> bool {
        &self.typ == typ
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn header_len(&self) -> usize {
        self.payload - self.start
    }

    pub fn payload_of<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        // The walker guarantees start <= payload <= end <= data.len().
        data.get(self.payload..self.end).unwrap_or(&[])
    }

    pub fn bytes_of<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        data.get(self.start..self.end).unwrap_or(&[])
    }
}

pub(crate) fn fourcc(s: &str) -> [u8; 4] {
    let b = s.as_bytes();
    [b[0], b[1], b[2], b[3]]
}

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

/// Parses the box header at `pos`; the box must end at or before `limit`.
pub(crate) fn parse_box(data: &[u8], pos: usize, limit: usize) -> Result<BoxRef, SplitError> {
    let limit = limit.min(data.len());
    if pos >= limit || limit - pos < 8 {
        return Err(SplitError::Truncated);
    }
    let size32 = u32_at(data, pos)?;
    let typ = {
        let b = slice(data, pos + 4, pos + 8)?;
        [b[0], b[1], b[2], b[3]]
    };
    let mut header = 8usize;
    let size: u64 = match size32 {
        0 => (limit - pos) as u64,
        1 => {
            if limit - pos < 16 {
                return Err(SplitError::Truncated);
            }
            header = 16;
            u64_at(data, pos + 8)?
        }
        n => u64::from(n),
    };
    if &typ == b"uuid" {
        header += 16;
    }
    if size < header as u64 {
        return Err(SplitError::Malformed(format!(
            "box '{}' at {pos} has size {size}, smaller than its {header}-byte header",
            type_str(&typ)
        )));
    }
    let end = usize::try_from(size)
        .ok()
        .and_then(|s| pos.checked_add(s))
        .ok_or(SplitError::Truncated)?;
    if end > limit {
        return Err(SplitError::Truncated);
    }
    Ok(BoxRef {
        typ,
        start: pos,
        payload: pos + header,
        end,
    })
}

/// Every box in `data[start..end]`, in order. Stops at the first error (which it yields).
pub(crate) fn boxes(data: &[u8], start: usize, end: usize) -> BoxIter<'_> {
    BoxIter {
        data,
        pos: start,
        end: end.min(data.len()),
        failed: false,
    }
}

pub(crate) struct BoxIter<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
    failed: bool,
}

impl Iterator for BoxIter<'_> {
    type Item = Result<BoxRef, SplitError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.pos >= self.end {
            return None;
        }
        match parse_box(self.data, self.pos, self.end) {
            Ok(b) => {
                // parse_box guarantees end >= start + 8, so this always advances.
                self.pos = b.end;
                Some(Ok(b))
            }
            Err(e) => {
                self.failed = true;
                Some(Err(e))
            }
        }
    }
}

/// The children of `parent` (a box found in `data`), as a vector.
pub(crate) fn children(data: &[u8], parent: &BoxRef) -> Result<Vec<BoxRef>, SplitError> {
    boxes(data, parent.payload, parent.end).collect()
}

/// The first child of `parent` with the given type.
pub(crate) fn find_child(
    data: &[u8],
    parent: &BoxRef,
    typ: &[u8; 4],
) -> Result<Option<BoxRef>, SplitError> {
    for b in boxes(data, parent.payload, parent.end) {
        let b = b?;
        if b.is(typ) {
            return Ok(Some(b));
        }
    }
    Ok(None)
}

/// Follows a path of box types from `parent` and returns the box at its end.
pub(crate) fn find_path(
    data: &[u8],
    parent: &BoxRef,
    path: &[&str],
) -> Result<Option<BoxRef>, SplitError> {
    let mut cur = *parent;
    for step in path {
        match find_child(data, &cur, &fourcc(step))? {
            Some(b) => cur = b,
            None => return Ok(None),
        }
    }
    Ok(Some(cur))
}

/// A plain box (32-bit size) around `payload`.
pub(crate) fn make_box(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
    out.extend_from_slice(typ);
    out.extend_from_slice(payload);
    out
}

/// Writes a box header of the same shape as `like` (32-bit or 64-bit size, same user type) for a box
/// whose total length is `total`, followed by nothing; the caller appends the payload.
pub(crate) fn header_like(like: &BoxRef, data: &[u8], total: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(like.header_len());
    if like.header_len() >= 16 && u32_at(data, like.start).ok() == Some(1) {
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&like.typ);
        out.extend_from_slice(&(total as u64).to_be_bytes());
    } else {
        // A "to the end" box (size 0) gets an explicit size: the rewritten box is followed by
        // nothing that the old size 0 could have covered differently.
        out.extend_from_slice(&(total as u32).to_be_bytes());
        out.extend_from_slice(&like.typ);
    }
    if &like.typ == b"uuid" {
        let ut = like.start + like.header_len() - 16;
        out.extend_from_slice(data.get(ut..ut + 16).unwrap_or(&[0; 16]));
    }
    out
}
