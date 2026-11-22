// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `Look`: what the stored appearance settings resolve to, and that `apply_look` reaches every
//! kind of window that takes the chrome theme (`settings::apply_chrome_theme` is generic over
//! them, so each must have both globals).

use std::rc::Rc;

use mercury_appearance::resolve::AppearanceSettings;
use mercury_appearance::{Metrics, Role, Scheme, palettes};
use mercury_ui::{
    AboutWindow, BookmarksManager, ChromePalette, ChromeTheme, Look, MainWindow, SettingsWindow,
    apply_look,
};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{Color, ComponentHandle};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

fn init_platform() {
    let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
}

fn appearance(light: &str, dark: &str) -> AppearanceSettings {
    AppearanceSettings {
        palette_light: light.to_owned(),
        palette_dark: dark.to_owned(),
        ..AppearanceSettings::default()
    }
}

fn ids(look: &Look) -> (&'static str, &'static str) {
    (look.palette_light.id, look.palette_dark.id)
}

#[test]
fn the_default_settings_give_signal_and_the_roomy_sizes_without_notes() {
    let (look, notes) = Look::from_appearance(&AppearanceSettings::default());
    assert_eq!(ids(&look), ("signal", "signal"));
    assert_eq!(look.metrics, Metrics::ROOMY);
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn an_unknown_palette_id_gives_signal_with_one_note_and_changes_nothing() {
    let stored = appearance("no-such-palette", "signal");
    let before = stored.clone();
    let (look, notes) = Look::from_appearance(&stored);
    assert_eq!(ids(&look), ("signal", "signal"));
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].field, "appearance.palette_light");
    assert!(notes[0].reason.contains("no-such-palette"), "{notes:?}");
    assert_eq!(stored, before, "the setting is read, never rewritten");
}

#[test]
fn hermes_cannot_be_chosen_by_hand_but_the_gate_can_ask_for_it() {
    let (look, notes) = Look::from_appearance(&appearance("hermes", "hermes"));
    assert_eq!(ids(&look), ("signal", "signal"));
    assert_eq!(notes.len(), 2, "{notes:?}");
    for id in ["mercury-b2", "graphite", "midnight"] {
        let (look, notes) = Look::from_appearance(&appearance(id, id));
        assert_eq!(ids(&look), ("signal", "signal"), "{id}");
        assert_eq!(notes.len(), 2, "{id}: {notes:?}");
    }

    let hermes = Look::hermes_roomy();
    assert_eq!(ids(&hermes), ("hermes", "hermes"));
    assert_eq!(hermes.metrics, Metrics::ROOMY);
}

#[test]
fn two_different_palettes_give_a_mixed_look() {
    let (look, notes) = Look::from_appearance(&appearance("forest", "glacier"));
    assert!(notes.is_empty());
    assert_eq!(ids(&look), ("forest", "glacier"));
    let mixed = Look::new(
        look.palette_light,
        palettes::find("hermes").expect("hermes exists"),
        look.metrics,
    );
    assert_eq!(ids(&mixed), ("forest", "hermes"));
    assert!(look.palette_light.id != mixed.palette_dark.id);
}

fn background(palette: &ChromePalette<'_>, theme: ChromeTheme) -> Color {
    palette.set_theme(theme);
    palette.get_window_background()
}

/// The `window-background` the look has for each scheme, as Slint colours.
fn expected_backgrounds(look: &Look) -> (Color, Color) {
    let colour = |palette: &mercury_appearance::Palette, scheme| {
        let c = palette.colour(scheme, Role::WindowBackground);
        Color::from_argb_u8(c.a, c.r, c.g, c.b)
    };
    (
        colour(look.palette_light, Scheme::Light),
        colour(look.palette_dark, Scheme::Dark),
    )
}

/// `apply_look` compiles for the window and its `ChromePalette` shows the look afterwards.
fn pushes<C>(window: &C)
where
    C: ComponentHandle,
    for<'a> ChromePalette<'a>: slint::Global<'a, C>,
    for<'a> mercury_ui::ChromeMetrics<'a>: slint::Global<'a, C>,
{
    let (look, _) = Look::from_appearance(&AppearanceSettings::default());
    let (light, dark) = expected_backgrounds(&look);
    let palette = window.global::<ChromePalette>();
    assert_ne!(
        background(&palette, ChromeTheme::Light),
        light,
        "the window starts with the built-in look"
    );
    apply_look(window, &look);
    assert_eq!(background(&palette, ChromeTheme::Light), light);
    assert_eq!(background(&palette, ChromeTheme::Dark), dark);
}

#[test]
fn apply_look_reaches_every_window_that_takes_the_chrome_theme() {
    init_platform();
    pushes(&MainWindow::new().expect("a window"));
    pushes(&AboutWindow::new().expect("a window"));
    pushes(&SettingsWindow::new().expect("a window"));
    pushes(&BookmarksManager::new().expect("a window"));
}
