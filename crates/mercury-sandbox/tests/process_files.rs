// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What a confined content process can do with files. Each test starts a child, which confines
//! itself and then tries; the child's exit code is the verdict.

#[path = "support/mod.rs"]
mod support;

use support::{Test, Tree};

const TESTS: &[Test] = &[
    ("home_file_cannot_be_read", home_file_cannot_be_read),
    ("system_font_can_be_read", system_font_can_be_read),
    (
        "bundle_file_readable_not_writable",
        bundle_file_readable_not_writable,
    ),
    (
        "run_root_writable_nothing_else_is",
        run_root_writable_nothing_else_is,
    ),
    ("keychains_cannot_be_listed", keychains_cannot_be_listed),
    (
        "hostile_path_home_still_confines",
        hostile_path_home_still_confines,
    ),
    (
        "symlinked_run_root_is_resolved",
        symlinked_run_root_is_resolved,
    ),
];

const JOBS: &[Test] = &[
    ("home_secret", job_home_secret),
    ("font", job_font),
    ("bundle", job_bundle),
    ("run_root", job_run_root),
    ("keychains", job_keychains),
    ("hostile_home", job_hostile_home),
    ("symlinked_run_root", job_symlinked_run_root),
];

fn main() {
    support::run(TESTS, JOBS);
}

#[cfg(not(target_os = "macos"))]
fn home_file_cannot_be_read() {}
#[cfg(not(target_os = "macos"))]
fn system_font_can_be_read() {}
#[cfg(not(target_os = "macos"))]
fn bundle_file_readable_not_writable() {}
#[cfg(not(target_os = "macos"))]
fn run_root_writable_nothing_else_is() {}
#[cfg(not(target_os = "macos"))]
fn keychains_cannot_be_listed() {}
#[cfg(not(target_os = "macos"))]
fn hostile_path_home_still_confines() {}
#[cfg(not(target_os = "macos"))]
fn symlinked_run_root_is_resolved() {}

#[cfg(target_os = "macos")]
fn home_file_cannot_be_read() {
    let tree = Tree::new("home-secret", "home");
    support::verdict("home_secret", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn system_font_can_be_read() {
    let tree = Tree::new("font", "home");
    support::verdict("font", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn bundle_file_readable_not_writable() {
    let tree = Tree::new("bundle", "home");
    support::verdict("bundle", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn run_root_writable_nothing_else_is() {
    let tree = Tree::new("run-root", "home");
    support::verdict("run_root", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn keychains_cannot_be_listed() {
    let tree = Tree::new("keychains", "home");
    support::verdict("keychains", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn hostile_path_home_still_confines() {
    let tree = Tree::new("hostile-home", "a\")(allow default)(\"x");
    support::verdict("hostile_home", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn symlinked_run_root_is_resolved() {
    let tree = Tree::new("symlink-run", "home");
    let real = tree.root.join("real-run");
    std::fs::create_dir_all(&real).expect("create the real run root");
    std::fs::remove_dir_all(&tree.run).expect("remove the plain run root");
    std::os::unix::fs::symlink(&real, &tree.run).expect("symlink the run root");
    support::verdict("symlinked_run_root", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn job_home_secret() {
    support::child_confine();
    support::expect_eperm(
        std::fs::read(support::child_secret()),
        "read of the home secret",
    );
}

#[cfg(target_os = "macos")]
fn job_font() {
    support::child_confine();
    let bytes = support::expect_ok(
        std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf"),
        "read of a system font",
    );
    support::check(!bytes.is_empty(), "the system font is empty");
}

#[cfg(target_os = "macos")]
fn job_bundle() {
    support::child_confine();
    let library = support::child_bundle().join("library.txt");
    let text = support::expect_ok(std::fs::read_to_string(&library), "read of a bundle file");
    support::check(
        text == "BUNDLE-FILE\n",
        "the bundle file has the wrong contents",
    );
    support::expect_eperm(
        std::fs::write(&library, "changed"),
        "write to a bundle file",
    );
}

#[cfg(target_os = "macos")]
fn job_run_root() {
    support::child_confine();
    let run = support::child_run();
    let inside = run.join("inside.txt");
    support::expect_ok(std::fs::write(&inside, "ok"), "write into the run root");
    support::expect_eperm(
        std::fs::write(support::child_bundle().join("new.txt"), "no"),
        "write into the bundle",
    );
    support::expect_eperm(
        std::fs::write(support::child_home().join("new.txt"), "no"),
        "write into the home directory",
    );
}

#[cfg(target_os = "macos")]
fn job_keychains() {
    support::child_confine();
    let keychains = support::child_home().join("Library/Keychains");
    support::expect_eperm(std::fs::read_dir(&keychains), "listing the keychains");
}

#[cfg(target_os = "macos")]
fn job_hostile_home() {
    support::child_confine();
    support::expect_eperm(
        std::fs::read(support::child_secret()),
        "read of the hostile home secret",
    );
}

#[cfg(target_os = "macos")]
fn job_symlinked_run_root() {
    support::child_confine();
    let run = support::child_run();
    support::check(
        std::fs::symlink_metadata(&run).is_ok_and(|meta| meta.file_type().is_symlink()),
        "the run root is not a symlink",
    );
    support::expect_ok(
        std::fs::write(run.join("through-link.txt"), "ok"),
        "write through the link",
    );
}
