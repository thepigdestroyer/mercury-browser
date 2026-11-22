// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The run itself: the preflight, a test server in this process, and for each test one browser
//! process with a watchdog, the journal line, and the comparison with the expectations.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::expect::{self, Expectations, Finding};
use super::journal::{Journal, Loaded};
use super::lock::{self, LockState};
use super::model::{Outcome, Phase, TestResult};
use super::phases::{self, Planned};
use super::reports::{self, Kind, Report};
use super::watchdog::{ChildGroup, Probe, Proc, PsProbe, Termination, terminate};
use crate::serve::Server;

/// Everything a run needs.
#[derive(Debug, Clone)]
pub struct Config {
    /// The directory the server serves as `/` (`tests/` of the repository).
    pub tests_root: PathBuf,
    /// Where `scripts/dev/fetch-mse-tools.sh` put the WPT checkout and the players.
    pub tools_dir: PathBuf,
    /// The browser binary, or a stub script for a dry run.
    pub browser: PathBuf,
    /// Extra arguments placed before the URL.
    pub browser_args: Vec<String>,
    pub phases: Vec<Phase>,
    /// Run only the tests whose id contains this text.
    pub only: Option<String>,
    /// The watchdog of one test (a WPT file that asks for the long timeout gets twice this).
    pub timeout: Duration,
    /// From SIGTERM to SIGKILL.
    pub grace: Duration,
    /// How long to wait after SIGKILL before calling a process stuck.
    pub kill_wait: Duration,
    pub resume: bool,
    /// Browser logs, profiles and the default journal and report go here.
    pub out_dir: PathBuf,
    pub journal: PathBuf,
    pub expect: PathBuf,
    /// `ioreg` (or a fake) for the lock check; `None` skips the check.
    pub ioreg: Option<PathBuf>,
}

/// Why a run could not start.
#[derive(Debug)]
pub enum RunError {
    /// Something the run needs is missing or wrong; each line says what and how to fix it.
    Preflight(Vec<String>),
    /// The screen is locked (or its state is unknown): nothing was started.
    Locked(LockState),
    Io(io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Preflight(problems) => {
                write!(f, "preflight failed:")?;
                for problem in problems {
                    write!(f, "\n  - {problem}")?;
                }
                Ok(())
            }
            RunError::Locked(LockState::Locked) => write!(
                f,
                "the screen is locked (CGSSessionScreenIsLocked): a window run would hang. Unlock it and run again (with --resume to continue)"
            ),
            RunError::Locked(_) => write!(
                f,
                "cannot tell whether the screen is locked (ioreg gave no console session). Run from the logged-in desktop, or pass --no-lock-check if you know better"
            ),
            RunError::Io(error) => write!(f, "{error}"),
        }
    }
}

impl From<io::Error> for RunError {
    fn from(error: io::Error) -> RunError {
        RunError::Io(error)
    }
}

/// What a run produced.
#[derive(Debug, Default)]
pub struct RunData {
    pub started: String,
    pub finished: String,
    /// The result of every planned test that has one, in plan order (this run's, or the
    /// journal's with `--resume`).
    pub results: Vec<TestResult>,
    /// Tests that have no result: the run stopped before them.
    pub not_run: Vec<Planned>,
    /// WPT files an expectation says not to run: (path, class, reason).
    pub skipped: Vec<(String, String, String)>,
    pub reftests: usize,
    /// The run stopped because the screen locked: which test, and when.
    pub interrupted: Option<String>,
    /// The run stopped because a browser could not be killed.
    pub stuck: Option<String>,
    /// Results taken from the journal rather than run now.
    pub resumed: usize,
    pub unreadable_journal_lines: usize,
    pub expectations: Option<Expectations>,
    pub findings: Vec<Finding>,
    /// Entries of the expectations file that name no test of this checkout.
    pub stale_entries: Vec<String>,
    pub plan_total: usize,
    pub browser: PathBuf,
    pub journal: PathBuf,
    /// The content of `VERSIONS` in the tools directory (what `fetch-mse-tools.sh` fetched).
    pub tools_versions: String,
}

fn now_utc() -> String {
    super::summary::utc_now()
}

/// The preflight: what must exist before the first browser starts.
fn preflight(config: &Config, need_browser: bool) -> Result<Option<Expectations>, RunError> {
    let mut problems = Vec::new();
    if need_browser && !config.browser.is_file() {
        problems.push(format!(
            "the browser {} does not exist (build it with make build-release, or pass --browser)",
            config.browser.display()
        ));
    }
    if !config.tests_root.join("pages").is_dir() {
        problems.push(format!(
            "{} has no pages/ directory: it is not the repository's tests/",
            config.tests_root.display()
        ));
    }
    let tools_hint = "run scripts/dev/fetch-mse-tools.sh";
    let wpt_wanted = config.phases.contains(&Phase::Wpt);
    if config.phases.contains(&Phase::Players) {
        let players = config.tools_dir.join("players");
        for file in ["dash.all.min.js", "shaka-player.compiled.js", "hls.min.js"] {
            if !players.join(file).is_file() {
                problems.push(format!(
                    "{} is missing: {tools_hint}",
                    players.join(file).display()
                ));
            }
        }
    }
    let mut expectations = None;
    if wpt_wanted {
        let wpt_root = config.tools_dir.join("wpt");
        if !wpt_root.join("media-source").is_dir()
            || !wpt_root.join("resources/testharness.js").is_file()
        {
            problems.push(format!(
                "{} is missing or incomplete: {tools_hint}",
                wpt_root.display()
            ));
        }
        match fs::read_to_string(&config.expect) {
            Ok(text) => match expect::parse(&text) {
                Ok(parsed) => expectations = Some(parsed),
                Err(errors) => {
                    for error in errors {
                        problems.push(format!("{}: {error}", config.expect.display()));
                    }
                }
            },
            Err(error) => {
                problems.push(format!("cannot read {}: {error}", config.expect.display()))
            }
        }
    }
    if problems.is_empty() {
        Ok(expectations)
    } else {
        Err(RunError::Preflight(problems))
    }
}

/// A WPT file an expectation says not to run: path, class, reason.
pub type Skipped = (String, String, String);

fn plan(
    config: &Config,
    expectations: Option<&Expectations>,
) -> io::Result<(Vec<Planned>, Vec<Skipped>, usize)> {
    let mut planned = Vec::new();
    let mut skipped = Vec::new();
    let mut reftests = 0;
    for phase in Phase::ALL {
        if !config.phases.contains(&phase) {
            continue;
        }
        match phase {
            Phase::Pages => planned.extend(phases::plan_pages(&config.tests_root)?),
            Phase::Players => planned.extend(phases::plan_players(&config.tests_root)?),
            Phase::Wpt => {
                let (tests, left_out) = phases::plan_wpt(&config.tools_dir.join("wpt"))?;
                reftests = left_out;
                for test in tests {
                    match expectations.and_then(|e| e.entries.get(&test.id)) {
                        Some(entry) if entry.skip => {
                            skipped.push((
                                test.id.clone(),
                                entry.class.clone(),
                                entry.reason.clone(),
                            ));
                        }
                        _ => planned.push(test),
                    }
                }
            }
        }
    }
    if let Some(only) = &config.only {
        planned.retain(|test| test.id.contains(only.as_str()));
    }
    Ok((planned, skipped, reftests))
}

/// The tests of a run, without starting anything (`--list`).
pub fn list(config: &Config) -> Result<Vec<Planned>, RunError> {
    // Listing needs the tests, not a browser.
    let expectations = preflight(config, false)?;
    Ok(plan(config, expectations.as_ref())?.0)
}

fn nonce() -> String {
    let mut bytes = [0u8; 4];
    if getrandom::fill(&mut bytes).is_err() {
        return format!("{:08x}", std::process::id());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn safe_name(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

struct Session {
    port: u16,
    reports: Receiver<Report>,
}

fn url_for(session: &Session, test: &Planned, token: &str) -> String {
    format!(
        "http://127.0.0.1:{}{}?{}={token}",
        session.port,
        test.url_path,
        crate::serve::TOKEN_PARAMETER
    )
}

/// Runs one test: starts the browser, waits for the report or the watchdog, ends the browser.
fn run_one(
    config: &Config,
    session: &Session,
    probe: &dyn Probe,
    index: usize,
    test: &Planned,
) -> (TestResult, Termination) {
    let token = format!("{}-{index:04}-{}", test.phase.name(), nonce());
    let url = url_for(session, test, &token);
    while session.reports.try_recv().is_ok() {}
    let limit = config.timeout * test.timeout_factor;

    let profile = config.out_dir.join("profiles").join(safe_name(&token));
    let log_dir = config.out_dir.join("browser-logs");
    let _ = fs::create_dir_all(&profile);
    let _ = fs::create_dir_all(&log_dir);
    let log_path = log_dir.join(format!("{}.log", safe_name(&test.id)));
    let mut command = Command::new(&config.browser);
    command
        .args([
            "--background",
            "--exit-after",
            &(limit + config.grace + Duration::from_secs(10))
                .as_millis()
                .to_string(),
            "--exit-by",
            "quit",
            "--profile-dir",
        ])
        .arg(&profile)
        .args(&config.browser_args)
        .arg(&url)
        .env("MERCURY_DEV_MSE", "1")
        .stdin(Stdio::null());
    for variable in [
        "http_proxy",
        "https_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
    ] {
        command.env_remove(variable);
    }
    match fs::File::create(&log_path).and_then(|file| Ok((file.try_clone()?, file))) {
        Ok((out, err)) => {
            command.stdout(out).stderr(err);
        }
        Err(_) => {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }

    let started = Instant::now();
    let mut result = TestResult::new(test.phase, test.id.clone(), Outcome::Crash);
    let mut child = match ChildGroup::spawn(&mut command) {
        Ok(child) => child,
        Err(error) => {
            result.notes.push(format!(
                "cannot start {}: {error}",
                config.browser.display()
            ));
            let _ = fs::remove_dir_all(&profile);
            return (result, Termination::Gone);
        }
    };

    let mut saw_hello = false;
    let mut last: Option<Report> = None;
    let mut final_report: Option<Report> = None;
    let mut peak_rss: Option<u64> = None;
    let mut next_sample = Instant::now();
    let mut browser_ended = false;
    let mut ended_at: Option<Instant> = None;
    loop {
        match session.reports.recv_timeout(Duration::from_millis(50)) {
            Ok(report) if report.token == token => match report.kind {
                Kind::Hello => saw_hello = true,
                Kind::Progress => {
                    saw_hello = true;
                    last = Some(report);
                }
                Kind::Page | Kind::Wpt => {
                    saw_hello = true;
                    final_report = Some(report);
                    break;
                }
            },
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if Instant::now() >= next_sample {
            next_sample = Instant::now() + Duration::from_millis(500);
            if let Some(kb) = probe.rss_kb(child.id()) {
                peak_rss = Some(peak_rss.map_or(kb, |peak| peak.max(kb)));
            }
        }
        if !browser_ended && child.exited() {
            browser_ended = true;
            ended_at = Some(Instant::now());
        }
        // A report may still be on its way when the process has gone.
        if ended_at.is_some_and(|at| at.elapsed() > Duration::from_secs(1)) {
            break;
        }
        if started.elapsed() >= limit {
            break;
        }
    }
    let elapsed = started.elapsed();
    let ended_before_report = browser_ended && final_report.is_none();
    let exit_note = if browser_ended {
        child.exit_description()
    } else {
        None
    };
    let termination = terminate(
        &mut child,
        probe,
        config.grace,
        config.kill_wait,
        Duration::from_millis(20),
    );
    let exit_note = exit_note.or_else(|| child.exit_description());
    let _ = fs::remove_dir_all(&profile);

    result = match final_report {
        Some(report) => result_from_report(test, &report),
        None => {
            let partial = last.as_ref();
            let outcome = if ended_before_report {
                Outcome::Crash
            } else if saw_hello {
                Outcome::Timeout
            } else {
                Outcome::NoLoad
            };
            let mut result = TestResult::new(test.phase, test.id.clone(), outcome);
            if let Some(partial) = partial {
                result.lines = partial.lines.clone();
                result
                    .notes
                    .extend(partial.errors.iter().map(|e| format!("script error: {e}")));
                result.subtests = partial.tests.clone();
            }
            match outcome {
                Outcome::Crash => result.notes.push(format!(
                    "the browser ended before the page reported ({})",
                    exit_note.as_deref().unwrap_or("no exit status")
                )),
                Outcome::Timeout => result.notes.push(format!(
                    "no completion within {} s (the page started)",
                    limit.as_secs()
                )),
                _ => result.notes.push(format!(
                    "the page never reported within {} s: it did not load, or its scripts did not run (see {})",
                    limit.as_secs(),
                    log_path.display()
                )),
            }
            result
        }
    };
    result.duration_ms = elapsed.as_millis() as u64;
    result.peak_rss_kb = peak_rss;
    (result, termination)
}

/// The result a final report stands for.
pub fn result_from_report(test: &Planned, report: &Report) -> TestResult {
    let mut result = TestResult::new(test.phase, test.id.clone(), Outcome::Fail);
    match report.kind {
        Kind::Wpt => {
            let (harness, message) = report
                .harness
                .clone()
                .unwrap_or_else(|| ("UNKNOWN".to_owned(), None));
            let clean = harness == "OK" && report.tests.iter().all(|t| t.status == "PASS");
            result.outcome = if clean { Outcome::Pass } else { Outcome::Fail };
            if let Some(message) = message {
                result.notes.push(format!("harness {harness}: {message}"));
            }
            result.harness = Some(harness);
            result.subtests = report.tests.clone();
        }
        _ => {
            let verdict = reports::judge_page(&report.title, &report.lines, &report.errors);
            result.outcome = verdict.outcome;
            result.lines = report.lines.clone();
            result.notes = verdict.notes;
        }
    }
    result
}

fn lock_state(config: &Config) -> LockState {
    match &config.ioreg {
        Some(ioreg) => lock::check(ioreg.as_os_str()),
        None => LockState::Unlocked,
    }
}

fn rotate_old_journal(path: &Path) {
    if !path.is_file() {
        return;
    }
    let stamp = super::summary::utc_now().replace([':', '-'], "");
    let mut old = path.as_os_str().to_owned();
    old.push(format!(".{stamp}.old"));
    let _ = fs::rename(path, old);
}

/// Runs the plan. `log` gets one line per test and per event, as it happens.
pub fn run(config: &Config, log: &mut dyn FnMut(&str)) -> Result<RunData, RunError> {
    let expectations = preflight(config, true)?;
    let state = lock_state(config);
    if state != LockState::Unlocked {
        return Err(RunError::Locked(state));
    }
    let (planned, skipped, reftests) = plan(config, expectations.as_ref())?;
    let journal = Journal::new(&config.journal);
    if !config.resume {
        rotate_old_journal(&config.journal);
    }
    let loaded: Loaded = if config.resume {
        journal.load()?
    } else {
        Loaded::default()
    };

    let (sender, receiver) = mpsc::channel::<Report>();
    let sender = Mutex::new(sender);
    let sink: crate::serve::ReportSink = std::sync::Arc::new(move |body: &[u8]| {
        if let Ok(report) = reports::parse(body)
            && let Ok(sender) = sender.lock()
        {
            let _ = sender.send(report);
        }
    });
    let server = Server::bind(&config.tests_root, 0)?
        .with_mse_tools_dir(config.tools_dir.clone())
        .with_report_sink(sink);
    let session = Session {
        port: server.spawn(),
        reports: receiver,
    };
    let probe = PsProbe;

    let mut data = RunData {
        started: now_utc(),
        reftests,
        skipped,
        plan_total: planned.len(),
        browser: config.browser.clone(),
        journal: config.journal.clone(),
        tools_versions: fs::read_to_string(config.tools_dir.join("VERSIONS")).unwrap_or_default(),
        unreadable_journal_lines: loaded.unreadable_lines,
        ..RunData::default()
    };
    log(&format!(
        "{} tests planned; server on 127.0.0.1:{}; journal {}",
        planned.len(),
        session.port,
        config.journal.display()
    ));

    let mut results: Vec<Option<TestResult>> = vec![None; planned.len()];
    let mut stop = false;
    for (index, test) in planned.iter().enumerate() {
        if let Some(done) = loaded.done(test.phase, &test.id) {
            results[index] = Some(done.clone());
            data.resumed += 1;
            log(&format!(
                "{} {} {}: from the journal",
                test.phase.name(),
                test.id,
                done.outcome.name()
            ));
            continue;
        }
        if stop {
            continue;
        }
        if lock_state(config) != LockState::Unlocked {
            data.interrupted = Some(format!(
                "the screen was locked before {} {} ({})",
                test.phase.name(),
                test.id,
                now_utc()
            ));
            log(data.interrupted.as_deref().unwrap_or_default());
            stop = true;
            continue;
        }
        log(&format!("{} {} ...", test.phase.name(), test.id));
        let (mut result, termination) = run_one(config, &session, &probe, index, test);
        if let Termination::Stuck { pid, state } = termination {
            result.outcome = Outcome::Stuck;
            result.notes.push(format!(
                "the browser (pid {pid}) could not be ended: state {}",
                state.map_or("unknown".to_owned(), |s| s.to_string())
            ));
            data.stuck = Some(format!(
                "{} {}: browser pid {pid} in state {state:?} cannot be killed; stopped here. Only a restart of the Mac clears a process in state U",
                test.phase.name(),
                test.id
            ));
            stop = true;
        } else if matches!(
            result.outcome,
            Outcome::Timeout | Outcome::NoLoad | Outcome::Crash
        ) && lock_state(config) != LockState::Unlocked
        {
            result.outcome = Outcome::Interrupted;
            result.notes.push("the screen locked during this test; it is not a failure and runs again with --resume".to_owned());
            data.interrupted = Some(format!(
                "the screen locked during {} {} ({})",
                test.phase.name(),
                test.id,
                now_utc()
            ));
            stop = true;
        }
        journal.append(&result)?;
        log(&format!(
            "{} {} {} ({} ms{})",
            test.phase.name(),
            test.id,
            result.outcome.name(),
            result.duration_ms,
            result
                .peak_rss_kb
                .map_or(String::new(), |kb| format!(", peak {} MB", kb / 1024))
        ));
        if result.outcome != Outcome::Interrupted && result.outcome != Outcome::Stuck {
            results[index] = Some(result);
        }
    }

    for (test, result) in planned.iter().zip(results) {
        match result {
            Some(result) => data.results.push(result),
            None => data.not_run.push(test.clone()),
        }
    }
    if let Some(expectations) = &expectations {
        for result in data.results.iter().filter(|r| r.phase == Phase::Wpt) {
            data.findings
                .extend(expect::judge(expectations.entries.get(&result.id), result));
        }
        if config.phases.contains(&Phase::Wpt) && config.only.is_none() {
            let known: std::collections::HashSet<&str> = planned
                .iter()
                .map(|t| t.id.as_str())
                .chain(data.skipped.iter().map(|(path, _, _)| path.as_str()))
                .collect();
            data.stale_entries = expectations
                .entries
                .keys()
                .filter(|path| !known.contains(path.as_str()))
                .cloned()
                .collect();
        }
    }
    data.expectations = expectations;
    data.finished = now_utc();
    Ok(data)
}
