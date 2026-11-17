//! Renders the camera and microphone UI (media-capture lane) headlessly with Slint's software
//! renderer: the tab mark, the button beside the address bar with its bubble and Stop, and the
//! permission prompt's camera and microphone form ("Remember for this site until you quit", and
//! the notice when macOS refuses). Set `MERCURY_UI_RENDER_DIR` to save each rendering as a PNG.

use std::cell::RefCell;
use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize};

slint::slint! {
    import { PageRequestStyle, PageRequestView, PermissionPrompt } from "../ui/dialogs/page-requests.slint";
    import { CaptureBubble, CaptureButton, CaptureTabMark, CaptureView } from "../ui/capture/capture-indicator.slint";
    import { ChromePalette, ChromeTheme } from "../ui/theme/chrome-palette.slint";

    export { CaptureView, ChromePalette, ChromeTheme, PageRequestStyle, PageRequestView }

    // A stand-in for the browser window: a tab, the toolbar with the capture button, the page.
    export component Harness inherits Window {
        in property <PageRequestView> request;
        in property <CaptureView> capture;
        in-out property <bool> bubble-open;
        callback answered(int, bool, bool);
        callback stop();

        width: 900px;
        height: 520px;
        background: ChromePalette.window-background;

        Rectangle {
            x: 10px;
            y: 7px;
            width: 184px;
            height: 38px;
            border-radius: 8px;
            background: ChromePalette.tab-active;

            Text {
                x: 12px;
                text: "Capture test";
                color: ChromePalette.text;
                font-size: 13px;
                vertical-alignment: center;
            }

            if root.capture.heading != "": CaptureTabMark {
                x: parent.width - self.width - 12px;
                y: (parent.height - self.height) / 2;
                camera: root.capture.camera;
                microphone: root.capture.microphone;
            }
        }

        Rectangle {
            y: 48px;
            height: 60px;
            background: ChromePalette.toolbar-background;

            Rectangle {
                x: 146px;
                y: 11px;
                width: parent.width - 240px;
                height: 38px;
                border-radius: 9px;
                border-width: 1px;
                border-color: ChromePalette.field-border;
                background: ChromePalette.field-background;

                Text {
                    x: 14px;
                    text: "http://127.0.0.1:8123/pages/capture.html";
                    color: ChromePalette.text;
                    font-size: 14px;
                    vertical-alignment: center;
                }
            }

            if root.capture.heading != "": button := CaptureButton {
                x: parent.width - 80px;
                y: 16px;
                capture: root.capture;
                clicked => {
                    root.bubble-open = !root.bubble-open;
                }
            }
        }

        content := Rectangle {
            y: 108px;
            height: parent.height - 108px;
            background: #ffffff;

            Text {
                x: 24px;
                y: 24px;
                text: "Page content";
                color: #000000;
                font-size: 26px;
            }

            if root.bubble-open && root.capture.heading != "": CaptureBubble {
                capture: root.capture;
                anchor-x: 900px - 80px + 26px;
                stop => {
                    root.bubble-open = false;
                    root.stop();
                }
                closed => {
                    root.bubble-open = false;
                }
            }

            if root.request.style == PageRequestStyle.permission: PermissionPrompt {
                request: root.request;
                anchor-x: 146px;
                answered(id, accepted, checked) => {
                    root.answered(id, accepted, checked);
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
const HEIGHT: u32 = 1040;

struct Fixture {
    prefix: RefCell<String>,
    window: Rc<MinimalSoftwareWindow>,
    harness: Harness,
    answers: Rc<RefCell<Vec<(i32, bool, bool)>>>,
    stops: Rc<RefCell<u32>>,
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
        let answers = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&answers);
        harness.on_answered(move |id, accepted, checked| {
            recorded.borrow_mut().push((id, accepted, checked));
        });
        let stops = Rc::new(RefCell::new(0));
        let counted = Rc::clone(&stops);
        harness.on_stop(move || *counted.borrow_mut() += 1);
        Self {
            prefix: RefCell::default(),
            window,
            harness,
            answers,
            stops,
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

    fn save(&self, name: &str) -> Vec<PremultipliedRgbaColor> {
        let pixels = self.render();
        if let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR") {
            let bytes = pixels
                .iter()
                .flat_map(|pixel| [pixel.red, pixel.green, pixel.blue, 255])
                .collect::<Vec<_>>();
            let path = std::path::Path::new(&directory)
                .join(format!("capture-{}-{name}.png", self.prefix.borrow()));
            image::save_buffer(
                &path,
                &bytes,
                WIDTH,
                HEIGHT,
                image::ExtendedColorType::Rgba8,
            )
            .expect("the PNG is written");
        }
        pixels
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
}

fn pixel_at(pixels: &[PremultipliedRgbaColor], x: f32, y: f32) -> (u8, u8, u8) {
    let index = (y * SCALE) as usize * WIDTH as usize + (x * SCALE) as usize;
    let pixel = pixels[index];
    (pixel.red, pixel.green, pixel.blue)
}

fn capture(camera: bool, microphone: bool, heading: &str) -> CaptureView {
    CaptureView {
        camera,
        microphone,
        heading: heading.into(),
    }
}

fn prompt(id: i32, heading: &str) -> PageRequestView {
    PageRequestView {
        id,
        style: PageRequestStyle::Permission,
        heading: heading.into(),
        accept_label: "Allow".into(),
        decline_label: "Block".into(),
        choice_label: "Remember for this site until you quit".into(),
        ..PageRequestView::default()
    }
}

/// Finds the first pixel of `colour` (within 2 per channel) in a horizontal band, scanning left
/// to right, so that clicks land on the rendered buttons and not on guessed coordinates.
fn find_colour(
    pixels: &[PremultipliedRgbaColor],
    colour: (u8, u8, u8),
    y_range: std::ops::Range<f32>,
) -> Option<(f32, f32)> {
    let close = |a: u8, b: u8| a.abs_diff(b) <= 2;
    let mut y = y_range.start;
    while y < y_range.end {
        let mut x = 0.0;
        while x < 900.0 {
            let pixel = pixel_at(pixels, x, y);
            if close(pixel.0, colour.0) && close(pixel.1, colour.1) && close(pixel.2, colour.2) {
                return Some((x, y));
            }
            x += 1.0;
        }
        y += 1.0;
    }
    None
}

// Slint's platform is set once per thread, so every check runs in this one test, once per theme.
#[test]
fn capture_components_render_and_answer() {
    let fixture = Fixture::new();
    for (theme, name) in [
        (ChromeTheme::System, "system"),
        (ChromeTheme::Light, "light"),
        (ChromeTheme::Dark, "dark"),
    ] {
        fixture.harness.global::<ChromePalette>().set_theme(theme);
        *fixture.prefix.borrow_mut() = name.to_owned();
        render_and_answer(&fixture);
    }
}

fn render_and_answer(fixture: &Fixture) {
    let palette = fixture.harness.global::<ChromePalette>();
    let red = palette.get_capture_indicator();
    let red = (red.red(), red.green(), red.blue());
    let primary = palette.get_primary_button_background();
    let primary = (primary.red(), primary.green(), primary.blue());

    // Nothing captured: no mark anywhere.
    fixture.harness.set_capture(CaptureView::default());
    fixture.harness.set_bubble_open(false);
    fixture.harness.set_request(PageRequestView::default());
    fixture.settle();
    let pixels = fixture.save("00-none");
    assert!(
        find_colour(&pixels, red, 0.0..108.0).is_none(),
        "no red mark"
    );

    // The combined prompt with "Remember for this site until you quit", unticked.
    fixture.harness.set_request(prompt(
        7,
        "127.0.0.1:8123 wants to use your camera and microphone",
    ));
    fixture.settle();
    let pixels = fixture.save("01-prompt-camera-and-microphone");
    let (allow_x, allow_y) =
        find_colour(&pixels, primary, 120.0..400.0).expect("the Allow button is drawn");
    // Tick the choice (its box is at the bubble's left edge, under the heading), then Allow.
    fixture.click(170.0, allow_y - 23.0);
    let pixels = fixture.save("02-prompt-remember-ticked");
    assert!(
        find_colour(&pixels, primary, 120.0..400.0).is_some(),
        "still shown"
    );
    fixture.click(allow_x + 10.0, allow_y + 6.0);
    assert_eq!(
        fixture.answers.borrow_mut().split_off(0),
        vec![(7, true, true)]
    );

    // A new prompt starts unticked: Allow alone does not remember.
    fixture.harness.set_request(PageRequestView::default());
    fixture.settle();
    fixture
        .harness
        .set_request(prompt(8, "127.0.0.1:8123 wants to use your camera"));
    fixture.settle();
    let pixels = fixture.save("03-prompt-camera");
    let (allow_x, allow_y) =
        find_colour(&pixels, primary, 120.0..400.0).expect("the Allow button is drawn");
    fixture.click(allow_x + 10.0, allow_y + 6.0);
    assert_eq!(
        fixture.answers.borrow_mut().split_off(0),
        vec![(8, true, false)]
    );

    // macOS refuses: the notice, no choice.
    fixture.harness.set_request(PageRequestView {
        message: "macOS does not allow Mercury Browser to use the camera. You can allow it in \
                  System Settings, Privacy & Security, Camera."
            .into(),
        accept_label: "Open Privacy Settings".into(),
        decline_label: "Close".into(),
        choice_label: "".into(),
        ..prompt(9, "127.0.0.1:8123 wants to use your camera")
    });
    fixture.settle();
    fixture.save("04-prompt-system-denied");
    fixture.harness.set_request(PageRequestView::default());

    // A page with access: the red mark on the tab and the button beside the address bar.
    fixture.harness.set_capture(capture(
        true,
        true,
        "127.0.0.1:8123 can use your camera and microphone",
    ));
    fixture.settle();
    let pixels = fixture.save("05-marks");
    assert!(
        find_colour(&pixels, red, 10.0..45.0).is_some(),
        "the tab's mark is drawn"
    );
    let (button_x, button_y) =
        find_colour(&pixels, red, 60.0..100.0).expect("the capture button is drawn");

    // Its bubble, with Close and Stop.
    fixture.click(button_x + 6.0, button_y + 4.0);
    assert!(fixture.harness.get_bubble_open());
    let pixels = fixture.save("06-bubble");
    let (stop_x, stop_y) =
        find_colour(&pixels, primary, 108.0..400.0).expect("the Stop button is drawn");
    fixture.click(stop_x + 10.0, stop_y + 6.0);
    assert_eq!(*fixture.stops.borrow(), 1);
    assert!(!fixture.harness.get_bubble_open());
    *fixture.stops.borrow_mut() = 0;

    // A click beside the bubble closes it without stopping.
    fixture.click(button_x + 6.0, button_y + 4.0);
    assert!(fixture.harness.get_bubble_open());
    fixture.click(100.0, 480.0);
    assert!(!fixture.harness.get_bubble_open());
    assert_eq!(*fixture.stops.borrow(), 0);

    // The microphone alone.
    fixture.harness.set_capture(capture(
        false,
        true,
        "127.0.0.1:8123 can use your microphone",
    ));
    fixture.settle();
    fixture.save("07-microphone-only");
    fixture.harness.set_capture(CaptureView::default());
}
