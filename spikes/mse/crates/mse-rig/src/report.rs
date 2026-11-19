// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The report both scenario binaries print: one JSON document, the same shape on Intel and Apple
//! Silicon, so that a run on the other architecture can be compared line by line.
//! Complete (not a stub); item 1 may extend it (keep the field names).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Behaved as an MSE player needs.
    Pass,
    /// Behaved wrongly or not at all: a finding, not a harness bug.
    Fail,
    /// Not run (missing fixture, plugin or architecture), with the reason in `notes`.
    Skip,
    /// The harness itself broke (panic, setup failure): fix the harness.
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    /// Stable identifier such as `C03` (conformance) or `T07` (stress); see the brief's scenario table.
    pub id: String,
    pub title: String,
    pub status: Status,
    /// Named numbers and strings: frame counts, latencies in ms, thread names, decoder names.
    pub metrics: BTreeMap<String, serde_json::Value>,
    /// One line per observation worth reading, in plain words.
    pub notes: Vec<String>,
    pub elapsed_ms: u64,
}

impl ScenarioResult {
    pub fn new(id: &str, title: &str) -> Self {
        ScenarioResult {
            id: id.to_owned(),
            title: title.to_owned(),
            status: Status::Error,
            metrics: BTreeMap::new(),
            notes: Vec::new(),
            elapsed_ms: 0,
        }
    }

    pub fn metric(&mut self, name: &str, value: impl Into<serde_json::Value>) -> &mut Self {
        self.metrics.insert(name.to_owned(), value.into());
        self
    }

    pub fn note(&mut self, line: impl Into<String>) -> &mut Self {
        self.notes.push(line.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInfo {
    pub tool: String,
    pub gst_version: String,
    /// `x86_64` or `aarch64`.
    pub arch: String,
    pub plugin_mode: String,
    pub shape: String,
    pub glib_main_loop: bool,
    /// Seconds since the Unix epoch at the start of the run. (No host name, user name or path: reports are shared.)
    pub started_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub run: RunInfo,
    pub results: Vec<ScenarioResult>,
}

impl Report {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a report always serialises")
    }

    /// One line per scenario: `PASS  C03  seek inside the buffered range  (1234 ms)`.
    pub fn summary(&self) -> String {
        let mut text = String::new();
        for result in &self.results {
            let status = match result.status {
                Status::Pass => "PASS ",
                Status::Fail => "FAIL ",
                Status::Skip => "SKIP ",
                Status::Error => "ERROR",
            };
            text.push_str(&format!(
                "{status} {:<5} {}  ({} ms)\n",
                result.id, result.title, result.elapsed_ms
            ));
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_round_trips_and_summarises() {
        let mut result = ScenarioResult::new("C01", "is_type_supported");
        result.status = Status::Pass;
        result.metric("frames", 200).note("fine");
        let report = Report {
            run: RunInfo {
                tool: "test".into(),
                gst_version: "1.28.9".into(),
                arch: "x86_64".into(),
                plugin_mode: "system-path".into(),
                shape: "bare".into(),
                glib_main_loop: false,
                started_unix: 0,
            },
            results: vec![result],
        };
        let back: Report = serde_json::from_str(&report.to_json()).unwrap();
        assert_eq!(back.results[0].metrics["frames"], 200);
        assert!(report.summary().starts_with("PASS  C01"));
    }
}
