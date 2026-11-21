// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The shape proofs of brief section 6, item 4, in the order the brief says: (a) the bare pipeline with a
//! muxed source buffer, (b) the same under `playbin3`, (c) split audio and video, (d) seeks, (e) a quality
//! switch. Everything here plays the fixtures through the real `MseSrc` with software decoders.

mod common;

use std::time::Duration;

use common::fakefeed::FakeFeed;
use common::rig::{Options, Rig, Shape, watchdog};
use common::sinks::{Colour, classify};

const LIMIT: Duration = Duration::from_secs(30);

fn muxed_plays(shape: Shape) {
    let feed = FakeFeed::from_fixtures(&["h264-aac"]);
    feed.release_all();
    feed.end();
    let rig = Rig::new(
        feed,
        Options {
            shape,
            ..Options::default()
        },
    );
    rig.play();
    rig.wait_eos(LIMIT);
    let report = rig.report();
    let video = rig.video_stats();
    let audio = rig.audio_stats();
    let video = video.lock().unwrap();
    let audio = audio.lock().unwrap();
    eprintln!(
        "{shape:?}: {} video frames, {} audio buffers ({:.3} s)\n{report}",
        video.count(),
        audio.count(),
        audio.total_seconds(),
    );
    assert!(
        (198..=202).contains(&video.count()),
        "video frames {}",
        video.count()
    );
    assert!(audio.total_seconds() > 7.9 && audio.total_seconds() < 8.1);
    let colours = video.colours_since(0);
    assert_eq!(classify(video.arrivals[0].rgb.unwrap()), Colour::Red);
    assert_eq!(colours.last().map(|c| c.1), Some(Colour::Blue));
}

#[test]
fn a_bare_muxed_plays() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "decodebin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("a", 60, || muxed_plays(Shape::Bare));
}

#[test]
fn b_playbin_muxed_plays() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("b", 60, || muxed_plays(Shape::PlayBin));
}

// ---- (c) split audio and video: two tracks that become known and fill independently ----

fn split_plays(shape: Shape, video: &str, audio: &str) {
    use mercury_mse::TrackKind;
    let feed = FakeFeed::new();
    for (name, kind) in [(video, TrackKind::Video), (audio, TrackKind::Audio)] {
        let mut track = common::frames::track(name, kind);
        let visible = false;
        let configs = vec![track.config.clone()];
        let frames = std::mem::take(&mut track.frames)
            .into_iter()
            .map(|f| (0, f))
            .collect();
        feed.add_track_with(kind, configs, frames, visible);
    }
    feed.set_duration(Some(8_000_000_000));
    let rig = Rig::new(
        feed.clone(),
        Options {
            shape,
            ..Options::default()
        },
    );
    rig.play();
    // The two source buffers get their initialization segments and their data apart, in turn.
    let feeder = {
        let feed = feed.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            feed.show_track(TrackKind::Video);
            std::thread::sleep(Duration::from_millis(40));
            feed.release(TrackKind::Video, 0, 2_000_000_000);
            std::thread::sleep(Duration::from_millis(40));
            feed.show_track(TrackKind::Audio);
            feed.set_initialized(true);
            for step in 0..4i64 {
                let (from, to) = (step * 2_000_000_000, (step + 1) * 2_000_000_000);
                feed.release(TrackKind::Audio, from, to);
                std::thread::sleep(Duration::from_millis(15));
                if step > 0 {
                    feed.release(TrackKind::Video, from, to);
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            feed.end();
        })
    };
    rig.wait_eos(LIMIT);
    feeder.join().unwrap();
    let video = rig.video_stats();
    let audio = rig.audio_stats();
    let (video, audio) = (video.lock().unwrap(), audio.lock().unwrap());
    eprintln!(
        "{shape:?} split {video_name}+audio: {} video frames, {} audio buffers ({:.3} s)",
        video.count(),
        audio.count(),
        audio.total_seconds(),
        video_name = video_name_of(&video),
    );
    assert!(
        (198..=202).contains(&video.count()),
        "video frames {}",
        video.count()
    );
    assert!(audio.total_seconds() > 7.9 && audio.total_seconds() < 8.1);
    assert_eq!(
        video.colours_since(0).last().map(|c| c.1),
        Some(Colour::Blue)
    );
}

fn video_name_of(video: &common::sinks::Recorded) -> String {
    video.caps.first().cloned().unwrap_or_default()
}

#[test]
fn c_bare_split_h264_aac_plays() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "decodebin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c-bare", 60, || split_plays(Shape::Bare, "h264-low", "aac"));
}

#[test]
fn c_playbin_split_h264_aac_plays() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c-playbin", 60, || {
        split_plays(Shape::PlayBin, "h264-low", "aac")
    });
}

// ---- (d) seeks inside the buffer, in real time ----

fn seeks_in_buffer(shape: Shape) {
    use common::scenario::seek_and_measure;
    let feed = FakeFeed::from_fixtures(&["h264-aac"]);
    feed.release_all();
    feed.end();
    let rig = Rig::new(
        feed.clone(),
        Options {
            shape,
            sync: true,
            ..Options::default()
        },
    );
    rig.play();
    rig.wait_until(LIMIT, "the first second of video", |rig| {
        rig.video_stats().lock().unwrap().count() >= 25
    });
    for (target, expected) in [
        (5_000_000_000u64, Colour::Blue),
        (500_000_000, Colour::Red),
        (7_900_000_000, Colour::Blue),
    ] {
        let outcome = seek_and_measure(&rig, target, true, Duration::from_secs(10));
        eprintln!("{shape:?} seek to {} ms: {outcome:?}", target / 1_000_000);
        assert_eq!(outcome.first_video_colour, Some(expected), "{outcome:?}");
        assert!(
            outcome.first_video_pts.unwrap() + 100_000_000 >= target,
            "{outcome:?}"
        );
        assert!(
            outcome.first_video.unwrap() < Duration::from_millis(500),
            "{outcome:?}"
        );
        assert_eq!(outcome.async_done, 1, "one seek-done per seek: {outcome:?}");
    }
    let seeks = feed.seeks();
    eprintln!("feed seeks: {seeks:?}");
    // Once per seek; `decodebin3` of the bare shape also seeks to the start when it selects its streams.
    assert!(
        seeks.len() == 3 || (seeks.len() == 4 && seeks[0] == 0),
        "the feed is seeked once per seek: {seeks:?}"
    );
    assert_eq!(
        seeks[seeks.len() - 3..],
        [5_000_000_000, 500_000_000, 7_900_000_000]
    );
}

#[test]
fn d_bare_seek_in_buffer() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "decodebin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("d-bare", 90, || seeks_in_buffer(Shape::Bare));
}

#[test]
fn d_playbin_seek_in_buffer() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("d-playbin", 90, || seeks_in_buffer(Shape::PlayBin));
}

// ---- (e) a quality switch inside one source buffer ----

/// Low rendition for the segments in `low_segments` (2 s each), high for the others.
fn switching_feed(low_segments: &[usize]) -> std::sync::Arc<FakeFeed> {
    use mercury_mse::TrackKind;
    let low = common::frames::track("h264-low", TrackKind::Video);
    let high = common::frames::track("h264-high", TrackKind::Video);
    let audio = common::frames::track("aac", TrackKind::Audio);
    let feed = FakeFeed::new();
    let frames = low
        .frames
        .iter()
        .zip(high.frames.iter())
        .map(|(l, h)| {
            let segment = (l.pts / 2_000_000_000) as usize;
            if low_segments.contains(&segment) {
                (0, l.clone())
            } else {
                (1, h.clone())
            }
        })
        .collect();
    feed.add_track_with(
        TrackKind::Video,
        vec![low.config.clone(), high.config.clone()],
        frames,
        true,
    );
    feed.add_track(&audio);
    feed.set_duration(Some(8_000_000_000));
    feed.set_initialized(true);
    feed
}

fn plays_switch(shape: Shape, low_segments: &[usize], switches: usize) {
    use common::scenario::max_pts_gap;
    let feed = switching_feed(low_segments);
    feed.release_all();
    feed.end();
    let rig = Rig::new(
        feed,
        Options {
            shape,
            ..Options::default()
        },
    );
    rig.play();
    rig.wait_eos(LIMIT);
    let video = rig.video_stats();
    let video = video.lock().unwrap();
    eprintln!("{shape:?} switches {switches}: caps {:?}", video.caps);
    assert!(
        (198..=202).contains(&video.count()),
        "frames {}",
        video.count()
    );
    assert_eq!(
        video.caps.len(),
        switches + 1,
        "caps change once per switch: {:?}",
        video.caps
    );
    assert!(
        max_pts_gap(&video, 0) <= 41_000_000,
        "gap {}",
        max_pts_gap(&video, 0)
    );
}

#[test]
fn e_playbin_quality_switch_up() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("e-up", 60, || plays_switch(Shape::PlayBin, &[0, 1], 1));
}

#[test]
fn e_bare_quality_switch_alternate() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "decodebin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("e-alt", 60, || plays_switch(Shape::Bare, &[0, 2], 3));
}

// ---- B-frames through the fake feed (frames from qtdemux): is a gap the element's or the backend's? ----

#[test]
fn f_playbin_b_frames_from_qtdemux_play_without_gaps() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("f-bframes", 60, || {
        let feed = FakeFeed::from_fixtures(&["h264-bframes", "aac"]);
        feed.release_all();
        feed.end();
        let rig = Rig::new(feed, Options::default());
        rig.play();
        rig.wait_eos(LIMIT);
        let video = rig.video_stats();
        let video = video.lock().unwrap();
        let gaps = common::scenario::gaps_over(&video, 41_000_000);
        eprintln!(
            "B-frames via qtdemux frames: {} frames, first pts {:?}, gaps over one frame {gaps:?}",
            video.count(),
            video.arrivals.first().map(|a| a.pts)
        );
        assert!(
            (198..=202).contains(&video.count()),
            "frames {}",
            video.count()
        );
        assert!(gaps.is_empty(), "{gaps:?}");
    });
}
