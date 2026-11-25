// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Mercury mark inside the app's own windows is `MercuryMark` (tinted by the palette). No
//! `.slint` file may draw the orange pictures `mercury-browser.svg` or `mercury-browser-small.svg`
//! except as a window's `icon:` property (a static picture macOS does not show). This keeps a merge
//! of another lane's old `Image { source: ...mercury-browser.svg }` from bringing the orange mark
//! back unnoticed.

use std::fs;
use std::path::{Path, PathBuf};

fn slint_files(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("the directory can be read") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            slint_files(&path, found);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "slint")
        {
            found.push(path);
        }
    }
}

#[test]
fn no_slint_file_draws_the_orange_mark_except_as_a_window_icon() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut files = Vec::new();
    slint_files(&root, &mut files);
    assert!(files.len() > 20, "the ui directory was found");
    let mut window_icons = 0;
    let mut offenders = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("the file can be read");
        for (number, line) in text.lines().enumerate() {
            if !line.contains("mercury-browser.svg") && !line.contains("mercury-browser-small.svg")
            {
                continue;
            }
            if line.trim_start().starts_with("icon:") {
                window_icons += 1;
            } else {
                offenders.push(format!(
                    "{}:{}: {}",
                    file.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "use MercuryMark (ui/theme/mercury-mark.slint) instead:\n{}",
        offenders.join("\n")
    );
    assert!(
        window_icons >= 4,
        "the guard still sees the window icons ({window_icons})"
    );
}
