// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! "Append everything, end the stream, play to the end": the play-through that C02 (split A/V), C03
//! (muxed), C04 (arbitrary chunking) and C05 (WebM) share, and the one list of checks they are judged by
//! (the brief's C02 checks):
//!
//! * 200 +- 2 video frames and the audio expected for 8 s reach the sinks;
//! * no `Error`/`Abort` event; per append the order is `UpdateStart -> Update -> UpdateEnd`;
//! * `buffered` is [0, 8 s] on every source buffer;
//! * the element's duration is 8 s after the init segments (when an init segment declares a duration:
//!   see `initinfo`) and when the stream has ended;
//! * `ElementReadyState` reaches `HaveEnoughData`;
//! * the mean colour is red in the first half and blue in the second;
//! * the end of the stream is reported.

use std::time::{Duration, Instant};

use mse_rig::ClockTime;
use mse_rig::backend::{EosError, TimeRange};
use mse_rig::events::{ElementReadyState, MseEvent, SourceBufferId, TimedEvent};
use mse_rig::frames::FrameStats;
use mse_rig::harness::Ctx;
use mse_rig::rig::RigConfig;
use mse_segments::SegmentedStream;

use crate::initinfo;
use crate::probes::keep_going;
use crate::support::{
    self, Abort, Chunking, Feed, LOOSE_TOL_MS, NOMINAL_FRAMES, NOMINAL_MS, Res, Scn, astats,
    compare_ranges, errors_in, event_order_problems, fast_config, fmt_opt, fmt_ranges, ms_of,
    nominal_colour, t_ms, vstats,
};

/// What to play and how.
pub struct PlaySpec<'a> {
    /// Prefix of the metric and check names, e.g. `c02` or `plan.fixed-1024`.
    pub label: String,
    /// One source buffer per entry, in the order they are added.
    pub streams: Vec<(&'a SegmentedStream, Chunking)>,
    pub config: RigConfig,
    pub expect_video: bool,
    pub expect_audio: bool,
}

/// Everything a play-through observed, captured before the rig is shut down.
pub struct PlayResult {
    pub video: FrameStats,
    pub audio: FrameStats,
    /// `buffered` once it had settled (or after 5 s), per source buffer.
    pub buffered: Vec<(SourceBufferId, Vec<TimeRange>)>,
    /// `buffered` read right after the last append, which is what a page sees in its `updateend` handler.
    pub buffered_immediate: Vec<(SourceBufferId, Vec<TimeRange>)>,
    /// How long `buffered` took to show all the appended data.
    pub buffered_lag: Vec<(SourceBufferId, Duration)>,
    pub ids: Vec<SourceBufferId>,
    /// Whether the element exposed the tracks of the streams after the appends (the split A/V deadlock does not).
    pub tracks: support::TrackWait,
    pub duration_after_init: Option<ClockTime>,
    pub backend_duration_after_init: Option<ClockTime>,
    pub duration_after_media: Option<ClockTime>,
    pub duration_final: Option<ClockTime>,
    pub init_declares_duration: bool,
    pub appends: usize,
    pub append_time: Duration,
    pub play_time: Duration,
    pub eos_reached: bool,
    pub events: Vec<TimedEvent>,
    /// Factory names of the pipeline's elements: which decoder was chosen.
    pub pipeline: Vec<String>,
    /// A diagnostic paragraph when the end of the stream was not reached.
    pub diagnostics: Option<String>,
}

/// Runs one play-through in a fresh rig and shuts the rig down. Never judges: see [`judge`].
pub fn full_play(scn: &mut Scn, spec: &PlaySpec) -> Res<PlayResult> {
    let rig = support::new_rig(scn, spec.config.clone())?;
    eprintln!("[{}] rig is up (SourceOpen)", spec.label);
    let mut feeds = Vec::new();
    for (stream, chunking) in &spec.streams {
        let id = support::add_source_buffer(&rig, stream)?;
        feeds.push(Feed {
            id,
            stream,
            chunking: *chunking,
        });
    }
    let ids: Vec<SourceBufferId> = feeds.iter().map(|feed| feed.id).collect();

    let started = Instant::now();
    eprintln!(
        "[{}] {} source buffer(s) added; appending the init segments",
        spec.label,
        feeds.len()
    );
    let mut appends = support::append_inits(&rig, &feeds)?;
    eprintln!(
        "[{}] init segments appended; appending the media segments",
        spec.label
    );
    // The duration notification may trail the init segment's UpdateEnd; a stream whose init segment says
    // nothing never produces one, which costs this wait.
    rig.wait_until(Duration::from_millis(300), |r| {
        r.element_duration().is_some()
    });
    let duration_after_init = rig.element_duration();
    let backend_duration_after_init = rig.backend.duration();

    let segment_count = feeds
        .iter()
        .map(|feed| feed.stream.media.len())
        .max()
        .unwrap_or(0);
    appends += support::append_media(&rig, &feeds, 0..segment_count)?;
    let append_time = started.elapsed();
    eprintln!(
        "[{}] media appended in {:?}; ending the stream and playing",
        spec.label, append_time
    );
    let duration_after_media = rig.element_duration();

    // GstMSE 1.28.9 deadlocks when a MediaSource has two source buffers: the element then never gets its
    // tracks. Look for them for a moment, and do not try to play what has no pads.
    let (want_video, want_audio) = expected_tracks(spec);
    let tracks = support::wait_tracks(&rig, want_video, want_audio, Duration::from_secs(3));
    let mut buffered = Vec::new();
    let mut buffered_immediate = Vec::new();
    let mut buffered_lag = Vec::new();
    for feed in &feeds {
        if tracks.reached {
            let wait = support::wait_buffered(
                &rig,
                feed.id,
                Duration::from_secs(5),
                support::matches_ranges(vec![(0, NOMINAL_MS)], LOOSE_TOL_MS),
            )?;
            buffered_lag.push((feed.id, wait.elapsed));
            buffered_immediate.push((feed.id, wait.immediate));
            buffered.push((feed.id, wait.settled));
        } else {
            let now = support::buffered(&rig, feed.id)?;
            buffered_lag.push((feed.id, Duration::ZERO));
            buffered_immediate.push((feed.id, now.clone()));
            buffered.push((feed.id, now));
        }
    }

    let eos = if tracks.reached {
        rig.backend.end_of_stream(EosError::None)?;
        support::play_to_end(&rig, Duration::from_secs(30))?
    } else {
        eprintln!(
            "[{}] the element has no tracks after the appends: {tracks:?}; not playing",
            spec.label
        );
        support::EosWait {
            reached: false,
            took: Duration::ZERO,
        }
    };
    let play_time = eos.took;
    let duration_final = rig.element_duration();
    let diagnostics = (!eos.reached).then(|| support::diagnostics(&rig));
    eprintln!(
        "[{}] end of stream reached: {} after {:?}; {}",
        spec.label,
        eos.reached,
        eos.took,
        diagnostics.as_deref().unwrap_or("")
    );
    if !eos.reached {
        eprintln!(
            "[{}] event timeline: {}",
            spec.label,
            support::timeline(&rig, 80)
        );
    }
    let pipeline = rig.describe_pipeline();
    let result = PlayResult {
        video: vstats(&rig),
        audio: astats(&rig),
        buffered,
        buffered_immediate,
        buffered_lag,
        ids,
        tracks,
        duration_after_init,
        backend_duration_after_init,
        duration_after_media,
        duration_final,
        init_declares_duration: feeds.iter().any(|feed| {
            initinfo::declares_nominal(
                feed.stream.container,
                &feed.stream.init.bytes,
                Duration::from_millis(NOMINAL_MS.unsigned_abs()),
            )
        }),
        appends,
        append_time,
        play_time,
        eos_reached: eos.reached,
        events: rig.log.snapshot(),
        pipeline,
        diagnostics,
    };
    eprintln!("[{}] shutting down", spec.label);
    support::shutdown(scn, &spec.label, rig);
    Ok(result)
}

/// The numbers of video and audio tracks the streams of `spec` hold, from their codec strings.
pub fn expected_tracks(spec: &PlaySpec) -> (u32, u32) {
    let (mut video, mut audio) = (0, 0);
    for (stream, _) in &spec.streams {
        for codec in crate::scenarios::c01::parse_codecs(&stream.mime) {
            match crate::scenarios::c01::codec_caps(&codec) {
                Some(caps) if caps.starts_with("video/") => video += 1,
                Some(caps) if caps.starts_with("audio/") => audio += 1,
                _ => {}
            }
        }
    }
    (video, audio)
}

fn within(value: u64, nominal: u64, tolerance: u64) -> bool {
    value.abs_diff(nominal) <= tolerance
}

/// Records every number of the play-through as metrics and judges it by the C02 checks.
pub fn judge(scn: &mut Scn, spec: &PlaySpec, result: &PlayResult) -> Res<()> {
    let label = &spec.label;
    let video = &result.video;
    let audio = &result.audio;

    scn.metric(&format!("{label}.video_frames"), video.count)
        .metric(&format!("{label}.video_bytes"), video.bytes)
        .metric(
            &format!("{label}.video_first_pts_ms"),
            video.first_pts.map(ms_of),
        )
        .metric(
            &format!("{label}.video_last_pts_ms"),
            video.last_pts.map(ms_of),
        )
        .metric(
            &format!("{label}.video_max_gap_ms"),
            video.max_gap.map(ms_of),
        )
        .metric(&format!("{label}.video_out_of_order"), video.out_of_order)
        .metric(
            &format!("{label}.video_caps_changes"),
            video
                .caps_changes
                .iter()
                .map(|change| change.caps.clone())
                .collect::<Vec<_>>(),
        )
        .metric(&format!("{label}.audio_buffers"), audio.count)
        .metric(&format!("{label}.audio_span_ms"), support::span_ms(audio))
        .metric(
            &format!("{label}.audio_max_gap_ms"),
            audio.max_gap.map(ms_of),
        )
        .metric(
            &format!("{label}.duration_after_init_ms"),
            result.duration_after_init.map(ms_of),
        )
        .metric(
            &format!("{label}.backend_duration_after_init_ms"),
            result.backend_duration_after_init.map(ms_of),
        )
        .metric(
            &format!("{label}.duration_after_media_ms"),
            result.duration_after_media.map(ms_of),
        )
        .metric(
            &format!("{label}.duration_final_ms"),
            result.duration_final.map(ms_of),
        )
        .metric(
            &format!("{label}.init_declares_duration"),
            result.init_declares_duration,
        )
        .metric(&format!("{label}.appends"), result.appends)
        .metric(
            &format!("{label}.append_ms"),
            support::dur_ms(result.append_time),
        )
        .metric(
            &format!("{label}.play_ms"),
            support::dur_ms(result.play_time),
        )
        .metric(&format!("{label}.pipeline"), result.pipeline.clone());
    for (id, ranges) in &result.buffered {
        scn.metric(
            &format!("{label}.buffered.{id}"),
            support::ranges_json(ranges),
        );
    }

    // 0. the element exposes the tracks (the split A/V deadlock of GstMSE 1.28.9 does not)
    let (want_video, want_audio) = expected_tracks(spec);
    scn.metric(
        &format!("{label}.tracks_expected"),
        format!("{want_video} video, {want_audio} audio"),
    )
    .metric(
        &format!("{label}.tracks_seen"),
        format!(
            "{:?} after {:?}, element ready state {:?}",
            result.tracks.counts, result.tracks.waited, result.tracks.ready
        ),
    );
    if !scn.check(
        &format!("{label}.tracks-exposed"),
        result.tracks.reached,
        format!(
            "after appending the init and media segments of {} source buffer(s) the element exposes {:?} (video, audio) tracks and is {:?} 3 s later, expected ({want_video}, {want_audio}) and HaveMetadata or more: GstMSE never created the pads; buffered is {}; {}",
            spec.streams.len(),
            result.tracks.counts,
            result.tracks.ready,
            result
                .buffered
                .iter()
                .map(|(id, ranges)| format!("source buffer {id}: {}", fmt_ranges(ranges)))
                .collect::<Vec<_>>()
                .join(", "),
            result.diagnostics.as_deref().unwrap_or("")
        ),
    ) {
        return Ok(());
    }

    // 1. the end of the stream arrived
    let eos_ok = scn.check(
        &format!("{label}.eos"),
        result.eos_reached,
        format!(
            "no EndOfStream within 30 s of play(); {}",
            result.diagnostics.as_deref().unwrap_or("")
        ),
    );

    // 2. frames and audio
    if spec.expect_video {
        scn.check(
            &format!("{label}.video-frames"),
            within(video.count, NOMINAL_FRAMES, 2),
            format!(
                "{} video frames reached the sink, expected {NOMINAL_FRAMES} +- 2 (first pts {}, last pts {})",
                video.count,
                fmt_opt(video.first_pts),
                fmt_opt(video.last_pts)
            ),
        );
        scn.check(
            &format!("{label}.video-order"),
            video.out_of_order == 0,
            format!(
                "{} video buffers went backwards in time without a flush",
                video.out_of_order
            ),
        );
    }
    if spec.expect_audio {
        let span = support::span_ms(audio);
        scn.check(
            &format!("{label}.audio-span"),
            span >= 7_800 && audio.count > 0,
            format!(
                "audio covers {span} ms ({} buffers, first pts {}, last pts {}), expected at least 7800 ms of the 8 s",
                audio.count,
                fmt_opt(audio.first_pts),
                fmt_opt(audio.last_pts)
            ),
        );
        scn.check(
            &format!("{label}.audio-gaps"),
            audio.max_gap.is_none_or(|gap| ms_of(gap) <= 100),
            format!(
                "the largest gap between audio buffers is {} ms (limit 100 ms)",
                audio.max_gap.map_or(0, ms_of)
            ),
        );
    }

    // 3. no errors; the order of the update events
    let errors = errors_in(&result.events);
    scn.check(
        &format!("{label}.no-errors"),
        errors.is_empty(),
        format!("error or abort events: {}", errors.join("; ")),
    );
    let order = event_order_problems(&result.events, &result.ids, false);
    scn.check(
        &format!("{label}.event-order"),
        order.is_empty(),
        order.join("; "),
    );

    // 4. buffered: what a page sees at `updateend`, and once GstMSE has caught up
    for ((id, immediate), (_, lag)) in result.buffered_immediate.iter().zip(&result.buffered_lag) {
        scn.metric(
            &format!("{label}.buffered_immediate.{id}"),
            support::ranges_json(immediate),
        )
        .metric(
            &format!("{label}.buffered_lag_ms.{id}"),
            support::dur_ms(*lag),
        );
        let deviations = compare_ranges(
            &format!("buffered of source buffer {id} at the last UpdateEnd"),
            immediate,
            &[(0, NOMINAL_MS)],
            LOOSE_TOL_MS,
        );
        scn.check(
            &format!("{label}.buffered-at-updateend.{id}"),
            deviations.is_empty(),
            format!(
                "right after the last append finished buffered was {} (it should already show [0.000-8.000]); it showed all the data {} ms later: {}",
                fmt_ranges(immediate),
                support::dur_ms(*lag),
                deviations.join("; ")
            ),
        );
    }
    for (id, ranges) in &result.buffered {
        let deviations = compare_ranges(
            &format!("buffered of source buffer {id}"),
            ranges,
            &[(0, NOMINAL_MS)],
            LOOSE_TOL_MS,
        );
        scn.check(
            &format!("{label}.buffered.{id}"),
            deviations.is_empty(),
            format!(
                "{} (expected [0.000-8.000] within {LOOSE_TOL_MS} ms)",
                if deviations.is_empty() {
                    fmt_ranges(ranges)
                } else {
                    deviations.join("; ")
                }
            ),
        );
    }

    // 5. duration
    if result.init_declares_duration {
        scn.check(
            &format!("{label}.duration-after-init"),
            result
                .duration_after_init
                .is_some_and(|d| (ms_of(d) - NOMINAL_MS).abs() <= LOOSE_TOL_MS),
            format!(
                "the init segment declares 8 s, the element's duration after the init segments is {} (MediaSource duration {})",
                fmt_opt(result.duration_after_init),
                fmt_opt(result.backend_duration_after_init)
            ),
        );
    } else {
        scn.note(format!(
            "{label}: the init segment declares no duration (mvhd 0 / no Duration), so the duration after the init segments is unknown by the spec: element {}, MediaSource {} (recorded, not a check)",
            fmt_opt(result.duration_after_init),
            fmt_opt(result.backend_duration_after_init)
        ));
    }
    if eos_ok {
        scn.check(
            &format!("{label}.duration-final"),
            result
                .duration_final
                .is_some_and(|d| (ms_of(d) - NOMINAL_MS).abs() <= LOOSE_TOL_MS),
            format!(
                "the element's duration at the end of the stream is {} (all of 8 s was appended and the stream ended); after the media appends and before the end it was {}",
                fmt_opt(result.duration_final),
                fmt_opt(result.duration_after_media)
            ),
        );
    }

    // 6. ready state
    scn.check(
        &format!("{label}.ready-state"),
        result.events.iter().any(|timed| {
            matches!(
                timed.event,
                MseEvent::ElementReadyState(ElementReadyState::HaveEnoughData)
            )
        }),
        format!(
            "ElementReadyState never reached HaveEnoughData; it went {}",
            support::fmt_ready_states(&support::ready_state_events(&result.events, 0))
        ),
    );

    // 7. colours
    if spec.expect_video {
        let verdict = support::judge_colours(&video.colour, nominal_colour, t_ms(40));
        scn.metric(&format!("{label}.colour"), verdict.summary());
        let before = video
            .colour
            .iter()
            .filter(|sample| ms_of(sample.pts) < 3_900)
            .count();
        let after = video
            .colour
            .iter()
            .filter(|sample| ms_of(sample.pts) > 4_100)
            .count();
        if (before < 2 || after < 2) && within(video.count, NOMINAL_FRAMES, 2) {
            return Err(Abort::Error(format!(
                "{label}: the video sink recorded {before} colour samples in the first half and {after} in the second although all frames arrived; the rig's colour sampling is broken"
            )));
        }
        if before >= 2 && after >= 2 {
            scn.check(
                &format!("{label}.colour"),
                verdict.ok(),
                format!("{}: {}", verdict.summary(), verdict.wrong.join("; ")),
            );
        }
    }
    Ok(())
}

/// Split A/V: video and audio in two source buffers of one `MediaSource`, which is how DASH delivers them
/// (C02 and C05). GstMSE 1.28.9 deadlocks inside `gst_mse_src_emit_streams` for this almost every time (one
/// lock is held by the first track's pad task while the second track's init segment waits for it), so the
/// play-through is tried `attempts` times, each one judged by the C02 checks, and then each half is played
/// alone as a control: when the controls pass, the failure is the combination of two source buffers, not
/// the codecs, the container or the rig.
pub fn split_av(
    ctx: &Ctx,
    scn: &mut Scn,
    prefix: &str,
    video: &SegmentedStream,
    audio: &SegmentedStream,
    attempts: usize,
) -> Res<()> {
    let mut deadlocked = 0usize;
    for attempt in 1..=attempts {
        let spec = PlaySpec {
            label: format!("{prefix}.attempt{attempt}"),
            streams: vec![(video, Chunking::Whole), (audio, Chunking::Whole)],
            config: fast_config(ctx),
            expect_video: true,
            expect_audio: true,
        };
        let outcome = full_play(scn, &spec);
        let Some(result) = keep_going(scn, &spec.label, outcome)? else {
            deadlocked += 1;
            continue;
        };
        if !result.tracks.reached {
            deadlocked += 1;
        }
        judge(scn, &spec, &result)?;
    }
    scn.metric(&format!("{prefix}.split_attempts"), attempts)
        .metric(&format!("{prefix}.split_deadlocked"), deadlocked);
    scn.note(format!(
        "{prefix}: with two source buffers (video + audio) {deadlocked} of {attempts} attempt(s) never exposed the element's tracks"
    ));

    for (name, stream, expect_video) in [("video", video, true), ("audio", audio, false)] {
        let spec = PlaySpec {
            label: format!("{prefix}.control-{name}-only"),
            streams: vec![(stream, Chunking::Whole)],
            config: fast_config(ctx),
            expect_video,
            expect_audio: !expect_video,
        };
        let outcome = full_play(scn, &spec);
        if let Some(result) = keep_going(scn, &spec.label, outcome)? {
            judge(scn, &spec, &result)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mse_rig::events::PlayEvent;
    use mse_rig::frames::ColourSample;
    use mse_rig::report::Status;

    fn timed(at_ms: u64, event: MseEvent) -> TimedEvent {
        TimedEvent {
            at: Duration::from_millis(at_ms),
            thread: "gst".to_owned(),
            event,
        }
    }

    fn spec() -> PlaySpec<'static> {
        PlaySpec {
            label: "t".to_owned(),
            streams: Vec::new(),
            config: RigConfig::default(),
            expect_video: true,
            expect_audio: true,
        }
    }

    /// A play-through that did everything right: 200 frames, 8 s of audio, [0, 8 s] buffered on two
    /// source buffers, clean events, red then blue.
    fn good() -> PlayResult {
        let colour = (0..8u64)
            .map(|second| ColourSample {
                pts: t_ms(second * 1_000),
                rgb: if second < 4 {
                    [236, 10, 12]
                } else {
                    [10, 12, 236]
                },
            })
            .collect();
        let video = FrameStats {
            count: 200,
            first_pts: Some(t_ms(0)),
            last_pts: Some(t_ms(7_960)),
            max_gap: Some(t_ms(40)),
            colour,
            ..FrameStats::default()
        };
        let audio = FrameStats {
            count: 375,
            first_pts: Some(t_ms(0)),
            last_pts: Some(t_ms(7_979)),
            max_gap: Some(t_ms(22)),
            ..FrameStats::default()
        };
        let mut events = Vec::new();
        for id in [0usize, 1] {
            for cycle in 0..5u64 {
                let at = cycle * 10 + id as u64;
                events.push(timed(at, MseEvent::UpdateStart(id)));
                events.push(timed(at + 1, MseEvent::Update(id)));
                events.push(timed(at + 2, MseEvent::UpdateEnd(id)));
            }
        }
        events.push(timed(
            100,
            MseEvent::ElementReadyState(ElementReadyState::HaveEnoughData),
        ));
        events.push(timed(900, MseEvent::Play(PlayEvent::EndOfStream)));
        let range = vec![TimeRange {
            start: t_ms(0),
            end: t_ms(8_000),
        }];
        PlayResult {
            video,
            audio,
            buffered: vec![(0, range.clone()), (1, range.clone())],
            buffered_immediate: vec![(0, range.clone()), (1, range)],
            buffered_lag: vec![(0, Duration::ZERO), (1, Duration::ZERO)],
            ids: vec![0, 1],
            tracks: support::TrackWait {
                reached: true,
                counts: Some((1, 1)),
                ready: ElementReadyState::HaveEnoughData,
                waited: Duration::from_millis(10),
            },
            duration_after_init: None,
            backend_duration_after_init: None,
            duration_after_media: Some(t_ms(8_000)),
            duration_final: Some(t_ms(8_000)),
            init_declares_duration: false,
            appends: 10,
            append_time: Duration::from_millis(50),
            play_time: Duration::from_millis(800),
            eos_reached: true,
            events,
            pipeline: vec!["playbin3:playbin3".to_owned()],
            diagnostics: None,
        }
    }

    fn verdict(result: &PlayResult) -> (Status, Vec<String>) {
        let mut scn = Scn::for_tests();
        let outcome = judge(&mut scn, &spec(), result);
        let finished = scn.finish(outcome);
        (finished.status, finished.notes)
    }

    #[test]
    fn a_correct_play_through_passes_every_check() {
        let (status, notes) = verdict(&good());
        assert_eq!(status, Status::Pass, "{notes:?}");
    }

    #[test]
    fn a_deadlocked_split_av_is_one_finding_not_a_pile_of_failed_checks() {
        let mut stuck = good();
        stuck.tracks.reached = false;
        stuck.tracks.counts = Some((0, 0));
        stuck.tracks.ready = ElementReadyState::HaveNothing;
        stuck.video.count = 0;
        stuck.eos_reached = false;
        stuck.buffered = vec![(0, Vec::new()), (1, Vec::new())];
        stuck.buffered_immediate = stuck.buffered.clone();
        let mut scn = Scn::for_tests();
        let outcome = judge(&mut scn, &spec(), &stuck);
        let result = scn.finish(outcome);
        assert_eq!(result.status, Status::Fail);
        let failed: Vec<&String> = result
            .notes
            .iter()
            .filter(|n| n.starts_with("FAIL "))
            .collect();
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(failed[0].starts_with("FAIL t.tracks-exposed"));
        assert!(failed[0].contains("Some((0, 0))"));
    }

    #[test]
    fn missing_frames_a_late_end_and_a_wrong_picture_each_fail_their_own_check() {
        let mut few = good();
        few.video.count = 150;
        let (status, notes) = verdict(&few);
        assert_eq!(status, Status::Fail);
        assert!(
            notes.iter().any(|n| n.starts_with("FAIL t.video-frames")),
            "{notes:?}"
        );

        let mut no_eos = good();
        no_eos.eos_reached = false;
        no_eos.events.pop();
        let (status, notes) = verdict(&no_eos);
        assert_eq!(status, Status::Fail);
        assert!(
            notes.iter().any(|n| n.starts_with("FAIL t.eos")),
            "{notes:?}"
        );

        let mut wrong = good();
        wrong.video.colour[6].rgb = [236, 10, 12];
        let (status, notes) = verdict(&wrong);
        assert_eq!(status, Status::Fail);
        assert!(
            notes.iter().any(|n| n.starts_with("FAIL t.colour")),
            "{notes:?}"
        );
    }

    #[test]
    fn events_and_buffered_ranges_are_judged() {
        let mut errored = good();
        errored.events.push(timed(500, MseEvent::Error(1)));
        let (_, notes) = verdict(&errored);
        assert!(
            notes.iter().any(|n| n.starts_with("FAIL t.no-errors")),
            "{notes:?}"
        );

        let mut short = good();
        short.buffered[1].1 = vec![TimeRange {
            start: t_ms(0),
            end: t_ms(6_000),
        }];
        short.buffered_immediate[1].1 = short.buffered[1].1.clone();
        let (_, notes) = verdict(&short);
        assert!(
            notes.iter().any(|n| n.starts_with("FAIL t.buffered.1")),
            "{notes:?}"
        );

        let mut reordered = good();
        reordered.events.insert(0, timed(0, MseEvent::UpdateEnd(0)));
        let (_, notes) = verdict(&reordered);
        assert!(
            notes.iter().any(|n| n.starts_with("FAIL t.event-order")),
            "{notes:?}"
        );

        let mut no_ready = good();
        no_ready
            .events
            .retain(|e| !matches!(e.event, MseEvent::ElementReadyState(_)));
        let (_, notes) = verdict(&no_ready);
        assert!(
            notes.iter().any(|n| n.starts_with("FAIL t.ready-state")),
            "{notes:?}"
        );
    }

    #[test]
    fn the_duration_check_applies_only_when_the_init_segment_declares_one() {
        let mut undeclared = good();
        undeclared.duration_after_init = None;
        assert_eq!(verdict(&undeclared).0, Status::Pass);

        let mut declared = good();
        declared.init_declares_duration = true;
        declared.duration_after_init = None;
        let (status, notes) = verdict(&declared);
        assert_eq!(status, Status::Fail);
        assert!(
            notes
                .iter()
                .any(|n| n.starts_with("FAIL t.duration-after-init")),
            "{notes:?}"
        );

        declared.duration_after_init = Some(t_ms(8_000));
        assert_eq!(verdict(&declared).0, Status::Pass);

        let mut wrong_final = good();
        wrong_final.duration_final = Some(t_ms(7_000));
        let (_, notes) = verdict(&wrong_final);
        assert!(
            notes.iter().any(|n| n.starts_with("FAIL t.duration-final")),
            "{notes:?}"
        );
    }

    #[test]
    fn a_sink_that_samples_no_colours_although_all_frames_arrived_is_a_harness_error() {
        let mut broken = good();
        broken.video.colour.clear();
        let (status, notes) = verdict(&broken);
        assert_eq!(status, Status::Error, "{notes:?}");
    }
}
