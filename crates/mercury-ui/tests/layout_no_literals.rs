// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The layout-switch lane puts no colour in its `.slint` files: every colour is a `ChromePalette`
//! role or a field of a row Rust fills, so the palette and the scheme decide how the window looks.

use std::fs;
use std::path::PathBuf;

/// The `.slint` files of `ui/layout`, with their text.
fn layout_files() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/layout");
    let mut files: Vec<(String, String)> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "slint"))
        .map(|path| {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()));
            (path.display().to_string(), text)
        })
        .collect();
    files.sort();
    files
}

/// Every place `text` holds a colour literal: `#` and 3, 4, 6 or 8 hex digits, or `rgb(`, `rgba(`.
fn colour_literals(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes = text.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'#' {
            continue;
        }
        let digits = bytes[index + 1..]
            .iter()
            .take_while(|b| b.is_ascii_hexdigit())
            .count();
        if matches!(digits, 3 | 4 | 6 | 8) {
            found.push(text[index..index + 1 + digits].to_string());
        }
    }
    for needle in ["rgb(", "rgba(", "hsl(", "hsla("] {
        if text.contains(needle) {
            found.push(needle.to_string());
        }
    }
    found
}

#[test]
fn the_layout_files_exist() {
    let names: Vec<String> = layout_files().into_iter().map(|(path, _)| path).collect();
    assert!(
        names.iter().any(|name| name.ends_with("bridges.slint")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|name| name.ends_with("hosts.slint")),
        "{names:?}"
    );
}

#[test]
fn the_layout_files_have_no_colour_literal() {
    for (path, text) in layout_files() {
        assert_eq!(colour_literals(&text), Vec::<String>::new(), "{path}");
    }
}

#[test]
fn the_literal_finder_sees_what_it_should() {
    assert_eq!(colour_literals("background: #1a2b3c;"), ["#1a2b3c"]);
    assert_eq!(colour_literals("c: #fff; d: #ABCD;"), ["#fff", "#ABCD"]);
    assert_eq!(colour_literals("c: rgb(1, 2, 3);"), ["rgb("]);
    assert!(colour_literals("// issue #12 and a #hash").is_empty());
    assert!(colour_literals("background: ChromePalette.accent;").is_empty());
}
