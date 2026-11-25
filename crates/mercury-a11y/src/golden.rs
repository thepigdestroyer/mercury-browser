// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Golden files for tree snapshots: compare, and bless with `MERCURY_A11Y_BLESS=1`.
//!
//! Owner: item 3 (`core-states`). The contract is the one of the interface's `assert_snapshot`
//! (`crates/mercury-ui/tests/common/a11y.rs`): a golden file is `<name>.txt` in a folder, `name` is
//! lower-case words joined by hyphens, the file starts with `# ` comment lines (here three: the two
//! SPDX lines and the command that rewrites it) that are ignored when comparing and written when
//! blessing, `MERCURY_A11Y_BLESS=1` rewrites the file instead of comparing, and a missing file fails
//! with the text it would hold, so that a person can read it before blessing it.
//!
//! [`assert_golden`] uses `crates/mercury-a11y/tests/snapshots/`; [`assert_golden_in`] takes another
//! folder (a test in another crate).

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// The environment variable that turns a comparison into a rewrite: set it to `1`.
pub const BLESS_VARIABLE: &str = "MERCURY_A11Y_BLESS";

/// The command that rewrites this crate's golden files, named in the header of each one.
pub const BLESS_COMMAND: &str =
    "MERCURY_A11Y_BLESS=1 cargo test --release -p mercury-a11y --test snapshot";

/// The folder of this crate's golden files: `tests/snapshots/` of `mercury-a11y`.
pub fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

fn bless_requested() -> bool {
    std::env::var(BLESS_VARIABLE).is_ok_and(|value| value == "1")
}

/// Compare `actual` with `tests/snapshots/<name>.txt` of this crate, or rewrite that file when
/// `MERCURY_A11Y_BLESS=1`. **Panics** (a test failure) on a difference and on a missing file, with
/// the line difference or the text to bless.
#[track_caller]
pub fn assert_golden(name: &str, actual: &str) {
    check_golden(
        &golden_dir(),
        name,
        actual,
        bless_requested(),
        BLESS_COMMAND,
    );
}

/// [`assert_golden`] for a golden file in another folder; `bless_command` is written into the file's
/// header and into the failure message.
#[track_caller]
pub fn assert_golden_in(dir: &Path, name: &str, actual: &str, bless_command: &str) {
    check_golden(dir, name, actual, bless_requested(), bless_command);
}

/// The comparison itself: `bless` is a parameter (not read from the environment) so that the
/// function's own tests can run in a temporary folder.
#[track_caller]
pub fn check_golden(dir: &Path, name: &str, actual: &str, bless: bool, bless_command: &str) {
    assert!(
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "a golden name is words joined by hyphens, got {name:?}"
    );
    let path = dir.join(format!("{name}.txt"));
    if bless {
        fs::create_dir_all(dir).expect("create the golden folder");
        fs::write(&path, format!("{}{actual}", header(bless_command)))
            .expect("write the golden file");
        eprintln!("blessed {}", path.display());
        return;
    }
    let Ok(text) = fs::read_to_string(&path) else {
        panic!(
            "no golden file {}.\nCreate it with: {bless_command}\nand read it as a screen reader user would. It would hold:\n{actual}",
            path.display()
        );
    };
    let expected = without_header(&text);
    assert!(
        expected == actual,
        "the tree {name} changed (- golden file, + now), {}.\nIf the change is wanted, read the new tree as a screen reader user would, then bless it with: {bless_command}\n{}",
        path.display(),
        line_diff(&expected, actual)
    );
}

/// The lines every golden file starts with: the two SPDX lines and the bless command.
pub fn header(bless_command: &str) -> String {
    format!(
        "# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0\n\
         # SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors\n\
         # Rewrite with: {bless_command}\n"
    )
}

/// A golden file's text without its leading `# ` comment lines (and with `\n` line ends).
pub fn without_header(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut lines = text.lines().peekable();
    while lines
        .peek()
        .is_some_and(|line| *line == "#" || line.starts_with("# "))
    {
        lines.next();
    }
    let mut body = String::new();
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
    body
}

/// The most cells of the table the line diff fills; beyond it only the first difference is shown.
const DIFF_CELLS: usize = 4_000_000;

/// A line diff of two texts: `-` a line only in the golden file, `+` one only in the dump.
fn line_diff(expected: &str, actual: &str) -> String {
    let old: Vec<&str> = expected.lines().collect();
    let new: Vec<&str> = actual.lines().collect();
    let mut diff = String::new();
    if old.len().saturating_mul(new.len()) > DIFF_CELLS {
        let first = old
            .iter()
            .zip(&new)
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| old.len().min(new.len()));
        let _ = writeln!(
            diff,
            "(too long for a full diff: {} lines against {}; first difference at line {})",
            old.len(),
            new.len(),
            first + 1
        );
        if let Some(line) = old.get(first) {
            let _ = writeln!(diff, "- {line}");
        }
        if let Some(line) = new.get(first) {
            let _ = writeln!(diff, "+ {line}");
        }
        return diff;
    }
    // The longest common subsequence of the lines, from the end.
    let mut common = vec![vec![0u32; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            common[i][j] = if old[i] == new[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            let _ = writeln!(diff, "  {}", old[i]);
            i += 1;
            j += 1;
        } else if j < new.len() && (i == old.len() || common[i][j + 1] >= common[i + 1][j]) {
            let _ = writeln!(diff, "+ {}", new[j]);
            j += 1;
        } else {
            let _ = writeln!(diff, "- {}", old[i]);
            i += 1;
        }
    }
    diff
}
