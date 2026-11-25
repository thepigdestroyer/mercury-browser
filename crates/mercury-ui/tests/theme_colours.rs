// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The colour layer of the chrome: `ChromePalette` holds two tables of colours that Rust pushes
//! (`mercury_ui::apply_look`), and every role reads the one for the scheme in use.
//!
//! - the role guard: the `out property <color>` roles of `chrome-palette.slint`, the fields of
//!   `ChromeColors` and `Role::ALL` are one set, and each table lists every field;
//! - the built-in tables are today's colours (the `hermes` palette): pushing Hermes changes no role;
//! - pushing `mercury-b2`, or two different palettes, gives exactly their colours;
//! - the scheme is chosen in Slint: changing the theme needs no second push.
//!
//! The system's colour scheme cannot be forced from a test (Slint's write path is for backends),
//! so "the scheme stays in Slint" is checked by switching `ChromePalette.theme` after one push.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;

use mercury_appearance::palettes;
use mercury_appearance::{Colour, Metrics, Palette, Role, Scheme};
use mercury_ui::{ChromePalette, ChromeTheme, Look, MainWindow, apply_look};
use slint::ComponentHandle;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

fn init_platform() {
    let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
}

/// What `ChromePalette` holds for `role` now, with its alpha.
fn read_back(palette: &ChromePalette<'_>, role: Role) -> Colour {
    let colour = match role {
        Role::WindowBackground => palette.get_window_background(),
        Role::ToolbarBackground => palette.get_toolbar_background(),
        Role::ToolbarDivider => palette.get_toolbar_divider(),
        Role::ContentBackground => palette.get_content_background(),
        Role::FieldBackground => palette.get_field_background(),
        Role::FieldBorder => palette.get_field_border(),
        Role::SearchFieldBackground => palette.get_search_field_background(),
        Role::SearchFieldBorder => palette.get_search_field_border(),
        Role::Text => palette.get_text(),
        Role::TextSecondary => palette.get_text_secondary(),
        Role::TextMuted => palette.get_text_muted(),
        Role::Placeholder => palette.get_placeholder(),
        Role::Accent => palette.get_accent(),
        Role::ButtonLabel => palette.get_button_label(),
        Role::ButtonLabelHover => palette.get_button_label_hover(),
        Role::ButtonHover => palette.get_button_hover(),
        Role::ButtonPressed => palette.get_button_pressed(),
        Role::TabActive => palette.get_tab_active(),
        Role::TabHover => palette.get_tab_hover(),
        Role::TabTitleInactive => palette.get_tab_title_inactive(),
        Role::TabCloseHover => palette.get_tab_close_hover(),
        Role::TabClosePressed => palette.get_tab_close_pressed(),
        Role::TabCloseLabel => palette.get_tab_close_label(),
        Role::StatusBackground => palette.get_status_background(),
        Role::StatusBorder => palette.get_status_border(),
        Role::StatusText => palette.get_status_text(),
        Role::NoticeBackground => palette.get_notice_background(),
        Role::NoticeBorder => palette.get_notice_border(),
        Role::InfoBackground => palette.get_info_background(),
        Role::InfoBorder => palette.get_info_border(),
        Role::InfoText => palette.get_info_text(),
        Role::PopupBackground => palette.get_popup_background(),
        Role::PopupBorder => palette.get_popup_border(),
        Role::ItemHover => palette.get_item_hover(),
        Role::ItemSelected => palette.get_item_selected(),
        Role::ItemSelectedText => palette.get_item_selected_text(),
        Role::Separator => palette.get_separator(),
        Role::ShortcutText => palette.get_shortcut_text(),
        Role::DisabledText => palette.get_disabled_text(),
        Role::Scrim => palette.get_scrim(),
        Role::SheetBackground => palette.get_sheet_background(),
        Role::Shadow => palette.get_shadow(),
        Role::PrimaryButtonBackground => palette.get_primary_button_background(),
        Role::PrimaryButtonHover => palette.get_primary_button_hover(),
        Role::PrimaryButtonPressed => palette.get_primary_button_pressed(),
        Role::PrimaryButtonText => palette.get_primary_button_text(),
        Role::Danger => palette.get_danger(),
        Role::FocusRing => palette.get_focus_ring(),
        Role::MatchHighlight => palette.get_match_highlight(),
        Role::PagePopupBackground => palette.get_page_popup_background(),
        Role::PagePopupBorder => palette.get_page_popup_border(),
        Role::PagePopupCaption => palette.get_page_popup_caption(),
        Role::CaptureIndicator => palette.get_capture_indicator(),
        Role::CaptureIndicatorGlyph => palette.get_capture_indicator_glyph(),
    };
    Colour::rgba(colour.red(), colour.green(), colour.blue(), colour.alpha())
}

fn read_all(window: &MainWindow, theme: ChromeTheme) -> Vec<(Role, Colour)> {
    let palette = window.global::<ChromePalette>();
    palette.set_theme(theme);
    Role::ALL
        .into_iter()
        .map(|role| (role, read_back(&palette, role)))
        .collect()
}

fn palette(id: &str) -> &'static Palette {
    palettes::find(id).expect("a palette this build has")
}

fn look(light: &str, dark: &str) -> Look {
    Look::new(palette(light), palette(dark), Metrics::ROOMY)
}

fn expected(palette: &Palette, scheme: Scheme) -> Vec<(Role, Colour)> {
    palette.colours(scheme).collect()
}

fn palette_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/theme/chrome-palette.slint");
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The file with `//` comments removed. (Nothing in it has a `//` inside a string.)
fn without_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The names declared as `out property <color> NAME:`.
fn out_properties(source: &str) -> Vec<(String, String)> {
    const MARKER: &str = "out property <color>";
    without_comments(source)
        .split(';')
        .filter_map(|statement| {
            let (_, declaration) = statement.split_once(MARKER)?;
            let (name, value) = declaration.split_once(':')?;
            Some((
                name.trim().to_owned(),
                value.split_whitespace().collect::<Vec<_>>().join(" "),
            ))
        })
        .collect()
}

/// The text between the braces that follow `start` (no nesting inside).
fn block<'a>(source: &'a str, start: &str) -> &'a str {
    let after = source
        .split_once(start)
        .unwrap_or_else(|| panic!("chrome-palette.slint has no `{start}`"))
        .1;
    let open = after.find('{').expect("an opening brace");
    let close = after[open..].find('}').expect("a closing brace") + open;
    &after[open + 1..close]
}

/// The `name:` keys of a block, in order.
fn keys(block: &str) -> Vec<String> {
    without_comments(block)
        .split(',')
        .filter_map(|entry| {
            entry
                .split_once(':')
                .map(|(name, _)| name.trim().to_owned())
        })
        .filter(|name| !name.is_empty())
        .collect()
}

fn missing(
    label: &str,
    from: &BTreeSet<String>,
    other: &BTreeSet<String>,
    fix: &str,
    out: &mut Vec<String>,
) {
    for name in from.difference(other) {
        out.push(format!("{label} lacks `{name}`: {fix}"));
    }
}

#[test]
fn the_roles_the_fields_and_the_tables_are_one_set() {
    let source = palette_source();
    let code = without_comments(&source);
    let properties = out_properties(&source);
    let roles: BTreeSet<String> = properties.iter().map(|(name, _)| name.clone()).collect();
    let fields: BTreeSet<String> = keys(block(&code, "export struct ChromeColors"))
        .into_iter()
        .collect();
    let variants: BTreeSet<String> = Role::ALL
        .iter()
        .map(|role| role.slint_name().to_owned())
        .collect();

    let mut problems = Vec::new();
    let sets = [
        ("an `out property <color>` in chrome-palette.slint", &roles),
        ("the `ChromeColors` struct", &fields),
        ("`Role` (mercury-appearance/src/palette.rs)", &variants),
    ];
    for (label, set) in sets {
        for (other_label, other) in sets {
            if label != other_label {
                missing(
                    label,
                    other,
                    set,
                    "add it to all of: a `ChromeColors` field, both `light-colours` and `dark-colours` \
                     literals, the `out property`, a `Role` variant, both palettes and a contrast rule",
                    &mut problems,
                );
            }
        }
    }
    for table in ["light-colours", "dark-colours"] {
        let listed = keys(block(&code, &format!("in property <ChromeColors> {table}")));
        let unique: BTreeSet<String> = listed.iter().cloned().collect();
        if unique.len() != listed.len() {
            problems.push(format!("`{table}` lists a colour twice"));
        }
        missing(
            &format!("`{table}`"),
            &fields,
            &unique,
            "a missing entry draws as transparent",
            &mut problems,
        );
        missing(
            "`ChromeColors`",
            &unique,
            &fields,
            "the table sets a colour that is no field",
            &mut problems,
        );
    }
    for (name, value) in &properties {
        let want = format!("root.dark ? root.dark-colours.{name} : root.light-colours.{name}");
        if *value != want {
            problems.push(format!("`{name}` is `{value}`, expected `{want}`"));
        }
    }
    problems.sort();
    problems.dedup();
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
    assert_eq!(roles.len(), Role::COUNT);
}

#[test]
fn pushing_hermes_changes_no_role_in_either_scheme() {
    init_platform();
    let window = MainWindow::new().expect("a window");
    for theme in [ChromeTheme::Light, ChromeTheme::Dark] {
        let built_in = read_all(&window, theme);
        apply_look(&window, &Look::hermes_roomy());
        let pushed = read_all(&window, theme);
        assert_eq!(built_in.len(), Role::COUNT);
        for (before, after) in built_in.iter().zip(&pushed) {
            assert_eq!(before, after, "{:?} in {theme:?}", before.0);
        }
    }
}

#[test]
fn the_built_in_tables_are_the_hermes_palette() {
    init_platform();
    let window = MainWindow::new().expect("a window");
    let hermes = palette("hermes");
    assert_eq!(
        read_all(&window, ChromeTheme::Light),
        expected(hermes, Scheme::Light)
    );
    assert_eq!(
        read_all(&window, ChromeTheme::Dark),
        expected(hermes, Scheme::Dark)
    );
}

#[test]
fn pushing_b2_gives_b2s_colours_with_their_alpha() {
    init_platform();
    let window = MainWindow::new().expect("a window");
    apply_look(&window, &look("mercury-b2", "mercury-b2"));
    let b2 = palette("mercury-b2");
    assert_eq!(
        read_all(&window, ChromeTheme::Light),
        expected(b2, Scheme::Light)
    );
    assert_eq!(
        read_all(&window, ChromeTheme::Dark),
        expected(b2, Scheme::Dark)
    );
    let hermes = palette("hermes");
    assert_ne!(
        read_all(&window, ChromeTheme::Light),
        expected(hermes, Scheme::Light),
        "B2 is not the built-in look"
    );
}

#[test]
fn the_light_table_comes_from_palette_light_and_the_dark_table_from_palette_dark() {
    init_platform();
    let window = MainWindow::new().expect("a window");
    apply_look(&window, &look("mercury-b2", "hermes"));
    assert_eq!(
        read_all(&window, ChromeTheme::Light),
        expected(palette("mercury-b2"), Scheme::Light)
    );
    assert_eq!(
        read_all(&window, ChromeTheme::Dark),
        expected(palette("hermes"), Scheme::Dark)
    );
}

#[test]
fn the_scheme_is_chosen_in_slint_without_a_second_push() {
    init_platform();
    let window = MainWindow::new().expect("a window");
    apply_look(&window, &look("mercury-b2", "mercury-b2"));
    let b2 = palette("mercury-b2");
    let palette_global = window.global::<ChromePalette>();

    palette_global.set_theme(ChromeTheme::Light);
    assert!(!palette_global.get_dark());
    assert_eq!(
        read_back(&palette_global, Role::Text),
        b2.colour(Scheme::Light, Role::Text)
    );
    palette_global.set_theme(ChromeTheme::Dark);
    assert!(palette_global.get_dark());
    assert_eq!(
        read_back(&palette_global, Role::Text),
        b2.colour(Scheme::Dark, Role::Text)
    );
    palette_global.set_theme(ChromeTheme::Light);
    assert_eq!(
        read_back(&palette_global, Role::Text),
        b2.colour(Scheme::Light, Role::Text)
    );

    // With no system scheme to follow (a platform that reports none) the chrome stays dark.
    palette_global.set_theme(ChromeTheme::System);
    assert!(palette_global.get_dark());
}

#[test]
fn pushing_twice_changes_nothing_and_pushing_hermes_restores_the_built_in_look() {
    init_platform();
    let window = MainWindow::new().expect("a window");
    let built_in = (
        read_all(&window, ChromeTheme::Light),
        read_all(&window, ChromeTheme::Dark),
    );
    let b2 = look("mercury-b2", "mercury-b2");

    apply_look(&window, &b2);
    let once = (
        read_all(&window, ChromeTheme::Light),
        read_all(&window, ChromeTheme::Dark),
    );
    apply_look(&window, &b2);
    let twice = (
        read_all(&window, ChromeTheme::Light),
        read_all(&window, ChromeTheme::Dark),
    );
    assert_ne!(built_in, once);
    assert_eq!(once, twice);

    apply_look(&window, &Look::hermes_roomy());
    let restored = (
        read_all(&window, ChromeTheme::Light),
        read_all(&window, ChromeTheme::Dark),
    );
    assert_eq!(built_in, restored);
}

#[test]
fn another_window_keeps_its_own_colours() {
    init_platform();
    let first = MainWindow::new().expect("a window");
    let second = MainWindow::new().expect("a window");
    let before = read_all(&second, ChromeTheme::Light);
    apply_look(&first, &look("mercury-b2", "mercury-b2"));
    assert_eq!(read_all(&second, ChromeTheme::Light), before);
}
