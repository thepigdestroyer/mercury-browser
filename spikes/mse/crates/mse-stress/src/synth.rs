// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Synthetic 1080p streams made at run time (T05, T07, T11): `videotestsrc` through `vtenc_h264`
//! (VideoToolbox, which keeps to its bit rate; H.264 into `cmafmux`) or `vp9enc` (into `webmmux`), written to the scenario's scratch
//! directory, split with `mse_segments::split` and deleted by the caller together with the scratch
//! directory. Never `x264enc` or `x265enc` (GPL), never committed.
//!
//! The picture is television snow (random noise), encoded at about 6 Mbit/s whatever the picture, so
//! the decoder has a realistic bit rate to chew on (the moving-ball pattern compresses to 15 kB/s and
//! measures nothing; `openh264enc` and `vp9enc` in VBR mode overshoot 6 Mbit/s on noise by a factor of 3 to 4).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use mse_segments::{Container, SegmentedStream, split};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthCodec {
    H264,
    Vp9,
}

impl SynthCodec {
    pub fn label(self) -> &'static str {
        match self {
            SynthCodec::H264 => "h264",
            SynthCodec::Vp9 => "vp9",
        }
    }

    pub fn container(self) -> Container {
        match self {
            SynthCodec::H264 => Container::Mp4,
            SynthCodec::Vp9 => Container::WebM,
        }
    }

    fn extension(self) -> &'static str {
        match self {
            SynthCodec::H264 => "mp4",
            SynthCodec::Vp9 => "webm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SynthSpec {
    pub codec: SynthCodec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub seconds: u32,
}

impl SynthSpec {
    /// The 1080p, 30 fps, 10 s stream of the brief.
    pub fn full_hd(codec: SynthCodec, seconds: u32) -> Self {
        SynthSpec {
            codec,
            width: 1920,
            height: 1080,
            fps: 30,
            seconds,
        }
    }

    pub fn frames(&self) -> u32 {
        self.fps * self.seconds
    }
}

/// The elements the pipeline of `codec` needs, in plugin-load order.
pub fn required_elements(codec: SynthCodec) -> &'static [&'static str] {
    match codec {
        SynthCodec::H264 => &[
            "videotestsrc",
            "videoconvert",
            "vtenc_h264",
            "h264parse",
            "cmafmux",
            "filesink",
        ],
        SynthCodec::Vp9 => &[
            "videotestsrc",
            "videoconvert",
            "vp9enc",
            "webmmux",
            "filesink",
        ],
    }
}

/// The plugin file (without directory) that provides `element`, for loading it by name when the
/// plugin list of the run (Servo's way) does not include it.
pub fn plugin_file_for(element: &str) -> Option<&'static str> {
    match element {
        "videotestsrc" => Some("libgstvideotestsrc.dylib"),
        "videoconvert" => Some("libgstvideoconvertscale.dylib"),
        "vtenc_h264" => Some("libgstapplemedia.dylib"),
        "h264parse" => Some("libgstvideoparsersbad.dylib"),
        "cmafmux" => Some("libgstisobmff.dylib"),
        "vp9enc" => Some("libgstvpx.dylib"),
        "webmmux" => Some("libgstmatroska.dylib"),
        "filesink" => Some("libgstcoreelements.dylib"),
        _ => None,
    }
}

/// The `gst-launch` description of the pipeline that writes the stream to `output`.
pub fn pipeline_description(spec: &SynthSpec, output: &Path) -> String {
    let source = format!(
        "videotestsrc num-buffers={} pattern=snow ! video/x-raw,width={},height={},framerate={}/1 \
         ! videoconvert ! video/x-raw,format=I420",
        spec.frames(),
        spec.width,
        spec.height,
        spec.fps
    );
    // A key frame every two seconds, and fragments of two seconds, as the fixtures have.
    let gop = spec.fps * 2;
    match spec.codec {
        SynthCodec::H264 => format!(
            "{source} ! vtenc_h264 bitrate=6000 max-keyframe-interval={gop} realtime=false \
             allow-frame-reordering=false \
             ! h264parse config-interval=-1 ! video/x-h264,stream-format=avc,alignment=au \
             ! cmafmux fragment-duration=2000000000 ! filesink location={}",
            output.display()
        ),
        SynthCodec::Vp9 => format!(
            "{source} ! vp9enc target-bitrate=6000000 keyframe-max-dist={gop} deadline=1 cpu-used=8 \
             end-usage=cbr threads=4 lag-in-frames=0 auto-alt-ref=false \
             ! webmmux streamable=true ! filesink location={}",
            output.display()
        ),
    }
}

#[derive(Debug)]
pub enum SynthError {
    /// An element or its plugin is not available: the scenario skips.
    Missing(String),
    /// The pipeline failed or timed out, or the output could not be split.
    Failed(String),
}

impl std::fmt::Display for SynthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SynthError::Missing(what) | SynthError::Failed(what) => f.write_str(what),
        }
    }
}

/// Makes sure every element of the pipeline exists; loads the plugin file by name when it does not.
fn ensure_elements(codec: SynthCodec) -> Result<(), SynthError> {
    for element in required_elements(codec) {
        if gst::ElementFactory::find(element).is_some() {
            continue;
        }
        let file = plugin_file_for(element)
            .ok_or_else(|| SynthError::Missing(format!("element {element} is not available")))?;
        let prefix = std::env::var("MERCURY_GSTREAMER_PREFIX").map_err(|_| {
            SynthError::Missing(format!(
                "element {element} is missing and MERCURY_GSTREAMER_PREFIX is not set"
            ))
        })?;
        let path: PathBuf = Path::new(&prefix).join("lib/gstreamer-1.0").join(file);
        gst::Plugin::load_file(&path).map_err(|error| {
            SynthError::Missing(format!("element {element}: loading {file} failed: {error}"))
        })?;
        if gst::ElementFactory::find(element).is_none() {
            return Err(SynthError::Missing(format!(
                "element {element} is still missing after loading {file}"
            )));
        }
    }
    Ok(())
}

/// How long the encode may take (the Intel i7 encodes 1080p H.264 at tens of frames per second).
const ENCODE_TIMEOUT: Duration = Duration::from_secs(150);

/// Encodes the stream into `scratch`, splits it and removes the file. Returns the stream and how
/// long the encode took.
pub fn make_stream(
    spec: &SynthSpec,
    scratch: &Path,
) -> Result<(SegmentedStream, Duration), SynthError> {
    ensure_elements(spec.codec)?;
    std::fs::create_dir_all(scratch)
        .map_err(|e| SynthError::Failed(format!("{}: {e}", scratch.display())))?;
    let output = scratch.join(format!(
        "synth-{}-{}x{}.{}",
        spec.codec.label(),
        spec.width,
        spec.height,
        spec.codec.extension()
    ));
    let description = pipeline_description(spec, &output);
    crate::common::trace(&format!("synth: {description}"));
    let started = Instant::now();
    let pipeline = gst::parse::launch(&description)
        .map_err(|e| SynthError::Failed(format!("parse_launch: {e}")))?;
    let bus = pipeline
        .bus()
        .ok_or_else(|| SynthError::Failed("the synthetic pipeline has no bus".into()))?;
    crate::common::trace("synth: playing");
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| SynthError::Failed(format!("set_state(Playing): {e}")))?;
    let message = bus.timed_pop_filtered(
        gst::ClockTime::from_nseconds(ENCODE_TIMEOUT.as_nanos() as u64),
        &[gst::MessageType::Eos, gst::MessageType::Error],
    );
    let outcome = match message {
        Some(message) => match message.view() {
            gst::MessageView::Eos(_) => Ok(()),
            gst::MessageView::Error(error) => Err(format!(
                "{} ({})",
                error.error(),
                error.debug().map(|d| d.to_string()).unwrap_or_default()
            )),
            _ => Err("unexpected bus message".to_owned()),
        },
        None => Err(format!(
            "the encode did not finish within {} s",
            ENCODE_TIMEOUT.as_secs()
        )),
    };
    let _ = pipeline.set_state(gst::State::Null);
    let took = started.elapsed();
    if let Err(why) = outcome {
        let _ = std::fs::remove_file(&output);
        return Err(SynthError::Failed(format!(
            "{} encode: {why}",
            spec.codec.label()
        )));
    }
    let bytes = std::fs::read(&output)
        .map_err(|e| SynthError::Failed(format!("{}: {e}", output.display())));
    let _ = std::fs::remove_file(&output);
    let bytes = bytes?;
    let stream = split(spec.codec.container(), &bytes, "")
        .map_err(|e| SynthError::Failed(format!("splitting the synthetic stream: {e}")))?;
    Ok((stream, took))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipelines_never_use_a_gpl_encoder() {
        for codec in [SynthCodec::H264, SynthCodec::Vp9] {
            let spec = SynthSpec::full_hd(codec, 10);
            let text = pipeline_description(&spec, Path::new("/tmp/out"));
            assert!(
                !text.contains("x264enc") && !text.contains("x265enc"),
                "{text}"
            );
            assert!(text.contains("num-buffers=300"));
            assert!(text.contains("width=1920,height=1080,framerate=30/1"));
            assert!(text.contains("location=/tmp/out"));
        }
    }

    #[test]
    fn h264_goes_through_videotoolbox_into_cmafmux_and_vp9_into_webmmux() {
        let h264 = pipeline_description(&SynthSpec::full_hd(SynthCodec::H264, 10), Path::new("a"));
        assert!(
            h264.contains("vtenc_h264")
                && h264.contains("cmafmux")
                && h264.contains("max-keyframe-interval=60")
        );
        let vp9 = pipeline_description(&SynthSpec::full_hd(SynthCodec::Vp9, 10), Path::new("a"));
        assert!(
            vp9.contains("vp9enc")
                && vp9.contains("webmmux")
                && vp9.contains("keyframe-max-dist=60")
        );
    }

    #[test]
    fn every_required_element_has_a_plugin_file() {
        for codec in [SynthCodec::H264, SynthCodec::Vp9] {
            for element in required_elements(codec) {
                let file = plugin_file_for(element)
                    .unwrap_or_else(|| panic!("no plugin file for {element}"));
                assert!(file.starts_with("libgst") && file.ends_with(".dylib"));
            }
        }
        assert_eq!(plugin_file_for("nonsense"), None);
    }

    #[test]
    fn frame_counts_follow_the_duration() {
        let spec = SynthSpec {
            codec: SynthCodec::H264,
            width: 320,
            height: 180,
            fps: 25,
            seconds: 4,
        };
        assert_eq!(spec.frames(), 100);
        assert_eq!(SynthSpec::full_hd(SynthCodec::Vp9, 10).frames(), 300);
        assert_eq!(SynthCodec::H264.container(), Container::Mp4);
        assert_eq!(SynthCodec::Vp9.container(), Container::WebM);
    }
}
