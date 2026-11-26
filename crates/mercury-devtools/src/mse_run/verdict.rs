// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `mse-run verdict`: after the central window run, one command that says mechanically whether
//! the flip conditions F1 to F5 of section 8.D of the MED-1b brief hold, so that the lane lead can
//! flip `MSE_ON_BY_DEFAULT` (or not) without reading the whole report by hand.
//!
//! The input is what the runner already leaves behind: the journal (one JSON line per test,
//! `journal.rs`, unchanged), the expectations file (`media-source.expect.toml`) and, for the
//! players, the engine-gaps file. F6 (the gates are green on the head to flip) and F7 (the skeptic
//! review has nothing open) are not in the journal: they are the flags `--gates-green` and
//! `--review-ok`, and without them the verdict says NOT PROVEN for those two lines (not FAIL).
//!
//! | Headline | Exit | When |
//! |---|---|---|
//! | `FLIP: GO` | 0 | F1 to F5 hold and both flags are given |
//! | `FLIP: NO-GO` | 1 | a condition fails, or F6 or F7 is not proven |
//! | `FLIP: INCOMPLETE` | 3 | the journal lacks a phase or tests, or a test was interrupted by a screen lock or left a stuck browser: `--resume` |
//!
//! Incomplete wins over no-go (an incomplete run proves nothing; the failures seen so far are
//! still listed first). Usage errors and unreadable inputs exit 2.
//!
//! This file is pure logic ([`evaluate`], [`render`]) plus the small glue [`run`] the binary calls.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::PathBuf;

use super::cli::{Environment, UsageError, value};
use super::expect::{self, Expectations};
use super::journal::Journal;
use super::model::{Outcome, Phase, TestResult};
use super::phases::{self, PAGE_ORDER, PLAYER_ORDER};

/// The two pages that prove the two blockers closed (F1).
pub const BLOCKER_PAGES: [&str; 2] = ["mse-abort.html", "mse-underrun.html"];

/// F3: an entry of class `mse-core` or `dom` whose file name contains one of these words is in a
/// core area (append, remove, abort, duration, end of stream, buffered, seek, `changeType`,
/// `timestampOffset`, append windows) and blocks the flip unless it has an `issue`.
pub const CORE_AREAS: [&str; 10] = [
    "append",
    "remove",
    "abort",
    "duration",
    "endofstream",
    "buffered",
    "seek",
    "changetype",
    "timestampoffset",
    "appendwindow",
];

/// F3: the share of passing subtests of all subtests that loaded, in percent.
pub const SUBTEST_FLOOR_PERCENT: u64 = 85;

/// F5: the largest peak RSS of any test above this is a warning (1.5 GiB, in kB). The 1 GiB MSE
/// budget is a separate limit that `mse-quota.html` checks.
pub const RSS_SOFT_LIMIT_KB: u64 = 1_572_864;

/// One line of a condition's result. The order is the order of badness: the worst status of a
/// condition's parts is the condition's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Pass,
    /// An input the journal cannot give (F6, F7 without their flag).
    NotProven,
    /// The journal does not hold the evidence yet.
    Incomplete,
    Fail,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::NotProven => "NOT PROVEN",
            Status::Incomplete => "INCOMPLETE",
            Status::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub id: &'static str,
    pub title: String,
    pub status: Status,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Go,
    NoGo,
    Incomplete,
}

impl Verdict {
    pub fn headline(self) -> &'static str {
        match self {
            Verdict::Go => "FLIP: GO",
            Verdict::NoGo => "FLIP: NO-GO",
            Verdict::Incomplete => "FLIP: INCOMPLETE",
        }
    }

    pub fn exit_code(self) -> i32 {
        match self {
            Verdict::Go => 0,
            Verdict::NoGo => 1,
            Verdict::Incomplete => 3,
        }
    }
}

/// The tests the run was supposed to cover, from the repository and the fetched tools. The
/// pages and players of the brief are required whatever the plan says.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub pages: Vec<String>,
    pub players: Vec<String>,
    /// The WPT files planned (`None`: the checkout is not here, so only "no WPT result at all"
    /// can be told).
    pub wpt: Option<Vec<String>>,
}

/// Everything the verdict is made from.
#[derive(Debug)]
pub struct Inputs {
    /// The latest result of every test in the journal.
    pub results: Vec<TestResult>,
    pub unreadable_journal_lines: usize,
    pub expectations: Expectations,
    pub plan: Plan,
    /// Players (by [`name_key`]) that the expectations or the engine-gaps file class `engine-gap`.
    pub engine_gap_players: BTreeSet<String>,
    /// `--allow-engine-gap <name>`, by [`name_key`].
    pub allow_engine_gap: BTreeSet<String>,
    /// `--gates-green` (F6).
    pub gates_green: bool,
    /// `--review-ok` (F7).
    pub review_ok: bool,
}

/// F3's share of passing subtests among the subtests of the files that loaded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Share {
    pub subtests: u64,
    pub passing: u64,
}

impl Share {
    /// Whether the floor is met: exact integer arithmetic (84.9% is below 85%, always).
    pub fn meets_floor(self) -> bool {
        self.subtests > 0 && self.passing * 100 >= self.subtests * SUBTEST_FLOOR_PERCENT
    }

    /// The percentage cut (not rounded) to one decimal, so that a failing value never prints as
    /// the floor.
    pub fn percent_text(self) -> String {
        if self.subtests == 0 {
            return "0.0".to_owned();
        }
        let tenths = self.passing * 1000 / self.subtests;
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssRow {
    pub phase: Phase,
    pub id: String,
    pub kb: u64,
}

#[derive(Debug, Clone)]
pub struct Evaluation {
    pub verdict: Verdict,
    pub conditions: Vec<Condition>,
    /// Why the journal cannot decide yet, each with the command that continues it.
    pub incomplete: Vec<String>,
    pub warnings: Vec<String>,
    pub share: Share,
    /// The peak RSS of every test that has one, largest first.
    pub rss: Vec<RssRow>,
    /// The tests with no peak RSS recorded (listed in the report, never a failure).
    pub unrecorded: Vec<(Phase, String)>,
    pub tests_without_rss: usize,
    pub results: usize,
    pub unreadable_journal_lines: usize,
}

impl Evaluation {
    pub fn condition(&self, id: &str) -> &Condition {
        self.conditions
            .iter()
            .find(|condition| condition.id == id)
            .unwrap_or_else(|| panic!("no condition {id}"))
    }
}

/// Lower case, letters and digits only, without a trailing `.html`: `dash.js`, `dashjs.html`
/// and `Dashjs` are all `dashjs`.
pub fn name_key(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".html").unwrap_or(&lower);
    stem.chars().filter(char::is_ascii_alphanumeric).collect()
}

/// The command that finishes a run that was interrupted or left a stuck browser.
const RESUME_ALL: &str = "scripts/dev/mse-window-run.sh all --resume";

fn resume_hint(phase: Phase) -> String {
    format!("scripts/dev/mse-window-run.sh {} --resume", phase.name())
}

fn note(incomplete: &mut Vec<String>, reason: String) {
    if !incomplete.contains(&reason) {
        incomplete.push(reason);
    }
}

fn note_missing(incomplete: &mut Vec<String>, phase: Phase, missing: &[String]) {
    if !missing.is_empty() {
        note(
            incomplete,
            format!(
                "{}: no result for {}; run `{}`",
                phase.name(),
                missing.join(", "),
                resume_hint(phase)
            ),
        );
    }
}

fn unfinished_reason(result: &TestResult) -> String {
    match result.outcome {
        Outcome::Stuck => format!(
            "{} left a stuck browser that could not be killed: restart the browser (or the Mac), then run `{RESUME_ALL}`",
            result.id
        ),
        _ => format!(
            "{} was interrupted by a screen lock: run `{RESUME_ALL}`",
            result.id
        ),
    }
}

fn find<'a>(results: &'a [TestResult], phase: Phase, id: &str) -> Option<&'a TestResult> {
    results.iter().find(|r| r.phase == phase && r.id == id)
}

/// A page or player passes when it says PASS and no line says FAIL.
fn passed(result: &TestResult) -> bool {
    result.outcome == Outcome::Pass && !result.lines.iter().any(|line| line.starts_with("FAIL"))
}

fn describe(result: &TestResult) -> String {
    let state = if result.outcome == Outcome::Pass {
        "FAIL"
    } else {
        result.outcome.name()
    };
    match result.lines.iter().find(|line| line.starts_with("FAIL")) {
        Some(line) => format!("{}: {state} ({line})", result.id),
        None => format!("{}: {state}", result.id),
    }
}

impl Condition {
    fn new(id: &'static str, title: &str) -> Condition {
        Condition {
            id,
            title: title.to_owned(),
            status: Status::Pass,
            details: Vec::new(),
        }
    }

    fn raise(&mut self, status: Status, detail: String) {
        self.status = self.status.max(status);
        self.details.push(detail);
    }
}

/// Checks one required test and raises the condition for anything but a pass. Returns whether it
/// passed; a test with no result is added to `missing`.
fn require(
    condition: &mut Condition,
    result: Option<&TestResult>,
    id: &str,
    missing: &mut Vec<String>,
) -> bool {
    match result {
        None => {
            missing.push(id.to_owned());
            condition.raise(Status::Incomplete, format!("{id}: not run"));
            false
        }
        Some(result) if !result.outcome.is_final() => {
            condition.raise(
                Status::Incomplete,
                format!("{}: {} (not finished)", result.id, result.outcome.name()),
            );
            false
        }
        Some(result) if passed(result) => true,
        Some(result) => {
            condition.raise(Status::Fail, describe(result));
            false
        }
    }
}

/// The required tests of one phase: the defaults of the brief, the plan, and what the journal holds.
fn required_ids(
    defaults: &[&str],
    planned: &[String],
    results: &[TestResult],
    phase: Phase,
) -> Vec<String> {
    let journal = results
        .iter()
        .filter(|r| r.phase == phase)
        .map(|r| r.id.clone());
    let mut ids: Vec<String> = Vec::new();
    for id in defaults
        .iter()
        .map(|id| (*id).to_owned())
        .chain(planned.iter().cloned())
        .chain(journal)
    {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

fn flag(id: &'static str, title: &str, given: bool, hint: &str) -> Condition {
    let mut condition = Condition::new(id, title);
    if given {
        condition
            .details
            .push("given on the command line".to_owned());
    } else {
        condition.raise(Status::NotProven, format!("not given: {hint}"));
    }
    condition
}

fn gib(kb: u64) -> String {
    format!("{:.2} GiB", kb as f64 / 1_048_576.0)
}

fn mib(kb: u64) -> String {
    format!("{:.1} MiB", kb as f64 / 1024.0)
}

/// Decides F1 to F7 from the journal, the expectations and the plan. Pure.
pub fn evaluate(inputs: &Inputs) -> Evaluation {
    let results = inputs.results.as_slice();
    let plan = &inputs.plan;
    let expectations = &inputs.expectations;
    let mut incomplete = Vec::new();
    let mut warnings = Vec::new();

    let mut f1 = Condition::new(
        "F1",
        "both blocker pages pass: mse-abort.html and mse-underrun.html",
    );
    for id in BLOCKER_PAGES {
        require(
            &mut f1,
            find(results, Phase::Pages, id),
            id,
            &mut Vec::new(),
        );
    }

    let mut f2 = Condition::new(
        "F2",
        "every page of the run passes, none skipped, interrupted or stuck",
    );
    let pages = required_ids(&PAGE_ORDER, &plan.pages, results, Phase::Pages);
    let mut missing_pages = Vec::new();
    let mut passed_pages = 0;
    for id in &pages {
        if require(
            &mut f2,
            find(results, Phase::Pages, id),
            id,
            &mut missing_pages,
        ) {
            passed_pages += 1;
        }
    }
    note_missing(&mut incomplete, Phase::Pages, &missing_pages);
    f2.details
        .insert(0, format!("{passed_pages} of {} pages pass", pages.len()));

    let mut f3 = Condition::new(
        "F3",
        "WPT media-source: no unexpected result, core areas have issues, at least 85% of subtests pass",
    );
    let mut share = Share::default();
    let mut loaded_files = 0;
    let mut finished_wpt = false;
    for result in results.iter().filter(|r| r.phase == Phase::Wpt) {
        if !result.outcome.is_final() {
            f3.raise(
                Status::Incomplete,
                format!("{}: {} (not finished)", result.id, result.outcome.name()),
            );
            continue;
        }
        finished_wpt = true;
        for finding in expect::judge(expectations.entries.get(&result.id), result) {
            f3.raise(Status::Fail, finding.describe());
        }
        let harness = expect::actual_harness(result);
        if harness == "NOLOAD" || harness == "CRASH" {
            f3.details.push(format!(
                "{} did not load ({harness}): left out of the subtest share",
                result.id
            ));
            continue;
        }
        loaded_files += 1;
        share.subtests += result.subtests.len() as u64;
        share.passing += result
            .subtests
            .iter()
            .filter(|subtest| subtest.status == "PASS")
            .count() as u64;
    }
    let planned = plan.wpt.iter().flatten().filter(|id| {
        !expectations
            .entries
            .get(id.as_str())
            .is_some_and(|entry| entry.skip)
    });
    let mut missing_wpt = Vec::new();
    for id in planned {
        if find(results, Phase::Wpt, id).is_none() {
            missing_wpt.push(id.clone());
        }
    }
    if plan.wpt.is_none() && !results.iter().any(|r| r.phase == Phase::Wpt) {
        f3.raise(
            Status::Incomplete,
            "no WPT result at all: the run did not reach the media-source tests".to_owned(),
        );
        note(
            &mut incomplete,
            format!(
                "wpt: no WPT result at all; run `{}`",
                resume_hint(Phase::Wpt)
            ),
        );
    }
    for id in &missing_wpt {
        f3.raise(Status::Incomplete, format!("{id}: not run"));
    }
    note_missing(&mut incomplete, Phase::Wpt, &missing_wpt);
    f3.details.push(format!(
        "{}% of {} subtests in the {loaded_files} files that loaded pass (the floor is {SUBTEST_FLOOR_PERCENT}%)",
        share.percent_text(),
        share.subtests
    ));
    if share.subtests == 0 && finished_wpt {
        f3.raise(
            Status::Fail,
            "no subtest ran: every WPT file failed to load or reported none".to_owned(),
        );
    } else if share.subtests > 0 && !share.meets_floor() {
        f3.raise(
            Status::Fail,
            format!(
                "{}% is below the {SUBTEST_FLOOR_PERCENT}% floor",
                share.percent_text()
            ),
        );
    }
    for entry in expectations.entries.values() {
        let file = entry
            .path
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let core = matches!(entry.class.as_str(), "mse-core" | "dom")
            && CORE_AREAS.iter().any(|word| file.contains(*word));
        if core && entry.issue.is_none() {
            f3.raise(
                Status::Fail,
                format!(
                    "{} ({}) is in a core area and has no issue: add its issue field",
                    entry.path, entry.class
                ),
            );
        }
    }

    let players = required_ids(&PLAYER_ORDER, &plan.players, results, Phase::Players);
    let mut f4 = Condition::new("F4", "the dash.js, Shaka Player and hls.js pages each pass");
    let mut missing_players = Vec::new();
    for id in &players {
        let key = name_key(id);
        let result = find(results, Phase::Players, id);
        match result {
            Some(r)
                if inputs.engine_gap_players.contains(&key)
                    && !passed(r)
                    && matches!(r.outcome, Outcome::Fail | Outcome::NoLoad) =>
            {
                if inputs.allow_engine_gap.contains(&key) {
                    f4.details.push(format!(
                        "{}; engine-gap, reported separately (allowed by --allow-engine-gap {key})",
                        describe(r)
                    ));
                } else {
                    f4.raise(
                        Status::Fail,
                        format!(
                            "{}; classed engine-gap: rerun with --allow-engine-gap {key} to report it separately",
                            describe(r)
                        ),
                    );
                }
            }
            _ => {
                require(&mut f4, result, id, &mut missing_players);
            }
        }
    }
    note_missing(&mut incomplete, Phase::Players, &missing_players);

    let mut f5 = Condition::new(
        "F5",
        "no crash, hang, stuck browser or interrupted test in the whole journal",
    );
    for result in results {
        match result.outcome {
            Outcome::Crash | Outcome::Timeout | Outcome::Stuck => f5.raise(
                Status::Fail,
                format!(
                    "{} {}: {}",
                    result.phase.name(),
                    result.id,
                    result.outcome.name()
                ),
            ),
            Outcome::Interrupted => {
                f5.raise(Status::Incomplete, format!("{}: interrupted", result.id))
            }
            Outcome::Pass | Outcome::Fail | Outcome::NoLoad => {}
        }
        if !result.outcome.is_final() {
            note(&mut incomplete, unfinished_reason(result));
        }
    }
    let mut rss = Vec::new();
    let mut unrecorded = Vec::new();
    for result in results {
        match result.peak_rss_kb {
            Some(kb) => rss.push(RssRow {
                phase: result.phase,
                id: result.id.clone(),
                kb,
            }),
            None => unrecorded.push((result.phase, result.id.clone())),
        }
    }
    rss.sort_by(|a, b| b.kb.cmp(&a.kb).then_with(|| a.id.cmp(&b.id)));
    if let Some(largest) = rss.first() {
        f5.details.push(format!(
            "largest peak RSS: {} at {}",
            largest.id,
            gib(largest.kb)
        ));
        if largest.kb > RSS_SOFT_LIMIT_KB {
            warnings.push(format!(
                "{} peaks at {}, above the 1.5 GiB soft limit (a warning, not a failure)",
                largest.id,
                gib(largest.kb)
            ));
        }
    }
    if inputs.unreadable_journal_lines > 0 {
        warnings.push(format!(
            "{} unreadable journal line(s) were skipped",
            inputs.unreadable_journal_lines
        ));
    }

    let f6 = flag(
        "F6",
        "the gates are green on the head",
        inputs.gates_green,
        "run the gates on the head, then pass --gates-green",
    );
    let f7 = flag(
        "F7",
        "the skeptic review is ok",
        inputs.review_ok,
        "pass --review-ok once the skeptic review says ok",
    );

    let conditions = vec![f1, f2, f3, f4, f5, f6, f7];
    let verdict = if conditions.iter().any(|c| c.status == Status::Incomplete) {
        Verdict::Incomplete
    } else if conditions
        .iter()
        .any(|c| matches!(c.status, Status::NotProven | Status::Fail))
    {
        Verdict::NoGo
    } else {
        Verdict::Go
    };
    let tests_without_rss = unrecorded.len();
    Evaluation {
        verdict,
        conditions,
        incomplete,
        warnings,
        share,
        rss,
        unrecorded,
        tests_without_rss,
        results: results.len(),
        unreadable_journal_lines: inputs.unreadable_journal_lines,
    }
}

/// The report: the conditions with the failing ones first, then what is open, then the peak RSS,
/// and the headline last.
pub fn render(evaluation: &Evaluation, now: &str, journal: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# MSE flip verdict\n");
    let _ = writeln!(out, "**{}**\n", evaluation.verdict.headline());
    let _ = writeln!(
        out,
        "Decided at {now} from `{journal}`: {} results, {} unreadable journal lines.\n",
        evaluation.results, evaluation.unreadable_journal_lines
    );
    let mut ordered: Vec<&Condition> = evaluation.conditions.iter().collect();
    ordered.sort_by(|a, b| b.status.cmp(&a.status).then_with(|| a.id.cmp(b.id)));
    let _ = writeln!(out, "## Conditions\n");
    for condition in ordered {
        let _ = writeln!(
            out,
            "- **{}** {}: {}",
            condition.id,
            condition.status.name(),
            condition.title
        );
        for detail in &condition.details {
            let _ = writeln!(out, "  - {detail}");
        }
    }
    if !evaluation.incomplete.is_empty() {
        let _ = writeln!(out, "\n## Still open\n");
        for reason in &evaluation.incomplete {
            let _ = writeln!(out, "- {reason}");
        }
    }
    if !evaluation.warnings.is_empty() {
        let _ = writeln!(out, "\n## Warnings\n");
        for warning in &evaluation.warnings {
            let _ = writeln!(out, "- {warning}");
        }
    }
    let _ = writeln!(out, "\n## Peak RSS, largest first\n");
    for row in &evaluation.rss {
        let _ = writeln!(out, "- {} {}: {}", row.phase.name(), row.id, mib(row.kb));
    }
    for (phase, id) in &evaluation.unrecorded {
        let _ = writeln!(out, "- {} {}: no peak RSS recorded", phase.name(), id);
    }
    let _ = writeln!(out, "\n{}", evaluation.verdict.headline());
    out
}

/// A row of the "Entries" table of `engine-gaps.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineGapRow {
    pub found_by: String,
    pub blocks_mse: bool,
}

/// The "Entries" table of `engine-gaps.md`: the rows that name a player, and whether the row says
/// the gap blocks MSE. Other sections (the suspected gaps) are not entries.
pub fn parse_engine_gaps(text: &str) -> Vec<EngineGapRow> {
    let mut rows = Vec::new();
    let mut in_entries = false;
    let mut columns: Option<(usize, usize)> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            in_entries = trimmed == "## Entries";
            columns = None;
            continue;
        }
        if !in_entries || !trimmed.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if columns.is_none() {
            let lower: Vec<String> = cells.iter().map(|c| c.to_ascii_lowercase()).collect();
            let found = lower.iter().position(|c| c.starts_with("found by"));
            let blocks = lower.iter().position(|c| c.starts_with("blocks"));
            if let (Some(found), Some(blocks)) = (found, blocks) {
                columns = Some((found, blocks));
            }
            continue;
        }
        if cells
            .iter()
            .all(|c| c.chars().all(|ch| ch == '-' || ch == ':'))
        {
            continue;
        }
        let Some((found, blocks)) = columns else {
            continue;
        };
        let found_by = cells.get(found).copied().unwrap_or_default();
        if found_by.is_empty() {
            continue;
        }
        let blocks_mse = cells
            .get(blocks)
            .is_some_and(|cell| cell.to_ascii_lowercase().starts_with("yes"));
        rows.push(EngineGapRow {
            found_by: found_by.to_owned(),
            blocks_mse,
        });
    }
    rows
}

/// The players that count as an `engine-gap` failure: a `players/` entry of class `engine-gap`
/// in the expectations, or an engine-gaps row that does not block MSE. Only known players count.
pub fn engine_gap_players(
    expectations: &Expectations,
    rows: &[EngineGapRow],
    players: &[String],
) -> BTreeSet<String> {
    let known: BTreeSet<String> = players.iter().map(|p| name_key(p)).collect();
    let from_expectations = expectations
        .entries
        .values()
        .filter(|entry| entry.class == "engine-gap" && entry.path.starts_with("players/"))
        .map(|entry| name_key(entry.path.rsplit('/').next().unwrap_or_default()));
    let from_table = rows
        .iter()
        .filter(|row| !row.blocks_mse)
        .map(|row| name_key(&row.found_by));
    from_expectations
        .chain(from_table)
        .filter(|key| known.contains(key))
        .collect()
}

/// What `mse-run verdict` was asked.
#[derive(Debug, Clone)]
pub struct VerdictConfig {
    pub journal: PathBuf,
    pub out_dir: PathBuf,
    pub report: PathBuf,
    pub expect: PathBuf,
    pub tests_root: PathBuf,
    pub tools_dir: PathBuf,
    pub engine_gaps: PathBuf,
    pub gates_green: bool,
    pub review_ok: bool,
    pub allow_engine_gap: Vec<String>,
}

pub fn parse_arguments(
    arguments: &[String],
    environment: &Environment,
) -> Result<VerdictConfig, UsageError> {
    let repo = &environment.repo;
    let mut out_dir = environment.out_dir.clone();
    let mut journal: Option<PathBuf> = None;
    let mut report: Option<PathBuf> = None;
    let mut expect = repo.join("tests/wpt/media-source.expect.toml");
    let mut engine_gaps: Option<PathBuf> = None;
    let mut tools_dir = environment.tools_dir.clone();
    let mut gates_green = false;
    let mut review_ok = false;
    let mut allow_engine_gap = Vec::new();

    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        let (name, inline) = match argument.split_once('=') {
            Some((name, inline)) if name.starts_with("--") => (name, Some(inline)),
            _ => (argument, None),
        };
        match name {
            "--gates-green" if inline.is_none() => gates_green = true,
            "--review-ok" if inline.is_none() => review_ok = true,
            "--allow-engine-gap" => {
                allow_engine_gap.push(value(arguments, &mut index, name, inline)?)
            }
            "--out-dir" => out_dir = PathBuf::from(value(arguments, &mut index, name, inline)?),
            "--journal" => {
                journal = Some(PathBuf::from(value(arguments, &mut index, name, inline)?))
            }
            "--report" => report = Some(PathBuf::from(value(arguments, &mut index, name, inline)?)),
            "--expect" => expect = PathBuf::from(value(arguments, &mut index, name, inline)?),
            "--engine-gaps" => {
                engine_gaps = Some(PathBuf::from(value(arguments, &mut index, name, inline)?))
            }
            "--tools-dir" => tools_dir = PathBuf::from(value(arguments, &mut index, name, inline)?),
            other => return Err(UsageError(format!("unknown argument {other:?}"))),
        }
        index += 1;
    }
    let journal = journal.unwrap_or_else(|| out_dir.join("journal.jsonl"));
    let report = report
        .unwrap_or_else(|| out_dir.join(format!("flip-verdict-{}.md", environment.now_stamp)));
    Ok(VerdictConfig {
        journal,
        report,
        expect,
        tests_root: repo.join("tests"),
        tools_dir,
        engine_gaps: engine_gaps.unwrap_or_else(|| out_dir.join("engine-gaps.md")),
        out_dir,
        gates_green,
        review_ok,
        allow_engine_gap,
    })
}

/// Reads a file that may be absent: a missing file is `None`, any other failure is an error.
fn read_optional(path: &std::path::Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// What the binary needs: the text to print, the exit status, where the report went.
#[derive(Debug)]
pub struct Finished {
    pub text: String,
    pub exit_code: i32,
    pub report: PathBuf,
    pub write_error: Option<String>,
}

/// Reads the journal and the expectations, decides, writes the report.
pub fn run(config: &VerdictConfig, now: &str) -> Result<Finished, String> {
    let loaded = Journal::new(&config.journal)
        .load()
        .map_err(|error| format!("cannot read {}: {error}", config.journal.display()))?;
    let expect_text = fs::read_to_string(&config.expect)
        .map_err(|error| format!("cannot read {}: {error}", config.expect.display()))?;
    let expectations = expect::parse(&expect_text)
        .map_err(|errors| format!("{}: {}", config.expect.display(), errors.join("; ")))?;
    let gaps_text = read_optional(&config.engine_gaps)?.unwrap_or_default();
    let rows = parse_engine_gaps(&gaps_text);

    let pages = phases::plan_pages(&config.tests_root)
        .map_err(|error| format!("cannot list the pages: {error}"))?
        .into_iter()
        .map(|planned| planned.id)
        .collect();
    let players: Vec<String> = phases::plan_players(&config.tests_root)
        .map_err(|error| format!("cannot list the players: {error}"))?
        .into_iter()
        .map(|planned| planned.id)
        .collect();
    let wpt_root = config.tools_dir.join("wpt");
    let wpt = if wpt_root.join("media-source").is_dir() {
        let (tests, _) = phases::plan_wpt(&wpt_root)
            .map_err(|error| format!("cannot list the WPT files: {error}"))?;
        Some(tests.into_iter().map(|planned| planned.id).collect())
    } else {
        None
    };

    let engine_gap_players = engine_gap_players(&expectations, &rows, &players);
    let inputs = Inputs {
        results: loaded.results,
        unreadable_journal_lines: loaded.unreadable_lines,
        expectations,
        plan: Plan {
            pages,
            players,
            wpt,
        },
        engine_gap_players,
        allow_engine_gap: config
            .allow_engine_gap
            .iter()
            .map(|name| name_key(name))
            .collect(),
        gates_green: config.gates_green,
        review_ok: config.review_ok,
    };
    let evaluation = evaluate(&inputs);
    let text = render(&evaluation, now, &config.journal.display().to_string());
    let write_error = fs::create_dir_all(&config.out_dir)
        .and_then(|()| fs::write(&config.report, &text))
        .err()
        .map(|error| error.to_string());
    Ok(Finished {
        text,
        exit_code: evaluation.verdict.exit_code(),
        report: config.report.clone(),
        write_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mse_run::expect::Entry;
    use crate::mse_run::model::Subtest;
    use std::collections::BTreeMap;
    use std::path::Path;

    fn page(phase: Phase, id: &str, outcome: Outcome) -> TestResult {
        let mut result = TestResult::new(phase, id, outcome);
        result.lines = match outcome {
            Outcome::Pass => vec!["PASS one".into(), "PASS two".into()],
            Outcome::Fail => vec!["PASS one".into(), "FAIL two: nope".into()],
            _ => Vec::new(),
        };
        result.peak_rss_kb = Some(400_000);
        result
    }

    fn subtests(pass: usize, fail: usize) -> Vec<Subtest> {
        (0..pass)
            .map(|n| (format!("pass {n}"), "PASS"))
            .chain((0..fail).map(|n| (format!("fail {n}"), "FAIL")))
            .map(|(name, status)| Subtest {
                name,
                status: status.to_owned(),
                message: None,
            })
            .collect()
    }

    fn wpt(path: &str, pass: usize, fail: usize) -> TestResult {
        let mut result = TestResult::new(
            Phase::Wpt,
            path,
            if fail == 0 {
                Outcome::Pass
            } else {
                Outcome::Fail
            },
        );
        result.harness = Some("OK".into());
        result.subtests = subtests(pass, fail);
        result.peak_rss_kb = Some(500_000);
        result
    }

    fn entry(path: &str, class: &str, issue: Option<&str>, fails: usize) -> Entry {
        Entry {
            path: path.to_owned(),
            class: class.to_owned(),
            reason: "known".to_owned(),
            issue: issue.map(str::to_owned),
            skip: false,
            harness: "OK".to_owned(),
            subtests: (0..fails)
                .map(|n| (format!("fail {n}"), "FAIL".to_owned()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    fn expectations(entries: Vec<Entry>) -> Expectations {
        Expectations {
            entries: entries
                .into_iter()
                .map(|entry| (entry.path.clone(), entry))
                .collect(),
        }
    }

    /// A complete, clean run: the 11 pages, the 3 players, 3 WPT files.
    fn green() -> Inputs {
        let mut results: Vec<TestResult> = PAGE_ORDER
            .iter()
            .map(|id| page(Phase::Pages, id, Outcome::Pass))
            .collect();
        results.extend(
            PLAYER_ORDER
                .iter()
                .map(|id| page(Phase::Players, id, Outcome::Pass)),
        );
        results.extend([
            wpt("media-source/a.html", 10, 0),
            wpt("media-source/b.html", 5, 0),
            wpt("media-source/mediasource-append-buffer.html", 20, 0),
        ]);
        Inputs {
            results,
            unreadable_journal_lines: 0,
            expectations: Expectations::default(),
            plan: Plan {
                pages: PAGE_ORDER.iter().map(|s| (*s).to_owned()).collect(),
                players: PLAYER_ORDER.iter().map(|s| (*s).to_owned()).collect(),
                wpt: Some(vec![
                    "media-source/a.html".into(),
                    "media-source/b.html".into(),
                    "media-source/mediasource-append-buffer.html".into(),
                ]),
            },
            engine_gap_players: BTreeSet::new(),
            allow_engine_gap: BTreeSet::new(),
            gates_green: true,
            review_ok: true,
        }
    }

    fn set(inputs: &mut Inputs, phase: Phase, id: &str, outcome: Outcome) {
        let replacement = page(phase, id, outcome);
        match inputs
            .results
            .iter_mut()
            .find(|r| r.phase == phase && r.id == id)
        {
            Some(slot) => *slot = replacement,
            None => inputs.results.push(replacement),
        }
    }

    fn remove(inputs: &mut Inputs, phase: Phase, id: &str) {
        inputs.results.retain(|r| !(r.phase == phase && r.id == id));
    }

    fn statuses(evaluation: &Evaluation) -> Vec<(&'static str, Status)> {
        evaluation
            .conditions
            .iter()
            .map(|c| (c.id, c.status))
            .collect()
    }

    /// Only the named conditions differ from PASS.
    fn only(evaluation: &Evaluation, expected: &[(&'static str, Status)]) {
        let mut want: Vec<(&'static str, Status)> = ["F1", "F2", "F3", "F4", "F5", "F6", "F7"]
            .iter()
            .map(|id| (*id, Status::Pass))
            .collect();
        for (id, status) in expected {
            want.iter_mut().find(|(i, _)| i == id).unwrap().1 = *status;
        }
        let mut got = statuses(evaluation);
        got.sort();
        want.sort();
        assert_eq!(got, want, "{evaluation:#?}");
    }

    #[test]
    fn a_clean_run_with_both_flags_is_go() {
        let evaluation = evaluate(&green());
        only(&evaluation, &[]);
        assert_eq!(evaluation.verdict, Verdict::Go);
        assert_eq!(evaluation.verdict.exit_code(), 0);
        assert_eq!(evaluation.verdict.headline(), "FLIP: GO");
        assert!(evaluation.incomplete.is_empty());
    }

    #[test]
    fn without_the_flags_f6_and_f7_are_not_proven_and_the_flip_is_no_go() {
        for (gates, review, unproven) in [
            (false, true, vec!["F6"]),
            (true, false, vec!["F7"]),
            (false, false, vec!["F6", "F7"]),
        ] {
            let mut inputs = green();
            inputs.gates_green = gates;
            inputs.review_ok = review;
            let evaluation = evaluate(&inputs);
            let expected: Vec<(&'static str, Status)> =
                unproven.iter().map(|id| (*id, Status::NotProven)).collect();
            only(&evaluation, &expected);
            assert_eq!(evaluation.verdict, Verdict::NoGo);
            assert_eq!(evaluation.verdict.exit_code(), 1);
        }
    }

    #[test]
    fn f1_fails_for_each_blocker_page_alone() {
        for blocker in BLOCKER_PAGES {
            let mut inputs = green();
            set(&mut inputs, Phase::Pages, blocker, Outcome::Fail);
            let evaluation = evaluate(&inputs);
            // A failing page fails F2 as well (all 11 pages).
            only(&evaluation, &[("F1", Status::Fail), ("F2", Status::Fail)]);
            assert_eq!(evaluation.verdict, Verdict::NoGo);
            let details = evaluation.condition("F1").details.join("\n");
            assert!(details.contains(blocker), "{details}");
        }
    }

    #[test]
    fn a_page_that_says_pass_but_shows_a_fail_line_is_not_a_pass() {
        let mut inputs = green();
        let mut result = page(Phase::Pages, "mse-abort.html", Outcome::Pass);
        result.lines.push("FAIL late".into());
        inputs.results.retain(|r| r.id != "mse-abort.html");
        inputs.results.push(result);
        only(
            &evaluate(&inputs),
            &[("F1", Status::Fail), ("F2", Status::Fail)],
        );
    }

    #[test]
    fn f2_fails_for_any_other_page_and_leaves_f1_alone() {
        for outcome in [
            Outcome::Fail,
            Outcome::Timeout,
            Outcome::NoLoad,
            Outcome::Crash,
        ] {
            let mut inputs = green();
            set(&mut inputs, Phase::Pages, "mse-quota.html", outcome);
            let evaluation = evaluate(&inputs);
            let mut expected = vec![("F2", Status::Fail)];
            match outcome {
                Outcome::Timeout | Outcome::Crash => expected.push(("F5", Status::Fail)),
                _ => {}
            }
            only(&evaluation, &expected);
        }
    }

    #[test]
    fn a_page_left_out_of_a_started_phase_is_incomplete_and_never_a_pass() {
        let mut inputs = green();
        remove(&mut inputs, Phase::Pages, "mse-tracks.html");
        let evaluation = evaluate(&inputs);
        only(&evaluation, &[("F2", Status::Incomplete)]);
        assert_eq!(evaluation.verdict, Verdict::Incomplete);
        assert_eq!(evaluation.verdict.exit_code(), 3);
        let reason = evaluation.incomplete.join("\n");
        assert!(reason.contains("mse-tracks.html"), "{reason}");
        assert!(reason.contains("--resume"), "{reason}");
    }

    #[test]
    fn a_page_the_plan_adds_is_required_too() {
        let mut inputs = green();
        inputs.plan.pages.push("mse-new.html".into());
        let evaluation = evaluate(&inputs);
        only(&evaluation, &[("F2", Status::Incomplete)]);
        assert!(evaluation.incomplete.join("\n").contains("mse-new.html"));
    }

    #[test]
    fn an_interrupted_test_makes_the_journal_incomplete_and_says_how_to_resume() {
        let mut inputs = green();
        set(
            &mut inputs,
            Phase::Pages,
            "mse-seek.html",
            Outcome::Interrupted,
        );
        let evaluation = evaluate(&inputs);
        only(
            &evaluation,
            &[("F2", Status::Incomplete), ("F5", Status::Incomplete)],
        );
        assert_eq!(evaluation.verdict, Verdict::Incomplete);
        let text = evaluation.incomplete.join("\n");
        assert!(text.contains("mse-seek.html"), "{text}");
        assert!(text.contains("screen lock"), "{text}");
        assert!(text.contains("mse-window-run.sh all --resume"), "{text}");
    }

    #[test]
    fn a_stuck_browser_fails_f5_and_the_journal_is_incomplete() {
        let mut inputs = green();
        set(&mut inputs, Phase::Players, "shaka.html", Outcome::Stuck);
        let evaluation = evaluate(&inputs);
        only(
            &evaluation,
            &[("F4", Status::Incomplete), ("F5", Status::Fail)],
        );
        assert_eq!(evaluation.verdict, Verdict::Incomplete);
        assert!(evaluation.incomplete.join("\n").contains("restart"));
    }

    #[test]
    fn a_missing_phase_is_incomplete_and_nothing_ran_is_incomplete() {
        for phase in Phase::ALL {
            let mut inputs = green();
            inputs.results.retain(|r| r.phase != phase);
            let evaluation = evaluate(&inputs);
            assert_eq!(evaluation.verdict, Verdict::Incomplete, "{phase:?}");
            let text = evaluation.incomplete.join("\n");
            assert!(
                text.contains(&format!("mse-window-run.sh {}", phase.name())),
                "{text}"
            );
        }
        let mut nothing = green();
        nothing.results.clear();
        let evaluation = evaluate(&nothing);
        assert_eq!(evaluation.verdict, Verdict::Incomplete);
        assert_eq!(evaluation.verdict.exit_code(), 3);
        for id in ["F1", "F2", "F3", "F4"] {
            assert_eq!(evaluation.condition(id).status, Status::Incomplete, "{id}");
        }
    }

    #[test]
    fn a_planned_wpt_file_with_no_result_is_incomplete() {
        let mut inputs = green();
        inputs
            .plan
            .wpt
            .as_mut()
            .unwrap()
            .push("media-source/z.html".into());
        let evaluation = evaluate(&inputs);
        only(&evaluation, &[("F3", Status::Incomplete)]);
        assert!(
            evaluation
                .incomplete
                .join("\n")
                .contains("media-source/z.html")
        );
    }

    #[test]
    fn incomplete_wins_over_a_failure_but_the_failure_is_still_listed() {
        let mut inputs = green();
        set(&mut inputs, Phase::Pages, "mse-abort.html", Outcome::Fail);
        inputs.results.retain(|r| r.phase != Phase::Wpt);
        let evaluation = evaluate(&inputs);
        assert_eq!(evaluation.verdict, Verdict::Incomplete);
        assert_eq!(evaluation.condition("F1").status, Status::Fail);
        let text = render(&evaluation, "2026-11-22T12:00:00Z", "journal.jsonl");
        assert!(text.contains("FLIP: INCOMPLETE"));
        let f1 = text.find("F1").unwrap();
        let f4 = text.find("F4").unwrap();
        assert!(f1 < f4, "failing conditions come first:\n{text}");
    }

    #[test]
    fn f3_an_unexpected_wpt_failure_fails_and_a_known_one_does_not() {
        let mut inputs = green();
        inputs.results.push(wpt("media-source/c.html", 99, 1));
        inputs
            .plan
            .wpt
            .as_mut()
            .unwrap()
            .push("media-source/c.html".into());
        only(&evaluate(&inputs), &[("F3", Status::Fail)]);

        inputs.expectations = expectations(vec![entry("media-source/c.html", "test-bug", None, 1)]);
        only(&evaluate(&inputs), &[]);
    }

    #[test]
    fn f3_an_unexpected_pass_is_an_unexpected_result() {
        let mut inputs = green();
        let mut expected_failure = entry("media-source/a.html", "test-bug", None, 0);
        expected_failure
            .subtests
            .insert("pass 0".to_owned(), "FAIL".to_owned());
        inputs.expectations = expectations(vec![expected_failure]);
        let evaluation = evaluate(&inputs);
        only(&evaluation, &[("F3", Status::Fail)]);
        assert!(
            evaluation
                .condition("F3")
                .details
                .join("\n")
                .contains("unexpected pass"),
            "{:?}",
            evaluation.condition("F3").details
        );
    }

    fn with_share(pass: usize, fail: usize) -> Evaluation {
        let mut inputs = green();
        inputs.results.retain(|r| r.phase != Phase::Wpt);
        inputs
            .results
            .push(wpt("media-source/share.html", pass, fail));
        inputs.plan.wpt = Some(vec!["media-source/share.html".into()]);
        inputs.expectations = expectations(vec![entry(
            "media-source/share.html",
            "test-bug",
            None,
            fail,
        )]);
        evaluate(&inputs)
    }

    #[test]
    fn f3_the_85_percent_floor_is_exact() {
        let below = with_share(849, 151);
        assert_eq!(
            below.share,
            Share {
                subtests: 1000,
                passing: 849
            }
        );
        only(&below, &[("F3", Status::Fail)]);
        assert!(
            below.condition("F3").details.join("\n").contains("84.9%"),
            "{:?}",
            below.condition("F3").details
        );

        let at = with_share(850, 150);
        only(&at, &[]);
        assert!(at.condition("F3").details.join("\n").contains("85.0%"));

        // 84.99% never prints as the floor.
        assert_eq!(
            Share {
                subtests: 10_000,
                passing: 8_499
            }
            .percent_text(),
            "84.9"
        );
        assert!(
            !Share {
                subtests: 10_000,
                passing: 8_499
            }
            .meets_floor()
        );
    }

    #[test]
    fn f3_a_run_with_no_subtest_does_not_pass() {
        let mut inputs = green();
        for result in inputs.results.iter_mut().filter(|r| r.phase == Phase::Wpt) {
            result.subtests.clear();
        }
        only(&evaluate(&inputs), &[("F3", Status::Fail)]);
    }

    #[test]
    fn f3_a_file_that_did_not_load_is_left_out_of_the_denominator_and_listed() {
        let mut inputs = green();
        inputs.results.retain(|r| r.phase != Phase::Wpt);
        inputs.results.push(wpt("media-source/loads.html", 90, 10));
        let mut broken = wpt("media-source/broken.html", 0, 10);
        broken.outcome = Outcome::NoLoad;
        broken.harness = None;
        inputs.results.push(broken);
        inputs.plan.wpt = Some(vec![
            "media-source/loads.html".into(),
            "media-source/broken.html".into(),
        ]);
        let mut expected_broken = entry("media-source/broken.html", "engine-gap", None, 10);
        expected_broken.harness = "NOLOAD".into();
        inputs.expectations = expectations(vec![
            entry("media-source/loads.html", "test-bug", None, 10),
            expected_broken,
        ]);
        let evaluation = evaluate(&inputs);
        assert_eq!(
            evaluation.share,
            Share {
                subtests: 100,
                passing: 90
            }
        );
        only(&evaluation, &[]);
        let details = evaluation.condition("F3").details.join("\n");
        assert!(details.contains("did not load"), "{details}");
        assert!(details.contains("media-source/broken.html"), "{details}");
    }

    #[test]
    fn f3_core_area_dom_and_mse_core_entries_need_an_issue() {
        for class in ["dom", "mse-core"] {
            let mut inputs = green();
            inputs.expectations = expectations(vec![entry(
                "media-source/mediasource-append-buffer.html",
                class,
                None,
                0,
            )]);
            // The entry expects no failing subtest, so only the core rule can fail F3.
            let evaluation = evaluate(&inputs);
            only(&evaluation, &[("F3", Status::Fail)]);
            let details = evaluation.condition("F3").details.join("\n");
            assert!(
                details.contains("mediasource-append-buffer.html"),
                "{details}"
            );
            assert!(details.contains("issue"), "{details}");

            inputs.expectations = expectations(vec![entry(
                "media-source/mediasource-append-buffer.html",
                class,
                Some("https://example.invalid/issues/1"),
                0,
            )]);
            only(&evaluate(&inputs), &[]);
        }
    }

    #[test]
    fn f3_other_classes_and_other_areas_do_not_need_an_issue() {
        let mut inputs = green();
        inputs.expectations = expectations(vec![
            entry(
                "media-source/mediasource-append-buffer.html",
                "engine-gap",
                None,
                0,
            ),
            entry(
                "media-source/mediasource-append-buffer.html.x",
                "player",
                None,
                0,
            ),
            // A `dom` entry outside the core areas.
            entry(
                "media-source/mediasource-getvideoplaybackquality.html",
                "dom",
                None,
                0,
            ),
            // The match is on the file name, not on the directory.
            entry("media-source/append-tests/idlharness.html", "dom", None, 0),
        ]);
        let evaluation = evaluate(&inputs);
        assert_ne!(
            evaluation.condition("F3").status,
            Status::Fail,
            "{:?}",
            evaluation.condition("F3")
        );
    }

    #[test]
    fn f3_every_core_word_counts() {
        for word in CORE_AREAS {
            let path = format!("media-source/mediasource-{word}-x.html");
            let mut inputs = green();
            inputs.expectations = expectations(vec![entry(&path, "dom", None, 0)]);
            assert_eq!(
                evaluate(&inputs).condition("F3").status,
                Status::Fail,
                "{word}"
            );
        }
    }

    #[test]
    fn f4_each_player_alone_blocks_the_flip() {
        for player in PLAYER_ORDER {
            let mut inputs = green();
            set(&mut inputs, Phase::Players, player, Outcome::Fail);
            let evaluation = evaluate(&inputs);
            only(&evaluation, &[("F4", Status::Fail)]);
            assert!(
                evaluation
                    .condition("F4")
                    .details
                    .join("\n")
                    .contains(player)
            );
        }
    }

    #[test]
    fn f4_an_engine_gap_player_blocks_unless_it_is_allowed_by_name() {
        let mut inputs = green();
        set(&mut inputs, Phase::Players, "hlsjs.html", Outcome::Fail);
        inputs.engine_gap_players = ["hlsjs".to_owned()].into();

        let blocked = evaluate(&inputs);
        only(&blocked, &[("F4", Status::Fail)]);
        let details = blocked.condition("F4").details.join("\n");
        assert!(details.contains("engine-gap"), "{details}");
        assert!(details.contains("--allow-engine-gap hlsjs"), "{details}");

        inputs.allow_engine_gap = [name_key("hls.js")].into();
        let allowed = evaluate(&inputs);
        only(&allowed, &[]);
        let details = allowed.condition("F4").details.join("\n");
        assert!(
            details.contains("hlsjs.html"),
            "reported separately: {details}"
        );
        assert!(details.contains("engine-gap"), "{details}");
        assert_eq!(allowed.verdict, Verdict::Go);
    }

    #[test]
    fn f4_the_allow_flag_does_not_excuse_a_failure_that_is_not_an_engine_gap() {
        let mut inputs = green();
        set(&mut inputs, Phase::Players, "shaka.html", Outcome::Fail);
        inputs.allow_engine_gap = [name_key("shaka")].into();
        only(&evaluate(&inputs), &[("F4", Status::Fail)]);

        // Allowing a different player does not help either.
        inputs.engine_gap_players = ["shaka".to_owned()].into();
        inputs.allow_engine_gap = [name_key("dash.js")].into();
        only(&evaluate(&inputs), &[("F4", Status::Fail)]);
    }

    #[test]
    fn f4_a_crashing_engine_gap_player_is_never_excused() {
        let mut inputs = green();
        set(&mut inputs, Phase::Players, "dashjs.html", Outcome::Crash);
        inputs.engine_gap_players = ["dashjs".to_owned()].into();
        inputs.allow_engine_gap = ["dashjs".to_owned()].into();
        only(
            &evaluate(&inputs),
            &[("F4", Status::Fail), ("F5", Status::Fail)],
        );
    }

    #[test]
    fn f5_a_crash_anywhere_fails_even_in_a_wpt_file_and_a_page_timeout_is_a_hang() {
        let mut inputs = green();
        let mut crashed = wpt("media-source/crash.html", 0, 0);
        crashed.outcome = Outcome::Crash;
        crashed.harness = Some("CRASH".into());
        inputs.results.push(crashed);
        inputs
            .plan
            .wpt
            .as_mut()
            .unwrap()
            .push("media-source/crash.html".into());
        let mut crash_entry = entry("media-source/crash.html", "player", None, 0);
        crash_entry.harness = "CRASH".into();
        inputs.expectations = expectations(vec![crash_entry]);
        let evaluation = evaluate(&inputs);
        only(&evaluation, &[("F5", Status::Fail)]);
        assert!(
            evaluation
                .condition("F5")
                .details
                .join("\n")
                .contains("crash.html")
        );
    }

    #[test]
    fn f5_prints_every_peak_rss_and_warns_above_1_5_gib_without_failing() {
        let mut inputs = green();
        inputs
            .results
            .iter_mut()
            .find(|r| r.id == "mse-quota.html")
            .unwrap()
            .peak_rss_kb = Some(RSS_SOFT_LIMIT_KB + 1);
        inputs
            .results
            .iter_mut()
            .find(|r| r.id == "mse-seek.html")
            .unwrap()
            .peak_rss_kb = None;
        let evaluation = evaluate(&inputs);
        only(&evaluation, &[]);
        assert_eq!(evaluation.verdict, Verdict::Go);
        assert_eq!(evaluation.rss[0].id, "mse-quota.html");
        assert_eq!(evaluation.rss.len(), inputs.results.len() - 1);
        assert_eq!(evaluation.tests_without_rss, 1);
        let warnings = evaluation.warnings.join("\n");
        assert!(
            warnings.contains("mse-quota.html") && warnings.contains("1.5 GiB"),
            "{warnings}"
        );
        let text = render(&evaluation, "2026-11-22T12:00:00Z", "j.jsonl");
        for result in &inputs.results {
            assert!(text.contains(&result.id), "{} is in the report", result.id);
        }

        let mut quiet = green();
        quiet
            .results
            .iter_mut()
            .for_each(|r| r.peak_rss_kb = Some(RSS_SOFT_LIMIT_KB));
        assert!(
            evaluate(&quiet).warnings.is_empty(),
            "exactly 1.5 GiB is not above it"
        );
    }

    #[test]
    fn the_report_names_the_headline_first_and_last_and_lists_failures_before_passes() {
        let mut inputs = green();
        inputs.gates_green = false;
        set(&mut inputs, Phase::Pages, "mse-abort.html", Outcome::Fail);
        let text = render(&evaluate(&inputs), "2026-11-22T12:00:00Z", "j.jsonl");
        assert!(text.contains("**FLIP: NO-GO**"), "{text}");
        assert_eq!(text.trim_end().lines().last(), Some("FLIP: NO-GO"));
        let order: Vec<usize> = ["F1", "F2", "F6", "F3"]
            .iter()
            .map(|id| {
                text.find(&format!("- **{id}**"))
                    .unwrap_or_else(|| panic!("{id}\n{text}"))
            })
            .collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "FAIL, FAIL, NOT PROVEN, PASS: {order:?}\n{text}"
        );
    }

    #[test]
    fn the_unreadable_journal_lines_are_a_warning() {
        let mut inputs = green();
        inputs.unreadable_journal_lines = 2;
        let evaluation = evaluate(&inputs);
        assert!(evaluation.warnings.join("\n").contains("2 unreadable"));
        assert_eq!(evaluation.verdict, Verdict::Go);
    }

    #[test]
    fn names_are_compared_without_dots_case_or_extension() {
        for name in ["dash.js", "DashJS", "dashjs.html", "dashjs"] {
            assert_eq!(name_key(name), "dashjs", "{name}");
        }
        assert_eq!(name_key("Shaka Player"), "shakaplayer");
        assert_eq!(name_key("hls.js"), "hlsjs");
    }

    #[test]
    fn the_engine_gaps_table_is_read_and_a_blocking_entry_is_not_a_plain_gap() {
        let text = "# Engine gaps\n\n## Entries\n\n| # | Found by | Missing | Seen how | Blocks MSE? | For whom |\n|---|---|---|---|---|---|\n| | | | | | |\n| 1 | hls.js | `Worker` from a Blob URL | console | no | lead |\n| 2 | Shaka Player | MediaCapabilities | console | Yes, it picks no codec | lead |\n\n## Suspected, not seen\n\n| Suspected gap | Where | What |\n|---|---|---|\n| `ResizeObserver` | Shaka's UI | nothing |\n";
        let rows = parse_engine_gaps(text);
        assert_eq!(
            rows,
            [
                EngineGapRow {
                    found_by: "hls.js".into(),
                    blocks_mse: false
                },
                EngineGapRow {
                    found_by: "Shaka Player".into(),
                    blocks_mse: true
                },
            ]
        );
        let players: Vec<String> = PLAYER_ORDER.iter().map(|s| (*s).to_owned()).collect();
        let found = engine_gap_players(&Expectations::default(), &rows, &players);
        assert_eq!(found, ["hlsjs".to_owned()].into());
    }

    #[test]
    fn an_expectations_entry_of_class_engine_gap_classes_a_player() {
        let players: Vec<String> = PLAYER_ORDER.iter().map(|s| (*s).to_owned()).collect();
        let expectations = expectations(vec![
            entry("players/shaka.html", "engine-gap", None, 0),
            entry("dashjs.html", "dom", None, 0),
            entry("media-source/hlsjs.html", "engine-gap", None, 0),
        ]);
        let found = engine_gap_players(&expectations, &[], &players);
        assert_eq!(found, ["shaka".to_owned()].into());
    }

    fn environment() -> Environment {
        Environment {
            repo: PathBuf::from("/repo"),
            tools_dir: PathBuf::from("/cache/mse"),
            out_dir: PathBuf::from("/out"),
            now_stamp: "20261122T100000Z".into(),
        }
    }

    fn parse(list: &[&str]) -> Result<VerdictConfig, UsageError> {
        let arguments: Vec<String> = list.iter().map(|a| (*a).to_owned()).collect();
        parse_arguments(&arguments, &environment())
    }

    #[test]
    fn the_command_line_defaults_to_the_run_output_directory() {
        let config = parse(&[]).unwrap();
        assert_eq!(config.journal, Path::new("/out/journal.jsonl"));
        assert_eq!(
            config.report,
            Path::new("/out/flip-verdict-20261122T100000Z.md")
        );
        assert_eq!(config.engine_gaps, Path::new("/out/engine-gaps.md"));
        assert_eq!(
            config.expect,
            Path::new("/repo/tests/wpt/media-source.expect.toml")
        );
        assert_eq!(config.tests_root, Path::new("/repo/tests"));
        assert_eq!(config.tools_dir, Path::new("/cache/mse"));
        assert!(!config.gates_green && !config.review_ok);
        assert!(config.allow_engine_gap.is_empty());
    }

    #[test]
    fn the_command_line_options() {
        let config = parse(&[
            "--out-dir",
            "/o",
            "--gates-green",
            "--review-ok",
            "--allow-engine-gap",
            "hls.js",
            "--allow-engine-gap=shaka",
            "--journal=/j.jsonl",
        ])
        .unwrap();
        assert_eq!(config.out_dir, Path::new("/o"));
        assert_eq!(config.journal, Path::new("/j.jsonl"));
        assert_eq!(
            config.report,
            Path::new("/o/flip-verdict-20261122T100000Z.md")
        );
        assert!(config.gates_green && config.review_ok);
        assert_eq!(config.allow_engine_gap, ["hls.js", "shaka"]);
        for (list, needle) in [
            (&["--nope"][..], "unknown argument"),
            (&["pages"][..], "unknown argument"),
            (&["--out-dir"][..], "needs a value"),
        ] {
            assert!(parse(list).unwrap_err().0.contains(needle), "{list:?}");
        }
    }
}
