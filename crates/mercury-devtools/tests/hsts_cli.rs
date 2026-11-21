// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The `hsts-preload` executable: `build`, `check` and `diff` end to end, through the real binary
//! (`CARGO_BIN_EXE_hsts-preload`), plus the round trip through Chromium's file shape and the
//! download script. No test touches the network: the fixtures are small synthetic documents the
//! tests write, the one real data file is the list Servo 0.8.0 ships (read by path), and the
//! script runs against stub `curl` and `cargo` programs in a temporary tree.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use mercury_devtools::hsts::CANARIES;
use mercury_devtools::hsts::fstmap::Preload;
use mercury_devtools::hsts::manifest::{Date, Manifest, sha256_hex};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_hsts-preload");

/// What one run printed and how it ended.
struct Outcome {
    status: i32,
    out: String,
    err: String,
}

impl Outcome {
    fn last_line(&self) -> &str {
        self.out.lines().last().unwrap_or_default()
    }
}

fn run(arguments: &[&str]) -> Outcome {
    run_with_input(arguments, b"")
}

fn run_with_input(arguments: &[&str], input: &[u8]) -> Outcome {
    let mut child = Command::new(BIN)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start hsts-preload");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input)
        .expect("write standard input");
    let output = child.wait_with_output().expect("wait for hsts-preload");
    Outcome {
        status: output.status.code().expect("exit status"),
        out: String::from_utf8(output.stdout).expect("UTF-8 on standard output"),
        err: String::from_utf8(output.stderr).expect("UTF-8 on standard error"),
    }
}

/// A scratch directory with a few helpers; every path is absolute and printable.
struct Work {
    dir: TempDir,
}

impl Work {
    fn new() -> Work {
        Work {
            dir: tempfile::tempdir().expect("temporary directory"),
        }
    }

    fn path(&self, name: &str) -> String {
        self.dir
            .path()
            .join(name)
            .to_str()
            .expect("UTF-8 path")
            .to_string()
    }

    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> String {
        std::fs::write(self.dir.path().join(name), bytes).expect("write a file");
        self.path(name)
    }

    fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.dir.path().join(name)).expect("read a file")
    }

    fn text(&self, name: &str) -> String {
        String::from_utf8(self.read(name)).expect("UTF-8 file")
    }

    fn exists(&self, name: &str) -> bool {
        self.dir.path().join(name).exists()
    }

    /// The names in the directory, sorted.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.dir.path())
            .expect("read the directory")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    fn preload(&self, name: &str) -> Preload {
        Preload::open(self.read(name)).expect("a data file that opens")
    }
}

/// `count` hosts: the canaries, then `site000.example`, `site001.example`, ... all with
/// `include_subdomains`.
fn hosts(count: usize) -> Vec<(String, bool)> {
    let mut hosts: Vec<(String, bool)> = CANARIES
        .iter()
        .map(|host| ((*host).to_string(), true))
        .collect();
    hosts.extend(
        (0..count.saturating_sub(CANARIES.len()))
            .map(|number| (format!("site{number:03}.example"), true)),
    );
    hosts
}

/// A document in the shape of Chromium's file: licence comments, a pin set with a trailing comma,
/// a block comment, a comment line in front of every fifth entry, the entries, two pin-only
/// objects (one with `include_subdomains`), the first host a second time with the flag off, and a
/// trailing comma before the closing brackets. It holds `hosts.len()` distinct hosts, all of them
/// `force-https`, and `hosts.len() + 3` objects.
fn chromium_json(hosts: &[(String, bool)]) -> String {
    let mut text = String::from(
        "// Copyright 2015 The Chromium Authors\n\
         // found in the LICENSE file; a test document.\n\n\
         {\n  \"pinsets\": [\n    { \"name\": \"test\", \"static_spki_hashes\": [\"TestSPKI\"], },\n  ],\n\
         \n  /* the entries follow */\n  \"entries\": [\n",
    );
    for (number, (name, include_subdomains)) in hosts.iter().enumerate() {
        if number % 5 == 0 {
            text.push_str("    // a group of entries\n");
        }
        text.push_str(&format!(
            "    {{ \"name\": \"{name}\", \"policy\": \"public-suffix\", \"mode\": \"force-https\", \
             \"include_subdomains\": {include_subdomains} }},\n"
        ));
    }
    text.push_str(
        "    { \"name\": \"pinned.example\", \"include_subdomains\": true, \"pins\": \"test\" },\n\
         \x20   { \"name\": \"pinned-two.example\", \"pins\": \"test\" },\n",
    );
    if let Some((name, _)) = hosts.first() {
        text.push_str(&format!(
            "    {{ \"name\": \"{name}\", \"mode\": \"force-https\", \"include_subdomains\": false }},\n"
        ));
    }
    text.push_str("  ],\n}\n");
    text
}

/// Writes `input.json` from `hosts` and builds it into `list.fstmap` with `--min-entries 1`.
fn build(work: &Work, hosts: &[(String, bool)], extra: &[&str]) -> Outcome {
    let input = work.write("input.json", chromium_json(hosts));
    let output = work.path("list.fstmap");
    let mut arguments = vec![
        "build",
        "--input",
        &input,
        "--output",
        &output,
        "--min-entries",
        "1",
    ];
    arguments.extend_from_slice(extra);
    run(&arguments)
}

fn assert_done(outcome: &Outcome) {
    assert_eq!(
        outcome.status, 0,
        "stdout:\n{}\nstderr:\n{}",
        outcome.out, outcome.err
    );
}

fn assert_refused(outcome: &Outcome, status: i32) {
    assert_eq!(
        outcome.status, status,
        "stdout:\n{}\nstderr:\n{}",
        outcome.out, outcome.err
    );
}

// ---------------------------------------------------------------- build

#[test]
fn build_writes_the_data_and_a_manifest_that_describe_each_other() {
    let work = Work::new();
    let list = hosts(40);
    let outcome = build(&work, &list, &["--retrieved", "2026-03-04"]);
    assert_done(&outcome);

    let preload = work.preload("list.fstmap");
    assert_eq!(preload.len(), 40, "the duplicate is merged");
    assert_eq!(
        preload.include_subdomains_count(),
        40,
        "and the flag that is on wins"
    );
    assert_eq!(preload.get(CANARIES[0]), Some(true));
    assert_eq!(
        preload.get("pinned.example"),
        None,
        "a pin-only object is not an entry"
    );

    let input = work.read("input.json");
    let manifest = Manifest::parse(&work.text("list.manifest.json")).expect("a manifest");
    assert_eq!(manifest.entries, 40);
    assert_eq!(manifest.include_subdomains, 40);
    assert_eq!(
        manifest.output_sha256,
        sha256_hex(&work.read("list.fstmap"))
    );
    assert_eq!(manifest.source.input_sha256, Some(sha256_hex(&input)));
    assert_eq!(
        manifest.source.retrieved,
        Some(Date::parse("2026-03-04").unwrap())
    );
    assert_eq!(
        manifest.source.objects,
        Some(43),
        "40 entries, a duplicate, two pin-only objects"
    );
    assert_eq!(manifest.source.force_https, Some(41));
    assert!(
        manifest.tool.starts_with("hsts-preload "),
        "{}",
        manifest.tool
    );

    // The report says the same.
    assert!(
        outcome
            .out
            .contains("43 objects, 41 force-https, 2 skipped"),
        "{}",
        outcome.out
    );
    assert!(
        outcome.out.contains("1 duplicate listings merged"),
        "{}",
        outcome.out
    );
    assert!(
        outcome
            .out
            .contains("40 entries, 40 include subdomains (100.0 %)"),
        "{}",
        outcome.out
    );
    assert!(
        outcome
            .out
            .contains(&format!("SHA-256 {}", manifest.output_sha256)),
        "{}",
        outcome.out
    );
    assert!(outcome.out.contains("no earlier file"), "{}", outcome.out);
    assert_eq!(outcome.err, "");
}

#[test]
fn build_twice_on_the_same_input_gives_identical_data_and_a_manifest_that_differs_only_in_the_date()
{
    let work = Work::new();
    let input = work.write("input.json", chromium_json(&hosts(40)));
    for (name, date) in [("a", "2026-01-01"), ("b", "2026-02-02")] {
        let output = work.path(&format!("{name}.fstmap"));
        let outcome = run(&[
            "build",
            "--input",
            &input,
            "--output",
            &output,
            "--min-entries",
            "1",
            "--retrieved",
            date,
        ]);
        assert_done(&outcome);
    }
    assert_eq!(work.read("a.fstmap"), work.read("b.fstmap"));
    let (a, b) = (work.text("a.manifest.json"), work.text("b.manifest.json"));
    assert_ne!(a, b);
    assert_eq!(a.replace("2026-01-01", "2026-02-02"), b);

    // The same date: the manifests are identical too.
    let output = work.path("c.fstmap");
    let outcome = run(&[
        "build",
        "--input",
        &input,
        "--output",
        &output,
        "--min-entries",
        "1",
        "--retrieved",
        "2026-01-01",
    ]);
    assert_done(&outcome);
    assert_eq!(work.text("a.manifest.json"), work.text("c.manifest.json"));
}

#[test]
fn build_records_today_unless_told_a_date() {
    let work = Work::new();
    let before = Date::today_utc();
    assert_done(&build(&work, &hosts(40), &[]));
    let after = Date::today_utc();
    let manifest = Manifest::parse(&work.text("list.manifest.json")).unwrap();
    let retrieved = manifest.source.retrieved.expect("a retrieval date");
    assert!(
        before <= retrieved && retrieved <= after,
        "{retrieved} not in {before}..{after}"
    );
}

#[test]
fn build_records_the_ref_and_the_url() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(40), &[]));
    let manifest = Manifest::parse(&work.text("list.manifest.json")).unwrap();
    assert_eq!(manifest.source.git_ref.as_deref(), Some("main"));
    assert_eq!(
        manifest.source.url.as_deref(),
        Some(
            "https://chromium.googlesource.com/chromium/src/+/main/net/http/transport_security_state_static.json?format=TEXT"
        )
    );

    let work = Work::new();
    assert_done(&build(&work, &hosts(40), &["--ref", "0123abc"]));
    let manifest = Manifest::parse(&work.text("list.manifest.json")).unwrap();
    assert_eq!(manifest.source.git_ref.as_deref(), Some("0123abc"));
    assert!(
        manifest
            .source
            .url
            .unwrap()
            .contains("/+/0123abc/net/http/")
    );

    let work = Work::new();
    assert_done(&build(
        &work,
        &hosts(40),
        &["--source-url", "https://example.test/list"],
    ));
    let manifest = Manifest::parse(&work.text("list.manifest.json")).unwrap();
    assert_eq!(
        manifest.source.url.as_deref(),
        Some("https://example.test/list")
    );
    assert_eq!(manifest.source.git_ref.as_deref(), Some("main"));
}

#[test]
fn build_writes_the_manifest_where_manifest_says() {
    let work = Work::new();
    let manifest_path = work.path("elsewhere.json");
    assert_done(&build(&work, &hosts(40), &["--manifest", &manifest_path]));
    assert!(work.exists("elsewhere.json"));
    assert!(!work.exists("list.manifest.json"));
    // A data file whose name does not end in .fstmap gets the suffix appended.
    let input = work.write("input2.json", chromium_json(&hosts(40)));
    let output = work.path("plain");
    assert_done(&run(&[
        "build",
        "--input",
        &input,
        "--output",
        &output,
        "--min-entries",
        "1",
    ]));
    assert!(work.exists("plain.manifest.json"));
}

#[test]
fn build_reads_the_base64_form_and_standard_input() {
    let work = Work::new();
    let json = chromium_json(&hosts(40));
    assert_done(&build(&work, &hosts(40), &[]));
    let plain = work.read("list.fstmap");

    // Gitiles' form: base64 in lines of 76 characters.
    let encoded = STANDARD.encode(&json);
    let wrapped: Vec<&str> = encoded
        .as_bytes()
        .chunks(76)
        .map(|chunk| std::str::from_utf8(chunk).unwrap())
        .collect();
    let wrapped = wrapped.join("\n") + "\n";
    let input = work.write("input.b64", &wrapped);
    let output = work.path("b64.fstmap");
    let outcome = run(&[
        "build",
        "--input",
        &input,
        "--base64",
        "--output",
        &output,
        "--min-entries",
        "1",
    ]);
    assert_done(&outcome);
    assert_eq!(work.read("b64.fstmap"), plain);
    let manifest = Manifest::parse(&work.text("b64.manifest.json")).unwrap();
    assert_eq!(
        manifest.source.input_sha256,
        Some(sha256_hex(wrapped.as_bytes())),
        "the input as read"
    );

    // Standard input, plain and base64.
    let output = work.path("stdin.fstmap");
    let outcome = run_with_input(
        &[
            "build",
            "--input",
            "-",
            "--output",
            &output,
            "--min-entries",
            "1",
        ],
        json.as_bytes(),
    );
    assert_done(&outcome);
    assert!(
        outcome.out.contains("read standard input:"),
        "{}",
        outcome.out
    );
    assert_eq!(work.read("stdin.fstmap"), plain);
    let output = work.path("stdin64.fstmap");
    let outcome = run_with_input(
        &[
            "build",
            "--input",
            "-",
            "--base64",
            "--output",
            &output,
            "--min-entries",
            "1",
        ],
        encoded.as_bytes(),
    );
    assert_done(&outcome);
    assert_eq!(work.read("stdin64.fstmap"), plain);
}

#[test]
fn rebuilding_over_an_existing_file_reports_what_changed() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(40), &[]));
    let outcome = build(&work, &hosts(40), &[]);
    assert_done(&outcome);
    assert!(
        outcome
            .out
            .contains("against the file it replaced: added 0 / removed 0 / changed 0"),
        "{}",
        outcome.out
    );
    let outcome = build(&work, &hosts(42), &[]);
    assert_done(&outcome);
    assert!(
        outcome
            .out
            .contains("against the file it replaced: added 2 / removed 0 / changed 0"),
        "{}",
        outcome.out
    );
    // Only the three files are there: no temporary file is left behind.
    assert_eq!(
        work.names(),
        ["input.json", "list.fstmap", "list.manifest.json"]
    );
}

#[test]
fn an_existing_file_that_is_not_a_list_is_replaced_with_a_note() {
    let work = Work::new();
    work.write("list.fstmap", b"");
    let outcome = build(&work, &hosts(40), &[]);
    assert_done(&outcome);
    assert!(
        outcome.out.contains("note: the existing"),
        "{}",
        outcome.out
    );
    assert_eq!(work.preload("list.fstmap").len(), 40);
}

// ---------------------------------------------------------------- build refusals

/// The output and its manifest are not there.
fn assert_nothing_written(work: &Work) {
    assert!(!work.exists("list.fstmap"), "{:?}", work.names());
    assert!(!work.exists("list.manifest.json"), "{:?}", work.names());
    assert_eq!(work.names(), ["input.json"], "no temporary file either");
}

#[test]
fn the_default_minimum_refuses_the_small_fixture() {
    let work = Work::new();
    let input = work.write("input.json", chromium_json(&hosts(40)));
    let output = work.path("list.fstmap");
    let outcome = run(&["build", "--input", &input, "--output", &output]);
    assert_refused(&outcome, 1);
    assert!(
        outcome.err.starts_with("hsts-preload: refusing to replace"),
        "{}",
        outcome.err
    );
    assert!(outcome.err.contains("100,000"), "{}", outcome.err);
    assert_nothing_written(&work);
}

#[test]
fn a_missing_canary_refuses_the_build() {
    let work = Work::new();
    let canary = CANARIES[2];
    let list: Vec<_> = hosts(40)
        .into_iter()
        .filter(|(host, _)| host != canary)
        .collect();
    let outcome = build(&work, &list, &[]);
    assert_refused(&outcome, 1);
    assert!(outcome.err.contains(canary), "{}", outcome.err);
    assert_nothing_written(&work);
}

#[test]
fn a_canary_without_the_flag_refuses_the_build() {
    let work = Work::new();
    let canary = CANARIES[1];
    let mut list = hosts(40);
    list.iter_mut().find(|(host, _)| host == canary).unwrap().1 = false;
    let outcome = build(&work, &list, &[]);
    assert_refused(&outcome, 1);
    assert!(outcome.err.contains(canary), "{}", outcome.err);
    assert_nothing_written(&work);
}

#[test]
fn a_list_in_which_few_entries_include_subdomains_is_refused() {
    let work = Work::new();
    let mut list = hosts(44);
    for entry in list.iter_mut().skip(CANARIES.len()) {
        entry.1 = false;
    }
    let outcome = build(&work, &list, &[]);
    assert_refused(&outcome, 1);
    assert!(
        outcome.err.contains("include subdomains"),
        "{}",
        outcome.err
    );
    assert!(outcome.err.contains("90 %"), "{}", outcome.err);
    assert_nothing_written(&work);
}

#[test]
fn a_shrink_of_11_percent_is_refused_and_one_of_10_percent_is_not() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(100), &[]));
    let data = work.read("list.fstmap");
    let manifest = work.read("list.manifest.json");

    let outcome = build(&work, &hosts(89), &[]);
    assert_refused(&outcome, 1);
    assert!(outcome.err.contains("fewer entries"), "{}", outcome.err);
    assert!(outcome.err.contains("--allow-shrink"), "{}", outcome.err);
    assert_eq!(work.read("list.fstmap"), data, "the file is as it was");
    assert_eq!(
        work.read("list.manifest.json"),
        manifest,
        "and so is the manifest"
    );
    assert_eq!(
        work.names(),
        ["input.json", "list.fstmap", "list.manifest.json"]
    );

    let outcome = build(&work, &hosts(90), &[]);
    assert_done(&outcome);
    assert_eq!(work.preload("list.fstmap").len(), 90);
}

#[test]
fn allow_shrink_passes_the_shrink_and_nothing_else() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(100), &[]));
    let outcome = build(&work, &hosts(89), &["--allow-shrink"]);
    assert_done(&outcome);
    assert_eq!(work.preload("list.fstmap").len(), 89);

    // A missing canary is still refused, and the file stays as it was.
    let data = work.read("list.fstmap");
    let list: Vec<_> = hosts(50)
        .into_iter()
        .filter(|(host, _)| host != CANARIES[0])
        .collect();
    let outcome = build(&work, &list, &["--allow-shrink"]);
    assert_refused(&outcome, 1);
    assert!(outcome.err.contains(CANARIES[0]), "{}", outcome.err);
    assert_eq!(work.read("list.fstmap"), data);
}

#[test]
fn max_shrink_percent_moves_the_limit() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(100), &[]));
    assert_refused(
        &build(&work, &hosts(89), &["--max-shrink-percent", "10"]),
        1,
    );
    assert_done(&build(&work, &hosts(80), &["--max-shrink-percent", "25"]));
}

#[test]
fn an_invalid_host_refuses_the_build_and_names_it() {
    let work = Work::new();
    let mut list = hosts(40);
    list.push(("bad_host.example".to_string(), true));
    let outcome = build(&work, &list, &[]);
    assert_refused(&outcome, 1);
    assert!(outcome.err.contains("bad_host.example"), "{}", outcome.err);
    assert!(
        outcome.err.contains("input.json"),
        "names the file: {}",
        outcome.err
    );
    assert_nothing_written(&work);
}

#[test]
fn truncated_json_refuses_the_build_and_names_the_file() {
    let work = Work::new();
    let json = chromium_json(&hosts(40));
    let input = work.write("input.json", &json[..json.len() / 2]);
    let output = work.path("list.fstmap");
    let outcome = run(&[
        "build",
        "--input",
        &input,
        "--output",
        &output,
        "--min-entries",
        "1",
    ]);
    assert_refused(&outcome, 1);
    assert!(outcome.err.contains("input.json"), "{}", outcome.err);
    assert!(outcome.err.contains("not valid JSON"), "{}", outcome.err);
    assert_nothing_written(&work);
}

#[test]
fn input_that_is_not_base64_or_not_there_refuses_the_build() {
    let work = Work::new();
    let input = work.write("input.json", chromium_json(&hosts(40)));
    let output = work.path("list.fstmap");
    let outcome = run(&[
        "build",
        "--input",
        &input,
        "--base64",
        "--output",
        &output,
        "--min-entries",
        "1",
    ]);
    assert_refused(&outcome, 1);
    assert!(outcome.err.contains("not base64"), "{}", outcome.err);

    let missing = work.path("missing.json");
    let outcome = run(&[
        "build",
        "--input",
        &missing,
        "--output",
        &output,
        "--min-entries",
        "1",
    ]);
    assert_refused(&outcome, 1);
    assert!(outcome.err.contains("missing.json"), "{}", outcome.err);
    assert_nothing_written(&work);
}

#[test]
fn a_missing_output_directory_refuses_the_build_before_anything_is_written() {
    let work = Work::new();
    let input = work.write("input.json", chromium_json(&hosts(40)));
    let output = work.path("no/such/directory/list.fstmap");
    let outcome = run(&[
        "build",
        "--input",
        &input,
        "--output",
        &output,
        "--min-entries",
        "1",
    ]);
    assert_refused(&outcome, 1);
    assert!(outcome.err.contains("does not exist"), "{}", outcome.err);
    // The data's directory is there but the manifest's is not: still nothing is written.
    let output = work.path("list.fstmap");
    let manifest = work.path("no/such/directory/m.json");
    let outcome = run(&[
        "build",
        "--input",
        &input,
        "--output",
        &output,
        "--manifest",
        &manifest,
        "--min-entries",
        "1",
    ]);
    assert_refused(&outcome, 1);
    assert_nothing_written(&work);
}

// ---------------------------------------------------------------- the command line

fn assert_usage(outcome: &Outcome, what: &str) {
    assert_eq!(
        outcome.status, 2,
        "stdout:\n{}\nstderr:\n{}",
        outcome.out, outcome.err
    );
    assert!(outcome.err.starts_with("hsts-preload: "), "{}", outcome.err);
    assert!(
        outcome.err.contains(what),
        "{what:?} not in {}",
        outcome.err
    );
    assert!(
        outcome.err.contains("usage:"),
        "the usage follows: {}",
        outcome.err
    );
    assert_eq!(outcome.out, "");
}

#[test]
fn an_unknown_option_is_a_usage_error_and_writes_nothing() {
    let work = Work::new();
    let input = work.write("input.json", chromium_json(&hosts(40)));
    let output = work.path("list.fstmap");
    let outcome = run(&[
        "build",
        "--input",
        &input,
        "--output",
        &output,
        "--frobnicate",
    ]);
    assert_usage(&outcome, "unknown option --frobnicate");
    assert_nothing_written(&work);
    assert_usage(
        &run(&["check", "x.fstmap", "--base64"]),
        "unknown option --base64",
    );
    assert_usage(&run(&["diff", "a", "b", "-x"]), "unknown option -x");
}

#[test]
fn a_missing_value_is_a_usage_error() {
    assert_usage(&run(&["build", "--input"]), "--input needs a value");
    assert_usage(
        &run(&["build", "--input", "in.json", "--output"]),
        "--output needs a value",
    );
    assert_usage(
        &run(&["build", "--input", "--base64", "--output", "x"]),
        "--input needs a value",
    );
    assert_usage(
        &run(&["check", "x.fstmap", "--max-age-days"]),
        "--max-age-days needs a value",
    );
}

#[test]
fn bad_numbers_dates_and_combinations_are_usage_errors() {
    assert_usage(
        &run(&[
            "build",
            "--input",
            "i",
            "--output",
            "o",
            "--min-entries",
            "lots",
        ]),
        "--min-entries needs a whole number",
    );
    assert_usage(
        &run(&[
            "build",
            "--input",
            "i",
            "--output",
            "o",
            "--retrieved",
            "2026-02-30",
        ]),
        "--retrieved",
    );
    assert_usage(&run(&["check", "x", "--today", "yesterday"]), "--today");
    assert_usage(
        &run(&["check", "x", "--max-age-days", "-3"]),
        "--max-age-days",
    );
    assert_usage(
        &run(&["diff", "a", "b", "--limit", "5", "--all"]),
        "cannot be used together",
    );
    assert_usage(
        &run(&[
            "build",
            "--input",
            "i",
            "--output",
            "i.fstmap",
            "--manifest",
            "i.fstmap",
        ]),
        "same file",
    );
    assert_usage(
        &run(&["build", "--input", "i", "--input", "j", "--output", "o"]),
        "--input is given twice",
    );
}

#[test]
fn a_command_without_what_it_needs_is_a_usage_error() {
    assert_usage(&run(&[]), "no command given");
    assert_usage(&run(&["frobnicate"]), "unknown command \"frobnicate\"");
    assert_usage(&run(&["build"]), "build needs --input");
    assert_usage(&run(&["build", "--input", "i"]), "build needs --output");
    assert_usage(
        &run(&["build", "--input", "i", "--output", "o", "extra"]),
        "found 1 argument",
    );
    assert_usage(&run(&["check"]), "<file.fstmap>");
    assert_usage(&run(&["diff", "only-one"]), "<old.fstmap> <new.fstmap>");
}

#[test]
fn help_prints_the_usage_on_standard_output_and_succeeds() {
    for arguments in [
        &["-h"][..],
        &["--help"],
        &["build", "--help"],
        &["check", "-h"],
        &["diff", "x", "--help"],
    ] {
        let outcome = run(arguments);
        assert_done(&outcome);
        assert!(
            outcome.out.starts_with("usage:"),
            "{arguments:?}: {}",
            outcome.out
        );
        assert!(
            outcome.out.contains("hsts-preload build --input"),
            "{}",
            outcome.out
        );
        assert!(outcome.out.contains("hsts-preload diff"), "{}", outcome.out);
        assert_eq!(outcome.err, "");
    }
}

// ---------------------------------------------------------------- check

#[test]
fn check_passes_a_file_that_build_wrote() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(40), &["--retrieved", "2026-01-01"]));
    let list = work.path("list.fstmap");
    let outcome = run(&[
        "check",
        &list,
        "--min-entries",
        "1",
        "--today",
        "2026-01-11",
    ]);
    assert_done(&outcome);
    assert_eq!(outcome.last_line(), "ok");
    assert!(
        outcome.out.starts_with(&format!("checked {list}\n")),
        "{}",
        outcome.out
    );
    assert!(
        outcome
            .out
            .contains("40 entries, 40 of them include subdomains"),
        "{}",
        outcome.out
    );
    assert!(outcome.out.contains("10 days old"), "{}", outcome.out);
    assert!(
        outcome.out.contains(&sha256_hex(&work.read("list.fstmap"))),
        "{}",
        outcome.out
    );
    assert!(!outcome.out.contains("problem"), "{}", outcome.out);
}

#[test]
fn check_applies_the_default_minimum_to_a_small_file() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(40), &[]));
    let outcome = run(&["check", &work.path("list.fstmap")]);
    assert_refused(&outcome, 1);
    assert_eq!(outcome.last_line(), "not ok");
    assert!(
        outcome.out.contains("problem: the list has 40 entries"),
        "{}",
        outcome.out
    );
}

#[test]
fn check_fails_a_corrupted_copy() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(40), &[]));
    let mut bytes = work.read("list.fstmap");
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    let copy = work.write("corrupt.fstmap", &bytes);
    let outcome = run(&["check", &copy, "--min-entries", "1"]);
    assert_refused(&outcome, 1);
    assert_eq!(outcome.last_line(), "not ok");
    assert!(outcome.out.contains("problem:"), "{}", outcome.out);

    // Not a data file at all.
    let junk = work.write("junk.fstmap", b"this is not a list");
    let outcome = run(&["check", &junk]);
    assert_refused(&outcome, 1);
    assert_eq!(outcome.last_line(), "not ok");
}

#[test]
fn check_fails_when_the_manifest_hash_is_not_the_files() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(40), &[]));
    let manifest = work.text("list.manifest.json");
    let hash = sha256_hex(&work.read("list.fstmap"));
    assert!(manifest.contains(&hash));
    work.write(
        "list.manifest.json",
        manifest.replace(&hash, &"0".repeat(64)),
    );
    let outcome = run(&["check", &work.path("list.fstmap"), "--min-entries", "1"]);
    assert_refused(&outcome, 1);
    assert!(outcome.out.contains("output_sha256"), "{}", outcome.out);
    assert_eq!(outcome.last_line(), "not ok");
}

#[test]
fn check_fails_an_old_list_only_when_asked_to_with_a_limit() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(40), &["--retrieved", "2026-01-01"]));
    let list = work.path("list.fstmap");
    let base = [
        "check",
        list.as_str(),
        "--min-entries",
        "1",
        "--today",
        "2026-11-20",
    ];

    let outcome = run(&base);
    assert_done(&outcome);
    assert!(
        outcome.out.contains("323 days old"),
        "age is a note: {}",
        outcome.out
    );

    let mut limited = base.to_vec();
    limited.extend(["--max-age-days", "30"]);
    let outcome = run(&limited);
    assert_refused(&outcome, 1);
    assert!(
        outcome
            .out
            .contains("problem: the list was retrieved 2026-01-01, 323 days ago"),
        "{}",
        outcome.out
    );
    assert_eq!(outcome.last_line(), "not ok");

    let mut generous = base.to_vec();
    generous.extend(["--max-age-days", "400"]);
    assert_done(&run(&generous));
}

#[test]
fn check_notes_a_missing_manifest_but_demands_one_that_was_asked_for() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(40), &[]));
    std::fs::remove_file(work.path("list.manifest.json")).unwrap();
    let list = work.path("list.fstmap");
    let outcome = run(&["check", &list, "--min-entries", "1"]);
    assert_done(&outcome);
    assert!(
        outcome.out.contains("note: no manifest next to the file"),
        "{}",
        outcome.out
    );
    assert!(outcome.out.contains("age unknown"), "{}", outcome.out);

    let outcome = run(&[
        "check",
        &list,
        "--min-entries",
        "1",
        "--manifest",
        &work.path("asked.json"),
    ]);
    assert_refused(&outcome, 1);
    assert!(
        outcome.out.contains("problem: cannot read the manifest"),
        "{}",
        outcome.out
    );

    // A manifest that is there but not a manifest is a problem too.
    let broken = work.write("list.manifest.json", b"{ \"entries\": ");
    let outcome = run(&["check", &list, "--min-entries", "1"]);
    assert_refused(&outcome, 1);
    assert!(
        outcome
            .out
            .contains(&format!("the manifest {broken} cannot be used")),
        "{}",
        outcome.out
    );
}

#[test]
fn check_of_a_missing_file_is_a_data_problem() {
    let work = Work::new();
    let missing = work.path("missing.fstmap");
    let outcome = run(&["check", &missing]);
    assert_refused(&outcome, 1);
    assert!(
        outcome.err.starts_with("hsts-preload: cannot read "),
        "{}",
        outcome.err
    );
    assert!(outcome.err.contains("missing.fstmap"), "{}", outcome.err);
    assert_eq!(outcome.out, "");
}

// ---------------------------------------------------------------- diff

#[test]
fn diff_shows_an_added_a_removed_and_a_changed_host() {
    let work = Work::new();
    let old: Vec<(String, bool)> = hosts(10)
        .into_iter()
        .chain([
            ("gone.example".to_string(), true),
            ("flip.example".to_string(), true),
        ])
        .collect();
    let new: Vec<(String, bool)> = hosts(10)
        .into_iter()
        .chain([
            ("new.example".to_string(), true),
            ("flip.example".to_string(), false),
        ])
        .collect();
    assert_done(&build(&work, &old, &["--manifest", &work.path("old.json")]));
    std::fs::rename(work.path("list.fstmap"), work.path("old.fstmap")).unwrap();
    assert_done(&build(&work, &new, &["--manifest", &work.path("new.json")]));
    std::fs::rename(work.path("list.fstmap"), work.path("new.fstmap")).unwrap();

    let outcome = run(&["diff", &work.path("old.fstmap"), &work.path("new.fstmap")]);
    assert_done(&outcome);
    assert_eq!(
        outcome.out,
        "added 1 / removed 1 / changed 1\n\
         + new.example (includes subdomains)\n\
         - gone.example (included subdomains)\n\
         ~ flip.example (now host only)\n"
    );
    assert_eq!(outcome.err, "");

    // The other way round, and a file against itself.
    let outcome = run(&["diff", &work.path("new.fstmap"), &work.path("old.fstmap")]);
    assert_done(&outcome);
    assert!(
        outcome.out.starts_with("added 1 / removed 1 / changed 1\n"),
        "{}",
        outcome.out
    );
    assert!(outcome.out.contains("+ gone.example"), "{}", outcome.out);
    let outcome = run(&["diff", &work.path("old.fstmap"), &work.path("old.fstmap")]);
    assert_done(&outcome);
    assert_eq!(outcome.out, "added 0 / removed 0 / changed 0\n");
}

#[test]
fn diff_shows_twenty_lines_of_each_kind_unless_told_otherwise() {
    let work = Work::new();
    assert_done(&build(
        &work,
        &hosts(40),
        &["--manifest", &work.path("a.json")],
    ));
    std::fs::rename(work.path("list.fstmap"), work.path("old.fstmap")).unwrap();
    assert_done(&build(
        &work,
        &hosts(70),
        &["--manifest", &work.path("b.json")],
    ));
    std::fs::rename(work.path("list.fstmap"), work.path("new.fstmap")).unwrap();
    let (old, new) = (work.path("old.fstmap"), work.path("new.fstmap"));

    let outcome = run(&["diff", &old, &new]);
    assert_done(&outcome);
    assert!(
        outcome
            .out
            .starts_with("added 30 / removed 0 / changed 0\n")
    );
    assert_eq!(
        outcome
            .out
            .lines()
            .filter(|line| line.starts_with("+ "))
            .count(),
        20
    );
    assert!(
        outcome.out.contains("... and 10 more added"),
        "{}",
        outcome.out
    );

    let outcome = run(&["diff", &old, &new, "--limit", "2"]);
    assert_done(&outcome);
    assert_eq!(
        outcome
            .out
            .lines()
            .filter(|line| line.starts_with("+ "))
            .count(),
        2
    );
    assert!(
        outcome.out.contains("... and 28 more added"),
        "{}",
        outcome.out
    );

    let outcome = run(&["diff", &old, &new, "--all"]);
    assert_done(&outcome);
    assert_eq!(
        outcome
            .out
            .lines()
            .filter(|line| line.starts_with("+ "))
            .count(),
        30
    );
    assert!(!outcome.out.contains("more added"), "{}", outcome.out);
}

#[test]
fn diff_of_a_file_that_is_not_a_list_is_a_data_problem_that_names_it() {
    let work = Work::new();
    assert_done(&build(&work, &hosts(40), &[]));
    let junk = work.write("junk.fstmap", b"not a list");
    let outcome = run(&["diff", &work.path("list.fstmap"), &junk]);
    assert_refused(&outcome, 1);
    assert!(outcome.err.contains("junk.fstmap"), "{}", outcome.err);
    let outcome = run(&[
        "diff",
        &work.path("list.fstmap"),
        &work.path("absent.fstmap"),
    ]);
    assert_refused(&outcome, 1);
    assert!(outcome.err.contains("absent.fstmap"), "{}", outcome.err);
}

// ---------------------------------------------------------------- the round trip

fn servo_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../third_party/servo-default-resources/resources/hsts_preload.fstmap")
}

/// `entries` as a document in the shape of Chromium's file, in reverse order, with a comment
/// header, a pin-only object (with `include_subdomains`) and trailing commas.
fn chromium_shaped(entries: &[mercury_devtools::hsts::Entry]) -> String {
    let mut json = String::with_capacity(entries.len() * 110 + 1_000);
    json.push_str(
        "// Copyright 2015 The Chromium Authors\n// round trip test\n{\n  \"pinsets\": [],\n  \"entries\": [\n",
    );
    json.push_str(
        "    { \"name\": \"pin-only.example\", \"pins\": \"test\", \"include_subdomains\": true },\n",
    );
    for entry in entries.iter().rev() {
        json.push_str(&format!(
            "    {{ \"name\": \"{}\", \"policy\": \"public-suffix\", \"mode\": \"force-https\", \"include_subdomains\": {} }},\n",
            entry.host, entry.include_subdomains
        ));
    }
    json.push_str("  ],\n}\n");
    json
}

/// The strongest offline proof of the format: the list Servo 0.8.0 ships, written back out in the
/// shape of Chromium's file and built with the default guards, comes out with the same entries
/// and flags, and byte for byte the same file.
#[test]
fn the_list_servo_ships_survives_a_round_trip_through_chromiums_shape() {
    let shipped = std::fs::read(servo_file()).expect("read the list Servo ships");
    let original = Preload::open(shipped.clone()).expect("the shipped list opens");
    assert_eq!(original.len(), 166_958);

    // The shipped list meets the default guards as it is: every canary is in it with the flag.
    for host in CANARIES {
        assert_eq!(original.get(host), Some(true), "{host}");
    }
    let expected = original.entries();

    let json = chromium_shaped(&expected);

    let work = Work::new();
    let input = work.write("chromium.json", &json);
    let output = work.path("rebuilt.fstmap");
    // The default guards: 166,958 entries, 99.85 % include subdomains, the canaries.
    let outcome = run(&["build", "--input", &input, "--output", &output]);
    assert_done(&outcome);
    assert!(
        outcome
            .out
            .contains("166959 objects, 166958 force-https, 1 skipped"),
        "{}",
        outcome.out
    );

    let rebuilt = work.preload("rebuilt.fstmap");
    assert_eq!(
        rebuilt.entries(),
        expected,
        "the same hosts with the same flags"
    );
    assert_eq!(rebuilt.include_subdomains_count(), 166_708);
    assert_eq!(rebuilt.id_scheme_violation(), None);

    // The tool's own `diff` agrees, and so does `check` on the result.
    let outcome = run(&["diff", &servo_file().to_string_lossy(), &output]);
    assert_done(&outcome);
    assert_eq!(
        outcome.out.lines().next(),
        Some("added 0 / removed 0 / changed 0"),
        "{}",
        outcome.out
    );
    let outcome = run(&["check", &output]);
    assert_done(&outcome);
    assert_eq!(outcome.last_line(), "ok");
    assert!(
        outcome
            .out
            .contains("166,958 entries, 166,708 of them include subdomains"),
        "{}",
        outcome.out
    );

    // The same build over the file it just wrote changes nothing.
    let again = run(&["build", "--input", &input, "--output", &output]);
    assert_done(&again);
    assert!(
        again
            .out
            .contains("against the file it replaced: added 0 / removed 0 / changed 0"),
        "{}",
        again.out
    );

    // Byte for byte: the encoding is deterministic, and this tool's encoder reproduces what the
    // generator behind the shipped file wrote (measured on the 166,958 entries of Servo 0.8.0).
    assert!(original.rebuilds_identically(&shipped));
    assert!(
        work.read("rebuilt.fstmap") == shipped,
        "the rebuilt list differs from the shipped one"
    );
}

// ---------------------------------------------------------------- the download script

#[cfg(unix)]
mod script {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use tempfile::TempDir;

    use mercury_devtools::hsts::DEFAULT_DATA;

    const URL_MAIN: &str = "https://chromium.googlesource.com/chromium/src/+/main/net/http/transport_security_state_static.json?format=TEXT";

    /// A temporary tree with the script at `scripts/dev/`, an empty placeholder where the data file
    /// is, and a `bin/` of stub `curl` and `cargo` programs that log their arguments (and the
    /// directory they ran in) and do nothing else. The script runs with that `bin/` first on PATH,
    /// from a different working directory.
    struct Tree {
        dir: TempDir,
    }

    impl Tree {
        fn new() -> Tree {
            let dir = tempfile::tempdir().expect("temporary directory");
            let root = dir.path();
            let script_dir = root.join("scripts/dev");
            std::fs::create_dir_all(&script_dir).unwrap();
            std::fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../scripts/dev/update-hsts-preload.sh"),
                script_dir.join("update-hsts-preload.sh"),
            )
            .expect("copy the script");
            let data = root.join(DEFAULT_DATA);
            std::fs::create_dir_all(data.parent().unwrap()).unwrap();
            std::fs::write(&data, b"").unwrap();
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            stub(
                &bin,
                "curl",
                "out=\nwhile [ $# -gt 0 ]; do\n  case $1 in -o | --output) out=$2; shift ;; esac\n  shift\ndone\nif [ -n \"$out\" ]; then echo stub > \"$out\"; fi\n",
            );
            stub(&bin, "cargo", "");
            Tree { dir }
        }

        /// Runs the script; returns the exit status, standard output and standard error.
        fn run(&self, arguments: &[&str]) -> (Option<i32>, String, String) {
            let elsewhere = tempfile::tempdir().expect("another directory");
            self.run_in(elsewhere.path(), arguments)
        }

        /// Runs the script with `directory` as its working directory.
        fn run_in(&self, directory: &Path, arguments: &[&str]) -> (Option<i32>, String, String) {
            let path = format!(
                "{}:{}",
                self.dir.path().join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            );
            let output = Command::new("/bin/sh")
                .arg(self.dir.path().join("scripts/dev/update-hsts-preload.sh"))
                .args(arguments)
                .env("PATH", path)
                .current_dir(directory)
                .output()
                .expect("run the script");
            (
                output.status.code(),
                String::from_utf8_lossy(&output.stdout).into_owned(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
            )
        }

        /// The calls a stub logged: (directory, arguments), one per call.
        fn calls(&self, program: &str) -> Vec<(String, String)> {
            let log = self.dir.path().join("bin").join(format!("{program}.log"));
            std::fs::read_to_string(log)
                .unwrap_or_default()
                .lines()
                .map(|line| {
                    let (directory, arguments) = line.split_once('\t').expect("a logged call");
                    (directory.to_string(), arguments.to_string())
                })
                .collect()
        }

        fn root(&self) -> PathBuf {
            self.dir.path().canonicalize().unwrap()
        }
    }

    /// A stub program: logs `directory<TAB>arguments`, then runs `body`.
    fn stub(bin: &Path, name: &str, body: &str) {
        let log =
            format!("printf '%s\\t%s\\n' \"$PWD\" \"$*\" >> \"$(dirname \"$0\")/{name}.log\"\n");
        let path = bin.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{log}{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn argument_after<'a>(call: &'a str, option: &str) -> &'a str {
        let words: Vec<&str> = call.split(' ').collect();
        let at = words
            .iter()
            .position(|word| *word == option)
            .unwrap_or_else(|| panic!("{option} in {call}"));
        words[at + 1]
    }

    #[test]
    fn a_bad_ref_is_refused_before_curl_or_cargo_run() {
        let tree = Tree::new();
        for bad in [
            "../x",
            "a b",
            "a/../b",
            "x;touch pwned",
            "$(id)",
            "/abs",
            "main/",
            "a//b",
            "-x",
            "",
            "é",
            "x\ny",
            "..",
        ] {
            let (status, _, stderr) = tree.run(&[bad]);
            assert!(
                status.is_some_and(|code| code != 0),
                "{bad:?} was accepted: {status:?}"
            );
            assert!(!stderr.is_empty(), "{bad:?} gave no message");
        }
        assert!(
            tree.calls("curl").is_empty(),
            "curl ran: {:?}",
            tree.calls("curl")
        );
        assert!(
            tree.calls("cargo").is_empty(),
            "cargo ran: {:?}",
            tree.calls("cargo")
        );
        // The same for a bad ref after --from, and for options that make no sense.
        let from = tree.root().join("input.json");
        std::fs::write(&from, b"{}").unwrap();
        let from = from.to_str().unwrap();
        for arguments in [
            vec!["--from", from, "../x"],
            vec!["--base64"],
            vec!["--from"],
            vec!["--frobnicate"],
            vec!["main", "extra"],
        ] {
            let (status, _, _) = tree.run(&arguments);
            assert!(
                status.is_some_and(|code| code != 0),
                "{arguments:?}: {status:?}"
            );
        }
        assert!(tree.calls("curl").is_empty() && tree.calls("cargo").is_empty());
    }

    #[test]
    fn good_refs_are_accepted() {
        for good in [
            "main",
            "refs/heads/main",
            "refs/tags/131.0.6778.1",
            "0123456789abcdef0123456789abcdef01234567",
            "release_1-2.x",
        ] {
            let tree = Tree::new();
            let (status, _, stderr) = tree.run(&[good]);
            assert_eq!(status, Some(0), "{good}: {stderr}");
            assert_eq!(tree.calls("curl").len(), 1, "{good}");
        }
    }

    #[test]
    fn the_normal_mode_calls_curl_once_for_the_exact_url_and_builds_from_its_download() {
        let tree = Tree::new();
        let (status, stdout, stderr) = tree.run(&[]);
        assert_eq!(status, Some(0), "{stderr}");
        let curl = tree.calls("curl");
        assert_eq!(curl.len(), 1, "{curl:?}");
        let (directory, arguments) = &curl[0];
        assert_eq!(
            Path::new(directory).canonicalize().unwrap(),
            tree.root(),
            "curl runs in the tree's root"
        );
        assert!(arguments.contains("--proto =https"), "{arguments}");
        assert!(
            arguments.contains("--fail") && arguments.contains("--tlsv1.2"),
            "{arguments}"
        );
        let words: Vec<&str> = arguments.split(' ').collect();
        let urls: Vec<&&str> = words.iter().filter(|word| word.contains("://")).collect();
        assert_eq!(urls, [&URL_MAIN], "no other host: {arguments}");
        assert!(!arguments.contains("http://"), "{arguments}");

        let cargo = tree.calls("cargo");
        assert_eq!(cargo.len(), 3, "build, check, diff: {cargo:?}");
        let build = &cargo[0].1;
        assert!(build.starts_with("run --locked --quiet --release -p mercury-devtools --bin hsts-preload -- build --base64 --input "), "{build}");
        assert!(
            build.contains(&format!(
                " --ref main --source-url {URL_MAIN} --output {DEFAULT_DATA}"
            )),
            "{build}"
        );
        assert!(
            cargo[1].1.ends_with(&format!("-- check {DEFAULT_DATA}")),
            "{}",
            cargo[1].1
        );
        assert!(
            cargo[2].1.contains("-- diff ") && cargo[2].1.ends_with(DEFAULT_DATA),
            "{}",
            cargo[2].1
        );
        assert_eq!(
            Path::new(&cargo[0].0).canonicalize().unwrap(),
            tree.root(),
            "cargo runs in the root, from any directory"
        );

        // The temporary directory with the download is gone.
        let input = argument_after(build, "--input");
        assert!(!Path::new(input).exists(), "{input} was not cleaned up");
        assert!(stdout.contains("Nothing is committed"), "{stdout}");
        assert!(stdout.contains("git diff --stat"), "{stdout}");
    }

    #[test]
    fn a_ref_goes_into_the_url_and_the_build() {
        let tree = Tree::new();
        let (status, _, stderr) = tree.run(&["refs/tags/131.0.6778.1"]);
        assert_eq!(status, Some(0), "{stderr}");
        let url = "https://chromium.googlesource.com/chromium/src/+/refs/tags/131.0.6778.1/net/http/transport_security_state_static.json?format=TEXT";
        assert!(
            tree.calls("curl")[0].1.ends_with(url),
            "{:?}",
            tree.calls("curl")
        );
        let build = &tree.calls("cargo")[0].1;
        assert!(build.contains(" --ref refs/tags/131.0.6778.1 "), "{build}");
        assert!(build.contains(&format!(" --source-url {url} ")), "{build}");
    }

    #[test]
    fn from_builds_a_local_copy_and_never_calls_curl() {
        let tree = Tree::new();
        let input = tree.root().join("copy.json");
        std::fs::write(&input, b"{}").unwrap();
        let input = input.to_str().unwrap();

        // The plain form: no --base64.
        let (status, _, stderr) = tree.run(&["--from", input]);
        assert_eq!(status, Some(0), "{stderr}");
        let build = &tree.calls("cargo")[0].1;
        assert!(build.contains("-- build --input "), "{build}");
        assert!(!build.contains("--base64"), "{build}");
        assert_eq!(argument_after(build, "--input"), input);
        assert!(build.contains(" --ref main "), "{build}");
        assert!(Path::new(input).exists(), "the local copy is not removed");

        // The base64 form, with a ref.
        let tree = Tree::new();
        let (status, _, stderr) = tree.run(&["--from", input, "--base64", "0123abc"]);
        assert_eq!(status, Some(0), "{stderr}");
        let build = &tree.calls("cargo")[0].1;
        assert!(build.contains("-- build --base64 --input "), "{build}");
        assert_eq!(argument_after(build, "--input"), input);
        assert!(build.contains(" --ref 0123abc "), "{build}");
        // Options and the ref in any order.
        let tree = Tree::new();
        let (status, _, stderr) = tree.run(&["0123abc", "--base64", "--from", input]);
        assert_eq!(status, Some(0), "{stderr}");
        assert!(
            tree.calls("cargo")[0]
                .1
                .contains("-- build --base64 --input ")
        );
        assert!(
            tree.calls("curl").is_empty(),
            "curl ran: {:?}",
            tree.calls("curl")
        );
    }

    #[test]
    fn a_relative_from_path_is_read_from_where_the_script_was_started() {
        let tree = Tree::new();
        let start = tempfile::tempdir().expect("a working directory");
        std::fs::write(start.path().join("local.json"), b"{}").unwrap();
        let (status, _, stderr) = tree.run_in(start.path(), &["--from", "local.json"]);
        assert_eq!(status, Some(0), "{stderr}");
        let build = &tree.calls("cargo")[0].1;
        let input = Path::new(argument_after(build, "--input"));
        assert!(input.is_absolute(), "{build}");
        assert_eq!(
            input.canonicalize().unwrap(),
            start.path().canonicalize().unwrap().join("local.json")
        );
        assert!(tree.calls("curl").is_empty());
    }

    #[test]
    fn a_failing_build_stops_the_script_before_the_check() {
        let tree = Tree::new();
        // A cargo that fails on build.
        stub(
            &tree.dir.path().join("bin"),
            "cargo",
            "case \"$*\" in *' build '*) exit 1 ;; esac\n",
        );
        let (status, stdout, _) = tree.run(&[]);
        assert!(status.is_some_and(|code| code != 0), "{status:?}");
        assert_eq!(tree.calls("cargo").len(), 1, "{:?}", tree.calls("cargo"));
        assert!(!stdout.contains("Nothing is committed"), "{stdout}");
    }
}
