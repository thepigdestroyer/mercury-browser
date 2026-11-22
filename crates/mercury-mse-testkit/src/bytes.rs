// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Checked big-endian readers. Every read that depends on the stream under test goes through these, so
//! malformed or truncated input gives a `SplitError` and never a panic or an out-of-range slice.

use crate::SplitError;

pub(crate) fn slice(data: &[u8], start: usize, end: usize) -> Result<&[u8], SplitError> {
    if start > end {
        return Err(SplitError::Malformed(format!(
            "range {start}..{end} runs backwards"
        )));
    }
    data.get(start..end).ok_or(SplitError::Truncated)
}

pub(crate) fn u8_at(data: &[u8], at: usize) -> Result<u8, SplitError> {
    data.get(at).copied().ok_or(SplitError::Truncated)
}

pub(crate) fn u16_at(data: &[u8], at: usize) -> Result<u16, SplitError> {
    let end = at.checked_add(2).ok_or(SplitError::Truncated)?;
    let b = slice(data, at, end)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

pub(crate) fn u32_at(data: &[u8], at: usize) -> Result<u32, SplitError> {
    let end = at.checked_add(4).ok_or(SplitError::Truncated)?;
    let b = slice(data, at, end)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn u64_at(data: &[u8], at: usize) -> Result<u64, SplitError> {
    let end = at.checked_add(8).ok_or(SplitError::Truncated)?;
    let b = slice(data, at, end)?;
    Ok(u64::from_be_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

/// `ticks` at `timescale` ticks per second, in whole milliseconds, rounded to the nearest.
pub(crate) fn ticks_to_ms(ticks: u64, timescale: u32) -> u64 {
    let ts = u128::from(timescale.max(1));
    ((u128::from(ticks) * 1000 + ts / 2) / ts) as u64
}

/// `ms` milliseconds in ticks at `timescale`, rounded to the nearest (negative values round the same way).
pub(crate) fn ms_to_ticks(ms: i64, timescale: u32) -> i128 {
    let ts = i128::from(timescale);
    let n = i128::from(ms) * ts;
    if n >= 0 {
        (n + 500) / 1000
    } else {
        -((-n + 500) / 1000)
    }
}
