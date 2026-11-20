// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The problems `check` reports (exit status 1), each through the library API.
mod common;

use common::{Fake, MIT};
use mercury_credits::app::{self, Outcome};
use mercury_credits::cargo::Options;
use mercury_credits::collect::Inputs;
use std::path::PathBuf;

fn problems(outcome: &Outcome) -> &[String] {
    &outcome.report.problems
}

fn assert_one_problem(outcome: &Outcome, needles: &[&str]) {
    assert!(!outcome.ok(), "expected a problem");
    assert_eq!(problems(outcome).len(), 1, "{:#?}", problems(outcome));
    for needle in needles {
        assert!(
            problems(outcome)[0].contains(needle),
            "{needle:?} not in {:?}",
            problems(outcome)[0]
        );
    }
    assert!(outcome.page.is_none(), "a run with problems has no page");
    assert!(outcome.summary().is_none());
}

#[test]
fn a_clean_run_has_a_page_and_a_summary_line_in_the_contract_form() {
    let mut fake = Fake::new();
    fake.add("alpha", "1.0.0", "MIT");
    fake.add("beta", "2.0.0", "MIT OR Apache-2.0");
    fake.add_vendored("gamma", "0.1.0", "MPL-2.0");
    fake.write(
        "third_party/vendor.toml",
        "[[crate]]\nname = \"gamma\"\nversion = \"0.1.0\"\nrepository = \"https://example.org/gamma\"\ncommit = \"c0ffee\"\n",
    );
    fake.crate_file("alpha", "LICENSE", MIT);
    let outcome = fake.outcome();
    assert!(outcome.ok(), "{:?}", problems(&outcome));
    let page = outcome.page.as_ref().expect("a page");
    let summary = outcome.summary().expect("a summary");
    // Four texts: alpha's MIT file, beta's MIT and Apache standard texts, gamma's MPL standard text.
    assert_eq!(
        summary,
        format!(
            "mercury-credits: 4 crates credited (2 registry, 1 vendored, 1 Mercury's own), 4 distinct licence texts, 2 crates on a standard text, {} bytes",
            page.len()
        )
    );
}

#[test]
fn a_tree_entry_that_is_not_in_the_metadata_is_a_problem() {
    let mut fake = Fake::new();
    fake.add("real", "1.0.0", "MIT");
    fake.crate_file("real", "LICENSE", MIT);
    let metadata = fake.metadata();
    let tree = format!("{}ghost v1.2.3\n", fake.tree_text());
    let outcome = app::analyse_inputs(&Inputs {
        root: fake.root(),
        tree: &tree,
        metadata: &metadata,
        target: "t",
        runtime_only: false,
    });
    assert_one_problem(&outcome, &["ghost 1.2.3", "not in cargo metadata"]);
}

#[test]
fn a_version_mismatch_between_tree_and_metadata_is_a_problem_too() {
    let mut fake = Fake::new();
    fake.add("real", "1.0.0", "MIT");
    let metadata = fake.metadata();
    let tree = fake.tree_text().replace("real v1.0.0", "real v1.0.1");
    let outcome = app::analyse_inputs(&Inputs {
        root: fake.root(),
        tree: &tree,
        metadata: &metadata,
        target: "t",
        runtime_only: false,
    });
    assert_one_problem(&outcome, &["real 1.0.1"]);
}

#[test]
fn no_taken_branch_is_a_problem_that_names_the_crate_and_the_expression() {
    let mut fake = Fake::new();
    fake.add("copyleft", "3.1.4", "GPL-3.0-only OR AGPL-3.0-only");
    let outcome = fake.outcome();
    assert_one_problem(
        &outcome,
        &[
            "copyleft 3.1.4",
            "GPL-3.0-only OR AGPL-3.0-only",
            "deny.toml",
        ],
    );
}

#[test]
fn an_and_term_that_is_not_taken_is_a_problem() {
    let mut fake = Fake::new();
    fake.add("mixed", "1.0.0", "MIT AND GPL-3.0-only");
    let outcome = fake.outcome();
    assert_one_problem(&outcome, &["mixed 1.0.0"]);
}

#[test]
fn an_expression_that_does_not_parse_is_a_problem() {
    let mut fake = Fake::new();
    fake.add("broken", "1.0.0", "MIT OR");
    let outcome = fake.outcome();
    assert_one_problem(&outcome, &["broken 1.0.0", "invalid licence expression"]);
}

#[test]
fn a_crate_that_declares_no_licence_is_a_problem_unless_clarify_names_one() {
    let mut fake = Fake::new();
    fake.add_without_license("nameless", "0.1.0");
    let outcome = fake.outcome();
    assert_one_problem(
        &outcome,
        &["nameless 0.1.0", "declares no `license`", "[[clarify]]"],
    );
}

#[test]
fn a_taken_licence_with_no_file_and_no_standard_text_is_a_problem() {
    let mut fake = Fake::new();
    fake.add("bare", "1.0.0", "MIT");
    std::fs::remove_file(fake.root().join("LICENSES/MIT.txt")).unwrap();
    let outcome = fake.outcome();
    assert_one_problem(
        &outcome,
        &[
            "bare 1.0.0",
            "licence MIT is taken",
            "LICENSES/MIT.txt is missing",
        ],
    );
}

#[test]
fn every_missing_standard_text_is_reported_not_only_the_first() {
    let mut fake = Fake::new();
    fake.add("bare", "1.0.0", "MIT OR Apache-2.0");
    std::fs::remove_file(fake.root().join("LICENSES/MIT.txt")).unwrap();
    std::fs::remove_file(fake.root().join("LICENSES/Apache-2.0.txt")).unwrap();
    let outcome = fake.outcome();
    assert!(!outcome.ok());
    assert_eq!(problems(&outcome).len(), 2, "{:#?}", problems(&outcome));
}

#[test]
fn an_extra_file_that_is_missing_is_a_problem() {
    let mut fake = Fake::new();
    fake.add("native-sys", "1.0.0", "MIT");
    fake.crate_file("native-sys", "LICENSE", MIT);
    fake.write(
        "crates/mercury-credits/credits.toml",
        "[[extra]]\ncrate = \"native-sys\"\nfiles = [\"deps/lz4/LICENSE\"]\n",
    );
    let outcome = fake.outcome();
    assert_one_problem(
        &outcome,
        &["native-sys 1.0.0", "[[extra]]", "deps/lz4/LICENSE"],
    );
}

#[test]
fn a_crate_from_somewhere_other_than_crates_io_is_a_problem() {
    let mut fake = Fake::new();
    fake.add("forked", "1.0.0", "MIT");
    fake.package("forked").source = Some("git+https://example.org/forked#abc".into());
    let outcome = fake.outcome();
    assert_one_problem(&outcome, &["forked 1.0.0", "not crates.io"]);
}

#[test]
fn a_path_crate_that_is_neither_a_member_nor_vendored_is_a_problem() {
    let mut fake = Fake::new();
    // The tree and the metadata agree that `loose` is a path crate; vendor.toml does not list it.
    fake.add_vendored("loose", "0.1.0", "MIT");
    let outcome = fake.outcome();
    assert_one_problem(&outcome, &["loose 0.1.0", "vendor.toml"]);
}

#[test]
fn a_missing_notice_or_notices_file_is_a_problem() {
    let mut fake = Fake::new();
    fake.add("dep", "1.0.0", "MIT");
    fake.crate_file("dep", "LICENSE", MIT);
    std::fs::remove_file(fake.root().join("NOTICE")).unwrap();
    let outcome = fake.outcome();
    assert_one_problem(&outcome, &["NOTICE cannot be read"]);
    fake.write("NOTICE", "back");
    std::fs::remove_file(fake.root().join("THIRD-PARTY-NOTICES.md")).unwrap();
    assert_one_problem(&fake.outcome(), &["THIRD-PARTY-NOTICES.md cannot be read"]);
}

#[test]
fn files_the_tool_cannot_do_without_are_problems_not_panics() {
    let mut fake = Fake::new();
    fake.add("dep", "1.0.0", "MIT");
    fake.crate_file("dep", "LICENSE", MIT);

    fake.write(
        "crates/mercury-credits/credits.toml",
        "this is = not [valid",
    );
    assert_one_problem(&fake.outcome(), &["credits.toml"]);
    fake.write(
        "crates/mercury-credits/credits.toml",
        "[[extra]]\ncrate = \"dep\"\nfilez = []\n",
    );
    assert_one_problem(&fake.outcome(), &["credits.toml", "filez"]);
    fake.write("crates/mercury-credits/credits.toml", "");

    fake.write("third_party/vendor.toml", "[[crate]]\nname = 3\n");
    assert_one_problem(&fake.outcome(), &["vendor.toml"]);
    fake.write("third_party/vendor.toml", "");

    std::fs::remove_file(fake.root().join("deny.toml")).unwrap();
    assert_one_problem(&fake.outcome(), &["deny.toml"]);
    fake.write("deny.toml", "[licenses\n");
    assert_one_problem(&fake.outcome(), &["deny.toml"]);
}

#[test]
fn an_empty_tree_is_a_problem() {
    let fake = Fake::new();
    let metadata = fake.metadata();
    let outcome = app::analyse_inputs(&Inputs {
        root: fake.root(),
        tree: "\n",
        metadata: &metadata,
        target: "t",
        runtime_only: false,
    });
    assert_one_problem(&outcome, &["cargo tree"]);
}

#[test]
fn problems_in_several_crates_are_all_reported_and_warnings_do_not_fail() {
    let mut fake = Fake::new();
    fake.add("a-gpl", "1.0.0", "GPL-3.0-only");
    fake.add("b-broken", "1.0.0", "(MIT");
    fake.add("c-fine", "1.0.0", "MIT");
    fake.add_without_license("d-nameless", "1.0.0");
    let outcome = fake.outcome();
    assert!(!outcome.ok());
    let text = problems(&outcome).join("\n");
    for name in ["a-gpl", "b-broken", "d-nameless"] {
        assert!(text.contains(name), "{name} in {text}");
    }
    assert!(!text.contains("c-fine"));
    assert_eq!(problems(&outcome).len(), 3);
    // c-fine is on a standard text: a warning, not a problem.
    assert!(
        outcome
            .report
            .warnings
            .iter()
            .any(|warning| warning.contains("c-fine 1.0.0"))
    );
}

fn options(cargo: &str, directory: &std::path::Path) -> Options {
    Options {
        package: "app".into(),
        target: "aarch64-apple-darwin".into(),
        runtime_only: false,
        manifest_path: directory.join("Cargo.toml"),
        cargo: PathBuf::from(cargo),
    }
}

#[test]
fn a_cargo_that_cannot_run_is_a_problem() {
    let dir = tempfile::tempdir().unwrap();
    let outcome = app::analyse(&options("/definitely/not/a/cargo", dir.path()));
    assert_one_problem(&outcome, &["cannot run"]);
}

#[cfg(unix)]
#[test]
fn a_cargo_that_fails_is_a_problem_with_its_message() {
    // `false` exits 1 without a word; the message still names the failed command.
    let dir = tempfile::tempdir().unwrap();
    let outcome = app::analyse(&options("/usr/bin/false", dir.path()));
    assert_one_problem(&outcome, &["`cargo tree` failed"]);
}

#[test]
fn write_page_creates_the_directory_and_leaves_no_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("nested/credits");
    let path = app::write_page(&out, "<html>one</html>").unwrap();
    assert_eq!(path, out.join("credits.html"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "<html>one</html>");
    app::write_page(&out, "<html>two</html>").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "<html>two</html>");
    let names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["credits.html"]);
}
