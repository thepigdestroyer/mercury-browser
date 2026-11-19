// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What gets credited for a crate: the files kept, the licences left out, the standard-text
//! fallback and its copyright lines, `credits.toml`, vendored crates.
mod common;

use common::{APACHE, Fake, ISC, MIT};
use mercury_credits::collect::{Block, BlockKind, Collected, Kind};

fn collected(fake: &Fake) -> Collected {
    let outcome = fake.outcome();
    assert!(outcome.ok(), "problems: {:#?}", outcome.report.problems);
    outcome.collected.expect("a collected result")
}

fn labels(block: &Block) -> Vec<String> {
    block
        .members
        .iter()
        .map(|member| member.krate.label())
        .collect()
}

/// The blocks that cover exactly these licences.
fn with_ids<'a>(collected: &'a Collected, ids: &[&str]) -> Vec<&'a Block> {
    collected
        .blocks
        .iter()
        .filter(|block| block.kind == BlockKind::License && block.ids == ids)
        .collect()
}

fn warnings_containing(collected: &Collected, needle: &str) -> Vec<String> {
    collected
        .report
        .warnings
        .iter()
        .filter(|warning| warning.contains(needle))
        .cloned()
        .collect()
}

#[test]
fn every_taken_branch_of_an_or_is_credited_from_the_files_the_package_ships() {
    let mut fake = Fake::new();
    fake.add("both", "1.0.0", "MIT OR Apache-2.0");
    fake.crate_file("both", "LICENSE-MIT", MIT);
    fake.crate_file("both", "LICENSE-APACHE", APACHE);
    let collected = collected(&fake);
    let mit = with_ids(&collected, &["MIT"]);
    let apache = with_ids(&collected, &["Apache-2.0"]);
    assert_eq!((mit.len(), apache.len()), (1, 1));
    assert_eq!(labels(mit[0]), ["both 1.0.0"]);
    assert_eq!(labels(apache[0]), ["both 1.0.0"]);
    assert!(!mit[0].members[0].standard);
    assert!(collected.standard_text_crates.is_empty());
    assert_eq!(collected.crates.len(), 1);
    assert_eq!(collected.crates[0].kind, Kind::Registry);
    assert_eq!(collected.crates[0].license, "MIT OR Apache-2.0");
}

#[test]
fn one_shipped_text_is_enough_for_an_or_and_no_standard_text_is_added() {
    let mut fake = Fake::new();
    fake.add("half", "1.0.0", "MIT OR Apache-2.0");
    fake.crate_file("half", "LICENSE-MIT", MIT);
    let collected = collected(&fake);
    assert_eq!(collected.blocks.len(), 1);
    assert_eq!(collected.blocks[0].ids, ["MIT"]);
    assert!(collected.standard_text_crates.is_empty());
}

#[test]
fn a_crate_with_no_licence_file_gets_the_standard_text_of_every_taken_branch() {
    let mut fake = Fake::new();
    fake.add("bare", "0.3.0", "MIT OR Apache-2.0");
    fake.package("bare").authors = vec!["Ann Lee <ann@example.org>".into(), "Bo Wu".into()];
    let collected = collected(&fake);
    assert_eq!(collected.standard_text_crates.len(), 1);
    assert_eq!(collected.standard_text_crates[0].label(), "bare 0.3.0");
    let mit = with_ids(&collected, &["MIT"]);
    let apache = with_ids(&collected, &["Apache-2.0"]);
    assert_eq!((mit.len(), apache.len()), (1, 1));
    assert!(mit[0].members[0].standard);
    assert!(mit[0].standard_only());
    assert!(
        mit[0].body.contains("Copyright (c) Ann Lee, Bo Wu"),
        "{}",
        mit[0].body
    );
    assert!(
        !mit[0].body.contains("ann@example.org"),
        "email addresses are dropped"
    );
    assert!(!mit[0].body.contains("<year>"));
    assert!(apache[0].body.contains("   Copyright (c) Ann Lee, Bo Wu"));
    assert_eq!(
        warnings_containing(&collected, "standard text").len(),
        1,
        "{:?}",
        collected.report.warnings
    );
    assert!(warnings_containing(&collected, "bare 0.3.0")[0].starts_with("1 crates"));
}

#[test]
fn the_copyright_line_comes_from_clarify_then_authors_then_the_name() {
    let mut fake = Fake::new();
    fake.add("clarified", "1.0.0", "MIT");
    fake.add("authored", "1.0.0", "MIT");
    fake.add("anonymous", "1.0.0", "MIT");
    fake.add("hermit", "1.0.0", "MIT");
    fake.package("authored").authors = vec!["Zed Zee".into()];
    fake.package("clarified").authors = vec!["Ignored Person".into()];
    fake.package("anonymous").repository = Some("https://example.org/anon".into());
    fake.package("anonymous").authors = vec!["<only@email.example>".into()];
    fake.write(
        "crates/mercury-credits/credits.toml",
        "[[clarify]]\ncrate = \"clarified\"\nlicence = \"MIT\"\ncopyright = \"Copyright (c) 2022 the clarified authors\"\nsource = \"read by hand\"\n",
    );
    let collected = collected(&fake);
    let body_of = |name: &str| {
        collected
            .blocks
            .iter()
            .find(|block| block.members.iter().any(|member| member.krate.name == name))
            .unwrap()
            .body
            .clone()
    };
    assert!(body_of("clarified").contains("\nCopyright (c) 2022 the clarified authors\n"));
    assert!(!body_of("clarified").contains("Ignored Person"));
    assert!(body_of("authored").contains("\nCopyright (c) Zed Zee\n"));
    assert!(
        body_of("anonymous")
            .contains("\nCopyright (c) the anonymous authors (https://example.org/anon)\n")
    );
    assert!(body_of("hermit").contains("\nCopyright (c) the hermit authors\n"));
}

#[test]
fn clarify_names_the_licence_its_copyright_line_belongs_to() {
    let mut fake = Fake::new();
    fake.add("two", "1.0.0", "MIT AND ISC");
    fake.package("two").authors = vec!["Authors Of Two".into()];
    fake.write(
        "crates/mercury-credits/credits.toml",
        "[[clarify]]\ncrate = \"two\"\nlicence = \"ISC\"\ncopyright = \"Copyright (c) 2001 Someone Else\"\n",
    );
    let collected = collected(&fake);
    let isc = with_ids(&collected, &["ISC"]);
    let mit = with_ids(&collected, &["MIT"]);
    assert!(
        isc[0].body.contains("Copyright (c) 2001 Someone Else"),
        "{}",
        isc[0].body
    );
    assert!(
        mit[0].body.contains("Copyright (c) Authors Of Two"),
        "{}",
        mit[0].body
    );
}

#[test]
fn clarify_can_name_the_licence_of_a_crate_that_declares_none() {
    let mut fake = Fake::new();
    fake.add_without_license("odd", "1.0.0");
    fake.write(
        "crates/mercury-credits/credits.toml",
        "[[clarify]]\ncrate = \"odd\"\nlicence = \"MIT\"\ncopyright = \"Copyright (c) the odd authors\"\n",
    );
    let collected = collected(&fake);
    assert_eq!(collected.crates[0].license, "MIT");
    assert_eq!(with_ids(&collected, &["MIT"]).len(), 1);
}

#[test]
fn an_and_expression_needs_a_text_for_every_term() {
    let mut fake = Fake::new();
    fake.add("pair", "2.0.0", "MIT AND ISC");
    fake.crate_file("pair", "LICENSE", MIT);
    fake.package("pair").authors = vec!["Pat".into()];
    let collected = collected(&fake);
    let mit = with_ids(&collected, &["MIT"]);
    let isc = with_ids(&collected, &["ISC"]);
    assert!(!mit[0].members[0].standard, "MIT comes from the file");
    assert!(
        isc[0].members[0].standard,
        "ISC has no file and comes from LICENSES/"
    );
    assert!(isc[0].body.contains("Copyright (c) Pat"));
    assert_eq!(collected.standard_text_crates.len(), 1);
}

#[test]
fn a_text_of_a_licence_mercury_does_not_use_is_left_out_and_counted() {
    let mut fake = Fake::new();
    fake.add(
        "r-efi-like",
        "5.3.0",
        "MIT OR Apache-2.0 OR LGPL-2.1-or-later",
    );
    fake.crate_file("r-efi-like", "LICENSE-MIT", MIT);
    fake.crate_file("r-efi-like", "LICENSE-APACHE", APACHE);
    fake.crate_file(
        "r-efi-like",
        "LICENSE-LGPL",
        "GNU LESSER GENERAL PUBLIC LICENSE\nVersion 2.1, February 1999\n\nCopyright (C) 1991, 1999 Free Software Foundation, Inc.",
    );
    let collected = collected(&fake);
    assert_eq!(collected.blocks.len(), 2, "MIT and Apache only");
    assert!(
        collected
            .blocks
            .iter()
            .all(|block| !block.body.contains("LESSER"))
    );
    let left_out = warnings_containing(&collected, "left out");
    assert_eq!(left_out.len(), 1);
    assert!(
        left_out[0].contains("LGPL-2.1-only (1 crates: r-efi-like 5.3.0)"),
        "{}",
        left_out[0]
    );
    assert!(left_out[0].starts_with("1 licence texts"));
}

#[test]
fn a_file_with_two_licences_is_credited_under_every_taken_one_and_kept_whole() {
    let mut fake = Fake::new();
    fake.add("combo", "1.0.0", "ISC AND (Apache-2.0 OR ISC) AND MIT");
    fake.crate_file(
        "combo",
        "LICENSE",
        &format!("{ISC}\n\n---\n\n{APACHE}\n\n---\n\n{MIT}"),
    );
    let collected = collected(&fake);
    assert_eq!(collected.blocks.len(), 1);
    assert_eq!(collected.blocks[0].ids, ["Apache-2.0", "ISC", "MIT"]);
    assert!(collected.blocks[0].body.contains("TERMS AND CONDITIONS"));
    assert!(
        collected.standard_text_crates.is_empty(),
        "the one file covers every term"
    );
}

#[test]
fn the_slint_exception_credits_only_the_royalty_free_text() {
    let mut fake = Fake::new();
    fake.add(
        "slinty",
        "1.17.1",
        "GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0",
    );
    fake.crate_file(
        "slinty",
        "LICENSES/GPL-3.0-only.txt",
        "GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007\n",
    );
    fake.crate_file(
        "slinty",
        "LICENSES/LicenseRef-Slint-Royalty-free-2.0.md",
        "# Slint Royalty-free Desktop, Mobile, and Web Applications License\n\nVersion 2.0\n\n## Preamble\n",
    );
    fake.crate_file(
        "slinty",
        "LICENSES/LicenseRef-Slint-Software-3.0.md",
        "# Slint Software License\n\nVersion 3.0.5\n\n## Preamble\n",
    );
    let collected = collected(&fake);
    assert_eq!(collected.blocks.len(), 1);
    assert_eq!(
        collected.blocks[0].ids,
        ["LicenseRef-Slint-Royalty-free-2.0"]
    );
    let left_out = warnings_containing(&collected, "left out");
    assert!(
        left_out[0].starts_with("2 licence texts"),
        "{}",
        left_out[0]
    );
    // A crate that is not in the exception list cannot use the same expression.
    let mut other = Fake::new();
    other.add(
        "not-slint",
        "1.0.0",
        "GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0",
    );
    let outcome = other.outcome();
    assert!(!outcome.ok());
    assert!(
        outcome.report.problems[0].contains("not-slint 1.0.0: no licence of"),
        "{:?}",
        outcome.report.problems
    );
}

#[test]
fn notice_and_patents_files_are_always_kept_in_blocks_of_their_own() {
    let mut fake = Fake::new();
    fake.add("apachey", "1.0.0", "Apache-2.0");
    fake.crate_file("apachey", "LICENSE", APACHE);
    fake.crate_file("apachey", "NOTICE", "Apachey\nCopyright 2020 Jane Roe\n");
    fake.crate_file(
        "apachey",
        "PATENTS",
        "Additional IP Rights Grant (Patents)\n",
    );
    let collected = collected(&fake);
    let kinds: Vec<BlockKind> = collected.blocks.iter().map(|block| block.kind).collect();
    assert_eq!(
        kinds,
        [BlockKind::License, BlockKind::Notice, BlockKind::Patents]
    );
    assert!(collected.blocks[1].body.starts_with("Apachey"));
    assert!(collected.blocks[1].ids.is_empty());
    assert!(collected.standard_text_crates.is_empty());
}

#[test]
fn texts_that_differ_only_in_line_ends_and_trailing_space_are_one_block() {
    let mut fake = Fake::new();
    fake.add("one", "1.0.0", "MIT");
    fake.add("two", "2.0.0", "MIT");
    fake.add("three", "3.0.0", "MIT");
    fake.crate_file("one", "LICENSE", MIT);
    fake.crate_file("two", "LICENSE", &MIT.replace('\n', "\r\n"));
    fake.crate_file(
        "three",
        "LICENSE",
        &format!(
            "\n\n{}\n\n   \n",
            MIT.replace("Software.\n", "Software.   \n")
        ),
    );
    let collected = collected(&fake);
    assert_eq!(collected.blocks.len(), 1);
    assert_eq!(
        labels(&collected.blocks[0]),
        ["one 1.0.0", "three 3.0.0", "two 2.0.0"]
    );
}

#[test]
fn a_statement_file_is_kept_but_does_not_stand_in_for_the_licence_text() {
    let mut fake = Fake::new();
    fake.add("pointer", "1.0.0", "MIT OR Apache-2.0");
    fake.crate_file("pointer", "LICENSE", "MIT OR Apache-2.0\n");
    fake.crate_file(
        "pointer",
        "COPYRIGHT",
        "This project is dual-licensed under the Apache License and MIT.\n",
    );
    let collected = collected(&fake);
    assert!(
        collected
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::Statement)
    );
    assert_eq!(
        with_ids(&collected, &["MIT"]).len(),
        1,
        "the standard text is there too"
    );
    assert_eq!(with_ids(&collected, &["Apache-2.0"]).len(), 1);
    assert_eq!(collected.standard_text_crates.len(), 1);
}

#[test]
fn an_unknown_text_is_kept_and_warned_about_unless_a_person_accepted_the_crate() {
    let custom = "Weird licence\n\nYou may use this software for any purpose, provided you keep this line.\n";
    let mut fake = Fake::new();
    fake.add("weird", "1.0.0", "MIT");
    fake.crate_file("weird", "LICENSE", custom);
    let before = collected(&fake);
    assert!(
        before
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::Unclassified
                && block.body.starts_with("Weird licence"))
    );
    let warned = warnings_containing(&before, "wording the tool does not know");
    assert_eq!(warned.len(), 1);
    assert!(warned[0].contains("weird 1.0.0: LICENSE"));
    assert_eq!(
        before.standard_text_crates.len(),
        1,
        "unaccepted, it does not stand in for MIT"
    );

    fake.write(
        "crates/mercury-credits/credits.toml",
        "[accept_unclassified]\ncrates = [\"weird\"]\n",
    );
    let after = collected(&fake);
    assert!(warnings_containing(&after, "wording the tool does not know").is_empty());
    assert!(
        after.standard_text_crates.is_empty(),
        "accepted, it covers the crate's licence"
    );
    assert!(
        after
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::Unclassified)
    );
}

#[test]
fn extra_files_are_credited_with_their_note_and_name_the_path() {
    let mut fake = Fake::new();
    fake.add("native-sys", "1.0.0", "MIT");
    fake.crate_file("native-sys", "LICENSE", MIT);
    fake.crate_file("native-sys", "deps/lz4/LICENSE", &common::BSD3);
    fake.write(
        "crates/mercury-credits/credits.toml",
        "[[extra]]\ncrate = \"native-sys\"\nfiles = [\"deps/lz4/LICENSE\"]\nnote = \"LZ4 (deps/lz4)\"\n",
    );
    let collected = collected(&fake);
    let bsd = with_ids(&collected, &["BSD-3-Clause"]);
    assert_eq!(
        bsd.len(),
        1,
        "BSD-3-Clause is on the allow-list of the fake repository"
    );
    let member = &bsd[0].members[0];
    assert_eq!(member.via, ["deps/lz4/LICENSE"]);
    assert_eq!(member.notes, ["LZ4 (deps/lz4)"]);
    let mit = with_ids(&collected, &["MIT"]);
    assert!(
        mit[0].members[0].via.is_empty(),
        "the crate's own file is not an extra"
    );
}

#[test]
fn an_extra_with_a_licence_off_the_allow_list_is_credited_and_reported() {
    let mut fake = Fake::new();
    fake.add("data", "1.0.0", "MIT");
    fake.crate_file("data", "LICENSE", MIT);
    fake.crate_file(
        "data",
        "tables/LICENSE-UNICODE",
        "UNICODE, INC. LICENSE AGREEMENT - DATA FILES AND SOFTWARE\n\nSee Terms of Use",
    );
    fake.write(
        "crates/mercury-credits/credits.toml",
        "[[extra]]\ncrate = \"data\"\nfiles = [\"tables/LICENSE-UNICODE\"]\n",
    );
    let collected = collected(&fake);
    assert!(
        collected
            .blocks
            .iter()
            .any(|block| block.ids == ["Unicode-DFS-2016"])
    );
    let warned = warnings_containing(&collected, "not on deny.toml's allow-list");
    assert_eq!(warned.len(), 1);
    assert!(warned[0].contains("data 1.0.0: tables/LICENSE-UNICODE (Unicode-DFS-2016)"));
}

#[test]
fn ignore_leaves_a_crate_out_and_says_so() {
    let mut fake = Fake::new();
    fake.add("tool", "1.0.0", "MIT");
    fake.add("kept", "1.0.0", "MIT");
    fake.crate_file("kept", "LICENSE", MIT);
    fake.write(
        "crates/mercury-credits/credits.toml",
        "[[ignore]]\ncrate = \"tool\"\nreason = \"a build tool; none of its code is in the output\"\n",
    );
    let collected = collected(&fake);
    assert_eq!(collected.crates.len(), 1);
    assert_eq!(collected.crates[0].krate.name, "kept");
    assert_eq!(collected.ignored.len(), 1);
    assert_eq!(collected.ignored[0].0.name, "tool");
    let warned = warnings_containing(&collected, "tool 1.0.0 is not credited");
    assert_eq!(warned.len(), 1);
    assert!(warned[0].contains("a build tool"));
    assert_eq!(collected.credited_count(), 2, "kept and the app itself");
}

#[test]
fn a_config_entry_for_a_crate_that_is_not_in_the_app_is_a_warning() {
    let mut fake = Fake::new();
    fake.add("here", "1.0.0", "MIT");
    fake.crate_file("here", "LICENSE", MIT);
    fake.write(
        "crates/mercury-credits/credits.toml",
        "[[extra]]\ncrate = \"gone\"\nfiles = [\"x\"]\n[[clarify]]\ncrate = \"gone2\"\ncopyright = \"c\"\n[[ignore]]\ncrate = \"gone3\"\nreason = \"r\"\n[accept_unclassified]\ncrates = [\"gone4\", \"here\"]\n",
    );
    let collected = collected(&fake);
    let warned = warnings_containing(&collected, "not in the app");
    assert_eq!(warned.len(), 1);
    for expected in [
        "[[extra]] gone",
        "[[clarify]] gone2",
        "[[ignore]] gone3",
        "[accept_unclassified] gone4",
    ] {
        assert!(warned[0].contains(expected), "{expected} in {}", warned[0]);
    }
    assert!(!warned[0].contains("here"), "{}", warned[0]);
}

#[test]
fn vendored_crates_carry_their_vendor_record_and_licence_note_under_every_text() {
    let mut fake = Fake::new();
    fake.add_vendored("servo-config", "0.8.0", "MPL-2.0");
    fake.add_vendored("i-slint-backend-winit", "1.17.1", "MIT");
    fake.crate_file("i-slint-backend-winit", "LICENSE", MIT);
    fake.write(
        "third_party/vendor.toml",
        "[[crate]]\nname = \"servo-config\"\nversion = \"0.8.0\"\nrepository = \"https://github.com/servo/servo\"\ncommit = \"abc123\"\npath = \"components/config\"\nlicense = \"MPL-2.0\"\n\n[[crate]]\nname = \"i-slint-backend-winit\"\nversion = \"1.17.1\"\nrepository = \"https://github.com/slint-ui/slint\"\ncommit = \"def456\"\nlicense = \"MIT\"\nlicense_note = \"Used under the Royalty-free licence.\"\n",
    );
    let collected = collected(&fake);
    let kinds: Vec<(String, Kind)> = collected
        .crates
        .iter()
        .map(|krate| (krate.krate.name.clone(), krate.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("i-slint-backend-winit".to_owned(), Kind::Vendored),
            ("servo-config".to_owned(), Kind::Vendored)
        ]
    );
    let servo = collected
        .crates
        .iter()
        .find(|krate| krate.krate.name == "servo-config")
        .unwrap();
    assert!(servo.requires_mpl);
    assert_eq!(
        servo.vendor.as_ref().unwrap().commit.as_deref(),
        Some("abc123")
    );
    let slint = collected
        .blocks
        .iter()
        .find(|block| block.ids == ["MIT"])
        .unwrap();
    assert_eq!(
        slint.members[0].notes,
        ["Used under the Royalty-free licence."]
    );
    assert_eq!(collected.vendored_count(), 2);
    assert_eq!(collected.registry_count(), 0);
}

#[test]
fn two_versions_of_a_crate_are_credited_separately_and_the_tree_may_repeat_lines() {
    let mut fake = Fake::new();
    fake.add("twice", "1.0.0", "MIT");
    fake.add("twice", "2.0.0", "MIT");
    fake.crate_file("twice", "LICENSE", MIT);
    let metadata = fake.metadata();
    let tree = format!(
        "{}twice v1.0.0 (*)\ntwice v2.0.0 (*)\ntwice v2.0.0 (proc-macro)\n",
        fake.tree_text()
    );
    let outcome = mercury_credits::app::analyse_inputs(&mercury_credits::collect::Inputs {
        root: fake.root(),
        tree: &tree,
        metadata: &metadata,
        target: "t",
        runtime_only: true,
    });
    assert!(outcome.ok(), "{:?}", outcome.report.problems);
    let collected = outcome.collected.unwrap();
    assert!(collected.runtime_only);
    let versions: Vec<&str> = collected
        .crates
        .iter()
        .map(|krate| krate.krate.version.as_str())
        .collect();
    assert_eq!(versions, ["1.0.0", "2.0.0"]);
}

#[test]
fn own_crates_are_counted_and_not_credited_one_by_one() {
    let mut fake = Fake::new();
    fake.add_own("mercury-store", "9.9.9");
    fake.add("dep", "1.0.0", "MIT");
    fake.crate_file("dep", "LICENSE", MIT);
    let collected = collected(&fake);
    let own: Vec<&str> = collected
        .own
        .iter()
        .map(|krate| krate.name.as_str())
        .collect();
    assert_eq!(own, ["app", "mercury-store"]);
    assert_eq!(collected.credited_count(), 3);
    assert_eq!(collected.crates.len(), 1);
}

#[test]
fn crates_that_link_native_code_without_an_extra_entry_are_listed_as_a_warning() {
    let mut fake = Fake::new();
    fake.add("zlib-sys", "1.0.0", "MIT");
    fake.add("checked-sys", "1.0.0", "MIT");
    fake.crate_file("zlib-sys", "LICENSE", MIT);
    fake.crate_file("checked-sys", "LICENSE", MIT);
    fake.package("zlib-sys").links = Some("z".into());
    fake.package("checked-sys").links = Some("checked".into());
    fake.write(
        "crates/mercury-credits/credits.toml",
        "[[extra]]\ncrate = \"checked-sys\"\nfiles = []\n",
    );
    let collected = collected(&fake);
    let warned = warnings_containing(&collected, "link native code");
    assert_eq!(warned.len(), 1);
    assert!(
        warned[0].contains("zlib-sys 1.0.0") && !warned[0].contains("checked-sys"),
        "{}",
        warned[0]
    );
}

#[test]
fn the_llvm_exception_is_covered_by_its_own_text_or_by_a_plain_alternative() {
    // A text that holds the exception covers the term.
    let mut fake = Fake::new();
    fake.add("llvm", "1.0.0", "Apache-2.0 WITH LLVM-exception");
    fake.crate_file(
        "llvm",
        "LICENSE",
        &format!(
            "{APACHE}\n\n--- LLVM Exceptions to the Apache 2.0 License ----\n\nAs an exception, ..."
        ),
    );
    let collected_exception = collected(&fake);
    assert!(collected_exception.standard_text_crates.is_empty());

    // With the plain licence as an alternative, the plain text is enough.
    let mut plain = Fake::new();
    plain.add(
        "alt",
        "1.0.0",
        "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT",
    );
    plain.crate_file("alt", "LICENSE-APACHE", APACHE);
    let collected_plain = collected(&plain);
    assert!(collected_plain.standard_text_crates.is_empty());
    assert_eq!(collected_plain.blocks[0].ids, ["Apache-2.0"]);

    // Relying on the exception alone with only the plain text needs the exception's standard
    // text, which this repository does not have.
    let mut alone = Fake::new();
    alone.add("only", "1.0.0", "Apache-2.0 WITH LLVM-exception");
    alone.crate_file("only", "LICENSE-APACHE", APACHE);
    let outcome = alone.outcome();
    assert!(!outcome.ok());
    assert!(
        outcome.report.problems[0].contains("LICENSES/LLVM-exception.txt is missing"),
        "{:?}",
        outcome.report.problems
    );
    alone.write(
        "LICENSES/LLVM-exception.txt",
        "---- LLVM Exceptions to the Apache 2.0 License ----\n\nAs an exception, if ...\n",
    );
    let fixed = collected(&alone);
    let block = fixed
        .blocks
        .iter()
        .find(|block| block.ids == ["Apache-2.0 WITH LLVM-exception"])
        .expect("the exception text");
    assert!(
        block
            .body
            .contains("LLVM Exceptions to the Apache 2.0 License")
    );
    assert!(block.members[0].standard);
}
