// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The centre disc of the hub ring (`RingCentre`, `ui/groups/hub-ring-centre.slint`), driven through
//! `RingPopoverPreview` and the `HubRingBridge` global only: what a screen reader is told in each
//! state, what the disc fires (`query-edited`, `result-activated`, `pause-clicked`, `mute-clicked`,
//! `see-everything`), long strings, and pixel probes of the four states (rest, searching, audio,
//! ⌥ held) in the light and dark scheme through Slint's software renderer. Set
//! `MERCURY_UI_RENDER_DIR` to a directory to get the PNGs (the disc, at 2x).
//!
//! The ring around the disc is the stub scaffold of `RingBands` until lane ring-bands lands, so
//! every probe is inside the disc. The disc is 272 px wide, centred in the 516 px window: a point
//! "(x, y)" below is in the disc's own coordinates (0..272), the centre being (136, 136).
#![cfg(feature = "test-support")]

mod common;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use common::a11y::{Node, Rect, node_in, reading_order};
use i_slint_backend_testing::AccessibleRole;
use mercury_ui::{
    CentreMode, ChromePalette, ChromeTheme, HubRingBridge, RingPopoverPreview, SearchRow,
};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{
    Color, ComponentHandle, Image, LogicalPosition, ModelRc, PhysicalSize, Rgba8Pixel,
    SharedPixelBuffer, SharedString, VecModel,
};

const SCALE: f32 = 2.0;
/// The preview window is the 516 px ring.
const WINDOW: u32 = 516;
/// Where the 272 px disc starts in the window (it is centred).
const ORIGIN: f32 = 122.0;
const SIZE: usize = (WINDOW as f32 * SCALE) as usize;

thread_local! {
    static PLATFORM_READY: Cell<bool> = const { Cell::new(false) };
    static LAST_WINDOW: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
}

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        LAST_WINDOW.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&window)));
        Ok(window)
    }
}

/// What the disc fired, in order: "query tram", "result 1", "pause", "mute", "see everything".
type Log = Rc<RefCell<Vec<String>>>;

struct Fixture {
    ui: RingPopoverPreview,
    window: Rc<MinimalSoftwareWindow>,
    log: Log,
}

impl Fixture {
    fn new(dark: bool) -> Self {
        PLATFORM_READY.with(|ready| {
            if !ready.replace(true) {
                slint::platform::set_platform(Box::new(SoftwarePlatform))
                    .expect("no platform is set on this thread yet");
            }
        });
        let ui = RingPopoverPreview::new().expect("the preview window can be created");
        let window = LAST_WINDOW
            .with(|slot| slot.borrow().clone())
            .expect("the platform made the window");
        ui.global::<ChromePalette>().set_theme(if dark {
            ChromeTheme::Dark
        } else {
            ChromeTheme::Light
        });
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: SCALE,
        });
        window.set_size(PhysicalSize::new(SIZE as u32, SIZE as u32));
        ui.show().expect("the preview window can be shown");

        let log: Log = Rc::default();
        let bridge = ui.global::<HubRingBridge>();
        let record = |message: &'static str| {
            let log = Rc::clone(&log);
            move || log.borrow_mut().push(message.to_owned())
        };
        bridge.on_pause_clicked(record("pause"));
        bridge.on_mute_clicked(record("mute"));
        bridge.on_see_everything(record("see everything"));
        let edited = Rc::clone(&log);
        bridge.on_query_edited(move |text| edited.borrow_mut().push(format!("query {text}")));
        let activated = Rc::clone(&log);
        bridge.on_result_activated(move |row| {
            activated.borrow_mut().push(format!("result {row}"));
        });
        let fixture = Self { ui, window, log };
        fixture.settle();
        fixture
    }

    fn bridge(&self) -> HubRingBridge<'_> {
        self.ui.global::<HubRingBridge>()
    }

    fn palette(&self) -> ChromePalette<'_> {
        self.ui.global::<ChromePalette>()
    }

    fn settle(&self) {
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            let _ = self.render();
        }
    }

    fn render(&self) -> Shot {
        self.window.request_redraw();
        let mut buffer = vec![PremultipliedRgbaColor::default(); SIZE * SIZE];
        self.window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, SIZE);
        });
        Shot(buffer)
    }

    fn shot(&self) -> Shot {
        self.settle();
        self.render()
    }

    fn calls(&self) -> Vec<String> {
        std::mem::take(&mut self.log.borrow_mut())
    }

    /// A click at a point of the disc.
    fn click(&self, x: f32, y: f32) {
        let position = LogicalPosition::new(ORIGIN + x, ORIGIN + y);
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

    fn key(&self, text: impl Into<SharedString>) {
        let text = text.into();
        self.window
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.window
            .dispatch_event(WindowEvent::KeyReleased { text });
        self.settle();
    }

    fn nodes(&self) -> Vec<Node> {
        reading_order(&self.ui)
    }

    /// The disc's rectangle in the window, which is where every control must lie.
    fn disc() -> Rect {
        Rect::new(ORIGIN, ORIGIN, 272.0, 272.0)
    }
}

/// A rendering of the whole window at 2x.
struct Shot(Vec<PremultipliedRgbaColor>);

impl Shot {
    fn pixel(&self, x: f32, y: f32) -> (u8, u8, u8) {
        let column = ((ORIGIN + x) * SCALE) as usize;
        let row = ((ORIGIN + y) * SCALE) as usize;
        let pixel = &self.0[row * SIZE + column];
        (pixel.red, pixel.green, pixel.blue)
    }

    /// The pixels of a box (disc coordinates) as `(r, g, b)`.
    fn area(&self, (x0, x1): (f32, f32), (y0, y1): (f32, f32)) -> Vec<(u8, u8, u8)> {
        let columns = ((ORIGIN + x0) * SCALE) as usize..((ORIGIN + x1) * SCALE) as usize;
        let rows = ((ORIGIN + y0) * SCALE) as usize..((ORIGIN + y1) * SCALE) as usize;
        rows.flat_map(|row| columns.clone().map(move |column| (row, column)))
            .map(|(row, column)| {
                let pixel = &self.0[row * SIZE + column];
                (pixel.red, pixel.green, pixel.blue)
            })
            .collect()
    }

    fn is(&self, x: f32, y: f32, colour: Color, what: &str) {
        let pixel = self.pixel(x, y);
        assert!(
            close(pixel, colour, 3),
            "{what}: at ({x}, {y}) expected {colour:?}, found {pixel:?}"
        );
    }

    /// How many pixels of the box are close to `colour`.
    fn count(&self, xs: (f32, f32), ys: (f32, f32), colour: Color) -> usize {
        self.area(xs, ys)
            .into_iter()
            .filter(|pixel| close(*pixel, colour, 10))
            .count()
    }

    /// How many pixels of the box differ from `background`: the ink of whatever is drawn there.
    fn ink(&self, xs: (f32, f32), ys: (f32, f32), background: Color) -> usize {
        self.area(xs, ys)
            .into_iter()
            .filter(|pixel| !close(*pixel, background, 3))
            .count()
    }

    fn save(&self, name: &str) {
        let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR").map(PathBuf::from) else {
            return;
        };
        // The disc and 8 px round it.
        let (from, to) = (
            ((ORIGIN - 8.0) * SCALE) as usize,
            ((ORIGIN + 280.0) * SCALE) as usize,
        );
        let mut bytes = Vec::new();
        for row in from..to {
            for column in from..to {
                let pixel = &self.0[row * SIZE + column];
                bytes.extend([pixel.red, pixel.green, pixel.blue]);
            }
        }
        std::fs::create_dir_all(&directory).expect("the render directory can be made");
        image::save_buffer(
            directory.join(name),
            &bytes,
            (to - from) as u32,
            (to - from) as u32,
            image::ExtendedColorType::Rgb8,
        )
        .expect("the PNG is written");
    }
}

fn close((r, g, b): (u8, u8, u8), colour: Color, tolerance: u8) -> bool {
    r.abs_diff(colour.red()) <= tolerance
        && g.abs_diff(colour.green()) <= tolerance
        && b.abs_diff(colour.blue()) <= tolerance
}

fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

fn row(tab: i32, title: &str, path: &str, selected: bool) -> SearchRow {
    SearchRow {
        tab,
        title: title.into(),
        path: path.into(),
        selected,
    }
}

// The four scenes of the mockups (design/ui-final/ring/ring-{open,search,keys,status}-*.svg).

/// ring-open: a tab under the pointer, no snapshot yet.
fn open_scene(f: &Fixture) {
    let bridge = f.bridge();
    bridge.set_mode(CentreMode::Rest);
    bridge.set_title("Essay on maps".into());
    bridge.set_subtitle("Reading · ⌥D 2 · essays.example.net".into());
    bridge.set_has_preview(false);
    bridge.set_preview_mark(Color::from_rgb_u8(0xEB, 0xC9, 0xB9));
    bridge.set_field_placeholder("Search 22 tabs".into());
    bridge.set_footer("Hold ⌥ to see group keys".into());
    bridge.set_query("".into());
}

/// ring-search: "tram", one result (selected) and the web row.
fn search_scene(f: &Fixture) {
    let bridge = f.bridge();
    bridge.set_mode(CentreMode::Searching);
    bridge.set_query("tram".into());
    bridge.set_field_placeholder("Search 22 tabs".into());
    bridge.set_results(model(vec![
        row(7, "Tram 28 timetable", "Lisbon trip · ⌥S 3 · ⏎ opens", true),
        row(-1, "Search the web for “tram”", "", false),
    ]));
    bridge.set_footer("↑ ↓ choose · esc clears".into());
}

/// ring-keys: ⌥ S held.
fn keys_scene(f: &Fixture) {
    let bridge = f.bridge();
    bridge.set_mode(CentreMode::Held);
    bridge.set_held_key("⌥ S".into());
    bridge.set_held_name("Lisbon trip".into());
    bridge.set_held_help("press 1–4 for a tab".into());
    bridge.set_footer("Keys: Settings › Tab groups".into());
}

/// A snapshot-like picture: a white page with bars.
fn striped_preview() -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(164, 90);
    let pixels = buffer.make_mut_slice();
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let (x, y) = (index % 164, index / 164);
        *pixel = if y < 8 {
            Rgba8Pixel::new(0xF1, 0xEC, 0xE5, 255)
        } else if (20..30).contains(&(y % 40)) && x > 10 && x < 120 {
            Rgba8Pixel::new(0xE2, 0xDA, 0xCF, 255)
        } else {
            Rgba8Pixel::new(0xFF, 0xFF, 0xFF, 255)
        };
    }
    Image::from_rgba8(buffer)
}

/// ring-status: an audio tab, with Pause and Mute.
fn status_scene(f: &Fixture) {
    let bridge = f.bridge();
    bridge.set_mode(CentreMode::Audio);
    bridge.set_title("Interview: Servo".into());
    bridge.set_play_state("Playing · 12:40 of 48:00".into());
    bridge.set_preview(striped_preview());
    bridge.set_has_preview(true);
    bridge.set_can_pause(true);
    bridge.set_paused(false);
    bridge.set_muted(false);
    bridge.set_footer("".into());
}

fn find<'a>(nodes: &'a [Node], role: AccessibleRole, label: &str) -> Option<&'a Node> {
    nodes.iter().find(|n| n.role == role && n.label == label)
}

#[test]
fn the_rest_state_names_its_field_and_draws_see_everything_only_when_orbit_exists() {
    let f = Fixture::new(false);
    open_scene(&f);
    f.settle();

    let nodes = f.nodes();
    let field = node_in(&nodes, AccessibleRole::TextInput, "Search tabs");
    assert_eq!(field.description, "Search 22 tabs");
    assert!(
        !nodes.iter().any(|n| n.role == AccessibleRole::ListItem),
        "the rest state lists no results"
    );
    assert!(find(&nodes, AccessibleRole::Button, "See everything").is_none());
    assert!(find(&nodes, AccessibleRole::Button, "Pause Essay on maps").is_none());
    assert!(find(&nodes, AccessibleRole::Text, "Essay on maps").is_some());
    assert!(
        find(
            &nodes,
            AccessibleRole::Text,
            "Reading · ⌥D 2 · essays.example.net"
        )
        .is_some()
    );
    assert!(find(&nodes, AccessibleRole::Text, "Hold ⌥ to see group keys").is_some());

    // Orbit exists: the button appears, in the rest state only.
    f.bridge().set_orbit_available(true);
    f.settle();
    let nodes = f.nodes();
    let see = node_in(&nodes, AccessibleRole::Button, "See everything");
    see.press();
    assert_eq!(f.calls(), ["see everything"]);
    // The button is 84 x 18 at (94, 244), on the lower edge of the disc.
    f.click(136.0, 253.0);
    assert_eq!(f.calls(), ["see everything"], "a click on it does the same");

    search_scene(&f);
    f.settle();
    assert!(
        find(&f.nodes(), AccessibleRole::Button, "See everything").is_none(),
        "not while searching"
    );
}

#[test]
fn every_control_is_labelled_and_inside_the_disc() {
    let f = Fixture::new(false);
    f.bridge().set_orbit_available(true);
    for scene in [
        open_scene as fn(&Fixture),
        search_scene,
        status_scene,
        keys_scene,
    ] {
        scene(&f);
        f.settle();
        for node in f.nodes() {
            if matches!(
                node.role,
                AccessibleRole::Button | AccessibleRole::ListItem | AccessibleRole::TextInput
            ) {
                assert!(!node.label.is_empty(), "an unlabelled {}", node.role_name());
                assert!(
                    Fixture::disc().contains(&node.rect()),
                    "{} {:?} lies outside the disc: {:?}",
                    node.role_name(),
                    node.label,
                    node.rect()
                );
            }
        }
    }
}

#[test]
fn results_are_list_items_that_fire_result_activated_with_their_row() {
    let f = Fixture::new(false);
    open_scene(&f);
    f.bridge().set_mode(CentreMode::Searching);
    f.bridge().set_results(model(vec![
        row(7, "Tram 28 timetable", "Lisbon trip · ⌥S 3 · ⏎ opens", true),
        row(9, "Trams of Lisbon", "Reading · ⌥D 1 · ⏎ opens", false),
        row(-1, "Search the web for “tram”", "", false),
    ]));
    f.settle();

    let nodes = f.nodes();
    let first = node_in(
        &nodes,
        AccessibleRole::ListItem,
        "Tram 28 timetable, Lisbon trip · ⌥S 3 · ⏎ opens",
    );
    let second = node_in(
        &nodes,
        AccessibleRole::ListItem,
        "Trams of Lisbon, Reading · ⌥D 1 · ⏎ opens",
    );
    let web = node_in(
        &nodes,
        AccessibleRole::ListItem,
        "Search the web for “tram”",
    );
    assert_eq!(first.selected, Some(true));
    assert_eq!(second.selected, Some(false));
    assert_eq!(web.selected, Some(false));
    assert_eq!(
        nodes
            .iter()
            .filter(|n| n.role == AccessibleRole::ListItem)
            .count(),
        3
    );

    // Assistive technology activates the row it names.
    web.press();
    second.press();
    assert_eq!(f.calls(), ["result 2", "result 1"]);

    // A click on each row, by pointer: the rows are 40 px (tabs) and 26 px (web) from y = 106.
    f.click(136.0, 126.0);
    f.click(136.0, 168.0);
    f.click(136.0, 203.0);
    assert_eq!(f.calls(), ["result 0", "result 1", "result 2"]);

    // Return on the row that has the focus (the one just clicked) opens it too.
    f.calls();
    f.click(136.0, 168.0);
    f.key(Key::Return);
    assert_eq!(f.calls(), ["result 1", "result 1"]);
}

#[test]
fn typing_in_the_field_fires_query_edited_and_fills_the_query() {
    let f = Fixture::new(false);
    open_scene(&f);
    f.settle();

    // The rest field is 200 x 30 at (36, 182); a click puts the caret in it.
    f.click(136.0, 197.0);
    for letter in ["t", "r", "a", "m"] {
        f.key(letter);
    }
    assert_eq!(
        f.calls(),
        ["query t", "query tr", "query tra", "query tram"]
    );
    assert_eq!(f.bridge().get_query().as_str(), "tram");

    // Rust clears the query (Esc): the field shows its placeholder again.
    f.bridge().set_query("".into());
    f.settle();
    assert_eq!(f.bridge().get_query().as_str(), "");
}

#[test]
fn the_audio_state_offers_pause_and_mute_by_the_tabs_state() {
    let f = Fixture::new(false);
    status_scene(&f);
    f.settle();

    let nodes = f.nodes();
    let pause = node_in(&nodes, AccessibleRole::Button, "Pause Interview: Servo");
    let mute = node_in(&nodes, AccessibleRole::Button, "Mute Interview: Servo");
    assert!(find(&nodes, AccessibleRole::Text, "Playing · 12:40 of 48:00").is_some());
    assert!(
        !nodes.iter().any(|n| n.role == AccessibleRole::TextInput),
        "no search field while an audio tab is shown"
    );
    assert!(
        pause.rect().right() <= mute.rect().x,
        "Pause is left of Mute, side by side"
    );
    assert!((pause.rect().y - mute.rect().y).abs() < 0.01);
    pause.press();
    mute.press();
    assert_eq!(f.calls(), ["pause", "mute"]);

    // Clicks do the same.
    f.click(88.0, 197.0);
    f.click(184.0, 197.0);
    assert_eq!(f.calls(), ["pause", "mute"]);

    // Paused: "Play"; muted: "Unmute".
    f.bridge().set_paused(true);
    f.bridge().set_muted(true);
    f.settle();
    let nodes = f.nodes();
    assert!(find(&nodes, AccessibleRole::Button, "Play Interview: Servo").is_some());
    assert!(find(&nodes, AccessibleRole::Button, "Unmute Interview: Servo").is_some());
    assert!(find(&nodes, AccessibleRole::Button, "Pause Interview: Servo").is_none());
    assert!(find(&nodes, AccessibleRole::Button, "Mute Interview: Servo").is_none());

    // No pause command (the media session has none): only Mute, and it moves to the middle.
    f.bridge().set_paused(false);
    f.bridge().set_muted(false);
    f.bridge().set_can_pause(false);
    f.settle();
    let nodes = f.nodes();
    assert!(
        !nodes
            .iter()
            .any(|n| n.label.starts_with("Pause ") || n.label.starts_with("Play ")),
        "Pause is drawn only when the tab can pause"
    );
    let mute = node_in(&nodes, AccessibleRole::Button, "Mute Interview: Servo");
    let middle = mute.rect().x + mute.rect().width / 2.0;
    assert!(
        (middle - (ORIGIN + 136.0)).abs() < 0.5,
        "alone, Mute is centred: {middle}"
    );
}

#[test]
fn the_held_state_is_three_lines_of_text_and_no_controls() {
    let f = Fixture::new(false);
    keys_scene(&f);
    f.settle();

    let nodes = f.nodes();
    for line in [
        "⌥ S",
        "Lisbon trip",
        "press 1–4 for a tab",
        "release ⌥ to switch",
        "Keys: Settings › Tab groups",
    ] {
        assert!(
            find(&nodes, AccessibleRole::Text, line).is_some(),
            "the held state says {line:?}"
        );
    }
    assert!(!nodes.iter().any(|n| matches!(
        n.role,
        AccessibleRole::Button | AccessibleRole::ListItem | AccessibleRole::TextInput
    )));
}

#[test]
fn long_titles_and_hosts_are_elided_inside_the_250_px_line() {
    let long_title: String = "Long Title of a Page about the Many Maps of the World "
        .chars()
        .cycle()
        .take(120)
        .collect();
    let host = "very-long-subdomain-name.another-long-label.example-host-name.test";
    assert_eq!(host.len(), 66);
    let long_subtitle = format!("Reading · ⌥D 2 · {}", &host[..60]);

    let f = Fixture::new(false);
    open_scene(&f);
    f.bridge().set_title(long_title.as_str().into());
    f.bridge().set_subtitle(long_subtitle.as_str().into());
    f.settle();
    let background = f.palette().get_window_background();

    let nodes = f.nodes();
    let title = node_in(&nodes, AccessibleRole::Text, &long_title);
    let subtitle = node_in(&nodes, AccessibleRole::Text, &long_subtitle);
    assert!((title.rect().width - 250.0).abs() < 0.01);
    assert!((subtitle.rect().width - 250.0).abs() < 0.01);

    // Nothing is drawn beside the 250 px line (11 px of the disc on either side).
    let long = f.shot();
    for side in [(8.0, 10.0), (262.0, 264.0)] {
        assert_eq!(long.ink(side, (130.0, 173.0), background), 0, "{side:?}");
    }
    assert!(long.ink((11.0, 261.0), (134.0, 152.0), background) > 200);
    assert!(long.ink((11.0, 261.0), (155.0, 172.0), background) > 200);

    // The line is cut at its end with "…", not at the middle: shortening the string (still too
    // long) changes nothing that is drawn. A clipped, centred line would shift.
    let shorter_title: String = long_title.chars().take(100).collect();
    let shorter_subtitle = format!("Reading · ⌥D 2 · {}", &host[..45]);
    f.bridge().set_title(shorter_title.as_str().into());
    f.bridge().set_subtitle(shorter_subtitle.as_str().into());
    let shorter = f.shot();
    for (band, what) in [((130.0, 153.0), "title"), ((155.0, 173.0), "subtitle")] {
        assert_eq!(
            long.area((11.0, 261.0), band),
            shorter.area((11.0, 261.0), band),
            "the {what} is elided at the end of the line"
        );
    }

    // A short title is drawn centred and is not elided.
    f.bridge().set_title("Maps".into());
    let short = f.shot();
    assert_ne!(
        long.area((11.0, 261.0), (130.0, 153.0)),
        short.area((11.0, 261.0), (130.0, 153.0))
    );
    assert_eq!(short.ink((11.0, 80.0), (130.0, 153.0), background), 0);
}

#[test]
fn the_four_states_paint_the_disc_in_each_scheme() {
    let mut discs = Vec::new();
    for (dark, scheme) in [(false, "light"), (true, "dark")] {
        // rest (ring-open)
        let f = Fixture::new(dark);
        let palette = f.palette();
        let (fill, accent) = (palette.get_window_background(), palette.get_accent());
        discs.push(fill);
        open_scene(&f);
        let rest = f.shot();
        rest.save(&format!("hub-ring-centre-open-{scheme}.png"));
        for (x, y) in [(16.0, 136.0), (256.0, 136.0), (136.0, 18.0), (136.0, 250.0)] {
            rest.is(x, y, fill, &format!("{scheme} rest: the disc fill"));
        }
        rest.is(
            60.0,
            40.0,
            Color::from_rgb_u8(0xEB, 0xC9, 0xB9),
            "the placeholder card is in the tab's mark colour",
        );
        rest.is(
            42.0,
            197.0,
            palette.get_search_field_background(),
            "the search field",
        );
        assert_eq!(
            rest.count((100.0, 170.0), (182.0, 184.0), accent),
            0,
            "{scheme}: an unfocused field has no accent border"
        );
        assert!(
            rest.ink((60.0, 212.0), (230.0, 245.0), fill) > 30,
            "{scheme}: the hint is drawn"
        );
        // The field, focused by a click: the 2 px accent border.
        f.click(136.0, 197.0);
        let focused = f.shot();
        let (hit, all) = (
            focused.count((100.0, 170.0), (182.0, 184.0), accent),
            focused.area((100.0, 170.0), (182.0, 184.0)).len(),
        );
        assert!(hit * 10 >= all * 9, "{scheme}: accent border {hit}/{all}");
        assert_eq!(
            rest.count((120.0, 160.0), (115.0, 125.0), accent),
            0,
            "no accent in the middle of the rest disc"
        );

        // searching (ring-search)
        let f = Fixture::new(dark);
        let palette = f.palette();
        search_scene(&f);
        f.click(136.0, 76.0);
        let search = f.shot();
        search.save(&format!("hub-ring-centre-search-{scheme}.png"));
        search.is(16.0, 136.0, fill, "searching: the disc fill");
        search.is(
            30.0,
            126.0,
            palette.get_item_selected(),
            "the selected result",
        );
        search.is(30.0, 161.0, fill, "the web row is not selected");
        assert!(
            search.count(
                (44.0, 120.0),
                (110.0, 142.0),
                palette.get_item_selected_text()
            ) > 20,
            "{scheme}: the selected result's text uses item-selected-text"
        );
        assert!(
            search.count((80.0, 200.0), (60.0, 62.0), accent) * 10
                >= search.area((80.0, 200.0), (60.0, 62.0)).len() * 9,
            "{scheme}: the focused field of the search state has the accent border"
        );

        // ⌥ held (ring-keys)
        let f = Fixture::new(dark);
        keys_scene(&f);
        let keys = f.shot();
        keys.save(&format!("hub-ring-centre-keys-{scheme}.png"));
        keys.is(16.0, 136.0, fill, "held: the disc fill");
        assert!(
            keys.count((90.0, 182.0), (62.0, 100.0), accent) > 60,
            "{scheme}: the key letter is drawn in the accent"
        );
        assert_eq!(rest.count((90.0, 182.0), (62.0, 100.0), accent), 0);

        // audio (ring-status)
        let f = Fixture::new(dark);
        let palette = f.palette();
        status_scene(&f);
        let status = f.shot();
        status.save(&format!("hub-ring-centre-status-{scheme}.png"));
        status.is(16.0, 136.0, fill, "audio: the disc fill");
        status.is(
            60.0,
            40.0,
            Color::from_rgb_u8(0xFF, 0xFF, 0xFF),
            "the snapshot is drawn",
        );
        for (x, y) in [(54.5, 32.5), (217.5, 32.5), (54.5, 121.5), (217.5, 121.5)] {
            status.is(x, y, fill, "the snapshot's corners are rounded off");
        }
        status.is(
            47.0,
            197.0,
            palette.get_popup_background(),
            "the Pause button",
        );
        status.is(
            143.0,
            197.0,
            palette.get_popup_background(),
            "the Mute button",
        );
        assert!(
            status.ink((54.0, 68.0), (190.0, 205.0), palette.get_popup_background()) > 15,
            "{scheme}: the Pause icon is drawn"
        );
        f.bridge().set_can_pause(false);
        let alone = f.shot();
        alone.is(47.0, 197.0, fill, "no Pause button without a pause command");
        alone.is(
            95.0,
            197.0,
            palette.get_popup_background(),
            "Mute, in the middle",
        );
    }
    assert_ne!(
        discs[0], discs[1],
        "the dark disc differs from the light one"
    );
}
