// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the find bar headlessly with Slint's software renderer and checks how it answers keys
//! and clicks. Set `MERCURY_UI_RENDER_DIR` to also save each rendering as a PNG.

use std::cell::RefCell;
use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, SharedString};

slint::slint! {
    import { FindBar } from "../ui/find/find-bar.slint";
    import { ChromePalette, ChromeTheme } from "../ui/theme/chrome-palette.slint";

    export { ChromePalette, ChromeTheme }

    // A stand-in for the browser window: toolbar, the find bar, then a white page.
    export component Harness inherits Window {
        in property <bool> shown: true;
        in-out property <string> query;
        in property <string> status;
        in property <bool> no-matches;
        in property <bool> match-case;
        out property <bool> field-focused: bar.field-focused;
        callback edited(string);
        callback next();
        callback previous();
        callback closed();
        callback toggle-match-case();

        public function focus-field() {
            bar.focus-field();
        }

        width: 900px;
        height: 200px;
        background: ChromePalette.window-background;

        VerticalLayout {
            Rectangle {
                height: 60px;
                background: ChromePalette.toolbar-background;
            }

            bar := FindBar {
                height: root.shown ? 42px : 0px;
                shown: root.shown;
                query <=> root.query;
                status: root.status;
                no-matches: root.no-matches;
                match-case: root.match-case;
                edited(text) => { root.edited(text); }
                next => { root.next(); }
                previous => { root.previous(); }
                close => { root.closed(); }
                toggle-match-case => { root.toggle-match-case(); }
            }

            Rectangle {
                background: #ffffff;

                Text {
                    x: 24px;
                    y: 16px;
                    text: "Page content under the find bar";
                    color: #000000;
                    font-size: 22px;
                }
            }
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

const SCALE: f32 = 2.0;
const WIDTH: u32 = 1800;
const HEIGHT: u32 = 400;

/// The checked "Aa" button, in image pixels: x from 908 to 976, y from 132 to 188.
const TOGGLE_LEFT: u32 = 912;
const TOGGLE_RIGHT: u32 = 972;
const TOGGLE_TOP: u32 = 136;
const TOGGLE_BOTTOM: u32 = 184;

/// WCAG relative luminance of a pixel.
fn luminance(pixel: &PremultipliedRgbaColor) -> f64 {
    let channel = |value: u8| {
        let value = f64::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(pixel.red) + 0.7152 * channel(pixel.green) + 0.0722 * channel(pixel.blue)
}

/// The contrast between the fill of the checked "Aa" button (read at its top edge, above the
/// label) and the pixel of its area that differs most from that fill: the label.
fn checked_toggle_label_contrast(pixels: &[PremultipliedRgbaColor]) -> f64 {
    let at = |x: u32, y: u32| &pixels[(y * WIDTH + x) as usize];
    let fill = luminance(at((TOGGLE_LEFT + TOGGLE_RIGHT) / 2, TOGGLE_TOP));
    let mut most = 1.0_f64;
    for y in TOGGLE_TOP..TOGGLE_BOTTOM {
        for x in TOGGLE_LEFT..TOGGLE_RIGHT {
            let other = luminance(at(x, y));
            let (light, dark) = if other > fill {
                (other, fill)
            } else {
                (fill, other)
            };
            most = most.max((light + 0.05) / (dark + 0.05));
        }
    }
    most
}

struct Fixture {
    window: Rc<MinimalSoftwareWindow>,
    harness: Harness,
    calls: Rc<RefCell<Vec<String>>>,
}

impl Fixture {
    fn new() -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform(window.clone())))
            .expect("the platform is set once");
        let harness = Harness::new().expect("the harness builds");
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: SCALE,
        });
        window.set_size(PhysicalSize::new(WIDTH, HEIGHT));
        harness.show().expect("the harness shows");
        let calls = Rc::new(RefCell::new(Vec::new()));
        let record = |name: &'static str| {
            let calls = Rc::clone(&calls);
            move || calls.borrow_mut().push(name.to_owned())
        };
        harness.on_next(record("next"));
        harness.on_previous(record("previous"));
        harness.on_closed(record("close"));
        harness.on_toggle_match_case(record("match case"));
        let edited = Rc::clone(&calls);
        harness.on_edited(move |text| edited.borrow_mut().push(format!("edited {text}")));
        Self {
            window,
            harness,
            calls,
        }
    }

    fn settle(&self) {
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            let _ = self.render();
        }
    }

    fn render(&self) -> Vec<PremultipliedRgbaColor> {
        self.window.request_redraw();
        let mut buffer = vec![PremultipliedRgbaColor::default(); (WIDTH * HEIGHT) as usize];
        self.window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, WIDTH as usize);
        });
        buffer
    }

    fn save(&self, name: &str) {
        self.settle();
        let pixels = self.render();
        if let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR") {
            let bytes = pixels
                .iter()
                .flat_map(|pixel| [pixel.red, pixel.green, pixel.blue, 255])
                .collect::<Vec<_>>();
            let path = std::path::Path::new(&directory).join(format!("{name}.png"));
            image::save_buffer(
                &path,
                &bytes,
                WIDTH,
                HEIGHT,
                image::ExtendedColorType::Rgba8,
            )
            .expect("the PNG is written");
        }
    }

    fn press(&self, key: impl Into<SharedString>) {
        self.window
            .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    }

    fn release(&self, key: impl Into<SharedString>) {
        self.window
            .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    }

    fn key(&self, key: impl Into<SharedString>) {
        let key = key.into();
        self.press(key.clone());
        self.release(key);
        self.settle();
    }

    fn click(&self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.window.dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        self.window.dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
        self.settle();
    }

    fn take_calls(&self) -> Vec<String> {
        std::mem::take(&mut self.calls.borrow_mut())
    }
}

// One test: Slint's platform is global to the process.
#[test]
fn the_find_bar_renders_and_answers_keys_and_clicks() {
    let fixture = Fixture::new();
    let harness = &fixture.harness;

    // Empty, focused: the placeholder shows.
    harness.invoke_focus_field();
    fixture.save("find-bar-empty");
    assert!(harness.get_field_focused());

    // Typing reports each edit.
    fixture.key("n");
    fixture.key("e");
    assert_eq!(fixture.take_calls(), vec!["edited n", "edited ne"]);

    // Return: next; Shift+Return: previous; Escape: close.
    fixture.key(Key::Return);
    fixture.press(Key::Shift);
    fixture.key(Key::Return);
    fixture.release(Key::Shift);
    fixture.key(Key::Escape);
    assert_eq!(fixture.take_calls(), vec!["next", "previous", "close"]);

    // A count, then match case on.
    harness.set_query("needle".into());
    harness.set_status("3 of 7".into());
    fixture.save("find-bar-count");
    harness.set_match_case(true);
    fixture.save("find-bar-match-case");

    // No matches: the field's border turns red.
    harness.set_query("absent".into());
    harness.set_status("No matches".into());
    harness.set_no_matches(true);
    harness.set_match_case(false);
    fixture.save("find-bar-no-matches");

    // The buttons, at their logical positions in the 900 px wide bar (row from y 60 to 102):
    // previous 397, next 432, match case 470, close 871 (read off the rendering).
    fixture.click(397.0, 81.0);
    fixture.click(432.0, 81.0);
    fixture.click(470.0, 81.0);
    fixture.click(871.0, 81.0);
    assert_eq!(
        fixture.take_calls(),
        vec!["previous", "next", "match case", "close"]
    );

    // Hiding the bar takes focus away from its field, so typing cannot go to a hidden field.
    harness.invoke_focus_field();
    fixture.settle();
    assert!(harness.get_field_focused());
    harness.set_shown(false);
    fixture.settle();
    assert!(!harness.get_field_focused());
    fixture.key("x");
    assert_eq!(fixture.take_calls(), Vec::<String>::new());
    harness.invoke_focus_field();
    fixture.settle();
    assert!(
        !harness.get_field_focused(),
        "a hidden bar's field cannot be focused"
    );
    fixture.save("find-bar-hidden");

    // The bar in each chrome theme. System follows the window's colour scheme, which is unknown
    // here, so the chrome stays dark.
    harness.set_shown(true);
    for (theme, name) in [
        (ChromeTheme::System, "system"),
        (ChromeTheme::Light, "light"),
        (ChromeTheme::Dark, "dark"),
    ] {
        harness.global::<ChromePalette>().set_theme(theme);
        harness.set_query("needle".into());
        harness.set_status("3 of 7".into());
        harness.set_no_matches(false);
        harness.set_match_case(true);
        fixture.save(&format!("find-bar-{name}-count"));
        // The checked "Aa" button is filled with the selection colour; its label must be
        // readable on that fill (WCAG AA, 4.5).
        let contrast = checked_toggle_label_contrast(&fixture.render());
        assert!(
            contrast >= 4.5,
            "the checked match-case label has contrast {contrast:.2} in the {name} theme"
        );
        harness.set_query("absent".into());
        harness.set_status("No matches".into());
        harness.set_no_matches(true);
        harness.set_match_case(false);
        fixture.save(&format!("find-bar-{name}-no-matches"));
    }
}
