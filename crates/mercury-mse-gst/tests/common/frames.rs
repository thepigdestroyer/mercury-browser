// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Coded frames and track configurations out of the committed fixtures, made by GStreamer's own demuxers
//! (`qtdemux`, `matroskademux`), independently of the parsers of `mercury-mse` (item 1): the playback tests
//! of this crate then say something about the source element, not about a parser.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use mercury_mse::{CodecConfig, CodedFrame, TrackConfig, TrackKind};

/// One track of a fixture: its configuration and every frame in decode order.
#[derive(Clone)]
pub struct Extracted {
    pub kind: TrackKind,
    pub config: TrackConfig,
    pub frames: Vec<CodedFrame>,
}

type Cache = Mutex<HashMap<String, Arc<Vec<Extracted>>>>;

/// The tracks of a fixture (`h264-low`, `aac`, `h264-aac`, `opus`, ...), extracted once per process.
pub fn extract(name: &str) -> Arc<Vec<Extracted>> {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(found) = cache.lock().unwrap().get(name) {
        return Arc::clone(found);
    }
    let extracted = Arc::new(demux(name));
    cache
        .lock()
        .unwrap()
        .insert(name.to_owned(), Arc::clone(&extracted));
    extracted
}

/// The one track of a given kind.
pub fn track(name: &str, kind: TrackKind) -> Extracted {
    extract(name)
        .iter()
        .find(|track| track.kind == kind)
        .unwrap_or_else(|| panic!("fixture {name} has no {} track", kind.name()))
        .clone()
}

fn demux(name: &str) -> Vec<Extracted> {
    super::env::init();
    let fixture = mercury_mse_testkit::fixture(name)
        .unwrap_or_else(|| panic!("fixture {name} is not in the testkit"));
    let bytes = mercury_mse_testkit::read_fixture(name).expect("fixture bytes");
    let (demuxer_name, caps) = match fixture.container {
        mercury_mse_testkit::Container::Mp4 => ("qtdemux", "video/quicktime"),
        mercury_mse_testkit::Container::WebM => ("matroskademux", "video/webm"),
    };

    let pipeline = gst::Pipeline::new();
    let appsrc = gst::ElementFactory::make("appsrc")
        .property("caps", gst::Caps::builder(caps).build())
        .property_from_str("format", "bytes")
        .build()
        .expect("appsrc")
        .downcast::<gst_app::AppSrc>()
        .unwrap();
    let demuxer = gst::ElementFactory::make(demuxer_name)
        .build()
        .unwrap_or_else(|_| panic!("{demuxer_name} is not available"));
    pipeline.add_many([appsrc.upcast_ref(), &demuxer]).unwrap();
    appsrc.link(&demuxer).unwrap();

    struct Collected {
        caps: Option<gst::Caps>,
        frames: Vec<CodedFrame>,
    }
    let collected: Arc<Mutex<Vec<Arc<Mutex<Collected>>>>> = Arc::default();
    {
        let collected = Arc::clone(&collected);
        let pipeline_weak = pipeline.downgrade();
        demuxer.connect_pad_added(move |_, pad| {
            let Some(pipeline) = pipeline_weak.upgrade() else {
                return;
            };
            let sink = gst_app::AppSink::builder().sync(false).build();
            let slot = Arc::new(Mutex::new(Collected {
                caps: None,
                frames: Vec::new(),
            }));
            collected.lock().unwrap().push(Arc::clone(&slot));
            sink.set_callbacks(
                gst_app::AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                        let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                        let mut slot = slot.lock().unwrap();
                        if slot.caps.is_none() {
                            slot.caps = sample.caps().map(|caps| caps.to_owned());
                        }
                        let kind = match slot.caps.as_ref().and_then(|c| c.structure(0)) {
                            Some(s) if s.name().starts_with("video/") => TrackKind::Video,
                            _ => TrackKind::Audio,
                        };
                        let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                        let pts = buffer.pts().map_or(0, |t| t.nseconds() as i64);
                        slot.frames.push(CodedFrame {
                            kind,
                            track_id: if kind == TrackKind::Video { 1 } else { 2 },
                            pts,
                            dts: buffer.dts().map_or(pts, |t| t.nseconds() as i64),
                            duration: buffer.duration().map_or(0, |t| t.nseconds() as i64),
                            keyframe: !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT),
                            data: Arc::from(map.as_slice()),
                        });
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            );
            pipeline.add(&sink).unwrap();
            sink.sync_state_with_parent().unwrap();
            pad.link(&sink.static_pad("sink").unwrap()).unwrap();
        });
    }

    let bus = pipeline.bus().unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    for chunk in bytes.chunks(64 * 1024) {
        appsrc
            .push_buffer(gst::Buffer::from_slice(chunk.to_vec()))
            .unwrap();
    }
    appsrc.end_of_stream().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "demuxing {name} timed out");
        let Some(message) = bus.timed_pop(gst::ClockTime::from_nseconds(left.as_nanos() as u64))
        else {
            continue;
        };
        match message.view() {
            gst::MessageView::Eos(_) => break,
            gst::MessageView::Error(error) => panic!("demuxing {name}: {}", error.error()),
            _ => {}
        }
    }
    pipeline.set_state(gst::State::Null).unwrap();

    let mut tracks = Vec::new();
    for slot in collected.lock().unwrap().iter() {
        let slot = slot.lock().unwrap();
        let caps = slot.caps.clone().expect("a track without caps");
        let kind = slot.frames.first().map_or(TrackKind::Video, |f| f.kind);
        tracks.push(Extracted {
            kind,
            config: config_from_caps(kind, &caps),
            frames: slot.frames.clone(),
        });
    }
    tracks.sort_by_key(|track| track.kind);
    tracks
}

fn bytes_of(structure: &gst::StructureRef, field: &str) -> Option<Vec<u8>> {
    let buffer = structure.get::<gst::Buffer>(field).ok()?;
    Some(buffer.map_readable().ok()?.as_slice().to_vec())
}

/// The `TrackConfig` the feed would hand over, from the demuxer's caps.
fn config_from_caps(kind: TrackKind, caps: &gst::Caps) -> TrackConfig {
    let s = caps.structure(0).expect("caps with a structure");
    let int = |field: &str| s.get::<i32>(field).unwrap_or(0).max(0) as u32;
    let (codec, codec_string) = match s.name().as_str() {
        "video/x-h264" => {
            let avcc = bytes_of(s, "codec_data").expect("avcC in the H.264 caps");
            let string = format!("avc1.{:02X}{:02X}{:02X}", avcc[1], avcc[2], avcc[3]);
            (CodecConfig::H264 { avcc }, string)
        }
        "video/x-vp9" => (CodecConfig::Vp9 { vpcc: None }, "vp09".to_owned()),
        "video/x-vp8" => (CodecConfig::Vp8, "vp8".to_owned()),
        "video/x-av1" => (
            CodecConfig::Av1 {
                av1c: bytes_of(s, "codec_data").unwrap_or_default(),
            },
            "av01".to_owned(),
        ),
        "audio/mpeg" => (
            CodecConfig::Aac {
                asc: bytes_of(s, "codec_data").expect("AudioSpecificConfig in the AAC caps"),
            },
            "mp4a.40.2".to_owned(),
        ),
        "audio/x-opus" => {
            let channels = int("channels").max(1) as u8;
            let rate = int("rate").max(8000);
            let mut head = b"OpusHead".to_vec();
            head.push(1);
            head.push(channels);
            head.extend_from_slice(&312u16.to_le_bytes());
            head.extend_from_slice(&rate.to_le_bytes());
            head.extend_from_slice(&0u16.to_le_bytes());
            head.push(int("channel-mapping-family") as u8);
            (CodecConfig::Opus { head }, "opus".to_owned())
        }
        other => panic!("no configuration for {other}"),
    };
    TrackConfig {
        kind,
        track_id: if kind == TrackKind::Video { 1 } else { 2 },
        codec,
        codec_string,
        width: int("width"),
        height: int("height"),
        sample_rate: int("rate"),
        channels: int("channels"),
        timescale: 0,
        language: String::new(),
        default: true,
    }
}

/// Synthetic 1080p video made at run time (T05): `videotestsrc` through a software encoder of the
/// bundle. `codec` is `"h264"` (`openh264enc`) or `"vp9"` (`vp9enc`). Returns the frames in decode order
/// with their configuration, or `None` when an element is missing.
pub fn synthetic_1080p(codec: &str, frames: u32) -> Option<Extracted> {
    super::env::init();
    let encoder = match codec {
        "h264" => "openh264enc",
        "vp9" => "vp9enc",
        other => panic!("no synthetic codec {other}"),
    };
    for element in ["videotestsrc", "videoconvert", encoder, "h264parse"] {
        if gst::ElementFactory::find(element).is_none() {
            super::env::skip_or_fail(element);
            return None;
        }
    }
    let description = match codec {
        "h264" => format!(
            "videotestsrc num-buffers={frames} pattern=snow ! video/x-raw,width=1920,height=1080,framerate=25/1 \
             ! videoconvert ! openh264enc bitrate=6000000 gop-size=50 \
             ! h264parse ! video/x-h264,stream-format=avc,alignment=au ! appsink name=out sync=false"
        ),
        _ => format!(
            "videotestsrc num-buffers={frames} pattern=snow ! video/x-raw,width=1920,height=1080,framerate=25/1 \
             ! videoconvert ! vp9enc deadline=1 cpu-used=8 target-bitrate=6000000 keyframe-max-dist=50 threads=4 \
             ! appsink name=out sync=false"
        ),
    };
    let pipeline = gst::parse::launch(&description)
        .expect("a pipeline")
        .downcast::<gst::Pipeline>()
        .unwrap();
    let sink = pipeline
        .by_name("out")
        .unwrap()
        .downcast::<gst_app::AppSink>()
        .unwrap();
    let collected: Arc<Mutex<(Option<gst::Caps>, Vec<CodedFrame>)>> = Arc::default();
    let slot = Arc::clone(&collected);
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let mut slot = slot.lock().unwrap();
                if slot.0.is_none() {
                    slot.0 = sample.caps().map(|c| c.to_owned());
                }
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let pts = buffer.pts().map_or(0, |t| t.nseconds() as i64);
                slot.1.push(CodedFrame {
                    kind: TrackKind::Video,
                    track_id: 1,
                    pts,
                    dts: buffer.dts().map_or(pts, |t| t.nseconds() as i64),
                    duration: buffer
                        .duration()
                        .map_or(40_000_000, |t| t.nseconds() as i64),
                    keyframe: !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT),
                    data: Arc::from(map.as_slice()),
                });
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    let bus = pipeline.bus().unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "encoding the synthetic {codec} stream timed out"
        );
        let Some(message) = bus.timed_pop(gst::ClockTime::from_nseconds(left.as_nanos() as u64))
        else {
            continue;
        };
        match message.view() {
            gst::MessageView::Eos(_) => break,
            gst::MessageView::Error(error) => panic!("encoding {codec}: {}", error.error()),
            _ => {}
        }
    }
    pipeline.set_state(gst::State::Null).unwrap();
    let (caps, frames) = std::mem::take(&mut *collected.lock().unwrap());
    let caps = caps.expect("caps from the encoder");
    Some(Extracted {
        kind: TrackKind::Video,
        config: config_from_caps(TrackKind::Video, &caps),
        frames,
    })
}
