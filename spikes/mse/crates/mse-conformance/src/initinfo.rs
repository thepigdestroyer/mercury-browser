// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What an initialisation segment says about the duration of the stream (`mvhd` and `mehd` of an MP4,
//! the Info `Duration` of a WebM).
//!
//! C02 checks that the element's duration is the fixture's 8 s after the init segments. That is a spec
//! rule only when the init segment itself carries a duration: a fragmented MP4 written for streaming has
//! `mvhd` duration 0 and no `mehd`, and a streamable WebM has no `Duration`, and then "unknown until media
//! arrives" is exactly what the spec says. This module tells the scenario which of the two cases it is in,
//! so that the check is applied where the spec requires it and the other case is recorded, not guessed.

use std::time::Duration;

use mse_segments::Container;

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn be64(data: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(data.get(at..at + 8)?.try_into().ok()?))
}

/// The boxes of `data[start..end]`: `(type, payload start, box end)`. Stops at the first malformed box.
fn boxes(data: &[u8], start: usize, end: usize) -> Vec<([u8; 4], usize, usize)> {
    let mut found = Vec::new();
    let mut pos = start;
    while pos + 8 <= end {
        let Some(size32) = be32(data, pos) else { break };
        let Some(kind) = data
            .get(pos + 4..pos + 8)
            .and_then(|s| <[u8; 4]>::try_from(s).ok())
        else {
            break;
        };
        let (header, size) = match size32 {
            0 => (8usize, end - pos),
            1 => match be64(data, pos + 8).and_then(|s| usize::try_from(s).ok()) {
                Some(size) => (16usize, size),
                None => break,
            },
            n => (8usize, n as usize),
        };
        if size < header || pos + size > end {
            break;
        }
        found.push((kind, pos + header, pos + size));
        pos += size;
    }
    found
}

/// `mvhd` duration and `mehd` fragment duration of an MP4 initialisation segment, the larger known one.
fn mp4_declared(init: &[u8]) -> Option<Duration> {
    let top = boxes(init, 0, init.len());
    let (_, moov_start, moov_end) = top.into_iter().find(|(kind, _, _)| kind == b"moov")?;
    let children = boxes(init, moov_start, moov_end);
    let mut timescale = 0u32;
    let mut best_ticks = 0u64;
    for (kind, start, _end) in &children {
        if kind == b"mvhd" {
            let version = *init.get(*start)?;
            let (scale_at, duration) = if version == 1 {
                (start + 20, be64(init, start + 24)?)
            } else {
                (start + 12, u64::from(be32(init, start + 16)?))
            };
            timescale = be32(init, scale_at)?;
            if duration != 0 && duration != u64::from(u32::MAX) && duration != u64::MAX {
                best_ticks = best_ticks.max(duration);
            }
        }
    }
    if timescale == 0 {
        return None;
    }
    for (kind, start, end) in &children {
        if kind == b"mvex" {
            for (inner, inner_start, _) in boxes(init, *start, *end) {
                if &inner == b"mehd" {
                    let version = *init.get(inner_start)?;
                    let ticks = if version == 1 {
                        be64(init, inner_start + 4)?
                    } else {
                        u64::from(be32(init, inner_start + 4)?)
                    };
                    if ticks != 0 && ticks != u64::from(u32::MAX) && ticks != u64::MAX {
                        best_ticks = best_ticks.max(ticks);
                    }
                }
            }
        }
    }
    if best_ticks == 0 {
        return None;
    }
    Some(Duration::from_secs_f64(
        best_ticks as f64 / f64::from(timescale),
    ))
}

/// The value of an EBML element whose id bytes are `id`: the first occurrence in `data`, size as a
/// 1-byte VINT up to 8 bytes. A scan, not a parse: the Info element is near the start of a small init
/// segment, and `declared_duration` only trusts a result close to the fixture's nominal length.
fn ebml_payload<'a>(data: &'a [u8], id: &[u8]) -> Option<&'a [u8]> {
    let mut at = 0;
    while at + id.len() < data.len() {
        if data[at..].starts_with(id) {
            let marker = *data.get(at + id.len())?;
            let length = marker.leading_zeros() as usize + 1;
            if (1..=8).contains(&length) {
                let mut size = u64::from(marker) & ((1u64 << (8 - length)) - 1);
                let size_end = at + id.len() + length;
                for byte in data.get(at + id.len() + 1..size_end)? {
                    size = (size << 8) | u64::from(*byte);
                }
                let payload_end = size_end.checked_add(usize::try_from(size).ok()?)?;
                if payload_end <= data.len() && size > 0 && size <= 8 {
                    return data.get(size_end..payload_end);
                }
            }
        }
        at += 1;
    }
    None
}

fn webm_declared(init: &[u8]) -> Option<Duration> {
    let scale_ns = ebml_payload(init, &[0x2A, 0xD7, 0xB1])
        .map(|bytes| bytes.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b)))
        .unwrap_or(1_000_000);
    let payload = ebml_payload(init, &[0x44, 0x89])?;
    let ticks = match payload.len() {
        4 => f64::from(f32::from_be_bytes(payload.try_into().ok()?)),
        8 => f64::from_be_bytes(payload.try_into().ok()?),
        _ => return None,
    };
    if !ticks.is_finite() || ticks <= 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(ticks * scale_ns as f64 / 1e9))
}

/// The duration the initialisation segment declares, if any. `None` means "the init segment does not
/// say" (duration 0 or absent): the stream's duration is then unknown until media segments arrive.
pub fn declared_duration(container: Container, init: &[u8]) -> Option<Duration> {
    match container {
        Container::Mp4 => mp4_declared(init),
        Container::WebM => webm_declared(init),
    }
}

/// Whether the declared duration is the fixture's nominal one (within 50 ms). A scan that finds some
/// other number is treated as "not declared" rather than trusted.
pub fn declares_nominal(container: Container, init: &[u8], nominal: Duration) -> bool {
    declared_duration(container, init)
        .is_some_and(|declared| declared.abs_diff(nominal) <= Duration::from_millis(50))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    fn mvhd_v0(timescale: u32, duration: u32) -> Vec<u8> {
        let mut payload = vec![0, 0, 0, 0]; // version 0 + flags
        payload.extend_from_slice(&[0; 8]); // creation, modification
        payload.extend_from_slice(&timescale.to_be_bytes());
        payload.extend_from_slice(&duration.to_be_bytes());
        payload.extend_from_slice(&[0; 80]); // rate, volume, matrix, ...
        make_box(b"mvhd", &payload)
    }

    fn mehd_v1(ticks: u64) -> Vec<u8> {
        let mut payload = vec![1, 0, 0, 0];
        payload.extend_from_slice(&ticks.to_be_bytes());
        make_box(b"mehd", &payload)
    }

    fn init_with(moov_children: &[Vec<u8>]) -> Vec<u8> {
        let mut out = make_box(b"ftyp", b"iso5\0\0\0\0iso5");
        out.extend(make_box(b"moov", &moov_children.concat()));
        out
    }

    #[test]
    fn mp4_mvhd_duration_is_read_and_zero_means_undeclared() {
        let declared = init_with(&[mvhd_v0(1000, 8000)]);
        assert_eq!(
            declared_duration(Container::Mp4, &declared),
            Some(Duration::from_secs(8))
        );
        assert!(declares_nominal(
            Container::Mp4,
            &declared,
            Duration::from_secs(8)
        ));
        assert!(!declares_nominal(
            Container::Mp4,
            &declared,
            Duration::from_secs(9)
        ));

        let streaming = init_with(&[mvhd_v0(1000, 0)]);
        assert_eq!(declared_duration(Container::Mp4, &streaming), None);
        let unknown = init_with(&[mvhd_v0(1000, u32::MAX)]);
        assert_eq!(declared_duration(Container::Mp4, &unknown), None);
    }

    #[test]
    fn mp4_mehd_fragment_duration_counts_in_movie_timescale() {
        let mvex = make_box(b"mvex", &mehd_v1(16_000));
        let init = init_with(&[mvhd_v0(2000, 0), mvex]);
        assert_eq!(
            declared_duration(Container::Mp4, &init),
            Some(Duration::from_secs(8))
        );
    }

    #[test]
    fn mp4_without_moov_or_with_garbage_is_undeclared() {
        assert_eq!(declared_duration(Container::Mp4, &[]), None);
        assert_eq!(
            declared_duration(Container::Mp4, &make_box(b"ftyp", b"iso5")),
            None
        );
        assert_eq!(declared_duration(Container::Mp4, &[0xFF; 40]), None);
        // A box that claims to be larger than the data.
        let mut bad = init_with(&[mvhd_v0(1000, 8000)]);
        bad.truncate(bad.len() - 30);
        assert_eq!(declared_duration(Container::Mp4, &bad), None);
    }

    #[test]
    fn webm_duration_uses_the_timecode_scale() {
        // TimecodeScale 1_000_000 (3 bytes), Duration float32 8000.0 (ms).
        let mut info = vec![0x2A, 0xD7, 0xB1, 0x83, 0x0F, 0x42, 0x40];
        info.extend_from_slice(&[0x44, 0x89, 0x84]);
        info.extend_from_slice(&8000.0f32.to_be_bytes());
        assert_eq!(
            declared_duration(Container::WebM, &info),
            Some(Duration::from_secs(8))
        );

        // Default scale, float64 Duration.
        let mut bare = vec![0x44, 0x89, 0x88];
        bare.extend_from_slice(&8000.0f64.to_be_bytes());
        assert_eq!(
            declared_duration(Container::WebM, &bare),
            Some(Duration::from_secs(8))
        );
        assert!(declares_nominal(
            Container::WebM,
            &bare,
            Duration::from_secs(8)
        ));

        assert_eq!(
            declared_duration(Container::WebM, &[0x1A, 0x45, 0xDF, 0xA3]),
            None
        );
        assert_eq!(declared_duration(Container::WebM, &[]), None);
        let mut zero = vec![0x44, 0x89, 0x84];
        zero.extend_from_slice(&0.0f32.to_be_bytes());
        assert_eq!(declared_duration(Container::WebM, &zero), None);
    }
}
