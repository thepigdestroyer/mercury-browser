// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The calls of the WebRTC spike, route L (LiveKit's bindings over the pinned Shiguredo libwebrtc).
//! t01 to t12 are the numbered tests of the sp-rtc brief (shared contract 4); every test prints one
//! line `RESULT tNN <pass|fail|skip> <detail>` on standard output, which the CI job greps. t13 is an
//! extra (the bridge's fake microphone through `LiveKitAudioSink`).
//!
//! These run only on Apple Silicon (the archive is arm64): on the Intel development Mac the crate
//! compiles and links with `--target aarch64-apple-darwin` and the binary cannot be run.
//! Run with `cargo test --release -- --nocapture --test-threads=1`.
//!
//! No test opens a device: no platform audio call is made, the audio sources are `NativeAudioSource`
//! (a tone made here, or the bridge's fake microphone), the video sources are `NativeVideoSource`
//! (a pattern made here, or the bridge's fake camera), and there is no ICE server.

use std::collections::BTreeSet;
use std::fmt::Display;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use libwebrtc::MediaType;
use libwebrtc::audio_frame::AudioFrame;
use libwebrtc::audio_source::AudioSourceOptions;
use libwebrtc::audio_source::native::NativeAudioSource;
use libwebrtc::audio_stream::native::NativeAudioStream;
use libwebrtc::data_channel::{DataChannel, DataChannelInit, DataChannelState};
use libwebrtc::media_stream_track::{MediaStreamTrack, RtcTrackState};
use libwebrtc::peer_connection::OfferOptions;
use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use libwebrtc::peer_connection_factory::{
    ContinualGatheringPolicy, IceTransportsType, PeerConnectionFactory, RtcConfiguration,
};
use libwebrtc::rtp_parameters::RtpEncodingParameters;
use libwebrtc::rtp_sender::{RtpSender, VideoEncoderBackend};
use libwebrtc::rtp_transceiver::{RtpTransceiverDirection, RtpTransceiverInit};
use libwebrtc::stats::{DtlsTransportState, IceCandidatePairState, RtcStats};
use libwebrtc::video_frame::{I420Buffer, VideoFrame, VideoRotation};
use libwebrtc::video_source::VideoResolution;
use libwebrtc::video_source::native::NativeVideoSource;
use libwebrtc::video_stream::native::NativeVideoStream;
use mercury_frame_sink::ClockMap;
use mercury_rtc_bridge::{CameraConfig, CaptureStream, MicrophoneConfig};
use mercury_rtc_livekit_spike::loopback::{Pair, Peer};
use mercury_rtc_livekit_spike::{LiveKitAudioSink, LiveKitVideoSink, consumer_now};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};
use tokio_stream::StreamExt;

// ---------------------------------------------------------------------------------------------
// reporting and small helpers

fn report(id: &str, status: &str, detail: impl Display) {
    println!("RESULT {id} {status} {detail}");
}

fn info(id: &str, detail: impl Display) {
    println!("INFO {id} {detail}");
}

/// Prints the RESULT line and turns a failure into a failed test.
fn finish(id: &str, result: Result<String, String>) {
    match result {
        Ok(detail) => report(id, "pass", detail),
        Err(error) => {
            report(id, "fail", &error);
            panic!("{id} failed: {error}");
        }
    }
}

/// The default configuration: no ICE server, gather continually, all transports.
fn config() -> RtcConfiguration {
    RtcConfiguration::default()
}

fn gather_once(transport: IceTransportsType) -> RtcConfiguration {
    let mut config = RtcConfiguration::default();
    config.continual_gathering_policy = ContinualGatheringPolicy::GatherOnce;
    config.ice_transport_type = transport;
    config
}

async fn wait_open(dc: &DataChannel, who: &str) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if dc.state() == DataChannelState::Open {
            return Ok(());
        }
        if Instant::now() > end {
            return Err(format!(
                "{who}: data channel '{}' is {:?} after 10 s",
                dc.label(),
                dc.state()
            ));
        }
        sleep(Duration::from_millis(20)).await;
    }
}

/// Sends every message back as it came (same bytes, same text/binary flag).
fn install_echo(dc: &DataChannel) {
    let echo = dc.clone();
    dc.on_message(Some(Box::new(move |buffer| {
        let _ = echo.send(buffer.data, buffer.binary);
    })));
}

/// Collects every message of a channel into a queue.
fn collect(dc: &DataChannel) -> mpsc::UnboundedReceiver<(Vec<u8>, bool)> {
    let (tx, rx) = mpsc::unbounded_channel();
    dc.on_message(Some(Box::new(move |buffer| {
        let _ = tx.send((buffer.data.to_vec(), buffer.binary));
    })));
    rx
}

fn stat_kind(stat: &RtcStats) -> &'static str {
    match stat {
        RtcStats::Codec(_) => "codec",
        RtcStats::InboundRtp(_) => "inbound-rtp",
        RtcStats::OutboundRtp(_) => "outbound-rtp",
        RtcStats::RemoteInboundRtp(_) => "remote-inbound-rtp",
        RtcStats::RemoteOutboundRtp(_) => "remote-outbound-rtp",
        RtcStats::MediaSource(_) => "media-source",
        RtcStats::MediaPlayout(_) => "media-playout",
        RtcStats::PeerConnection(_) => "peer-connection",
        RtcStats::DataChannel(_) => "data-channel",
        RtcStats::Transport(_) => "transport",
        RtcStats::CandidatePair(_) => "candidate-pair",
        RtcStats::LocalCandidate(_) => "local-candidate",
        RtcStats::RemoteCandidate(_) => "remote-candidate",
        RtcStats::Certificate(_) => "certificate",
        RtcStats::Stream(_) => "stream",
        RtcStats::Track => "track",
    }
}

/// `kind x count` for every kind present, sorted by kind.
fn kinds(stats: &[RtcStats]) -> String {
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for stat in stats {
        *counts.entry(stat_kind(stat)).or_default() += 1;
    }
    counts
        .iter()
        .map(|(k, n)| format!("{k}x{n}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// A moving pattern as a video source, for the tests that need frames but not the bridge.
fn spawn_pattern_video(
    source: NativeVideoSource,
    width: u32,
    height: u32,
    fps: u32,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_micros(1_000_000 / u64::from(fps)));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut n: u32 = 0;
        loop {
            tick.tick().await;
            let mut buffer = I420Buffer::new(width, height);
            let (y, u, v) = buffer.data_mut();
            let shift = (n % 256) as u8;
            for (i, px) in y.iter_mut().enumerate() {
                *px =
                    ((i as u32 % width) as u8).wrapping_add(shift) ^ ((i as u32 / width / 4) as u8);
            }
            u.fill(96u8.wrapping_add(shift / 4));
            v.fill(160u8.wrapping_sub(shift / 4));
            let mut frame = VideoFrame::new(VideoRotation::VideoRotation0, buffer);
            frame.timestamp_us = consumer_now().0;
            source.capture_frame(&frame);
            n = n.wrapping_add(1);
        }
    })
}

// ---------------------------------------------------------------------------------------------
// t01 two peers, loopback, no ICE server

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t01_two_peers_connect() {
    finish("t01", t01().await);
}

async fn t01() -> Result<String, String> {
    let factory = PeerConnectionFactory::default();
    let pair = Pair::new(&factory, &config())?;
    let _dc = pair
        .offerer
        .pc
        .create_data_channel("t01", DataChannelInit::default())
        .map_err(|e| format!("create_data_channel: {e:?}"))?;
    pair.negotiate(OfferOptions::default()).await?;
    let took = pair.wait_connected(Duration::from_secs(10)).await?;
    let (a, b) = (pair.offerer.candidates(), pair.answerer.candidates());
    if a.is_empty() || b.is_empty() {
        return Err(format!(
            "connected but no local candidates were reported (offerer {}, answerer {})",
            a.len(),
            b.len()
        ));
    }
    Ok(format!(
        "connected in {} ms with no ICE server; local candidates offerer {} answerer {}; ice {:?}/{:?}",
        took.as_millis(),
        a.len(),
        b.len(),
        pair.offerer.pc.ice_connection_state(),
        pair.answerer.pc.ice_connection_state()
    ))
}

// ---------------------------------------------------------------------------------------------
// t02 data channels

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t02_data_channel_echo() {
    finish("t02", t02().await);
}

fn payload(size: usize, binary: bool) -> Vec<u8> {
    if binary {
        (0..size)
            .map(|i| (i.wrapping_mul(31) % 251) as u8)
            .collect()
    } else {
        (0..size).map(|i| b'a' + (i % 26) as u8).collect()
    }
}

async fn t02() -> Result<String, String> {
    let factory = PeerConnectionFactory::default();
    let pair = Pair::new(&factory, &config())?;
    let (seen_tx, mut seen_rx) = mpsc::unbounded_channel::<String>();
    pair.answerer.pc.on_data_channel(Some(Box::new(move |dc| {
        install_echo(&dc);
        let _ = seen_tx.send(dc.label());
    })));
    let create = |label: &str, init: DataChannelInit| {
        pair.offerer
            .pc
            .create_data_channel(label, init)
            .map_err(|e| format!("create_data_channel({label}): {e:?}"))
    };
    let ordered = create(
        "echo",
        DataChannelInit {
            protocol: "mercury-echo".into(),
            ..Default::default()
        },
    )?;
    let unordered = create(
        "unordered",
        DataChannelInit {
            ordered: false,
            max_retransmits: Some(0),
            ..Default::default()
        },
    )?;
    let negotiated = DataChannelInit {
        negotiated: true,
        id: 7,
        protocol: "mercury-neg".into(),
        ..Default::default()
    };
    let neg_a = create("neg", negotiated.clone())?;
    let neg_b = pair
        .answerer
        .pc
        .create_data_channel("neg", negotiated)
        .map_err(|e| format!("answerer neg: {e:?}"))?;
    install_echo(&neg_b);
    let mut ordered_rx = collect(&ordered);
    let mut unordered_rx = collect(&unordered);
    let mut neg_rx = collect(&neg_a);

    pair.negotiate(OfferOptions::default()).await?;
    pair.wait_connected(Duration::from_secs(10)).await?;
    for (dc, who) in [
        (&ordered, "offerer"),
        (&unordered, "offerer"),
        (&neg_a, "offerer"),
        (&neg_b, "answerer"),
    ] {
        wait_open(dc, who).await?;
    }
    let mut labels = BTreeSet::new();
    for _ in 0..2 {
        let label = timeout(Duration::from_secs(5), seen_rx.recv())
            .await
            .map_err(|_| "the answerer did not announce both in-band channels".to_owned())?
            .ok_or("announce queue closed")?;
        labels.insert(label);
    }
    if labels != BTreeSet::from(["echo".to_owned(), "unordered".to_owned()]) {
        return Err(format!(
            "the answerer announced {labels:?}, expected echo and unordered (the negotiated one is not announced)"
        ));
    }

    // text and binary echo, in order, 1 B / 16 KiB / 256 KiB
    let mut sent = Vec::new();
    for size in [1usize, 16 * 1024, 256 * 1024] {
        for binary in [false, true] {
            let data = payload(size, binary);
            ordered
                .send(&data, binary)
                .map_err(|e| format!("send {size} B binary={binary}: {e}"))?;
            sent.push((data, binary));
        }
    }
    for (index, expected) in sent.iter().enumerate() {
        let got = timeout(Duration::from_secs(20), ordered_rx.recv())
            .await
            .map_err(|_| {
                format!(
                    "echo {index} ({} B, binary={}) did not arrive in 20 s",
                    expected.0.len(),
                    expected.1
                )
            })?
            .ok_or("echo queue closed")?;
        if got != *expected {
            return Err(format!(
                "echo {index} differs: sent {} B binary={}, got {} B binary={}",
                expected.0.len(),
                expected.1,
                got.0.len(),
                got.1
            ));
        }
    }

    // unordered, no retransmits: on loopback everything should still arrive
    let total = 30usize;
    for i in 0..total {
        unordered
            .send(format!("msg-{i:03}").as_bytes(), true)
            .map_err(|e| format!("unordered send {i}: {e}"))?;
    }
    let mut received = BTreeSet::new();
    let end = Instant::now() + Duration::from_secs(5);
    while received.len() < total && Instant::now() < end {
        if let Ok(Some((data, _))) = timeout(Duration::from_millis(500), unordered_rx.recv()).await
        {
            received.insert(data);
        }
    }
    if received.len() * 10 < total * 9 {
        return Err(format!(
            "unordered channel with maxRetransmits=0 delivered {}/{total} on loopback",
            received.len()
        ));
    }

    // negotiated channel: same id on both sides, no in-band announcement
    neg_a
        .send(b"neg-hello", false)
        .map_err(|e| format!("negotiated send: {e}"))?;
    let (echoed, _) = timeout(Duration::from_secs(5), neg_rx.recv())
        .await
        .map_err(|_| "negotiated channel echo did not arrive".to_owned())?
        .ok_or("neg queue closed")?;
    if echoed != b"neg-hello" || neg_a.id() != 7 || neg_b.id() != 7 {
        return Err(format!(
            "negotiated channel ids {}/{} or echo {:?} wrong",
            neg_a.id(),
            neg_b.id(),
            echoed
        ));
    }

    // label, protocol, id as the stats report them (the channel object has no protocol getter)
    let stats = pair
        .offerer
        .pc
        .get_stats()
        .await
        .map_err(|e| format!("get_stats: {e:?}"))?;
    let mut described = Vec::new();
    for stat in &stats {
        if let RtcStats::DataChannel(d) = stat {
            described.push(format!(
                "{}:{}:id{}",
                d.dc.label, d.dc.protocol, d.dc.data_channel_identifier
            ));
            if d.dc.label == "echo" && d.dc.protocol != "mercury-echo" {
                return Err(format!(
                    "protocol of 'echo' is '{}' in the stats",
                    d.dc.protocol
                ));
            }
            if d.dc.label == "neg"
                && (d.dc.protocol != "mercury-neg" || d.dc.data_channel_identifier != 7)
            {
                return Err(format!(
                    "negotiated channel in the stats: protocol '{}' id {}",
                    d.dc.protocol, d.dc.data_channel_identifier
                ));
            }
        }
    }
    if described.len() < 3 {
        return Err(format!(
            "only {} data-channel stats entries: {described:?}",
            described.len()
        ));
    }
    Ok(format!(
        "6 ordered echoes (1 B, 16 KiB, 256 KiB; text and binary) equal and in order; unordered maxRetransmits=0 {}/{total}; negotiated id 7 ok; stats {}",
        received.len(),
        described.join(" ")
    ))
}

// ---------------------------------------------------------------------------------------------
// t03 stats

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t03_stats_shape() {
    finish("t03", t03().await);
}

async fn t03() -> Result<String, String> {
    let factory = PeerConnectionFactory::default();
    let pair = Pair::new(&factory, &config())?;
    let dc = pair
        .offerer
        .pc
        .create_data_channel("stats", DataChannelInit::default())
        .map_err(|e| format!("{e:?}"))?;
    let (dc_tx, mut dc_rx) = mpsc::unbounded_channel::<DataChannel>();
    pair.answerer.pc.on_data_channel(Some(Box::new(move |dc| {
        install_echo(&dc);
        let _ = dc_tx.send(dc);
    })));
    let mut rx = collect(&dc);
    pair.negotiate(OfferOptions::default()).await?;
    pair.wait_connected(Duration::from_secs(10)).await?;
    wait_open(&dc, "offerer").await?;
    for i in 0..10 {
        dc.send(format!("stats-{i}").as_bytes(), false)
            .map_err(|e| format!("send: {e}"))?;
    }
    for _ in 0..10 {
        timeout(Duration::from_secs(5), rx.recv())
            .await
            .map_err(|_| "echo missing".to_owned())?;
    }
    let _remote = timeout(Duration::from_secs(5), dc_rx.recv())
        .await
        .map_err(|_| "no remote channel".to_owned())?;

    let mut detail = Vec::new();
    for (who, peer) in [("offerer", &pair.offerer), ("answerer", &pair.answerer)] {
        let stats = peer
            .pc
            .get_stats()
            .await
            .map_err(|e| format!("{who} get_stats: {e:?}"))?;
        let have: BTreeSet<&str> = stats.iter().map(stat_kind).collect();
        for need in [
            "transport",
            "candidate-pair",
            "local-candidate",
            "remote-candidate",
            "data-channel",
            "peer-connection",
            "certificate",
        ] {
            if !have.contains(need) {
                return Err(format!("{who}: no '{need}' entry; got {}", kinds(&stats)));
            }
        }
        let nominated = stats.iter().any(|s| {
            matches!(s, RtcStats::CandidatePair(p) if p.candidate_pair.nominated && p.candidate_pair.state == Some(IceCandidatePairState::Succeeded))
        });
        if !nominated {
            return Err(format!(
                "{who}: no nominated candidate pair in state succeeded"
            ));
        }
        let dtls_ok = stats.iter().any(|s| matches!(s, RtcStats::Transport(t) if t.transport.dtls_state == Some(DtlsTransportState::Connected)));
        if !dtls_ok {
            return Err(format!("{who}: no transport with dtlsState connected"));
        }
        let sent: u32 = stats
            .iter()
            .filter_map(|s| {
                if let RtcStats::DataChannel(d) = s {
                    Some(d.dc.messages_sent)
                } else {
                    None
                }
            })
            .sum();
        let received: u32 = stats
            .iter()
            .filter_map(|s| {
                if let RtcStats::DataChannel(d) = s {
                    Some(d.dc.messages_received)
                } else {
                    None
                }
            })
            .sum();
        if sent < 10 || received < 10 {
            return Err(format!(
                "{who}: data-channel counters sent {sent} received {received}, expected at least 10 each"
            ));
        }
        detail.push(format!("{who} {} entries [{}]", stats.len(), kinds(&stats)));
    }
    Ok(detail.join("; "))
}

// ---------------------------------------------------------------------------------------------
// t04 audio: a tone from a NativeAudioSource reaches the remote audio stream, no device is opened

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t04_audio_tone() {
    finish("t04", t04().await);
}

/// What a remote audio stream delivered during a window.
struct AudioWindow {
    blocks: f64,
    seconds: f64,
    rms: f64,
    hz: f64,
}

/// Reads a remote audio stream: skips `skip`, then measures for `measure`.
async fn measure_audio(
    mut stream: NativeAudioStream,
    channels: usize,
    skip: Duration,
    measure: Duration,
) -> AudioWindow {
    let start = Instant::now();
    let (mut samples, mut square_sum, mut crossings, mut previous): (u64, f64, u64, i16) =
        (0, 0.0, 0, 0);
    let mut window_started: Option<Instant> = None;
    while start.elapsed() < skip + measure + Duration::from_secs(5) {
        let Ok(Some(frame)) = timeout(Duration::from_millis(500), stream.next()).await else {
            continue;
        };
        if start.elapsed() < skip {
            continue;
        }
        let began = *window_started.get_or_insert_with(Instant::now);
        if began.elapsed() > measure {
            break;
        }
        for chunk in frame.data.chunks(channels.max(1)) {
            let s = chunk[0];
            square_sum += f64::from(s) * f64::from(s);
            if (previous < 0) != (s < 0) {
                crossings += 1;
            }
            previous = s;
            samples += 1;
        }
    }
    let seconds = window_started
        .map(|t| t.elapsed().as_secs_f64())
        .unwrap_or(0.0)
        .max(0.001);
    let rms = if samples > 0 {
        (square_sum / samples as f64).sqrt()
    } else {
        0.0
    };
    AudioWindow {
        blocks: samples as f64 / 480.0,
        seconds,
        rms,
        hz: crossings as f64 / 2.0 / (samples as f64 / 48_000.0).max(0.001),
    }
}

async fn t04() -> Result<String, String> {
    let factory = PeerConnectionFactory::default();
    let pair = Pair::new(&factory, &config())?;
    let source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
    let track = factory.create_audio_track("tone", source.clone());
    pair.offerer
        .pc
        .add_track(MediaStreamTrack::Audio(track), &["tone-stream"])
        .map_err(|e| format!("add_track: {e:?}"))?;
    let (track_tx, mut track_rx) = mpsc::unbounded_channel::<MediaStreamTrack>();
    pair.answerer.pc.on_track(Some(Box::new(move |event| {
        let _ = track_tx.send(event.track);
    })));
    pair.negotiate(OfferOptions::default()).await?;
    pair.wait_connected(Duration::from_secs(10)).await?;

    // 440 Hz, amplitude 10000 (RMS 7071), pushed as 10 ms blocks for 7 s
    let pump = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
        for block in 0..700u64 {
            tick.tick().await;
            let mut frame = AudioFrame::new(48_000, 1, 480);
            let first = block * 480;
            for (i, sample) in frame.data.to_mut().iter_mut().enumerate() {
                *sample = (10_000.0
                    * (2.0 * std::f64::consts::PI * 440.0 * (first + i as u64) as f64 / 48_000.0)
                        .sin()) as i16;
            }
            let _ = source.capture_frame(&frame).await;
        }
    });
    let remote = timeout(Duration::from_secs(10), track_rx.recv())
        .await
        .map_err(|_| "no remote track".to_owned())?
        .ok_or("track queue closed")?;
    let MediaStreamTrack::Audio(audio) = remote else {
        return Err("the remote track is not audio".to_owned());
    };
    let stream = NativeAudioStream::new(audio, 48_000, 1);
    let window = measure_audio(
        stream,
        1,
        Duration::from_millis(1500),
        Duration::from_secs(3),
    )
    .await;
    pump.abort();

    // no device: the factory's audio device module is the synthetic one, so there is nothing to list
    let (playout, recording) = (factory.playout_devices(), factory.recording_devices());
    if playout != 0 || recording != 0 {
        return Err(format!(
            "a platform audio device module exists: {playout} playout and {recording} recording devices"
        ));
    }
    let per_second = window.blocks / window.seconds;
    if window.rms < 3000.0 {
        return Err(format!(
            "remote RMS {:.0} is below 3000 (sent 7071)",
            window.rms
        ));
    }
    if !(90.0..=110.0).contains(&per_second) {
        return Err(format!("{per_second:.1} blocks/s, expected about 100"));
    }
    if !(400.0..=480.0).contains(&window.hz) {
        return Err(format!(
            "zero crossings say {:.0} Hz, expected 440",
            window.hz
        ));
    }
    Ok(format!(
        "RMS {:.0} (sent 7071), {per_second:.1} blocks/s over {:.1} s, {:.0} Hz; playout devices {playout}, recording devices {recording}",
        window.rms, window.seconds, window.hz
    ))
}

// ---------------------------------------------------------------------------------------------
// video call from the bridge's fake camera (t05, t07, t10)

struct VideoCall {
    #[allow(dead_code)]
    factory: PeerConnectionFactory,
    pair: Pair,
    camera: CaptureStream,
    sink: Arc<LiveKitVideoSink>,
    sender: RtpSender,
    stream: NativeVideoStream,
    offerer_dc: DataChannel,
    answerer_dc: DataChannel,
}

impl VideoCall {
    async fn start(width: u32, height: u32) -> Result<VideoCall, String> {
        let factory = PeerConnectionFactory::default();
        let pair = Pair::new(&factory, &config())?;
        let (track_tx, mut track_rx) = mpsc::unbounded_channel::<MediaStreamTrack>();
        pair.answerer.pc.on_track(Some(Box::new(move |event| {
            let _ = track_tx.send(event.track);
        })));
        let (dc_tx, mut dc_rx) = mpsc::unbounded_channel::<DataChannel>();
        pair.answerer.pc.on_data_channel(Some(Box::new(move |dc| {
            let _ = dc_tx.send(dc);
        })));
        let offerer_dc = pair
            .offerer
            .pc
            .create_data_channel("ctl", DataChannelInit::default())
            .map_err(|e| format!("{e:?}"))?;

        let camera = CaptureStream::fake_camera(CameraConfig::new(width, height, 30))
            .map_err(|e| format!("fake_camera: {e}"))?;
        let source = NativeVideoSource::new(VideoResolution { width, height }, false);
        let track = factory.create_video_track("cam", source.clone());
        let clock = ClockMap::new(camera.producer_now(), consumer_now());
        let sink = Arc::new(LiveKitVideoSink::new(source, Some(clock)));
        camera
            .add_frame_sink(sink.clone())
            .map_err(|e| format!("add_frame_sink: {e}"))?;
        let sender = pair
            .offerer
            .pc
            .add_track(MediaStreamTrack::Video(track), &["cam-stream"])
            .map_err(|e| format!("add_track: {e:?}"))?;

        pair.negotiate(OfferOptions::default()).await?;
        pair.wait_connected(Duration::from_secs(10)).await?;
        let remote = timeout(Duration::from_secs(10), track_rx.recv())
            .await
            .map_err(|_| "no remote video track".to_owned())?
            .ok_or("track queue closed")?;
        let MediaStreamTrack::Video(video) = remote else {
            return Err("the remote track is not video".to_owned());
        };
        let answerer_dc = timeout(Duration::from_secs(10), dc_rx.recv())
            .await
            .map_err(|_| "no remote data channel".to_owned())?
            .ok_or("dc queue closed")?;
        let stream = NativeVideoStream::new(video);
        Ok(VideoCall {
            factory,
            pair,
            camera,
            sink,
            sender,
            stream,
            offerer_dc,
            answerer_dc,
        })
    }
}

/// What the remote video stream delivered during a window.
struct VideoWindow {
    frames: usize,
    seconds: f64,
    sizes: BTreeSet<(u32, u32)>,
    monotonic: bool,
}

async fn measure_video(
    stream: &mut NativeVideoStream,
    warmup: Duration,
    measure: Duration,
) -> Result<VideoWindow, String> {
    let first = timeout(Duration::from_secs(15), stream.next())
        .await
        .map_err(|_| "no video frame in 15 s".to_owned())?;
    if first.is_none() {
        return Err("the remote video stream ended".to_owned());
    }
    let warm_end = Instant::now() + warmup;
    while Instant::now() < warm_end {
        let _ = timeout(Duration::from_millis(200), stream.next()).await;
    }
    let started = Instant::now();
    let (mut frames, mut sizes, mut monotonic, mut last) =
        (0usize, BTreeSet::new(), true, i64::MIN);
    while started.elapsed() < measure {
        let Ok(Some(frame)) = timeout(Duration::from_millis(500), stream.next()).await else {
            continue;
        };
        frames += 1;
        sizes.insert((frame.buffer.width(), frame.buffer.height()));
        if frame.timestamp_us < last {
            monotonic = false;
        }
        last = frame.timestamp_us;
    }
    Ok(VideoWindow {
        frames,
        seconds: started.elapsed().as_secs_f64(),
        sizes,
        monotonic,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t05_video_from_the_bridge() {
    finish("t05", t05().await);
}

async fn t05() -> Result<String, String> {
    let mut detail = Vec::new();
    for (width, height) in [(640u32, 360u32), (641, 361)] {
        let mut call = VideoCall::start(width, height).await?;
        // The first CI run counted 51 frames in the window from 1 s to 4 s: libwebrtc starts every
        // call at a low bandwidth estimate and drops frames until it ramps up. So the first window
        // is recorded (`cold`) and the requirement of 60 frames in 3 s applies to a second window
        // that starts about five seconds after the first frame.
        let cold = measure_video(
            &mut call.stream,
            Duration::from_secs(1),
            Duration::from_secs(3),
        )
        .await?;
        let window = measure_video(
            &mut call.stream,
            Duration::from_secs(1),
            Duration::from_secs(3),
        )
        .await?;
        let counters = call.sink.counters().snapshot();
        call.camera.stop();
        if window.frames < 60 {
            return Err(format!(
                "{width}x{height}: {} frames in {:.1} s after ramp-up (need 60 in 3 s; the cold window had {}); sink {counters:?}",
                window.frames, window.seconds, cold.frames
            ));
        }
        if !window.sizes.contains(&(width, height)) || window.sizes.len() != 1 {
            return Err(format!(
                "{width}x{height}: remote frame sizes {:?}",
                window.sizes
            ));
        }
        if !window.monotonic {
            return Err(format!(
                "{width}x{height}: remote frame timestamps went backwards"
            ));
        }
        info(
            "t05",
            format!(
                "{width}x{height} sink {counters:?}, copy {:.0} us/frame",
                counters.copy_ns as f64 / counters.frames.max(1) as f64 / 1000.0
            ),
        );
        detail.push(format!(
            "{width}x{height}: {} frames in {:.1} s ({:.1} fps) after ramp-up (cold window {} frames), size kept, timestamps non-decreasing",
            window.frames,
            window.seconds,
            window.frames as f64 / window.seconds,
            cold.frames
        ));
    }
    // cost of handing libwebrtc an NV12 frame (what a real camera gives) into a source with no track
    let probe = NativeVideoSource::new(
        VideoResolution {
            width: 640,
            height: 360,
        },
        false,
    );
    let sink = LiveKitVideoSink::new(probe, None);
    let (y, uv) = (vec![90u8; 640 * 360], vec![128u8; 640 * 180]);
    let started = Instant::now();
    for i in 0..200 {
        sink.push_nv12(640, 360, &y, &uv, mercury_frame_sink::Micros(i * 33_333));
    }
    info(
        "t05",
        format!(
            "NV12 640x360 push into a NativeVideoSource: {:.0} us/frame",
            started.elapsed().as_micros() as f64 / 200.0
        ),
    );
    Ok(detail.join("; "))
}

// ---------------------------------------------------------------------------------------------
// t06 simulcast

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t06_simulcast() {
    finish("t06", t06().await);
}

fn encoding(rid: &str, scale: f64) -> RtpEncodingParameters {
    RtpEncodingParameters {
        rid: rid.to_owned(),
        scale_resolution_down_by: Some(scale),
        ..Default::default()
    }
}

/// The answer of a server that receives the three layers, appended to libwebrtc's own answer (the
/// test has one video section, the last one).
fn server_style_answer(answer: &str) -> String {
    format!(
        "{}\r\na=rid:q recv\r\na=rid:h recv\r\na=rid:f recv\r\na=simulcast:recv q;h;f\r\n",
        answer.trim_end()
    )
}

fn count_lines(sdp: &str, prefix: &str) -> usize {
    sdp.lines().filter(|l| l.starts_with(prefix)).count()
}

async fn t06() -> Result<String, String> {
    let factory = PeerConnectionFactory::default();
    let pair = Pair::new(&factory, &config())?;
    let source = NativeVideoSource::new(
        VideoResolution {
            width: 1280,
            height: 720,
        },
        false,
    );
    let track = factory.create_video_track("sim", source.clone());
    let pump = spawn_pattern_video(source, 1280, 720, 30);
    let init = RtpTransceiverInit {
        direction: RtpTransceiverDirection::SendOnly,
        stream_ids: vec!["sim-stream".to_owned()],
        send_encodings: vec![encoding("q", 4.0), encoding("h", 2.0), encoding("f", 1.0)],
    };
    let transceiver = pair
        .offerer
        .pc
        .add_transceiver(MediaStreamTrack::Video(track), init)
        .map_err(|e| format!("add_transceiver with three encodings: {e:?}"))?;
    // libwebrtc as the *answerer* does not accept simulcast (the first CI run: 0 a=rid and 0
    // a=simulcast in its answer, with the rid extension negotiated), so the offerer gets the answer
    // a server (an SFU) would write: the three rids with `recv` and `a=simulcast:recv`. webrtc's own
    // PeerConnectionSimulcastTests rewrite the answer the same way ("to look like a server response").
    let sdps = pair
        .negotiate_with_answer(OfferOptions::default(), Some(&server_style_answer))
        .await?;
    pair.wait_connected(Duration::from_secs(10)).await?;
    let (offer_rids, answer_rids, applied_rids) = (
        count_lines(&sdps.offer, "a=rid:"),
        count_lines(&sdps.answer, "a=rid:"),
        count_lines(&sdps.applied_answer, "a=rid:"),
    );
    let (offer_sim, answer_sim, applied_sim) = (
        count_lines(&sdps.offer, "a=simulcast:"),
        count_lines(&sdps.answer, "a=simulcast:"),
        count_lines(&sdps.applied_answer, "a=simulcast:"),
    );
    if offer_rids != 3 || offer_sim != 1 || applied_rids != 3 || applied_sim != 1 {
        pump.abort();
        return Err(format!(
            "SDP: offer has {offer_rids} a=rid and {offer_sim} a=simulcast, the applied answer {applied_rids} and {applied_sim}\n{}",
            sdps.applied_answer
        ));
    }
    let negotiated_rids: Vec<String> = transceiver
        .sender()
        .parameters()
        .encodings
        .iter()
        .map(|e| e.rid.clone())
        .collect();

    // the outbound stats show one entry per layer; wait until each layer has encoded something
    let end = Instant::now() + Duration::from_secs(25);
    let mut layers: Vec<(String, u32, u32, u32)>;
    loop {
        let stats = pair
            .offerer
            .pc
            .get_stats()
            .await
            .map_err(|e| format!("get_stats: {e:?}"))?;
        layers = stats
            .iter()
            .filter_map(|s| match s {
                RtcStats::OutboundRtp(o) if o.stream.kind == "video" => Some((
                    o.outbound.rid.clone(),
                    o.outbound.frame_width,
                    o.outbound.frame_height,
                    o.outbound.frames_encoded,
                )),
                _ => None,
            })
            .collect();
        layers.sort();
        let all_encoding = ["f", "h", "q"]
            .iter()
            .all(|rid| layers.iter().any(|l| l.0 == *rid && l.3 > 0));
        if all_encoding || Instant::now() > end {
            break;
        }
        sleep(Duration::from_millis(500)).await;
    }
    pump.abort();
    let rids: BTreeSet<&str> = layers.iter().map(|l| l.0.as_str()).collect();
    if rids != BTreeSet::from(["f", "h", "q"]) {
        return Err(format!(
            "outbound-rtp entries have rids {rids:?}, expected f, h, q; layers {layers:?}"
        ));
    }
    let described: Vec<String> = layers
        .iter()
        .map(|l| format!("{}:{}x{}:{} frames", l.0, l.1, l.2, l.3))
        .collect();

    // scalability mode: record whether the engine accepts L1T3 on a VP8 sender
    let scalability = scalability_probe(&factory).await;
    Ok(format!(
        "negotiated rids {negotiated_rids:?}, SDP a=rid offer {offer_rids} / libwebrtc answer {answer_rids} / applied {applied_rids}, a=simulcast offer {offer_sim} / libwebrtc answer {answer_sim} / applied {applied_sim} (libwebrtc does not answer simulcast; the offerer got a server-style answer); layers {}; scalability_mode L1T3: {scalability}",
        described.join(" ")
    ))
}

async fn scalability_probe(factory: &PeerConnectionFactory) -> String {
    let pair = match Pair::new(factory, &config()) {
        Ok(pair) => pair,
        Err(e) => return format!("probe could not start: {e}"),
    };
    let source = NativeVideoSource::new(
        VideoResolution {
            width: 640,
            height: 360,
        },
        false,
    );
    let track = factory.create_video_track("svc", source);
    let one = RtpEncodingParameters {
        scalability_mode: Some("L1T3".to_owned()),
        ..Default::default()
    };
    let init = RtpTransceiverInit {
        direction: RtpTransceiverDirection::SendOnly,
        stream_ids: vec![],
        send_encodings: vec![one],
    };
    match pair
        .offerer
        .pc
        .add_transceiver(MediaStreamTrack::Video(track), init)
    {
        Err(e) => format!("refused at add_transceiver ({e:?})"),
        Ok(transceiver) => {
            let before = transceiver
                .sender()
                .parameters()
                .encodings
                .first()
                .and_then(|e| e.scalability_mode.clone());
            match pair.negotiate(OfferOptions::default()).await {
                Err(e) => format!(
                    "accepted at add_transceiver (parameters say {before:?}) but negotiation failed: {e}"
                ),
                Ok(_) => {
                    let after = transceiver
                        .sender()
                        .parameters()
                        .encodings
                        .first()
                        .and_then(|e| e.scalability_mode.clone());
                    format!(
                        "accepted (sender parameters before negotiation {before:?}, after {after:?})"
                    )
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// t07 teardown

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t07_teardown() {
    finish("t07", t07().await);
}

async fn t07() -> Result<String, String> {
    let mut call = VideoCall::start(640, 360).await?;
    wait_open(&call.offerer_dc, "offerer").await?;
    wait_open(&call.answerer_dc, "answerer").await?;
    // Video starts slowly (the first CI run: 9 frames in the first second after 0.5 s): look at
    // one-second windows for up to ten seconds until one has at least ten frames.
    let mut first_frames = 0;
    for _ in 0..10 {
        let window = measure_video(
            &mut call.stream,
            Duration::from_millis(0),
            Duration::from_secs(1),
        )
        .await?;
        first_frames = window.frames;
        if first_frames >= 10 {
            break;
        }
    }
    if first_frames < 10 {
        return Err(format!(
            "only {first_frames} frames in the last second before the stop"
        ));
    }

    // 1: the camera is released -> ended() reaches the adapter once -> the adapter detaches the track
    let sender = call.sender.clone();
    call.sink.set_on_ended(move || {
        let _ = sender.set_track(None);
    });
    call.camera.stop();
    call.camera.stop();
    let ended = call.sink.counters().snapshot().ended;
    if ended != 1 {
        return Err(format!(
            "ended() reached the sink {ended} times after two stop() calls, expected exactly once"
        ));
    }
    // frames stop reaching the remote side (a few queued ones may still arrive)
    sleep(Duration::from_millis(500)).await;
    while timeout(Duration::from_millis(100), call.stream.next())
        .await
        .is_ok()
    {}
    let after = measure_video(
        &mut call.stream,
        Duration::from_millis(0),
        Duration::from_secs(1),
    )
    .await;
    let frames_after = after.map(|w| w.frames).unwrap_or(0);
    if frames_after > 5 {
        return Err(format!(
            "{frames_after} frames still arrived in the second after the sender's track was detached"
        ));
    }

    // 2: close() on one side ends the remote data channel; what happens to the remote track and connection
    call.pair.offerer.pc.close();
    let end = Instant::now() + Duration::from_secs(10);
    let mut dc_state = call.answerer_dc.state();
    while Instant::now() < end && dc_state != DataChannelState::Closed {
        sleep(Duration::from_millis(100)).await;
        dc_state = call.answerer_dc.state();
    }
    if dc_state != DataChannelState::Closed {
        return Err(format!(
            "the remote data channel is {dc_state:?} 10 s after the other side closed"
        ));
    }
    let remote_track_state = call.stream.track().state();
    let connection = call.pair.answerer.pc.connection_state();
    let ended_remote = remote_track_state == RtcTrackState::Ended;
    Ok(format!(
        "ended() once after two stop() calls; sender detached, {frames_after} frames after; remote data channel Closed after close(); remote track {remote_track_state:?} (ended={ended_remote}), answerer connection {connection:?}"
    ))
}

// ---------------------------------------------------------------------------------------------
// t08 privacy policy (OD-9)

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t08_candidate_policy() {
    finish("t08", t08().await);
}

/// The type (`host`, `srflx`, `relay`) and whether the address is an mDNS name.
fn classify(candidate: &str) -> (String, bool) {
    let fields: Vec<&str> = candidate.split_whitespace().collect();
    let kind = fields
        .iter()
        .position(|f| *f == "typ")
        .and_then(|i| fields.get(i + 1))
        .copied()
        .unwrap_or("?")
        .to_owned();
    let mdns = fields.get(4).is_some_and(|a| a.ends_with(".local"));
    (kind, mdns)
}

async fn gather(transport: IceTransportsType) -> Result<Vec<String>, String> {
    let factory = PeerConnectionFactory::default();
    let peer = Peer::new(&factory, "gatherer", gather_once(transport))?;
    let _dc = peer
        .pc
        .create_data_channel("g", DataChannelInit::default())
        .map_err(|e| format!("{e:?}"))?;
    let offer = peer
        .pc
        .create_offer(OfferOptions::default())
        .await
        .map_err(|e| format!("create_offer: {e:?}"))?;
    peer.pc
        .set_local_description(offer)
        .await
        .map_err(|e| format!("set_local_description: {e:?}"))?;
    peer.wait_gathered(Duration::from_secs(10)).await?;
    let lines = peer.candidates();
    peer.pc.close();
    Ok(lines)
}

async fn t08() -> Result<String, String> {
    let all = gather(IceTransportsType::All).await?;
    let no_host = gather(IceTransportsType::NoHost).await?;
    let relay = gather(IceTransportsType::Relay).await?;
    let summary = |lines: &[String]| {
        let parts: Vec<String> = lines
            .iter()
            .map(|l| classify(l))
            .map(|(k, m)| if m { format!("{k}(mdns)") } else { k })
            .collect();
        format!("{} [{}]", lines.len(), parts.join(","))
    };
    let host_in = |lines: &[String]| lines.iter().any(|l| classify(l).0 == "host");
    if !host_in(&all) {
        return Err(format!(
            "All: no host candidate was gathered: {}",
            summary(&all)
        ));
    }
    if host_in(&no_host) {
        return Err(format!(
            "NoHost still gathered a host candidate: {}",
            summary(&no_host)
        ));
    }
    if !relay.is_empty() {
        return Err(format!(
            "Relay with no TURN server gathered candidates: {}",
            summary(&relay)
        ));
    }
    let mdns = all.iter().any(|l| classify(l).1);
    Ok(format!(
        "All {} ; NoHost {} ; Relay (no TURN) {} ; host candidates are {} (RtcConfiguration has no mDNS switch, webrtc-sys installs no mDNS responder)",
        summary(&all),
        summary(&no_host),
        summary(&relay),
        if mdns {
            "mDNS names (*.local)"
        } else {
            "plain addresses"
        }
    ))
}

// ---------------------------------------------------------------------------------------------
// t09 codecs

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t09_codecs() {
    finish("t09", t09().await);
}

async fn t09() -> Result<String, String> {
    let factory = PeerConnectionFactory::default();
    let names = |media: MediaType, send: bool| -> BTreeSet<String> {
        let caps = if send {
            factory.get_rtp_sender_capabilities(media)
        } else {
            factory.get_rtp_receiver_capabilities(media)
        };
        caps.codecs
            .iter()
            .map(|c| c.mime_type.to_ascii_lowercase())
            .collect()
    };
    let (video_send, video_recv) = (
        names(MediaType::Video, true),
        names(MediaType::Video, false),
    );
    let (audio_send, audio_recv) = (
        names(MediaType::Audio, true),
        names(MediaType::Audio, false),
    );
    let video_need = ["video/vp8", "video/vp9", "video/av1"];
    let audio_need = ["audio/opus"];
    for (what, set, need) in [
        ("video sender", &video_send, video_need.as_slice()),
        ("video receiver", &video_recv, video_need.as_slice()),
        ("audio sender", &audio_send, audio_need.as_slice()),
        ("audio receiver", &audio_recv, audio_need.as_slice()),
    ] {
        for codec in need {
            if !set.contains(*codec) {
                return Err(format!("{what} capabilities lack {codec}: {set:?}"));
            }
        }
    }
    let software = factory.video_encoder_codecs(VideoEncoderBackend::Software);
    let auto = factory.video_encoder_codecs(VideoEncoderBackend::Auto);
    let toolbox = factory.video_encoder_codecs(VideoEncoderBackend::VideoToolbox);
    let h264 = |list: &[libwebrtc::video_codec::VideoCodec]| {
        list.contains(&libwebrtc::video_codec::VideoCodec::H264)
    };
    if h264(&software) {
        return Err(format!(
            "the software encoder set offers H264: {software:?}"
        ));
    }
    let backends: Vec<VideoEncoderBackend> =
        VideoEncoderBackend::list_available().into_iter().collect();
    Ok(format!(
        "video send {video_send:?}; audio send {audio_send:?}; H264 in sender capabilities {}; encoder sets: software {software:?}, auto {auto:?}, VideoToolbox {toolbox:?}; backends {backends:?}; no software H264 (H264 {} in the software set)",
        video_send.contains("video/h264"),
        if h264(&software) { "present" } else { "absent" }
    ))
}

// ---------------------------------------------------------------------------------------------
// t10 coexistence with GStreamer's own codecs in the same process

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t10_coexists_with_gstreamer() {
    finish("t10", t10().await);
}

/// Calls into the C libraries Mercury links (zlib, SQLite, aws-lc), which sit in this binary next to
/// libwebrtc.a's own zlib, BoringSSL and libvpx. A duplicate symbol would have stopped the link; this
/// shows that each resolves to a working implementation at run time.
fn c_libraries() -> Result<String, String> {
    // SAFETY: plain calls into the C libraries with valid pointers and lengths.
    unsafe {
        let zlib = std::ffi::CStr::from_ptr(libz_sys::zlibVersion())
            .to_string_lossy()
            .into_owned();
        let sqlite = std::ffi::CStr::from_ptr(libsqlite3_sys::sqlite3_libversion())
            .to_string_lossy()
            .into_owned();
        let mut random = [0u8; 16];
        if aws_lc_sys::RAND_bytes(random.as_mut_ptr(), random.len()) != 1 {
            return Err("aws-lc RAND_bytes failed".to_owned());
        }
        if random == [0u8; 16] {
            return Err("aws-lc RAND_bytes returned zeros".to_owned());
        }
        Ok(format!(
            "zlib {zlib}, sqlite {sqlite}, aws-lc RAND_bytes ok"
        ))
    }
}

async fn t10() -> Result<String, String> {
    let libraries = c_libraries()?;
    mercury_rtc_bridge::init().map_err(|e| format!("gstreamer init: {e}"))?;
    let launch = |description: &str| -> Result<gst::Element, String> {
        gst::parse::launch(description).map_err(|e| format!("gst launch '{description}': {e}"))
    };
    let video = launch(
        "videotestsrc is-live=true ! video/x-raw,width=320,height=240 ! vp8enc deadline=1 ! vp8dec ! fakesink sync=false",
    )?;
    let audio = launch("audiotestsrc is-live=true ! opusenc ! opusdec ! fakesink sync=false")?;
    for element in [&video, &audio] {
        element
            .set_state(gst::State::Playing)
            .map_err(|e| format!("gst playing: {e}"))?;
    }
    sleep(Duration::from_millis(500)).await;
    let first = t01().await?;
    let mut call = VideoCall::start(640, 360).await?;
    // As in t05: a cold window is recorded and the requirement applies to the window after it.
    let cold = measure_video(
        &mut call.stream,
        Duration::from_secs(1),
        Duration::from_secs(3),
    )
    .await?;
    let window = measure_video(
        &mut call.stream,
        Duration::from_secs(1),
        Duration::from_secs(3),
    )
    .await?;
    call.camera.stop();
    let mut gst_errors = Vec::new();
    for element in [&video, &audio] {
        if let Some(bus) = element.bus() {
            while let Some(message) = bus.pop_filtered(&[gst::MessageType::Error]) {
                gst_errors.push(format!("{message:?}"));
            }
        }
        element
            .set_state(gst::State::Null)
            .map_err(|e| format!("gst null: {e}"))?;
    }
    if !gst_errors.is_empty() {
        return Err(format!("GStreamer reported errors: {gst_errors:?}"));
    }
    if window.frames < 60 {
        return Err(format!(
            "with GStreamer codecs running only {} frames in 3 s after ramp-up (cold window {})",
            window.frames, cold.frames
        ));
    }
    Ok(format!(
        "{libraries}; vp8enc/vp8dec and opusenc/opusdec ran in the same process; t01 ({first}); t05 {} frames in {:.1} s after ramp-up (cold window {})",
        window.frames, window.seconds, cold.frames
    ))
}

// ---------------------------------------------------------------------------------------------
// t11 threads and shutdown

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t11_factory_churn() {
    finish("t11", t11().await);
}

/// Threads of this process (`ps -M` prints one line per thread).
fn thread_count() -> usize {
    let output = std::process::Command::new("ps")
        .args(["-M", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .count()
        .saturating_sub(1)
}

async fn t11() -> Result<String, String> {
    sleep(Duration::from_millis(500)).await;
    let start = thread_count();
    let mut after_first = 0;
    for round in 0..10 {
        let factory = PeerConnectionFactory::default();
        let pair = Pair::new(&factory, &config())?;
        let dc = pair
            .offerer
            .pc
            .create_data_channel("churn", DataChannelInit::default())
            .map_err(|e| format!("{e:?}"))?;
        pair.negotiate(OfferOptions::default()).await?;
        pair.wait_connected(Duration::from_secs(10)).await?;
        wait_open(&dc, "offerer").await?;
        match round % 3 {
            0 => {
                pair.close(true);
                drop(pair);
                drop(factory);
            }
            1 => {
                pair.close(false);
                drop(factory);
                drop(pair);
            }
            _ => {
                drop(factory);
                drop(pair);
            }
        }
        drop(dc);
        if round == 0 {
            sleep(Duration::from_millis(500)).await;
            after_first = thread_count();
        }
    }
    // Whatever the first factory leaves behind for good (a process-wide thread) is not a leak; growth
    // after it is. The first CI run counted one more thread after ten cycles than after the first
    // (4, 6 and 7): tokio's blocking pool keeps an idle thread for ten seconds, so the count is
    // polled for up to fifteen seconds before it is judged.
    let limit = start.max(after_first);
    let settle_until = Instant::now() + Duration::from_secs(15);
    let mut end = thread_count();
    while end > limit && Instant::now() < settle_until {
        sleep(Duration::from_millis(500)).await;
        end = thread_count();
    }
    if end > limit {
        return Err(format!(
            "threads at start {start}, after the first factory {after_first}, after ten {end}: the count grows"
        ));
    }
    Ok(format!(
        "10 factories created, connected, closed in three orders and dropped; threads: start {start}, after the first {after_first}, after ten {end} (back to the start: {})",
        end == start
    ))
}

// ---------------------------------------------------------------------------------------------
// t12 fail-closed build (the part that runs in the test binary; the cargo-level check is in the part file)

#[test]
fn t12_build_refuses_without_the_pinned_directory() {
    finish("t12", t12());
}

struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl EnvGuard {
    fn set(vars: &[(&'static str, &str)]) -> EnvGuard {
        let saved = vars
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .collect();
        for (k, v) in vars {
            // SAFETY: the test binary runs these tests one at a time (--test-threads=1).
            unsafe { std::env::set_var(k, v) };
        }
        EnvGuard(saved)
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in self.0.drain(..) {
            // SAFETY: as above.
            unsafe {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}

type Defines = Vec<(String, Option<String>)>;

fn panic_message(result: Result<Defines, Box<dyn std::any::Any + Send>>) -> Option<String> {
    match result {
        Ok(_) => None,
        Err(payload) => Some(
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default(),
        ),
    }
}

fn t12() -> Result<String, String> {
    let scratch = std::env::temp_dir().join(format!("mercury-t12-{}", std::process::id()));
    let empty = scratch.join("empty");
    let unmarked = scratch.join("unmarked");
    std::fs::create_dir_all(empty.join("lib")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(unmarked.join("lib")).map_err(|e| e.to_string())?;
    std::fs::write(unmarked.join("lib/libwebrtc.a"), b"not an archive")
        .map_err(|e| e.to_string())?;

    let mut outcomes = Vec::new();
    for (name, dir, minos, needle) in [
        (
            "empty directory",
            &empty,
            "14.0",
            "not a verified libwebrtc directory",
        ),
        (
            "lib without the verification marker",
            &unmarked,
            "14.0",
            "not a verified libwebrtc directory",
        ),
        (
            "deployment target 13.0",
            &unmarked,
            "13.0",
            "needs 14.0 or later",
        ),
    ] {
        let dir_text = dir.to_string_lossy().into_owned();
        let _guard = EnvGuard::set(&[
            ("CARGO_CFG_TARGET_OS", "macos"),
            ("CARGO_CFG_TARGET_ARCH", "aarch64"),
            ("MACOSX_DEPLOYMENT_TARGET", minos),
            ("LK_CUSTOM_WEBRTC", &dir_text),
        ]);
        match panic_message(std::panic::catch_unwind(webrtc_sys_build::webrtc_defines)) {
            Some(text) if text.contains(needle) => outcomes.push(format!("{name}: refused")),
            Some(text) => {
                return Err(format!(
                    "{name}: refused with an unexpected message: {text}"
                ));
            }
            None => {
                return Err(format!(
                    "{name}: webrtc_defines() returned defines instead of refusing"
                ));
            }
        }
    }
    // a verified directory (the marker exists) yields the defines, so the refusals above are the checks and not a broken build
    std::fs::write(unmarked.join(".mercury-webrtc-verified"), b"test")
        .map_err(|e| e.to_string())?;
    let defines = {
        let dir_text = unmarked.to_string_lossy().into_owned();
        let _guard = EnvGuard::set(&[
            ("CARGO_CFG_TARGET_OS", "macos"),
            ("CARGO_CFG_TARGET_ARCH", "aarch64"),
            ("MACOSX_DEPLOYMENT_TARGET", "14.0"),
            ("LK_CUSTOM_WEBRTC", &dir_text),
        ]);
        webrtc_sys_build::webrtc_defines()
    };
    if !defines.iter().any(|(k, _)| k == "WEBRTC_MAC") {
        return Err("a verified directory did not produce the defines".to_owned());
    }
    if webrtc_sys_build::download_webrtc().is_ok() {
        return Err(
            "download_webrtc() succeeded; the local build helper must have no downloader"
                .to_owned(),
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(format!(
        "{}; verified directory gives {} defines; download_webrtc() refuses",
        outcomes.join(", "),
        defines.len()
    ))
}

// ---------------------------------------------------------------------------------------------
// t13 extra: the bridge's fake microphone through LiveKitAudioSink

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t13_bridge_microphone() {
    finish("t13", t13().await);
}

async fn t13() -> Result<String, String> {
    let factory = PeerConnectionFactory::default();
    let pair = Pair::new(&factory, &config())?;
    let microphone = CaptureStream::fake_microphone(MicrophoneConfig {
        channels: 1,
        frequency: 440.0,
    })
    .map_err(|e| format!("fake_microphone: {e}"))?;
    let source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
    let track = factory.create_audio_track("mic", source.clone());
    let sink = Arc::new(LiveKitAudioSink::new(source));
    microphone
        .add_frame_sink(sink.clone())
        .map_err(|e| format!("add_frame_sink: {e}"))?;
    pair.offerer
        .pc
        .add_track(MediaStreamTrack::Audio(track), &["mic-stream"])
        .map_err(|e| format!("add_track: {e:?}"))?;
    let (track_tx, mut track_rx) = mpsc::unbounded_channel::<MediaStreamTrack>();
    pair.answerer.pc.on_track(Some(Box::new(move |event| {
        let _ = track_tx.send(event.track);
    })));
    pair.negotiate(OfferOptions::default()).await?;
    pair.wait_connected(Duration::from_secs(10)).await?;
    let remote = timeout(Duration::from_secs(10), track_rx.recv())
        .await
        .map_err(|_| "no remote track".to_owned())?
        .ok_or("track queue closed")?;
    let MediaStreamTrack::Audio(audio) = remote else {
        return Err("the remote track is not audio".to_owned());
    };
    let window = measure_audio(
        NativeAudioStream::new(audio, 48_000, 1),
        1,
        Duration::from_millis(1500),
        Duration::from_secs(3),
    )
    .await;
    microphone.stop();
    let counters = sink.counters().snapshot();
    let per_second = window.blocks / window.seconds;
    if counters.rejected > 0 || window.rms < 500.0 || !(90.0..=110.0).contains(&per_second) {
        return Err(format!(
            "RMS {:.0}, {per_second:.1} blocks/s, sink {counters:?}",
            window.rms
        ));
    }
    Ok(format!(
        "RMS {:.0}, {per_second:.1} blocks/s, {:.0} Hz; sink pushed {} blocks, rejected {}",
        window.rms, window.hz, counters.frames, counters.rejected
    ))
}
