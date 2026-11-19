// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Small pictures of the tabs, for the Orbit view (design/ui-final/SPEC.md section 6.1).
//!
//! The policy (when to take a picture, what to refuse), the shrinking and the memory-capped store
//! are in `crates/mercury-tab-snapshots`, which needs no Servo. This file is the glue: it reads the
//! pixels from the web view's rendering context, hands them to one worker thread to shrink, and
//! keeps the result in a [`SnapshotRegistry`] in a thread-local, like `activation.rs`.
//!
//! Servo draws every tab into one shared rendering context, so the context holds the last frame
//! that was presented, whichever tab drew it. Three hooks in `engine.rs` keep that true:
//! [`frame_presented`] after a frame was shown, [`before_tab_switch`] before the new tab paints
//! into the context (which overwrites the old tab's frame), and [`forget_tab`] when a tab closes.
//! The registry refuses a capture that would attach one tab's picture to another.
//!
//! Pictures are kept in memory only. Nothing is written to disk, except by the diagnostic
//! `MERCURY_TAB_SNAPSHOT_DUMP_DIR`, which works in automated runs only.
//!
//! There are no snapshots on Linux yet: its rendering context cannot read pixels back, so the
//! registry gives up after three failed captures and Orbit shows placeholders.

use std::cell::RefCell;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use euclid::default::Point2D;
use image::{ExtendedColorType, ImageFormat};
use mercury_tab_snapshots::{
    CaptureReason, CaptureRequest, DEFAULT_CAP_BYTES, FinishOutcome, FrameError, Limits, RgbaFrame,
    Skip, Snapshot, SnapshotRegistry, Stats, downscale,
};
use servo::{DeviceIntRect, RenderingContext};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use talaria_core::TabId;

use crate::rendering::BrowserRenderingContext;

/// In automated runs only: a directory (it must exist) to write each snapshot into as a PNG.
const DUMP_DIR_VARIABLE: &str = "MERCURY_TAB_SNAPSHOT_DUMP_DIR";
/// In automated runs only: replaces the memory cap (bytes), to check eviction with a small one.
const CAP_BYTES_VARIABLE: &str = "MERCURY_TAB_SNAPSHOT_CAP_BYTES";

thread_local! {
    /// The window's snapshots, made when the first hook runs. `None` until then.
    static CAPTURER: RefCell<Option<Capturer>> = const { RefCell::new(None) };
}

/// Time since the first call, the clock the registry is given (it never reads one itself).
fn now() -> Duration {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

// ---------------------------------------------------------------------------------------------
// The hooks (called from engine.rs) and the API for the Orbit view
// ---------------------------------------------------------------------------------------------

/// Hook: a frame of `tab` was just shown, so the context holds exactly that frame. Takes a
/// picture if the tab has been active for 30 seconds since its last one.
pub(crate) fn frame_presented(context: &BrowserRenderingContext, tab: TabId) {
    with_capturer(|capturer| capturer.frame_presented(context, tab, now()));
}

/// Hook: the active tab is about to change. Must run before the new tab paints into the context.
/// `leaving` is the tab that was on screen, if any; with `None` nothing was on screen (the first
/// switch, or after the rendering context was made again), so what the registry remembers about
/// the context is stale and is dropped.
pub(crate) fn before_tab_switch(context: &BrowserRenderingContext, leaving: Option<TabId>) {
    with_capturer(|capturer| capturer.before_tab_switch(context, leaving, now()));
}

/// Hook: `tab` was closed. Its picture goes, and a capture in flight for it is thrown away.
pub(crate) fn forget_tab(tab: TabId) {
    with_existing_capturer(|capturer| capturer.forget(tab));
}

/// Takes a picture of the active tab now, for the Orbit view opening over the page, so that the
/// picture is fresh without waiting for the 30 second timer. The tab must be the one whose frame
/// was presented last; otherwise nothing is taken.
#[allow(dead_code)] // used by the orbit-view lane
pub(crate) fn capture_active_now(context: &BrowserRenderingContext, tab: TabId) {
    with_capturer(|capturer| capturer.capture_now(context, tab, now()));
}

/// The tab's picture as a Slint image (at most 480 by 300 pixels), or `None` if there is none
/// (never shown, evicted, or captures are unavailable). The pixels are copied on every call, so
/// the caller should keep the `Image` for as long as it shows it. Marks the picture as recently
/// used, so a picture Orbit shows is not the next one evicted.
#[allow(dead_code)] // used by the orbit-view lane
pub(crate) fn image_for(tab: TabId) -> Option<Image> {
    with_existing_capturer(|capturer| capturer.image_for(tab)).flatten()
}

/// Changes the memory cap in bytes (the seam for the setting that will come later); returns how
/// many pictures were evicted at once.
#[allow(dead_code)] // used by the orbit-view lane
pub(crate) fn set_cap_bytes(bytes: usize) -> usize {
    with_capturer(|capturer| capturer.registry.set_cap_bytes(bytes)).unwrap_or(0)
}

/// Counters for logs and tests.
#[allow(dead_code)] // used by the orbit-view lane
pub(crate) fn stats() -> Stats {
    with_existing_capturer(|capturer| capturer.registry.stats()).unwrap_or_default()
}

/// Runs `use_capturer`, making the state first if this is the first hook. `None` if the state is
/// borrowed (a hook reached from inside another one; it skips) or gone (the thread is ending).
/// The hooks run inside the rendering notifier, so nothing here may panic.
fn with_capturer<R>(use_capturer: impl FnOnce(&mut Capturer) -> R) -> Option<R> {
    CAPTURER
        .try_with(|cell| {
            let mut slot = cell.try_borrow_mut().ok()?;
            Some(use_capturer(
                slot.get_or_insert_with(Capturer::for_this_process),
            ))
        })
        .ok()
        .flatten()
}

/// Like [`with_capturer`], but does nothing if no hook has made the state yet.
fn with_existing_capturer<R>(use_capturer: impl FnOnce(&mut Capturer) -> R) -> Option<R> {
    CAPTURER
        .try_with(|cell| {
            let mut slot = cell.try_borrow_mut().ok()?;
            Some(use_capturer(slot.as_mut()?))
        })
        .ok()
        .flatten()
}

/// Back on the main thread: the worker's result goes to the registry.
fn finish_capture(done: Done) {
    with_existing_capturer(|capturer| capturer.finish(done, now()));
}

// ---------------------------------------------------------------------------------------------
// Reading pixels
// ---------------------------------------------------------------------------------------------

/// Where a picture of the page comes from: the web view's rendering context, or a fake in tests.
trait PixelSource {
    /// The size in pixels of the surface that `read` would read.
    fn surface_size(&self) -> (u32, u32);
    /// The whole surface as width, height and RGBA8 pixels (top row first), or `None` if it cannot
    /// be read. Must be called on the main thread (the GL context is bound to it).
    fn read(&self) -> Option<(u32, u32, Vec<u8>)>;
}

impl PixelSource for BrowserRenderingContext {
    fn surface_size(&self) -> (u32, u32) {
        let size = self.size2d();
        (size.width, size.height)
    }

    /// The same read as the frame read-back in `rendering/metal.rs`. `None` on Linux.
    fn read(&self) -> Option<(u32, u32, Vec<u8>)> {
        let size = self.size2d().to_i32();
        let image = self.read_to_image(DeviceIntRect::from_origin_and_size(
            Point2D::origin().cast_unit(),
            size.cast_unit(),
        ))?;
        let (width, height) = image.dimensions();
        Some((width, height, image.into_raw()))
    }
}

/// Why a read gave no usable picture.
#[derive(Debug, PartialEq, Eq)]
enum ReadFailure {
    /// The context returned nothing (Linux, or the read failed).
    Unavailable,
    /// The picture is not the size the request was made for, so it is not that tab's frame.
    WrongSize { wanted: (u32, u32), got: (u32, u32) },
    /// The pixels do not make a picture.
    NotAPicture(FrameError),
}

impl fmt::Display for ReadFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("the rendering context returned no pixels"),
            Self::WrongSize { wanted, got } => write!(
                formatter,
                "the context gave {}x{} pixels, but the frame was {}x{}",
                got.0, got.1, wanted.0, wanted.1
            ),
            Self::NotAPicture(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

/// Reads the surface and checks that it is the `wanted` size (the size of the frame the registry
/// approved; a different size means the context was resized and its pixels are not the tab's).
fn read_frame(source: &impl PixelSource, wanted: (u32, u32)) -> Result<RgbaFrame, ReadFailure> {
    let (width, height, pixels) = source.read().ok_or(ReadFailure::Unavailable)?;
    if (width, height) != wanted {
        return Err(ReadFailure::WrongSize {
            wanted,
            got: (width, height),
        });
    }
    RgbaFrame::new(width, height, pixels).map_err(ReadFailure::NotAPicture)
}

// ---------------------------------------------------------------------------------------------
// The worker: shrinking off the main thread
// ---------------------------------------------------------------------------------------------

/// A full frame to shrink.
struct Job {
    request: CaptureRequest<TabId>,
    frame: RgbaFrame,
    limits: Limits,
    captured_at: Duration,
    /// How long the read on the main thread took, for the log.
    read_time: Duration,
    /// Where to write the picture as a PNG too (diagnostics only).
    dump_dir: Option<PathBuf>,
}

/// What the worker found: the shrunk picture, or `None` if shrinking failed.
struct Done {
    request: CaptureRequest<TabId>,
    snapshot: Option<Snapshot>,
    read_time: Duration,
    shrink_time: Duration,
}

/// The picture made from `frame`: shrunk to fit `limits`, remembering the size it came from.
fn make_snapshot(frame: RgbaFrame, limits: Limits, captured_at: Duration) -> Snapshot {
    let source_size = (frame.width(), frame.height());
    Snapshot::new(downscale(frame, limits), captured_at, source_size)
}

/// The worker's job. A panic while shrinking is a failure result, never a request left in flight.
fn run_job(job: Job) -> Done {
    let Job {
        request,
        frame,
        limits,
        captured_at,
        read_time,
        dump_dir,
    } = job;
    let started = Instant::now();
    let made = catch_unwind(AssertUnwindSafe(|| {
        make_snapshot(frame, limits, captured_at)
    }));
    let shrink_time = started.elapsed();
    let snapshot = match made {
        Ok(snapshot) => Some(snapshot),
        Err(_) => {
            log::warn!("tab snapshots: shrinking a frame failed");
            None
        }
    };
    if let (Some(snapshot), Some(dir)) = (&snapshot, &dump_dir) {
        write_dump(dir, request, snapshot);
    }
    Done {
        request,
        snapshot,
        read_time,
        shrink_time,
    }
}

/// The diagnostic dump: the snapshot as a PNG in `dir`. The only place a snapshot reaches the disk.
fn write_dump(dir: &Path, request: CaptureRequest<TabId>, snapshot: &Snapshot) {
    let path = dir.join(dump_file_name(
        request.key(),
        request.reason(),
        request.ticket(),
    ));
    let frame = snapshot.frame();
    if let Err(error) = image::save_buffer_with_format(
        &path,
        frame.pixels(),
        frame.width(),
        frame.height(),
        ExtendedColorType::Rgba8,
        ImageFormat::Png,
    ) {
        log::warn!("tab snapshots: could not write {}: {error}", path.display());
    }
}

/// `tab-<id>-<reason>-<n>.png`; `n` is the request's number, different for every capture.
fn dump_file_name(tab: TabId, reason: CaptureReason, n: u64) -> String {
    let reason = match reason {
        CaptureReason::Leaving => "leaving",
        CaptureReason::Periodic => "periodic",
        CaptureReason::OnDemand => "on-demand",
    };
    format!("tab-{}-{reason}-{n}.png", tab.value())
}

/// Takes a job to another thread. At most one capture is in flight (the registry sees to that),
/// so a refusal means the worker is not running.
trait Worker {
    fn submit(&self, job: Job) -> Result<(), WorkerRefused>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerRefused {
    /// The worker has a job waiting already.
    Busy,
    /// The worker is not running (it could not be started, or it ended).
    Stopped,
}

impl fmt::Display for WorkerRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Busy => "the snapshot worker is busy",
            Self::Stopped => "the snapshot worker is not running",
        })
    }
}

/// The one worker thread, fed through a channel that holds one job.
struct ThreadWorker {
    jobs: SyncSender<Job>,
}

impl Worker for ThreadWorker {
    fn submit(&self, job: Job) -> Result<(), WorkerRefused> {
        self.jobs.try_send(job).map_err(|error| match error {
            TrySendError::Full(_) => WorkerRefused::Busy,
            TrySendError::Disconnected(_) => WorkerRefused::Stopped,
        })
    }
}

/// Starts the worker thread. It ends when the sender is dropped.
fn start_worker() -> Option<ThreadWorker> {
    let (jobs, receiver) = sync_channel::<Job>(1);
    std::thread::Builder::new()
        .name("mercury-tab-snapshots".to_owned())
        .spawn(move || {
            for job in receiver {
                post_result(run_job(job));
            }
        })
        .ok()
        .map(|_thread| ThreadWorker { jobs })
}

/// Sends the result to the main thread, where the registry lives.
fn post_result(done: Done) {
    // An error means the event loop has ended (the browser is quitting): nothing to deliver to.
    let _queued = slint::invoke_from_event_loop(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        finish_capture(done);
    });
}

/// Stands in when the thread could not be started: every capture fails, and the registry switches
/// itself off after three.
struct NoWorker;

impl Worker for NoWorker {
    fn submit(&self, _job: Job) -> Result<(), WorkerRefused> {
        Err(WorkerRefused::Stopped)
    }
}

// ---------------------------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------------------------

/// The two switches for automated runs.
#[derive(Debug, Default, PartialEq, Eq)]
struct Diagnostics {
    dump_dir: Option<PathBuf>,
    cap_bytes: Option<usize>,
}

/// A line for the log, decided apart from logging so that it can be tested.
#[derive(Debug, PartialEq, Eq)]
struct Notice {
    level: log::Level,
    message: String,
}

impl Notice {
    fn warn(message: String) -> Self {
        Self {
            level: log::Level::Warn,
            message,
        }
    }

    fn info(message: String) -> Self {
        Self {
            level: log::Level::Info,
            message,
        }
    }
}

/// Reads the two switches, which count only in automated runs (`--screenshot` or `--background`,
/// as `MERCURY_ALLOW_POP_UPS` does); anywhere else they are ignored with a warning.
fn read_diagnostics(
    automated: bool,
    dump_dir: Option<PathBuf>,
    cap_bytes: Option<String>,
    is_directory: impl Fn(&Path) -> bool,
) -> (Diagnostics, Vec<Notice>) {
    let mut diagnostics = Diagnostics::default();
    let mut notices = Vec::new();
    if let Some(dir) = dump_dir {
        if !automated {
            notices.push(Notice::warn(format!(
                "{DUMP_DIR_VARIABLE} is ignored outside --screenshot and --background runs"
            )));
        } else if !is_directory(&dir) {
            notices.push(Notice::warn(format!(
                "{DUMP_DIR_VARIABLE} is not an existing directory; no tab snapshots are written"
            )));
        } else {
            notices.push(Notice::warn(format!(
                "{DUMP_DIR_VARIABLE}: pictures of the open pages are written to disk in {} in \
                 this automated run",
                dir.display()
            )));
            diagnostics.dump_dir = Some(dir);
        }
    }
    if let Some(value) = cap_bytes {
        if !automated {
            notices.push(Notice::warn(format!(
                "{CAP_BYTES_VARIABLE} is ignored outside --screenshot and --background runs"
            )));
        } else if let Ok(bytes) = value.trim().parse::<usize>() {
            notices.push(Notice::info(format!(
                "{CAP_BYTES_VARIABLE}: the tab snapshot memory cap is {bytes} bytes in this \
                 automated run"
            )));
            diagnostics.cap_bytes = Some(bytes);
        } else {
            notices.push(Notice::warn(format!(
                "{CAP_BYTES_VARIABLE} is not a number of bytes and is ignored"
            )));
        }
    }
    (diagnostics, notices)
}

// ---------------------------------------------------------------------------------------------
// The state
// ---------------------------------------------------------------------------------------------

/// The registry with the worker and the diagnostics. Every decision is the registry's; this only
/// reads the pixels, hands them over and logs.
struct Capturer {
    registry: SnapshotRegistry<TabId>,
    worker: Box<dyn Worker>,
    diagnostics: Diagnostics,
    /// The two warnings that would otherwise repeat on every frame, given once.
    reported_unavailable: bool,
    reported_disabled: bool,
}

impl Capturer {
    /// The state for this process: switches from the environment, the worker thread.
    fn for_this_process() -> Self {
        let automated = mercury_site_security::is_automated_run(
            std::env::args_os().map(|argument| argument.to_string_lossy().into_owned()),
        );
        let (diagnostics, notices) = read_diagnostics(
            automated,
            std::env::var_os(DUMP_DIR_VARIABLE).map(PathBuf::from),
            std::env::var(CAP_BYTES_VARIABLE).ok(),
            Path::is_dir,
        );
        for notice in notices {
            log::log!(notice.level, "{}", notice.message);
        }
        let worker: Box<dyn Worker> = match start_worker() {
            Some(worker) => Box::new(worker),
            None => {
                log::warn!("tab snapshots: could not start the worker thread");
                Box::new(NoWorker)
            }
        };
        Self::new(diagnostics, worker)
    }

    fn new(diagnostics: Diagnostics, worker: Box<dyn Worker>) -> Self {
        let cap_bytes = diagnostics.cap_bytes.unwrap_or(DEFAULT_CAP_BYTES);
        Self {
            registry: SnapshotRegistry::new(cap_bytes, Limits::DEFAULT),
            worker,
            diagnostics,
            reported_unavailable: false,
            reported_disabled: false,
        }
    }

    fn frame_presented(&mut self, source: &impl PixelSource, tab: TabId, now: Duration) {
        let decision = self
            .registry
            .frame_presented(tab, source.surface_size(), now);
        // A refusal is the usual answer and is not logged: this runs on every presented frame.
        self.begin_capture(source, decision, now);
    }

    fn before_tab_switch(
        &mut self,
        source: &impl PixelSource,
        leaving: Option<TabId>,
        now: Duration,
    ) {
        let Some(tab) = leaving else {
            self.registry.rendering_changed();
            return;
        };
        let decision = self.registry.tab_leaving(tab, source.surface_size(), now);
        if let Some(skip) = self.begin_capture(source, decision, now) {
            log::debug!("tab snapshot of tab {} not taken: {skip:?}", tab.value());
        }
    }

    fn capture_now(&mut self, source: &impl PixelSource, tab: TabId, now: Duration) {
        let decision = self.registry.capture_now(tab, source.surface_size(), now);
        if let Some(skip) = self.begin_capture(source, decision, now) {
            log::debug!("tab snapshot of tab {} not taken: {skip:?}", tab.value());
        }
    }

    fn forget(&mut self, tab: TabId) {
        self.registry.forget(tab);
    }

    fn image_for(&mut self, tab: TabId) -> Option<Image> {
        self.registry.get(tab).map(snapshot_image)
    }

    /// Acts on the registry's decision: reads the frame and hands it to the worker. Returns why
    /// nothing was started, if it was refused.
    fn begin_capture(
        &mut self,
        source: &impl PixelSource,
        decision: Result<CaptureRequest<TabId>, Skip>,
        now: Duration,
    ) -> Option<Skip> {
        let request = match decision {
            Ok(request) => request,
            Err(skip) => {
                if skip == Skip::Disabled && !std::mem::replace(&mut self.reported_disabled, true) {
                    log::warn!(
                        "tab snapshots are switched off after repeated failures; \
                         Orbit will show placeholders"
                    );
                }
                return Some(skip);
            }
        };
        let started = Instant::now();
        let frame = match read_frame(source, request.surface()) {
            Ok(frame) => frame,
            Err(failure) => {
                self.fail(request, now, &failure.to_string());
                return None;
            }
        };
        let job = Job {
            request,
            frame,
            limits: self.registry.limits(),
            captured_at: now,
            read_time: started.elapsed(),
            dump_dir: self.diagnostics.dump_dir.clone(),
        };
        if let Err(refused) = self.worker.submit(job) {
            self.fail(request, now, &refused.to_string());
        }
        None
    }

    /// A capture that could not be made: the registry counts it (and backs off).
    fn fail(&mut self, request: CaptureRequest<TabId>, now: Duration, cause: &str) {
        let outcome = self.registry.finish(request, None, now);
        if outcome == FinishOutcome::Failed
            && !std::mem::replace(&mut self.reported_unavailable, true)
        {
            log::warn!("tab snapshots are unavailable on this rendering path ({cause})");
        } else {
            log::debug!(
                "tab snapshot of tab {} failed: {cause}",
                request.key().value()
            );
        }
    }

    /// The worker's result is back on the main thread. The registry stores the picture only if
    /// the request is still wanted: a tab closed meanwhile never gets one.
    fn finish(&mut self, done: Done, now: Duration) -> FinishOutcome {
        let Done {
            request,
            snapshot,
            read_time,
            shrink_time,
        } = done;
        let picture = snapshot
            .as_ref()
            .map(|snapshot| (snapshot.frame().width(), snapshot.frame().height()));
        let outcome = self.registry.finish(request, snapshot, now);
        let stats = self.registry.stats();
        log::debug!(
            "tab snapshot {outcome:?}: {:?} of tab {}, read {:.1} ms, shrink {:.1} ms, \
             picture {picture:?}, {} stored ({} bytes of {}), {} evicted so far",
            request.reason(),
            request.key().value(),
            read_time.as_secs_f64() * 1000.0,
            shrink_time.as_secs_f64() * 1000.0,
            stats.entries,
            stats.used_bytes,
            stats.cap_bytes,
            stats.evicted,
        );
        outcome
    }
}

/// A copy of the snapshot's pixels as a Slint image.
fn snapshot_image(snapshot: &Snapshot) -> Image {
    let frame = snapshot.frame();
    Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        frame.pixels(),
        frame.width(),
        frame.height(),
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use mercury_tab_snapshots::{MAX_HEIGHT, MAX_WIDTH, PERIODIC_INTERVAL};

    use super::*;

    const PALE_CYAN: [u8; 4] = [221, 255, 255, 255];
    const YELLOW: [u8; 4] = [255, 255, 0, 255];

    fn tab(value: u64) -> TabId {
        TabId::new(value)
    }

    fn seconds(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    /// A rendering context that holds one solid colour at a size, or nothing.
    struct FakeSource {
        size: Cell<(u32, u32)>,
        colour: Cell<[u8; 4]>,
        readable: Cell<bool>,
        /// When set, `read` gives a picture of this size instead of the surface's.
        read_size: Cell<Option<(u32, u32)>>,
        reads: Cell<u32>,
    }

    impl FakeSource {
        fn new(width: u32, height: u32, colour: [u8; 4]) -> Self {
            Self {
                size: Cell::new((width, height)),
                colour: Cell::new(colour),
                readable: Cell::new(true),
                read_size: Cell::new(None),
                reads: Cell::new(0),
            }
        }
    }

    impl PixelSource for FakeSource {
        fn surface_size(&self) -> (u32, u32) {
            self.size.get()
        }

        fn read(&self) -> Option<(u32, u32, Vec<u8>)> {
            self.reads.set(self.reads.get() + 1);
            if !self.readable.get() {
                return None;
            }
            let (width, height) = self.read_size.get().unwrap_or(self.size.get());
            Some((
                width,
                height,
                self.colour.get().repeat(width as usize * height as usize),
            ))
        }
    }

    /// Runs the job on the calling thread and keeps the result for the test to deliver, so that
    /// the test decides when the result comes back to the registry.
    struct InlineWorker {
        results: Rc<RefCell<Vec<Done>>>,
        refuse: Rc<Cell<Option<WorkerRefused>>>,
    }

    impl Worker for InlineWorker {
        fn submit(&self, job: Job) -> Result<(), WorkerRefused> {
            if let Some(refused) = self.refuse.get() {
                return Err(refused);
            }
            self.results.borrow_mut().push(run_job(job));
            Ok(())
        }
    }

    struct Fixture {
        capturer: Capturer,
        results: Rc<RefCell<Vec<Done>>>,
        refuse: Rc<Cell<Option<WorkerRefused>>>,
    }

    fn fixture(diagnostics: Diagnostics) -> Fixture {
        let results = Rc::new(RefCell::new(Vec::new()));
        let refuse = Rc::new(Cell::new(None));
        let worker = InlineWorker {
            results: Rc::clone(&results),
            refuse: Rc::clone(&refuse),
        };
        Fixture {
            capturer: Capturer::new(diagnostics, Box::new(worker)),
            results,
            refuse,
        }
    }

    impl Fixture {
        /// Delivers every result the worker has made, as the event loop would.
        fn deliver(&mut self, now: Duration) -> Vec<FinishOutcome> {
            let results = std::mem::take(&mut *self.results.borrow_mut());
            results
                .into_iter()
                .map(|done| self.capturer.finish(done, now))
                .collect()
        }

        fn pending(&self) -> usize {
            self.results.borrow().len()
        }

        fn stored(&self, key: TabId) -> Option<Snapshot> {
            self.capturer.registry.peek(key).cloned()
        }
    }

    fn pixel_of(snapshot: &Snapshot, index: usize) -> [u8; 4] {
        let pixels = snapshot.frame().pixels();
        [
            pixels[index * 4],
            pixels[index * 4 + 1],
            pixels[index * 4 + 2],
            pixels[index * 4 + 3],
        ]
    }

    // ----- reading -----

    #[test]
    fn a_picture_of_the_wanted_size_becomes_a_frame() {
        let source = FakeSource::new(8, 4, PALE_CYAN);

        let frame = read_frame(&source, (8, 4)).expect("the size matches");

        assert_eq!((frame.width(), frame.height()), (8, 4));
        assert_eq!(&frame.pixels()[..4], &PALE_CYAN);
    }

    #[test]
    fn a_picture_of_another_size_is_a_failure() {
        let source = FakeSource::new(8, 4, PALE_CYAN);
        source.read_size.set(Some((8, 5)));

        assert_eq!(
            read_frame(&source, (8, 4)).unwrap_err(),
            ReadFailure::WrongSize {
                wanted: (8, 4),
                got: (8, 5)
            }
        );
    }

    #[test]
    fn nothing_to_read_is_a_failure() {
        let source = FakeSource::new(8, 4, PALE_CYAN);
        source.readable.set(false);

        assert_eq!(
            read_frame(&source, (8, 4)).unwrap_err(),
            ReadFailure::Unavailable
        );
    }

    #[test]
    fn pixels_that_are_not_a_picture_are_a_failure() {
        struct Short;
        impl PixelSource for Short {
            fn surface_size(&self) -> (u32, u32) {
                (2, 2)
            }
            fn read(&self) -> Option<(u32, u32, Vec<u8>)> {
                Some((2, 2, vec![0; 15]))
            }
        }

        assert!(matches!(
            read_frame(&Short, (2, 2)),
            Err(ReadFailure::NotAPicture(FrameError::LengthMismatch {
                expected: 16,
                actual: 15
            }))
        ));
    }

    // ----- the worker's job -----

    fn job(width: u32, height: u32, colour: [u8; 4], dump_dir: Option<PathBuf>) -> Job {
        Job {
            request: CaptureRequest::new(tab(1), CaptureReason::Leaving, (width, height), 9),
            frame: RgbaFrame::new(
                width,
                height,
                colour.repeat(width as usize * height as usize),
            )
            .expect("a checked size"),
            limits: Limits::DEFAULT,
            captured_at: seconds(12),
            read_time: Duration::from_millis(4),
            dump_dir,
        }
    }

    #[test]
    fn the_job_shrinks_a_frame_and_remembers_where_it_came_from() {
        let done = run_job(job(1200, 800, PALE_CYAN, None));

        let snapshot = done.snapshot.expect("a frame shrinks");
        assert_eq!(
            (snapshot.frame().width(), snapshot.frame().height()),
            (450, 300)
        );
        assert_eq!(snapshot.source_size(), (1200, 800));
        assert_eq!(snapshot.captured_at(), seconds(12));
        assert_eq!(pixel_of(&snapshot, 0), PALE_CYAN);
        assert_eq!(done.request.ticket(), 9);
        assert_eq!(done.read_time, Duration::from_millis(4));
    }

    #[test]
    fn a_frame_that_already_fits_is_not_changed() {
        let done = run_job(job(MAX_WIDTH, MAX_HEIGHT, YELLOW, None));

        let snapshot = done.snapshot.expect("a frame shrinks");
        assert_eq!(
            (snapshot.frame().width(), snapshot.frame().height()),
            (MAX_WIDTH, MAX_HEIGHT)
        );
        assert_eq!(pixel_of(&snapshot, 0), YELLOW);
    }

    #[test]
    fn the_job_writes_a_png_only_when_asked_to() {
        let directory = tempfile::tempdir().expect("a temporary directory");

        run_job(job(1200, 800, PALE_CYAN, None));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);

        run_job(job(
            1200,
            800,
            PALE_CYAN,
            Some(directory.path().to_path_buf()),
        ));
        let written = directory.path().join("tab-1-leaving-9.png");
        assert_eq!(
            image::image_dimensions(&written).expect("a readable PNG"),
            (450, 300)
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn dump_files_are_named_by_tab_reason_and_request() {
        assert_eq!(
            dump_file_name(tab(3), CaptureReason::Leaving, 7),
            "tab-3-leaving-7.png"
        );
        assert_eq!(
            dump_file_name(tab(12), CaptureReason::Periodic, 1),
            "tab-12-periodic-1.png"
        );
        assert_eq!(
            dump_file_name(tab(1), CaptureReason::OnDemand, 40),
            "tab-1-on-demand-40.png"
        );
    }

    // ----- the Slint image -----

    #[test]
    fn a_snapshot_becomes_an_image_with_the_same_pixels() {
        let pixels = vec![10, 20, 30, 255, 40, 50, 60, 255];
        let frame = RgbaFrame::new(2, 1, pixels.clone()).expect("2x1 needs 8 bytes");
        let snapshot = Snapshot::new(frame, Duration::ZERO, (200, 100));

        let image = snapshot_image(&snapshot);

        assert_eq!((image.size().width, image.size().height), (2, 1));
        let copy = image.to_rgba8().expect("a pixel buffer image");
        assert_eq!(copy.as_bytes(), &pixels[..]);
    }

    // ----- the two switches -----

    fn existing(_: &Path) -> bool {
        true
    }

    fn missing(_: &Path) -> bool {
        false
    }

    #[test]
    fn the_switches_are_ignored_outside_automated_runs() {
        let (diagnostics, notices) = read_diagnostics(
            false,
            Some(PathBuf::from("/tmp/dump")),
            Some("1000".to_owned()),
            existing,
        );

        assert_eq!(diagnostics, Diagnostics::default());
        assert_eq!(notices.len(), 2);
        assert!(
            notices
                .iter()
                .all(|notice| notice.level == log::Level::Warn)
        );
        assert!(notices[0].message.contains(DUMP_DIR_VARIABLE));
        assert!(notices[1].message.contains(CAP_BYTES_VARIABLE));
    }

    #[test]
    fn unset_switches_change_nothing_and_say_nothing() {
        for automated in [false, true] {
            let (diagnostics, notices) = read_diagnostics(automated, None, None, existing);
            assert_eq!(diagnostics, Diagnostics::default());
            assert!(notices.is_empty());
        }
    }

    #[test]
    fn in_an_automated_run_the_dump_directory_is_used_and_a_warning_says_so() {
        let (diagnostics, notices) =
            read_diagnostics(true, Some(PathBuf::from("/tmp/dump")), None, |path| {
                path == Path::new("/tmp/dump")
            });

        assert_eq!(diagnostics.dump_dir, Some(PathBuf::from("/tmp/dump")));
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].level, log::Level::Warn);
        assert!(notices[0].message.contains("written to disk"));
    }

    #[test]
    fn a_dump_directory_that_does_not_exist_is_not_used() {
        let (diagnostics, notices) =
            read_diagnostics(true, Some(PathBuf::from("/nowhere")), None, missing);

        assert_eq!(diagnostics.dump_dir, None);
        assert_eq!(notices.len(), 1);
        assert!(notices[0].message.contains("not an existing directory"));
    }

    #[test]
    fn in_an_automated_run_the_cap_can_be_replaced() {
        let (diagnostics, notices) =
            read_diagnostics(true, None, Some(" 1200000\n".to_owned()), existing);

        assert_eq!(diagnostics.cap_bytes, Some(1_200_000));
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].level, log::Level::Info);
    }

    #[test]
    fn a_cap_that_is_not_a_number_is_ignored_with_a_warning() {
        for value in ["", "12 MB", "-5", "1e6"] {
            let (diagnostics, notices) =
                read_diagnostics(true, None, Some(value.to_owned()), existing);
            assert_eq!(diagnostics.cap_bytes, None, "{value:?}");
            assert_eq!(notices.len(), 1);
            assert_eq!(notices[0].level, log::Level::Warn);
        }
    }

    #[test]
    fn the_cap_switch_sets_the_registry_cap_and_the_default_is_128_million_bytes() {
        let small = fixture(Diagnostics {
            dump_dir: None,
            cap_bytes: Some(1_200_000),
        });
        let default = fixture(Diagnostics::default());

        assert_eq!(small.capturer.registry.stats().cap_bytes, 1_200_000);
        assert_eq!(default.capturer.registry.stats().cap_bytes, 128_000_000);
    }

    // ----- decide, read, shrink, finish -----

    #[test]
    fn a_tab_that_stays_active_gets_a_picture_every_thirty_seconds() {
        let mut fixture = fixture(Diagnostics::default());
        let source = FakeSource::new(1200, 800, PALE_CYAN);

        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(5));
        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(20));
        assert_eq!(source.reads.get(), 0, "not due yet: nothing is read");

        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(5) + PERIODIC_INTERVAL);
        assert_eq!(source.reads.get(), 1);
        assert_eq!(fixture.pending(), 1);
        assert_eq!(
            fixture.deliver(seconds(36)),
            vec![FinishOutcome::Stored { evicted: 0 }]
        );

        let snapshot = fixture.stored(tab(1)).expect("the picture is stored");
        assert_eq!(
            (snapshot.frame().width(), snapshot.frame().height()),
            (450, 300)
        );
        assert_eq!(pixel_of(&snapshot, 0), PALE_CYAN);
        assert_eq!(fixture.capturer.registry.stats().stored, 1);
    }

    #[test]
    fn leaving_a_tab_takes_its_picture_before_the_next_tab_paints() {
        let mut fixture = fixture(Diagnostics::default());
        let source = FakeSource::new(1200, 800, PALE_CYAN);
        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(10));

        // The engine calls the hook while the context still holds tab 1's frame ...
        fixture
            .capturer
            .before_tab_switch(&source, Some(tab(1)), seconds(11));
        // ... and tab 2 paints (yellow) only after it.
        source.colour.set(YELLOW);
        fixture
            .capturer
            .frame_presented(&source, tab(2), seconds(12));
        fixture.deliver(seconds(12));

        let picture = fixture.stored(tab(1)).expect("tab 1 has its picture");
        assert_eq!(pixel_of(&picture, 0), PALE_CYAN, "no yellow on tab 1");
        assert!(fixture.stored(tab(2)).is_none());
    }

    #[test]
    fn a_tab_that_was_not_the_one_shown_is_never_captured() {
        let mut fixture = fixture(Diagnostics::default());
        let source = FakeSource::new(1200, 800, YELLOW);
        fixture
            .capturer
            .frame_presented(&source, tab(2), seconds(10));

        // Tab 1 is said to be leaving, but the context holds tab 2's frame.
        fixture
            .capturer
            .before_tab_switch(&source, Some(tab(1)), seconds(11));

        assert_eq!(source.reads.get(), 0);
        assert_eq!(fixture.pending(), 0);
        assert!(fixture.stored(tab(1)).is_none());
    }

    #[test]
    fn a_resized_surface_is_not_captured() {
        let mut fixture = fixture(Diagnostics::default());
        let source = FakeSource::new(1200, 800, PALE_CYAN);
        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(10));

        source.size.set((1200, 700));
        fixture
            .capturer
            .before_tab_switch(&source, Some(tab(1)), seconds(11));

        assert_eq!(source.reads.get(), 0);
        assert!(fixture.stored(tab(1)).is_none());
    }

    #[test]
    fn nothing_on_screen_before_the_switch_forgets_what_the_context_held() {
        let mut fixture = fixture(Diagnostics::default());
        let source = FakeSource::new(1200, 800, PALE_CYAN);
        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(10));

        // The rendering context was made again: the engine says nothing was visible.
        fixture
            .capturer
            .before_tab_switch(&source, None, seconds(11));
        fixture
            .capturer
            .before_tab_switch(&source, Some(tab(1)), seconds(12));

        assert_eq!(source.reads.get(), 0);
    }

    #[test]
    fn a_tab_closed_while_its_picture_is_being_shrunk_never_gets_one() {
        let mut fixture = fixture(Diagnostics::default());
        let source = FakeSource::new(1200, 800, PALE_CYAN);
        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(10));
        fixture
            .capturer
            .before_tab_switch(&source, Some(tab(1)), seconds(11));
        assert_eq!(fixture.pending(), 1, "the capture is in flight");

        fixture.capturer.forget(tab(1));

        assert_eq!(fixture.deliver(seconds(12)), vec![FinishOutcome::Discarded]);
        assert!(fixture.stored(tab(1)).is_none());
        assert!(fixture.capturer.image_for(tab(1)).is_none());
        assert_eq!(fixture.capturer.registry.stats().discarded, 1);
    }

    #[test]
    fn the_image_for_a_tab_is_its_stored_picture() {
        let mut fixture = fixture(Diagnostics::default());
        let source = FakeSource::new(480, 300, YELLOW);
        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(10));
        fixture.capturer.capture_now(&source, tab(1), seconds(11));
        fixture.deliver(seconds(11));

        let image = fixture
            .capturer
            .image_for(tab(1))
            .expect("a stored picture");

        assert_eq!((image.size().width, image.size().height), (480, 300));
        assert_eq!(&image.to_rgba8().unwrap().as_bytes()[..4], &YELLOW);
        assert!(fixture.capturer.image_for(tab(2)).is_none());
    }

    #[test]
    fn a_failed_read_is_counted_and_waits_out_the_backoff() {
        let mut fixture = fixture(Diagnostics::default());
        let source = FakeSource::new(1200, 800, PALE_CYAN);
        source.readable.set(false);
        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(10));

        fixture
            .capturer
            .before_tab_switch(&source, Some(tab(1)), seconds(11));
        assert_eq!(source.reads.get(), 1);
        assert_eq!(fixture.capturer.registry.stats().failed, 1);
        assert!(fixture.capturer.reported_unavailable);

        // Inside the five seconds of backoff nothing is read again.
        fixture.capturer.capture_now(&source, tab(1), seconds(12));
        assert_eq!(source.reads.get(), 1);
    }

    #[test]
    fn captures_switch_themselves_off_after_three_failed_reads() {
        let mut fixture = fixture(Diagnostics::default());
        let source = FakeSource::new(1200, 800, PALE_CYAN);
        source.readable.set(false);
        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(10));

        for attempt in 0..6 {
            fixture
                .capturer
                .capture_now(&source, tab(1), seconds(20 + attempt * 10));
        }

        assert_eq!(source.reads.get(), 3, "no read after the third failure");
        assert_eq!(fixture.capturer.registry.stats().failed, 3);
        assert!(fixture.capturer.reported_disabled);
        assert_eq!(fixture.capturer.registry.stats().entries, 0);
    }

    #[test]
    fn a_picture_of_the_wrong_size_is_a_failure_and_is_not_stored() {
        let mut fixture = fixture(Diagnostics::default());
        let source = FakeSource::new(1200, 800, PALE_CYAN);
        source.read_size.set(Some((1200, 799)));
        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(10));

        fixture
            .capturer
            .before_tab_switch(&source, Some(tab(1)), seconds(11));

        assert_eq!(fixture.pending(), 0);
        assert_eq!(fixture.capturer.registry.stats().failed, 1);
        assert!(fixture.stored(tab(1)).is_none());
    }

    #[test]
    fn a_worker_that_refuses_the_job_is_a_failure() {
        let mut fixture = fixture(Diagnostics::default());
        let source = FakeSource::new(1200, 800, PALE_CYAN);
        fixture.refuse.set(Some(WorkerRefused::Stopped));
        fixture
            .capturer
            .frame_presented(&source, tab(1), seconds(10));

        fixture
            .capturer
            .before_tab_switch(&source, Some(tab(1)), seconds(11));

        assert_eq!(fixture.pending(), 0);
        assert_eq!(fixture.capturer.registry.stats().failed, 1);
    }

    #[test]
    fn a_small_cap_evicts_and_the_counters_say_so() {
        // Room for two pictures of 450x300 (540,256 bytes each), not three.
        let mut fixture = fixture(Diagnostics {
            dump_dir: None,
            cap_bytes: Some(1_200_000),
        });
        let source = FakeSource::new(1200, 800, PALE_CYAN);
        for (number, time) in [(1_u64, 10_u64), (2, 20), (3, 30)] {
            fixture
                .capturer
                .frame_presented(&source, tab(number), seconds(time));
            fixture
                .capturer
                .before_tab_switch(&source, Some(tab(number)), seconds(time + 1));
            fixture.deliver(seconds(time + 1));
        }

        let stats = fixture.capturer.registry.stats();
        assert_eq!(stats.entries, 2);
        assert!(stats.used_bytes <= 1_200_000);
        assert_eq!(stats.evicted, 1);
        assert!(fixture.stored(tab(1)).is_none(), "the oldest went first");
        assert!(fixture.stored(tab(3)).is_some());
    }
}
