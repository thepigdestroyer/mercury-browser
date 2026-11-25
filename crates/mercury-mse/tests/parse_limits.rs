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
