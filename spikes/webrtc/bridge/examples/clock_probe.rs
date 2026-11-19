// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What a live `videotestsrc` does under two base-time policies.
//!
//! ```text
//! clock_probe default    the pipeline's own: base time = the clock when it goes PLAYING
//! clock_probe product    what servo-media-gstreamer's pipeline_or_new does: start time NONE and a
//!                        fixed base time chosen earlier (here: 20 s before the pipeline starts)
//! ```
//!
//! Prints the frames in the first second and the PTS of the first frames. Run from the crate
//! directory: `cargo run --release --example clock_probe -- product`.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

fn main() {
    gst::init().unwrap();
    let product = std::env::args().nth(1).as_deref() == Some("product");
    let pipeline = gst::Pipeline::new();
    let clock = gst::SystemClock::obtain();
    pipeline.use_clock(Some(&clock));
    let ahead = gst::ClockTime::from_seconds(20);
    if product {
        pipeline.set_start_time(gst::ClockTime::NONE);
        pipeline.set_base_time(clock.time() - ahead);
    }
    let src = gst::ElementFactory::make("videotestsrc")
        .property("is-live", true)
        .build()
        .unwrap();
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", "I420")
        .field("width", 640)
        .field("height", 360)
        .field("framerate", gst::Fraction::new(30, 1))
        .build();
    let filter = gst::ElementFactory::make("capsfilter")
        .property("caps", &caps)
        .build()
        .unwrap();
    let sink = gst::ElementFactory::make("appsink")
        .property("sync", false)
        .build()
        .unwrap();
    pipeline.add_many([&src, &filter, &sink]).unwrap();
    gst::Element::link_many([&src, &filter, &sink]).unwrap();
    let seen: Arc<Mutex<Vec<(f64, u64)>>> = Arc::default();
    let start = Instant::now();
    let record = seen.clone();
    sink.clone()
        .downcast::<gst_app::AppSink>()
        .unwrap()
        .set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let pts = sample
                        .buffer()
                        .and_then(|b| b.pts())
                        .map_or(0, |p| p.nseconds());
                    record
                        .lock()
                        .unwrap()
                        .push((start.elapsed().as_secs_f64() * 1000.0, pts));
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
    pipeline.set_state(gst::State::Playing).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3000));
    let base = sink.base_time().map_or(0, |b| b.nseconds());
    pipeline.set_state(gst::State::Null).unwrap();
    let seen = seen.lock().unwrap();
    let first_second = seen.iter().filter(|(t, _)| *t < 1000.0).count();
    println!(
        "policy={} frames_in_first_second={first_second} total_in_3s={} base_time={:.3} s ({} before the clock at start)",
        if product {
            "product (start NONE, fixed base 20 s ago)"
        } else {
            "default"
        },
        seen.len(),
        base as f64 / 1e9,
        if product { "20 s" } else { "0 s" }
    );
    for (arrival_ms, pts) in seen.iter().take(8) {
        println!(
            "  arrival {arrival_ms:8.1} ms  pts {:8.1} ms  (pts - arrival offset {:+.1} ms)",
            *pts as f64 / 1e6,
            *pts as f64 / 1e6 - arrival_ms
        );
    }
}
