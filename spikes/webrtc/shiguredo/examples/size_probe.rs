// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A program that uses the whole call path (factory, audio and video tracks, data channel, ICE,
//! stats) without GStreamer or any test dependency, so that the stripped size of this binary next
//! to a `hello world` built the same way is the size libwebrtc adds (G7 of the sp-rtc brief). It
//! needs an Apple Silicon Mac to run; the sizes come from linking it.

use std::time::Duration;

use mercury_rtc_shiguredo_spike::channel::ChannelOptions;
use mercury_rtc_shiguredo_spike::engine::{AudioMode, Engine, EngineConfig, VideoCodecs};
use mercury_rtc_shiguredo_spike::peer::{Peer, PeerConfig, connect};
use mercury_rtc_shiguredo_spike::video::LocalVideo;

fn main() {
    let engine = Engine::new(EngineConfig {
        audio: AudioMode::Virtual,
        video: VideoCodecs::Builtin,
    })
    .expect("engine");
    let a = Peer::new(&engine, "a", PeerConfig::default()).expect("peer a");
    let b = Peer::new(&engine, "b", PeerConfig::default()).expect("peer b");
    let video = LocalVideo::new(&engine, "cam").expect("video");
    let _sender = a
        .add_track(&video.media_track(), "stream")
        .expect("add_track");
    let _channel = a
        .create_channel("probe", &ChannelOptions::default())
        .expect("channel");
    let _link = connect(&a, &b, Duration::from_secs(10)).expect("connect");
    println!("{}", a.stats_json().expect("stats").len());
}
