// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Sub-runs: a scenario that must survive a crash or a deadlock of what it tests (T02's matrix, T04's
//! teardown orders, T12's seeds) starts the same executable again with `--only <ID>` and an
//! environment variable that tells the nested run which part to do, and reads the JSON report the
//! nested run writes. A cell that deadlocks or aborts then costs one process and not the matrix.

use std::collections::BTreeMap;
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mse_rig::env::PluginMode;
use mse_rig::harness::Ctx;
use mse_rig::report::{Report, ScenarioResult};
use mse_rig::rig::Shape;

/// One nested run.
#[derive(Debug, Clone)]
pub struct SubSpec {
    /// Scenario id passed to `--only`.
    pub id: String,
    /// A short name for files and notes (`cell-3`, `seed-17`, `order-2`).
    pub tag: String,
    /// Environment variables for the nested run (the scenario reads them to pick its part).
    pub env: Vec<(String, String)>,
    pub iterations: Option<usize>,
    pub seed: Option<u64>,
    /// A safety net: the whole process group is killed after this long.
    pub deadline: Duration,
}

#[derive(Debug, Clone)]
pub enum SubOutcome {
    /// The nested run wrote a report with this result.
    Report {
        result: ScenarioResult,
        exit: String,
    },
    /// No usable report: the process died or was killed. `tail` is the last lines of its output.
    Died {
        exit: String,
        tail: Vec<String>,
    },
    /// The safety-net deadline passed and the process group was killed.
    TimedOut {
        after: Duration,
        tail: Vec<String>,
    },
    CouldNotStart(String),
    /// The overall time budget of the driver ran out before this nested run could start.
    NotRun,
}

impl SubOutcome {
    /// A one-line description for notes.
    pub fn describe(&self) -> String {
        match self {
            SubOutcome::Report { result, exit } => {
                format!("report {:?} (exit {exit})", result.status)
            }
            SubOutcome::Died { exit, tail } => format!(
                "died without a report ({exit}); last output: {}",
                tail.join(" | ")
            ),
            SubOutcome::TimedOut { after, tail } => format!(
                "killed after {} s without a report; last output: {}",
                after.as_secs(),
                tail.join(" | ")
            ),
            SubOutcome::CouldNotStart(why) => format!("could not start: {why}"),
            SubOutcome::NotRun => "not run: the driver's time budget ran out first".to_owned(),
        }
    }
}

pub fn shape_arg(shape: Shape) -> &'static str {
    match shape {
        Shape::Bare => "bare",
        Shape::PlayBin => "playbin",
    }
}

pub fn plugin_mode_arg(mode: &PluginMode) -> &'static str {
    match mode {
        PluginMode::SystemPath => "system",
        PluginMode::ServoStyle { .. } => "servo",
    }
}

/// The command line of a nested run (without the executable).
pub fn build_args(
    shape: Shape,
    mode: &PluginMode,
    glib_main_loop: bool,
    spec: &SubSpec,
    report: &Path,
    scratch: &Path,
) -> Vec<String> {
    let mut args = vec![
        "--only".to_owned(),
        spec.id.clone(),
        "--shape".to_owned(),
        shape_arg(shape).to_owned(),
        "--plugin-mode".to_owned(),
        plugin_mode_arg(mode).to_owned(),
    ];
    if glib_main_loop {
        args.push("--glib-main-loop".to_owned());
    }
    if let Some(iterations) = spec.iterations {
        args.push("--iterations".to_owned());
        args.push(iterations.to_string());
    }
    if let Some(seed) = spec.seed {
        args.push("--seed".to_owned());
        args.push(seed.to_string());
    }
    args.push("--report".to_owned());
    args.push(report.display().to_string());
    args.push("--scratch".to_owned());
    args.push(scratch.display().to_string());
    args
}

struct Running {
    child: Child,
    started: Instant,
    spec: SubSpec,
    report: PathBuf,
    log: PathBuf,
}

fn start(ctx: &Ctx, spec: &SubSpec) -> Result<Running, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let dir = ctx.scratch.join(format!("sub-{}", spec.tag));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let report = dir.join("report.json");
    let log = dir.join("output.txt");
    let scratch = dir.join("scratch");
    let args = build_args(
        ctx.shape,
        &ctx.plugin_mode,
        ctx.glib_main_loop,
        spec,
        &report,
        &scratch,
    );
    let output = File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let output_err = output.try_clone().map_err(|e| format!("{e}"))?;
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(output))
        .stderr(Stdio::from(output_err))
        .process_group(0);
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    let child = command.spawn().map_err(|e| format!("spawn: {e}"))?;
    Ok(Running {
        child,
        started: Instant::now(),
        spec: spec.clone(),
        report,
        log,
    })
}

fn tail_of(path: &Path, lines: usize) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|line| (*line).to_owned())
        .collect()
}

fn read_report(path: &Path) -> Option<ScenarioResult> {
    let text = std::fs::read_to_string(path).ok()?;
    let report: Report = serde_json::from_str(&text).ok()?;
    report.results.into_iter().next()
}

fn kill_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-s", "KILL", "--", &format!("-{pid}")])
        .output();
}

/// Runs every spec, at most `parallel` at a time, and returns the outcomes in the order of `specs`.
/// After `budget` no further run is started (those are `NotRun`) and the ones still running are
/// killed (`TimedOut`): the driver itself has a time limit from the harness, and a driver that is killed
/// would leave its nested runs behind.
pub fn run_all(ctx: &Ctx, specs: &[SubSpec], parallel: usize, budget: Duration) -> Vec<SubOutcome> {
    let parallel = parallel.max(1);
    let began = Instant::now();
    let mut outcomes: BTreeMap<usize, SubOutcome> = BTreeMap::new();
    let mut running: Vec<(usize, Running)> = Vec::new();
    let mut next = 0;
    while outcomes.len() < specs.len() {
        while running.len() < parallel && next < specs.len() {
            if began.elapsed() > budget {
                outcomes.insert(next, SubOutcome::NotRun);
                next += 1;
                continue;
            }
            match start(ctx, &specs[next]) {
                Ok(job) => running.push((next, job)),
                Err(why) => {
                    outcomes.insert(next, SubOutcome::CouldNotStart(why));
                }
            }
            next += 1;
        }
        let mut still = Vec::new();
        for (index, mut job) in running.drain(..) {
            match job.child.try_wait() {
                Ok(Some(status)) => {
                    let exit = status.to_string();
                    let outcome = match read_report(&job.report) {
                        Some(result) => SubOutcome::Report { result, exit },
                        None => SubOutcome::Died {
                            exit,
                            tail: tail_of(&job.log, 6),
                        },
                    };
                    outcomes.insert(index, outcome);
                }
                Ok(None)
                    if job.started.elapsed() > job.spec.deadline
                        || began.elapsed() > budget + Duration::from_secs(5) =>
                {
                    kill_group(job.child.id());
                    let _ = job.child.kill();
                    let _ = job.child.wait();
                    outcomes.insert(
                        index,
                        SubOutcome::TimedOut {
                            after: job.started.elapsed(),
                            tail: tail_of(&job.log, 6),
                        },
                    );
                }
                Ok(None) => still.push((index, job)),
                Err(error) => {
                    kill_group(job.child.id());
                    outcomes.insert(index, SubOutcome::CouldNotStart(format!("wait: {error}")));
                }
            }
        }
        running = still;
        if outcomes.len() < specs.len() {
            std::thread::sleep(Duration::from_millis(40));
        }
    }
    (0..specs.len())
        .filter_map(|index| outcomes.remove(&index))
        .collect()
}

/// The environment variable a nested run reads to find out which part to do; `None` in a top-level run.
pub fn part(variable: &str) -> Option<String> {
    std::env::var(variable)
        .ok()
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SubSpec {
        SubSpec {
            id: "T12".into(),
            tag: "seed-3".into(),
            env: vec![("MSE_STRESS_T12_SEED".into(), "3".into())],
            iterations: Some(5),
            seed: Some(3),
            deadline: Duration::from_secs(60),
        }
    }

    #[test]
    fn the_arguments_carry_the_run_configuration() {
        let args = build_args(
            Shape::Bare,
            &PluginMode::SystemPath,
            true,
            &spec(),
            Path::new("/tmp/r.json"),
            Path::new("/tmp/s"),
        );
        let joined = args.join(" ");
        assert!(
            joined.starts_with("--only T12 --shape bare --plugin-mode system --glib-main-loop")
        );
        assert!(joined.contains("--iterations 5"));
        assert!(joined.contains("--seed 3"));
        assert!(joined.ends_with("--report /tmp/r.json --scratch /tmp/s"));
    }

    #[test]
    fn the_glib_flag_and_options_are_left_out_when_unset() {
        let mut spec = spec();
        spec.iterations = None;
        spec.seed = None;
        let args = build_args(
            Shape::PlayBin,
            &PluginMode::ServoStyle {
                dir: None,
                names: vec![],
            },
            false,
            &spec,
            Path::new("r"),
            Path::new("s"),
        );
        let joined = args.join(" ");
        assert!(joined.contains("--shape playbin --plugin-mode servo --report"));
        assert!(!joined.contains("--glib-main-loop"));
        assert!(!joined.contains("--iterations"));
        assert!(!joined.contains("--seed"));
    }

    #[test]
    fn a_missing_report_is_not_a_result() {
        assert!(read_report(Path::new("/nonexistent/report.json")).is_none());
        assert!(tail_of(Path::new("/nonexistent/output.txt"), 3).is_empty());
    }

    #[test]
    fn describe_mentions_the_exit_and_the_output() {
        let died = SubOutcome::Died {
            exit: "signal: 11".into(),
            tail: vec!["a".into(), "b".into()],
        };
        let text = died.describe();
        assert!(text.contains("signal: 11") && text.contains("a | b"));
        assert!(
            SubOutcome::CouldNotStart("x".into())
                .describe()
                .contains("could not start")
        );
    }

    #[test]
    fn an_unset_part_variable_is_none() {
        assert_eq!(part("MSE_STRESS_THIS_IS_NEVER_SET"), None);
    }
}

/// The top of the faulting thread's stack from every macOS crash report of this executable made
/// since `since` (`~/Library/Logs/DiagnosticReports/mse-stress-*.ips`): which thread crashed and in
/// which functions. Local diagnostics only; nothing from the report but thread and symbol names is kept.
pub fn crash_summaries_since(since: std::time::SystemTime) -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let dir = PathBuf::from(home).join("Library/Logs/DiagnosticReports");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("mse-stress-") && name.ends_with(".ips")) {
            continue;
        }
        let Some(modified) = entry.metadata().ok().and_then(|m| m.modified().ok()) else {
            continue;
        };
        if modified < since {
            continue;
        }
        if let Some(summary) = std::fs::read_to_string(entry.path())
            .ok()
            .and_then(|t| summarise_ips(&t))
        {
            found.push((modified, summary));
        }
    }
    found.sort();
    found
        .into_iter()
        .map(|(_, summary)| summary)
        .take(12)
        .collect()
}

/// Pure part of [`crash_summary_since`]: the `.ips` text is a one-line JSON header and a JSON body.
pub fn summarise_ips(text: &str) -> Option<String> {
    let body = text.split_once('\n')?.1;
    let report: serde_json::Value = serde_json::from_str(body).ok()?;
    let index = usize::try_from(report.get("faultingThread")?.as_u64()?).ok()?;
    let thread = report.get("threads")?.get(index)?;
    let name = thread
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("unnamed thread");
    let frames: Vec<String> = thread
        .get("frames")?
        .as_array()?
        .iter()
        .take(6)
        .map(|frame| {
            frame
                .get("symbol")
                .and_then(|s| s.as_str())
                .unwrap_or("?")
                .to_owned()
        })
        .collect();
    let signal = report
        .get("exception")
        .and_then(|e| e.get("signal"))
        .and_then(|s| s.as_str())
        .unwrap_or("?");
    Some(format!(
        "crash report: {signal} on thread {name:?}: {}",
        frames.join(" < ")
    ))
}

#[cfg(test)]
mod crash_tests {
    use super::*;

    #[test]
    fn a_crash_report_is_summarised_by_thread_and_symbols() {
        let text = "{\"app_name\":\"mse-stress\"}\n{\"faultingThread\":1,\"exception\":{\"signal\":\"SIGSEGV\"},\"threads\":[{\"frames\":[]},{\"name\":\"sourcebuffer1:video-0\",\"frames\":[{\"symbol\":\"pthread_mutex_unlock\"},{\"symbol\":\"gst_task_func\"}]}]}";
        let summary = summarise_ips(text).unwrap();
        assert_eq!(
            summary,
            "crash report: SIGSEGV on thread \"sourcebuffer1:video-0\": pthread_mutex_unlock < gst_task_func"
        );
        assert_eq!(summarise_ips("not a report"), None);
        assert_eq!(summarise_ips("{}\n{}"), None);
    }
}
