// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Prints how a fragmented MP4 or WebM file splits: `run.sh run -p mse-segments --example dump -- FILE...`
//! (the container is taken from the extension: `.mp4`/`.m4s` or `.webm`).

use std::process::ExitCode;

use mse_segments::{Container, sha256_hex, split};

fn main() -> ExitCode {
    let mut failed = false;
    for path in std::env::args().skip(1) {
        let container = if path.ends_with(".webm") || path.ends_with(".mkv") {
            Container::WebM
        } else {
            Container::Mp4
        };
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{path}: {e}");
                failed = true;
                continue;
            }
        };
        match split(container, &data, "") {
            Ok(s) => {
                println!("{path}: {} bytes, sha256 {}", data.len(), sha256_hex(&data));
                println!("  mime {}", s.mime);
                println!("  init {} bytes", s.init.len());
                for (i, m) in s.media.iter().enumerate() {
                    println!(
                        "  media {i}: {} bytes, start {:?} ms, duration {:?} ms",
                        m.len(),
                        m.start_ms,
                        m.duration_ms
                    );
                }
            }
            Err(e) => {
                println!("{path}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
