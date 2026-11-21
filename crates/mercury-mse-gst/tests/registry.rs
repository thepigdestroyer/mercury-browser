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
fn the_codecs_of_this_round_have_decoders() {
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

#[test]
fn a_kind_that_does_not_match_the_codec_is_refused() {
    common::env::init();
    let support = GstTypeSupport;
    assert!(!support.has_decoder(TrackKind::Audio, CodecId::H264));
    assert!(!support.has_decoder(TrackKind::Video, CodecId::Aac));
}

#[test]
fn the_codecs_decision_d4_leaves_out_have_no_decoder_here() {
    common::env::init();
    let support = GstTypeSupport;
    for codec in [
        CodecId::Hevc,
        CodecId::Mp3,
        CodecId::Flac,
        CodecId::Vorbis,
        CodecId::Ac3,
        CodecId::Eac3,
    ] {
        assert!(decoder_caps(codec).is_none(), "{codec:?}");
        assert!(!support.has_decoder(codec.kind(), codec), "{codec:?}");
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
