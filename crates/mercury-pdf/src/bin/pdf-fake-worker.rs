// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A helper process for tests: the real request loop (`mercury_pdf::serve`) over standard input
//! and output with the fake backend, steered by command line words.
//!
//! The words are those of `FakeConfig::from_args` (`--pages=N`, `--password=P`,
//! `--render-delay-ms=N`, `--panic-after=K`, `--corrupt`, ...), plus three that only this
//! program knows, for the client's tests:
//!
//! - `--report-env`: the document title is the comma-separated names of the environment
//!   variables the process was given;
//! - `--garbage`: after the ready frame, writes bytes that are no frame and ends;
//! - `--silent`: never says it is ready and never exits by itself.
//!
//! It is never part of the browser; only the tests build it.

use std::io::Write as _;

use mercury_pdf::fake::{FakeBackend, FakeConfig};
use mercury_pdf::protocol::{PROTOCOL_VERSION, Reply, write_reply};
use mercury_pdf::serve::serve_with_engine;

fn main() {
    let mut words: Vec<String> = std::env::args().skip(1).collect();
    let mut take = |word: &str| {
        let before = words.len();
        words.retain(|candidate| candidate != word);
        words.len() != before
    };
    let report_env = take("--report-env");
    let garbage = take("--garbage");
    let silent = take("--silent");
    let mut config = match FakeConfig::from_args(&words) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("pdf-fake-worker: {message}");
            std::process::exit(2);
        }
    };
    if report_env {
        let mut names: Vec<String> = std::env::vars().map(|(name, _)| name).collect();
        names.sort();
        config.title = Some(names.join(","));
    }
    if silent {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    if garbage {
        let mut output = std::io::stdout();
        let ready = Reply::Ready {
            protocol_version: PROTOCOL_VERSION,
            engine: "garbage".to_owned(),
        };
        let _ = write_reply(&mut output, &ready, &[]);
        let _ = output.write_all(&[0xFF; 16]);
        let _ = output.flush();
        return;
    }
    let exit = serve_with_engine(
        "fake",
        FakeBackend::new(config),
        std::io::stdin(),
        std::io::stdout(),
    );
    std::process::exit(exit.status());
}
