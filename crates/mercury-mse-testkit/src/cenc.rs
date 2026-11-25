// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A protected-shaped initialisation segment, for the protection-event probe (T08).
//!
//! [`with_cenc_protection`] turns the first track's sample entry (`avc1`, `mp4a`, ...) into the entry
//! Common Encryption (ISO/IEC 23001-7) prescribes for protected tracks: `encv` (or `enca`) with a
//! `sinf` box (`frma` with the original format, `schm` with the scheme `cenc`, `schi` with a `tenc` that
//! says "protected, this IV size, this key ID"). A demuxer then treats the track as protected: it
//! reports `application/x-cenc` caps and, for the `pssh` boxes of the `moov`, sends protection events.
//!
//! **The samples are not encrypted.** The point is the event path, not decryption: a clear stream with
//! `pssh` boxes makes `qtdemux` emit no protection event (measured in T08), so it cannot serve as a positive
//! control for a probe that looks for them. Nothing here can be decrypted or played.

use crate::bmff::{BoxRef, find_child, make_box, parse_box};
use crate::{Segment, SegmentKind, SplitError};

/// Four-character codes of sample entries that carry pictures (they become `encv`); every other
/// entry becomes `enca`.
const VISUAL_ENTRIES: [&[u8; 4]; 6] = [b"avc1", b"avc3", b"hvc1", b"hev1", b"vp09", b"av01"];

/// What the protection information of the first track says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CencInfo {
    /// The original format from `frma` (`avc1`).
    pub original_format: [u8; 4],
    /// The scheme from `schm` (`cenc`).
    pub scheme: [u8; 4],
    /// `default_isProtected` of the `tenc` box.
    pub is_protected: bool,
    /// `default_Per_Sample_IV_Size` of the `tenc` box.
    pub iv_size: u8,
    /// `default_KID` of the `tenc` box.
    pub key_id: [u8; 16],
}

/// What rewrites the payload of the box at the end of a path.
type Edit<'a> = &'a dyn Fn(&[u8]) -> Result<Vec<u8>, SplitError>;

/// The boxes from `moov` down to the sample description box, in order.
const TO_STSD: [&[u8; 4]; 5] = [b"trak", b"mdia", b"minf", b"stbl", b"stsd"];

/// Rebuilds `parent` with the box at the end of `path` replaced by whatever `edit` makes of its payload;
/// every box on the way keeps its type and gets its size fixed.
fn rewrite(
    data: &[u8],
    parent: &BoxRef,
    path: &[&[u8; 4]],
    edit: Edit<'_>,
) -> Result<Vec<u8>, SplitError> {
    let Some((next, rest)) = path.split_first() else {
        return Ok(make_box(&parent.typ, &edit(parent.payload_of(data))?));
    };
    let child = find_child(data, parent, next)?.ok_or_else(|| {
        SplitError::Malformed(format!(
            "no {} box inside {}",
            String::from_utf8_lossy(*next),
            String::from_utf8_lossy(&parent.typ)
        ))
    })?;
    let rebuilt = rewrite(data, &child, rest, edit)?;
    let mut payload = Vec::with_capacity(parent.end - parent.payload + rebuilt.len());
    payload.extend_from_slice(data.get(parent.payload..child.start).unwrap_or(&[]));
    payload.extend_from_slice(&rebuilt);
    payload.extend_from_slice(data.get(child.end..parent.end).unwrap_or(&[]));
    Ok(make_box(&parent.typ, &payload))
}

/// `sinf` for a track whose original sample entry is `original`.
fn sinf(original: [u8; 4], key_id: [u8; 16], iv_size: u8) -> Vec<u8> {
    let frma = make_box(b"frma", &original);
    let mut schm = vec![0, 0, 0, 0]; // version 0, flags 0
    schm.extend_from_slice(b"cenc");
    schm.extend_from_slice(&[0, 1, 0, 0]); // scheme version 1.0
    let mut tenc = vec![0, 0, 0, 0]; // version 0, flags 0
    tenc.extend_from_slice(&[0, 0]); // two reserved bytes
    tenc.push(1); // default_isProtected
    tenc.push(iv_size);
    tenc.extend_from_slice(&key_id);
    let schi = make_box(b"schi", &make_box(b"tenc", &tenc));
    let mut payload = frma;
    payload.extend_from_slice(&make_box(b"schm", &schm));
    payload.extend_from_slice(&schi);
    make_box(b"sinf", &payload)
}

/// `init` (MP4 only) with its first track declared protected by Common Encryption, as described in the
/// module documentation. The result is not a stream [`split`](crate::split) understands (the type
/// string is derived from the clear sample entries), so append it after splitting the clear one:
/// `with_cenc_protection(&stream.init, ...)`. `iv_size` is 8 or 16 (the per-sample IV size a `senc`
/// box would use); `key_id` is the 16-byte key ID.
pub fn with_cenc_protection(
    init: &Segment,
    key_id: [u8; 16],
    iv_size: u8,
) -> Result<Segment, SplitError> {
    if init.bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Err(SplitError::Unsupported(
            "Common Encryption boxes belong to MP4; WebM protects with ContentEncodings".into(),
        ));
    }
    if iv_size != 8 && iv_size != 16 {
        return Err(SplitError::Unsupported(format!(
            "a per-sample IV size of {iv_size} bytes (8 or 16)"
        )));
    }
    let data = &init.bytes;
    let mut out = Vec::with_capacity(data.len() + 160);
    let mut done = false;
    let mut pos = 0;
    while pos < data.len() {
        let top = parse_box(data, pos, data.len())?;
        if top.is(b"moov") && !done {
            let rebuilt = rewrite(data, &top, &TO_STSD, &|stsd| {
                protect_entry(stsd, key_id, iv_size)
            })?;
            out.extend_from_slice(&rebuilt);
            done = true;
        } else {
            out.extend_from_slice(top.bytes_of(data));
        }
        pos = top.end;
    }
    if !done {
        return Err(SplitError::Malformed("no moov box to protect".into()));
    }
    Ok(Segment {
        kind: SegmentKind::Init,
        bytes: out,
        start_ms: None,
        duration_ms: None,
    })
}

/// The new payload of an `stsd` box whose first entry becomes `encv`/`enca` with a `sinf` appended.
fn protect_entry(stsd: &[u8], key_id: [u8; 16], iv_size: u8) -> Result<Vec<u8>, SplitError> {
    // FullBox header (4 bytes) and entry_count (4 bytes), then the sample entries.
    let entry = parse_box(stsd, 8, stsd.len())?;
    let visual = VISUAL_ENTRIES.iter().any(|typ| entry.is(typ));
    let mut payload = entry.payload_of(stsd).to_vec();
    payload.extend_from_slice(&sinf(entry.typ, key_id, iv_size));
    let new_type: &[u8; 4] = if visual { b"encv" } else { b"enca" };
    let mut out = stsd.get(..8).unwrap_or(&[]).to_vec();
    out.extend_from_slice(&make_box(new_type, &payload));
    out.extend_from_slice(stsd.get(entry.end..).unwrap_or(&[]));
    Ok(out)
}

/// What the protection information of the first track of `init` says; `None` when the first sample
/// entry is not a protected one (`encv`/`enca`) or has no `sinf`.
pub fn cenc_info(init: &Segment) -> Result<Option<CencInfo>, SplitError> {
    let data = &init.bytes;
    let mut pos = 0;
    while pos < data.len() {
        let top = parse_box(data, pos, data.len())?;
        if top.is(b"moov") {
            let mut current = top;
            for step in TO_STSD {
                let Some(next) = find_child(data, &current, step)? else {
                    return Ok(None);
                };
                current = next;
            }
            let stsd = current.payload_of(data);
            let entry = parse_box(stsd, 8, stsd.len())?;
            if !(entry.is(b"encv") || entry.is(b"enca")) {
                return Ok(None);
            }
            // The sample entry has fixed fields before its child boxes (78 bytes of VisualSampleEntry,
            // 28 of AudioSampleEntry): look for the `sinf` by scanning for its box header instead.
            let entry_bytes = entry.bytes_of(stsd);
            let Some(at) = entry_bytes.windows(4).position(|window| window == b"sinf") else {
                return Ok(None);
            };
            let sinf_start = at.checked_sub(4).ok_or(SplitError::Truncated)?;
            let sinf = parse_box(entry_bytes, sinf_start, entry_bytes.len())?;
            return Ok(read_sinf(entry_bytes, &sinf));
        }
        pos = top.end;
    }
    Ok(None)
}

fn read_sinf(data: &[u8], sinf: &BoxRef) -> Option<CencInfo> {
    let frma = find_child(data, sinf, b"frma").ok()??;
    let schm = find_child(data, sinf, b"schm").ok()??;
    let schi = find_child(data, sinf, b"schi").ok()??;
    let tenc = find_child(data, &schi, b"tenc").ok()??;
    let original_format: [u8; 4] = frma.payload_of(data).get(..4)?.try_into().ok()?;
    let scheme: [u8; 4] = schm.payload_of(data).get(4..8)?.try_into().ok()?;
    let tenc = tenc.payload_of(data);
    let key_id: [u8; 16] = tenc.get(8..24)?.try_into().ok()?;
    Some(CencInfo {
        original_format,
        scheme,
        is_protected: *tenc.get(6)? == 1,
        iv_size: *tenc.get(7)?,
        key_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Container, load, split, with_pssh};

    const KID: [u8; 16] = [
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        0x1f,
    ];

    #[test]
    fn a_clear_init_segment_has_no_protection_information() {
        let stream = load("h264-low").unwrap();
        assert_eq!(cenc_info(&stream.init).unwrap(), None);
    }

    #[test]
    fn the_first_track_becomes_a_protected_entry_that_says_what_it_was() {
        let stream = load("h264-low").unwrap();
        let protected = with_cenc_protection(&stream.init, KID, 8).unwrap();
        let info = cenc_info(&protected).unwrap().expect("protection info");
        assert_eq!(&info.original_format, b"avc1");
        assert_eq!(&info.scheme, b"cenc");
        assert!(info.is_protected);
        assert_eq!(info.iv_size, 8);
        assert_eq!(info.key_id, KID);
        // 8 bytes of box header, `frma` 12, `schm` 20, `schi`+`tenc` 8 + 8 + 24: 80 bytes more, all in `moov`.
        assert_eq!(protected.bytes.len(), stream.init.bytes.len() + 80);
    }

    #[test]
    fn every_box_size_on_the_way_down_is_fixed() {
        // moov is the last box of the init segment of this fixture: it must end exactly at the end of the
        // data, and every box inside must parse (the walker checks sizes against their parents).
        let stream = load("h264-low").unwrap();
        let protected = with_cenc_protection(&stream.init, KID, 16).unwrap();
        let mut pos = 0;
        let mut types = Vec::new();
        while pos < protected.bytes.len() {
            let top = parse_box(&protected.bytes, pos, protected.bytes.len()).unwrap();
            types.push(String::from_utf8_lossy(&top.typ).into_owned());
            pos = top.end;
        }
        assert_eq!(pos, protected.bytes.len());
        assert_eq!(types, ["ftyp", "moov"]);
        assert_eq!(cenc_info(&protected).unwrap().unwrap().iv_size, 16);
    }

    #[test]
    fn pssh_boxes_can_be_added_afterwards_and_the_protection_survives() {
        let stream = load("h264-low").unwrap();
        let protected = with_cenc_protection(&stream.init, KID, 8).unwrap();
        let with_box = with_pssh(&protected, [7; 16], b"payload").unwrap();
        assert!(with_box.bytes.len() > protected.bytes.len());
        assert_eq!(
            cenc_info(&with_box).unwrap(),
            cenc_info(&protected).unwrap()
        );
    }

    #[test]
    fn the_samples_and_the_clear_original_are_untouched() {
        let stream = load("h264-low").unwrap();
        let before = stream.init.bytes.clone();
        let _ = with_cenc_protection(&stream.init, KID, 8).unwrap();
        assert_eq!(stream.init.bytes, before);
        // The protected init segment is not a stream `split` can type (the codec string comes from the clear
        // entry), which is why the media segments are taken from the clear stream.
        let mut joined = with_cenc_protection(&stream.init, KID, 8).unwrap().bytes;
        joined.extend_from_slice(&stream.media[0].bytes);
        assert!(split(Container::Mp4, &joined, "").is_err());
    }

    #[test]
    fn webm_other_iv_sizes_and_audio_entries_are_handled() {
        let webm = load("vp9-low").unwrap();
        assert!(matches!(
            with_cenc_protection(&webm.init, KID, 8),
            Err(SplitError::Unsupported(_))
        ));
        let video = load("h264-low").unwrap();
        assert!(matches!(
            with_cenc_protection(&video.init, KID, 12),
            Err(SplitError::Unsupported(_))
        ));
        let audio = load("aac").unwrap();
        let protected = with_cenc_protection(&audio.init, KID, 8).unwrap();
        let info = cenc_info(&protected).unwrap().unwrap();
        assert_eq!(&info.original_format, b"mp4a");
        assert!(
            protected.bytes.windows(4).any(|w| w == b"enca"),
            "an audio entry becomes enca"
        );
        assert!(!protected.bytes.windows(4).any(|w| w == b"encv"));
    }
}
