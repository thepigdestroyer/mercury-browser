// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The scenario runner both binaries share (item 1), so that items 3 and 4 write scenarios only.
//!
//! `main_for` parses the command line (`--list`, `--only C01,C04`, `--shape bare|playbin`,
//! `--plugin-mode system|servo`, `--glib-main-loop`, `--report FILE`, `--scratch DIR`,
//! `--iterations N`, `--seed N`), and runs **every scenario in a child process of the same
//! executable** (`<exe> --child <ID> <the same options>`), so that a crash, an abort or a hang in
//! GStreamer costs one scenario and not the report. The child initialises GStreamer
//! (`env::init`), calls the scenario function, catches panics, and prints one JSON line
//! (`ScenarioResult`) as the last line of its standard output. The parent enforces
//! `ScenarioDef::timeout` (kills the child), turns a missing result, a signal or a timeout into
//! `Status::Error` with the exit status in `notes`, builds the `Report`, prints `Report::summary`,
//! writes `--report`, and returns exit status 0 unless some scenario is `Error` (1 then; 2 for usage
//! errors).
//!
//! The parent never initialises GStreamer itself. Before the first scenario it starts one extra child
//! (`--child-init`) that only runs `env::init`: that child fills the registry cache (so the scenarios'
//! timeouts do not include a plugin scan) and tells the parent the GStreamer version for the report.
//!
//! Two options are for the harness's own tests: `--no-gstreamer` (the child skips `env::init`) and the
//! internal `--child`/`--child-init`.

use std::collections::BTreeSet;
use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::env::{self, GstInfo, InitOptions, PluginMode};
use crate::report::{Report, RunInfo, ScenarioResult, Status};
use crate::rig::{RigConfig, Shape};

/// P0 must be done and reported; P1 next; P2 if time remains (the brief's §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    P0,
    P1,
    P2,
}

/// What a scenario function gets: how the run was configured, and a scratch directory of its own.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub shape: Shape,
    pub plugin_mode: PluginMode,
    pub glib_main_loop: bool,
    /// Created before the scenario runs, removed after it, private to it.
    pub scratch: PathBuf,
    /// `--iterations` (default 1; the repeated scenarios choose their own default when it is 1).
    pub iterations: usize,
    /// `--seed` (default: derived from the time and printed in `notes` by the scenario that uses it).
    pub seed: u64,
    pub info: GstInfo,
}

impl Ctx {
    /// A `RigConfig` with this run's shape and everything else at its default.
    pub fn rig_config(&self) -> RigConfig {
        RigConfig {
            shape: self.shape,
            ..RigConfig::default()
        }
    }
}

/// A scenario: it returns its result (and sets `status`); it must not call `process::exit`.
pub type ScenarioFn = fn(&Ctx) -> ScenarioResult;

#[derive(Debug, Clone, Copy)]
pub struct ScenarioDef {
    pub id: &'static str,
    pub title: &'static str,
    pub priority: Priority,
    /// Wall-clock limit for the child process; 60 s unless the scenario is long.
    pub timeout: Duration,
    pub run: ScenarioFn,
}

// ----------------------------------------------------------------------------------------------
// Command line
// ----------------------------------------------------------------------------------------------

/// Which plugin mode the command line asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginChoice {
    System,
    Servo,
}

impl PluginChoice {
    fn flag_value(self) -> &'static str {
        match self {
            PluginChoice::System => "system",
            PluginChoice::Servo => "servo",
        }
    }

    pub fn mode(self) -> PluginMode {
        match self {
            PluginChoice::System => PluginMode::SystemPath,
            PluginChoice::Servo => PluginMode::servo_default(),
        }
    }
}

/// The parsed command line (both binaries, and the child's own arguments).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    pub list: bool,
    pub help: bool,
    pub only: Option<Vec<String>>,
    pub shape: Shape,
    pub plugin_mode: PluginChoice,
    pub glib_main_loop: bool,
    pub report: Option<PathBuf>,
    pub scratch: Option<PathBuf>,
    pub iterations: usize,
    pub seed: Option<u64>,
    /// Internal: run this one scenario as a child (`--child ID`).
    pub child: Option<String>,
    /// Internal: only run `env::init` and print what it found (`--child-init`).
    pub child_init: bool,
    /// For the harness's own tests: the child does not initialise GStreamer.
    pub no_gstreamer: bool,
}

impl Default for Cli {
    fn default() -> Self {
        Cli {
            list: false,
            help: false,
            only: None,
            shape: Shape::PlayBin,
            plugin_mode: PluginChoice::System,
            glib_main_loop: false,
            report: None,
            scratch: None,
            iterations: 1,
            seed: None,
            child: None,
            child_init: false,
            no_gstreamer: false,
        }
    }
}

pub const USAGE: &str = "\
usage: <scenario binary> [options]
  --list                     print the scenario table and exit
  --only ID[,ID...]          run only these scenarios
  --shape bare|playbin       pipeline shape (default playbin)
  --plugin-mode system|servo how plugins load (default system)
  --glib-main-loop           run a GLib main loop on a background thread
  --report FILE              write the JSON report to FILE
  --scratch DIR              scratch directory (default: a fresh one under $TMPDIR/opencode/lanes/sp-mse)
  --iterations N             repeat count for the scenarios that repeat (default 1)
  --seed N                   seed for the scenarios that use random numbers (default: from the time)
";

/// Parses the arguments after the program name. `--flag value` and `--flag=value` both work.
pub fn parse_args(args: &[String]) -> Result<Cli, String> {
    let mut cli = Cli::default();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        index += 1;
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (argument.as_str(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            if let Some(value) = inline.clone() {
                return Ok(value);
            }
            let next = args
                .get(index)
                .ok_or_else(|| format!("{name} needs a value"))?;
            index += 1;
            Ok(next.clone())
        };
        match flag {
            "--list" => cli.list = true,
            "--help" | "-h" => cli.help = true,
            "--glib-main-loop" => cli.glib_main_loop = true,
            "--child-init" => cli.child_init = true,
            "--no-gstreamer" => cli.no_gstreamer = true,
            "--only" => {
                let ids: Vec<String> = value("--only")?
                    .split(',')
                    .map(|id| id.trim().to_owned())
                    .filter(|id| !id.is_empty())
                    .collect();
                if ids.is_empty() {
                    return Err("--only needs at least one scenario ID".to_owned());
                }
                cli.only = Some(ids);
            }
            "--shape" => {
                let text = value("--shape")?;
                cli.shape = Shape::parse(&text)
                    .ok_or_else(|| format!("--shape must be bare or playbin, not {text:?}"))?;
            }
            "--plugin-mode" => {
                cli.plugin_mode = match value("--plugin-mode")?.as_str() {
                    "system" => PluginChoice::System,
                    "servo" => PluginChoice::Servo,
                    other => {
                        return Err(format!(
                            "--plugin-mode must be system or servo, not {other:?}"
                        ));
                    }
                };
            }
            "--report" => cli.report = Some(PathBuf::from(value("--report")?)),
            "--scratch" => cli.scratch = Some(PathBuf::from(value("--scratch")?)),
            "--iterations" => {
                let text = value("--iterations")?;
                cli.iterations =
                    text.parse::<usize>()
                        .ok()
                        .filter(|n| *n >= 1)
                        .ok_or_else(|| {
                            format!("--iterations must be a positive number, not {text:?}")
                        })?;
            }
            "--seed" => {
                let text = value("--seed")?;
                cli.seed = Some(
                    text.parse::<u64>()
                        .map_err(|_| format!("--seed must be a number, not {text:?}"))?,
                );
            }
            "--child" => cli.child = Some(value("--child")?),
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    Ok(cli)
}

/// The arguments the parent gives a child so that it runs with the same options.
fn child_args(cli: &Cli, scratch: &Path, seed: u64, target: ChildTarget<'_>) -> Vec<String> {
    let mut args = Vec::new();
    match target {
        ChildTarget::Scenario(id) => {
            args.push("--child".to_owned());
            args.push(id.to_owned());
        }
        ChildTarget::Init => args.push("--child-init".to_owned()),
    }
    args.push("--shape".to_owned());
    args.push(cli.shape.label().to_owned());
    args.push("--plugin-mode".to_owned());
    args.push(cli.plugin_mode.flag_value().to_owned());
    if cli.glib_main_loop {
        args.push("--glib-main-loop".to_owned());
    }
    args.push("--scratch".to_owned());
    args.push(scratch.display().to_string());
    args.push("--iterations".to_owned());
    args.push(cli.iterations.to_string());
    args.push("--seed".to_owned());
    args.push(seed.to_string());
    if cli.no_gstreamer {
        args.push("--no-gstreamer".to_owned());
    }
    args
}

#[derive(Clone, Copy)]
enum ChildTarget<'a> {
    Scenario(&'a str),
    Init,
}

/// The `--list` text: one line per scenario, `ID  P0  60s  title`.
pub fn list_text(scenarios: &[ScenarioDef]) -> String {
    let mut text = String::new();
    for def in scenarios {
        text.push_str(&format!(
            "{:<5} {:?}  {:>4}s  {}\n",
            def.id,
            def.priority,
            def.timeout.as_secs(),
            def.title
        ));
    }
    text
}

/// Checks `--only` against the table: every ID must exist and appear once in the table.
pub fn select<'a>(
    scenarios: &'a [ScenarioDef],
    only: Option<&[String]>,
) -> Result<Vec<&'a ScenarioDef>, String> {
    let known: BTreeSet<&str> = scenarios.iter().map(|def| def.id).collect();
    if let Some(only) = only {
        let unknown: Vec<&str> = only
            .iter()
            .map(String::as_str)
            .filter(|id| !known.contains(id))
            .collect();
        if !unknown.is_empty() {
            return Err(format!(
                "unknown scenario(s) {}; known: {}",
                unknown.join(", "),
                known.iter().copied().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    Ok(scenarios
        .iter()
        .filter(|def| only.is_none_or(|only| only.iter().any(|id| id == def.id)))
        .collect())
}

// ----------------------------------------------------------------------------------------------
// The child's answer, and how the parent reads it
// ----------------------------------------------------------------------------------------------

/// What `--child-init` prints (one JSON line): what `env::init` found.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitReport {
    pub gst_version: String,
    pub arch: String,
    pub plugin_mode: String,
    pub msesrc_available: bool,
    /// `file: reason`
    pub failed_plugins: Vec<String>,
}

impl InitReport {
    fn from_info(info: &GstInfo) -> Self {
        InitReport {
            gst_version: info.version.clone(),
            arch: info.arch.to_owned(),
            plugin_mode: info.plugin_mode.clone(),
            msesrc_available: info.msesrc_available,
            failed_plugins: info
                .failed_plugins
                .iter()
                .map(|(file, reason)| {
                    // The reason may contain the framework path: reports carry no paths.
                    let reason = reason.lines().next().unwrap_or_default();
                    format!("{file}: {}", scrub_paths(reason))
                })
                .collect(),
        }
    }
}

/// Replaces every absolute path in `text` by its file name, so that reports carry no directory names.
/// A path starts at a `/` that follows the start of the text, a space, `(`, `[`, `=` or a quote, and
/// runs to the next space, bracket, quote, comma, colon or semicolon (so `file.rs:12:5` keeps its line).
pub fn scrub_paths(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let boundary =
            index == 0 || matches!(chars[index - 1], ' ' | '(' | '[' | '=' | '\'' | '"' | '\t');
        let starts_path = chars[index] == '/'
            && boundary
            && chars
                .get(index + 1)
                .is_some_and(|next| next.is_alphanumeric() || matches!(next, '.' | '_' | '-'));
        if starts_path {
            let start = index;
            while index < chars.len()
                && !matches!(
                    chars[index],
                    ' ' | ')' | ']' | '\'' | '"' | ',' | ':' | ';' | '\t'
                )
            {
                index += 1;
            }
            let path: String = chars[start..index].iter().collect();
            let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
            out.push_str(name);
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

/// How a child process ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildExit {
    Code(i32),
    Signal(i32),
    TimedOut(Duration),
    SpawnFailed(String),
}

/// The name of a signal as `kill -l` knows it on macOS, for the ones a crash can be.
pub fn signal_name(signal: i32) -> &'static str {
    match signal {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        8 => "SIGFPE",
        9 => "SIGKILL",
        10 => "SIGBUS",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        14 => "SIGALRM",
        15 => "SIGTERM",
        _ => "an unknown signal",
    }
}

impl ChildExit {
    fn describe(&self) -> String {
        match self {
            ChildExit::Code(code) => format!("exit status {code}"),
            ChildExit::Signal(signal) => {
                format!("killed by signal {signal} ({})", signal_name(*signal))
            }
            ChildExit::TimedOut(limit) => {
                format!("timed out after {} s and was killed", limit.as_secs_f64())
            }
            ChildExit::SpawnFailed(error) => format!("could not be started: {error}"),
        }
    }
}

/// What a child left behind.
#[derive(Debug, Clone)]
pub struct ChildRun {
    pub exit: ChildExit,
    pub stdout: String,
    /// The last lines of its standard error (GStreamer warnings, panic messages).
    pub stderr_tail: Vec<String>,
    pub stderr_lines: usize,
    pub wall: Duration,
}

/// The last line of `stdout` that parses as `T`, scanning from the end.
fn last_json_line<T: for<'de> Deserialize<'de>>(stdout: &str) -> Option<T> {
    stdout
        .lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .find_map(|line| serde_json::from_str::<T>(line.trim()).ok())
}

/// Builds the scenario's result from what its child did (the parent's half of the protocol). A result
/// line wins over the exit status; without one the result is `Error` and `notes` says how the child ended.
pub fn interpret_child(def: &ScenarioDef, run: &ChildRun) -> ScenarioResult {
    let reported: Option<ScenarioResult> = last_json_line(&run.stdout);
    let mut result = match reported {
        Some(mut result) => {
            if result.id != def.id {
                result.note(format!(
                    "the child reported id {:?}, the table says {:?}",
                    result.id, def.id
                ));
                result.id = def.id.to_owned();
            }
            if run.exit != ChildExit::Code(0) {
                result.note(format!(
                    "the child ended with {} after it had reported its result",
                    run.exit.describe()
                ));
            }
            result
        }
        None => {
            let mut result = ScenarioResult::new(def.id, def.title);
            result.status = Status::Error;
            result.note(format!(
                "the child {} and left no result line",
                run.exit.describe()
            ));
            for line in &run.stderr_tail {
                result.note(format!("stderr: {line}"));
            }
            result.elapsed_ms = run.wall.as_millis() as u64;
            result
        }
    };
    if run.stderr_lines > 0 {
        result.metric("child_stderr_lines", run.stderr_lines as u64);
    }
    result
}

/// Starts `exe args...`, collects its standard output, writes its standard error to `stderr_file`,
/// and kills it when `timeout` passes.
pub fn run_child_process(
    exe: &Path,
    args: &[String],
    timeout: Duration,
    stderr_file: &Path,
) -> ChildRun {
    let started = Instant::now();
    let failed = |error: String| ChildRun {
        exit: ChildExit::SpawnFailed(error),
        stdout: String::new(),
        stderr_tail: Vec::new(),
        stderr_lines: 0,
        wall: started.elapsed(),
    };
    let stderr = match std::fs::File::create(stderr_file) {
        Ok(file) => Stdio::from(file),
        Err(_) => Stdio::null(),
    };
    let mut child = match Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return failed(error.to_string()),
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        return failed("no standard output pipe".to_owned());
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = sender.send(String::from_utf8_lossy(&bytes).into_owned());
    });

    let deadline = started + timeout;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break match (status.code(), status.signal()) {
                    (Some(code), _) => ChildExit::Code(code),
                    (None, Some(signal)) => ChildExit::Signal(signal),
                    (None, None) => ChildExit::Code(-1),
                };
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break ChildExit::TimedOut(timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                let _ = child.kill();
                return failed(format!("waiting for it failed: {error}"));
            }
        }
    };
    // The pipe closes when the child is gone; do not wait for ever if something else holds it open.
    let stdout = receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_default();
    let (tail, count) = tail_of(stderr_file, 10);
    ChildRun {
        exit,
        stdout,
        stderr_tail: tail,
        stderr_lines: count,
        wall: started.elapsed(),
    }
}

/// The last `keep` non-empty lines of a file (each cut to 300 characters) and the number of non-empty lines.
fn tail_of(path: &Path, keep: usize) -> (Vec<String>, usize) {
    let text = std::fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let tail = lines[lines.len().saturating_sub(keep)..]
        .iter()
        .map(|line| scrub_paths(&line.chars().take(300).collect::<String>()))
        .collect();
    (tail, lines.len())
}

// ----------------------------------------------------------------------------------------------
// The parent
// ----------------------------------------------------------------------------------------------

/// The outcome of a whole run.
#[derive(Debug, Clone)]
pub struct Suite {
    pub report: Report,
    /// The process exit status: 0, or 1 when some scenario is `Error`.
    pub exit: u8,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// A seed from the clock and the process id.
fn derived_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |elapsed| elapsed.as_nanos() as u64);
    (nanos ^ (u64::from(std::process::id()) << 32)) | 1
}

fn default_scratch(tool: &str) -> PathBuf {
    std::env::temp_dir()
        .join("opencode")
        .join("lanes")
        .join("sp-mse")
        .join(format!("{tool}-{}-{}", std::process::id(), unix_now()))
}

/// Runs the selected scenarios, each in a child process of `exe`, and assembles the report. The parent
/// part of the protocol; `main_for` is this plus the command line. `progress` gets one line per scenario
/// as it starts.
pub fn run_suite(
    tool: &str,
    scenarios: &[ScenarioDef],
    cli: &Cli,
    exe: &Path,
    mut progress: impl FnMut(&str),
) -> Result<Suite, String> {
    let selected = select(scenarios, cli.only.as_deref())?;
    let explicit_scratch = cli.scratch.is_some();
    let scratch = cli.scratch.clone().unwrap_or_else(|| default_scratch(tool));
    std::fs::create_dir_all(&scratch)
        .map_err(|error| format!("cannot create the scratch directory: {error}"))?;
    let seed = cli.seed.unwrap_or_else(derived_seed);
    let started_unix = unix_now();

    // One child that only initialises GStreamer: it fills the registry cache and names the version.
    let init_args = child_args(cli, &scratch, seed, ChildTarget::Init);
    let init_run = run_child_process(
        exe,
        &init_args,
        Duration::from_secs(300),
        &scratch.join("init.stderr.txt"),
    );
    let init: Option<InitReport> = last_json_line(&init_run.stdout);

    let mut run_info = RunInfo {
        tool: tool.to_owned(),
        gst_version: init
            .as_ref()
            .map_or_else(|| "unknown".to_owned(), |i| i.gst_version.clone()),
        arch: init
            .as_ref()
            .map_or_else(|| std::env::consts::ARCH.to_owned(), |i| i.arch.clone()),
        plugin_mode: init.as_ref().map_or_else(
            || cli.plugin_mode.mode().label().to_owned(),
            |i| i.plugin_mode.clone(),
        ),
        shape: cli.shape.label().to_owned(),
        glib_main_loop: cli.glib_main_loop,
        started_unix,
        seed,
        iterations: cli.iterations,
        msesrc_available: init.as_ref().is_some_and(|i| i.msesrc_available),
        failed_plugins: init
            .as_ref()
            .map(|i| i.failed_plugins.clone())
            .unwrap_or_default(),
    };
    if cli.no_gstreamer {
        run_info.gst_version = "none".to_owned();
    }

    let mut results = Vec::new();
    for def in selected {
        progress(&format!("running {} {}", def.id, def.title));
        let result = if init.is_none() && !cli.no_gstreamer {
            // GStreamer cannot be set up: every scenario says so instead of failing in its own way.
            let mut result = ScenarioResult::new(def.id, def.title);
            result.status = Status::Error;
            result.note(format!(
                "GStreamer set-up failed: the init child {}",
                init_run.exit.describe()
            ));
            for line in &init_run.stderr_tail {
                result.note(format!("stderr: {line}"));
            }
            result
        } else {
            let args = child_args(cli, &scratch, seed, ChildTarget::Scenario(def.id));
            let run = run_child_process(
                exe,
                &args,
                def.timeout,
                &scratch.join(format!("{}.stderr.txt", def.id)),
            );
            interpret_child(def, &run)
        };
        results.push(result);
    }

    // Clean up: a default scratch directory goes entirely; a given one keeps its registry cache.
    if explicit_scratch {
        for result in &results {
            let _ = std::fs::remove_dir_all(scratch.join(&result.id));
            let _ = std::fs::remove_file(scratch.join(format!("{}.stderr.txt", result.id)));
        }
        let _ = std::fs::remove_file(scratch.join("init.stderr.txt"));
    } else {
        let _ = std::fs::remove_dir_all(&scratch);
    }

    let exit = u8::from(results.iter().any(|result| result.status == Status::Error));
    Ok(Suite {
        report: Report {
            run: run_info,
            results,
        },
        exit,
    })
}

/// The whole `main` of a scenario binary: `fn main() -> ExitCode { harness::main_for("mse-conformance", SCENARIOS) }`.
pub fn main_for(tool: &'static str, scenarios: &'static [ScenarioDef]) -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match parse_args(&args) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{tool}: {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if cli.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if cli.child_init {
        return child_init(&cli);
    }
    if let Some(id) = &cli.child {
        return child_scenario(scenarios, &cli, id);
    }
    if cli.list {
        print!("{}", list_text(scenarios));
        return ExitCode::SUCCESS;
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            eprintln!("{tool}: cannot find my own executable: {error}");
            return ExitCode::from(2);
        }
    };
    let suite = match run_suite(tool, scenarios, &cli, &exe, |line| {
        eprintln!("{tool}: {line}")
    }) {
        Ok(suite) => suite,
        Err(message) => {
            eprintln!("{tool}: {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let run = &suite.report.run;
    println!(
        "{tool}: GStreamer {} {}, plugins {}, shape {}, glib main loop {}, seed {}",
        run.gst_version,
        run.arch,
        run.plugin_mode,
        run.shape,
        if run.glib_main_loop { "on" } else { "off" },
        run.seed
    );
    for failed in &run.failed_plugins {
        println!("{tool}: plugin did not load: {failed}");
    }
    print!("{}", suite.report.summary());
    if let Some(path) = &cli.report
        && let Err(error) = std::fs::write(path, suite.report.to_json())
    {
        eprintln!("{tool}: cannot write {}: {error}", path.display());
        return ExitCode::from(1);
    }
    ExitCode::from(suite.exit)
}

// ----------------------------------------------------------------------------------------------
// The children
// ----------------------------------------------------------------------------------------------

static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// Remembers the message and location of the last panic (and still prints it to standard error, where
/// the parent collects it).
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let text = info.to_string();
        eprintln!("{text}");
        if let Ok(mut slot) = LAST_PANIC.lock() {
            *slot = Some(text);
        }
    }));
}

/// Prints `line` as the last line of standard output and ends the process at once with status 0. The
/// process does not return through `main`: GStreamer and GLib are not torn down, because nothing after
/// the result matters and a crash in their exit handlers must not cost the result.
fn finish_with(line: &str) -> ! {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
    drop(stdout);
    // SAFETY: `_exit` ends the process immediately; no destructor or atexit handler is skipped that
    // matters (standard output is flushed above).
    unsafe { libc::_exit(0) }
}

fn init_options(cli: &Cli, scratch: PathBuf) -> InitOptions {
    InitOptions {
        prefix: None,
        plugin_mode: cli.plugin_mode.mode(),
        scratch_dir: scratch,
        glib_main_loop: cli.glib_main_loop,
    }
}

/// `--child-init`: initialise GStreamer and print the `InitReport`.
fn child_init(cli: &Cli) -> ExitCode {
    let Some(scratch) = cli.scratch.clone() else {
        eprintln!("--child-init needs --scratch");
        return ExitCode::from(2);
    };
    if cli.no_gstreamer {
        let info = no_gstreamer_info();
        finish_with(&serde_json::to_string(&InitReport::from_info(&info)).unwrap_or_default());
    }
    match env::init(&init_options(cli, scratch)) {
        Ok(info) => {
            finish_with(&serde_json::to_string(&InitReport::from_info(&info)).unwrap_or_default())
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn no_gstreamer_info() -> GstInfo {
    GstInfo {
        version: "none".to_owned(),
        arch: std::env::consts::ARCH,
        plugin_mode: "none".to_owned(),
        msesrc_available: false,
        failed_plugins: Vec::new(),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    let location = LAST_PANIC
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_default();
    let message = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(not a text payload)".to_owned());
    if location.is_empty() {
        message
    } else {
        // The hook's text is "panicked at <file>:<line>:<col>:\n<message>": keep the first line's location.
        let place = location.lines().next().unwrap_or_default();
        format!("{message} [{}]", scrub_paths(place))
    }
}

/// `--child ID`: run one scenario and print its result as the last line of standard output.
fn child_scenario(scenarios: &[ScenarioDef], cli: &Cli, id: &str) -> ExitCode {
    let Some(def) = scenarios.iter().find(|def| def.id == id) else {
        eprintln!("--child: no scenario {id:?}");
        return ExitCode::from(2);
    };
    let Some(scratch_root) = cli.scratch.clone() else {
        eprintln!("--child needs --scratch");
        return ExitCode::from(2);
    };
    install_panic_hook();

    let started = Instant::now();
    let info = if cli.no_gstreamer {
        no_gstreamer_info()
    } else {
        match env::init(&init_options(cli, scratch_root.clone())) {
            Ok(info) => info,
            Err(error) => {
                let mut result = ScenarioResult::new(def.id, def.title);
                result.status = Status::Error;
                result.note(format!("GStreamer set-up failed in the child: {error}"));
                finish_with(&serde_json::to_string(&result).unwrap_or_default());
            }
        }
    };
    let scratch = scratch_root.join(def.id);
    if let Err(error) = std::fs::create_dir_all(&scratch) {
        let mut result = ScenarioResult::new(def.id, def.title);
        result.status = Status::Error;
        result.note(format!(
            "cannot create the scenario's scratch directory: {error}"
        ));
        finish_with(&serde_json::to_string(&result).unwrap_or_default());
    }
    let ctx = Ctx {
        shape: cli.shape,
        plugin_mode: cli.plugin_mode.mode(),
        glib_main_loop: cli.glib_main_loop,
        scratch: scratch.clone(),
        iterations: cli.iterations,
        seed: cli.seed.unwrap_or_else(derived_seed),
        info,
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (def.run)(&ctx)));
    let mut result = match outcome {
        Ok(result) => result,
        Err(payload) => {
            let mut result = ScenarioResult::new(def.id, def.title);
            result.status = Status::Error;
            result.note(format!(
                "the scenario panicked: {}",
                panic_message(&*payload)
            ));
            result
        }
    };
    result.elapsed_ms = started.elapsed().as_millis() as u64;
    result
        .metrics
        .entry("shape".to_owned())
        .or_insert_with(|| cli.shape.label().into());
    result
        .metrics
        .entry("plugin_mode".to_owned())
        .or_insert_with(|| cli.plugin_mode.mode().label().into());
    result
        .metrics
        .entry("glib_main_loop".to_owned())
        .or_insert_with(|| cli.glib_main_loop.into());
    let _ = std::fs::remove_dir_all(&scratch);
    finish_with(&serde_json::to_string(&result).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn ok_scenario(_: &Ctx) -> ScenarioResult {
        ScenarioResult::new("X", "x")
    }

    fn def(id: &'static str) -> ScenarioDef {
        ScenarioDef {
            id,
            title: "a title",
            priority: Priority::P0,
            timeout: Duration::from_secs(60),
            run: ok_scenario,
        }
    }

    #[test]
    fn the_defaults_are_the_servo_shape_with_the_system_plugin_scan() {
        let cli = parse_args(&[]).unwrap();
        assert_eq!(cli, Cli::default());
        assert_eq!(cli.shape, Shape::PlayBin);
        assert_eq!(cli.plugin_mode, PluginChoice::System);
        assert_eq!(cli.iterations, 1);
        assert!(!cli.glib_main_loop && !cli.list && cli.only.is_none());
    }

    #[test]
    fn every_option_parses_in_both_spellings() {
        let cli = parse_args(&args(&[
            "--only",
            "C01, C04",
            "--shape=bare",
            "--plugin-mode",
            "servo",
            "--glib-main-loop",
            "--report",
            "r.json",
            "--scratch=/tmp/x",
            "--iterations",
            "50",
            "--seed",
            "99",
            "--list",
        ]))
        .unwrap();
        assert_eq!(cli.only, Some(vec!["C01".to_owned(), "C04".to_owned()]));
        assert_eq!(cli.shape, Shape::Bare);
        assert_eq!(cli.plugin_mode, PluginChoice::Servo);
        assert!(cli.glib_main_loop && cli.list);
        assert_eq!(cli.report, Some(PathBuf::from("r.json")));
        assert_eq!(cli.scratch, Some(PathBuf::from("/tmp/x")));
        assert_eq!((cli.iterations, cli.seed), (50, Some(99)));
    }

    #[test]
    fn bad_command_lines_are_usage_errors() {
        for bad in [
            &["--bogus"][..],
            &["--shape", "triangle"],
            &["--plugin-mode", "magic"],
            &["--iterations", "0"],
            &["--iterations", "many"],
            &["--seed", "-3"],
            &["--only"],
            &["--only", " , "],
            &["--report"],
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn the_child_gets_the_same_options_and_its_own_target() {
        let cli = Cli {
            shape: Shape::Bare,
            plugin_mode: PluginChoice::Servo,
            glib_main_loop: true,
            iterations: 7,
            ..Cli::default()
        };
        let list = child_args(&cli, Path::new("/s"), 5, ChildTarget::Scenario("C02"));
        assert_eq!(&list[..2], ["--child", "C02"]);
        let parsed = parse_args(&list).unwrap();
        assert_eq!(parsed.child.as_deref(), Some("C02"));
        assert_eq!(parsed.shape, Shape::Bare);
        assert_eq!(parsed.plugin_mode, PluginChoice::Servo);
        assert!(parsed.glib_main_loop);
        assert_eq!((parsed.iterations, parsed.seed), (7, Some(5)));
        assert_eq!(parsed.scratch, Some(PathBuf::from("/s")));
        let init = parse_args(&child_args(&cli, Path::new("/s"), 5, ChildTarget::Init)).unwrap();
        assert!(init.child_init && init.child.is_none());
    }

    #[test]
    fn the_list_is_one_line_per_scenario() {
        let table = [def("C01"), def("T02")];
        let text = list_text(&table);
        assert_eq!(text.lines().count(), 2);
        assert!(text.starts_with("C01   P0"));
        assert!(text.contains("60s  a title"));
    }

    #[test]
    fn only_selects_in_table_order_and_refuses_unknown_ids() {
        let table = [def("C01"), def("C02"), def("C03")];
        let all = select(&table, None).unwrap();
        assert_eq!(all.len(), 3);
        let some = select(&table, Some(&args(&["C03", "C01"]))).unwrap();
        assert_eq!(
            some.iter().map(|d| d.id).collect::<Vec<_>>(),
            ["C01", "C03"]
        );
        let error = select(&table, Some(&args(&["C09"]))).unwrap_err();
        assert!(
            error.contains("C09") && error.contains("C01, C02, C03"),
            "{error}"
        );
    }

    fn run_of(exit: ChildExit, stdout: &str) -> ChildRun {
        ChildRun {
            exit,
            stdout: stdout.to_owned(),
            stderr_tail: vec!["GStreamer-CRITICAL: boom".to_owned()],
            stderr_lines: 3,
            wall: Duration::from_millis(1500),
        }
    }

    fn json_of(status: Status, id: &str) -> String {
        let mut result = ScenarioResult::new(id, "t");
        result.status = status;
        serde_json::to_string(&result).unwrap()
    }

    #[test]
    fn a_result_line_wins_and_noise_before_it_is_ignored() {
        let stdout = format!("some print\n{}\n", json_of(Status::Pass, "C01"));
        let result = interpret_child(&def("C01"), &run_of(ChildExit::Code(0), &stdout));
        assert_eq!(result.status, Status::Pass);
        assert_eq!(result.metrics["child_stderr_lines"], 3);
        assert!(result.notes.is_empty());
    }

    #[test]
    fn a_crash_after_the_result_keeps_the_result_and_says_so() {
        let stdout = format!("{}\n", json_of(Status::Fail, "C01"));
        let result = interpret_child(&def("C01"), &run_of(ChildExit::Signal(11), &stdout));
        assert_eq!(result.status, Status::Fail);
        assert!(result.notes[0].contains("SIGSEGV"), "{:?}", result.notes);
    }

    #[test]
    fn no_result_line_is_an_error_naming_the_signal_or_the_timeout() {
        let abort = interpret_child(&def("C01"), &run_of(ChildExit::Signal(6), "log line\n"));
        assert_eq!(abort.status, Status::Error);
        assert!(
            abort.notes[0].contains("signal 6 (SIGABRT)"),
            "{:?}",
            abort.notes
        );
        assert!(abort.notes.iter().any(|n| n.contains("GStreamer-CRITICAL")));
        assert_eq!(abort.elapsed_ms, 1500);

        let hung = interpret_child(
            &def("C01"),
            &run_of(ChildExit::TimedOut(Duration::from_secs(2)), ""),
        );
        assert_eq!(hung.status, Status::Error);
        assert!(
            hung.notes[0].contains("timed out after 2 s"),
            "{:?}",
            hung.notes
        );

        let code = interpret_child(&def("C01"), &run_of(ChildExit::Code(2), ""));
        assert!(code.notes[0].contains("exit status 2"));
    }

    #[test]
    fn a_result_with_the_wrong_id_is_corrected_with_a_note() {
        let stdout = format!("{}\n", json_of(Status::Pass, "C99"));
        let result = interpret_child(&def("C01"), &run_of(ChildExit::Code(0), &stdout));
        assert_eq!(result.id, "C01");
        assert!(result.notes[0].contains("C99"));
    }

    #[test]
    fn signals_have_names() {
        assert_eq!(signal_name(6), "SIGABRT");
        assert_eq!(signal_name(9), "SIGKILL");
        assert_eq!(signal_name(11), "SIGSEGV");
        assert_eq!(signal_name(77), "an unknown signal");
    }

    #[test]
    fn paths_are_scrubbed_from_report_text() {
        assert_eq!(
            scrub_paths("dlopen(/Users/someone/lib/libx.dylib, 0x5): not found"),
            "dlopen(libx.dylib, 0x5): not found"
        );
        assert_eq!(
            scrub_paths("panicked at /a/b/src/x.rs:12:5"),
            "panicked at x.rs:12:5"
        );
        assert_eq!(
            scrub_paths("uri mse://rig and 1/2 and/or"),
            "uri mse://rig and 1/2 and/or"
        );
        assert_eq!(
            scrub_paths("Referenced from: '/a/b/c.dylib' (no such file)"),
            "Referenced from: 'c.dylib' (no such file)"
        );
        assert_eq!(scrub_paths("no path here"), "no path here");
    }
}
