// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! One case of a scenario in a process of its own, so that a crash or a hang inside GstMSE costs that case and
//! not the others' results (a crash kills the scenario's child process before it can print anything).
//!
//! The scenario re-runs this executable with `--only <ID>` (and the shape, plugin mode and main-loop options of the
//! current run) and the environment variable [`CASE_ENV`] naming the case; that run executes only that case and
//! writes its own report, which is merged into the scenario's result: every metric, every check with its detail,
//! every note. A process that dies or times out is a finding ("the case crashed: killed by signal 11"), the
//! exact thing the harness would otherwise have shown as an `Error` for the whole scenario.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use mse_rig::harness::Ctx;
use mse_rig::report::{Report, ScenarioResult, Status};

use crate::support::{Abort, Res, Scn, plugin_mode_name, shape_name};

/// The environment variable that tells a re-run which case to execute.
pub const CASE_ENV: &str = "MSE_CONFORMANCE_CASE";

/// Keys the sub-run records for itself, not worth copying.
const BOOKKEEPING: &[&str] = &[
    "shape",
    "plugin_mode",
    "glib_main_loop",
    "gst_version",
    "arch",
    "child_stderr_lines",
];

/// The case this process was asked to run, if it is a re-run.
pub fn selected_case() -> Option<String> {
    std::env::var(CASE_ENV).ok().filter(|name| !name.is_empty())
}

/// Runs `case` of scenario `id` in a child process and merges its result into `scn`.
pub fn run_case_in_child(
    ctx: &Ctx,
    scn: &mut Scn,
    id: &str,
    case: &str,
    timeout: Duration,
) -> Res<()> {
    let executable = std::env::current_exe()
        .map_err(|error| Abort::Error(format!("cannot find the running executable: {error}")))?;
    let report_path = ctx.scratch.join(format!("{id}-{case}.json"));
    let _ = std::fs::remove_file(&report_path);
    let mut command = Command::new(executable);
    command
        .args(["--only", id])
        .args(["--shape", shape_name(ctx.shape)])
        .args(["--plugin-mode", plugin_mode_name(&ctx.plugin_mode)])
        .arg("--report")
        .arg(&report_path)
        .env(CASE_ENV, case)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if ctx.glib_main_loop {
        command.arg("--glib-main-loop");
    }
    let mut child = command
        .spawn()
        .map_err(|error| Abort::Error(format!("cannot start the sub-run of {case}: {error}")))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => {
                return Err(Abort::Error(format!(
                    "waiting for the sub-run of {case}: {error}"
                )));
            }
        }
    };
    scn.metric(
        &format!("{case}.sub_run_ms"),
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
    let Some(status) = status else {
        scn.finding(format!(
            "case {case}: hung, killed after {timeout:?} (GstMSE or the pipeline never returned)"
        ));
        return Ok(());
    };
    let report = std::fs::read_to_string(&report_path)
        .ok()
        .and_then(|text| serde_json::from_str::<Report>(&text).ok());
    let Some(result) = report.and_then(|report| report.results.into_iter().next()) else {
        scn.finding(format!(
            "case {case}: the sub-run left no result (exit status {status}): the process died before it could report"
        ));
        return Ok(());
    };
    merge(scn, case, &result);
    Ok(())
}

/// Copies a sub-run's result into the scenario's: metrics, checks (with the detail of a failed one from its
/// `FAIL <name>: <detail>` note), findings, notes. An `Error` of the sub-run (a crash, a hang the harness
/// killed, a panic) becomes a finding.
fn merge(scn: &mut Scn, case: &str, result: &ScenarioResult) {
    for (key, value) in &result.metrics {
        if key.starts_with("check.") || BOOKKEEPING.contains(&key.as_str()) {
            continue;
        }
        scn.metric(key, value.clone());
    }
    for (key, value) in &result.metrics {
        let Some(name) = key.strip_prefix("check.") else {
            continue;
        };
        let detail = result
            .notes
            .iter()
            .find_map(|note| note.strip_prefix(&format!("FAIL {name}: ")))
            .unwrap_or_default();
        scn.check(name, value.as_bool().unwrap_or(false), detail);
    }
    for note in &result.notes {
        if note.starts_with("ran with ") || note.starts_with("FAIL ") {
            continue;
        }
        if let Some(finding) = note.strip_prefix("FINDING ") {
            scn.finding(finding);
        } else if let Some(stopped) = note.strip_prefix("STOPPED by a GstMSE failure: ") {
            scn.finding(format!("case {case} stopped: {stopped}"));
        } else if note.starts_with("HARNESS ERROR: ") || note.starts_with("SKIPPED: ") {
            // handled by the status below
        } else if note.starts_with("rig: ") {
            scn.note_once(note.clone());
        } else {
            scn.note(note.clone());
        }
    }
    match result.status {
        Status::Error => {
            let reason = result
                .notes
                .iter()
                .find(|note| note.starts_with("HARNESS ERROR: "))
                .map_or("no reason given", |note| &note["HARNESS ERROR: ".len()..]);
            scn.finding(format!(
                "case {case} did not finish: {reason} (the sub-run's own notes: {})",
                result
                    .notes
                    .iter()
                    .filter(|note| note.starts_with("stderr: ") || note.starts_with("the child"))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
        Status::Skip => {
            scn.note(format!("case {case} was skipped: {}", skip_reason(result)));
        }
        Status::Pass | Status::Fail => {}
    }
}

fn skip_reason(result: &ScenarioResult) -> String {
    result
        .notes
        .iter()
        .find_map(|note| note.strip_prefix("SKIPPED: "))
        .unwrap_or("no reason given")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub_result(status: Status) -> ScenarioResult {
        let mut result = ScenarioResult::new("C13", "t");
        result.status = status;
        result
            .metric("shape", "bare")
            .metric("garbage.outcome", "UpdateEnd without an error")
            .metric("check.garbage.defined-error", false)
            .metric("check.garbage.something", true)
            .metric("child_stderr_lines", 3);
        result.note("ran with --shape bare ...");
        result.note("FAIL garbage.defined-error: bad data ended as: UpdateEnd");
        result.note("FINDING garbage: shutdown failed: timed out");
        result.note("garbage: after the bad append the MediaSource is Open");
        result
    }

    #[test]
    fn a_sub_result_is_merged_with_its_checks_findings_and_notes() {
        let mut scn = Scn::for_tests();
        merge(&mut scn, "garbage", &sub_result(Status::Fail));
        let merged = scn.finish(Ok(()));
        assert_eq!(merged.status, Status::Fail);
        assert_eq!(
            merged.metrics["garbage.outcome"],
            "UpdateEnd without an error"
        );
        assert_eq!(merged.metrics["check.garbage.defined-error"], false);
        assert_eq!(merged.metrics["check.garbage.something"], true);
        assert!(!merged.metrics.contains_key("shape"));
        assert!(!merged.metrics.contains_key("child_stderr_lines"));
        assert!(
            merged
                .notes
                .iter()
                .any(|n| n == "FAIL garbage.defined-error: bad data ended as: UpdateEnd")
        );
        assert!(
            merged
                .notes
                .iter()
                .any(|n| n.starts_with("FINDING garbage: shutdown failed"))
        );
        assert!(
            merged
                .notes
                .iter()
                .any(|n| n == "garbage: after the bad append the MediaSource is Open")
        );
        assert!(!merged.notes.iter().any(|n| n.starts_with("ran with")));
    }

    #[test]
    fn a_crashed_sub_run_becomes_a_finding_with_what_it_said() {
        let mut crashed = ScenarioResult::new("C13", "t");
        crashed.status = Status::Error;
        crashed.note("ran with --shape bare");
        crashed.note("the child killed by signal 11 (SIGSEGV) and left no result line");
        crashed.note("stderr: [C13] case init-twice");
        crashed
            .note("HARNESS ERROR: the child killed by signal 11 (SIGSEGV) and left no result line");
        let mut scn = Scn::for_tests();
        merge(&mut scn, "init-twice", &crashed);
        let merged = scn.finish(Ok(()));
        assert_eq!(merged.status, Status::Fail);
        let finding = merged
            .notes
            .iter()
            .find(|n| n.starts_with("FINDING case init-twice did not finish"))
            .expect("a finding");
        assert!(finding.contains("signal 11"), "{finding}");
        assert!(
            finding.contains("stderr: [C13] case init-twice"),
            "{finding}"
        );
    }

    #[test]
    fn a_skipped_sub_run_is_only_a_note() {
        let mut skipped = ScenarioResult::new("C13", "t");
        skipped.status = Status::Skip;
        skipped.note("SKIPPED: fixture vp9-low is not available");
        let mut scn = Scn::for_tests();
        merge(&mut scn, "webm-into-mp4", &skipped);
        let merged = scn.finish(Ok(()));
        assert_eq!(merged.status, Status::Pass);
        assert!(
            merged
                .notes
                .iter()
                .any(|n| n.contains("was skipped: fixture vp9-low"))
        );
    }

    #[test]
    fn selected_case_reads_the_environment_name() {
        assert_eq!(CASE_ENV, "MSE_CONFORMANCE_CASE");
        // Not set in the test process: an ordinary run.
        assert!(std::env::var(CASE_ENV).is_err() || selected_case().is_some());
    }
}
