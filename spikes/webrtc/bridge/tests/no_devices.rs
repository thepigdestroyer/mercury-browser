// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! No real device is created: the sources are the fake ones and the crate never names a device
//! element (decision D9 of the sp-rtc lane).

use std::sync::Arc;

use gstreamer::prelude::*;
use mercury_frame_sink::MediaFrameSink;
use mercury_rtc_bridge::{CameraConfig, CaptureStream, MicrophoneConfig, RemoteSource};

/// Element names a device would use; written in pieces so that this file does not contain them.
fn device_elements() -> Vec<String> {
    vec![
        ["avf", "videosrc"].concat(),
        ["osx", "audiosrc"].concat(),
        ["osx", "audiosink"].concat(),
        ["auto", "audiosink"].concat(),
        ["auto", "videosink"].concat(),
        ["auto", "audiosrc"].concat(),
        ["auto", "videosrc"].concat(),
        ["avf", "deviceprovider"].concat(),
    ]
}

#[test]
fn the_sources_do_not_name_a_device_element() {
    let forbidden = device_elements();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0;
    for folder in ["src", "tests", "examples"] {
        for entry in std::fs::read_dir(dir.join(folder)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for name in &forbidden {
                    assert!(
                        !text.contains(name.as_str()),
                        "{} names {name}",
                        path.display()
                    );
                }
                checked += 1;
            }
        }
    }
    assert!(checked >= 8, "only {checked} files checked");
}

struct Nothing;
impl MediaFrameSink for Nothing {}

#[test]
fn the_running_pipelines_hold_only_fake_sources_and_plain_elements() {
    let allowed = [
        "videotestsrc",
        "audiotestsrc",
        "capsfilter",
        "videoconvert",
        "audioconvert",
        "audioresample",
        "audiobuffersplit",
        "tee",
        "queue",
        "appsink",
        "appsrc",
    ];
    let forbidden = device_elements();
    let camera = CaptureStream::fake_camera(CameraConfig::default()).unwrap();
    let mic = CaptureStream::fake_microphone(MicrophoneConfig::default()).unwrap();
    for stream in [&camera, &mic] {
        stream.add_frame_sink(Arc::new(Nothing)).unwrap();
        stream.add_frame_sink(Arc::new(Nothing)).unwrap();
    }
    let remote_video = RemoteSource::video(16, 16).unwrap();
    let remote_audio = RemoteSource::audio(1).unwrap();
    let pipelines = [
        camera.pipeline().clone(),
        mic.pipeline().clone(),
        remote_video.take_sink_pipeline(),
        remote_audio.take_sink_pipeline(),
    ];
    let mut seen = Vec::new();
    for pipeline in &pipelines {
        for element in pipeline.iterate_elements().into_iter().flatten() {
            let factory = element.factory().expect("a factory").name().to_string();
            assert!(
                allowed.contains(&factory.as_str()),
                "unexpected element {factory} in {}",
                pipeline.name()
            );
            assert!(!forbidden.contains(&factory));
            seen.push(factory);
        }
    }
    seen.sort();
    seen.dedup();
    println!(
        "RESULT no devices: element kinds in use: {}",
        seen.join(", ")
    );
    // And no device provider is needed: the registry is not asked for one.
    assert!(seen.contains(&"videotestsrc".to_string()));
    assert!(seen.contains(&"audiotestsrc".to_string()));
}
