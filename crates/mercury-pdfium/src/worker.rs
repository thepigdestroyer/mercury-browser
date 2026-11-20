// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The PDF helper process: `mercury-browser --pdf-worker`.
//!
//! [`run_worker_if_requested`] is the first thing `main` calls. If `--pdf-worker` is among the
//! arguments it **never returns to the caller's normal start-up**: it turns itself into the helper
//! (the request loop of `mercury_pdf::serve` on standard input and output, with PDFium behind
//! it), and ends the process when the loop ends. The helper is started by the browser's
//! `PdfClient`; it is not for people:
//!
//! - it **refuses to run on a terminal** (its input is a binary protocol);
//! - **core dumps are switched off** (a crash dump would hold the document and any password), and
//!   on Linux the process is made non-dumpable;
//! - `SIGINT`, `SIGHUP` and `SIGTERM` are **ignored**, like the content process does: the browser
//!   ends the helper (and so does the end of its standard input, if the browser dies);
//! - the **protocol owns the real standard output**: the descriptor is duplicated for the protocol
//!   and descriptor 1 is pointed at standard error, so that anything else that prints (a C
//!   library, a stray `println!`) lands in the log instead of corrupting a frame;
//! - the library is loaded from `MERCURY_PDFIUM_LIBRARY` (set by the client; the helper's
//!   environment is otherwise empty) or, failing that, where [`crate::locate_library`] looks. If
//!   it cannot be loaded, the helper says so in one error frame and ends with status 1: the
//!   client reports `LibraryMissing`.
//!
//! The exit status is 0 after an orderly end, 70 after a panic of the engine, 1 when the library
//! cannot be loaded and 2 when started on a terminal.
//!
//! This is the only module of the viewer with `unsafe` code, and each use says why it is sound.

#![allow(unsafe_code)]

use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::os::fd::FromRawFd as _;

use mercury_pdf::error::PdfError;
use mercury_pdf::protocol::{Reply, write_reply};
use mercury_pdf::serve::{ServeExit, serve_with_engine};

use crate::backend::PdfiumBackend;
use crate::library::locate_library;

/// The argument that makes a process the helper.
pub const WORKER_FLAG: &str = "--pdf-worker";

/// A logger for the helper: warnings and errors to standard error.
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("pdf-worker [{}]: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

/// Whether `--pdf-worker` is among the process's arguments.
pub fn worker_requested() -> bool {
    std::env::args_os().any(|argument| argument == WORKER_FLAG)
}

/// If this process was started as the PDF helper, runs it and exits the process; returns `false`
/// at once otherwise. After an orderly end it returns `true` (the caller then returns from
/// `main`); after any other end it exits the process with the status itself.
pub fn run_worker_if_requested() -> bool {
    if !worker_requested() {
        return false;
    }
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Warn);
    // The browser's own panic hook belongs to the browser (it consults the engine's options and
    // may stop the process); the helper's backend panics are caught by the request loop, which
    // answers the request and exits with status 70, so a hook that only reports is what it needs.
    std::panic::set_hook(Box::new(|info| eprintln!("pdf-worker: panic: {info}")));

    if stdin_is_a_terminal() {
        eprintln!(
            "{WORKER_FLAG} is started by the browser and speaks a binary protocol on its standard \
             input and output; it cannot be run from a terminal."
        );
        std::process::exit(2);
    }
    harden_process();
    ignore_signals();

    // Keep the real standard output for the protocol; point descriptor 1 at standard error.
    let Some(protocol_output) = take_standard_output() else {
        eprintln!("pdf-worker: could not take over standard output");
        std::process::exit(1);
    };
    let mut output = BufWriter::with_capacity(64 * 1024, protocol_output);

    let library = locate_library();
    let backend = match library
        .as_deref()
        .ok_or_else(|| PdfError::LibraryMissing("no PDFium library was found".to_owned()))
        .and_then(PdfiumBackend::load)
    {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("pdf-worker: {error:?}");
            let reply = Reply::Error { id: None, error };
            let _ = write_reply(&mut output, &reply, &[]);
            let _ = output.flush();
            std::process::exit(1);
        }
    };

    let exit = serve_with_engine("pdfium", backend, std::io::stdin(), output);
    match &exit {
        ServeExit::Eof | ServeExit::OutputClosed => {}
        ServeExit::Protocol(detail) => eprintln!("pdf-worker: malformed request stream: {detail}"),
        ServeExit::Panicked(message) => eprintln!("pdf-worker: the engine panicked: {message}"),
    }
    match exit.status() {
        0 => true,
        status => std::process::exit(status),
    }
}

fn stdin_is_a_terminal() -> bool {
    // SAFETY: `isatty` only reads the state of a file descriptor; descriptor 0 is always a valid
    // number to ask about (it returns 0 if nothing is open there).
    unsafe { libc::isatty(0) == 1 }
}

/// No core dumps, and on Linux no ptrace or dump by processes of the same user.
fn harden_process() {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `setrlimit` reads the struct it is given, which lives for the call; lowering
    // the core file size limit to zero cannot fail in a way that matters (the return value is
    // checked and only logged).
    let status = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) };
    if status != 0 {
        log::warn!("could not switch core dumps off");
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `prctl(PR_SET_DUMPABLE, 0)` takes plain integers and changes only this
        // process's own attribute.
        let status = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
        if status != 0 {
            log::warn!("could not make the process non-dumpable");
        }
    }
}

fn ignore_signals() {
    for signal in [libc::SIGINT, libc::SIGHUP, libc::SIGTERM] {
        // SAFETY: `signal` with `SIG_IGN` installs no handler code, so there is nothing that
        // could run in a signal context; the signal numbers are valid constants.
        unsafe {
            libc::signal(signal, libc::SIG_IGN);
        }
    }
}

/// Duplicates descriptor 1 for the protocol and points descriptor 1 at standard error.
fn take_standard_output() -> Option<File> {
    // SAFETY: `dup` and `dup2` act on descriptors this process owns; descriptor 1 is the
    // protocol pipe, which the browser opened for the helper.
    let protocol = unsafe { libc::dup(1) };
    if protocol < 0 {
        return None;
    }
    // SAFETY: as above; descriptor 2 is standard error.
    if unsafe { libc::dup2(2, 1) } < 0 {
        // SAFETY: `protocol` is the descriptor `dup` just returned and nothing else owns it.
        unsafe { libc::close(protocol) };
        return None;
    }
    // SAFETY: `protocol` is a valid, open descriptor that nothing else owns, so the `File` may
    // take ownership of it and close it.
    Some(unsafe { File::from_raw_fd(protocol) })
}
