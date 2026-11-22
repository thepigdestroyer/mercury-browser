// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What a page or a WPT file posts to `/__report` (see the two scripts under
//! `crates/mercury-devtools/resources/`), and how a finished page is judged.

use serde_json::Value;

use super::model::{Outcome, Subtest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The page started.
    Hello,
    /// Part of the result, sent while the test runs.
    Progress,
    /// A page or player page: finished.
    Page,
    /// A WPT file: finished.
    Wpt,
}

/// One message from the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub token: String,
    pub kind: Kind,
    pub title: String,
    /// Errors the page's scripts raised.
    pub errors: Vec<String>,
    /// The `.line` elements of a page, in order.
    pub lines: Vec<String>,
    /// WPT: the harness status and message.
    pub harness: Option<(String, Option<String>)>,
    pub tests: Vec<Subtest>,
}

impl Report {
    /// Whether this ends the test.
    pub fn is_final(&self) -> bool {
        matches!(self.kind, Kind::Page | Kind::Wpt)
    }
}

/// The name of a subtest status number of testharness.js.
pub fn subtest_status(code: i64) -> &'static str {
    match code {
        0 => "PASS",
        1 => "FAIL",
        2 => "TIMEOUT",
        3 => "NOTRUN",
        4 => "PRECONDITION_FAILED",
        _ => "UNKNOWN",
    }
}

/// The name of a harness status number of testharness.js.
pub fn harness_status(code: i64) -> &'static str {
    match code {
        0 => "OK",
        1 => "ERROR",
        2 => "TIMEOUT",
        3 => "PRECONDITION_FAILED",
        _ => "UNKNOWN",
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| match item {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reads a message body. A message that is not JSON, or has no token or a kind that is not
/// known, is an error (the server drops it: nothing a page can send stops the run).
pub fn parse(body: &[u8]) -> Result<Report, String> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| format!("not JSON: {error}"))?;
    let token = value["token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or("no token")?
        .to_owned();
    let kind = match value["kind"].as_str() {
        Some("hello") => Kind::Hello,
        Some("progress") => Kind::Progress,
        Some("page") => Kind::Page,
        Some("wpt") => Kind::Wpt,
        other => return Err(format!("unknown kind {other:?}")),
    };
    let tests = value["tests"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| Subtest {
                    name: item["name"].as_str().unwrap_or("").to_owned(),
                    status: subtest_status(item["status"].as_i64().unwrap_or(-1)).to_owned(),
                    message: item["message"].as_str().map(str::to_owned),
                })
                .collect()
        })
        .unwrap_or_default();
    let harness = value["harness"].as_object().map(|harness| {
        (
            harness_status(harness.get("status").and_then(Value::as_i64).unwrap_or(-1)).to_owned(),
            harness
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned),
        )
    });
    Ok(Report {
        token,
        kind,
        title: value["title"].as_str().unwrap_or("").to_owned(),
        errors: strings(&value["errors"]),
        lines: strings(&value["lines"]),
        harness,
        tests,
    })
}

/// The judgement of a page.
#[derive(Debug, PartialEq, Eq)]
pub struct PageVerdict {
    pub outcome: Outcome,
    pub passed: usize,
    pub failed: usize,
    pub notes: Vec<String>,
}

/// Judges what a page showed: it passes when its title ends `(done)`, no line starts `FAIL`
/// and at least one starts `PASS`. A title that says `<n> PASS, <m> FAIL` must agree with the lines
/// (a page whose own count and lines differ is a page bug worth seeing).
pub fn judge_page(title: &str, lines: &[String], errors: &[String]) -> PageVerdict {
    let passed = lines.iter().filter(|line| line.starts_with("PASS")).count();
    let failed = lines.iter().filter(|line| line.starts_with("FAIL")).count();
    let mut notes: Vec<String> = Vec::new();
    let mut ok = true;
    if !title.trim_end().ends_with("(done)") {
        ok = false;
        notes.push(format!("the title does not end with (done): {title:?}"));
    }
    if failed > 0 {
        ok = false;
    }
    if passed == 0 {
        ok = false;
        notes.push("no PASS line".to_owned());
    }
    if let Some((title_passed, title_failed)) = counts_in_title(title)
        && (title_passed != passed || title_failed != failed)
    {
        ok = false;
        notes.push(format!(
            "the title counts {title_passed} PASS, {title_failed} FAIL but the page shows {passed} PASS, {failed} FAIL lines"
        ));
    }
    for error in errors {
        notes.push(format!("script error: {error}"));
    }
    PageVerdict {
        outcome: if ok { Outcome::Pass } else { Outcome::Fail },
        passed,
        failed,
        notes,
    }
}

/// `3 PASS, 1 FAIL` anywhere in a title.
fn counts_in_title(title: &str) -> Option<(usize, usize)> {
    let words: Vec<&str> = title
        .split(|c: char| c.is_whitespace() || c == ',' || c == ':' || c == '(' || c == ')')
        .filter(|word| !word.is_empty())
        .collect();
    words.windows(4).find_map(|window| {
        if window[1] == "PASS" && window[3] == "FAIL" {
            Some((window[0].parse().ok()?, window[2].parse().ok()?))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(list: &[&str]) -> Vec<String> {
        list.iter().map(|line| (*line).to_owned()).collect()
    }

    #[test]
    fn a_wpt_report_is_read_with_status_names() {
        let body = br#"{"token":"t1","kind":"wpt","url":"u","title":"x",
            "harness":{"status":2,"message":"Test timed out"},
            "tests":[{"name":"a","status":0,"message":null},
                     {"name":"b","status":1,"message":"assert_equals"},
                     {"name":"c","status":3},{"name":"d","status":9}]}"#;
        let report = parse(body).unwrap();
        assert_eq!(report.kind, Kind::Wpt);
        assert!(report.is_final());
        assert_eq!(
            report.harness,
            Some(("TIMEOUT".into(), Some("Test timed out".into())))
        );
        let statuses: Vec<&str> = report.tests.iter().map(|t| t.status.as_str()).collect();
        assert_eq!(statuses, ["PASS", "FAIL", "NOTRUN", "UNKNOWN"]);
        assert_eq!(report.tests[1].message.as_deref(), Some("assert_equals"));
    }

    #[test]
    fn a_page_report_keeps_lines_and_errors() {
        let body = br#"{"token":"t2","kind":"page","title":"MSE basic: 2 PASS, 0 FAIL (done)",
            "lines":["PASS a","INFO b"],"errors":["boom (x.js:3)"]}"#;
        let report = parse(body).unwrap();
        assert_eq!(report.kind, Kind::Page);
        assert_eq!(report.lines, ["PASS a", "INFO b"]);
        assert_eq!(report.errors, ["boom (x.js:3)"]);
        assert!(
            !parse(br#"{"token":"t","kind":"hello"}"#)
                .unwrap()
                .is_final()
        );
        assert!(
            !parse(br#"{"token":"t","kind":"progress"}"#)
                .unwrap()
                .is_final()
        );
    }

    #[test]
    fn nonsense_is_refused() {
        for body in [
            &b""[..],
            b"not json",
            b"[]",
            br#"{"kind":"page"}"#,
            br#"{"token":"","kind":"page"}"#,
            br#"{"token":"t","kind":"other"}"#,
            br#"{"token":"t"}"#,
        ] {
            assert!(parse(body).is_err(), "{}", String::from_utf8_lossy(body));
        }
    }

    #[test]
    fn a_page_passes_only_when_done_without_failures() {
        let pass = judge_page(
            "MSE basic: 2 PASS, 0 FAIL (done)",
            &lines(&["PASS a", "INFO x", "PASS b"]),
            &[],
        );
        assert_eq!(pass.outcome, Outcome::Pass);
        assert_eq!((pass.passed, pass.failed), (2, 0));

        let failing = judge_page(
            "MSE basic: 1 PASS, 1 FAIL (done)",
            &lines(&["PASS a", "FAIL b: nope"]),
            &[],
        );
        assert_eq!(failing.outcome, Outcome::Fail);
        assert_eq!(failing.failed, 1);

        let unfinished = judge_page("MSE basic", &lines(&["PASS a"]), &[]);
        assert_eq!(unfinished.outcome, Outcome::Fail);
        assert!(unfinished.notes[0].contains("(done)"));

        let empty = judge_page("MSE basic: 0 PASS, 0 FAIL (done)", &[], &[]);
        assert_eq!(empty.outcome, Outcome::Fail, "a page that checked nothing");

        let miscounted = judge_page("x: 3 PASS, 0 FAIL (done)", &lines(&["PASS a"]), &[]);
        assert_eq!(miscounted.outcome, Outcome::Fail);
        assert!(
            miscounted
                .notes
                .iter()
                .any(|n| n.contains("the title counts"))
        );
    }

    #[test]
    fn script_errors_are_noted_but_do_not_fail_a_passing_page() {
        let verdict = judge_page(
            "p: 1 PASS, 0 FAIL (done)",
            &lines(&["PASS a"]),
            &["TypeError: x (p.html:9)".to_owned()],
        );
        assert_eq!(verdict.outcome, Outcome::Pass);
        assert_eq!(verdict.notes, ["script error: TypeError: x (p.html:9)"]);
    }

    #[test]
    fn counts_are_found_in_a_title() {
        assert_eq!(
            counts_in_title("MSE seek: 12 PASS, 0 FAIL (done)"),
            Some((12, 0))
        );
        assert_eq!(counts_in_title("title only"), None);
    }
}
