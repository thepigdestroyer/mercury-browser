// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What happens when confinement cannot be done, or cannot be trusted. Parameter checks are pure
//! and run here; anything that applies a sandbox runs in a child, which is the process that gets
//! confined.

#[path = "support/mod.rs"]
mod support;

use std::path::PathBuf;

use mercury_sandbox::{
    EXIT_CODE_UNCONFINED, EnterError, Profile, ProfileError, ProfileInputs, confine,
    content_profile, report_profile,
};
use support::{Test, Tree};

const TESTS: &[Test] = &[
    (
        "parameters_must_be_absolute_and_resolved",
        parameters_must_be_absolute_and_resolved,
    ),
    (
        "broken_profile_exits_70_without_reading_the_secret",
        broken_profile_exits_70_without_reading_the_secret,
    ),
    (
        "canary_catches_an_allow_all_profile",
        canary_catches_an_allow_all_profile,
    ),
    (
        "second_confinement_is_refused",
        second_confinement_is_refused,
    ),
    (
        "report_mode_is_announced_as_unconfined",
        report_mode_is_announced_as_unconfined,
    ),
];

const JOBS: &[Test] = &[
    ("broken", job_broken),
    ("allow_all", job_allow_all),
    ("twice", job_twice),
    ("report", job_report),
];

fn main() {
    support::run(TESTS, JOBS);
}

fn inputs() -> ProfileInputs {
    ProfileInputs::new(
        PathBuf::from("/Applications/Mercury.app"),
        PathBuf::from("/Applications/Mercury.app/Contents/MacOS/mercury-browser"),
        PathBuf::from("/Users/someone"),
    )
}

fn parameters_must_be_absolute_and_resolved() {
    let mut relative = inputs();
    relative.home = PathBuf::from("Users/someone");
    assert_eq!(
        content_profile(&relative),
        Err(ProfileError::NotAbsolute("HOMEDIR"))
    );

    let mut dotdot = inputs();
    dotdot.bundle_path = PathBuf::from("/Applications/../etc");
    assert_eq!(
        content_profile(&dotdot),
        Err(ProfileError::ParentComponent("BUNDLE_PATH"))
    );

    let mut root = inputs();
    root.home = PathBuf::from("/");
    assert_eq!(content_profile(&root), Err(ProfileError::Root("HOMEDIR")));

    let mut empty = inputs();
    empty.executable = PathBuf::new();
    assert_eq!(
        content_profile(&empty),
        Err(ProfileError::Empty("EXECUTABLE_PATH"))
    );

    let mut run_root = inputs();
    run_root.run_root = Some(PathBuf::from("relative/run"));
    assert_eq!(
        content_profile(&run_root),
        Err(ProfileError::NotAbsolute("RUN_ROOT"))
    );
}

fn broken_profile_exits_70_without_reading_the_secret() {
    let tree = Tree::new("broken", "home");
    let (child, output) = support::spawn_child("broken", &tree, &[]);
    let outcome = support::finish(child, &output);
    assert_eq!(
        outcome.code,
        Some(EXIT_CODE_UNCONFINED),
        "a child with a broken profile must exit {EXIT_CODE_UNCONFINED}:\n{}",
        outcome.stdout
    );
    assert!(
        !outcome.stdout.contains("SECRET"),
        "the child read the secret before it was confined"
    );
}

fn canary_catches_an_allow_all_profile() {
    let tree = Tree::new("allow-all", "home");
    support::verdict("allow_all", &tree, &[]);
}

fn second_confinement_is_refused() {
    let tree = Tree::new("twice", "home");
    support::verdict("twice", &tree, &[]);
}

fn report_mode_is_announced_as_unconfined() {
    let tree = Tree::new("report", "home");
    support::verdict("report", &tree, &[]);
}

fn job_broken() {
    let profile = Profile {
        text: "(version 1)\n(allow (((\n".to_owned(),
        parameters: Vec::new(),
    };
    match confine(&profile) {
        Err(error) => {
            println!("REFUSED: {error}");
            mercury_sandbox::exit_unconfined()
        }
        Ok(_) => support::fail("a broken profile was accepted"),
    }
}

fn job_allow_all() {
    let profile = Profile {
        text: "(version 1)\n(allow default)\n".to_owned(),
        parameters: Vec::new(),
    };
    match confine(&profile) {
        Err(EnterError::CanaryFailed(_)) => {}
        Err(error) => support::fail(format!("the wrong refusal: {error}")),
        Ok(_) => support::fail("the canary did not catch an allow-all profile"),
    }
}

fn job_twice() {
    let profile = support::child_profile();
    support::child_confine();
    match confine(&profile) {
        Err(EnterError::AlreadyConfined) => {}
        Err(error) => support::fail(format!("the wrong refusal: {error}")),
        Ok(_) => support::fail("a second confinement was accepted"),
    }
}

fn job_report() {
    match confine(&report_profile()) {
        Ok(confinement) if confinement.report_mode => {}
        Ok(_) => support::fail("report mode did not say it is report mode"),
        Err(error) => support::fail(format!("report mode refused: {error}")),
    }
}
