// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T07: decoder choice and speed. For H.264, VP9, AAC, Opus (and AV1 when the fixture exists): which
//! decoder `decodebin3` picks with the default ranks and with `prefer_decoder` set to each
//! alternative, frames per second and CPU time, whether VideoToolbox hardware decode produced frames
//! on this Mac, and which decoders are FFmpeg's (R16 P2: the commercial route must use only
//! `vtdec*` and `atdec` for H.264, HEVC and AAC).

use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::PluginFeatureExtManual;
use mse_rig::backend::EosError;
use mse_rig::events::{MseEvent, PlayEvent};
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::RigConfig;
use mse_segments::{SegmentedStream, tile};
use serde_json::{Value, json};

use crate::common::{
    APPEND_TIMEOUT, Findings, Verdict, audio_buffers, fast_config, new_rig, video_frames,
};
use crate::proc;
use crate::stats::ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Video,
    Audio,
}

pub struct Case {
    pub codec: &'static str,
    pub fixture: &'static str,
    pub kind: Kind,
    /// Decoder factories to try as `prefer_decoder` after the default run.
    pub alternatives: &'static [&'static str],
    /// A missing fixture skips the case (only AV1 may be missing).
    pub optional: bool,
    /// Whether the commercial route (R16 P2) may use only Apple decoders for this codec.
    pub apple_only_required: bool,
}

pub const CASES: &[Case] = &[
    Case {
        codec: "H.264",
        fixture: "h264-high",
        kind: Kind::Video,
        alternatives: &["vtdec_hw", "vtdec", "avdec_h264"],
        optional: false,
        apple_only_required: true,
    },
    Case {
        codec: "VP9",
        fixture: "vp9-high",
        kind: Kind::Video,
        alternatives: &["vp9dec", "avdec_vp9"],
        optional: false,
        apple_only_required: false,
    },
    Case {
        codec: "AAC",
        fixture: "aac",
        kind: Kind::Audio,
        alternatives: &["avdec_aac", "atdec"],
        optional: false,
        apple_only_required: true,
    },
    Case {
        codec: "Opus",
        fixture: "opus",
        kind: Kind::Audio,
        alternatives: &["opusdec"],
        optional: false,
        apple_only_required: false,
    },
    // The two AV1 fixtures differ only in their av1C record: `av1-low` has the hand-written four bytes with no
    // sequence header OBU, `av1-low-av1c` the full record (a skeptic asked for the control: VideoToolbox is
    // said to start an AV1 session only when the record carries the sequence header). The alternatives name
    // the three decoders that could take the stream: dav1d (software), VideoToolbox hardware-only and
    // VideoToolbox in general.
    Case {
        codec: "AV1",
        fixture: "av1-low",
        kind: Kind::Video,
        alternatives: AV1_ALTERNATIVES,
        optional: true,
        apple_only_required: false,
    },
    Case {
        codec: "AV1 (full av1C)",
        fixture: "av1-low-av1c",
        kind: Kind::Video,
        alternatives: AV1_ALTERNATIVES,
        optional: true,
        apple_only_required: false,
    },
];

const AV1_ALTERNATIVES: &[&str] = &["dav1ddec", "vtdec_hw", "vtdec"];

/// Every decoder factory named above, for the table of default ranks.
pub fn candidate_decoders() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = CASES
        .iter()
        .flat_map(|c| c.alternatives.iter().copied())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Who wrote the decoder behind a factory name (for the R16 P2 question).
pub fn decoder_family(factory: &str) -> &'static str {
    if factory.starts_with("avdec_") {
        "FFmpeg (gst-libav)"
    } else if factory.starts_with("vtdec") {
        "Apple VideoToolbox"
    } else if factory == "atdec" {
        "Apple AudioToolbox"
    } else if factory == "vp8dec" || factory == "vp9dec" {
        "libvpx"
    } else if factory == "dav1ddec" {
        "dav1d"
    } else if factory == "opusdec" {
        "libopus"
    } else if factory == "vorbisdec" {
        "libvorbis"
    } else if factory == "flacdec" {
        "libFLAC"
    } else {
        "other"
    }
}

/// `vtdec_hw0:vtdec_hw` -> `vtdec_hw`.
pub fn factory_of(description: &str) -> &str {
    description.rsplit(':').next().unwrap_or(description)
}

pub fn run(ctx: &Ctx) -> ScenarioResult {
    crate::common::run_isolated(
        "T07",
        "Decoder choice and speed",
        ctx,
        "MSE_STRESS_T07_NESTED",
        std::time::Duration::from_secs(230),
        body,
    )
}

#[derive(Debug, Clone)]
struct RunStats {
    decoders: Vec<String>,
    /// Video frames or audio buffers that reached the sink.
    delivered: u64,
    /// For video: the frames the stream holds. For audio: 0 (the count differs per codec).
    expected: u64,
    /// Video: `delivered + 2 >= expected`. Audio: the stream reached its end and the sink saw its whole span.
    complete: bool,
    wall_ms: f64,
    cpu_ms: u64,
    per_second: Option<f64>,
    errors: Vec<String>,
}

impl RunStats {
    fn complete(&self) -> bool {
        self.complete
    }

    fn to_json(&self, kind: Kind) -> Value {
        let mut object = serde_json::Map::new();
        object.insert("decoders".to_owned(), json!(self.decoders));
        object.insert(
            "families".to_owned(),
            json!(
                self.decoders
                    .iter()
                    .map(|d| decoder_family(factory_of(d)))
                    .collect::<Vec<_>>()
            ),
        );
        object.insert("delivered".to_owned(), json!(self.delivered));
        object.insert("expected".to_owned(), json!(self.expected));
        object.insert("complete".to_owned(), json!(self.complete));
        object.insert("wall_ms".to_owned(), json!(self.wall_ms));
        object.insert("cpu_ms".to_owned(), json!(self.cpu_ms));
        let rate_name = match kind {
            Kind::Video => "fps",
            Kind::Audio => "buffers_per_s",
        };
        object.insert(
            rate_name.to_owned(),
            json!(self.per_second.map(|v| (v * 10.0).round() / 10.0)),
        );
        object.insert("errors".to_owned(), json!(self.errors));
        Value::Object(object)
    }
}

/// A pipeline error (or a source buffer error) is in the log.
fn has_play_error(rig: &mse_rig::rig::Rig) -> bool {
    rig.log.count(|event| {
        matches!(
            event,
            MseEvent::Play(PlayEvent::Error(_)) | MseEvent::Error(_)
        )
    }) > 0
}

fn rank_of(name: &str) -> Option<gst::Rank> {
    gst::Registry::get()
        .lookup_feature(name)
        .map(|feature| feature.rank())
}

fn body(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let mut findings = Findings::new();
    // The default ranks of every candidate.
    let mut ranks = serde_json::Map::new();
    for name in candidate_decoders() {
        ranks.insert(
            name.to_owned(),
            match rank_of(name) {
                Some(rank) => json!(i32::from(rank)),
                None => json!("not in the registry"),
            },
        );
    }
    result.metric("default_ranks", Value::Object(ranks.clone()));
    result.note(format!("default ranks: {}", Value::Object(ranks)));

    let mut table = serde_json::Map::new();
    let mut vt_hw_produced_frames: Option<bool> = None;
    let mut ffmpeg_in_default_route = Vec::new();
    let mut av1_defaults: Vec<Av1Default> = Vec::new();
    for case in CASES {
        let stream = match mse_segments::load(case.fixture) {
            Ok(stream) => stream,
            Err(error) if case.optional => {
                result.note(format!(
                    "{}: fixture {} is not available ({error}): skipped",
                    case.codec, case.fixture
                ));
                table.insert(
                    case.codec.to_owned(),
                    json!({ "skipped": error.to_string() }),
                );
                continue;
            }
            Err(error) => return Err(format!("fixture {}: {error}", case.fixture)),
        };
        let copies = match case.kind {
            Kind::Video => 8,
            Kind::Audio => 20,
        };
        let tiled = tile(&stream, copies).map_err(|e| format!("tile {}: {e}", case.fixture))?;
        let mut entry = serde_json::Map::new();

        // Default ranks.
        let default = measure(ctx, &tiled, case.kind, None)?;
        result.note(format!(
            "{} default route: {:?}, {}, {}",
            case.codec,
            default.decoders,
            delivered_text(&default, case.kind),
            speed_text(&default, case.kind)
        ));
        if !default.complete() {
            findings.fail(format!(
                "{}: the default route delivered {} of {} {} (decoders {:?}, errors {:?})",
                case.codec,
                default.delivered,
                default.expected,
                if case.kind == Kind::Video {
                    "frames"
                } else {
                    "audio buffers"
                },
                default.decoders,
                default.errors
            ));
        }
        let uses_ffmpeg = default
            .decoders
            .iter()
            .any(|d| decoder_family(factory_of(d)) == "FFmpeg (gst-libav)");
        if uses_ffmpeg {
            ffmpeg_in_default_route.push(case.codec);
        }
        if case.apple_only_required {
            result.note(format!(
                "R16 P2 on the MSE path for {}: the default route {} (decoders {:?})",
                case.codec,
                if uses_ffmpeg { "USES FFmpeg, so the commercial route needs a rank change (decodebin3 inside playbin3 picks it exactly as for <video>)" } else { "uses only Apple decoders" },
                default.decoders
            ));
        }
        if case.fixture.starts_with("av1") {
            av1_defaults.push(Av1Default {
                fixture: case.fixture,
                complete: default.complete(),
                decoders: default.decoders.clone(),
                errors: default.errors.clone(),
            });
        }
        entry.insert("default".to_owned(), default.to_json(case.kind));
        entry.insert("default_uses_ffmpeg".to_owned(), json!(uses_ffmpeg));
        entry.insert(
            "apple_only_required_by_r16".to_owned(),
            json!(case.apple_only_required),
        );

        // Each alternative.
        let mut alternatives = serde_json::Map::new();
        for name in case.alternatives {
            let Some(original) = rank_of(name) else {
                alternatives.insert(
                    (*name).to_owned(),
                    json!({ "skipped": "not in the registry" }),
                );
                result.note(format!(
                    "{}: {name} is not in the registry: skipped",
                    case.codec
                ));
                continue;
            };
            let outcome = measure(ctx, &tiled, case.kind, Some(name));
            if let Some(feature) = gst::Registry::get().lookup_feature(name) {
                feature.set_rank(original);
            }
            match outcome {
                Ok(run) => {
                    result.note(format!(
                        "{} with prefer_decoder={name}: {:?}, {}, {}",
                        case.codec,
                        run.decoders,
                        delivered_text(&run, case.kind),
                        speed_text(&run, case.kind)
                    ));
                    if *name == "vtdec_hw" {
                        vt_hw_produced_frames = Some(
                            run.complete()
                                && run.decoders.iter().any(|d| factory_of(d) == "vtdec_hw"),
                        );
                    }
                    alternatives.insert((*name).to_owned(), run.to_json(case.kind));
                }
                Err(why) => {
                    result.note(format!("{} with prefer_decoder={name}: {why}", case.codec));
                    alternatives.insert((*name).to_owned(), json!({ "error": why }));
                }
            }
        }
        entry.insert("alternatives".to_owned(), Value::Object(alternatives));
        table.insert(case.codec.to_owned(), Value::Object(entry));
    }
    av1_controls(result, &av1_defaults);
    result.metric("codecs", Value::Object(table));
    result.metric("vtdec_hw_produced_frames", vt_hw_produced_frames);
    result.metric(
        "codecs_whose_default_route_uses_ffmpeg",
        json!(ffmpeg_in_default_route),
    );
    result.note(match vt_hw_produced_frames {
        Some(true) => {
            "VideoToolbox hardware decode (vtdec_hw) produced every frame on this Mac".to_owned()
        }
        Some(false) => {
            "vtdec_hw did NOT produce the frames on this Mac (see the H.264 alternatives)"
                .to_owned()
        }
        None => "vtdec_hw was not tried (not in the registry or H.264 skipped)".to_owned(),
    });
    Ok(findings.verdict())
}

/// What the default route did with one AV1 fixture through MSE.
struct Av1Default {
    fixture: &'static str,
    complete: bool,
    decoders: Vec<String>,
    errors: Vec<String>,
}

/// Frames in every fixture file: 8 s at 25 fps.
const FIXTURE_FRAMES: u64 = 200;

/// The AV1 controls (added after a skeptic found that "AV1 fails because the chip lacks AV1 hardware" had
/// no control): what VideoToolbox itself reports for this machine, and the same two files through a plain
/// `playbin3` with no MSE, with the default ranks, with dav1d preferred and with `vtdec_hw` preferred.
/// The notes end with a reading computed from these numbers, never written by hand.
fn av1_controls(result: &mut ScenarioResult, defaults: &[Av1Default]) {
    if defaults.is_empty() {
        return;
    }
    let mut controls = serde_json::Map::new();
    controls.insert(
        "videotoolbox_hardware_decode".to_owned(),
        crate::vt::support_table(),
    );
    result.note(format!(
        "VideoToolbox hardware decode on this machine (VTIsHardwareDecodeSupported): {}",
        crate::vt::support_line()
    ));

    let mut through_mse = serde_json::Map::new();
    for default in defaults {
        through_mse.insert(
            default.fixture.to_owned(),
            json!({
                "default_route_complete": default.complete,
                "decoders": default.decoders,
                "errors": default.errors,
            }),
        );
    }
    controls.insert("mse_default_route".to_owned(), Value::Object(through_mse));

    let mut plain = serde_json::Map::new();
    let mut plain_default_complete: Vec<(&'static str, bool)> = Vec::new();
    for default in defaults {
        let Some(fixture) = mse_segments::fixture(default.fixture) else {
            continue;
        };
        let path = mse_segments::fixtures_dir().join(fixture.path);
        let mut per_route = serde_json::Map::new();
        for (label, prefer) in [
            ("default", None),
            ("dav1ddec", Some("dav1ddec")),
            ("vtdec_hw", Some("vtdec_hw")),
        ] {
            crate::common::trace(&format!(
                "T07 plain playbin3 {} prefer={prefer:?}",
                default.fixture
            ));
            // On a helper thread with a limit: a pipeline that hangs in a state change must cost this one
            // route, not the scenario (T07 was once killed at its 230 s budget although a rerun took 45 s).
            let (watch_path, watch_prefer) = (path.clone(), prefer);
            let outcome = crate::watchdog::call_with_timeout(
                "plain playbin3",
                Duration::from_secs(45),
                move || {
                    crate::plain::play_video_file(
                        &watch_path,
                        watch_prefer,
                        Duration::from_secs(30),
                    )
                },
            )
            .map_err(|watch| watch.to_string())
            .and_then(|run| run);
            match outcome {
                Ok(run) => {
                    result.note(format!(
                        "{} through a plain playbin3 (no MSE), {label}: {}",
                        default.fixture,
                        run.describe(FIXTURE_FRAMES)
                    ));
                    if prefer.is_none() {
                        plain_default_complete
                            .push((default.fixture, run.complete(FIXTURE_FRAMES)));
                    }
                    per_route.insert(label.to_owned(), run.to_json(FIXTURE_FRAMES));
                }
                Err(why) => {
                    result.note(format!(
                        "{} through a plain playbin3, {label}: {why}",
                        default.fixture
                    ));
                    per_route.insert(label.to_owned(), json!({ "error": why }));
                }
            }
        }
        plain.insert(default.fixture.to_owned(), Value::Object(per_route));
    }
    controls.insert("plain_playbin3".to_owned(), Value::Object(plain));

    let reading = av1_reading(defaults, &plain_default_complete);
    for line in &reading {
        result.note(format!("AV1 reading: {line}"));
    }
    controls.insert("reading".to_owned(), json!(reading));
    result.metric("av1_controls", Value::Object(controls));
}

/// What the AV1 numbers say, in sentences that follow from them. `plain` is, per fixture, whether the plain
/// `playbin3` default route played the whole file.
fn av1_reading(defaults: &[Av1Default], plain: &[(&'static str, bool)]) -> Vec<String> {
    let mse = |name: &str| {
        defaults
            .iter()
            .find(|d| d.fixture == name)
            .map(|d| d.complete)
    };
    let plain_of = |name: &str| plain.iter().find(|(f, _)| *f == name).map(|(_, ok)| *ok);
    let mut lines = Vec::new();
    match (mse("av1-low"), mse("av1-low-av1c")) {
        (Some(false), Some(false)) => lines.push(
            "the default route through MSE fails for both fixtures, the one with the four-byte av1C and the one with the full record: the incomplete av1C is NOT what breaks the default route on this machine".to_owned(),
        ),
        (Some(false), Some(true)) => lines.push(
            "the default route through MSE fails for the fixture with the four-byte av1C and works with the full record: the hand-written av1C WAS the cause".to_owned(),
        ),
        (Some(true), Some(true)) => lines.push(
            "the default route through MSE plays both fixtures: AV1 works by default on this machine".to_owned(),
        ),
        (Some(true), Some(false)) => lines.push(
            "the default route plays the four-byte fixture but fails with the full record: unexpected, look at the errors".to_owned(),
        ),
        _ => lines.push("one of the two AV1 fixtures was not available: no comparison".to_owned()),
    }
    for fixture in ["av1-low", "av1-low-av1c"] {
        if let (Some(through_mse), Some(through_plain)) = (mse(fixture), plain_of(fixture)) {
            lines.push(format!(
                "{fixture}: MSE default route {}, plain playbin3 default route {}: {}",
                if through_mse { "plays" } else { "fails" },
                if through_plain { "plays" } else { "fails" },
                if through_mse == through_plain {
                    "the same outcome with and without MSE (not MSE-specific)"
                } else {
                    "DIFFERENT outcomes with and without MSE"
                }
            ));
        }
    }
    lines
}

fn delivered_text(run: &RunStats, kind: Kind) -> String {
    match kind {
        Kind::Video => format!("{} of {} frames delivered", run.delivered, run.expected),
        Kind::Audio => format!(
            "{} buffers delivered, stream {}",
            run.delivered,
            if run.complete {
                "played to its end"
            } else {
                "did NOT play to its end"
            }
        ),
    }
}

fn speed_text(run: &RunStats, kind: Kind) -> String {
    let unit = match kind {
        Kind::Video => "fps",
        Kind::Audio => "buffers/s",
    };
    format!(
        "{} {unit}, {} ms CPU in {:.0} ms wall",
        run.per_second
            .map_or("n/a".to_owned(), |v| format!("{v:.0}")),
        run.cpu_ms,
        run.wall_ms
    )
}

/// One run: play `tiled` through a fresh rig (decode as fast as possible) with `prefer` raised in rank.
fn measure(
    ctx: &Ctx,
    tiled: &SegmentedStream,
    kind: Kind,
    prefer: Option<&str>,
) -> Result<RunStats, String> {
    crate::common::trace(&format!("T07: {:?} prefer_decoder={prefer:?}", kind));
    let config = RigConfig {
        prefer_decoder: prefer.map(str::to_owned),
        video: kind == Kind::Video,
        audio: kind == Kind::Audio,
        ..fast_config(ctx)
    };
    let rig = new_rig(config)?;
    let id = rig
        .backend
        .add_source_buffer(&tiled.mime)
        .map_err(|e| format!("add_source_buffer({}): {e:?}", tiled.mime))?;
    rig.append_and_wait(id, &tiled.init.bytes, APPEND_TIMEOUT)
        .map_err(|e| format!("init append: {e}"))?;
    rig.play().map_err(|e| format!("play: {e}"))?;
    let seconds = tiled.duration_ms().unwrap_or(0) as f64 / 1000.0;
    let expected_video = (seconds * 25.0).round() as u64;
    let cpu_before = proc::cpu_time_ms()?;
    let started = Instant::now();
    for segment in &tiled.media {
        rig.append_and_wait(id, &segment.bytes, APPEND_TIMEOUT)
            .map_err(|e| format!("append: {e}"))?;
    }
    rig.backend
        .end_of_stream(EosError::None)
        .map_err(|e| format!("end_of_stream: {e:?}"))?;
    let finished = match kind {
        // Also stops at the first error: a decoder that cannot start (AV1 on VideoToolbox without a hardware
        // decoder) reports it within a second, and waiting 90 s for frames that cannot come would let four such
        // runs eat the scenario's time.
        Kind::Video => rig.wait_until(Duration::from_secs(90), |rig| {
            video_frames(rig) + 2 >= expected_video || has_play_error(rig)
        }),
        Kind::Audio => rig.wait_for_eos(Duration::from_secs(60)),
    };
    let wall = started.elapsed();
    let cpu_ms = proc::cpu_time_ms()?.saturating_sub(cpu_before);
    std::thread::sleep(Duration::from_millis(200));
    // AAC frames are 1024 samples at 48 kHz and Opus frames 20 ms, so the number of audio buffers differs
    // per codec: audio is complete when the stream reached its end and the sink saw the whole span.
    let (delivered, expected, complete) = match kind {
        Kind::Video => {
            let frames = video_frames(&rig);
            (frames, expected_video, frames + 2 >= expected_video)
        }
        Kind::Audio => {
            let span = crate::common::Snapshot::read(&rig.audio)
                .span_ms()
                .unwrap_or(0.0)
                / 1000.0;
            (audio_buffers(&rig), 0, finished && span + 1.0 >= seconds)
        }
    };
    let per_second = (wall.as_secs_f64() > 0.0).then(|| delivered as f64 / wall.as_secs_f64());
    let errors: Vec<String> = rig
        .log
        .snapshot()
        .into_iter()
        .filter_map(|t| match t.event {
            MseEvent::Play(PlayEvent::Error(text)) => Some(text),
            MseEvent::Error(id) => Some(format!("source buffer {id} error event")),
            _ => None,
        })
        .take(3)
        .collect();
    let decoders = rig.chosen_decoders();
    let _ = rig.shutdown();
    Ok(RunStats {
        decoders,
        delivered,
        expected,
        complete,
        wall_ms: ms(wall),
        cpu_ms,
        per_second,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_families_follow_the_factory_names() {
        assert_eq!(decoder_family("avdec_h264"), "FFmpeg (gst-libav)");
        assert_eq!(decoder_family("avdec_aac"), "FFmpeg (gst-libav)");
        assert_eq!(decoder_family("vtdec_hw"), "Apple VideoToolbox");
        assert_eq!(decoder_family("vtdec"), "Apple VideoToolbox");
        assert_eq!(decoder_family("atdec"), "Apple AudioToolbox");
        assert_eq!(decoder_family("vp9dec"), "libvpx");
        assert_eq!(decoder_family("dav1ddec"), "dav1d");
        assert_eq!(decoder_family("opusdec"), "libopus");
        assert_eq!(decoder_family("mystery"), "other");
    }

    #[test]
    fn factory_names_are_read_from_element_descriptions() {
        assert_eq!(factory_of("vtdec_hw0:vtdec_hw"), "vtdec_hw");
        assert_eq!(factory_of("avdec_aac0:avdec_aac"), "avdec_aac");
        assert_eq!(factory_of("plain"), "plain");
    }

    #[test]
    fn the_cases_cover_the_brief() {
        let codecs: Vec<&str> = CASES.iter().map(|c| c.codec).collect();
        assert_eq!(
            codecs,
            ["H.264", "VP9", "AAC", "Opus", "AV1", "AV1 (full av1C)"]
        );
        let av1: Vec<&str> = CASES
            .iter()
            .filter(|c| c.codec.starts_with("AV1"))
            .map(|c| c.fixture)
            .collect();
        assert_eq!(av1, ["av1-low", "av1-low-av1c"]);
        assert_eq!(CASES[4].alternatives, ["dav1ddec", "vtdec_hw", "vtdec"]);
        let h264 = &CASES[0];
        assert_eq!(h264.alternatives, ["vtdec_hw", "vtdec", "avdec_h264"]);
        assert!(h264.apple_only_required && CASES[2].apple_only_required);
        assert!(!CASES[1].apple_only_required);
        let names = candidate_decoders();
        for expected in [
            "vtdec_hw",
            "vtdec",
            "avdec_h264",
            "vp9dec",
            "avdec_vp9",
            "avdec_aac",
            "atdec",
            "opusdec",
            "dav1ddec",
            "vtdec",
        ] {
            assert!(names.contains(&expected), "{expected}");
        }
    }

    fn av1_default(fixture: &'static str, complete: bool) -> Av1Default {
        Av1Default {
            fixture,
            complete,
            decoders: vec![],
            errors: vec![],
        }
    }

    #[test]
    fn the_av1_reading_says_the_record_is_not_the_cause_when_both_fail() {
        let defaults = [
            av1_default("av1-low", false),
            av1_default("av1-low-av1c", false),
        ];
        let lines = av1_reading(&defaults, &[("av1-low", false), ("av1-low-av1c", false)]);
        assert!(lines[0].contains("NOT what breaks"), "{lines:?}");
        assert!(lines[1].contains("not MSE-specific"), "{lines:?}");
        assert!(lines[2].contains("not MSE-specific"), "{lines:?}");
    }

    #[test]
    fn the_av1_reading_blames_the_record_when_only_the_short_one_fails() {
        let defaults = [
            av1_default("av1-low", false),
            av1_default("av1-low-av1c", true),
        ];
        let lines = av1_reading(&defaults, &[("av1-low", false), ("av1-low-av1c", true)]);
        assert!(lines[0].contains("WAS the cause"), "{lines:?}");
    }

    #[test]
    fn the_av1_reading_flags_a_difference_between_mse_and_plain_playback() {
        let defaults = [
            av1_default("av1-low", false),
            av1_default("av1-low-av1c", false),
        ];
        let lines = av1_reading(&defaults, &[("av1-low", true), ("av1-low-av1c", false)]);
        assert!(lines[1].contains("DIFFERENT outcomes"), "{lines:?}");
        assert!(lines[2].contains("not MSE-specific"), "{lines:?}");
    }

    #[test]
    fn the_av1_reading_needs_both_fixtures() {
        let lines = av1_reading(&[av1_default("av1-low", true)], &[]);
        assert!(lines[0].contains("not available"), "{lines:?}");
    }

    #[test]
    fn a_run_is_complete_within_two_items() {
        let run = RunStats {
            decoders: vec!["vtdec_hw0:vtdec_hw".into()],
            delivered: 198,
            expected: 200,
            complete: true,
            wall_ms: 10.0,
            cpu_ms: 5,
            per_second: Some(100.0),
            errors: vec![],
        };
        assert!(run.complete());
        let short = RunStats {
            delivered: 150,
            complete: false,
            ..run
        };
        assert!(!short.complete());
        assert_eq!(
            short.to_json(Kind::Video)["families"][0],
            "Apple VideoToolbox"
        );
        assert!(short.to_json(Kind::Audio).get("buffers_per_s").is_some());
    }
}
