// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `RigConfig` options over the real GStreamer: `prefer_decoder` raises a decoder's rank (the choice of
//! `decodebin3` changes), and `video: false` / `audio: false` leave a kind of stream uncounted without
//! opening a window or an audio device. A process of its own because ranks are process-wide.

mod common;

use std::time::Duration;

use mse_rig::backend::EosError;
use mse_rig::env::PluginMode;
use mse_rig::rig::{Rig, RigConfig, Shape};

use common::{init, one_rig_at_a_time};

fn play(config: RigConfig) -> Rig {
    let stream = mse_segments::load("h264-aac").expect("the fixture loads");
    let rig = Rig::new(config).expect("a rig");
    let id = rig
        .backend
        .add_source_buffer(&stream.mime)
        .expect("a source buffer");
    let step = Duration::from_secs(10);
    rig.append_and_wait(id, &stream.init.bytes, step)
        .expect("init");
    for segment in &stream.media {
        rig.append_and_wait(id, &segment.bytes, step)
            .expect("segment");
    }
    rig.play().expect("play");
    rig.backend
        .end_of_stream(EosError::None)
        .expect("end of stream");
    assert!(
        rig.wait_for_eos(Duration::from_secs(30)),
        "no end of stream"
    );
    rig
}

fn config(shape: Shape) -> RigConfig {
    RigConfig {
        shape,
        sync: false,
        ..RigConfig::default()
    }
}

#[test]
fn a_preferred_decoder_replaces_the_default_choice() {
    let _one = one_rig_at_a_time();
    init(PluginMode::servo_default());

    let rig = play(config(Shape::PlayBin));
    let default_choice = rig.chosen_decoders();
    println!("default decoders: {default_choice:?}");
    assert!(
        default_choice.iter().any(|d| d.contains(":vtdec")),
        "the platform decoder is the default for H.264: {default_choice:?}"
    );
    rig.shutdown().expect("teardown");

    let rig = play(RigConfig {
        prefer_decoder: Some("avdec_h264".to_owned()),
        ..config(Shape::PlayBin)
    });
    let preferred = rig.chosen_decoders();
    println!("with avdec_h264 preferred: {preferred:?}");
    assert!(
        preferred.iter().any(|d| d.ends_with(":avdec_h264")),
        "{preferred:?}"
    );
    assert!((198..=202).contains(&rig.video.lock().unwrap().count));
    rig.shutdown().expect("teardown");

    let missing = Rig::new(RigConfig {
        prefer_decoder: Some("no_such_decoder".to_owned()),
        ..config(Shape::PlayBin)
    });
    assert!(
        matches!(missing, Err(mse_rig::RigError::Setup(_))),
        "unknown factory"
    );
}

#[test]
fn a_rig_without_video_counts_only_audio_in_both_shapes() {
    let _one = one_rig_at_a_time();
    init(PluginMode::servo_default());
    for shape in [Shape::PlayBin, Shape::Bare] {
        let rig = play(RigConfig {
            video: false,
            ..config(shape)
        });
        let audio = rig.audio.lock().unwrap().count;
        let video = rig.video.lock().unwrap().count;
        println!("{shape:?}: audio {audio}, video {video}");
        assert!(
            (370..=380).contains(&audio),
            "{shape:?}: {audio} audio buffers"
        );
        assert_eq!(video, 0, "{shape:?}: the video was not asked for");
        rig.shutdown().expect("teardown");
    }
}
