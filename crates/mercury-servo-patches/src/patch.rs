// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Multi-file unified diffs between a pristine crate and Mercury's copy.
//!
//! The format is the one `git diff` writes, so `git apply` understands it too:
//! each file starts with `diff --git a/<path> b/<path>`, a created file has a
//! `new file mode` line and `--- /dev/null`, a deleted one `deleted file mode`
//! and `+++ /dev/null`. Text before the first file is a free-form preamble.
//! Only text files can be changed: a file containing a NUL byte is binary.

use crate::error::Result;
use crate::fail;
use crate::tree::Tree;
use diffy::{DiffOptions, Line, Patch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Added,
    Deleted,
    Modified,
}

/// One file's part of a patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    pub path: String,
    pub kind: Kind,
    /// The `---`/`+++` lines and the hunks; empty for an empty created or deleted file.
    body: Vec<u8>,
}

const DIFF_PREFIX: &[u8] = b"diff --git a/";

fn is_binary(data: &[u8]) -> bool {
    data.contains(&0)
}

/// Paths are written unquoted, so they must not need quoting.
fn check_patch_path(path: &str) -> Result<()> {
    if path
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '"')
    {
        return Err(fail!(
            "{path:?}: paths with spaces, quotes or control characters cannot be patched"
        ));
    }
    Ok(())
}

impl FilePatch {
    fn new(path: &str, old: Option<&[u8]>, new: Option<&[u8]>) -> Result<Option<FilePatch>> {
        let (kind, old_name, new_name) = match (old, new) {
            (None, None) => return Ok(None),
            (None, Some(_)) => (Kind::Added, "/dev/null".to_owned(), format!("b/{path}")),
            (Some(_), None) => (Kind::Deleted, format!("a/{path}"), "/dev/null".to_owned()),
            (Some(old), Some(new)) if old == new => return Ok(None),
            (Some(_), Some(_)) => (Kind::Modified, format!("a/{path}"), format!("b/{path}")),
        };
        let (old, new) = (old.unwrap_or_default(), new.unwrap_or_default());
        if is_binary(old) || is_binary(new) {
            return Err(fail!(
                "{path}: binary files cannot be patched; leave the file as published"
            ));
        }
        check_patch_path(path)?;
        let body = if old.is_empty() && new.is_empty() {
            Vec::new()
        } else {
            DiffOptions::new()
                .set_original_filename(old_name)
                .set_modified_filename(new_name)
                .create_patch_bytes(old, new)
                .to_bytes()
        };
        Ok(Some(FilePatch {
            path: path.to_owned(),
            kind,
            body,
        }))
    }

    fn parsed(&self) -> Result<Option<Patch<'_, [u8]>>> {
        if self.body.is_empty() {
            return Ok(None);
        }
        Patch::from_bytes(&self.body)
            .map(Some)
            .map_err(|error| fail!("{}: the patch cannot be read: {error}", self.path))
    }

    /// The lines this file's patch adds.
    pub fn added_lines(&self) -> Result<Vec<Vec<u8>>> {
        let Some(patch) = self.parsed()? else {
            return Ok(Vec::new());
        };
        Ok(patch
            .hunks()
            .iter()
            .flat_map(|hunk| hunk.lines())
            .filter_map(|line| match line {
                Line::Insert(text) => Some(text.to_vec()),
                _ => None,
            })
            .collect())
    }

    /// Applies this file's change to `base`, the pristine file (`None` if it
    /// does not exist). Returns the new contents, or `None` for a deletion.
    pub fn apply(&self, base: Option<&[u8]>) -> Result<Option<Vec<u8>>> {
        match (self.kind, base) {
            (Kind::Added, Some(_)) => {
                return Err(fail!(
                    "{}: the patch creates it, but the crate already has it",
                    self.path
                ));
            }
            (Kind::Deleted | Kind::Modified, None) => {
                return Err(fail!(
                    "{}: the patch changes it, but the crate does not have it",
                    self.path
                ));
            }
            _ => {}
        }
        let base = base.unwrap_or_default();
        let result = match self.parsed()? {
            None => base.to_vec(),
            Some(patch) => {
                diffy::apply_bytes(base, &patch).map_err(|error| fail!("{}: {error}", self.path))?
            }
        };
        if self.kind == Kind::Deleted {
            if !result.is_empty() {
                return Err(fail!(
                    "{}: the patch deletes it, but not all of its contents",
                    self.path
                ));
            }
            return Ok(None);
        }
        Ok(Some(result))
    }

    fn render(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(format!("diff --git a/{0} b/{0}\n", self.path).as_bytes());
        match self.kind {
            Kind::Added => out.extend_from_slice(b"new file mode 100644\n"),
            Kind::Deleted => out.extend_from_slice(b"deleted file mode 100644\n"),
            Kind::Modified => {}
        }
        out.extend_from_slice(&self.body);
    }
}

/// The changes that turn `pristine` into `tree`, in path order.
pub fn diff_trees(pristine: &Tree, tree: &Tree) -> Result<Vec<FilePatch>> {
    let mut paths: Vec<&String> = pristine.files.keys().chain(tree.files.keys()).collect();
    paths.sort();
    paths.dedup();
    let mut patches = Vec::new();
    for path in paths {
        let old = pristine.files.get(path).map(Vec::as_slice);
        let new = tree.files.get(path).map(Vec::as_slice);
        if let Some(patch) = FilePatch::new(path, old, new)? {
            patches.push(patch);
        }
    }
    Ok(patches)
}

/// The patch file: `preamble` (each line prefixed with `# `), then every file.
pub fn render(preamble: &str, patches: &[FilePatch]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in preamble.lines() {
        out.extend_from_slice(
            if line.is_empty() {
                b"#\n".to_vec()
            } else {
                format!("# {line}\n").into_bytes()
            }
            .as_slice(),
        );
    }
    for patch in patches {
        patch.render(&mut out);
    }
    out
}

/// Splits a patch file into its files. The preamble is ignored.
pub fn parse(patch: &[u8]) -> Result<Vec<FilePatch>> {
    let mut sections: Vec<(usize, usize)> = Vec::new();
    let mut offset = 0;
    for line in patch.split_inclusive(|&byte| byte == b'\n') {
        if line.starts_with(DIFF_PREFIX) {
            if let Some(last) = sections.last_mut() {
                last.1 = offset;
            }
            sections.push((offset, patch.len()));
        }
        offset += line.len();
    }
    let mut files: Vec<FilePatch> = Vec::new();
    for (start, end) in sections {
        let section = &patch[start..end];
        let header_end = section
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(section.len(), |at| at + 1);
        let header = std::str::from_utf8(&section[..header_end])
            .map_err(|_| fail!("a `diff --git` line is not UTF-8"))?
            .trim_end();
        let names = header.strip_prefix("diff --git a/").unwrap_or_default();
        let path = match names.split_once(" b/") {
            Some((old, new)) if old == new => old.to_owned(),
            _ => return Err(fail!("cannot read the header {header:?}")),
        };
        let rest = &section[header_end..];
        let mut kind = Kind::Modified;
        let mut body_start = rest.len();
        let mut line_start = 0;
        for line in rest.split_inclusive(|&byte| byte == b'\n') {
            if line.starts_with(b"new file mode ") {
                kind = Kind::Added;
            } else if line.starts_with(b"deleted file mode ") {
                kind = Kind::Deleted;
            } else if line.starts_with(b"--- ") {
                body_start = line_start;
                break;
            }
            line_start += line.len();
        }
        let body = rest[body_start..].to_vec();
        if body.is_empty() && kind == Kind::Modified {
            return Err(fail!("{path}: the patch has no changes for it"));
        }
        if files.iter().any(|file| file.path == path) {
            return Err(fail!("{path}: the patch changes it twice"));
        }
        let file = FilePatch { path, kind, body };
        file.parsed()?;
        files.push(file);
    }
    Ok(files)
}

/// `pristine` with `patches` applied.
pub fn apply(pristine: &Tree, patches: &[FilePatch]) -> Result<Tree> {
    let mut tree = pristine.clone();
    for patch in patches {
        match patch.apply(pristine.files.get(&patch.path).map(Vec::as_slice))? {
            Some(data) => {
                tree.files.insert(patch.path.clone(), data);
            }
            None => {
                tree.files.remove(&patch.path);
                tree.executable.remove(&patch.path);
            }
        }
    }
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> Tree {
        let mut tree = Tree::default();
        for (path, contents) in files {
            tree.files
                .insert((*path).to_owned(), contents.as_bytes().to_vec());
        }
        tree
    }

    #[test]
    fn identical_trees_have_an_empty_patch() {
        let a = tree(&[("src/lib.rs", "fn a() {}\n")]);
        assert!(diff_trees(&a, &a).unwrap().is_empty());
        assert_eq!(render("", &[]), b"");
    }

    #[test]
    fn round_trip_covers_changes_additions_deletions_and_missing_newlines() {
        let pristine = tree(&[
            ("Cargo.toml", "[package]\nname = \"demo\"\n"),
            ("src/lib.rs", "fn a() {}\nfn b() {}\nfn c() {}\n"),
            ("src/old.rs", "gone\n"),
            ("src/empty.rs", ""),
            ("README.md", "no newline"),
        ]);
        let ours = tree(&[
            ("Cargo.toml", "[package]\nname = \"demo\"\n"),
            (
                "src/lib.rs",
                "fn a() {}\n// Mercury: b changed\nfn b2() {}\nfn c() {}\n",
            ),
            ("src/new.rs", "// Mercury: new\n"),
            ("src/new_empty.rs", ""),
            ("README.md", "no newline, still"),
        ]);
        let patches = diff_trees(&pristine, &ours).unwrap();
        let kinds: Vec<_> = patches.iter().map(|p| (p.path.as_str(), p.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("README.md", Kind::Modified),
                ("src/empty.rs", Kind::Deleted),
                ("src/lib.rs", Kind::Modified),
                ("src/new.rs", Kind::Added),
                ("src/new_empty.rs", Kind::Added),
                ("src/old.rs", Kind::Deleted),
            ]
        );
        let text = render("Mercury's changes.\n\nSecond paragraph.", &patches);
        let text_str = String::from_utf8(text.clone()).unwrap();
        assert!(text_str.starts_with("# Mercury's changes.\n#\n# Second paragraph.\ndiff --git"));
        assert!(text_str.contains("\\ No newline at end of file"));
        assert!(text_str.contains("diff --git a/src/new.rs b/src/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/new.rs\n"));

        let parsed = parse(&text).unwrap();
        assert_eq!(parsed, patches);
        assert_eq!(apply(&pristine, &parsed).unwrap(), ours);
        assert_eq!(
            parsed[2].added_lines().unwrap(),
            [
                b"// Mercury: b changed\n".to_vec(),
                b"fn b2() {}\n".to_vec()
            ]
        );
    }

    #[test]
    fn a_patch_that_does_not_fit_is_an_error() {
        let pristine = tree(&[("src/lib.rs", "one\ntwo\nthree\n")]);
        let ours = tree(&[("src/lib.rs", "one\n2\nthree\n")]);
        let patches = diff_trees(&pristine, &ours).unwrap();
        let other = tree(&[("src/lib.rs", "uno\ndos\ntres\n")]);
        assert!(
            apply(&other, &patches)
                .unwrap_err()
                .0
                .contains("src/lib.rs")
        );
        assert!(
            apply(&Tree::default(), &patches)
                .unwrap_err()
                .0
                .contains("does not have it")
        );
    }

    #[test]
    fn binary_changes_and_unquotable_paths_are_refused() {
        let pristine = tree(&[("logo.png", "a\0b")]);
        let ours = tree(&[("logo.png", "a\0c")]);
        assert!(
            diff_trees(&pristine, &ours)
                .unwrap_err()
                .0
                .contains("binary")
        );
        // An unchanged binary file is fine.
        assert!(diff_trees(&pristine, &pristine).unwrap().is_empty());
        let spaced = tree(&[("a b.rs", "x\n")]);
        assert!(
            diff_trees(&Tree::default(), &spaced)
                .unwrap_err()
                .0
                .contains("spaces")
        );
    }

    #[test]
    fn content_that_looks_like_a_header_stays_inside_its_file() {
        let pristine = tree(&[("notes.txt", "a\n")]);
        let ours = tree(&[("notes.txt", "a\ndiff --git a/x b/x\n--- a/x\n")]);
        let patches = diff_trees(&pristine, &ours).unwrap();
        let parsed = parse(&render("", &patches)).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(apply(&pristine, &parsed).unwrap(), ours);
    }

    #[test]
    fn malformed_patches_are_rejected() {
        assert!(parse(b"diff --git a/x b/y\n--- a/x\n+++ b/y\n").is_err());
        assert!(parse(b"diff --git a/x b/x\nindex 0..1\n").is_err());
        let twice = b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n\
diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        assert!(parse(twice).unwrap_err().0.contains("twice"));
        assert!(parse(b"just a preamble\n").unwrap().is_empty());
    }
}
