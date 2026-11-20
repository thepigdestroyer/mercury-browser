// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A field that hides what is typed must hide it from assistive technology too.
//!
//! Slint's std `LineEdit` binds its `accessible-value` to its text, and the AccessKit adapter
//! hands that value on for the password role as well, which macOS gives to any program with
//! accessibility access as the field's AXValue, unmasked. A `TextInput` with a password input
//! type is safe only when it sets `accessible-value` to nothing itself. This test reads every
//! `.slint` file of the interface and fails for a password input type anywhere but in the two
//! components written for it (`RequestField` and `SecretField`), or in a component that leaves
//! the value alone. The typing tests (`a11y_windows.rs`, `a11y_page_ui.rs`) read the value of
//! those two components on a real window.
//!
//! No window and no Slint platform are needed: it reads the files.

use std::fs;
use std::path::{Path, PathBuf};

/// The files (relative to `ui/`) that declare a password input type, and the component in each.
const ALLOWED: &[(&str, &str)] = &[
    ("dialogs/page-request-parts.slint", "RequestField"),
    ("passwords/secret-field.slint", "SecretField"),
];

fn ui_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ui")
}

fn slint_files(directory: &Path, found: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
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

/// One element with a password input type.
#[derive(Debug, PartialEq, Eq)]
struct PasswordField {
    /// The line (counted from 1) of its `input-type`.
    line: usize,
    /// Whether the same element sets `accessible-value` to nothing (when it is a password).
    hides_value: bool,
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn is_comment(line: &str) -> bool {
    line.trim_start().starts_with("//")
}

/// The elements in `source` whose `input-type` can be a password.
fn password_fields(source: &str) -> Vec<PasswordField> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut fields = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if is_comment(line)
            || !trimmed.starts_with("input-type:")
            || !(trimmed.contains("InputType.password") || trimmed.ends_with(": password;"))
        {
            continue;
        }
        let own = indent(line);
        // The element's braces: the nearest line above with less indentation that opens one, and
        // the first line below at that indentation that closes it.
        let open = (0..index)
            .rev()
            .find(|&above| {
                !lines[above].trim().is_empty()
                    && indent(lines[above]) < own
                    && lines[above].trim_end().ends_with('{')
            })
            .expect("an input-type belongs to an element");
        let close = (index + 1..lines.len())
            .find(|&below| {
                indent(lines[below]) == indent(lines[open])
                    && lines[below].trim_start().starts_with('}')
            })
            .expect("an element is closed");
        let hides_value = lines[open + 1..close].iter().any(|child| {
            if is_comment(child) || indent(child) != own {
                return false;
            }
            let child = child.trim();
            let Some(value) = child.strip_prefix("accessible-value:") else {
                return false;
            };
            let value = value.trim().trim_end_matches(';').trim();
            // Nothing, or nothing when the field is a password.
            value == "\"\"" || value.contains("? \"\" :")
        });
        fields.push(PasswordField {
            line: index + 1,
            hides_value,
        });
    }
    fields
}

#[test]
fn the_scan_tells_a_field_that_hides_its_value_from_one_that_does_not() {
    let std_line_edit = "\
LineEdit {
    accessible-label: \"New password\";
    input-type: password;
    text <=> root.password;
}
";
    let fields = password_fields(std_line_edit);
    assert_eq!(
        fields,
        [PasswordField {
            line: 3,
            hides_value: false
        }]
    );

    let hiding = "\
component Field inherits Rectangle {
    input := TextInput {
        input-type: root.revealed ? InputType.text : InputType.password;
        // never the text
        accessible-value: \"\";
        accessible-label: \"Password\";
    }
}
";
    assert_eq!(
        password_fields(hiding),
        [PasswordField {
            line: 3,
            hides_value: true
        }]
    );

    let conditional = "\
TextInput {
    input-type: root.password ? InputType.password : InputType.text;
    accessible-value: root.password ? \"\" : root.text;
}
";
    assert_eq!(
        password_fields(conditional),
        [PasswordField {
            line: 2,
            hides_value: true
        }]
    );

    // A value set on another element, a value that is the text, and a comment do not count.
    let wrong = "\
Rectangle {
    TextInput {
        input-type: password;
        accessible-value: root.text;
    }
    TextInput {
        // input-type: password;
        accessible-value: \"\";
    }
    Text {
        accessible-value: \"\";
    }
}
";
    assert_eq!(
        password_fields(wrong),
        [PasswordField {
            line: 3,
            hides_value: false
        }]
    );

    // A plain text field is not a password field.
    assert!(password_fields("TextInput {\n    input-type: text;\n}\n").is_empty());
}

#[test]
fn a_password_input_type_is_only_where_the_value_is_kept_from_assistive_technology() {
    let mut files = Vec::new();
    slint_files(&ui_dir(), &mut files);
    assert!(files.len() > 30, "the interface's files were not found");

    let mut found_in = Vec::new();
    let mut problems = Vec::new();
    for path in &files {
        let relative = path
            .strip_prefix(ui_dir())
            .expect("inside ui/")
            .to_string_lossy()
            .replace('\\', "/");
        let source = fs::read_to_string(path).expect("a readable file");
        let fields = password_fields(&source);
        if fields.is_empty() {
            continue;
        }
        found_in.push(relative.clone());
        let allowed = ALLOWED.iter().any(|(file, _)| *file == relative);
        for field in fields {
            if !allowed {
                problems.push(format!(
                    "ui/{relative}:{}: a password input type outside {}. A std `LineEdit` hands its text \
                     to assistive technology; use `SecretField` (passwords/secret-field.slint) or \
                     `RequestField` (dialogs/page-request-parts.slint)",
                    field.line,
                    ALLOWED
                        .iter()
                        .map(|(file, component)| format!("{component} in ui/{file}"))
                        .collect::<Vec<_>>()
                        .join(" and ")
                ));
            } else if !field.hides_value {
                problems.push(format!(
                    "ui/{relative}:{}: this password field does not set `accessible-value` to nothing",
                    field.line
                ));
            }
        }
    }
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));

    // The list above is not stale: each component is where it is said to be.
    for (file, component) in ALLOWED {
        assert!(
            found_in.iter().any(|found| found == file),
            "{component} no longer declares a password input type in ui/{file}: update ALLOWED"
        );
    }
}
