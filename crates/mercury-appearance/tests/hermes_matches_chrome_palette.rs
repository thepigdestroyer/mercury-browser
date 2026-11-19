// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The `hermes` palette is a copy of today's `ChromePalette`; this test keeps it a faithful one.
//!
//! `crates/mercury-ui/ui/theme/chrome-palette.slint` is still the interface's source of colours
//! (the palettes in this crate are not wired in yet). The `hermes` palette exists so that the
//! theme-plumbing lane's pixel-diff gate can say "the chrome driven by palette data renders what
//! it renders today". That is only true while `hermes` equals the file, so this test reads the
//! file, parses every `out property <color>` (the three forms it uses: `root.dark ? #D : #L;`, a
//! plain `#ffffff;`, and the `#rrggbbaa` translucent values) and fails, naming the role, when
//!
//! - the file has a colour that [`Role`] lacks, or [`Role`] has one the file lacks, or
//! - the two disagree on a value in either scheme.
//!
//! **theme-plumbing replaces or deletes this test** when it turns `ChromePalette` into a table
//! filled from palette data (there is then no file to compare with), and the layout-switch lane
//! deletes it together with the `hermes` palette.

use std::collections::HashSet;
use std::path::PathBuf;

use mercury_appearance::Colour;
use mercury_appearance::palette::{Palette, Role, Scheme};
use mercury_appearance::palettes::hermes;

/// One `out property <color>` of the Slint file, with the two values it can take.
#[derive(Debug, PartialEq)]
struct SlintColour {
    name: String,
    dark: Colour,
    light: Colour,
}

/// The file with `//` comments removed. (Nothing in it has a `//` inside a string.)
fn without_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `out property <color>` in `source`, and a message for each one whose value is in a form
/// this parser does not know.
fn colour_properties(source: &str) -> (Vec<SlintColour>, Vec<String>) {
    const MARKER: &str = "out property <color>";
    let mut found = Vec::new();
    let mut unreadable = Vec::new();
    for statement in without_comments(source).split(';') {
        let Some((_, declaration)) = statement.split_once(MARKER) else {
            continue;
        };
        let Some((name, value)) = declaration.split_once(':') else {
            unreadable.push(format!("`{}` has no value", declaration.trim()));
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        let colours = if let Some(themed) = value.strip_prefix("root.dark") {
            themed
                .trim_start()
                .strip_prefix('?')
                .and_then(|choice| choice.split_once(':'))
                .and_then(|(dark, light)| {
                    Some((Colour::parse(dark.trim())?, Colour::parse(light.trim())?))
                })
        } else {
            Colour::parse(value).map(|both| (both, both))
        };
        match colours {
            Some((dark, light)) => found.push(SlintColour {
                name: name.to_owned(),
                dark,
                light,
            }),
            None => unreadable.push(format!(
                "cannot read the value of `{name}`: `{value}` is not `root.dark ? #D : #L` or a plain colour"
            )),
        }
    }
    (found, unreadable)
}

/// What is wrong, in plain words, if `palette` is not the colours `source` declares. Empty when
/// they agree.
fn disagreements(source: &str, palette: &Palette) -> Vec<String> {
    let (properties, mut problems) = colour_properties(source);
    if properties.is_empty() {
        problems.push("found no `out property <color>` in the Slint file".to_owned());
    }
    let mut named = HashSet::new();
    for property in &properties {
        if !named.insert(property.name.as_str()) {
            problems.push(format!(
                "chrome-palette.slint declares `{}` twice",
                property.name
            ));
            continue;
        }
        let Some(role) = Role::from_slint_name(&property.name) else {
            problems.push(format!(
                "chrome-palette.slint has `{}`, which Role has no variant for: add the role to \
                 palette.rs and a value to every palette table",
                property.name
            ));
            continue;
        };
        for scheme in Scheme::ALL {
            let slint = if scheme.is_dark() {
                property.dark
            } else {
                property.light
            };
            let ours = palette.colour(scheme, role);
            if slint != ours {
                problems.push(format!(
                    "`{}` differs in {}: chrome-palette.slint has {}, {} has {}",
                    property.name,
                    scheme.name(),
                    slint.to_hex(),
                    palette.id,
                    ours.to_hex()
                ));
            }
        }
    }
    for role in Role::ALL {
        if !named.contains(role.slint_name()) {
            problems.push(format!(
                "Role::{role:?} (`{}`) has no `out property <color>` in chrome-palette.slint",
                role.slint_name()
            ));
        }
    }
    problems
}

/// The Slint file as the lines a `ChromePalette` would have if it were `palette`.
fn render(palette: &Palette) -> String {
    let mut text = String::from("export global ChromePalette {\n    in property <bool> x: true;\n");
    for role in Role::ALL {
        let (dark, light) = (
            palette.colour(Scheme::Dark, role),
            palette.colour(Scheme::Light, role),
        );
        if dark == light {
            text.push_str(&format!(
                "    out property <color> {}: {};\n",
                role.slint_name(),
                dark.to_hex()
            ));
        } else {
            text.push_str(&format!(
                "    // {}\n    out property <color> {}: root.dark ? {} : {};\n",
                role.slint_name(),
                role.slint_name(),
                dark.to_hex(),
                light.to_hex()
            ));
        }
    }
    text.push_str("}\n");
    text
}

fn chrome_palette_slint() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../mercury-ui/ui/theme/chrome-palette.slint");
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read {}: {error}. This test compares the hermes palette with the interface's \
             ChromePalette; if the file moved, update the path (or delete the test, as the \
             theme-plumbing lane does when ChromePalette becomes data-driven)",
            path.display()
        )
    })
}

#[test]
fn hermes_is_exactly_the_chrome_palette_slint_file() {
    let problems = disagreements(&chrome_palette_slint(), &hermes::PALETTE);
    assert!(
        problems.is_empty(),
        "the hermes palette and chrome-palette.slint disagree:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn the_slint_file_has_the_54_colours_the_roles_name() {
    let (properties, unreadable) = colour_properties(&chrome_palette_slint());
    assert!(unreadable.is_empty(), "{unreadable:?}");
    assert_eq!(properties.len(), Role::COUNT);
    // Three of the forms are in use: themed, a plain colour, and translucent values.
    assert!(
        properties
            .iter()
            .any(|property| property.dark != property.light)
    );
    assert!(
        properties
            .iter()
            .any(|property| property.dark == property.light)
    );
    assert!(properties.iter().any(|property| property.dark.a != 255));
}

#[test]
fn the_parser_reads_the_three_forms_and_skips_comments_and_other_properties() {
    let source = "\
        // out property <color> ghost: root.dark ? #111111 : #eeeeee;\n\
        import { Palette } from \"std-widgets.slint\";\n\
        export global G {\n\
            in property <ChromeTheme> theme: ChromeTheme.system;\n\
            out property <bool> dark: root.theme == ChromeTheme.dark;\n\
            // A themed colour, then a plain one, then translucent ones.\n\
            out property <color> window-background: root.dark ? #0b0f14 : #e9edf2;\n\
            out property <color> capture-indicator-glyph: #ffffff;\n\
            out property <color> scrim: root.dark ?\n\
                #00000099 : #0000004d;\n\
            out property <color> short: root.dark ? #abc : #DEF;\n\
        }\n";
    let (properties, unreadable) = colour_properties(source);
    assert!(unreadable.is_empty(), "{unreadable:?}");
    assert_eq!(
        properties,
        vec![
            SlintColour {
                name: "window-background".into(),
                dark: Colour::hex(0x0b0f14),
                light: Colour::hex(0xe9edf2),
            },
            SlintColour {
                name: "capture-indicator-glyph".into(),
                dark: Colour::WHITE,
                light: Colour::WHITE,
            },
            SlintColour {
                name: "scrim".into(),
                dark: Colour::rgba(0, 0, 0, 0x99),
                light: Colour::rgba(0, 0, 0, 0x4d),
            },
            SlintColour {
                name: "short".into(),
                dark: Colour::rgb(0xaa, 0xbb, 0xcc),
                light: Colour::rgb(0xdd, 0xee, 0xff),
            },
        ]
    );
}

#[test]
fn a_value_the_parser_does_not_know_is_reported_not_skipped() {
    let source = "\
        out property <color> a: root.dark ? #000000 : red;\n\
        out property <color> b: Palette.accent;\n\
        out property <color> c: root.dark ? #000000;\n\
        out property <color> d\n";
    let (properties, unreadable) = colour_properties(source);
    assert!(properties.is_empty(), "{properties:?}");
    assert_eq!(unreadable.len(), 4, "{unreadable:?}");
    assert!(unreadable[0].contains("`a`") && unreadable[0].contains("red"));
    assert!(unreadable[1].contains("`b`"));
    assert!(unreadable[2].contains("`c`"));
}

#[test]
fn a_file_written_from_hermes_agrees_with_hermes() {
    let problems = disagreements(&render(&hermes::PALETTE), &hermes::PALETTE);
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn a_role_the_file_lacks_is_named() {
    let source = render(&hermes::PALETTE)
        .lines()
        .filter(|line| !line.contains("out property <color> text-muted:"))
        .collect::<Vec<_>>()
        .join("\n");
    let problems = disagreements(&source, &hermes::PALETTE);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("text-muted") && problems[0].contains("no `out property"));
}

#[test]
fn a_colour_the_roles_lack_is_named() {
    let mut source = render(&hermes::PALETTE);
    source.push_str("out property <color> brand-new: root.dark ? #000000 : #ffffff;\n");
    let problems = disagreements(&source, &hermes::PALETTE);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("brand-new") && problems[0].contains("no variant"));
}

#[test]
fn a_value_that_differs_is_named_with_both_values() {
    let hermes_text_dark = hermes::PALETTE.colour(Scheme::Dark, Role::Text).to_hex();
    let source = render(&hermes::PALETTE).replace(&hermes_text_dark, "#123456");
    let problems = disagreements(&source, &hermes::PALETTE);
    // `text` has that dark value, and so may other roles that share it (info-text does).
    assert!(!problems.is_empty());
    let text = problems
        .iter()
        .find(|problem| problem.starts_with("`text` differs in dark"))
        .unwrap_or_else(|| panic!("no problem names `text`: {problems:?}"));
    assert!(
        text.contains("#123456") && text.contains(&hermes_text_dark) && text.contains("hermes")
    );
    assert!(problems.iter().all(|problem| problem.contains("in dark")));
}

#[test]
fn a_property_declared_twice_is_reported() {
    let mut source = render(&hermes::PALETTE);
    source.push_str("out property <color> text: root.dark ? #000000 : #ffffff;\n");
    let problems = disagreements(&source, &hermes::PALETTE);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("`text` twice"));
}

#[test]
fn an_empty_file_is_a_failure_not_a_pass() {
    let problems = disagreements("", &hermes::PALETTE);
    assert!(problems.iter().any(|problem| problem.contains("found no")));
    assert!(
        problems.len() > Role::COUNT,
        "every role is reported missing too"
    );
}
