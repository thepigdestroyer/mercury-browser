// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The harness's own test (a custom `main`, `harness = false`): a tiny scenario table with a passing
//! scenario, one that prints noise, one that panics, one that sleeps past its timeout and one that
//! aborts the process. This executable is both the parent (no arguments) and, when the parent starts
//! it again with the harness's own flags, the child that runs one scenario (`--child H3`) or the whole
//! command line (`--list`, `--only ...`). No GStreamer is involved (`--no-gstreamer`).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use mse_rig::harness::{self, Ctx, Priority, ScenarioDef, parse_args};
use mse_rig::report::{Report, ScenarioResult, Status};

fn passes(_: &Ctx) -> ScenarioResult {
    let mut result = ScenarioResult::new("H1", "passes");
    result.status = Status::Pass;
    result.metric("answer", 42).note("fine");
    result
}

fn prints_noise(_: &Ctx) -> ScenarioResult {
    println!("a line the scenario printed itself");
    println!("{{\"not\": \"a result\"}}");
    let mut result = ScenarioResult::new("H2", "prints noise first");
    result.status = Status::Fail;
    result.note("a finding, not an error");
    result
}

fn panics(_: &Ctx) -> ScenarioResult {
    panic!("boom from the scenario");
}

fn sleeps(_: &Ctx) -> ScenarioResult {
    std::thread::sleep(Duration::from_secs(120));
    ScenarioResult::new("H4", "sleeps")
}

fn aborts(_: &Ctx) -> ScenarioResult {
    std::process::abort();
}

fn uses_its_context(ctx: &Ctx) -> ScenarioResult {
    let mut result = ScenarioResult::new("H6", "reads its context");
    result.status = Status::Pass;
    result
        .metric("iterations", ctx.iterations as u64)
        .metric("seed", ctx.seed)
        .metric("scratch_exists", ctx.scratch.is_dir());
    std::fs::write(ctx.scratch.join("file"), b"x").ok();
    result
}

static TABLE: &[ScenarioDef] = &[
    ScenarioDef {
        id: "H1",
        title: "passes",
        priority: Priority::P0,
        timeout: Duration::from_secs(30),
        run: passes,
    },
    ScenarioDef {
        id: "H2",
        title: "prints noise first",
        priority: Priority::P1,
        timeout: Duration::from_secs(30),
        run: prints_noise,
    },
    ScenarioDef {
        id: "H3",
        title: "panics",
        priority: Priority::P0,
        timeout: Duration::from_secs(30),
        run: panics,
    },
    ScenarioDef {
        id: "H4",
        title: "sleeps past its timeout",
        priority: Priority::P2,
        timeout: Duration::from_secs(1),
        run: sleeps,
    },
    ScenarioDef {
        id: "H5",
        title: "aborts the process",
        priority: Priority::P0,
        timeout: Duration::from_secs(30),
        run: aborts,
    },
    ScenarioDef {
        id: "H6",
        title: "reads its context",
        priority: Priority::P1,
        timeout: Duration::from_secs(30),
        run: uses_its_context,
    },
];

fn result<'a>(report: &'a Report, id: &str) -> &'a ScenarioResult {
    report
        .results
        .iter()
        .find(|result| result.id == id)
        .unwrap_or_else(|| panic!("no result for {id}"))
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mse-rig-harness-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Runs this executable again with `args` and returns (exit code, stdout).
fn rerun(exe: &Path, args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(exe).args(args).output().expect("start myself");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn parent() -> ExitCode {
    let exe = std::env::current_exe().expect("my own path");

    // 1. The whole table: each outcome has its own status, and one Error makes the exit status 1.
    let scratch = scratch_dir("all");
    let cli = parse_args(&strings(&[
        "--no-gstreamer",
        "--scratch",
        &scratch.display().to_string(),
        "--iterations",
        "3",
        "--seed",
        "77",
    ]))
    .expect("a valid command line");
    let suite = harness::run_suite("harness-test", TABLE, &cli, &exe, |_| {}).expect("a run");
    let report = &suite.report;
    assert_eq!(report.results.len(), 6, "{}", report.summary());
    assert_eq!(suite.exit, 1, "an Error result must give exit status 1");

    let h1 = result(report, "H1");
    assert_eq!(h1.status, Status::Pass);
    assert_eq!(h1.metrics["answer"], 42);
    assert_eq!(
        h1.metrics["shape"], "playbin",
        "the child adds the run's options as metrics"
    );
    assert_eq!(h1.metrics["glib_main_loop"], false);

    let h2 = result(report, "H2");
    assert_eq!(
        h2.status,
        Status::Fail,
        "noise before the result line must not matter"
    );

    let h3 = result(report, "H3");
    assert_eq!(h3.status, Status::Error);
    assert!(
        h3.notes
            .iter()
            .any(|n| n.contains("panicked") && n.contains("boom from the scenario")),
        "{:?}",
        h3.notes
    );

    let h4 = result(report, "H4");
    assert_eq!(h4.status, Status::Error);
    assert!(
        h4.notes.iter().any(|n| n.contains("timed out after 1 s")),
        "{:?}",
        h4.notes
    );
    assert!(
        h4.elapsed_ms >= 900 && h4.elapsed_ms < 20_000,
        "killed at the limit: {} ms",
        h4.elapsed_ms
    );

    let h5 = result(report, "H5");
    assert_eq!(h5.status, Status::Error);
    assert!(
        h5.notes.iter().any(|n| n.contains("SIGABRT")),
        "an abort is named by its signal: {:?}",
        h5.notes
    );

    let h6 = result(report, "H6");
    assert_eq!(h6.status, Status::Pass);
    assert_eq!(h6.metrics["iterations"], 3);
    assert_eq!(h6.metrics["seed"], 77);
    assert_eq!(h6.metrics["scratch_exists"], true);
    assert!(
        !scratch.join("H6").exists(),
        "a scenario's scratch directory is removed after it"
    );

    assert_eq!(report.run.tool, "harness-test");
    assert_eq!(report.run.seed, 77);
    assert_eq!(report.run.iterations, 3);
    let json = report.to_json();
    assert!(
        !json.contains(&scratch.display().to_string()),
        "reports carry no paths"
    );
    let summary = report.summary();
    assert_eq!(summary.lines().count(), 6);
    assert!(summary.contains("ERROR H4"), "{summary}");
    let _ = std::fs::remove_dir_all(&scratch);

    // 2. --only, and exit status 0 when nothing is an Error (a Fail is a finding, not an error).
    let scratch = scratch_dir("only");
    let cli = parse_args(&strings(&[
        "--no-gstreamer",
        "--only",
        "H6,H1,H2",
        "--scratch",
        &scratch.display().to_string(),
    ]))
    .expect("a valid command line");
    let suite = harness::run_suite("harness-test", TABLE, &cli, &exe, |_| {}).expect("a run");
    assert_eq!(suite.exit, 0);
    let ids: Vec<&str> = suite.report.results.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["H1", "H2", "H6"], "table order, only the chosen ones");
    let _ = std::fs::remove_dir_all(&scratch);

    // 3. An unknown --only ID is refused before anything runs.
    let cli = parse_args(&strings(&["--only", "H9"])).expect("parses");
    let error = harness::run_suite("harness-test", TABLE, &cli, &exe, |_| {}).unwrap_err();
    assert!(error.contains("H9"), "{error}");

    // 4. The real `main_for`, as a process: --list, usage errors, --report, exit statuses.
    let (code, stdout) = rerun(&exe, &["--list"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout.lines().count(), 6, "{stdout}");
    assert!(stdout.contains("H4") && stdout.contains("sleeps past its timeout"));

    let (code, _) = rerun(&exe, &["--only", "NOPE", "--no-gstreamer"]);
    assert_eq!(code, Some(2), "an unknown scenario is a usage error");
    let (code, _) = rerun(&exe, &["--shape", "triangle"]);
    assert_eq!(code, Some(2), "a bad option value is a usage error");

    let scratch = scratch_dir("main");
    let report_path = scratch.join("report.json");
    let (code, stdout) = rerun(
        &exe,
        &[
            "--no-gstreamer",
            "--only",
            "H1,H6",
            "--report",
            &report_path.display().to_string(),
            "--scratch",
            &scratch.join("work").display().to_string(),
        ],
    );
    assert_eq!(code, Some(0), "{stdout}");
    assert!(stdout.contains("PASS  H1"), "{stdout}");
    let written: Report =
        serde_json::from_str(&std::fs::read_to_string(&report_path).expect("the report file"))
            .expect("the report is JSON");
    assert_eq!(written.results.len(), 2);
    let (code, _) = rerun(
        &exe,
        &[
            "--no-gstreamer",
            "--only",
            "H3",
            "--scratch",
            &scratch.join("work2").display().to_string(),
        ],
    );
    assert_eq!(code, Some(1), "a panicking scenario gives exit status 1");
    let _ = std::fs::remove_dir_all(&scratch);

    println!("harness_protocol: ok");
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Started with the harness's own flags (by the parent, or by `rerun`): be the scenario binary.
    // Anything else (no arguments, or what cargo passes to a test binary) runs the checks.
    let harness_flags = [
        "--child",
        "--child-init",
        "--list",
        "--only",
        "--shape",
        "--plugin-mode",
        "--report",
        "--scratch",
        "--iterations",
        "--seed",
        "--glib-main-loop",
        "--no-gstreamer",
        "--help",
    ];
    let is_harness_call = args.iter().any(|argument| {
        let flag = argument.split('=').next().unwrap_or_default();
        harness_flags.contains(&flag)
    });
    if is_harness_call {
        return harness::main_for("harness-test", TABLE);
    }
    parent()
}
