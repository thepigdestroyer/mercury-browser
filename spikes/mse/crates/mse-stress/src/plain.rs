// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A plain `playbin3` on a file, with no MSE anywhere in the pipeline: what a `<video src=...>` goes
//! through for decoder choice (`decodebin3` picks the decoders after the demuxer, exactly as it does
//! behind `msesrc`). T07 uses it as the control for "this codec fails in the default route": if the same
//! file fails here too, the cause is not GstMSE, and not the append path either.

use std::path::Path;
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use mse_rig::events::EventLog;
use mse_rig::frames::{CountingSink, FrameStats};

/// What one plain playback did.
#[derive(Debug, Clone, Default)]
pub struct PlainRun {
    /// The decoder elements in the pipeline when it ended, as `vtdechw0:vtdec_hw`.
    pub decoders: Vec<String>,
    /// Video frames that reached the sink.
    pub frames: u64,
    /// Error messages from the bus (at most three, first line each).
    pub errors: Vec<String>,
    pub reached_eos: bool,
    pub timed_out: bool,
    pub wall_ms: f64,
}

impl PlainRun {
    /// The run delivered `expected` frames (give or take the two a decoder may hold back) and ended.
    pub fn complete(&self, expected: u64) -> bool {
        self.reached_eos && self.errors.is_empty() && self.frames + 2 >= expected
    }

    pub fn to_json(&self, expected: u64) -> serde_json::Value {
        serde_json::json!({
            "decoders": self.decoders,
            "frames": self.frames,
            "expected": expected,
            "complete": self.complete(expected),
            "reached_eos": self.reached_eos,
            "timed_out": self.timed_out,
            "errors": self.errors,
            "wall_ms": (self.wall_ms * 10.0).round() / 10.0,
        })
    }

    /// One line for notes.
    pub fn describe(&self, expected: u64) -> String {
        format!(
            "decoders {:?}, {} of {expected} frames, {}{}",
            self.decoders,
            self.frames,
            if self.reached_eos {
                "reached the end"
            } else {
                "did not reach the end"
            },
            if self.errors.is_empty() {
                String::new()
            } else {
                format!(", errors {:?}", self.errors)
            }
        )
    }
}

/// The decoder elements of `pipeline` (class `Decoder`, not a bin such as `decodebin3`), as
/// `vtdechw0:vtdec_hw`.
fn decoders_of(pipeline: &gst::Element) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(bin) = pipeline.downcast_ref::<gst::Bin>() {
        for element in bin.iterate_recurse().into_iter().filter_map(Result::ok) {
            let Some(factory) = element.factory() else {
                continue;
            };
            let klass = factory.klass();
            if klass.contains("Decoder") && !klass.contains("Bin") {
                found.push(format!("{}:{}", element.name(), factory.name()));
            }
        }
    }
    found
}

/// The first line of an error message, without the element path noise.
fn first_lines(message: &gst::message::Error) -> String {
    let mut text = message.error().to_string();
    if let Some(debug) = message.debug() {
        // The debug string carries the file, the line and (for VideoToolbox) the OSStatus.
        if let Some(line) = debug.lines().find(|l| l.contains("returned")) {
            text.push_str(" | ");
            text.push_str(line.trim());
        }
    }
    text
}

/// Plays the video file at `path` with a `playbin3` into a counting sink, as fast as possible, until the
/// end, an error or `limit`. `prefer` raises that decoder factory above every other (and puts its rank
/// back afterwards).
pub fn play_video_file(
    path: &Path,
    prefer: Option<&str>,
    limit: Duration,
) -> Result<PlainRun, String> {
    let uri = gst::glib::filename_to_uri(path, None)
        .map_err(|e| format!("{}: not a file URI: {e}", path.display()))?;
    let raised = match prefer {
        Some(name) => {
            let feature = gst::Registry::get()
                .lookup_feature(name)
                .ok_or_else(|| format!("no element factory {name:?}"))?;
            let original = feature.rank();
            feature.set_rank(gst::Rank::from(1000));
            Some((feature, original))
        }
        None => None,
    };
    let outcome = play_with_default_ranks_set(&uri, limit);
    if let Some((feature, original)) = raised {
        feature.set_rank(original);
    }
    outcome
}

fn play_with_default_ranks_set(uri: &str, limit: Duration) -> Result<PlainRun, String> {
    let log = EventLog::new();
    let stats = FrameStats::shared();
    let video = CountingSink::video("plain-video", stats.clone(), false, &log)
        .map_err(|e| format!("video sink: {e}"))?;
    let audio =
        CountingSink::discard("plain-audio", false).map_err(|e| format!("audio sink: {e}"))?;
    let playbin = gst::ElementFactory::make("playbin3")
        .name("plain-playbin3")
        .property("uri", uri)
        .property("video-sink", &video.element)
        .property("audio-sink", &audio.element)
        .build()
        .map_err(|e| format!("playbin3: {e}"))?;
    let bus = playbin.bus().ok_or("playbin3 has no bus")?;
    let started = Instant::now();
    playbin
        .set_state(gst::State::Playing)
        .map_err(|e| format!("set_state(Playing): {e}"))?;
    let mut run = PlainRun::default();
    loop {
        let left = limit.saturating_sub(started.elapsed());
        if left.is_zero() {
            run.timed_out = true;
            break;
        }
        let Some(message) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(left.as_millis().min(500) as u64),
            &[gst::MessageType::Error, gst::MessageType::Eos],
        ) else {
            continue;
        };
        match message.view() {
            gst::MessageView::Eos(_) => {
                run.reached_eos = true;
                break;
            }
            gst::MessageView::Error(error) => {
                if run.errors.len() < 3 {
                    run.errors.push(first_lines(error));
                }
                break;
            }
            _ => {}
        }
    }
    run.wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    run.decoders = decoders_of(&playbin);
    run.frames = stats.lock().unwrap_or_else(|e| e.into_inner()).count;
    let _ = playbin.set_state(gst::State::Null);
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_complete_only_with_the_end_no_error_and_the_frames() {
        let mut run = PlainRun {
            frames: 200,
            reached_eos: true,
            ..PlainRun::default()
        };
        assert!(run.complete(200));
        assert!(run.complete(202));
        assert!(!run.complete(203));
        run.errors.push("boom".into());
        assert!(!run.complete(200));
        run.errors.clear();
        run.reached_eos = false;
        assert!(!run.complete(200));
    }

    #[test]
    fn the_description_names_decoders_frames_and_errors() {
        let run = PlainRun {
            decoders: vec!["dav1ddec0:dav1ddec".into()],
            frames: 7,
            errors: vec!["no luck".into()],
            ..PlainRun::default()
        };
        let text = run.describe(200);
        assert!(text.contains("dav1ddec0:dav1ddec"));
        assert!(text.contains("7 of 200 frames"));
        assert!(text.contains("did not reach the end"));
        assert!(text.contains("no luck"));
    }

    #[test]
    fn json_has_the_numbers_a_report_needs() {
        let json = PlainRun {
            frames: 1,
            wall_ms: 12.345,
            ..PlainRun::default()
        }
        .to_json(5);
        assert_eq!(json["frames"], 1);
        assert_eq!(json["expected"], 5);
        assert_eq!(json["complete"], false);
        assert_eq!(json["wall_ms"], 12.3);
    }
}
