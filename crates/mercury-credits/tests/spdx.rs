// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! SPDX parsing and the taken-branch rule, with the expressions that really occur in the lock file.
use mercury_credits::spdx::{self, Expr, TakenSet, Term};

/// The allow-list of `deny.toml`.
fn allow_list() -> TakenSet {
    TakenSet::new([
        "Apache-2.0",
        "Apache-2.0 WITH LLVM-exception",
        "BSD-2-Clause",
        "BSD-3-Clause",
        "BSL-1.0",
        "CC0-1.0",
        "CDLA-Permissive-2.0",
        "ISC",
        "MIT",
        "MPL-2.0",
        "NCSA",
        "Unicode-3.0",
        "Zlib",
    ])
}

fn credited(expression: &str, taken: &TakenSet) -> Option<Vec<String>> {
    spdx::parse(expression).unwrap().credited(taken)
}

fn strings(items: &[&str]) -> Option<Vec<String>> {
    Some(items.iter().map(|item| (*item).to_owned()).collect())
}

/// Every licence expression of every crate in Cargo.lock on 19 November 2026.
const CORPUS: &[&str] = &[
    "(Apache-2.0 OR MIT) AND BSD-3-Clause",
    "(MIT OR Apache-2.0) AND NCSA",
    "(MIT OR Apache-2.0) AND Unicode-3.0",
    "0BSD OR MIT OR Apache-2.0",
    "Apache-2.0",
    "Apache-2.0 / MIT",
    "Apache-2.0 AND ISC",
    "Apache-2.0 AND MIT",
    "Apache-2.0 OR BSL-1.0",
    "Apache-2.0 OR ISC OR MIT",
    "Apache-2.0 OR MIT",
    "Apache-2.0 WITH LLVM-exception",
    "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT",
    "Apache-2.0/MIT",
    "BSD-2-Clause",
    "BSD-2-Clause OR Apache-2.0 OR MIT",
    "BSD-2-Clause OR MIT OR Apache-2.0",
    "BSD-3-Clause",
    "BSD-3-Clause AND BSD-2-Clause",
    "BSD-3-Clause AND MIT",
    "BSD-3-Clause OR Apache-2.0",
    "BSD-3-Clause OR MIT OR Apache-2.0",
    "BSD-3-Clause/MIT",
    "BSL-1.0",
    "CC0-1.0",
    "CC0-1.0 OR Apache-2.0",
    "CC0-1.0 OR MIT-0 OR Apache-2.0",
    "CDLA-Permissive-2.0",
    "GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0",
    "ISC",
    "ISC AND (Apache-2.0 OR ISC)",
    "ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0)",
    "MIT",
    "MIT / Apache-2.0",
    "MIT OR Apache-2.0",
    "MIT OR Apache-2.0 OR BSD-1-Clause",
    "MIT OR Apache-2.0 OR LGPL-2.1-or-later",
    "MIT OR Apache-2.0 OR MPL-2.0",
    "MIT OR Apache-2.0 OR Zlib",
    "MIT OR Zlib OR Apache-2.0",
    "MIT/Apache-2.0",
    "MPL-2.0",
    "MPL-2.0 AND BSD-3-Clause",
    "MPL-2.0 AND PolyForm-Noncommercial-1.0.0",
    "PolyForm-Noncommercial-1.0.0",
    "Unicode-3.0",
    "Unlicense OR MIT",
    "Unlicense/MIT",
    "Zlib",
    "Zlib OR Apache-2.0 OR MIT",
];

#[test]
fn every_expression_in_the_lock_file_parses() {
    assert!(CORPUS.len() >= 50, "the corpus was cut short");
    for expression in CORPUS {
        spdx::parse(expression).unwrap_or_else(|error| panic!("{error}"));
    }
}

#[test]
fn every_expression_in_the_lock_file_gets_an_answer_for_the_app() {
    // The expressions of the crates in the app must be satisfiable under deny.toml; the
    // expressions of crates that are not in the app (BSD-1-Clause, 0BSD ones) may name other
    // licences but all offer a taken branch here too, except Slint's, which needs its exception.
    let mut taken = allow_list();
    for expression in CORPUS {
        let parsed = spdx::parse(expression).unwrap();
        let answer = parsed.credited(&taken);
        let slint = expression.contains("LicenseRef-Slint");
        let polyform = expression.contains("PolyForm");
        if slint || polyform {
            continue;
        }
        assert!(answer.is_some(), "{expression} has no taken branch");
    }
    taken.insert("LicenseRef-Slint-Royalty-free-2.0");
    let slint =
        "GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0";
    assert_eq!(
        credited(slint, &taken),
        strings(&["LicenseRef-Slint-Royalty-free-2.0"])
    );
}

#[test]
fn or_credits_every_taken_branch_in_sorted_order() {
    let taken = allow_list();
    assert_eq!(
        credited("MIT OR Apache-2.0", &taken),
        strings(&["Apache-2.0", "MIT"])
    );
    assert_eq!(
        credited("Zlib OR Apache-2.0 OR MIT", &taken),
        strings(&["Apache-2.0", "MIT", "Zlib"])
    );
    assert_eq!(
        credited("Unlicense OR MIT", &taken),
        strings(&["MIT"]),
        "Unlicense is not on the allow-list"
    );
}

#[test]
fn the_lgpl_branch_is_never_credited() {
    let taken = allow_list();
    assert_eq!(
        credited("MIT OR Apache-2.0 OR LGPL-2.1-or-later", &taken),
        strings(&["Apache-2.0", "MIT"])
    );
}

#[test]
fn and_needs_every_term() {
    let taken = allow_list();
    assert_eq!(
        credited("(MIT OR Apache-2.0) AND Unicode-3.0", &taken),
        strings(&["Apache-2.0", "MIT", "Unicode-3.0"])
    );
    assert_eq!(credited("MIT AND GPL-3.0-only", &taken), None);
    assert_eq!(
        credited("(MIT OR Apache-2.0) AND NCSA", &taken),
        strings(&["Apache-2.0", "MIT", "NCSA"])
    );
}

#[test]
fn the_aws_lc_expression_credits_every_taken_term() {
    let taken = allow_list();
    let expression = "ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0)";
    assert_eq!(
        credited(expression, &taken),
        strings(&["Apache-2.0", "BSD-3-Clause", "ISC", "MIT"])
    );
}

#[test]
fn none_taken_is_no_answer() {
    let taken = allow_list();
    assert_eq!(credited("GPL-3.0-only", &taken), None);
    assert_eq!(credited("0BSD OR GPL-2.0-or-later", &taken), None);
    assert_eq!(credited("PolyForm-Noncommercial-1.0.0", &taken), None);
}

#[test]
fn slint_needs_its_exception_and_credits_only_the_royalty_free_branch() {
    let slint =
        "GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0";
    let global = allow_list();
    assert_eq!(
        credited(slint, &global),
        None,
        "without the exception nothing is taken"
    );
    let mut for_slint = allow_list();
    for_slint.insert("LicenseRef-Slint-Royalty-free-2.0");
    assert_eq!(
        credited(slint, &for_slint),
        strings(&["LicenseRef-Slint-Royalty-free-2.0"])
    );
}

#[test]
fn with_is_part_of_the_term() {
    let expr = spdx::parse("Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT").unwrap();
    let Expr::Or(branches) = &expr else {
        panic!("{expr:?}")
    };
    assert_eq!(
        branches[0],
        Expr::Term(Term {
            id: "Apache-2.0".into(),
            exception: Some("LLVM-exception".into())
        })
    );
    assert_eq!(
        branches[0].terms()[0].key(),
        "Apache-2.0 WITH LLVM-exception"
    );
    let taken = allow_list();
    assert_eq!(
        expr.credited(&taken),
        strings(&["Apache-2.0", "Apache-2.0 WITH LLVM-exception", "MIT"])
    );
    // With only the plain licence taken, the exception term is not.
    let plain_only = TakenSet::new(["Apache-2.0", "MIT"]);
    assert_eq!(expr.credited(&plain_only), strings(&["Apache-2.0", "MIT"]));
    let only_with = spdx::parse("Apache-2.0 WITH LLVM-exception").unwrap();
    assert_eq!(only_with.credited(&plain_only), None);
    assert_eq!(
        only_with.credited(&allow_list()),
        strings(&["Apache-2.0 WITH LLVM-exception"])
    );
}

#[test]
fn the_old_slash_means_or() {
    let taken = allow_list();
    for expression in [
        "MIT/Apache-2.0",
        "MIT / Apache-2.0",
        "Apache-2.0/MIT",
        "Apache-2.0 / MIT",
    ] {
        assert_eq!(
            credited(expression, &taken),
            strings(&["Apache-2.0", "MIT"]),
            "{expression}"
        );
    }
    assert_eq!(
        spdx::parse("Unlicense/MIT").unwrap(),
        spdx::parse("Unlicense OR MIT").unwrap()
    );
}

#[test]
fn operators_are_case_insensitive_and_precedence_is_spdx() {
    let lower = spdx::parse("mit or apache-2.0 and isc").unwrap();
    // AND binds tighter than OR: MIT OR (Apache-2.0 AND ISC).
    let Expr::Or(branches) = &lower else {
        panic!("{lower:?}")
    };
    assert_eq!(branches.len(), 2);
    assert!(matches!(branches[1], Expr::And(_)));
    let only_apache = TakenSet::new(["Apache-2.0"]);
    assert_eq!(
        lower.credited(&only_apache),
        None,
        "Apache-2.0 alone does not satisfy the AND"
    );
    let taken = TakenSet::new(["Apache-2.0", "ISC"]);
    assert_eq!(lower.credited(&taken), strings(&["Apache-2.0", "ISC"]));
    let grouped = spdx::parse("(MIT OR Apache-2.0) AND ISC").unwrap();
    assert!(matches!(grouped, Expr::And(_)));
}

#[test]
fn matching_ignores_case_and_credits_the_allow_list_spelling() {
    let taken = allow_list();
    assert_eq!(
        credited("mit OR apache-2.0", &taken),
        strings(&["Apache-2.0", "MIT"])
    );
    assert_eq!(
        credited("apache-2.0 with llvm-exception", &taken),
        strings(&["Apache-2.0 WITH LLVM-exception"])
    );
}

#[test]
fn license_refs_and_plus_are_identifiers() {
    let expr = spdx::parse("LicenseRef-Foo-1.0 OR GPL-2.0+ OR DocumentRef-x:LicenseRef-y").unwrap();
    let ids: Vec<String> = expr.terms().iter().map(|term| term.id.clone()).collect();
    assert_eq!(
        ids,
        [
            "LicenseRef-Foo-1.0",
            "GPL-2.0+",
            "DocumentRef-x:LicenseRef-y"
        ]
    );
}

#[test]
fn bad_expressions_are_errors_that_name_the_expression() {
    for bad in [
        "",
        "   ",
        "MIT OR",
        "OR MIT",
        "MIT AND AND ISC",
        "(MIT OR ISC",
        "MIT OR ISC)",
        "MIT WITH",
        "(MIT OR ISC) WITH Exception",
        "MIT ISC",
        "MIT OR *",
        "MIT / ",
        "()",
    ] {
        let error = spdx::parse(bad).unwrap_err();
        assert_eq!(error.expression, bad, "{bad:?}");
        assert!(
            error.to_string().contains("invalid licence expression"),
            "{bad:?}"
        );
    }
}

#[test]
fn requires_mpl_when_it_is_the_only_taken_branch_or_an_and_term() {
    let taken = allow_list();
    let requires = |expression: &str| spdx::parse(expression).unwrap().requires(&taken, "MPL-2.0");
    assert!(requires("MPL-2.0"));
    assert!(requires("MPL-2.0 AND BSD-3-Clause"));
    assert!(requires("BSD-3-Clause AND MPL-2.0"));
    assert!(
        requires("MPL-2.0 OR GPL-2.0-or-later"),
        "the GPL branch is not taken"
    );
    assert!(
        !requires("MIT OR Apache-2.0 OR MPL-2.0"),
        "MIT is an alternative"
    );
    assert!(!requires("MIT OR Apache-2.0"));
    assert!(!requires("MIT"));
    assert!(!requires("(MPL-2.0 OR MIT) AND Unicode-3.0"));
    assert!(requires("(MPL-2.0 OR GPL-3.0-only) AND Unicode-3.0"));
    assert!(
        !requires("GPL-3.0-only"),
        "an expression that cannot be satisfied requires nothing"
    );
}

#[test]
fn satisfied_takes_any_predicate() {
    let expr = spdx::parse("A AND (B OR C)").unwrap();
    assert!(expr.satisfied(&|term| term.id == "A" || term.id == "C"));
    assert!(!expr.satisfied(&|term| term.id == "B" || term.id == "C"));
    assert!(!expr.satisfied(&|term| term.id == "A"));
}

#[test]
fn taken_set_keeps_the_first_spelling() {
    let mut taken = TakenSet::new(["MIT"]);
    taken.insert("mit");
    taken.insert("Apache-2.0   WITH   LLVM-exception");
    assert_eq!(taken.len(), 2);
    assert_eq!(taken.spelling("MIT"), Some("MIT"));
    assert_eq!(
        taken.spelling("apache-2.0 with llvm-exception"),
        Some("Apache-2.0 WITH LLVM-exception")
    );
    assert!(taken.contains_key("Mit"));
    assert!(!taken.contains_key("ISC"));
    assert!(!taken.is_empty());
}
