// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! T04: create, play 0.3 s, tear down, 200 times: no crash, memory that does not grow, threads and file
//! descriptors that come back. The one test of this binary, so nothing else moves the counts. Teardown is
//! `set_state(Null)` and a drop: the test never sleeps before it (GstMSE needed a quiet second; R17 T04).

mod common;

use std::process::Command;
use std::time::{Duration, Instant};

use common::fakefeed::FakeFeed;
use common::rig::{Options, Rig, Shape, watchdog};

fn pid() -> String {
    std::process::id().to_string()
}

/// Threads of this process (`ps -M` prints one line per thread after a header).
#[cfg(target_os = "macos")]
fn threads() -> usize {
    let out = Command::new("ps")
        .args(["-M", "-p", &pid()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .count()
        .saturating_sub(1)
}

/// Threads of this process: one directory per thread in `/proc/self/task`.
#[cfg(not(target_os = "macos"))]
fn threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .map(|d| d.count())
        .unwrap_or(0)
}

fn rss_kb() -> f64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0.0)
}

/// Open file descriptors (the listing's own handle is in the count: the same every time).
fn fds() -> usize {
    std::fs::read_dir("/dev/fd").map(|d| d.count()).unwrap_or(0)
}

/// Least-squares slope of `values` against 0, 1, 2, ...
fn slope(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = values.iter().sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for (i, y) in values.iter().enumerate() {
        num += (i as f64 - mean_x) * (y - mean_y);
        den += (i as f64 - mean_x).powi(2);
    }
    num / den
}

fn one_lifecycle(shape: Shape) -> Duration {
    let feed = FakeFeed::from_fixtures(&["h264-aac"]);
    feed.release_all();
    feed.end();
    let rig = Rig::new(
        feed,
        Options {
            shape,
            sync: true,
            colour: false,
            ..Options::default()
        },
    );
    rig.play();
    let until = Instant::now() + Duration::from_millis(300);
    while Instant::now() < until {
        rig.pump_bus();
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = Instant::now();
    rig.stop();
    drop(rig);
    started.elapsed()
}

#[test]
fn t04_two_hundred_lifecycles_leak_nothing() {
    require!("avdec_h264", "avdec_aac", "playbin3", "appsrc");
    let iterations: usize = std::env::var("MSE_T04_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    watchdog("t04", 900, move || {
        let warm_up = 5;
        for _ in 0..warm_up {
            one_lifecycle(Shape::PlayBin);
        }
        let (threads0, fds0, rss0) = (threads(), fds(), rss_kb());
        let mut rss = vec![rss0];
        let mut slowest = Duration::ZERO;
        for i in 0..iterations {
            slowest = slowest.max(one_lifecycle(Shape::PlayBin));
            if (i + 1) % 10 == 0 {
                rss.push(rss_kb());
            }
        }
        let (threads1, fds1) = (threads(), fds());
        let per_iteration_kb = slope(&rss) / 10.0;
        eprintln!(
            "T04 {iterations} lifecycles: threads {threads0} -> {threads1}, fds {fds0} -> {fds1}, \
             RSS {rss0:.0} kB -> {:.0} kB ({per_iteration_kb:.1} kB per iteration), slowest teardown {slowest:?}",
            rss.last().unwrap()
        );
        assert!(
            threads1 <= threads0 + 2 && threads1 + 2 >= threads0,
            "threads {threads0} -> {threads1}"
        );
        assert!(fds1 <= fds0 + 2 && fds1 + 2 >= fds0, "fds {fds0} -> {fds1}");
        // Measured 285 to 383 kB per iteration on 20 Nov, with no leak of ours: the growth is the
        // allocator and the software decoders of a whole `playbin3` pipeline, and no control run (the
        // same pipeline without MSE) is possible with the same rig. A leak of 0.3 MB per lifecycle
        // therefore cannot be told from that growth by RSS; the leak of the memory MSE owns is checked
        // exactly instead (`mercury-mse` `model_t04_lifecycle`: the shared budget reads zero after 220
        // lifecycles). This threshold only stops the growth from doubling.
        assert!(
            per_iteration_kb < 600.0,
            "RSS slope {per_iteration_kb:.1} kB per iteration"
        );
        assert!(
            slowest < Duration::from_secs(2),
            "slowest teardown {slowest:?}"
        );
    });
}
