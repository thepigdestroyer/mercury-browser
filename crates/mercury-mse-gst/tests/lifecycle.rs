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

/// The same lifecycle with no MSE in it: `playbin3` on the fixture file, the same decoders and sinks
/// (decision D14's control).
fn one_control_lifecycle() -> Duration {
    let path = mercury_mse_testkit::fixtures_dir().join("fmp4/muxed-h264-aac.mp4");
    let rig = Rig::control(
        &path,
        Options {
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

/// `iterations` lifecycles of `run`, RSS sampled every ten: the slope in kB per iteration and the slowest
/// teardown.
fn rss_slope(iterations: usize, mut run: impl FnMut() -> Duration) -> (f64, f64, Duration) {
    let rss0 = rss_kb();
    let mut rss = vec![rss0];
    let mut slowest = Duration::ZERO;
    for i in 0..iterations {
        slowest = slowest.max(run());
        if (i + 1) % 10 == 0 {
            rss.push(rss_kb());
        }
    }
    (
        slope(&rss) / 10.0,
        rss.last().copied().unwrap_or(rss0),
        slowest,
    )
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
    watchdog("t04", 1800, move || {
        let warm_up = 5;
        for _ in 0..warm_up {
            one_lifecycle(Shape::PlayBin);
            one_control_lifecycle();
        }
        // Phases in one process, `C` the control and `M` MSE. The default is C, M, C: the controls bracket
        // MSE, so that growth that only comes from the process getting older (allocator and caches
        // filling) shows in both. `MSE_T04_ORDER` (for example `MMCC`) runs another order and only
        // prints: that is how the attribution in the notes was made.
        let custom = std::env::var("MSE_T04_ORDER").ok();
        let order = custom.clone().unwrap_or_else(|| "CMC".to_owned());
        let mut results: Vec<(char, f64, f64)> = Vec::new();
        let (mut threads0, mut fds0) = (0, 0);
        let (mut threads1, mut fds1) = (0, 0);
        let mut slowest = Duration::ZERO;
        for phase in order.chars() {
            let (slope, end, _) = if phase == 'M' {
                (threads0, fds0) = (threads(), fds());
                let result = rss_slope(iterations, || one_lifecycle(Shape::PlayBin));
                (threads1, fds1) = (threads(), fds());
                slowest = slowest.max(result.2);
                result
            } else {
                rss_slope(iterations, one_control_lifecycle)
            };
            results.push((phase, slope, end));
        }
        let summary: Vec<String> = results
            .iter()
            .map(|(phase, slope, end)| {
                let name = if *phase == 'M' { "MSE" } else { "control" };
                format!("{name} {slope:.1} kB/iteration (RSS ends {end:.0} kB)")
            })
            .collect();
        eprintln!(
            "T04 {iterations} lifecycles per phase: {}; MSE threads {threads0} -> {threads1}, fds {fds0} -> {fds1}, \
             slowest MSE teardown {slowest:?}",
            summary.join("; ")
        );
        if custom.is_some() {
            return;
        }
        assert!(
            threads1 <= threads0 + 2 && threads1 + 2 >= threads0,
            "threads {threads0} -> {threads1}"
        );
        assert!(fds1 <= fds0 + 2 && fds1 + 2 >= fds0, "fds {fds0} -> {fds1}");
        // D14: the growth is attributed, not tolerated. The control (the same `playbin3`, decoders and
        // sinks on a file, no MSE anywhere) shows the growth too: the cause is on the GStreamer side
        // (the notes of lane med-1b, item 2: it is the process getting older, the first hundreds of
        // pipelines fill the allocator and the decoders' caches, and it falls to tens of kB). So the
        // limit is the larger slope of the two controls plus 100 kB per iteration, the room for the
        // source element's own appsrcs and thread. The memory MSE owns is checked exactly elsewhere
        // (`mercury-mse` `model_t04_lifecycle`: the shared budget reads zero after 220 lifecycles).
        let (mse, control) = (results[1].1, results[0].1.max(results[2].1));
        assert!(
            mse < control + 100.0,
            "MSE RSS slope {mse:.1} kB per iteration against {control:.1} for the control"
        );
        assert!(
            slowest < Duration::from_secs(2),
            "slowest teardown {slowest:?}"
        );
    });
}
