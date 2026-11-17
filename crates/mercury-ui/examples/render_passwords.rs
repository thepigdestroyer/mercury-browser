// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the password manager's interface to PNG files without a display, through Slint's
//! software renderer: the Passwords section of the settings window (list, an opened login, the
//! editor, the delete confirmation, the generator) in each theme, and the page context menu with
//! "Fill login…" and the chooser it opens. All data is invented.
//!
//! `cargo run --release -p mercury-ui --example render_passwords -- <output directory>`

use std::cell::RefCell;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mercury_ui::{
    ChromePalette, ChromeTheme, ContextMenuRow, LoginListRow, MainWindow, PasswordManagerUi,
    SettingsWindow,
};
use slint::language::ColorScheme;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, ModelRc, PhysicalSize, Rgb8Pixel, VecModel};

const SCALE: f32 = 2.0;

struct HeadlessPlatform {
    windows: RefCell<Vec<Rc<MinimalSoftwareWindow>>>,
}

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.windows.borrow_mut().push(Rc::clone(&window));
        Ok(window)
    }
}

fn render(
    window: &slint::Window,
    width: u32,
    height: u32,
    path: &Path,
) -> Result<(), Box<dyn Error>> {
    window.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: SCALE,
    });
    window.set_size(PhysicalSize::new(width, height));
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let _ = window.take_snapshot();
    }
    let snapshot = window.take_snapshot()?;
    let bytes = snapshot
        .as_slice()
        .iter()
        .map(|pixel| Rgb8Pixel::new(pixel.r, pixel.g, pixel.b))
        .flat_map(|pixel| [pixel.r, pixel.g, pixel.b])
        .collect::<Vec<_>>();
    image::save_buffer(
        path,
        &bytes,
        snapshot.width(),
        snapshot.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    println!("wrote {}", path.display());
    Ok(())
}

fn row(site: &str, username: &str, kind: &str) -> LoginListRow {
    LoginListRow {
        site: site.into(),
        username: username.into(),
        kind: kind.into(),
    }
}

fn sample_settings_window(scheme: ColorScheme) -> Result<SettingsWindow, Box<dyn Error>> {
    let window = SettingsWindow::new()?;
    window.set_forced_color_scheme(scheme);
    window.set_section(4);
    window.set_offer_to_save_passwords(true);
    let ui = window.global::<PasswordManagerUi>();
    ui.set_storage_note("Saved in your login keychain.".into());
    ui.set_rows(ModelRc::new(VecModel::from(vec![
        row("accounts.example", "alice@example.com", ""),
        row("http://127.0.0.1:8123", "bob", "HTTP sign-in"),
        row("mail.example", "No user name", ""),
        row("shop.test", "a.very.long.user.name.that.does.not.fit@example.org", ""),
    ])));
    ui.set_total(4);
    ui.set_generated("vH7#qP2-kZ9wT4mX@cE6".into());
    Ok(window)
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_passwords <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;
    slint::platform::set_platform(Box::new(HeadlessPlatform {
        windows: RefCell::default(),
    }))
    .map_err(|error| format!("{error:?}"))?;

    for (scheme, name) in [
        (ColorScheme::Light, "light"),
        (ColorScheme::Dark, "dark"),
        (ColorScheme::Unknown, "system"),
    ] {
        let window = sample_settings_window(scheme)?;
        if scheme == ColorScheme::Unknown {
            window.set_system_color_scheme(ColorScheme::Dark);
        }
        window.show()?;
        render(
            window.window(),
            1560,
            2000,
            &output.join(format!("passwords-{name}-list.png")),
        )?;

        let ui = window.global::<PasswordManagerUi>();
        ui.set_selected(0);
        ui.set_detail_site("accounts.example".into());
        ui.set_detail_username("alice@example.com".into());
        ui.set_status("Copied the user name.".into());
        render(
            window.window(),
            1560,
            2000,
            &output.join(format!("passwords-{name}-opened.png")),
        )?;

        ui.set_password_shown(true);
        ui.set_shown_password("fake-correct-horse".into());
        ui.set_status(String::new().into());
        render(
            window.window(),
            1560,
            2000,
            &output.join(format!("passwords-{name}-revealed.png")),
        )?;

        ui.set_password_shown(false);
        ui.set_editing(true);
        ui.set_edit_username("alice@example.com".into());
        ui.set_edit_problem("A login with this user name is already saved for this site.".into());
        render(
            window.window(),
            1560,
            2000,
            &output.join(format!("passwords-{name}-editing.png")),
        )?;

        ui.set_editing(false);
        ui.set_edit_problem(String::new().into());
        ui.set_confirming_delete(true);
        render(
            window.window(),
            1560,
            2000,
            &output.join(format!("passwords-{name}-delete.png")),
        )?;
        window.hide()?;
    }

    // The page context menu over a password field, then the chooser "Fill login…" opens.
    let menu_row = |label: &str, enabled: bool, separator: bool| ContextMenuRow {
        label: label.into(),
        enabled,
        separator,
    };
    for (theme, name) in [(ChromeTheme::Light, "light"), (ChromeTheme::Dark, "dark")] {
        for (step, rows) in [
            (
                "menu",
                vec![
                    menu_row("Fill login…", true, false),
                    menu_row("", false, true),
                    menu_row("Cut", false, false),
                    menu_row("Copy", false, false),
                    menu_row("Paste", true, false),
                    menu_row("Select all", true, false),
                ],
            ),
            (
                "chooser",
                vec![
                    menu_row("Fill a login for accounts.example", false, false),
                    menu_row("", false, true),
                    menu_row("alice@example.com", true, false),
                    menu_row("bob", true, false),
                ],
            ),
        ] {
            let window = MainWindow::new()?;
            window.global::<ChromePalette>().set_theme(theme);
            let height = rows
                .iter()
                .map(|row| if row.separator { 9.0 } else { 26.0 })
                .sum::<f32>()
                + 8.0;
            window.set_controls_menu_rows(ModelRc::new(VecModel::from(rows)));
            window.set_controls_highlighted(if step == "chooser" { 2 } else { 0 });
            window.set_controls_popup_x(180.0);
            window.set_controls_popup_y(160.0);
            window.set_controls_popup_width(260.0);
            window.set_controls_popup_height(height);
            window.set_controls_popup(3);
            window.show()?;
            render(
                window.window(),
                1400,
                900,
                &output.join(format!("fill-{name}-{step}.png")),
            )?;
            window.hide()?;
        }
    }
    Ok(())
}
