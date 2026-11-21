// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C14: which pipeline message or feed event drives which `HTMLMediaElement` event (R17 section 12), as a
//! table in the test output, measured on a real `MseBackend` feeding `playbin3` through `MseSrc`.
//!
//! The rule of the lane (D2, R17 section 12): `playing`, `seeked`, `timeupdate`, `ended` come from the
//! pipeline (GstPlay turns the bus messages below into them); `durationchange` comes from the model
//! (`MseEvent::DurationChanged`) and is confirmed by the pipeline's `duration-changed`; `loadedmetadata`,
//! `loadeddata`, `canplay`, `canplaythrough`, `waiting`, `stalled`, `seeking` and `readyState` are computed
//! by the model from `buffered` and the position, never from the pipeline.

mod common;

use std::time::Duration;

use common::real::{RealSource, load};
use common::rig::{Options, Rig, Shape, watchdog};
use common::scenario::seek_and_measure;
use mercury_mse::{ElementReadyState, MediaSourceBackend, MseEvent};
use mercury_mse_testkit::ChunkPlan;

fn ms(d: Duration) -> String {
    format!("{:.0}", d.as_secs_f64() * 1000.0)
}

#[test]
fn c14_the_events_of_the_element_and_what_drives_them() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c14", 120, || {
        let source = RealSource::new();
        let low = load("h264-low");
        let high = load("h264-high");
        let audio = load("aac");
        let video_id = source.add(&low.mime);
        let audio_id = source.add(&audio.mime);
        let rig = Rig::new(
            source.feed(),
            Options {
                shape: Shape::PlayBin,
                sync: true,
                ..Options::default()
            },
        );
        rig.play();
        // The model's own view at each step (what the DOM computes loadedmetadata, canplay, ... from).
        let mut model: Vec<(Duration, String)> = Vec::new();
        let ready = |source: &RealSource, at: f64| source.backend.element_ready_state(at);
        model.push((
            rig.elapsed(),
            format!("{:?} before any data", ready(&source, 0.0)),
        ));
        source.append_init(video_id, &low);
        source.append_init(audio_id, &audio);
        model.push((
            rig.elapsed(),
            format!("{:?} after the init segments", ready(&source, 0.0)),
        ));
        for index in 0..2 {
            source.append_segment(video_id, &low, index);
            source.append_segment(audio_id, &audio, index);
        }
        model.push((
            rig.elapsed(),
            format!("{:?} with 4 s buffered", ready(&source, 0.0)),
        ));
        assert!(ready(&source, 0.0) >= ElementReadyState::HaveEnoughData);
        // A rendition switch (the video size changes: `resize`).
        source.append_init(video_id, &high);
        for index in 2..4 {
            source.append_segment(video_id, &high, index);
        }
        for index in 2..4 {
            source.append_segment(audio_id, &audio, index);
        }
        // A longer duration, then one more wake: `durationchange`.
        let duration_set_at = rig.elapsed();
        source
            .backend
            .set_duration(Some(9.0))
            .expect("a longer duration");
        let video_frames = |rig: &Rig| rig.video_stats().lock().unwrap().count();
        rig.wait_until(Duration::from_secs(10), "a second of playback", |rig| {
            video_frames(rig) >= 25
        });
        let seek = seek_and_measure(&rig, 5_000_000_000, true, Duration::from_secs(10));
        let positions: Vec<u64> = (0..4)
            .map(|_| {
                std::thread::sleep(Duration::from_millis(400));
                rig.position().map_or(0, |p| p.nseconds())
            })
            .collect();
        source.end_of_stream();
        rig.wait_eos(Duration::from_secs(30));
        rig.pump_bus();

        let log = rig.bus_log.lock().unwrap();
        let first = |needle: &str| {
            log.log
                .iter()
                .find(|(_, line)| line.contains(needle))
                .map(|(at, _)| *at)
        };
        let video = rig.video_stats();
        let video = video.lock().unwrap();
        let first_frame = video.arrivals.first().map(|a| a.at);
        let resize = video
            .arrivals
            .windows(2)
            .find(|w| w[0].caps != w[1].caps)
            .map(|w| w[1].at);
        let events = source.events();
        let duration_event = events
            .iter()
            .any(|e| matches!(e, MseEvent::DurationChanged(Some(d)) if (*d - 9.0).abs() < 1e-6));
        let pipeline_duration_changes = log.duration_changed;

        let row = |event: &str, driver: &str, at: Option<Duration>| {
            eprintln!(
                "C14 | {event:<16} | {driver:<62} | {} ms",
                at.map_or("-".to_owned(), ms)
            );
        };
        eprintln!(
            "C14 | HTMLMediaElement | driven by                                                       | first seen"
        );
        row(
            "loadedmetadata",
            "model: InitSegment event, element_ready_state >= HaveMetadata",
            model.get(1).map(|m| m.0),
        );
        row(
            "loadeddata",
            "model: element_ready_state >= HaveCurrentData at the position",
            model.get(2).map(|m| m.0),
        );
        row(
            "canplay",
            "model: HaveFutureData (buffered ahead of the position)",
            model.get(2).map(|m| m.0),
        );
        row(
            "canplaythrough",
            "model: HaveEnoughData (3 s ahead, or ended)",
            model.get(2).map(|m| m.0),
        );
        row(
            "waiting",
            "model: position outside buffered (never the pipeline)",
            None,
        );
        row(
            "stalled",
            "page's fetches (not the pipeline, not the model)",
            None,
        );
        row(
            "seeking",
            "DOM: currentTime assigned; backend.seeking(t)",
            None,
        );
        row(
            "playing",
            "pipeline: state-changed -> Playing (GstPlay state)",
            first("-> Playing"),
        );
        row(
            "(first frame)",
            "feed: first FeedItem::Frame reaches the sink",
            first_frame,
        );
        row(
            "resize",
            "feed: FeedItem::Config -> caps change at the video sink",
            resize,
        );
        row(
            "durationchange",
            "model: MseEvent::DurationChanged; pipeline: duration-changed",
            first("duration-changed"),
        );
        row(
            "seeked",
            "pipeline: async-done after the flushing seek (GstPlay seek-done)",
            None,
        );
        eprintln!(
            "C14 |   seek to 5 s: first frame after the flush {:?}, async-done messages {}",
            seek.first_video, seek.async_done
        );
        eprintln!("C14 | timeupdate       | pipeline: position query every 400 ms: {positions:?}");
        row(
            "ended",
            "pipeline: EOS after FeedState::Ended and the last frame",
            first("eos"),
        );
        eprintln!("C14 | model readiness: {model:?}");
        eprintln!(
            "C14 | set_duration(9 s) at {} ms: MseEvent::DurationChanged seen {duration_event}, pipeline duration-changed messages {pipeline_duration_changes}",
            ms(duration_set_at)
        );

        assert!(
            first("-> Playing").is_some(),
            "playing comes from the pipeline"
        );
        assert_eq!(seek.async_done, 1, "seeked: one async-done per seek");
        assert!(
            resize.is_some(),
            "the rendition switch changed the caps at the sink"
        );
        assert!(first("eos").is_some(), "ended comes from the pipeline");
        assert!(duration_event, "durationchange comes from the model");
        assert!(
            log.buffering.is_empty(),
            "no buffering messages: {:?}",
            log.buffering
        );
        assert!(
            positions.windows(2).filter(|w| w[1] > w[0]).count() >= 2,
            "timeupdate: the position advances: {positions:?}"
        );
        let last = video.arrivals.last().and_then(|a| a.pts).unwrap_or(0);
        assert!(
            last > 7_900_000_000,
            "ended comes after the last frame (pts {last})"
        );
        let _ = ChunkPlan::Whole;
    });
}
