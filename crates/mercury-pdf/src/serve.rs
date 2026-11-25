// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The helper process's request loop: reads requests, runs them on the one thread that owns the
//! backend, and writes answers.
//!
//! [`serve`] is what `mercury-browser --pdf-worker` runs on its standard input and output (and
//! what the client runs in a thread for in-process tests). It uses two threads:
//!
//! - a **reader** decodes frames ([`crate::protocol`]) into a queue and applies `Cancel` and
//!   `CancelFind` to that queue **at once**, even while the backend is busy, so a request that
//!   has not started never runs after it was cancelled (a cancelled tile is answered with
//!   [`PdfError::Cancelled`] so the client's bookkeeping balances);
//! - the **calling thread** owns the backend (PDFium is not thread-safe) and runs one request
//!   at a time. It is the only writer of the output.
//!
//! **What runs next.** Requests run in the order they were sent, except inside a stretch with no
//! `Open` or `Close` in it (those two are barriers: everything asked before them is answered
//! first, and nothing asked after them starts before them). Inside a stretch: page text, links
//! and outline first (a person is waiting for a selection), then tile requests by priority
//! (highest first, then oldest), and finally one page of a find. A find is searched **one page at
//! a time and goes back to the queue between pages**, so a tile never waits for a whole-document
//! search. A find belongs to the open document: a barrier ends it silently.
//!
//! **Validation.** Everything a request says is checked before the backend sees it: one open
//! document at a time (a second `Open` is refused), the page index, [`TileRequest::validate`],
//! the query length. Everything a backend returns is clamped to [`limits`] and cleaned (control
//! characters, non-finite numbers, link addresses that are too long or hold control characters)
//! before it is sent, because the backend runs a C++ library on a file an attacker may have made.
//!
//! **Failure.** A malformed frame gets one [`Reply::Error`] and the loop ends at once
//! ([`ServeExit::Protocol`], status 0: an orderly end). The end of the input is an orderly end
//! too, after everything already asked has been answered ([`ServeExit::Eof`]). A panic in the
//! backend is caught, the request is answered with an error, and the loop ends with
//! [`ServeExit::Panicked`], which the process turns into exit status [`EXIT_PANIC`] (70).
//! Nothing but frames is ever written to the output.
//!
//! The reader thread is not joined when the loop ends early (it may be blocked in a read that
//! never returns); the helper process exits right after, and in tests the input is closed.

use std::any::Any;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use zeroize::Zeroizing;

use crate::backend::PdfBackend;
use crate::clean::{
    DETAIL_BYTES, accept_document, clean_error, clean_hits, clean_links, clean_outline, clip,
    finite_rect,
};
use crate::error::PdfError;
use crate::protocol::{
    self, MAX_BLOB_BYTES, PROTOCOL_VERSION, Reply, Request, encode_list, encode_page_sizes,
    encode_page_text, read_request, write_reply,
};
use crate::types::{
    DocumentInfo, FindOptions, Link, OutlineItem, PageSize, Rect, SearchHit, TileRequest, limits,
};

/// The exit status of a helper whose backend panicked (BSD `EX_SOFTWARE`).
pub const EXIT_PANIC: i32 = 70;

/// The most requests waiting at once; more is a broken or hostile client.
const MAX_QUEUED: usize = 4096;

/// How [`serve`] ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServeExit {
    /// The input ended and everything asked before was answered.
    Eof,
    /// Nobody reads the output any more.
    OutputClosed,
    /// A frame was malformed; one error reply was sent.
    Protocol(String),
    /// The backend panicked; the request was answered with an error.
    Panicked(String),
}

impl ServeExit {
    /// The process exit status: 0 for every orderly end, [`EXIT_PANIC`] after a panic.
    pub const fn status(&self) -> i32 {
        match self {
            Self::Panicked(_) => EXIT_PANIC,
            Self::Eof | Self::OutputClosed | Self::Protocol(_) => 0,
        }
    }
}

/// A boxed backend is a backend, so `serve` and the client can take `Box<dyn PdfBackend + Send>`.
impl<B: PdfBackend + ?Sized> PdfBackend for Box<B> {
    fn open(&mut self, path: &Path, password: Option<&str>) -> Result<DocumentInfo, PdfError> {
        (**self).open(path, password)
    }
    fn render(&mut self, request: &TileRequest) -> Result<crate::types::Tile, PdfError> {
        (**self).render(request)
    }
    fn page_text(&mut self, page: u32) -> Result<crate::types::PageText, PdfError> {
        (**self).page_text(page)
    }
    fn find_in_page(
        &mut self,
        page: u32,
        query: &str,
        options: FindOptions,
    ) -> Result<Vec<SearchHit>, PdfError> {
        (**self).find_in_page(page, query, options)
    }
    fn links(&mut self, page: u32) -> Result<Vec<Link>, PdfError> {
        (**self).links(page)
    }
    fn outline(&mut self) -> Result<Vec<OutlineItem>, PdfError> {
        (**self).outline()
    }
    fn close(&mut self) {
        (**self).close();
    }
}

/// [`serve_with_engine`] with the engine name `"mercury-pdf"`.
pub fn serve<B, R, W>(backend: B, input: R, output: W) -> ServeExit
where
    B: PdfBackend,
    R: Read + Send + 'static,
    W: Write,
{
    serve_with_engine("mercury-pdf", backend, input, output)
}

/// Runs the request loop until the input ends, a frame is malformed, the output closes or the
/// backend panics. `engine` is announced in the first frame.
pub fn serve_with_engine<B, R, W>(engine: &str, backend: B, input: R, output: W) -> ServeExit
where
    B: PdfBackend,
    R: Read + Send + 'static,
    W: Write,
{
    let shared = Arc::new(Shared::default());
    let reader_shared = Arc::clone(&shared);
    let spawned = thread::Builder::new()
        .name("pdf-serve-reader".to_owned())
        .spawn(move || read_loop(&reader_shared, input));
    if let Err(error) = spawned {
        return ServeExit::Protocol(format!("could not start the reader thread: {error}"));
    }
    let mut session = Session {
        backend,
        output,
        shared: &shared,
        pages: Vec::new(),
        open: false,
    };
    session.run(engine)
}

// ---------------------------------------------------------------------------------------------
// The queue shared by the reader thread and the backend thread.
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct FindJob {
    id: u64,
    query: String,
    options: FindOptions,
    from_page: u32,
    /// The next page to search and how many pages are left; `None` before the first step.
    next: Option<(u32, u32)>,
}

enum Job {
    Open {
        id: u64,
        path: String,
        password: Option<Zeroizing<String>>,
    },
    Close {
        id: u64,
    },
    Tile {
        id: u64,
        priority: u8,
        request: TileRequest,
    },
    PageText {
        id: u64,
        page: u32,
    },
    Links {
        id: u64,
        page: u32,
    },
    Outline {
        id: u64,
    },
    Find(FindJob),
}

impl Job {
    fn is_barrier(&self) -> bool {
        matches!(self, Self::Open { .. } | Self::Close { .. })
    }
}

struct Entry {
    seq: u64,
    job: Job,
}

enum End {
    Eof,
    Protocol(String),
}

#[derive(Default)]
struct Queue {
    entries: VecDeque<Entry>,
    /// Answers the reader made itself (a cancelled tile), written by the backend thread.
    outbox: VecDeque<Reply>,
    next_seq: u64,
    end: Option<End>,
}

enum Work {
    Reply(Reply),
    Job(Job),
    FindStep(FindJob),
    Finish(ServeExit),
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn push(&self, request: Request) {
        let mut queue = self.lock();
        if queue.end.is_some() {
            return;
        }
        match request {
            Request::Cancel { id } => {
                let position = queue.entries.iter().position(
                    |entry| matches!(entry.job, Job::Tile { id: tile, .. } if tile == id),
                );
                if let Some(position) = position {
                    queue.entries.remove(position);
                    queue.outbox.push_back(Reply::Tile {
                        id,
                        width: 0,
                        height: 0,
                        error: Some(PdfError::Cancelled),
                    });
                }
            }
            Request::CancelFind { id } => {
                queue
                    .entries
                    .retain(|entry| !matches!(&entry.job, Job::Find(find) if find.id == id));
            }
            other => {
                if queue.entries.len() >= MAX_QUEUED {
                    queue.end = Some(End::Protocol("too many requests are waiting".to_owned()));
                } else {
                    let job = match other {
                        Request::Open { id, path, password } => Job::Open {
                            id,
                            path,
                            password: password.map(Zeroizing::new),
                        },
                        Request::Close { id } => Job::Close { id },
                        Request::Tile {
                            id,
                            priority,
                            request,
                        } => Job::Tile {
                            id,
                            priority,
                            request,
                        },
                        Request::PageText { id, page } => Job::PageText { id, page },
                        Request::Links { id, page } => Job::Links { id, page },
                        Request::Outline { id } => Job::Outline { id },
                        Request::Find {
                            id,
                            query,
                            options,
                            from_page,
                        } => Job::Find(FindJob {
                            id,
                            query,
                            options,
                            from_page,
                            next: None,
                        }),
                        Request::Cancel { .. } | Request::CancelFind { .. } => {
                            unreachable!("handled above")
                        }
                    };
                    if job.is_barrier() {
                        // A find belongs to the document that is going away.
                        queue
                            .entries
                            .retain(|entry| !matches!(entry.job, Job::Find(_)));
                    }
                    let seq = queue.next_seq;
                    queue.next_seq += 1;
                    queue.entries.push_back(Entry { seq, job });
                }
            }
        }
        drop(queue);
        self.wake.notify_all();
    }

    fn finish(&self, end: End) {
        let mut queue = self.lock();
        if queue.end.is_none() {
            queue.end = Some(end);
        }
        drop(queue);
        self.wake.notify_all();
    }

    /// Blocks until there is something for the backend thread to do or the loop must end.
    fn next(&self) -> Work {
        let mut queue = self.lock();
        loop {
            if let Some(reply) = queue.outbox.pop_front() {
                return Work::Reply(reply);
            }
            if let Some(End::Protocol(detail)) = &queue.end {
                return Work::Finish(ServeExit::Protocol(detail.clone()));
            }
            if let Some(work) = queue.pick() {
                return work;
            }
            if queue.end.is_some() {
                return Work::Finish(ServeExit::Eof);
            }
            queue = self
                .wake
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Records the progress of a find after a step. Returns whether the find is still wanted (it
    /// may have been cancelled while the step ran); `next` of `None` ends it.
    fn advance_find(&self, id: u64, next: Option<(u32, u32)>) -> bool {
        let mut queue = self.lock();
        let position = queue
            .entries
            .iter()
            .position(|entry| matches!(&entry.job, Job::Find(find) if find.id == id));
        let Some(position) = position else {
            return false;
        };
        match next {
            None => {
                queue.entries.remove(position);
            }
            Some(next) => {
                if let Job::Find(find) = &mut queue.entries[position].job {
                    find.next = Some(next);
                }
            }
        }
        true
    }
}

impl Queue {
    /// The next piece of work, by the rules in the module documentation.
    fn pick(&mut self) -> Option<Work> {
        let barrier = self.entries.iter().position(|entry| entry.job.is_barrier());
        let stretch = barrier.unwrap_or(self.entries.len());

        let interactive = self.entries.iter().take(stretch).position(|entry| {
            matches!(
                entry.job,
                Job::PageText { .. } | Job::Links { .. } | Job::Outline { .. }
            )
        });
        if let Some(index) = interactive {
            return self.entries.remove(index).map(|entry| Work::Job(entry.job));
        }

        let best_tile = self
            .entries
            .iter()
            .take(stretch)
            .enumerate()
            .filter_map(|(index, entry)| match entry.job {
                Job::Tile { priority, .. } => Some((index, priority, entry.seq)),
                _ => None,
            })
            // The highest priority; on a tie the oldest (smallest sequence number).
            .max_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)));
        if let Some((index, _, _)) = best_tile {
            return self.entries.remove(index).map(|entry| Work::Job(entry.job));
        }

        let find = self
            .entries
            .iter()
            .take(stretch)
            .find_map(|entry| match &entry.job {
                Job::Find(find) => Some(find.clone()),
                _ => None,
            });
        if let Some(find) = find {
            return Some(Work::FindStep(find));
        }

        barrier.and_then(|index| self.entries.remove(index).map(|entry| Work::Job(entry.job)))
    }
}

fn read_loop<R: Read>(shared: &Shared, mut input: R) {
    loop {
        match read_request(&mut input) {
            Ok(Some(request)) => shared.push(request),
            Ok(None) => {
                shared.finish(End::Eof);
                return;
            }
            Err(error) => {
                shared.finish(End::Protocol(error.to_string()));
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The backend thread.
// ---------------------------------------------------------------------------------------------

struct Session<'a, B, W> {
    backend: B,
    output: W,
    shared: &'a Shared,
    pages: Vec<PageSize>,
    open: bool,
}

/// What a job's handler tells the loop: carry on, or end with this.
type Flow = Result<(), ServeExit>;

impl<B: PdfBackend, W: Write> Session<'_, B, W> {
    fn run(&mut self, engine: &str) -> ServeExit {
        let ready = Reply::Ready {
            protocol_version: PROTOCOL_VERSION,
            engine: clip(engine, 64),
        };
        if let Err(exit) = self.send(&ready, &[]) {
            return exit;
        }
        loop {
            let flow = match self.shared.next() {
                Work::Finish(exit) => return self.finish(exit),
                Work::Reply(reply) => self.send(&reply, &[]),
                Work::Job(job) => self.run_job(job),
                Work::FindStep(find) => self.find_step(find),
            };
            if let Err(exit) = flow {
                return exit;
            }
        }
    }

    /// Ends the loop; a malformed frame is answered first.
    fn finish(&mut self, exit: ServeExit) -> ServeExit {
        if let ServeExit::Protocol(detail) = &exit {
            log::warn!("pdf worker: malformed request stream: {detail}");
            let reply = Reply::Error {
                id: None,
                error: PdfError::InvalidRequest(clip(
                    &format!("malformed request: {detail}"),
                    DETAIL_BYTES,
                )),
            };
            // The client may already be gone; there is nothing more to do about it.
            let _ = self.send(&reply, &[]);
        }
        exit
    }

    fn send(&mut self, reply: &Reply, blob: &[u8]) -> Flow {
        match write_reply(&mut self.output, reply, blob) {
            Ok(()) => Ok(()),
            Err(protocol::FrameError::Io(_)) => Err(ServeExit::OutputClosed),
            // The handlers keep every reply within the limits; getting here is a bug, and the
            // honest thing is to stop rather than to send a half-formed answer.
            Err(error) => Err(ServeExit::Protocol(format!("internal: {error}"))),
        }
    }

    /// Runs one backend call, catching a panic: the failure reply is sent and the loop ends.
    fn call<T>(
        &mut self,
        fail: &dyn Fn(PdfError) -> Reply,
        action: impl FnOnce(&mut B) -> Result<T, PdfError>,
    ) -> Result<Result<T, PdfError>, ServeExit> {
        let backend = &mut self.backend;
        match catch_unwind(AssertUnwindSafe(|| action(backend))) {
            Ok(result) => Ok(result.map_err(clean_error)),
            Err(payload) => {
                let message = panic_message(payload.as_ref());
                log::error!("pdf worker: the backend panicked: {message}");
                let reply = fail(PdfError::WorkerLost(clip(
                    &format!("the PDF engine panicked: {message}"),
                    DETAIL_BYTES,
                )));
                // The client may already be gone.
                let _ = self.send(&reply, &[]);
                Err(ServeExit::Panicked(message))
            }
        }
    }

    fn run_job(&mut self, job: Job) -> Flow {
        match job {
            Job::Open { id, path, password } => self.open(id, &path, password),
            Job::Close { id } => self.close(id),
            Job::Tile { id, request, .. } => self.tile(id, &request),
            Job::PageText { id, page } => self.page_text(id, page),
            Job::Links { id, page } => self.links(id, page),
            Job::Outline { id } => self.outline(id),
            Job::Find(find) => self.find_step(find),
        }
    }

    fn open(&mut self, id: u64, path: &str, password: Option<Zeroizing<String>>) -> Flow {
        let fail = |error: PdfError| Reply::Opened {
            id,
            title: None,
            page_count: 0,
            error: Some(clean_error(error)),
        };
        if self.open {
            return self.send(
                &fail(PdfError::InvalidRequest(
                    "a document is already open".to_owned(),
                )),
                &[],
            );
        }
        if path.is_empty() || path.contains('\0') {
            return self.send(
                &fail(PdfError::InvalidRequest("not a usable path".to_owned())),
                &[],
            );
        }
        let outcome = self.call(&fail, |backend| {
            backend.open(Path::new(path), password.as_ref().map(|word| word.as_str()))
        })?;
        // The password is not needed any more.
        drop(password);
        let accepted = outcome.and_then(accept_document);
        match accepted {
            Ok(info) => {
                let reply = Reply::Opened {
                    id,
                    title: info.title.clone(),
                    page_count: info.page_count(),
                    error: None,
                };
                let blob = encode_page_sizes(&info.pages);
                self.pages = info.pages;
                self.open = true;
                self.send(&reply, &blob)
            }
            Err(error) => self.send(&fail(error), &[]),
        }
    }

    fn close(&mut self, id: u64) -> Flow {
        let fail = |error: PdfError| Reply::Error {
            id: Some(id),
            error,
        };
        // The backend cannot fail to close; a panic ends the loop through `?`.
        self.call(&fail, |backend| {
            backend.close();
            Ok(())
        })?
        .ok();
        self.open = false;
        self.pages.clear();
        self.send(&Reply::Closed { id }, &[])
    }

    fn tile(&mut self, id: u64, request: &TileRequest) -> Flow {
        let fail = |error: PdfError| Reply::Tile {
            id,
            width: 0,
            height: 0,
            error: Some(clean_error(error)),
        };
        if !self.open {
            return self.send(&fail(PdfError::NoDocument), &[]);
        }
        if let Err(error) = request.validate(&self.pages) {
            return self.send(&fail(error), &[]);
        }
        match self.call(&fail, |backend| backend.render(request))? {
            Ok(tile)
                if tile.is_consistent()
                    && tile.width == request.width
                    && tile.height == request.height =>
            {
                let reply = Reply::Tile {
                    id,
                    width: tile.width,
                    height: tile.height,
                    error: None,
                };
                self.send(&reply, &tile.rgba)
            }
            Ok(_) => self.send(
                &fail(PdfError::InvalidRequest(
                    "the engine returned a tile of the wrong size".to_owned(),
                )),
                &[],
            ),
            Err(error) => self.send(&fail(error), &[]),
        }
    }

    fn page_in_range(&self, page: u32) -> Result<(), PdfError> {
        if !self.open {
            Err(PdfError::NoDocument)
        } else if (page as usize) < self.pages.len() {
            Ok(())
        } else {
            Err(PdfError::InvalidRequest("no such page".to_owned()))
        }
    }

    fn page_text(&mut self, id: u64, page: u32) -> Flow {
        let fail = |error: PdfError| Reply::PageText {
            id,
            page,
            chars: 0,
            error: Some(clean_error(error)),
        };
        if let Err(error) = self.page_in_range(page) {
            return self.send(&fail(error), &[]);
        }
        match self.call(&fail, |backend| backend.page_text(page))? {
            Ok(mut text) => {
                text.chars.truncate(limits::MAX_TEXT_CHARS);
                for character in &mut text.chars {
                    if !finite_rect(&character.bounds) {
                        character.bounds = Rect::default();
                    }
                }
                let reply = Reply::PageText {
                    id,
                    page,
                    chars: u32::try_from(text.chars.len()).unwrap_or(u32::MAX),
                    error: None,
                };
                self.send(&reply, &encode_page_text(&text))
            }
            Err(error) => self.send(&fail(error), &[]),
        }
    }

    fn links(&mut self, id: u64, page: u32) -> Flow {
        let fail = |error: PdfError| Reply::Links {
            id,
            page,
            error: Some(clean_error(error)),
        };
        if let Err(error) = self.page_in_range(page) {
            return self.send(&fail(error), &[]);
        }
        match self.call(&fail, |backend| backend.links(page))? {
            Ok(links) => {
                let links = clean_links(links, self.pages.len());
                match encode_within_limit(&links) {
                    Ok(blob) => self.send(
                        &Reply::Links {
                            id,
                            page,
                            error: None,
                        },
                        &blob,
                    ),
                    Err(error) => self.send(&fail(error), &[]),
                }
            }
            Err(error) => self.send(&fail(error), &[]),
        }
    }

    fn outline(&mut self, id: u64) -> Flow {
        let fail = |error: PdfError| Reply::Outline {
            id,
            error: Some(clean_error(error)),
        };
        if !self.open {
            return self.send(&fail(PdfError::NoDocument), &[]);
        }
        match self.call(&fail, |backend| backend.outline())? {
            Ok(items) => {
                let mut budget = limits::MAX_OUTLINE_ITEMS;
                let items = clean_outline(items, self.pages.len(), &mut budget, 1);
                match encode_within_limit(&items) {
                    Ok(blob) => self.send(&Reply::Outline { id, error: None }, &blob),
                    Err(error) => self.send(&fail(error), &[]),
                }
            }
            Err(error) => self.send(&fail(error), &[]),
        }
    }

    /// Searches one page of a find and reports it; the find stays in the queue until its last
    /// page, so requests that arrive meanwhile are picked first.
    fn find_step(&mut self, find: FindJob) -> Flow {
        let id = find.id;
        let fail = |error: PdfError, page: u32| Reply::Found {
            find: id,
            page,
            finished: true,
            error: Some(clean_error(error)),
        };
        let count = u32::try_from(self.pages.len()).unwrap_or(u32::MAX);
        if find.next.is_none() {
            // The first step: check the request itself.
            let problem = if !self.open {
                Some(PdfError::NoDocument)
            } else if find.from_page >= count {
                Some(PdfError::InvalidRequest("no such page".to_owned()))
            } else if find.query.chars().count() > limits::MAX_QUERY_CHARS {
                Some(PdfError::InvalidRequest(
                    "the search text is too long".to_owned(),
                ))
            } else {
                None
            };
            if let Some(error) = problem {
                self.shared.advance_find(id, None);
                return self.send(&fail(error, find.from_page), &[]);
            }
            if find.query.is_empty() {
                // An empty query matches nothing; say so once.
                self.shared.advance_find(id, None);
                let reply = Reply::Found {
                    find: id,
                    page: find.from_page,
                    finished: true,
                    error: None,
                };
                return self.send(&reply, b"[]");
            }
        }
        let (cursor, remaining) = find.next.unwrap_or((find.from_page, count));
        let step_fail = |error: PdfError| fail(error, cursor);
        let hits = match self.call(&step_fail, |backend| {
            backend.find_in_page(cursor, &find.query, find.options)
        })? {
            Ok(hits) => clean_hits(hits, cursor),
            Err(error) => {
                // One page that cannot be searched does not end the find.
                log::warn!("pdf worker: searching page {cursor} failed: {error:?}");
                Vec::new()
            }
        };
        let finished = remaining <= 1;
        let next = if finished {
            None
        } else {
            Some(((cursor + 1) % count.max(1), remaining - 1))
        };
        if !self.shared.advance_find(id, next) {
            // Cancelled while this page was searched: say nothing.
            return Ok(());
        }
        match encode_within_limit(&hits) {
            Ok(blob) => self.send(
                &Reply::Found {
                    find: id,
                    page: cursor,
                    finished,
                    error: None,
                },
                &blob,
            ),
            Err(error) => {
                self.shared.advance_find(id, None);
                self.send(&fail(error, cursor), &[])
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Cleaning what the backend returns.
// ---------------------------------------------------------------------------------------------

fn panic_message(payload: &(dyn Any + Send)) -> String {
    let text = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned());
    clip(&text, DETAIL_BYTES)
}

/// A list as the JSON blob of a reply, refused (as too large) when it would not fit a frame.
fn encode_within_limit<T: serde::Serialize>(list: &[T]) -> Result<Vec<u8>, PdfError> {
    match encode_list(list) {
        Ok(blob) if blob.len() <= MAX_BLOB_BYTES => Ok(blob),
        Ok(_) => Err(PdfError::TooLarge("the answer is too large".to_owned())),
        Err(error) => Err(PdfError::InvalidRequest(clip(
            &error.to_string(),
            DETAIL_BYTES,
        ))),
    }
}
