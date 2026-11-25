// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Fragmented MP4: splitting into an initialisation segment and media segments, timing of each
//! segment, shifting media time (`tfdt`), and adding a `pssh` box.
//!
//! Segmentation rules (the "ISO BMFF byte stream format" of the MSE registry, simplified):
//!
//! * The initialisation segment is every top-level box before the first `styp`, `sidx`, `ssix`,
//!   `prft`, `emsg`, `moof` or `mdat` box (normally `ftyp` + `moov`, perhaps `free`).
//! * A media segment is `[styp] [sidx] [prft] [emsg] moof mdat`. A `styp` always starts a new
//!   segment. Without a `sidx`, **each `moof` + `mdat` pair is its own segment**. With a `sidx`,
//!   **the fragments it indexes (its `first_offset` plus the sum of its `referenced_size` fields,
//!   counted from the end of the `sidx`) are one segment**: that is what a DASH segment with a
//!   subsegment index is.
//! * Boxes between segments that belong to none (`free`, `skip`, `mfra`, unknown) are dropped.
//! * A `moov` after media data (an initialisation segment in the middle of a stream) is
//!   `Unsupported`; an `mdat` before any `moof`, or a file without `moof` and without `mvex`, is
//!   `NotFragmented`.

use std::collections::HashMap;

use crate::bmff::{BoxRef, boxes, children, header_like, make_box};
use crate::bytes::{ms_to_ticks, slice, ticks_to_ms, u8_at, u16_at, u32_at, u64_at};
use crate::mp4_init::{InitInfo, mime_of, parse_init};
use crate::{Container, Segment, SegmentKind, SegmentedStream, SplitError};

/// Boxes that start the leading part of a media segment.
fn is_leading(typ: &[u8; 4]) -> bool {
    matches!(typ, b"styp" | b"sidx" | b"ssix" | b"prft" | b"emsg")
}

/// Where the segment grouped by a `sidx` ends (clamped to the data).
fn sidx_group_end(data: &[u8], sidx: &BoxRef) -> Result<usize, SplitError> {
    let ix = parse_sidx(data, sidx)?;
    let mut total = ix.first_offset;
    for r in &ix.refs {
        total = total.saturating_add(u64::from(r.size));
    }
    let end = (sidx.end as u64).saturating_add(total);
    Ok(usize::try_from(end).unwrap_or(usize::MAX).min(data.len()))
}

#[derive(Debug, Clone, Copy)]
struct SidxRef {
    /// Offset of the size field relative to the start of the `sidx` box.
    field_at: usize,
    size: u32,
    duration: u32,
}

/// A parsed `sidx` box.
struct Sidx {
    refs: Vec<SidxRef>,
    first_offset: u64,
    timescale: u32,
    /// Offset of the earliest_presentation_time field relative to the start of the box.
    ept_at: usize,
    /// Its length in bytes: 4 (version 0) or 8 (version 1).
    ept_len: usize,
}

fn parse_sidx(data: &[u8], sidx: &BoxRef) -> Result<Sidx, SplitError> {
    let p = sidx.payload_of(data);
    let version = u8_at(p, 0)?;
    let timescale = u32_at(p, 8)?;
    let (first_offset, ept_len, refs_at, count_at) = if version == 0 {
        (u64::from(u32_at(p, 16)?), 4, 24, 22)
    } else {
        (u64_at(p, 20)?, 8, 32, 30)
    };
    let count = usize::from(u16_at(p, count_at)?);
    if p.len() < refs_at + count * 12 {
        return Err(SplitError::Truncated);
    }
    let mut refs = Vec::with_capacity(count);
    for i in 0..count {
        let at = refs_at + i * 12;
        refs.push(SidxRef {
            field_at: sidx.header_len() + at,
            size: u32_at(p, at)? & 0x7fff_ffff,
            duration: u32_at(p, at + 4)?,
        });
    }
    Ok(Sidx {
        refs,
        first_offset,
        timescale,
        ept_at: sidx.header_len() + 12,
        ept_len,
    })
}

/// Splits fragmented MP4 `data`; `mime_hint` is used only when the codec cannot be derived.
pub fn split_mp4(data: &[u8], mime_hint: &str) -> Result<SegmentedStream, SplitError> {
    let tops: Vec<BoxRef> = boxes(data, 0, data.len()).collect::<Result<_, _>>()?;
    if tops.is_empty() {
        return Err(SplitError::Truncated);
    }
    let first_media = tops
        .iter()
        .position(|b| is_leading(&b.typ) || b.is(b"moof") || b.is(b"mdat") || b.is(b"mfra"));
    let init_boxes = first_media.unwrap_or(tops.len());
    if !tops[..init_boxes].iter().any(|b| b.is(b"moov")) {
        return Err(match first_media.map(|i| &tops[i]) {
            Some(b) if b.is(b"mdat") => SplitError::NotFragmented(
                "an mdat box comes before any moov or moof box".to_string(),
            ),
            _ => SplitError::Malformed("no moov box before the first media box".to_string()),
        });
    }
    let init_end = if init_boxes == tops.len() {
        data.len()
    } else {
        tops[init_boxes].start
    };
    let init_bytes = &data[..init_end];
    let info = parse_init(init_bytes)?;

    // Group the remaining top-level boxes into segments.
    struct Open {
        start: usize,
        end: usize,
        has_moof: bool,
        has_mdat: bool,
        group_end: usize,
    }
    fn flush(cur: &mut Option<Open>, ranges: &mut Vec<(usize, usize)>) -> Result<(), SplitError> {
        if let Some(c) = cur.take() {
            if !c.has_moof {
                return Err(SplitError::Truncated);
            }
            if !c.has_mdat {
                return Err(SplitError::Malformed(
                    "a moof box is not followed by an mdat box".into(),
                ));
            }
            ranges.push((c.start, c.end));
        }
        Ok(())
    }
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut cur: Option<Open> = None;
    let mut saw_moof = false;
    for b in &tops[init_boxes..] {
        let in_group = cur.as_ref().is_some_and(|c| b.start < c.group_end);
        if b.is(b"moov") {
            return Err(SplitError::Unsupported(
                "a second moov box after media data (an initialisation segment in the middle)"
                    .into(),
            ));
        } else if is_leading(&b.typ) {
            if cur.as_ref().is_some_and(|c| c.has_moof) && !in_group {
                flush(&mut cur, &mut ranges)?;
            }
            let group_end = if b.is(b"sidx") {
                sidx_group_end(data, b)?
            } else {
                0
            };
            match cur.as_mut() {
                Some(c) => {
                    c.end = b.end;
                    c.group_end = c.group_end.max(group_end);
                }
                None => {
                    cur = Some(Open {
                        start: b.start,
                        end: b.end,
                        has_moof: false,
                        has_mdat: false,
                        group_end,
                    });
                }
            }
        } else if b.is(b"moof") {
            saw_moof = true;
            match cur.as_mut() {
                Some(c) if !c.has_moof => {
                    c.has_moof = true;
                    c.end = b.end;
                }
                Some(c) if in_group => {
                    c.has_moof = true;
                    c.end = b.end;
                }
                Some(_) => {
                    flush(&mut cur, &mut ranges)?;
                    cur = Some(Open {
                        start: b.start,
                        end: b.end,
                        has_moof: true,
                        has_mdat: false,
                        group_end: 0,
                    });
                }
                None => {
                    cur = Some(Open {
                        start: b.start,
                        end: b.end,
                        has_moof: true,
                        has_mdat: false,
                        group_end: 0,
                    });
                }
            }
        } else if b.is(b"mdat") {
            match cur.as_mut() {
                Some(c) if c.has_moof => {
                    c.end = b.end;
                    c.has_mdat = true;
                }
                _ => {
                    return Err(SplitError::NotFragmented(
                        "an mdat box with no moof box before it".to_string(),
                    ));
                }
            }
        } else if in_group && let Some(c) = cur.as_mut() {
            c.end = b.end;
        }
        // free, skip, mfra, unknown boxes outside a group: dropped.
    }
    flush(&mut cur, &mut ranges)?;
    if !saw_moof && !info.has_mvex {
        return Err(SplitError::NotFragmented(
            "no moof box and the moov box has no mvex".to_string(),
        ));
    }

    let mime = match mime_of(&info) {
        Ok(m) => m,
        Err(e) if mime_hint.is_empty() => return Err(e),
        Err(_) => mime_hint.to_string(),
    };

    let mut next_ticks: HashMap<u32, u64> = HashMap::new();
    let mut media = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        let bytes = data[start..end].to_vec();
        let t = analyze_segment(&bytes, &info, &mut next_ticks)?;
        media.push(Segment {
            kind: SegmentKind::Media,
            bytes,
            start_ms: Some(t.start_ms),
            duration_ms: t.duration_ms,
        });
    }
    Ok(SegmentedStream {
        container: Container::Mp4,
        mime,
        init: Segment {
            kind: SegmentKind::Init,
            bytes: init_bytes.to_vec(),
            start_ms: None,
            duration_ms: None,
        },
        media,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SegmentTiming {
    pub start_ms: u64,
    pub duration_ms: Option<u64>,
}

struct Tfhd {
    track_id: u32,
    flags: u32,
    default_duration: Option<u32>,
}

fn parse_tfhd(data: &[u8], b: &BoxRef) -> Result<Tfhd, SplitError> {
    let p = b.payload_of(data);
    let flags = u32_at(p, 0)? & 0x00ff_ffff;
    let track_id = u32_at(p, 4)?;
    let mut pos = 8;
    if flags & 0x01 != 0 {
        pos += 8;
    }
    if flags & 0x02 != 0 {
        pos += 4;
    }
    let default_duration = if flags & 0x08 != 0 {
        Some(u32_at(p, pos)?)
    } else {
        None
    };
    Ok(Tfhd {
        track_id,
        flags,
        default_duration,
    })
}

/// Sum of sample durations of a `trun`, `None` when it says no duration and `default` is `None`.
fn trun_duration(data: &[u8], b: &BoxRef, default: Option<u32>) -> Result<Option<u64>, SplitError> {
    let p = b.payload_of(data);
    let flags = u32_at(p, 0)? & 0x00ff_ffff;
    let count = u32_at(p, 4)? as usize;
    let mut pos = 8;
    if flags & 0x01 != 0 {
        pos += 4;
    }
    if flags & 0x04 != 0 {
        pos += 4;
    }
    let entry_size = 4 * [0x100u32, 0x200, 0x400, 0x800]
        .iter()
        .filter(|&&bit| flags & bit != 0)
        .count();
    if entry_size > 0 {
        let need = count
            .checked_mul(entry_size)
            .and_then(|n| n.checked_add(pos))
            .ok_or(SplitError::Truncated)?;
        if need > p.len() {
            return Err(SplitError::Truncated);
        }
    }
    if flags & 0x100 != 0 {
        let mut total = 0u64;
        for i in 0..count {
            total = total.saturating_add(u64::from(u32_at(p, pos + i * entry_size)?));
        }
        Ok(Some(total))
    } else {
        match default {
            Some(d) => Ok(Some((count as u64).saturating_mul(u64::from(d)))),
            None => Ok(None),
        }
    }
}

struct TrafTiming {
    track_id: u32,
    tfdt: Option<u64>,
    duration: Option<u64>,
}

fn traf_timing(data: &[u8], traf: &BoxRef, info: &InitInfo) -> Result<TrafTiming, SplitError> {
    let mut tfhd = None;
    let mut tfdt = None;
    let mut truns = Vec::new();
    for c in boxes(data, traf.payload, traf.end) {
        let c = c?;
        if c.is(b"tfhd") {
            tfhd = Some(parse_tfhd(data, &c)?);
        } else if c.is(b"tfdt") {
            let p = c.payload_of(data);
            tfdt = Some(if u8_at(p, 0)? == 1 {
                u64_at(p, 4)?
            } else {
                u64::from(u32_at(p, 4)?)
            });
        } else if c.is(b"trun") {
            truns.push(c);
        }
    }
    let tfhd = tfhd.ok_or_else(|| SplitError::Malformed("traf without tfhd".into()))?;
    let track = info.track(tfhd.track_id).ok_or_else(|| {
        SplitError::Malformed(format!("traf for unknown track {}", tfhd.track_id))
    })?;
    let default = tfhd
        .default_duration
        .or((track.default_duration != 0).then_some(track.default_duration));
    let mut duration = Some(0u64);
    for t in &truns {
        duration = match (duration, trun_duration(data, t, default)?) {
            (Some(a), Some(b)) => Some(a.saturating_add(b)),
            _ => None,
        };
    }
    if tfhd.flags & 0x10000 != 0 {
        // duration-is-empty
        duration = Some(0);
    }
    if truns.is_empty() {
        duration = Some(0);
    }
    Ok(TrafTiming {
        track_id: tfhd.track_id,
        tfdt,
        duration,
    })
}

/// Start and duration of one media segment, from `tfdt` and the `trun` sample durations (falling
/// back to the `sidx` durations when no `trun` gives any).
pub(crate) fn analyze_segment(
    seg: &[u8],
    info: &InitInfo,
    next_ticks: &mut HashMap<u32, u64>,
) -> Result<SegmentTiming, SplitError> {
    let mut start: Option<u64> = None;
    let mut end: Option<u64> = None;
    let mut unknown = false;
    let mut sidx_ms = 0u64;
    let mut have_sidx = false;
    for b in boxes(seg, 0, seg.len()) {
        let b = b?;
        if b.is(b"sidx") {
            let ix = parse_sidx(seg, &b)?;
            have_sidx = true;
            let ticks: u64 = ix.refs.iter().map(|r| u64::from(r.duration)).sum();
            sidx_ms = sidx_ms.saturating_add(ticks_to_ms(ticks, ix.timescale));
        } else if b.is(b"moof") {
            for c in boxes(seg, b.payload, b.end) {
                let c = c?;
                if !c.is(b"traf") {
                    continue;
                }
                let t = traf_timing(seg, &c, info)?;
                let ts = info.track(t.track_id).map_or(1, |tr| tr.timescale);
                let start_ticks = t
                    .tfdt
                    .or_else(|| next_ticks.get(&t.track_id).copied())
                    .unwrap_or(0);
                let s = ticks_to_ms(start_ticks, ts);
                start = Some(start.map_or(s, |x| x.min(s)));
                match t.duration {
                    Some(d) => {
                        let e_ticks = start_ticks.saturating_add(d);
                        next_ticks.insert(t.track_id, e_ticks);
                        let e = ticks_to_ms(e_ticks, ts);
                        end = Some(end.map_or(e, |x| x.max(e)));
                    }
                    None => unknown = true,
                }
            }
        }
    }
    let start = start.ok_or_else(|| SplitError::Malformed("a segment without a moof".into()))?;
    let mut duration = match end {
        Some(e) if !unknown && e >= start => Some(e - start),
        _ => None,
    };
    if duration.is_none_or(|d| d == 0) && have_sidx && sidx_ms > 0 {
        duration = Some(sidx_ms);
    }
    Ok(SegmentTiming {
        start_ms: start,
        duration_ms: duration,
    })
}

// ---------------------------------------------------------------------------------------------
// Shifting media time

/// The new bytes of a `tfdt` box and how much it grew.
fn shifted_tfdt(data: &[u8], tfdt: &BoxRef, delta_ticks: i128) -> Result<Vec<u8>, SplitError> {
    let p = tfdt.payload_of(data);
    let version = u8_at(p, 0)?;
    let flags = slice(p, 1, 4)?;
    let old = if version == 1 {
        i128::from(u64_at(p, 4)?)
    } else {
        i128::from(u32_at(p, 4)?)
    };
    let new = old + delta_ticks;
    if new < 0 {
        return Err(SplitError::Malformed(
            "the shift moves a segment before time zero".into(),
        ));
    }
    let new = u64::try_from(new)
        .map_err(|_| SplitError::Malformed("the shifted time does not fit in 64 bits".into()))?;
    let mut payload = Vec::with_capacity(12);
    if version == 0 && u32::try_from(new).is_ok() {
        payload.push(0);
        payload.extend_from_slice(flags);
        payload.extend_from_slice(&(new as u32).to_be_bytes());
    } else {
        // Version 0 holds 32 bits: version 1 holds 64 and makes the box 4 bytes longer.
        payload.push(1);
        payload.extend_from_slice(flags);
        payload.extend_from_slice(&new.to_be_bytes());
    }
    Ok(make_box(b"tfdt", &payload))
}

/// Rewrites one `moof`; returns the new bytes. `growth` is how much longer the moof became.
fn rewrite_moof(
    seg: &[u8],
    moof: &BoxRef,
    info: &InitInfo,
    delta_ms: i64,
) -> Result<(Vec<u8>, usize), SplitError> {
    struct TrafPlan {
        traf: BoxRef,
        tfdt_bytes: Option<(BoxRef, Vec<u8>)>,
        base_offset_present: bool,
        default_base_is_moof: bool,
    }
    let kids = children(seg, moof)?;
    let mut plans: Vec<Option<TrafPlan>> = Vec::new();
    let mut growth = 0usize;
    for k in &kids {
        if !k.is(b"traf") {
            plans.push(None);
            continue;
        }
        let mut tfhd = None;
        let mut tfdt = None;
        for c in boxes(seg, k.payload, k.end) {
            let c = c?;
            if c.is(b"tfhd") {
                tfhd = Some(parse_tfhd(seg, &c)?);
            } else if c.is(b"tfdt") {
                tfdt = Some(c);
            }
        }
        let tfhd = tfhd.ok_or_else(|| SplitError::Malformed("traf without tfhd".into()))?;
        let track = info.track(tfhd.track_id).ok_or_else(|| {
            SplitError::Malformed(format!("traf for unknown track {}", tfhd.track_id))
        })?;
        let tfdt_bytes = match tfdt {
            Some(t) => {
                let bytes = shifted_tfdt(seg, &t, ms_to_ticks(delta_ms, track.timescale))?;
                growth += bytes.len().saturating_sub(t.len());
                Some((t, bytes))
            }
            None if delta_ms != 0 => {
                return Err(SplitError::Unsupported(
                    "a traf without tfdt cannot be shifted".into(),
                ));
            }
            None => None,
        };
        plans.push(Some(TrafPlan {
            traf: *k,
            tfdt_bytes,
            base_offset_present: tfhd.flags & 0x01 != 0,
            default_base_is_moof: tfhd.flags & 0x02_0000 != 0,
        }));
    }

    let mut body: Vec<u8> = Vec::with_capacity(moof.len() + growth);
    let mut first_traf = true;
    for (k, plan) in kids.iter().zip(plans) {
        let Some(plan) = plan else {
            body.extend_from_slice(k.bytes_of(seg));
            continue;
        };
        let mut traf_body: Vec<u8> = Vec::with_capacity(plan.traf.len() + 8);
        for c in boxes(seg, plan.traf.payload, plan.traf.end) {
            let c = c?;
            if let Some((old, new)) = &plan.tfdt_bytes
                && c.start == old.start
            {
                traf_body.extend_from_slice(new);
                continue;
            }
            if growth > 0 && c.is(b"saio") {
                return Err(SplitError::Unsupported(
                    "growing a moof with saio offsets (encrypted fragments) is not supported"
                        .into(),
                ));
            }
            let mut copy = c.bytes_of(seg).to_vec();
            if growth > 0 && c.is(b"tfhd") && plan.base_offset_present {
                // base_data_offset is an absolute offset: it moves with the data.
                let at = c.header_len() + 8;
                let old = u64_at(&copy, at)?;
                copy[at..at + 8].copy_from_slice(&(old + growth as u64).to_be_bytes());
            }
            if growth > 0
                && c.is(b"trun")
                && !plan.base_offset_present
                && (plan.default_base_is_moof || first_traf)
            {
                let p = c.payload_of(seg);
                let flags = u32_at(p, 0)? & 0x00ff_ffff;
                if flags & 0x01 != 0 {
                    let at = c.header_len() + 8;
                    let old = i64::from(u32_at(&copy, at)? as i32);
                    let new = i32::try_from(old + growth as i64)
                        .map_err(|_| SplitError::Malformed("trun data_offset overflows".into()))?;
                    copy[at..at + 4].copy_from_slice(&new.to_be_bytes());
                }
            }
            traf_body.extend_from_slice(&copy);
        }
        let total = plan.traf.header_len() + traf_body.len();
        body.extend_from_slice(&header_like(&plan.traf, seg, total));
        body.extend_from_slice(&traf_body);
        first_traf = false;
    }
    // Everything between the moof header and its first child is part of the header (no full-box
    // fields in a moof), so the new header is rebuilt from the new total.
    let total = moof.header_len() + body.len();
    let mut out = header_like(moof, seg, total);
    out.extend_from_slice(&body);
    Ok((out, total.saturating_sub(moof.len())))
}

/// Moves the media time of one media segment by `delta_ms`.
pub(crate) fn shift_segment(
    seg: &[u8],
    info: &InitInfo,
    delta_ms: i64,
) -> Result<Vec<u8>, SplitError> {
    let tops: Vec<BoxRef> = boxes(seg, 0, seg.len()).collect::<Result<_, _>>()?;
    let mut out: Vec<u8> = Vec::with_capacity(seg.len() + 16);
    // (original start of a moof, its growth)
    let mut grown: Vec<(usize, usize)> = Vec::new();
    // (start of the sidx in `out`, the sidx as found in `seg`)
    let mut sidxs: Vec<(usize, BoxRef)> = Vec::new();
    for b in &tops {
        if b.is(b"moof") {
            let (bytes, growth) = rewrite_moof(seg, b, info, delta_ms)?;
            if growth > 0 {
                grown.push((b.start, growth));
            }
            out.extend_from_slice(&bytes);
        } else if b.is(b"sidx") {
            let at = out.len();
            out.extend_from_slice(b.bytes_of(seg));
            let ix = parse_sidx(seg, b)?;
            let ept_at = at + ix.ept_at;
            let delta = ms_to_ticks(delta_ms, ix.timescale);
            if ix.ept_len == 4 {
                let old = i128::from(u32_at(&out, ept_at)?);
                let new = u32::try_from(old + delta).map_err(|_| {
                    SplitError::Unsupported(
                        "the shifted sidx earliest_presentation_time does not fit a version 0 sidx"
                            .into(),
                    )
                })?;
                out[ept_at..ept_at + 4].copy_from_slice(&new.to_be_bytes());
            } else {
                let old = i128::from(u64_at(&out, ept_at)?);
                let new = u64::try_from(old + delta).map_err(|_| {
                    SplitError::Malformed("the shift moves a sidx before time zero".into())
                })?;
                out[ept_at..ept_at + 8].copy_from_slice(&new.to_be_bytes());
            }
            sidxs.push((at, *b));
        } else {
            out.extend_from_slice(b.bytes_of(seg));
        }
    }
    // A moof that grew makes the sidx reference that covers it longer.
    for (at, sidx) in sidxs {
        let ix = parse_sidx(seg, &sidx)?;
        let mut pos = (sidx.end as u64).saturating_add(ix.first_offset);
        for r in ix.refs {
            let range_end = pos.saturating_add(u64::from(r.size));
            let extra: usize = grown
                .iter()
                .filter(|(m, _)| (*m as u64) >= pos && (*m as u64) < range_end)
                .map(|(_, g)| *g)
                .sum();
            if extra > 0 {
                let field = at + r.field_at;
                let raw = u32_at(&out, field)?;
                let new = (raw & 0x7fff_ffff) as u64 + extra as u64;
                if new > 0x7fff_ffff {
                    return Err(SplitError::Malformed(
                        "sidx referenced_size overflows".into(),
                    ));
                }
                out[field..field + 4]
                    .copy_from_slice(&((raw & 0x8000_0000) | new as u32).to_be_bytes());
            }
            pos = range_end;
        }
    }
    Ok(out)
}

/// `delta_ms`-shifted copies of the media segments of an MP4 stream with initialisation segment `init`.
pub(crate) fn shifted_media(
    init: &[u8],
    media: &[Segment],
    delta_ms: i64,
) -> Result<Vec<Segment>, SplitError> {
    let info = parse_init(init)?;
    let mut next_ticks = HashMap::new();
    let mut out = Vec::with_capacity(media.len());
    for seg in media {
        let bytes = shift_segment(&seg.bytes, &info, delta_ms)?;
        let t = analyze_segment(&bytes, &info, &mut next_ticks)?;
        out.push(Segment {
            kind: SegmentKind::Media,
            bytes,
            start_ms: Some(t.start_ms),
            duration_ms: t.duration_ms,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// pssh

/// `init` with a version 0 `pssh` box appended as the last child of `moov`.
pub(crate) fn add_pssh(
    init: &[u8],
    system_id: [u8; 16],
    data: &[u8],
) -> Result<Vec<u8>, SplitError> {
    let mut moov = None;
    for b in boxes(init, 0, init.len()) {
        let b = b?;
        if b.is(b"moov") {
            moov = Some(b);
            break;
        }
    }
    let moov =
        moov.ok_or_else(|| SplitError::Malformed("no moov box to add a pssh box to".into()))?;
    let size = u32::try_from(data.len())
        .map_err(|_| SplitError::Unsupported("pssh data longer than 4 GiB".into()))?;
    let mut payload = Vec::with_capacity(24 + data.len());
    payload.extend_from_slice(&[0, 0, 0, 0]); // version 0, flags 0
    payload.extend_from_slice(&system_id);
    payload.extend_from_slice(&size.to_be_bytes());
    payload.extend_from_slice(data);
    let pssh = make_box(b"pssh", &payload);
    let total = moov.len() + pssh.len();
    let mut out = Vec::with_capacity(init.len() + pssh.len());
    out.extend_from_slice(&init[..moov.start]);
    out.extend_from_slice(&header_like(&moov, init, total));
    out.extend_from_slice(moov.payload_of(init));
    out.extend_from_slice(&pssh);
    out.extend_from_slice(&init[moov.end..]);
    Ok(out)
}
