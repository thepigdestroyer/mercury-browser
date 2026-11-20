// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Shared by the integration tests: one `env::init` per process (a `OnceLock`), a private scratch
//! directory removed when the process exits, and a lock so that tests run one rig at a time.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use mse_rig::backend::EosError;
use mse_rig::env::{GstInfo, InitOptions, PluginMode};
use mse_rig::events::{MseEvent, SourceBufferId};
use mse_rig::frames::{ColourSample, Dominant};
use mse_rig::rig::{Rig, RigConfig, Shape, TeardownOrder};

static SCRATCH: OnceLock<PathBuf> = OnceLock::new();
static INFO: OnceLock<GstInfo> = OnceLock::new();
static ONE_RIG_AT_A_TIME: Mutex<()> = Mutex::new(());

extern "C" fn remove_scratch() {
    if let Some(dir) = SCRATCH.get() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The private scratch directory of this test process.
pub fn scratch() -> &'static PathBuf {
    SCRATCH.get_or_init(|| {
        let dir = std::env::temp_dir()
            .join("opencode")
            .join("lanes")
            .join("sp-mse")
            .join(format!("rig-tests-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // SAFETY: `remove_scratch` is a plain `extern "C" fn()` that touches only a `OnceLock`.
        unsafe { libc::atexit(remove_scratch) };
        dir
    })
}

/// Initialises GStreamer once for the whole test process, in `mode`. A second call with another mode
/// returns the first call's result (see `env::init`), so each test file uses one mode.
pub fn init(mode: PluginMode) -> &'static GstInfo {
    INFO.get_or_init(|| {
        mse_rig::env::init(&InitOptions {
            prefix: None,
            plugin_mode: mode,
            scratch_dir: scratch().clone(),
            glib_main_loop: false,
        })
        .expect(
            "GStreamer initialises (run through spikes/mse/run.sh, after `make gstreamer-macos`)",
        )
    })
}

/// Holds the lock that keeps two rigs from running at once in this process.
pub fn one_rig_at_a_time() -> MutexGuard<'static, ()> {
    ONE_RIG_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What a full muxed append/play cycle left behind.
pub struct Cycle {
    pub rig: Rig,
    pub ids: Vec<SourceBufferId>,
    pub eos: bool,
}

/// Appends the init segment and the four media segments of `fixture` (one source buffer) as a page
/// would, plays, calls `end_of_stream` and waits for the element's end of stream.
pub fn play_fixture(shape: Shape, fixture: &str) -> Cycle {
    let stream = mse_segments::load(fixture).expect("the fixture loads");
    let rig = Rig::new(RigConfig {
        shape,
        sync: false,
        ..RigConfig::default()
    })
    .expect("a rig");
    let id = rig
        .backend
        .add_source_buffer(&stream.mime)
        .expect("a source buffer");
    let step = Duration::from_secs(10);
    rig.append_and_wait(id, &stream.init.bytes, step)
        .expect("init segment");
    for segment in &stream.media {
        rig.append_and_wait(id, &segment.bytes, step)
            .expect("media segment");
    }
    rig.play().expect("play");
    rig.backend
        .end_of_stream(EosError::None)
        .expect("end of stream");
    let eos = rig.wait_for_eos(Duration::from_secs(30));
    Cycle {
        rig,
        ids: vec![id],
        eos,
    }
}

/// The dominant colour of each sample, `R`, `B` or `?`.
pub fn colour_letters(samples: &[ColourSample]) -> String {
    samples
        .iter()
        .map(|sample| match sample.dominant() {
            Dominant::Red => 'R',
            Dominant::Blue => 'B',
            Dominant::Other => '?',
        })
        .collect()
}

/// Whether the log shows `UpdateStart`, `Update`, `UpdateEnd` in that order, in threes, for `id`, and
/// nothing else about it (no `Error`, no `Abort`).
pub fn updates_are_well_ordered(
    events: &[mse_rig::events::TimedEvent],
    id: SourceBufferId,
) -> bool {
    let mine: Vec<&MseEvent> = events
        .iter()
        .map(|timed| &timed.event)
        .filter(|event| {
            matches!(
                event,
                MseEvent::UpdateStart(i)
                    | MseEvent::Update(i)
                    | MseEvent::UpdateEnd(i)
                    | MseEvent::Abort(i)
                    | MseEvent::Error(i) if *i == id
            )
        })
        .collect();
    !mine.is_empty()
        && mine.len().is_multiple_of(3)
        && mine.chunks(3).all(|triple| {
            matches!(
                triple,
                [
                    MseEvent::UpdateStart(_),
                    MseEvent::Update(_),
                    MseEvent::UpdateEnd(_)
                ]
            )
        })
}

pub const ALL_ORDERS: [TeardownOrder; 4] = [
    TeardownOrder::PipelineNullThenBackend,
    TeardownOrder::BackendThenPipelineNull,
    TeardownOrder::DetachThenNull,
    TeardownOrder::DropWithoutNull,
];
