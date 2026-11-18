// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the session bar on its own, through Slint's software renderer, to PNG files in
//! `MERCURY_UI_RENDER_DIR` (when set), so that its states that need a click can be looked at.

use std::rc::Rc;

use slint::Rgb8Pixel;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, ModelRc, PhysicalSize, SharedString, VecModel};

slint::slint! {
    import { SessionBar } from "ui/session/session-bar.slint";
    import { ChromePalette, ChromeTheme } from "ui/theme/chrome-palette.slint";
    export { ChromePalette, ChromeTheme }

    export component SessionBarPreview inherits Window {
        in property <[string]> sessions;
        in property <string> notice;
        out property <int> restored: -1;
        out property <bool> dismissed;
        width: 1000px;
        height: 260px;
        background: ChromePalette.content-background;

        public function open-chooser() {
            bar.open-chooser();
        }

        public function click-choice(index: int) {
            bar.choose(index);
        }

        VerticalLayout {
            alignment: start;
            Rectangle { height: 108px; background: ChromePalette.toolbar-background; }
            bar := SessionBar {
                sessions: root.sessions;
                notice: root.notice;
                restore(index) => { root.restored = index; }
                dismiss => { root.dismissed = true; }
            }
        }
    }
}

struct HeadlessPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

fn render(window: &MinimalSoftwareWindow, name: &str) {
    let (width, height) = (1000_u32, 260_u32);
    window.set_size(PhysicalSize::new(width, height));
    let mut buffer = vec![Rgb8Pixel::default(); (width * height) as usize];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut buffer, width as usize);
    });
    let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR") else {
        return;
    };
    let bytes: Vec<u8> = buffer.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
    image::save_buffer(
        std::path::Path::new(&directory).join(format!("{name}.png")),
        &bytes,
        width,
        height,
        image::ColorType::Rgb8,
    )
    .unwrap();
}

fn press(window: &MinimalSoftwareWindow, key: Key) {
    let text = SharedString::from(key);
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

fn labels(items: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items
            .iter()
            .map(|item| SharedString::from(*item))
            .collect::<Vec<_>>(),
    ))
}

// One test: Slint's platform can be set once per process.
#[test]
fn the_session_bar_renders_its_offer_its_chooser_and_its_notice() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(HeadlessPlatform {
        window: window.clone(),
    }))
    .unwrap();
    let preview = SessionBarPreview::new().unwrap();
    preview.show().unwrap();

    // Tab reaches Restore, Return presses it; a second Tab and Space press Dismiss.
    preview.set_sessions(labels(&["Session Alpha (1 tab)"]));
    render(&window, "session-bar-before-keys");
    press(&window, Key::Tab);
    press(&window, Key::Return);
    assert_eq!(preview.get_restored(), 0);
    press(&window, Key::Tab);
    press(&window, Key::Space);
    assert!(preview.get_dismissed());

    // System follows the window's colour scheme, unknown here: the chrome then stays dark.
    for (theme, name) in [
        (ChromeTheme::Light, "light"),
        (ChromeTheme::Dark, "dark"),
        (ChromeTheme::System, "system"),
    ] {
        preview.global::<ChromePalette>().set_theme(theme);
        preview.set_notice(SharedString::new());
        preview.set_sessions(labels(&["Session Alpha (1 tab)"]));
        render(&window, &format!("session-bar-one-{name}"));

        preview.set_sessions(labels(&[
            "Session Alpha (1 tab)",
            "Session Bravo (2 tabs)",
            "A very long page title that has to be cut short in the chooser (12 tabs)",
        ]));
        preview.invoke_open_chooser();
        render(&window, &format!("session-bar-chooser-{name}"));
        preview.invoke_click_choice(1);
        assert_eq!(preview.get_restored(), 1);

        preview.set_sessions(labels(&[]));
        preview.set_notice(
            "Your tabs won't be remembered this time: they were saved by a newer version of \
             Mercury Browser."
                .into(),
        );
        render(&window, &format!("session-bar-notice-{name}"));
    }
}
