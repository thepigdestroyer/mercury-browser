// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The journal of a run: one JSON line per finished test, written and synced after each test,
//! so that a screen lock in the middle of a run costs one test and `--resume` continues.
//!
//! A line that cannot be read (the machine went down in the middle of a write) is skipped and
//! counted, never an error: the test it belonged to simply runs again.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::model::{Phase, TestResult};

pub struct Journal {
    path: PathBuf,
}

/// What a journal file holds.
#[derive(Debug, Default)]
pub struct Loaded {
    /// The latest result of each test, in the order each was first written.
    pub results: Vec<TestResult>,
    /// Lines that were not a result (truncated, or from another version).
    pub unreadable_lines: usize,
}

impl Loaded {
    /// The kept result of a test: its latest line, if that result is final.
    pub fn done(&self, phase: Phase, id: &str) -> Option<&TestResult> {
        self.results
            .iter()
            .find(|result| result.key() == (phase, id))
            .filter(|result| result.outcome.is_final())
    }
}

impl Journal {
    pub fn new(path: impl Into<PathBuf>) -> Journal {
        Journal { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one result and syncs the file.
    pub fn append(&self, result: &TestResult) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut line = result.to_json().to_string();
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        // A file that ends inside a line (a truncated last write) would glue the new line to
        // the broken one; start a fresh line first.
        if fs::read(&self.path).is_ok_and(|bytes| !bytes.is_empty() && !bytes.ends_with(b"\n")) {
            file.write_all(b"\n")?;
        }
        file.write_all(line.as_bytes())?;
        file.sync_data()
    }

    /// Reads the journal; a missing file is an empty journal.
    pub fn load(&self) -> io::Result<Loaded> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Loaded::default()),
            Err(error) => return Err(error),
        };
        Ok(parse(&String::from_utf8_lossy(&bytes)))
    }
}

/// The journal text as results: a later line for a test replaces an earlier one.
pub fn parse(text: &str) -> Loaded {
    let mut loaded = Loaded::default();
    let mut index_of: HashMap<(Phase, String), usize> = HashMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let result = serde_json::from_str(line)
            .ok()
            .and_then(|value| TestResult::from_json(&value));
        let Some(result) = result else {
            loaded.unreadable_lines += 1;
            continue;
        };
        match index_of.get(&(result.phase, result.id.clone())) {
            Some(&index) => loaded.results[index] = result,
            None => {
                index_of.insert((result.phase, result.id.clone()), loaded.results.len());
                loaded.results.push(result);
            }
        }
    }
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mse_run::model::Outcome;

    fn result(id: &str, outcome: Outcome) -> TestResult {
        TestResult::new(Phase::Pages, id, outcome)
    }

    #[test]
    fn resume_skips_finished_tests_and_reruns_interrupted_ones() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::new(dir.path().join("j.jsonl"));
        journal.append(&result("a.html", Outcome::Pass)).unwrap();
        journal.append(&result("b.html", Outcome::Fail)).unwrap();
        journal
            .append(&result("c.html", Outcome::Interrupted))
            .unwrap();
        journal.append(&result("d.html", Outcome::Stuck)).unwrap();
        journal.append(&result("e.html", Outcome::Timeout)).unwrap();
        let loaded = journal.load().unwrap();
        for done in ["a.html", "b.html", "e.html"] {
            assert!(loaded.done(Phase::Pages, done).is_some(), "{done}");
        }
        for rerun in ["c.html", "d.html", "never-ran.html"] {
            assert!(loaded.done(Phase::Pages, rerun).is_none(), "{rerun}");
        }
        assert!(loaded.done(Phase::Players, "a.html").is_none());
    }

    #[test]
    fn a_later_line_replaces_an_earlier_one() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::new(dir.path().join("j.jsonl"));
        journal
            .append(&result("a.html", Outcome::Interrupted))
            .unwrap();
        journal.append(&result("a.html", Outcome::Pass)).unwrap();
        let loaded = journal.load().unwrap();
        assert_eq!(loaded.results.len(), 1);
        assert_eq!(
            loaded.done(Phase::Pages, "a.html").unwrap().outcome,
            Outcome::Pass
        );
    }

    #[test]
    fn a_truncated_last_line_is_tolerated_and_the_next_append_is_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.jsonl");
        let journal = Journal::new(&path);
        journal.append(&result("a.html", Outcome::Pass)).unwrap();
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("{\"phase\":\"pages\",\"id\":\"b.ht");
        fs::write(&path, &text).unwrap();
        let loaded = journal.load().unwrap();
        assert_eq!(loaded.results.len(), 1);
        assert_eq!(loaded.unreadable_lines, 1);

        journal.append(&result("c.html", Outcome::Pass)).unwrap();
        let loaded = journal.load().unwrap();
        let ids: Vec<&str> = loaded.results.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["a.html", "c.html"]);
        assert_eq!(loaded.unreadable_lines, 1, "the broken line stays one line");
    }

    #[test]
    fn a_missing_journal_is_empty_and_garbage_is_counted() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::new(dir.path().join("none.jsonl"));
        let loaded = journal.load().unwrap();
        assert!(loaded.results.is_empty());
        assert_eq!(loaded.unreadable_lines, 0);
        let parsed = parse("not json\n\n{\"phase\":1}\n[]\n");
        assert!(parsed.results.is_empty());
        assert_eq!(parsed.unreadable_lines, 3);
    }
}
