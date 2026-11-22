// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What web-platform-tests' own server (`wptserve`) does for a test directory, as far as
//! `media-source/` needs it: the test files of a directory, and the `.any.html` / `.window.html`
//! pages it makes from `.any.js` / `.window.js` files and their `// META:` lines.
//!
//! Only the window variants are made: the dedicated-worker variants of `.any.js` files (and the
//! `dedicated-worker/` tests) are MSE in a worker, which is not planned (the MED-1b brief, D10).
//! Server-side substitution (`.sub.`), `.headers` files and the other `wptserve` features are not
//! implemented; no test of `media-source/` uses them (checked at the pinned commit).

use std::fs;
use std::io;
use std::path::Path;

/// The `// META:` lines at the top of a `.js` test file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// `script=` lines, in order (relative to the file, or absolute from the WPT root).
    pub scripts: Vec<String>,
    pub title: Option<String>,
    /// `timeout=long`.
    pub long_timeout: bool,
    /// `global=` (comma separated), when the file says.
    pub globals: Option<Vec<String>>,
}

/// Reads the `// META: name=value` lines at the top of a `.js` test file (the lines before the
/// first one that is neither blank nor a comment).
pub fn parse_meta(js: &str) -> Meta {
    let mut meta = Meta::default();
    for line in js.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(comment) = line.strip_prefix("//") else {
            break;
        };
        let Some(rest) = comment.trim().strip_prefix("META:") else {
            continue;
        };
        let Some((name, value)) = rest.trim().split_once('=') else {
            continue;
        };
        let value = value.trim();
        match name.trim() {
            "script" => meta.scripts.push(value.to_owned()),
            "title" => meta.title = Some(value.to_owned()),
            "timeout" => meta.long_timeout = value == "long",
            "global" => {
                meta.globals = Some(value.split(',').map(|g| g.trim().to_owned()).collect());
            }
            _ => {}
        }
    }
    meta
}

impl Meta {
    /// Whether the file has a window variant (`global=` is absent, or names `window`).
    pub fn has_window(&self) -> bool {
        self.globals
            .as_ref()
            .is_none_or(|globals| globals.iter().any(|g| g == "window"))
    }
}

/// The kind of a test file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An ordinary `.html` test page.
    Html,
    /// `<name>.any.html`, made from `<name>.any.js`.
    Any,
    /// `<name>.window.html`, made from `<name>.window.js`.
    Window,
    /// A page that is the test half of a reftest (`<link rel=match>`): it needs a screenshot
    /// comparison, which the runner does not do.
    Reftest,
}

/// One test file the runner can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Test {
    /// The URL path below the WPT root, with `/` separators, e.g. `media-source/mediasource-duration.html`.
    pub path: String,
    pub kind: Kind,
    /// The test asks for the long timeout.
    pub long_timeout: bool,
}

fn compact(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '"' | '\'') && !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn html_kind(html: &str) -> (Kind, bool) {
    let compact = compact(html);
    let long = compact.contains("name=timeoutcontent=long")
        || compact.contains("content=longname=timeout");
    let reftest = compact.contains("rel=match") || compact.contains("rel=mismatch");
    (if reftest { Kind::Reftest } else { Kind::Html }, long)
}

/// Every test of the directory `dir` (below the WPT root `root`), sorted by path: `.html` files
/// that are not references, and the `.any.html` and `.window.html` pages of `.any.js` and
/// `.window.js` files. Subdirectories are included; a file whose name ends `-ref.html` or
/// `-manual.html` is not a test.
pub fn discover(root: &Path, dir: &str) -> io::Result<Vec<Test>> {
    let mut found = Vec::new();
    walk(root, &root.join(dir), &mut found)?;
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<Test>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk(root, &path, found)?;
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let renamed =
            |new_name: String| format!("{}{new_name}", &relative[..relative.len() - name.len()]);
        if name.ends_with(".html") {
            if name.ends_with("-ref.html") || name.ends_with("-manual.html") {
                continue;
            }
            let (kind, long_timeout) = html_kind(&fs::read_to_string(&path).unwrap_or_default());
            found.push(Test {
                path: relative,
                kind,
                long_timeout,
            });
        } else if let Some(stem) = name.strip_suffix(".any.js") {
            let meta = parse_meta(&fs::read_to_string(&path).unwrap_or_default());
            if meta.has_window() {
                found.push(Test {
                    path: renamed(format!("{stem}.any.html")),
                    kind: Kind::Any,
                    long_timeout: meta.long_timeout,
                });
            }
        } else if let Some(stem) = name.strip_suffix(".window.js") {
            let meta = parse_meta(&fs::read_to_string(&path).unwrap_or_default());
            found.push(Test {
                path: renamed(format!("{stem}.window.html")),
                kind: Kind::Window,
                long_timeout: meta.long_timeout,
            });
        }
    }
    Ok(())
}

/// For a requested `.any.html` or `.window.html` path: the `.js` file it is made from (below the
/// WPT root) and whether it is an `.any` page.
pub fn source_of_wrapper(path: &str) -> Option<(String, Kind)> {
    if let Some(stem) = path.strip_suffix(".any.html") {
        Some((format!("{stem}.any.js"), Kind::Any))
    } else {
        path.strip_suffix(".window.html")
            .map(|stem| (format!("{stem}.window.js"), Kind::Window))
    }
}

/// The page `wptserve` makes for a `.any.js` (`kind` Any, which defines `GLOBAL` as `wptserve`
/// does) or `.window.js` file: testharness, the report script, the `META: script=` files, then
/// the test file itself, named `js_name` (relative to the page).
pub fn wrapper_html(js_name: &str, meta: &Meta, kind: Kind) -> String {
    let mut html = String::from("<!doctype html>\n<meta charset=utf-8>\n");
    let title = meta.title.as_deref().unwrap_or(js_name);
    html.push_str(&format!("<title>{}</title>\n", escape(title)));
    if meta.long_timeout {
        html.push_str("<meta name=\"timeout\" content=\"long\">\n");
    }
    html.push_str("<script src=\"/resources/testharness.js\"></script>\n");
    html.push_str("<script src=\"/resources/testharnessreport.js\"></script>\n");
    if kind == Kind::Any {
        html.push_str(
            "<script>\nself.GLOBAL = {\n  isWindow: function() { return true; },\n  isWorker: function() { return false; },\n  isShadowRealm: function() { return false; },\n};\n</script>\n",
        );
    }
    for script in &meta.scripts {
        html.push_str(&format!("<script src=\"{}\"></script>\n", escape(script)));
    }
    html.push_str("<div id=log></div>\n");
    html.push_str(&format!("<script src=\"{}\"></script>\n", escape(js_name)));
    html
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_lines_are_read_until_the_code_starts() {
        let meta = parse_meta(
            "// META: global=window,dedicatedworker\n// META: script=/resources/WebIDLParser.js\n// META: script=mediasource-util.js\n// META: timeout=long\n\n// a comment\n'use strict';\n// META: script=late.js\n",
        );
        assert_eq!(
            meta,
            Meta {
                scripts: vec![
                    "/resources/WebIDLParser.js".into(),
                    "mediasource-util.js".into()
                ],
                title: None,
                long_timeout: true,
                globals: Some(vec!["window".into(), "dedicatedworker".into()]),
            }
        );
        assert!(meta.has_window());
        assert!(Meta::default().has_window());
        let worker_only = parse_meta("// META: global=worker\n");
        assert!(!worker_only.has_window());
    }

    #[test]
    fn a_wrapper_names_its_scripts_in_order() {
        let meta =
            parse_meta("// META: script=/resources/idlharness.js\n// META: title=A <test>\n");
        let page = wrapper_html("idl.any.js", &meta, Kind::Any);
        let order: Vec<usize> = [
            "/resources/testharness.js",
            "/resources/testharnessreport.js",
            "self.GLOBAL",
            "/resources/idlharness.js",
            "idl.any.js",
        ]
        .iter()
        .map(|needle| {
            page.find(needle)
                .unwrap_or_else(|| panic!("{needle} missing"))
        })
        .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{page}");
        assert!(page.contains("<title>A &lt;test></title>"));
        let window = wrapper_html("x.window.js", &Meta::default(), Kind::Window);
        assert!(!window.contains("self.GLOBAL"));
    }

    #[test]
    fn wrapper_paths_map_back_to_their_source() {
        assert_eq!(
            source_of_wrapper("media-source/idlharness.any.html"),
            Some(("media-source/idlharness.any.js".into(), Kind::Any))
        );
        assert_eq!(
            source_of_wrapper("a/b.window.html"),
            Some(("a/b.window.js".into(), Kind::Window))
        );
        assert_eq!(source_of_wrapper("a/b.html"), None);
    }

    #[test]
    fn discovery_finds_pages_wrappers_and_skips_references() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let tree = root.join("media-source");
        fs::create_dir_all(tree.join("sub")).unwrap();
        fs::write(tree.join("a.html"), "<!doctype html><title>a</title>").unwrap();
        fs::write(
            tree.join("long.html"),
            "<meta name=\"timeout\" content=\"long\">",
        )
        .unwrap();
        fs::write(
            tree.join("visible.html"),
            "<link rel=\"match\" href=\"visible-ref.html\">",
        )
        .unwrap();
        fs::write(tree.join("visible-ref.html"), "ref").unwrap();
        fs::write(tree.join("m-manual.html"), "manual").unwrap();
        fs::write(tree.join("util.js"), "// not a test\n").unwrap();
        fs::write(tree.join("idl.any.js"), "// META: timeout=long\n").unwrap();
        fs::write(tree.join("w.any.js"), "// META: global=worker\n").unwrap();
        fs::write(tree.join("sub/x.window.js"), "// nothing\n").unwrap();
        let found = discover(root, "media-source").unwrap();
        let summary: Vec<(&str, Kind, bool)> = found
            .iter()
            .map(|t| (t.path.as_str(), t.kind, t.long_timeout))
            .collect();
        assert_eq!(
            summary,
            [
                ("media-source/a.html", Kind::Html, false),
                ("media-source/idl.any.html", Kind::Any, true),
                ("media-source/long.html", Kind::Html, true),
                ("media-source/sub/x.window.html", Kind::Window, false),
                ("media-source/visible.html", Kind::Reftest, false),
            ]
        );
    }
}
