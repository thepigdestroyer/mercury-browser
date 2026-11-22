// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What the parsers keep of an initialization segment (lane med-1, review fixes S4 and S5): a codec
//! configuration record has a cap (the track buffer keeps up to 64 of them and the feed copies each into
//! caps), and the `pssh` boxes copied out of a `moov` have a total cap.

mod parse_support;

use mercury_mse::api::{Container, ParseError, ParseEvent};
use mercury_mse::parse::{MAX_CODEC_CONFIG_BYTES, MAX_PSSH_BYTES, new_parser};
use parse_support::*;

fn push_all(container: Container, bytes: &[u8]) -> Result<Vec<ParseEvent>, ParseError> {
    new_parser(container).push(bytes)
}

fn init_of_events(events: &[ParseEvent]) -> &mercury_mse::api::InitSegment {
    events
        .iter()
        .find_map(|e| {
            if let ParseEvent::Init(i) = e {
                Some(i)
            } else {
                None
            }
        })
        .expect("an initialization segment")
}

#[test]
fn s4_an_avcc_over_the_cap_is_refused_and_one_under_it_is_kept() {
    // `avcC` = the valid record followed by padding: the codec string reads only the first bytes.
    let entry = |padding: usize| {
        let mut record = avcc();
        record.extend(std::iter::repeat_n(0u8, padding));
        TrackSpec {
            id: 1,
            handler: "vide",
            timescale: 1000,
            entry: visual_entry("avc1", 320, 180, &bx("avcC", &record)),
        }
    };
    let too_big = init_of_tracks(&[entry(MAX_CODEC_CONFIG_BYTES)]);
    let error = push_all(Container::Mp4, &too_big).expect_err("an avcC over the cap");
    assert!(matches!(error, ParseError::Unsupported(_)), "{error:?}");

    let fine = init_of_tracks(&[entry(1000)]);
    let events = push_all(Container::Mp4, &fine).expect("a normal avcC");
    assert_eq!(init_of_events(&events).tracks.len(), 1);
}

#[test]
fn s4_a_webm_codec_private_over_the_cap_is_refused() {
    let track = |private: &[u8]| {
        elem(
            TRACK_ENTRY,
            &cat(&[
                &uint_elem(0xD7, 1),
                &uint_elem(0x83, 2),
                &elem(0x86, b"A_OPUS"),
                &elem(0x63A2, private),
            ]),
        )
    };
    let big = vec![0u8; MAX_CODEC_CONFIG_BYTES + 1];
    let error = push_all(Container::WebM, &webm_init(&[track(&big)]))
        .expect_err("a CodecPrivate over the cap");
    assert!(matches!(error, ParseError::Unsupported(_)), "{error:?}");
    // A real OpusHead is accepted.
    let events = push_all(Container::WebM, &webm_init(&[track(&opus_head(2))])).expect("OpusHead");
    assert_eq!(init_of_events(&events).tracks.len(), 1);
}

#[test]
fn s5_pssh_boxes_are_copied_only_up_to_the_cap() {
    // 3,000 boxes of about 1 KB: 3 MB in a `moov` the parser accepts, of which at most 1 MiB is copied.
    let pssh: Vec<Vec<u8>> = (0..3000)
        .map(|i| bx("pssh", &payload(i as u8, 1000)))
        .collect();
    let moov = moov_of(&[TrackSpec::video(1, 1000)], &[], &pssh);
    let bytes = cat(&[&ftyp(), &moov]);
    let events = push_all(Container::Mp4, &bytes).expect("a moov with many pssh boxes");
    let copied: usize = init_of_events(&events).pssh.iter().map(Vec::len).sum();
    assert!(
        copied > 0 && copied <= MAX_PSSH_BYTES,
        "{copied} bytes copied"
    );
    // The boxes that are kept are whole boxes, byte for byte (T08 still holds).
    for kept in &init_of_events(&events).pssh {
        assert_eq!(&kept[4..8], b"pssh");
        assert_eq!(kept.len(), 1008);
    }
}

#[test]
fn a_few_pssh_boxes_are_all_kept() {
    let pssh = vec![bx("pssh", &payload(1, 40)), bx("pssh", &payload(2, 80))];
    let moov = moov_of(&[TrackSpec::video(1, 1000)], &[], &pssh);
    let events = push_all(Container::Mp4, &cat(&[&ftyp(), &moov])).expect("parses");
    assert_eq!(init_of_events(&events).pssh, pssh);
}

// ---- the codecs of lane med-1b (decision D7): the 64 KiB cap holds for their records too ----

/// A sample entry whose configuration box is exactly valid with `padding` bytes after the record.
fn entry_with_padding(
    typ: &str,
    config_box: &str,
    record: &[u8],
    padding: usize,
    audio: bool,
) -> TrackSpec {
    let mut payload = record.to_vec();
    payload.extend(std::iter::repeat_n(0u8, padding));
    let children = bx(config_box, &payload);
    TrackSpec {
        id: 1,
        handler: if audio { "soun" } else { "vide" },
        timescale: 48_000,
        entry: if audio {
            audio_entry(typ, 2, 48_000, &children)
        } else {
            visual_entry(typ, 320, 180, &children)
        },
    }
}

fn parses(spec: TrackSpec) -> Result<mercury_mse::api::InitSegment, ParseError> {
    push_all(Container::Mp4, &init_of_tracks(&[spec])).map(|events| init_of_events(&events).clone())
}

fn hvcc_of(nal_len: usize) -> Vec<u8> {
    let mut v = vec![1, 0x01, 0x60, 0, 0, 0, 0xB0, 0, 0, 0, 0, 0, 93];
    v.extend_from_slice(&[0xF0, 0x00, 0xFC, 0xFD, 0xF8, 0xF8, 0, 0, 0x0F, 1]);
    v.push(0x80 | 32);
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend_from_slice(&(nal_len as u16).to_be_bytes());
    v.extend(std::iter::repeat_n(0x42u8, nal_len));
    v
}

#[test]
fn s4_an_hvcc_at_the_cap_is_kept_and_one_byte_more_is_refused() {
    // 23 fixed bytes + array header 3 + NAL length 2 + the NAL: 28 bytes besides the NAL.
    let at_cap = hvcc_of(MAX_CODEC_CONFIG_BYTES - 28);
    assert_eq!(at_cap.len(), MAX_CODEC_CONFIG_BYTES);
    let init =
        parses(entry_with_padding("hvc1", "hvcC", &at_cap, 0, false)).expect("an hvcC at the cap");
    assert_eq!(init.tracks[0].codec_string, "hvc1.1.6.L93.B0");
    let e =
        parses(entry_with_padding("hvc1", "hvcC", &at_cap, 1, false)).expect_err("one byte over");
    assert!(matches!(e, ParseError::Unsupported(_)), "{e:?}");
    let e = parses(entry_with_padding(
        "hev1",
        "hvcC",
        &hvcc_of(10),
        MAX_CODEC_CONFIG_BYTES,
        false,
    ))
    .expect_err("a padded hvcC");
    assert!(matches!(e, ParseError::Unsupported(_)), "{e:?}");
}

#[test]
fn s4_dac3_dec3_dfla_and_an_mp3_esds_over_the_cap_are_refused() {
    let dac3 = [0x10, 0x11, 0x40]; // fscod 0 (48 kHz), bsid 8, acmod 2 (stereo)
    let dec3 = [0x01, 0x80, 0x00, 0x00, 0x00];
    let streaminfo = {
        // The last-block flag is set, so the padding after it is not read as more blocks.
        let mut v = vec![0x80, 0, 0, 34, 0x10, 0, 0x10, 0, 0, 0, 0, 0, 0, 0];
        let packed = (48_000u64 << 44) | (1u64 << 41) | (15u64 << 36);
        v.extend_from_slice(&packed.to_be_bytes());
        v.extend_from_slice(&[0; 16]);
        v
    };
    let mut dfla = vec![0, 0, 0, 0];
    dfla.extend_from_slice(&streaminfo);
    let mut esds = esds_aac();
    let at = esds.iter().position(|&b| b == 0x40).unwrap();
    esds[at] = 0x6B;
    let esds_payload = esds[8..].to_vec(); // the box's own header (size, type) is added by `bx`

    type Case<'a> = (&'a str, &'a str, &'a [u8]);
    let cases: [Case; 4] = [
        ("ac-3", "dac3", &dac3),
        ("ec-3", "dec3", &dec3),
        ("fLaC", "dfLa", &dfla),
        ("mp4a", "esds", &esds_payload),
    ];
    for (typ, config_box, record) in cases {
        // The record alone parses (it is valid), padded to exactly the cap it still does...
        let ok = parses(entry_with_padding(
            typ,
            config_box,
            record,
            MAX_CODEC_CONFIG_BYTES - record.len(),
            true,
        ));
        assert!(ok.is_ok(), "{typ} at the cap: {ok:?}");
        // ... and one byte more is Unsupported before anything is read.
        let e = parses(entry_with_padding(
            typ,
            config_box,
            record,
            MAX_CODEC_CONFIG_BYTES - record.len() + 1,
            true,
        ))
        .expect_err("over the cap");
        assert!(matches!(e, ParseError::Unsupported(_)), "{typ}: {e:?}");
    }
}

#[test]
fn s4_a_webm_vorbis_or_flac_codec_private_over_the_cap_is_refused() {
    let track = |id: &str, private: &[u8]| {
        elem(
            TRACK_ENTRY,
            &cat(&[
                &uint_elem(0xD7, 1),
                &uint_elem(0x83, 2),
                &elem(0x86, id.as_bytes()),
                &elem(0x63A2, private),
            ]),
        )
    };
    let big = vec![0u8; MAX_CODEC_CONFIG_BYTES + 1];
    for id in ["A_VORBIS", "A_FLAC"] {
        let e =
            push_all(Container::WebM, &webm_init(&[track(id, &big)])).expect_err("over the cap");
        assert!(matches!(e, ParseError::Unsupported(_)), "{id}: {e:?}");
        // At the cap the record is read (and found to be no record: Malformed, not Unsupported).
        let e = push_all(
            Container::WebM,
            &webm_init(&[track(id, &big[..MAX_CODEC_CONFIG_BYTES])]),
        )
        .expect_err("zeros are no record");
        assert!(matches!(e, ParseError::Malformed(_)), "{id}: {e:?}");
    }
}

#[test]
fn s5_pssh_boxes_next_to_the_new_codecs_are_copied_byte_for_byte() {
    let pssh = vec![bx("pssh", &payload(1, 40)), bx("pssh", &payload(2, 80))];
    let moov = moov_of(
        &[
            entry_with_padding("hvc1", "hvcC", &hvcc_of(10), 0, false),
            TrackSpec {
                id: 2,
                ..entry_with_padding("ec-3", "dec3", &[0x01, 0x80, 0x00, 0x00, 0x00], 0, true)
            },
        ],
        &[],
        &pssh,
    );
    let events = push_all(Container::Mp4, &cat(&[&ftyp(), &moov])).expect("parses");
    assert_eq!(init_of_events(&events).pssh, pssh);
    assert_eq!(init_of_events(&events).tracks.len(), 2);
}
