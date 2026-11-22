// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `GstTypeSupport` against the registry of the private test GStreamer, and the registration of the
//! element and its URI handler.

mod common;

use gstreamer as gst;
use gstreamer::prelude::*;
use mercury_mse::{CodecId, TrackKind, TypeSupport};
use mercury_mse_gst::{GstTypeSupport, MSE_URI_SCHEME, decoder_caps};

#[test]
fn the_codecs_of_round_one_have_decoders() {
    common::env::init();
    let support = GstTypeSupport;
    for (codec, element) in [
        (CodecId::H264, "avdec_h264"),
        (CodecId::Vp8, "vp8dec"),
        (CodecId::Vp9, "vp9dec"),
        (CodecId::Av1, "dav1ddec"),
        (CodecId::Aac, "avdec_aac"),
        (CodecId::Opus, "opusdec"),
    ] {
        if !common::env::have(&[element]) {
            continue;
        }
        assert!(
            support.has_decoder(codec.kind(), codec),
            "{codec:?} has {element} but GstTypeSupport says no"
        );
    }
}

/// D7: each new codec is accepted when the registry has a decoder that takes it. The elements are the
/// ones the shipped plugin list (libav and vorbis) provides; any of the listed names will do.
#[test]
fn the_new_codecs_have_decoders_in_the_bundle() {
    common::env::init();
    let support = GstTypeSupport;
    for (codec, elements) in [
        (CodecId::Hevc, &["avdec_h265", "vtdec", "vtdec_hw"][..]),
        (CodecId::Ac3, &["avdec_ac3", "a52dec"][..]),
        (CodecId::Eac3, &["avdec_eac3"][..]),
        (
            CodecId::Mp3,
            &["avdec_mp3", "avdec_mp3float", "mpg123audiodec"][..],
        ),
        (CodecId::Flac, &["avdec_flac", "flacdec"][..]),
        (CodecId::Vorbis, &["vorbisdec"][..]),
    ] {
        let present: Vec<&str> = elements
            .iter()
            .copied()
            .filter(|name| gst::ElementFactory::find(name).is_some())
            .collect();
        if present.is_empty() {
            common::env::skip_or_fail(elements[0]);
            continue;
        }
        assert!(
            support.has_decoder(codec.kind(), codec),
            "{codec:?} has {present:?} but GstTypeSupport says no"
        );
    }
}

/// The answer comes from the registry, not from a list: a codec whose decoder this registry lacks is
/// refused. `libgstflac` and `libgstmpg123` are not loaded here, and the Dolby decoders are libav's, so
/// the only refusal that can be forced is the wrong kind and an uninitialised GStreamer elsewhere; what
/// this checks is that the answer for each new codec is a plain bool for its own kind only.
#[test]
fn a_new_codec_is_only_offered_for_its_own_kind() {
    common::env::init();
    let support = GstTypeSupport;
    for codec in [
        CodecId::Hevc,
        CodecId::Ac3,
        CodecId::Eac3,
        CodecId::Mp3,
        CodecId::Flac,
        CodecId::Vorbis,
    ] {
        let other = match codec.kind() {
            TrackKind::Video => TrackKind::Audio,
            _ => TrackKind::Video,
        };
        assert!(!support.has_decoder(other, codec), "{codec:?} as {other:?}");
    }
}

#[test]
fn a_kind_that_does_not_match_the_codec_is_refused() {
    common::env::init();
    let support = GstTypeSupport;
    assert!(!support.has_decoder(TrackKind::Audio, CodecId::H264));
    assert!(!support.has_decoder(TrackKind::Video, CodecId::Aac));
}

#[test]
fn every_codec_has_decoder_caps() {
    for codec in [
        CodecId::H264,
        CodecId::Hevc,
        CodecId::Vp8,
        CodecId::Vp9,
        CodecId::Av1,
        CodecId::Aac,
        CodecId::Opus,
        CodecId::Mp3,
        CodecId::Flac,
        CodecId::Vorbis,
        CodecId::Ac3,
        CodecId::Eac3,
    ] {
        common::env::init();
        assert!(decoder_caps(codec).is_some(), "{codec:?}");
    }
}

#[test]
fn registration_is_idempotent_and_makes_the_uri_handler() {
    common::env::init();
    mercury_mse_gst::register().expect("a second registration");
    mercury_mse_gst::register().expect("a third registration");
    assert!(gst::ElementFactory::find(mercury_mse_gst::ELEMENT_NAME).is_some());
    let element = gst::Element::make_from_uri(
        gst::URIType::Src,
        &format!("{MSE_URI_SCHEME}://7"),
        Some("mse-from-uri"),
    )
    .expect("an element for a mercury-mse URI");
    assert_eq!(
        element.factory().unwrap().name(),
        mercury_mse_gst::ELEMENT_NAME
    );
    let handler = element.dynamic_cast_ref::<gst::URIHandler>().unwrap();
    assert_eq!(
        handler.uri().unwrap().as_str(),
        format!("{MSE_URI_SCHEME}://7")
    );
    assert!(handler.set_uri("http://example.com/").is_err());
}

#[test]
fn attaching_something_else_is_an_error() {
    common::env::init();
    let identity = gst::ElementFactory::make("identity").build().unwrap();
    let feed = common::fakefeed::FakeFeed::new();
    let error = mercury_mse_gst::attach(&identity, feed).unwrap_err();
    assert!(error.contains("not a"), "{error}");
}

#[test]
fn a_second_attach_is_refused_and_a_dropped_element_ends_its_thread() {
    common::env::init();
    let element = gst::ElementFactory::make(mercury_mse_gst::ELEMENT_NAME)
        .build()
        .unwrap();
    let feed = common::fakefeed::FakeFeed::new();
    mercury_mse_gst::attach(&element, feed.clone()).unwrap();
    assert!(mercury_mse_gst::attach(&element, feed).is_err());
    drop(element); // joins the feeder thread: returns, never hangs
}

#[test]
fn a_missing_element_is_a_failure_when_the_bundled_gstreamer_is_in_use() {
    // `test.sh` sets MERCURY_GSTREAMER_PREFIX: there a missing element must fail the test and not skip
    // it (a suite that skips everything passes vacuously). Without it (Linux CI) it skips.
    let prefix_set = std::env::var_os("MERCURY_GSTREAMER_PREFIX").is_some();
    let outcome = std::panic::catch_unwind(|| common::env::skip_or_fail("no-such-element"));
    assert_eq!(outcome.is_err(), prefix_set);
}
