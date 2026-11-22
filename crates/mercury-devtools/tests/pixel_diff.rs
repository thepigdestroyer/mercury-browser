// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `pixel-diff`: the comparison of two folders of pictures, and its executable's exit codes.

use std::fs;
use std::path::Path;
use std::process::Command;

use mercury_devtools::pixel_diff::{Outcome, compare_dirs};
use tempfile::TempDir;

/// A `width` x `height` picture of one colour, with `changes` as `(x, y, [r, g, b])`.
fn write(path: &Path, width: u32, height: u32, changes: &[(u32, u32, [u8; 3])]) {
    let mut picture = image::RgbImage::from_pixel(width, height, image::Rgb([10, 20, 30]));
    for &(x, y, colour) in changes {
        picture.put_pixel(x, y, image::Rgb(colour));
    }
    fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
    picture.save(path).expect("a picture");
}

fn folders() -> (TempDir, TempDir) {
    (
        tempfile::tempdir().expect("a folder"),
        tempfile::tempdir().expect("a folder"),
    )
}

#[test]
fn identical_folders_pass() {
    let (before, after) = folders();
    for root in [before.path(), after.path()] {
        write(&root.join("a.png"), 4, 3, &[]);
        write(&root.join("b.png"), 2, 2, &[(1, 1, [200, 0, 0])]);
    }
    let report = compare_dirs(before.path(), after.path(), 0).expect("compares");
    assert!(report.is_identical());
    assert_eq!(report.entries.len(), 2);
    assert_eq!(report.problems(), 0);
    assert!(
        report
            .to_string()
            .contains("2 files compared (tolerance 0): 2 identical, 0 not")
    );
}

#[test]
fn one_pixel_that_differs_is_counted_with_its_delta_and_box() {
    let (before, after) = folders();
    write(&before.path().join("a.png"), 5, 4, &[]);
    write(&after.path().join("a.png"), 5, 4, &[(3, 2, [10, 25, 30])]);
    let report = compare_dirs(before.path(), after.path(), 0).expect("compares");
    assert_eq!(
        report.entry("a.png").expect("an entry").outcome,
        Outcome::Differs {
            pixels: 1,
            max_delta: 5,
            bounds: (3, 2, 3, 2)
        }
    );
    assert!(!report.is_identical());
    assert!(
        report
            .to_string()
            .contains("DIFFERENT  a.png: 1 pixels differ")
    );
}

#[test]
fn the_box_covers_every_differing_pixel() {
    let (before, after) = folders();
    write(&before.path().join("a.png"), 8, 8, &[]);
    write(
        &after.path().join("a.png"),
        8,
        8,
        &[(1, 6, [0, 0, 0]), (5, 2, [10, 20, 90])],
    );
    let report = compare_dirs(before.path(), after.path(), 0).expect("compares");
    assert_eq!(
        report.entry("a.png").expect("an entry").outcome,
        Outcome::Differs {
            pixels: 2,
            max_delta: 60,
            bounds: (1, 2, 5, 6)
        }
    );
}

#[test]
fn a_changed_size_is_reported() {
    let (before, after) = folders();
    write(&before.path().join("a.png"), 4, 4, &[]);
    write(&after.path().join("a.png"), 4, 5, &[]);
    let report = compare_dirs(before.path(), after.path(), 0).expect("compares");
    assert_eq!(
        report.entry("a.png").expect("an entry").outcome,
        Outcome::SizeChanged {
            before: (4, 4),
            after: (4, 5)
        }
    );
    assert!(!report.is_identical());
}

#[test]
fn a_file_missing_on_either_side_is_a_difference() {
    let (before, after) = folders();
    write(&before.path().join("gone.png"), 2, 2, &[]);
    write(&after.path().join("new.png"), 2, 2, &[]);
    write(&before.path().join("both.png"), 2, 2, &[]);
    write(&after.path().join("both.png"), 2, 2, &[]);
    let report = compare_dirs(before.path(), after.path(), 0).expect("compares");
    assert_eq!(
        report.entry("gone.png").expect("an entry").outcome,
        Outcome::OnlyBefore
    );
    assert_eq!(
        report.entry("new.png").expect("an entry").outcome,
        Outcome::OnlyAfter
    );
    assert_eq!(
        report.entry("both.png").expect("an entry").outcome,
        Outcome::Identical
    );
    assert!(!report.is_identical());
}

#[test]
fn nested_folders_are_matched_by_relative_path_and_other_files_are_ignored() {
    let (before, after) = folders();
    write(&before.path().join("one/two/a.png"), 3, 3, &[]);
    write(
        &after.path().join("one/two/a.png"),
        3,
        3,
        &[(0, 0, [0, 0, 0])],
    );
    write(&before.path().join("a.png"), 3, 3, &[]);
    write(&after.path().join("a.png"), 3, 3, &[]);
    fs::write(before.path().join("MANIFEST.txt"), "not a picture").expect("a file");
    let report = compare_dirs(before.path(), after.path(), 0).expect("compares");
    assert_eq!(report.entries.len(), 2);
    assert_eq!(
        report.entry("a.png").expect("an entry").outcome,
        Outcome::Identical
    );
    assert!(matches!(
        report.entry("one/two/a.png").expect("an entry").outcome,
        Outcome::Differs { pixels: 1, .. }
    ));
}

#[test]
fn a_tolerance_of_two_accepts_a_difference_of_two_and_rejects_three() {
    let (before, after) = folders();
    write(&before.path().join("two.png"), 3, 3, &[]);
    write(&after.path().join("two.png"), 3, 3, &[(1, 1, [12, 18, 30])]);
    write(&before.path().join("three.png"), 3, 3, &[]);
    write(
        &after.path().join("three.png"),
        3,
        3,
        &[(1, 1, [13, 20, 30])],
    );
    let report = compare_dirs(before.path(), after.path(), 2).expect("compares");
    assert_eq!(
        report.entry("two.png").expect("an entry").outcome,
        Outcome::Identical
    );
    assert_eq!(
        report.entry("three.png").expect("an entry").outcome,
        Outcome::Differs {
            pixels: 1,
            max_delta: 3,
            bounds: (1, 1, 1, 1)
        }
    );
    let strict = compare_dirs(before.path(), after.path(), 0).expect("compares");
    assert_eq!(strict.problems(), 2);
}

#[test]
fn a_file_that_is_not_a_png_is_an_error_naming_it() {
    let (before, after) = folders();
    write(&before.path().join("good.png"), 2, 2, &[]);
    write(&after.path().join("good.png"), 2, 2, &[]);
    fs::write(before.path().join("broken.png"), "not a png").expect("a file");
    fs::write(after.path().join("broken.png"), "not a png").expect("a file");
    let error = compare_dirs(before.path(), after.path(), 0).expect_err("an error");
    assert!(error.to_string().contains("broken.png"), "{error}");
}

#[test]
fn a_missing_folder_is_an_error_naming_it() {
    let (before, _) = folders();
    let error = compare_dirs(before.path(), &before.path().join("nope"), 0).expect_err("an error");
    assert!(error.to_string().contains("nope"), "{error}");
}

#[test]
fn two_single_files_are_compared() {
    let (before, after) = folders();
    write(&before.path().join("x.png"), 2, 2, &[]);
    write(&after.path().join("y.png"), 2, 2, &[(0, 1, [0, 0, 0])]);
    let report = compare_dirs(&before.path().join("x.png"), &after.path().join("y.png"), 0)
        .expect("compares");
    assert_eq!(report.entries.len(), 1);
    assert!(matches!(
        report.entries[0].outcome,
        Outcome::Differs { pixels: 1, .. }
    ));
}

#[test]
fn a_folder_with_no_pictures_is_an_error_not_a_pass() {
    let (before, after) = folders();
    write(&before.path().join("a.png"), 2, 2, &[]);
    write(&after.path().join("a.png"), 2, 2, &[]);
    fs::write(after.path().join("notes.txt"), "no picture").expect("a file");
    let empty = tempfile::tempdir().expect("a folder");
    for (one, other) in [(empty.path(), before.path()), (before.path(), empty.path())] {
        let error = compare_dirs(one, other, 0).expect_err("an error");
        assert!(error.to_string().contains("no PNG files"), "{error}");
    }
    let both = compare_dirs(empty.path(), empty.path(), 0).expect_err("an error");
    assert!(
        both.to_string()
            .contains(empty.path().to_str().expect("a path")),
        "{both}"
    );
}

#[cfg(unix)]
#[test]
fn a_linked_picture_is_compared() {
    let (before, after) = folders();
    let target = tempfile::tempdir().expect("a folder");
    write(&target.path().join("real.png"), 3, 3, &[]);
    write(&before.path().join("a.png"), 3, 3, &[]);
    std::os::unix::fs::symlink(target.path().join("real.png"), after.path().join("a.png"))
        .expect("a link");
    let report = compare_dirs(before.path(), after.path(), 0).expect("compares");
    assert_eq!(report.entries.len(), 1);
    assert!(report.is_identical());
}

mod executable {
    use super::*;

    const PIXEL_DIFF: &str = env!("CARGO_BIN_EXE_pixel-diff");

    fn run(arguments: &[&Path]) -> (i32, String, String) {
        let output = Command::new(PIXEL_DIFF)
            .args(arguments)
            .output()
            .expect("runs");
        (
            output.status.code().expect("an exit status"),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    #[test]
    fn exit_0_when_identical_1_when_different_2_on_errors() {
        let (before, after) = folders();
        write(&before.path().join("a.png"), 3, 3, &[]);
        write(&after.path().join("a.png"), 3, 3, &[]);
        let (status, out, _) = run(&[before.path(), after.path()]);
        assert_eq!(
            (status, out.contains("1 identical, 0 not")),
            (0, true),
            "{out}"
        );

        write(&after.path().join("a.png"), 3, 3, &[(2, 2, [99, 99, 99])]);
        let (status, out, _) = run(&[before.path(), after.path()]);
        assert_eq!(status, 1);
        assert!(out.contains("DIFFERENT  a.png"), "{out}");

        let (status, _, err) = run(&[before.path()]);
        assert_eq!(status, 2);
        assert!(err.contains("usage"), "{err}");
        let (status, _, err) = run(&[before.path(), &before.path().join("nope")]);
        assert_eq!(status, 2);
        assert!(err.contains("nope"), "{err}");
    }

    #[test]
    fn tolerance_and_write_diffs_are_options() {
        let (before, after) = folders();
        let diffs = tempfile::tempdir().expect("a folder");
        write(&before.path().join("a.png"), 3, 3, &[]);
        write(&after.path().join("a.png"), 3, 3, &[(1, 1, [12, 20, 30])]);

        let (status, _, _) = run(&[
            before.path(),
            after.path(),
            Path::new("--tolerance"),
            Path::new("2"),
        ]);
        assert_eq!(status, 0);

        let (status, _, _) = run(&[
            before.path(),
            after.path(),
            Path::new("--write-diffs"),
            diffs.path(),
        ]);
        assert_eq!(status, 1);
        let marked = image::open(diffs.path().join("a.png"))
            .expect("a picture")
            .to_rgb8();
        assert_eq!(marked.get_pixel(1, 1).0, [255, 0, 0]);
        assert_ne!(marked.get_pixel(0, 0).0, [255, 0, 0]);

        let (status, _, err) = run(&[
            before.path(),
            after.path(),
            Path::new("--tolerance"),
            Path::new("x"),
        ]);
        assert_eq!(status, 2);
        assert!(err.contains("--tolerance"), "{err}");
    }

    #[test]
    fn an_empty_folder_exits_2() {
        let (before, after) = folders();
        write(&before.path().join("a.png"), 3, 3, &[]);
        let (status, _, err) = run(&[before.path(), after.path()]);
        assert_eq!(status, 2);
        assert!(err.contains("no PNG files"), "{err}");
    }

    #[test]
    fn a_file_that_is_not_a_png_exits_2_and_names_the_file() {
        let (before, after) = folders();
        fs::write(before.path().join("broken.png"), "nothing").expect("a file");
        fs::write(after.path().join("broken.png"), "nothing").expect("a file");
        let (status, _, err) = run(&[before.path(), after.path()]);
        assert_eq!(status, 2);
        assert!(err.contains("broken.png"), "{err}");
    }
}
