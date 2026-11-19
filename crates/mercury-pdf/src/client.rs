// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The browser's side of the pipe: starts the helper process, sends requests, and delivers the
//! answers as [`ClientEvent`]s.
//!
//! One [`PdfClient`] owns one helper process and one document. It has a **reader thread** (it
//! decodes the helper's replies and calls the event callback, in order, on that thread) and a
//! **watchdog thread**. Callers send requests from any thread; nothing a caller does waits for the
//! helper.
//!
//! - **Tiles.** [`PdfClient::request_tile`] queues the request in the client; at most
//!   [`MAX_TILES_IN_FLIGHT`] (2) are sent to the helper at once, the best priority first (then
//!   the oldest). A request that is cancelled before it is sent is dropped without a trace; one
//!   that was sent is cancelled in the helper's queue, which answers it with `Cancelled` (this
//!   client does not report that answer). A request equal to one already waiting or in flight is
//!   merged with it: the priority is raised and the same id is returned.
//! - **Loss.** The client never restarts a helper. When the helper ends or breaks the protocol,
//!   a hang is cut short by the watchdog, or a reply is not what it should be, the helper is
//!   killed and **exactly one** [`ClientEvent::WorkerLost`] is delivered; after it, nothing. A
//!   request that was still waiting for its answer gets none.
//! - **Time limits.** The watchdog ends a helper that makes no progress: with a request
//!   outstanding, [`ClientConfig::open_timeout`] (60 s) while the document is opening and
//!   [`ClientConfig::request_timeout`] (20 s) otherwise, counted from the last reply (or from
//!   the request, if nothing has answered since). A helper that is slow but steady is not cut
//!   off. The loss is reported as `WorkerLost(PdfError::Timeout)`.
//! - **Hygiene.** The helper runs with an empty environment (only `MERCURY_PDFIUM_LIBRARY`), in
//!   its own process group, with standard input and output piped and standard error inherited.
//!   A reply that breaks the protocol (a tile of the wrong size, a malformed frame) ends the
//!   helper: it is the less trusted side.
//! - **Drop.** Dropping the client kills the helper, reaps it and joins both threads (a client
//!   dropped from inside its own event callback does not join the thread it is on).

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use zeroize::Zeroize as _;

use crate::backend::PdfBackend;
use crate::error::PdfError;
use crate::protocol::{
    PROTOCOL_VERSION, Reply, Request, decode_list, decode_page_sizes, decode_page_text, read_reply,
    write_request,
};
use crate::serve::{ServeExit, serve};
use crate::testing::pipe;
use crate::types::{
    DocumentInfo, FindOptions, Link, OutlineItem, PageText, SearchHit, Tile, TileRequest, limits,
};

/// The most tile requests the helper is asked for at once.
pub const MAX_TILES_IN_FLIGHT: usize = 2;

/// The name of the environment variable that tells the helper where PDFium is.
pub const LIBRARY_VARIABLE: &str = "MERCURY_PDFIUM_LIBRARY";

/// How to start a helper process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerLaunch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// The PDFium library, passed to the helper in [`LIBRARY_VARIABLE`].
    pub library: Option<PathBuf>,
}

impl WorkerLaunch {
    /// This executable with `--pdf-worker`.
    pub fn current_exe(library: Option<PathBuf>) -> io::Result<Self> {
        Ok(Self {
            program: std::env::current_exe()?,
            args: vec![OsString::from("--pdf-worker")],
            library,
        })
    }
}

/// The time limits of a client; see the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientConfig {
    pub open_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            open_timeout: Duration::from_secs(60),
            request_timeout: Duration::from_secs(20),
        }
    }
}

/// The id of a tile request, chosen by the client.
pub type RequestId = u64;

/// What the helper told the browser. Delivered on the client's reader thread, in order.
#[derive(Clone, PartialEq)]
pub enum ClientEvent {
    Opened(Result<DocumentInfo, PdfError>),
    Tile {
        id: RequestId,
        request: TileRequest,
        result: Result<Tile, PdfError>,
    },
    PageText {
        page: u32,
        result: Result<PageText, PdfError>,
    },
    Links {
        page: u32,
        result: Result<Vec<Link>, PdfError>,
    },
    Outline(Result<Vec<OutlineItem>, PdfError>),
    /// The hits on one page of a find, page by page; `finished` on the last.
    Found {
        find: u64,
        page: u32,
        hits: Vec<SearchHit>,
        finished: bool,
    },
    /// The helper is gone; exactly once, then silence.
    WorkerLost(PdfError),
}

impl std::fmt::Debug for ClientEvent {
    /// A summary: tile pixels and text are not printed.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Opened(result) => formatter
                .debug_tuple("Opened")
                .field(&result.as_ref().map(|info| info.pages.len()))
                .finish(),
            Self::Tile {
                id,
                request,
                result,
            } => formatter
                .debug_struct("Tile")
                .field("id", id)
                .field("request", request)
                .field(
                    "result",
                    &result.as_ref().map(|tile| (tile.width, tile.height)),
                )
                .finish(),
            Self::PageText { page, result } => formatter
                .debug_struct("PageText")
                .field("page", page)
                .field("chars", &result.as_ref().map(|text| text.chars.len()))
                .finish(),
            Self::Links { page, result } => formatter
                .debug_struct("Links")
                .field("page", page)
                .field("links", &result.as_ref().map(Vec::len))
                .finish(),
            Self::Outline(result) => formatter
                .debug_tuple("Outline")
                .field(&result.as_ref().map(Vec::len))
                .finish(),
            Self::Found {
                find,
                page,
                hits,
                finished,
            } => formatter
                .debug_struct("Found")
                .field("find", find)
                .field("page", page)
                .field("hits", &hits.len())
                .field("finished", finished)
                .finish(),
            Self::WorkerLost(error) => formatter.debug_tuple("WorkerLost").field(error).finish(),
        }
    }
}

type Events = Box<dyn Fn(ClientEvent) + Send + Sync>;

/// A request the helper has not answered yet.
enum Pending {
    Open,
    Tile {
        request: TileRequest,
        cancelled: bool,
    },
    PageText(u32),
    Links(u32),
    Outline,
}

struct QueuedTile {
    id: RequestId,
    priority: u8,
    request: TileRequest,
}

#[derive(Default)]
struct State {
    next_id: u64,
    pending: HashMap<u64, Pending>,
    finds: HashSet<u64>,
    /// Tile requests not yet sent, in arrival order.
    queue: Vec<QueuedTile>,
    in_flight: usize,
    /// The helper's `Ready` frame has arrived.
    ready: bool,
    /// The loss was (or is being) reported.
    lost: bool,
    timed_out: bool,
    /// The helper reported a failure of its own before it ended.
    helper_error: Option<PdfError>,
    /// When the oldest wait for the helper started or when it last answered, while a request is
    /// outstanding.
    waiting_since: Option<Instant>,
}

impl State {
    fn outstanding(&self) -> bool {
        !self.ready || !self.pending.is_empty() || !self.finds.is_empty()
    }

    /// Restarts the progress timer if something is outstanding, stops it otherwise.
    fn refresh_timer(&mut self) {
        self.waiting_since = self.outstanding().then(Instant::now);
    }

    /// Starts the timer if something is outstanding and it was not running.
    fn start_timer(&mut self) {
        if self.outstanding() {
            self.waiting_since.get_or_insert_with(Instant::now);
        }
    }
}

struct Inner {
    events: Events,
    config: ClientConfig,
    state: Mutex<State>,
    /// Wakes the watchdog when the state changes or the client is dropped.
    changed: Condvar,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Option<Child>>,
    serve_thread: Mutex<Option<JoinHandle<ServeExit>>>,
    /// Ends the in-process transport (the process transport is killed instead).
    killed: Arc<AtomicBool>,
    closing: AtomicBool,
    pid: Option<u32>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Writes one request; on a failure the writer is dropped (the helper is gone or stuck).
fn send_on(writer: &mut Option<Box<dyn Write + Send>>, request: &Request) -> bool {
    let Some(stream) = writer.as_mut() else {
        return false;
    };
    match write_request(stream, request) {
        Ok(()) => true,
        Err(error) => {
            log::warn!("pdf client: could not write to the helper: {error}");
            *writer = None;
            false
        }
    }
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    fn emit(&self, event: ClientEvent) {
        if !self.closing.load(Ordering::SeqCst) {
            (self.events)(event);
        }
    }

    /// Ends the transport: the process is killed; the in-process stream is cut.
    fn kill_transport(&self) {
        self.killed.store(true, Ordering::SeqCst);
        if let Some(child) = lock(&self.child).as_mut() {
            // It may have exited already, which is fine.
            let _ = child.kill();
        }
    }

    /// Registers a request that waits for an answer and sends it.
    fn send_pending(&self, id: u64, pending: Pending, request: &Request) {
        let mut writer = lock(&self.writer);
        {
            let mut state = self.state();
            if state.lost {
                return;
            }
            state.pending.insert(id, pending);
            state.start_timer();
        }
        self.changed.notify_all();
        send_on(&mut writer, request);
    }

    /// Sends queued tile requests while the window has room.
    fn pump(&self) {
        let mut writer = lock(&self.writer);
        loop {
            let next = {
                let mut state = self.state();
                if state.lost || state.in_flight >= MAX_TILES_IN_FLIGHT || state.queue.is_empty() {
                    None
                } else {
                    // The highest priority; on a tie the one that was queued first.
                    let mut best = 0;
                    for (index, queued) in state.queue.iter().enumerate() {
                        if queued.priority > state.queue[best].priority {
                            best = index;
                        }
                    }
                    let queued = state.queue.remove(best);
                    state.in_flight += 1;
                    state.pending.insert(
                        queued.id,
                        Pending::Tile {
                            request: queued.request,
                            cancelled: false,
                        },
                    );
                    state.start_timer();
                    Some(queued)
                }
            };
            let Some(queued) = next else { break };
            self.changed.notify_all();
            let request = Request::Tile {
                id: queued.id,
                priority: queued.priority,
                request: queued.request,
            };
            if !send_on(&mut writer, &request) {
                break;
            }
        }
    }

    /// Removes the pending entry of `id` (a tile frees its place in the window) and restarts
    /// the progress timer: the helper just answered.
    fn take_pending(&self, id: u64) -> Option<Pending> {
        let mut state = self.state();
        let pending = state.pending.remove(&id);
        if matches!(pending, Some(Pending::Tile { .. })) {
            state.in_flight = state.in_flight.saturating_sub(1);
        }
        state.refresh_timer();
        drop(state);
        self.changed.notify_all();
        pending
    }

    /// Handles one reply. `Err` is a protocol violation: the helper must be ended.
    fn handle_reply(&self, reply: Reply, blob: Vec<u8>) -> Result<(), String> {
        {
            let mut state = self.state();
            if !state.ready {
                match reply {
                    Reply::Ready {
                        protocol_version, ..
                    } if protocol_version == PROTOCOL_VERSION => {
                        state.ready = true;
                        state.refresh_timer();
                        drop(state);
                        self.changed.notify_all();
                        return Ok(());
                    }
                    Reply::Ready {
                        protocol_version, ..
                    } => {
                        return Err(format!(
                            "the helper speaks protocol {protocol_version}, not {PROTOCOL_VERSION}"
                        ));
                    }
                    Reply::Error { id: None, error } => {
                        // The helper could not start (no library, for instance) and says why
                        // before it ends; that is the reason the loss reports.
                        state.helper_error = Some(error);
                        return Ok(());
                    }
                    _ => return Err("the helper did not start with a ready message".to_owned()),
                }
            }
        }
        match reply {
            Reply::Ready { .. } => Err("a second ready message".to_owned()),
            Reply::Opened {
                id,
                title,
                page_count,
                error,
            } => {
                let Some(Pending::Open) = self.take_pending(id) else {
                    return Err("an answer to an open request nobody made".to_owned());
                };
                let result = match error {
                    Some(error) => Err(error),
                    None => {
                        if page_count == 0 || page_count > limits::MAX_PAGES {
                            return Err("the helper reported an impossible page count".to_owned());
                        }
                        let pages = decode_page_sizes(&blob, page_count)
                            .map_err(|error| error.to_string())?;
                        Ok(DocumentInfo { pages, title })
                    }
                };
                self.emit(ClientEvent::Opened(result));
                Ok(())
            }
            Reply::Tile {
                id,
                width,
                height,
                error,
            } => {
                let Some(Pending::Tile { request, .. }) = self.take_pending(id) else {
                    return Err("an answer to a tile request nobody made".to_owned());
                };
                let result = match error {
                    // The answer to a cancel: the caller does not want it.
                    Some(PdfError::Cancelled) => None,
                    Some(error) => Some(Err(error)),
                    None => {
                        let tile = Tile {
                            width,
                            height,
                            rgba: blob,
                        };
                        if width != request.width
                            || height != request.height
                            || !tile.is_consistent()
                        {
                            return Err("the helper sent a tile of the wrong size".to_owned());
                        }
                        Some(Ok(tile))
                    }
                };
                if let Some(result) = result {
                    self.emit(ClientEvent::Tile {
                        id,
                        request,
                        result,
                    });
                }
                self.pump();
                Ok(())
            }
            Reply::PageText {
                id,
                page,
                chars,
                error,
            } => {
                let Some(Pending::PageText(asked)) = self.take_pending(id) else {
                    return Err("an answer to a text request nobody made".to_owned());
                };
                let result = match error {
                    Some(error) => Err(error),
                    None => {
                        if page != asked || chars as usize > limits::MAX_TEXT_CHARS {
                            return Err("the helper sent text for the wrong page".to_owned());
                        }
                        Ok(decode_page_text(&blob, chars).map_err(|error| error.to_string())?)
                    }
                };
                self.emit(ClientEvent::PageText {
                    page: asked,
                    result,
                });
                Ok(())
            }
            Reply::Links { id, page, error } => {
                let Some(Pending::Links(asked)) = self.take_pending(id) else {
                    return Err("an answer to a links request nobody made".to_owned());
                };
                let result = match error {
                    Some(error) => Err(error),
                    None => {
                        let links =
                            decode_list::<Link>(&blob).map_err(|error| error.to_string())?;
                        if page != asked || links.len() > limits::MAX_LINKS_PER_PAGE {
                            return Err("the helper sent links that do not fit".to_owned());
                        }
                        Ok(links)
                    }
                };
                self.emit(ClientEvent::Links {
                    page: asked,
                    result,
                });
                Ok(())
            }
            Reply::Outline { id, error } => {
                let Some(Pending::Outline) = self.take_pending(id) else {
                    return Err("an answer to an outline request nobody made".to_owned());
                };
                let result = match error {
                    Some(error) => Err(error),
                    None => {
                        Ok(decode_list::<OutlineItem>(&blob).map_err(|error| error.to_string())?)
                    }
                };
                self.emit(ClientEvent::Outline(result));
                Ok(())
            }
            Reply::Found {
                find,
                page,
                finished,
                error,
            } => {
                let failed = error.is_some();
                {
                    let mut state = self.state();
                    if !state.finds.contains(&find) {
                        // Cancelled by the caller; its last answers are not wanted.
                        return Ok(());
                    }
                    if finished || failed {
                        state.finds.remove(&find);
                    }
                    state.refresh_timer();
                }
                self.changed.notify_all();
                let hits = match error {
                    Some(error) => {
                        log::warn!("pdf client: a search failed: {error:?}");
                        Vec::new()
                    }
                    None => {
                        let hits =
                            decode_list::<SearchHit>(&blob).map_err(|error| error.to_string())?;
                        if hits.len() > limits::MAX_HITS_PER_PAGE {
                            return Err("the helper sent too many search hits".to_owned());
                        }
                        hits
                    }
                };
                self.emit(ClientEvent::Found {
                    find,
                    page,
                    hits,
                    // A search that failed is over for the caller.
                    finished: finished || failed,
                });
                Ok(())
            }
            Reply::Closed { .. } => Ok(()),
            Reply::Error { id: None, error } => {
                self.state().helper_error = Some(error);
                Ok(())
            }
            Reply::Error {
                id: Some(id),
                error,
            } => {
                // The helper failed this request in a way that ends it (a panic): the request
                // gets the error, then the end of the helper is reported as the loss.
                match self.take_pending(id) {
                    Some(Pending::Open) => self.emit(ClientEvent::Opened(Err(error))),
                    Some(Pending::Tile { request, .. }) => self.emit(ClientEvent::Tile {
                        id,
                        request,
                        result: Err(error),
                    }),
                    Some(Pending::PageText(page)) => self.emit(ClientEvent::PageText {
                        page,
                        result: Err(error),
                    }),
                    Some(Pending::Links(page)) => self.emit(ClientEvent::Links {
                        page,
                        result: Err(error),
                    }),
                    Some(Pending::Outline) => self.emit(ClientEvent::Outline(Err(error))),
                    None => {
                        self.state().helper_error = Some(error);
                    }
                }
                Ok(())
            }
        }
    }

    /// The reader has seen the end of the helper: reports the loss, once.
    fn finish(&self, reason: Option<PdfError>) {
        let describe = self.reap();
        if self.closing.load(Ordering::SeqCst) {
            return;
        }
        let error = {
            let mut state = self.state();
            if state.lost {
                return;
            }
            state.lost = true;
            state.queue.clear();
            state.pending.clear();
            state.finds.clear();
            state.waiting_since = None;
            if state.timed_out {
                PdfError::Timeout
            } else if let Some(reason) = reason {
                reason
            } else if let Some(error) = state.helper_error.take() {
                error
            } else {
                PdfError::WorkerLost(format!("the helper ended ({describe})"))
            }
        };
        self.changed.notify_all();
        self.emit(ClientEvent::WorkerLost(error));
    }

    /// Ends the helper (if it is still there) and says how it ended.
    fn reap(&self) -> String {
        self.kill_transport();
        if let Some(child) = lock(&self.child).as_mut() {
            return match child.wait() {
                Ok(status) => status.to_string(),
                Err(error) => format!("wait failed: {error}"),
            };
        }
        let Some(handle) = lock(&self.serve_thread).take() else {
            return "no helper".to_owned();
        };
        // The stream ended, so the thread is returning; a backend call that never returns is
        // left to finish alone.
        let deadline = Instant::now() + Duration::from_millis(200);
        while !handle.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        if handle.is_finished() {
            match handle.join() {
                Ok(exit) => format!("{exit:?}"),
                Err(_) => "the serve thread panicked".to_owned(),
            }
        } else {
            "the in-process helper was cut off".to_owned()
        }
    }
}

fn read_loop(inner: &Arc<Inner>, mut reader: Box<dyn Read + Send>) {
    let mut reason = None;
    loop {
        match read_reply(&mut reader) {
            Ok(Some((reply, blob))) => {
                if let Err(detail) = inner.handle_reply(reply, blob) {
                    log::warn!("pdf client: protocol violation by the helper: {detail}");
                    reason = Some(PdfError::WorkerLost(detail));
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                if !inner.closing.load(Ordering::SeqCst) {
                    log::warn!("pdf client: bad frame from the helper: {error}");
                }
                reason = Some(PdfError::WorkerLost(format!("a bad frame: {error}")));
                break;
            }
        }
    }
    inner.finish(reason);
}

fn watchdog_loop(inner: &Arc<Inner>) {
    let mut state = inner.state();
    loop {
        if inner.closing.load(Ordering::SeqCst) || state.lost {
            return;
        }
        let Some(since) = state.waiting_since else {
            state = inner
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
            continue;
        };
        let limit = if !state.ready || state.pending.values().any(|p| matches!(p, Pending::Open)) {
            inner.config.open_timeout
        } else {
            inner.config.request_timeout
        };
        let now = Instant::now();
        let deadline = since + limit;
        if now >= deadline {
            state.timed_out = true;
            // Forget the clock so this does not fire again while the reader finishes.
            state.waiting_since = None;
            drop(state);
            log::warn!("pdf client: the helper made no progress for {limit:?}; ending it");
            inner.kill_transport();
            state = inner.state();
            continue;
        }
        state = inner
            .changed
            .wait_timeout(state, deadline - now)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

/// One helper process (or in-process loop) and the document it holds. See the module
/// documentation.
pub struct PdfClient {
    inner: Arc<Inner>,
    reader: Option<JoinHandle<()>>,
    watchdog: Option<JoinHandle<()>>,
}

impl PdfClient {
    /// Starts the helper process. The first event is `Opened`, after [`PdfClient::open`].
    pub fn start(launch: WorkerLaunch, events: Events) -> Result<Self, PdfError> {
        Self::start_with_config(launch, events, ClientConfig::default())
    }

    /// [`PdfClient::start`] with other time limits (tests).
    pub fn start_with_config(
        launch: WorkerLaunch,
        events: Events,
        config: ClientConfig,
    ) -> Result<Self, PdfError> {
        let mut command = Command::new(&launch.program);
        command
            .args(&launch.args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            // Its own process group: a Ctrl-C meant for the browser does not reach it; the
            // browser ends it (and end of input ends it too, if the browser is killed).
            .process_group(0);
        if let Some(library) = &launch.library {
            command.env(LIBRARY_VARIABLE, library);
        }
        let mut child = command
            .spawn()
            .map_err(|error| PdfError::Io(format!("could not start the PDF helper: {error}")))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PdfError::Io("the PDF helper has no pipes".to_owned()));
        };
        let pid = Some(child.id());
        Ok(Self::assemble(
            config,
            events,
            Box::new(stdin),
            Box::new(stdout),
            Some(child),
            None,
            Arc::new(AtomicBool::new(false)),
            pid,
        ))
    }

    /// For tests: the same request loop and the same client, with a backend in a thread and no
    /// process. A backend call that never returns cannot be interrupted; its thread is left to
    /// finish by itself when the client is dropped.
    pub fn start_in_process(
        backend: Box<dyn PdfBackend + Send>,
        events: Box<dyn Fn(ClientEvent) + Send + Sync>,
    ) -> Self {
        Self::start_in_process_with_config(backend, events, ClientConfig::default())
    }

    /// [`PdfClient::start_in_process`] with other time limits.
    pub fn start_in_process_with_config(
        backend: Box<dyn PdfBackend + Send>,
        events: Events,
        config: ClientConfig,
    ) -> Self {
        let killed = Arc::new(AtomicBool::new(false));
        let (request_writer, request_reader) = pipe();
        let (reply_writer, reply_reader) = pipe();
        let serve_thread = thread::Builder::new()
            .name("pdf-in-process-worker".to_owned())
            .spawn(move || serve(backend, request_reader, reply_writer))
            .expect("the in-process worker thread starts");
        Self::assemble(
            config,
            events,
            Box::new(request_writer),
            Box::new(reply_reader.with_cancel(Arc::clone(&killed))),
            None,
            Some(serve_thread),
            killed,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn assemble(
        config: ClientConfig,
        events: Events,
        writer: Box<dyn Write + Send>,
        reader: Box<dyn Read + Send>,
        child: Option<Child>,
        serve_thread: Option<JoinHandle<ServeExit>>,
        killed: Arc<AtomicBool>,
        pid: Option<u32>,
    ) -> Self {
        let mut state = State::default();
        // Until the helper says it is ready, it is being waited for.
        state.start_timer();
        let inner = Arc::new(Inner {
            events,
            config,
            state: Mutex::new(state),
            changed: Condvar::new(),
            writer: Mutex::new(Some(writer)),
            child: Mutex::new(child),
            serve_thread: Mutex::new(serve_thread),
            killed,
            closing: AtomicBool::new(false),
            pid,
        });
        let reader_inner = Arc::clone(&inner);
        let reader_thread = thread::Builder::new()
            .name("pdf-client-reader".to_owned())
            .spawn(move || read_loop(&reader_inner, reader))
            .expect("the PDF client's reader thread starts");
        let watchdog_inner = Arc::clone(&inner);
        let watchdog_thread = thread::Builder::new()
            .name("pdf-client-watchdog".to_owned())
            .spawn(move || watchdog_loop(&watchdog_inner))
            .expect("the PDF client's watchdog thread starts");
        Self {
            inner,
            reader: Some(reader_thread),
            watchdog: Some(watchdog_thread),
        }
    }

    /// The helper's process id, for tests and logs (`None` in process).
    pub fn process_id(&self) -> Option<u32> {
        self.inner.pid
    }

    /// Whether the loss of the helper has been reported.
    pub fn has_ended(&self) -> bool {
        self.inner.state().lost
    }

    fn next_id(&self) -> u64 {
        let mut state = self.inner.state();
        state.next_id += 1;
        state.next_id
    }

    /// Opens the document at `path` (read from disk by the helper, not copied). The answer is
    /// [`ClientEvent::Opened`]; a password is sent in this message only and is wiped after.
    pub fn open(&self, path: &Path, password: Option<&str>) {
        let Some(text) = path.to_str() else {
            // Only this one case answers on the calling thread: there is no helper involved.
            self.inner.emit(ClientEvent::Opened(Err(PdfError::Io(
                "the file name is not valid text".to_owned(),
            ))));
            return;
        };
        let id = self.next_id();
        let mut request = Request::Open {
            id,
            path: text.to_owned(),
            password: password.map(str::to_owned),
        };
        self.inner.send_pending(id, Pending::Open, &request);
        // The password has been sent; the copy in the request is wiped before it is freed.
        if let Request::Open {
            password: Some(word),
            ..
        } = &mut request
        {
            word.zeroize();
        }
    }

    /// Asks for one tile; higher `priority` is sent sooner. Returns the id its answer carries.
    pub fn request_tile(&self, request: TileRequest, priority: u8) -> RequestId {
        let id = {
            let mut state = self.inner.state();
            let same_pending = state
                .pending
                .iter()
                .find_map(|(id, pending)| match pending {
                    Pending::Tile {
                        request: sent,
                        cancelled: false,
                    } if *sent == request => Some(*id),
                    _ => None,
                });
            if let Some(existing) = same_pending {
                return existing;
            }
            if let Some(queued) = state
                .queue
                .iter_mut()
                .find(|queued| queued.request == request)
            {
                queued.priority = queued.priority.max(priority);
                queued.id
            } else {
                state.next_id += 1;
                let id = state.next_id;
                state.queue.push(QueuedTile {
                    id,
                    priority,
                    request,
                });
                id
            }
        };
        self.inner.pump();
        id
    }

    /// Cancels a tile request: dropped silently if it was not sent yet, cancelled in the
    /// helper's queue if it was.
    pub fn cancel_tile(&self, id: RequestId) {
        let mut writer = lock(&self.inner.writer);
        let send_cancel = {
            let mut state = self.inner.state();
            if let Some(position) = state.queue.iter().position(|queued| queued.id == id) {
                state.queue.remove(position);
                false
            } else if let Some(Pending::Tile { cancelled, .. }) = state.pending.get_mut(&id) {
                !std::mem::replace(cancelled, true)
            } else {
                false
            }
        };
        if send_cancel {
            send_on(&mut writer, &Request::Cancel { id });
        }
    }

    /// Asks for the text of a page (answered with [`ClientEvent::PageText`]).
    pub fn request_page_text(&self, page: u32) {
        let id = self.next_id();
        self.inner
            .send_pending(id, Pending::PageText(page), &Request::PageText { id, page });
    }

    /// Asks for the links of a page (answered with [`ClientEvent::Links`]).
    pub fn request_links(&self, page: u32) {
        let id = self.next_id();
        self.inner
            .send_pending(id, Pending::Links(page), &Request::Links { id, page });
    }

    /// Asks for the outline (answered with [`ClientEvent::Outline`]).
    pub fn request_outline(&self) {
        let id = self.next_id();
        self.inner
            .send_pending(id, Pending::Outline, &Request::Outline { id });
    }

    /// Starts a find from `from_page`, page by page and wrapping round; returns its id. The
    /// answers are [`ClientEvent::Found`], the last with `finished`.
    pub fn find(&self, query: &str, options: FindOptions, from_page: u32) -> u64 {
        let id = self.next_id();
        let request = Request::Find {
            id,
            query: query.to_owned(),
            options,
            from_page,
        };
        let mut writer = lock(&self.inner.writer);
        {
            let mut state = self.inner.state();
            if state.lost {
                return id;
            }
            state.finds.insert(id);
            state.start_timer();
        }
        self.inner.changed.notify_all();
        send_on(&mut writer, &request);
        id
    }

    /// Stops a find; answers still on the way are dropped.
    pub fn cancel_find(&self, find: u64) {
        let mut writer = lock(&self.inner.writer);
        let known = {
            let mut state = self.inner.state();
            let known = state.finds.remove(&find);
            state.refresh_timer();
            known
        };
        self.inner.changed.notify_all();
        if known {
            send_on(&mut writer, &Request::CancelFind { id: find });
        }
    }
}

impl Drop for PdfClient {
    fn drop(&mut self) {
        let inner = &self.inner;
        inner.closing.store(true, Ordering::SeqCst);
        // Wake the watchdog under the lock so it cannot miss the flag.
        drop(inner.state());
        inner.changed.notify_all();
        // End the input first: an in-process loop then sees the end of its requests.
        *lock(&inner.writer) = None;
        inner.kill_transport();
        if let Some(child) = lock(&inner.child).as_mut() {
            // Reaps it: no zombie, no process left behind.
            let _ = child.wait();
        }
        let current = thread::current().id();
        for handle in [self.reader.take(), self.watchdog.take()]
            .into_iter()
            .flatten()
        {
            if handle.thread().id() != current {
                let _ = handle.join();
            }
        }
        // The in-process loop is joined only if it has already finished; a backend call that
        // never returns is left to end by itself.
        if let Some(handle) = lock(&inner.serve_thread).take()
            && handle.is_finished()
        {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver};

    use super::*;
    use crate::fake::{FakeBackend, FakeConfig, expected_tile, fake_page_text};
    use crate::types::Rotation;

    const WAIT: Duration = Duration::from_secs(10);

    fn collect() -> (Events, Receiver<ClientEvent>) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        (
            Box::new(move |event| {
                let _ = lock(&sender).send(event);
            }),
            receiver,
        )
    }

    fn in_process(config: FakeConfig) -> (PdfClient, Receiver<ClientEvent>) {
        let (events, receiver) = collect();
        (
            PdfClient::start_in_process(Box::new(FakeBackend::new(config)), events),
            receiver,
        )
    }

    fn tile(page: u32, x: u32) -> TileRequest {
        TileRequest {
            page,
            scale: 1.0,
            rotation: Rotation::Deg0,
            x,
            y: 0,
            width: 8,
            height: 8,
        }
    }

    fn next(receiver: &Receiver<ClientEvent>) -> ClientEvent {
        receiver.recv_timeout(WAIT).expect("an event in time")
    }

    #[test]
    fn open_then_a_tile_arrive_in_order() {
        let (client, events) = in_process(FakeConfig::default());
        client.open(Path::new("x.pdf"), None);
        let ClientEvent::Opened(Ok(info)) = next(&events) else {
            panic!("expected Opened");
        };
        assert_eq!(info.page_count(), 3);
        assert_eq!(info.title.as_deref(), Some("Fake document"));
        let request = tile(1, 16);
        let id = client.request_tile(request, 5);
        let ClientEvent::Tile {
            id: got,
            request: got_request,
            result,
        } = next(&events)
        else {
            panic!("expected a tile");
        };
        assert_eq!(got, id);
        assert_eq!(got_request, request);
        assert_eq!(result.unwrap(), expected_tile(&request));
    }

    #[test]
    fn text_links_outline_and_find_come_back() {
        let (client, events) = in_process(FakeConfig::default());
        client.open(Path::new("x.pdf"), None);
        assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
        client.request_page_text(2);
        client.request_links(0);
        client.request_outline();
        let find = client.find("fox", FindOptions::default(), 1);
        let mut seen_text = false;
        let mut seen_links = false;
        let mut seen_outline = false;
        let mut pages = Vec::new();
        while pages.len() < 3 {
            match next(&events) {
                ClientEvent::PageText { page, result } => {
                    assert_eq!(page, 2);
                    assert_eq!(result.unwrap(), fake_page_text(2));
                    seen_text = true;
                }
                ClientEvent::Links { page, result } => {
                    assert_eq!(page, 0);
                    assert_eq!(result.unwrap().len(), 1);
                    seen_links = true;
                }
                ClientEvent::Outline(result) => {
                    assert_eq!(result.unwrap().len(), 2);
                    seen_outline = true;
                }
                ClientEvent::Found {
                    find: id,
                    page,
                    hits,
                    finished,
                } => {
                    assert_eq!(id, find);
                    assert_eq!(hits.len(), 1, "one fox per page");
                    pages.push(page);
                    assert_eq!(finished, pages.len() == 3);
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(pages, [1, 2, 0], "from page 1, wrapping round");
        assert!(seen_text && seen_links && seen_outline);
    }

    #[test]
    fn a_password_is_asked_for_and_then_accepted() {
        let (client, events) = in_process(FakeConfig {
            password: Some("sesame".to_owned()),
            ..FakeConfig::default()
        });
        client.open(Path::new("x.pdf"), None);
        assert!(matches!(
            next(&events),
            ClientEvent::Opened(Err(PdfError::PasswordRequired))
        ));
        client.open(Path::new("x.pdf"), Some("wrong"));
        assert!(matches!(
            next(&events),
            ClientEvent::Opened(Err(PdfError::PasswordIncorrect))
        ));
        client.open(Path::new("x.pdf"), Some("sesame"));
        assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
    }

    #[test]
    fn at_most_two_tiles_are_in_flight_and_the_best_priority_goes_first() {
        let (client, events) = in_process(FakeConfig {
            render_delay: Duration::from_millis(150),
            ..FakeConfig::default()
        });
        client.open(Path::new("x.pdf"), None);
        assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
        // Two go out at once (a, b); the rest wait in the client.
        let a = client.request_tile(tile(0, 0), 1);
        let b = client.request_tile(tile(0, 8), 1);
        let low = client.request_tile(tile(0, 16), 2);
        let high = client.request_tile(tile(0, 24), 200);
        let mid = client.request_tile(tile(0, 32), 100);
        {
            let state = client.inner.state();
            assert_eq!(state.in_flight, 2);
            assert_eq!(state.queue.len(), 3);
        }
        let mut order = Vec::new();
        for _ in 0..5 {
            let ClientEvent::Tile { id, .. } = next(&events) else {
                panic!("expected a tile");
            };
            order.push(id);
        }
        assert_eq!(order, [a, b, high, mid, low]);
    }

    #[test]
    fn a_tile_cancelled_before_it_is_sent_is_dropped_without_an_event() {
        let (client, events) = in_process(FakeConfig {
            render_delay: Duration::from_millis(100),
            ..FakeConfig::default()
        });
        client.open(Path::new("x.pdf"), None);
        assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
        client.request_tile(tile(0, 0), 1);
        client.request_tile(tile(0, 8), 1);
        let waiting = client.request_tile(tile(0, 16), 1);
        client.cancel_tile(waiting);
        let mut ids = Vec::new();
        for _ in 0..2 {
            let ClientEvent::Tile { id, .. } = next(&events) else {
                panic!("expected a tile");
            };
            ids.push(id);
        }
        assert!(!ids.contains(&waiting));
        assert!(events.recv_timeout(Duration::from_millis(400)).is_err());
    }

    #[test]
    fn a_tile_cancelled_after_it_was_sent_is_cancelled_in_the_helper_and_not_reported() {
        let (client, events) = in_process(FakeConfig {
            render_delay: Duration::from_millis(200),
            ..FakeConfig::default()
        });
        client.open(Path::new("x.pdf"), None);
        assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
        let first = client.request_tile(tile(0, 0), 1);
        // The helper is busy with `first`; `second` waits in its queue and is cancelled there.
        let second = client.request_tile(tile(0, 8), 1);
        std::thread::sleep(Duration::from_millis(50));
        client.cancel_tile(second);
        let ClientEvent::Tile { id, .. } = next(&events) else {
            panic!("expected a tile");
        };
        assert_eq!(id, first);
        assert!(events.recv_timeout(Duration::from_millis(500)).is_err());
        // The window is free again.
        assert_eq!(client.inner.state().in_flight, 0);
    }

    #[test]
    fn the_same_tile_asked_twice_is_one_request() {
        let (client, events) = in_process(FakeConfig {
            render_delay: Duration::from_millis(100),
            ..FakeConfig::default()
        });
        client.open(Path::new("x.pdf"), None);
        assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
        let first = client.request_tile(tile(0, 0), 1);
        assert_eq!(client.request_tile(tile(0, 0), 9), first);
        client.request_tile(tile(0, 8), 1);
        let queued = client.request_tile(tile(0, 16), 1);
        assert_eq!(client.request_tile(tile(0, 16), 50), queued);
        assert_eq!(client.inner.state().queue.len(), 1);
        assert_eq!(client.inner.state().queue[0].priority, 50);
    }

    #[test]
    fn a_cancelled_find_stops_reporting() {
        let (client, events) = in_process(FakeConfig {
            find_delay: Duration::from_millis(100),
            ..FakeConfig::with_pages(5)
        });
        client.open(Path::new("x.pdf"), None);
        assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
        let find = client.find("fox", FindOptions::default(), 0);
        assert!(matches!(
            next(&events),
            ClientEvent::Found {
                finished: false,
                ..
            }
        ));
        client.cancel_find(find);
        // At most the page that was already being searched still arrives.
        let late = events.recv_timeout(Duration::from_millis(600));
        assert!(
            late.is_err(),
            "a cancelled find's answers are dropped: {late:?}"
        );
    }

    #[test]
    fn a_panicking_backend_gives_the_request_an_error_and_then_one_loss() {
        let (client, events) = in_process(FakeConfig {
            panic_after: Some(1),
            ..FakeConfig::default()
        });
        client.open(Path::new("x.pdf"), None);
        assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
        client.request_tile(tile(0, 0), 1);
        assert!(matches!(
            next(&events),
            ClientEvent::Tile { result: Ok(_), .. }
        ));
        client.request_tile(tile(0, 8), 1);
        assert!(matches!(
            next(&events),
            ClientEvent::Tile {
                result: Err(PdfError::WorkerLost(_)),
                ..
            }
        ));
        assert!(matches!(next(&events), ClientEvent::WorkerLost(_)));
        assert!(events.recv_timeout(Duration::from_millis(300)).is_err());
        assert!(client.has_ended());
    }

    #[test]
    fn nothing_is_delivered_after_the_client_is_dropped() {
        let (client, events) = in_process(FakeConfig::default());
        client.open(Path::new("x.pdf"), None);
        assert!(matches!(next(&events), ClientEvent::Opened(Ok(_))));
        drop(client);
        assert!(matches!(
            events.recv_timeout(Duration::from_millis(300)),
            Err(mpsc::RecvTimeoutError::Disconnected | mpsc::RecvTimeoutError::Timeout)
        ));
    }

    #[test]
    fn a_hung_in_process_backend_times_out_with_one_loss() {
        let (events, receiver) = collect();
        let client = PdfClient::start_in_process_with_config(
            Box::new(FakeBackend::new(FakeConfig {
                render_delay: Duration::from_secs(30),
                ..FakeConfig::default()
            })),
            events,
            ClientConfig {
                open_timeout: Duration::from_secs(5),
                request_timeout: Duration::from_millis(300),
            },
        );
        client.open(Path::new("x.pdf"), None);
        assert!(matches!(next(&receiver), ClientEvent::Opened(Ok(_))));
        client.request_tile(tile(0, 0), 1);
        assert!(matches!(
            next(&receiver),
            ClientEvent::WorkerLost(PdfError::Timeout)
        ));
        assert!(receiver.recv_timeout(Duration::from_millis(300)).is_err());
    }
}
