// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The tab strip in the browser window, driven with Slint's own events through the software
//! renderer (no display, no OS input): pinned tabs, the overflow "all tabs" button, tab search,
//! dragging, the tab menu and the hover preview. Set `MERCURY_UI_RENDER_DIR` to keep the PNGs.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use mercury_ui::{
    BrowserTab, ChromePalette, ChromeTheme, MainWindow, TabAudioState, TabMenuCommand,
    TabSearchRow, update_tab_rows,
};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, SharedPixelBuffer, VecModel,
};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

/// Slint's platform belongs to the thread, and a test thread may be reused for a second test
/// when tests run one at a time.
fn init_platform() {
    let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
}

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 320.0;
/// The strip starts after the toolbar row's left padding, and its tabs are centred here.
const STRIP_X: f32 = 10.0;
const TAB_Y: f32 = 26.0;
const PINNED_STRIDE: f32 = 46.0;
const STRIDE: f32 = 190.0;

fn output_path(name: &str) -> PathBuf {
    let directory = std::env::var_os("MERCURY_UI_RENDER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    directory.join(name)
}

fn save(pixels: &SharedPixelBuffer<slint::Rgba8Pixel>, name: &str) {
    image::save_buffer_with_format(
        output_path(name),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .expect("the rendering can be saved");
}

fn pixel(pixels: &SharedPixelBuffer<slint::Rgba8Pixel>, x: f32, y: f32) -> [u8; 3] {
    let pixel = pixels.as_slice()[(y as u32 * pixels.width() + x as u32) as usize];
    [pixel.r, pixel.g, pixel.b]
}

fn rgb(color: slint::Color) -> [u8; 3] {
    [color.red(), color.green(), color.blue()]
}

fn tab(id: i32, title: &str, active: bool, pinned: bool) -> BrowserTab {
    BrowserTab {
        title: title.into(),
        active,
        id,
        pinned,
        url: format!("https://site{id}.test/page").into(),
        ..Default::default()
    }
}

/// Two pinned tabs, then three others; the fourth tab (id 4, an unpinned one) is active.
fn sample_tabs() -> Vec<BrowserTab> {
    vec![
        tab(1, "Mail", false, true),
        tab(2, "Calendar", false, true),
        tab(3, "Servo, the embeddable engine", false, false),
        tab(4, "Slint documentation", true, false),
        tab(5, "Rust Weekly", false, false),
    ]
}

fn window_with(theme: ChromeTheme, tabs: Vec<BrowserTab>, pinned: i32) -> MainWindow {
    let window = MainWindow::new().expect("the main window can be created");
    window.global::<ChromePalette>().set_theme(theme);
    window.window().set_size(LogicalSize::new(WIDTH, HEIGHT));
    let active = tabs.iter().position(|tab| tab.active).unwrap_or(0);
    window.set_tabs(ModelRc::new(VecModel::from(tabs)));
    window.set_pinned_tab_count(pinned);
    window.set_active_tab_index(active as i32);
    window.set_is_new_tab(false);
    window.set_address("https://docs.slint.dev/".into());
    window.show().expect("the main window can be shown");
    window
}

fn pinned_x(index: usize) -> f32 {
    STRIP_X + index as f32 * PINNED_STRIDE + 20.0
}

/// The centre of unpinned tab `index` (counted from the first unpinned tab), two pinned first.
fn tab_x(index: usize) -> f32 {
    STRIP_X + 2.0 * PINNED_STRIDE + index as f32 * STRIDE + 92.0
}

fn pointer(window: &slint::Window, event: WindowEvent) {
    window.dispatch_event(event);
    slint::platform::update_timers_and_animations();
}

fn press_at(window: &slint::Window, x: f32, button: PointerEventButton) {
    pointer(
        window,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(x, TAB_Y),
        },
    );
    pointer(
        window,
        WindowEvent::PointerPressed {
            position: LogicalPosition::new(x, TAB_Y),
            button,
        },
    );
}

/// A secondary click: the menu opens on the press; the release follows, as with a real mouse.
fn right_click(window: &slint::Window, x: f32) {
    press_at(window, x, PointerEventButton::Right);
    pointer(
        window,
        WindowEvent::PointerReleased {
            position: LogicalPosition::new(x, TAB_Y),
            button: PointerEventButton::Right,
        },
    );
}

fn drag(window: &slint::Window, from: f32, to: f32) {
    press_at(window, from, PointerEventButton::Left);
    let steps = 8;
    for step in 1..=steps {
        let x = from + (to - from) * step as f32 / steps as f32;
        pointer(
            window,
            WindowEvent::PointerMoved {
                position: LogicalPosition::new(x, TAB_Y),
            },
        );
    }
    pointer(
        window,
        WindowEvent::PointerReleased {
            position: LogicalPosition::new(to, TAB_Y),
            button: PointerEventButton::Left,
        },
    );
}

fn key(window: &slint::Window, text: impl Into<slint::SharedString> + Clone) {
    window.dispatch_event(WindowEvent::KeyPressed {
        text: text.clone().into(),
    });
    window.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
}

fn render_pinned_tabs(theme: ChromeTheme, name: &str) {
    let window = window_with(theme, sample_tabs(), 2);
    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("tab-strip-pinned-{name}.png"));

    let palette = window.global::<ChromePalette>();
    // The active (unpinned) tab is the second unpinned one; the pinned tabs are compact.
    assert_eq!(
        pixel(&pixels, tab_x(1) - 60.0, TAB_Y - 12.0),
        rgb(palette.get_tab_active()),
        "{name}"
    );
    assert_ne!(
        pixel(&pixels, pinned_x(0) - 14.0, TAB_Y - 12.0),
        rgb(palette.get_tab_active()),
        "{name}"
    );
    window.hide().expect("the window can be hidden");
}

fn dragging_reorders_and_pins() {
    let window = window_with(ChromeTheme::Dark, sample_tabs(), 2);
    let moves = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&moves);
    window.on_move_tab(move |id, index, pinned| recorded.borrow_mut().push((id, index, pinned)));
    let selected = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&selected);
    window.on_select_tab(move |index| recorded.borrow_mut().push(index));

    // The last tab onto the first unpinned one: index 2, unpinned.
    drag(window.window(), tab_x(2), tab_x(0));
    // The first unpinned tab onto the first pinned tab: pinned at index 0.
    drag(window.window(), tab_x(0), pinned_x(0));
    // The second pinned tab out over the last tab: unpinned, at the end.
    drag(window.window(), pinned_x(1), tab_x(2));
    // A click without movement selects and moves nothing.
    press_at(window.window(), tab_x(1), PointerEventButton::Left);
    pointer(
        window.window(),
        WindowEvent::PointerReleased {
            position: LogicalPosition::new(tab_x(1), TAB_Y),
            button: PointerEventButton::Left,
        },
    );

    assert_eq!(
        *moves.borrow(),
        [(5, 2, false), (3, 0, true), (2, 4, false)]
    );
    // Each press selects the tab under it.
    assert_eq!(*selected.borrow(), [4, 2, 1, 3]);

    // Mid-drag: the dragged tab follows the pointer and the drop mark shows.
    press_at(window.window(), tab_x(2), PointerEventButton::Left);
    for x in [tab_x(2) - 40.0, tab_x(1) - 20.0, tab_x(0) + 10.0] {
        pointer(
            window.window(),
            WindowEvent::PointerMoved {
                position: LogicalPosition::new(x, TAB_Y),
            },
        );
    }
    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, "tab-strip-dragging-dark.png");
    pointer(
        window.window(),
        WindowEvent::PointerReleased {
            position: LogicalPosition::new(tab_x(0) + 10.0, TAB_Y),
            button: PointerEventButton::Left,
        },
    );
    window.hide().expect("the window can be hidden");
}

fn the_tab_menu_pins_and_closes(theme: ChromeTheme, name: &str) {
    let window = window_with(theme, sample_tabs(), 2);
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&chosen);
    window.on_tab_menu_activated(move |id, command| recorded.borrow_mut().push((id, command)));

    right_click(window.window(), tab_x(0));
    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("tab-menu-{name}.png"));
    // "Pin tab" is the first row; Return chooses it.
    key(window.window(), Key::DownArrow);
    key(window.window(), Key::Return);

    // On a pinned tab the first row unpins it; the last row closes the others.
    right_click(window.window(), pinned_x(1));
    key(window.window(), Key::UpArrow);
    key(window.window(), Key::Return);
    right_click(window.window(), pinned_x(0));
    key(window.window(), Key::DownArrow);
    key(window.window(), Key::Return);

    assert_eq!(
        *chosen.borrow(),
        [
            (3, TabMenuCommand::Pin),
            (2, TabMenuCommand::CloseOthers),
            (1, TabMenuCommand::Unpin)
        ],
        "{name}"
    );
    window.hide().expect("the window can be hidden");
}

fn the_overflow_button_opens_tab_search(theme: ChromeTheme, name: &str) {
    let tabs = (1..=50)
        .map(|id| tab(id, &format!("Page number {id}"), id == 50, false))
        .collect::<Vec<_>>();
    let window = window_with(theme, tabs, 0);
    window.invoke_reveal_active_tab();
    let queries = Rc::new(RefCell::new(Vec::<String>::new()));
    let recorded = Rc::clone(&queries);
    window.on_tab_search_edited(move |query| recorded.borrow_mut().push(query.into()));
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&chosen);
    window.on_tab_search_chosen(move |id| recorded.borrow_mut().push(id));

    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("tab-strip-50-tabs-{name}.png"));
    let palette = window.global::<ChromePalette>();
    // The active (last) tab is scrolled into view, just left of the "all tabs" button.
    let strip_right = WIDTH - 14.0 - 34.0 - 8.0;
    let list_button_x = strip_right - 17.0;
    assert_eq!(
        pixel(&pixels, list_button_x - 40.0 - 60.0, TAB_Y - 12.0),
        rgb(palette.get_tab_active()),
        "{name}"
    );

    press_at(window.window(), list_button_x, PointerEventButton::Left);
    pointer(
        window.window(),
        WindowEvent::PointerReleased {
            position: LogicalPosition::new(list_button_x, TAB_Y),
            button: PointerEventButton::Left,
        },
    );
    assert_eq!(*queries.borrow(), [""], "{name}");

    for character in ["4", "2"] {
        key(window.window(), character);
    }
    assert_eq!(*queries.borrow(), ["", "4", "42"], "{name}");
    let row = |id: i32| TabSearchRow {
        id,
        title: format!("Page number {id}").into(),
        url: format!("https://site{id}.test/page").into(),
        pinned: id == 4,
        ..Default::default()
    };
    window.set_tab_search_rows(ModelRc::new(VecModel::from(vec![row(4), row(42)])));
    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("tab-search-{name}.png"));
    key(window.window(), Key::DownArrow);
    key(window.window(), Key::Return);
    assert_eq!(*chosen.borrow(), [42], "{name}");
    window.hide().expect("the window can be hidden");
}

fn hovering_shows_the_preview(theme: ChromeTheme, name: &str) {
    let window = window_with(theme, sample_tabs(), 2);
    pointer(
        window.window(),
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(tab_x(0), TAB_Y),
        },
    );
    let before = window.window().take_snapshot().expect("the window renders");
    std::thread::sleep(Duration::from_millis(700));
    slint::platform::update_timers_and_animations();
    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("tab-preview-{name}.png"));

    let popup = rgb(window.global::<ChromePalette>().get_popup_background());
    // Below the hovered tab's left edge, over the toolbar.
    let (x, y) = (tab_x(0) - 92.0 + 4.0, 70.0);
    assert_ne!(pixel(&before, x, y), popup, "{name}");
    assert_eq!(pixel(&pixels, x, y), popup, "{name}");
    window.hide().expect("the window can be hidden");
}

fn tabs_model(window: &MainWindow) -> ModelRc<BrowserTab> {
    window.get_tabs()
}

fn with_rows(window: &MainWindow, change: impl FnOnce(&VecModel<BrowserTab>)) {
    let model = tabs_model(window);
    let rows = model
        .as_any()
        .downcast_ref::<VecModel<BrowserTab>>()
        .expect("the tabs are a VecModel");
    change(rows);
    slint::platform::update_timers_and_animations();
}

fn release_at(window: &slint::Window, x: f32) {
    pointer(
        window,
        WindowEvent::PointerReleased {
            position: LogicalPosition::new(x, TAB_Y),
            button: PointerEventButton::Left,
        },
    );
}

fn move_to(window: &slint::Window, x: f32) {
    pointer(
        window,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(x, TAB_Y),
        },
    );
}

/// Whether the hover preview shows after resting on the first unpinned tab: it does not while a
/// drag is (or is wrongly thought to be) in progress.
fn preview_shows_after_resting(window: &MainWindow) -> bool {
    move_to(window.window(), STRIP_X + 400.0);
    move_to(window.window(), tab_x(0));
    std::thread::sleep(Duration::from_millis(700));
    slint::platform::update_timers_and_animations();
    let pixels = window.window().take_snapshot().expect("the window renders");
    let popup = rgb(window.global::<ChromePalette>().get_popup_background());
    pixel(&pixels, tab_x(0) - 92.0 + 4.0, 70.0) == popup
}

// tabs-plus review I2: the strip changes while a tab is being dragged.
fn a_drag_survives_a_tab_opening_elsewhere() {
    let window = window_with(ChromeTheme::Dark, sample_tabs(), 2);
    let moves = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&moves);
    window.on_move_tab(move |id, index, pinned| recorded.borrow_mut().push((id, index, pinned)));

    // Pick up the last tab (id 5)...
    press_at(window.window(), tab_x(2), PointerEventButton::Left);
    move_to(window.window(), tab_x(2) - 40.0);
    // ... a page opens a tab in front of it (strip index 2) ...
    with_rows(&window, |rows| {
        rows.insert(2, tab(9, "Opened", false, false))
    });
    // ... and it is dropped over the first unpinned slot.
    move_to(window.window(), tab_x(0));
    release_at(window.window(), tab_x(0));

    assert_eq!(*moves.borrow(), [(5, 2, false)]);
    assert!(preview_shows_after_resting(&window), "the drag ended");
    window.hide().expect("the window can be hidden");
}

fn a_drag_whose_tab_closes_is_cancelled() {
    let window = window_with(ChromeTheme::Dark, sample_tabs(), 2);
    let moves = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&moves);
    window.on_move_tab(move |id, index, pinned| recorded.borrow_mut().push((id, index, pinned)));

    press_at(window.window(), tab_x(2), PointerEventButton::Left);
    move_to(window.window(), tab_x(2) - 40.0);
    // The dragged tab's page closes it; another tab takes its row.
    with_rows(&window, |rows| {
        rows.set_row_data(4, tab(9, "Other", false, false))
    });
    move_to(window.window(), tab_x(0));
    release_at(window.window(), tab_x(0));

    assert!(moves.borrow().is_empty(), "{:?}", moves.borrow());
    assert!(
        preview_shows_after_resting(&window),
        "no drag is left behind"
    );

    // The same when its row is removed outright.
    press_at(window.window(), tab_x(1), PointerEventButton::Left);
    move_to(window.window(), tab_x(1) - 40.0);
    with_rows(&window, |rows| {
        rows.remove(3);
    });
    move_to(window.window(), tab_x(0));
    release_at(window.window(), tab_x(0));

    assert!(moves.borrow().is_empty(), "{:?}", moves.borrow());
    assert!(
        preview_shows_after_resting(&window),
        "no drag is left behind"
    );
    window.hide().expect("the window can be hidden");
}

// tabs-plus review I2: the application's sync closes one tab and opens another (same count, ids
// shift) while a tab is dragged.
fn a_drag_survives_a_sync_that_closes_and_opens_tabs() {
    let window = window_with(ChromeTheme::Dark, sample_tabs(), 2);
    let moves = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&moves);
    window.on_move_tab(move |id, index, pinned| recorded.borrow_mut().push((id, index, pinned)));

    press_at(window.window(), tab_x(2), PointerEventButton::Left);
    move_to(window.window(), tab_x(2) - 40.0);
    let mut rows = sample_tabs();
    rows.remove(3);
    rows.push(tab(9, "Opened", false, false));
    with_rows(&window, |model| mercury_ui::update_tab_rows(model, rows));
    move_to(window.window(), tab_x(0));
    release_at(window.window(), tab_x(0));

    assert_eq!(*moves.borrow(), [(5, 2, false)]);
    let ids = tabs_model(&window)
        .iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, [1, 2, 3, 5, 9]);
    window.hide().expect("the window can be hidden");
}

// tabs-plus review M1: with no pinned tabs, a drop left of the strip does not pin.
fn a_drop_left_of_the_strip_does_not_pin() {
    let tabs = vec![
        tab(1, "One", true, false),
        tab(2, "Two", false, false),
        tab(3, "Three", false, false),
    ];
    let window = window_with(ChromeTheme::Dark, tabs, 0);
    let moves = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&moves);
    window.on_move_tab(move |id, index, pinned| recorded.borrow_mut().push((id, index, pinned)));

    drag(window.window(), STRIP_X + STRIDE + 92.0, 2.0);

    assert_eq!(*moves.borrow(), [(2, 0, false)]);
    window.hide().expect("the window can be hidden");
}

// tabs-plus review I3: the close button of a narrow inactive tab.
fn the_close_button_of_a_narrow_tab_can_be_clicked() {
    let tabs = (1..=15)
        .map(|id| tab(id, &format!("Page {id}"), id == 1, false))
        .collect::<Vec<_>>();
    let window = window_with(ChromeTheme::Dark, tabs, 0);
    let closed = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&closed);
    window.on_close_tab(move |index| recorded.borrow_mut().push(index));
    let selected = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&selected);
    window.on_select_tab(move |index| recorded.borrow_mut().push(index));

    // Tabs are 112 px wide (stride 118); the second one's × sits 8 px from its right edge.
    let close_x = STRIP_X + 118.0 + 112.0 - 8.0 - 11.0;
    move_to(window.window(), STRIP_X + 118.0 + 30.0);
    move_to(window.window(), close_x - 4.0);
    move_to(window.window(), close_x);
    press_at(window.window(), close_x, PointerEventButton::Left);
    release_at(window.window(), close_x);

    assert_eq!(*closed.borrow(), [1]);
    assert!(selected.borrow().is_empty(), "{:?}", selected.borrow());
    window.hide().expect("the window can be hidden");
}

// tabs-plus: Duplicate tab in the tab menu.
fn the_tab_menu_duplicates() {
    let window = window_with(ChromeTheme::Light, sample_tabs(), 2);
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&chosen);
    window.on_tab_menu_activated(move |id, command| recorded.borrow_mut().push((id, command)));

    right_click(window.window(), tab_x(1));
    key(window.window(), Key::DownArrow);
    key(window.window(), Key::DownArrow);
    key(window.window(), Key::Return);

    assert_eq!(*chosen.borrow(), [(4, TabMenuCommand::Duplicate)]);
    window.hide().expect("the window can be hidden");
}

// ---- The sound indicator (media-session lane, ported onto this strip) ----

/// A copy of `tab` with this sound state.
fn sound(mut tab: BrowserTab, audio: TabAudioState) -> BrowserTab {
    tab.audio = audio;
    tab
}

/// `sample_tabs()` with sound: the first pinned tab plays, the second is muted, the active tab
/// (id 4) plays and the last (id 5) is muted.
fn sound_tabs() -> Vec<BrowserTab> {
    sample_tabs()
        .into_iter()
        .map(|tab| match tab.id {
            1 | 4 => sound(tab, TabAudioState::Playing),
            2 | 5 => sound(tab, TabAudioState::Muted),
            _ => tab,
        })
        .collect()
}

/// The left edge of the pinned tab `index`.
fn pinned_left(index: usize) -> f32 {
    STRIP_X + index as f32 * PINNED_STRIDE
}

/// The left edge of the unpinned tab `index` (counted from the first unpinned tab), when two
/// pinned tabs come first and the tabs are 184 px wide.
fn tab_left(index: usize) -> f32 {
    tab_x(index) - 92.0
}

/// The middle of the speaker of a 184 px tab with its close button shown: it spans 124 to 146 px
/// from the tab's left edge.
fn speaker_x(left: f32) -> f32 {
    left + 135.0
}

/// The middle of a pinned tab's sound badge (16 px at the bottom right of the 40 x 38 px tab,
/// which starts 7 px from the top).
fn badge_at(index: usize) -> (f32, f32) {
    (pinned_left(index) + 30.0, 34.0)
}

fn snapshot(window: &MainWindow) -> SharedPixelBuffer<slint::Rgba8Pixel> {
    window.window().take_snapshot().expect("the window renders")
}

/// Pixels in a box that differ from the box's top-left pixel: whatever is drawn in it.
fn ink(pixels: &SharedPixelBuffer<slint::Rgba8Pixel>, x: (f32, f32), y: (f32, f32)) -> usize {
    let at = |x: u32, y: u32| pixels.as_slice()[(y * pixels.width() + x) as usize];
    let background = at(x.0 as u32, y.0 as u32);
    (y.0 as u32..y.1 as u32)
        .flat_map(|row| (x.0 as u32..x.1 as u32).map(move |column| (column, row)))
        .filter(|(column, row)| at(*column, *row) != background)
        .count()
}

/// The rows a 184 px speaker spans (22 px centred on `TAB_Y`).
const SPEAKER_Y: (f32, f32) = (15.0, 37.0);

fn click_at(window: &slint::Window, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    pointer(window, WindowEvent::PointerMoved { position });
    pointer(
        window,
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
    );
    pointer(
        window,
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    );
}

fn move_at(window: &slint::Window, x: f32, y: f32) {
    pointer(
        window,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(x, y),
        },
    );
}

fn rest(milliseconds: u64) {
    std::thread::sleep(Duration::from_millis(milliseconds));
    slint::platform::update_timers_and_animations();
}

/// Whether the hover preview shows for the tab whose left edge is `left`.
fn preview_shows_at(window: &MainWindow, left: f32) -> bool {
    let popup = rgb(window.global::<ChromePalette>().get_popup_background());
    pixel(&snapshot(window), left + 4.0, 70.0) == popup
}

type Log<T> = Rc<RefCell<Vec<T>>>;

fn recorded_toggles(window: &MainWindow) -> Log<i32> {
    let log = Log::default();
    let recorded = Rc::clone(&log);
    window.on_toggle_tab_muted(move |id| recorded.borrow_mut().push(id));
    log
}

fn recorded_selects(window: &MainWindow) -> Log<i32> {
    let log = Log::default();
    let recorded = Rc::clone(&log);
    window.on_select_tab(move |index| recorded.borrow_mut().push(index));
    log
}

fn recorded_moves(window: &MainWindow) -> Log<(i32, i32, bool)> {
    let log = Log::default();
    let recorded = Rc::clone(&log);
    window.on_move_tab(move |id, index, pinned| recorded.borrow_mut().push((id, index, pinned)));
    log
}

// Clicking the speaker of a pinned or another tab mutes by tab id, selects nothing and starts no
// drag; the id is the right one after a drag has reordered the tabs.
#[test]
fn clicking_the_speaker_mutes_by_id_without_selecting_or_dragging() {
    init_platform();
    let window = window_with(ChromeTheme::Dark, sound_tabs(), 2);
    let toggled = recorded_toggles(&window);
    let selected = recorded_selects(&window);
    let moves = recorded_moves(&window);

    // Pinned tabs (ids 1, 2): the badge. Other tabs (ids 4, 5): the speaker.
    for index in 0..2 {
        let (x, y) = badge_at(index);
        click_at(window.window(), x, y);
    }
    for index in [1, 2] {
        click_at(window.window(), speaker_x(tab_left(index)), TAB_Y);
    }
    assert_eq!(*toggled.borrow(), [1, 2, 4, 5]);
    assert!(selected.borrow().is_empty(), "{:?}", selected.borrow());

    // A press on the speaker that moves away is no drag, and no click.
    for (x, y) in [badge_at(0), (speaker_x(tab_left(2)), TAB_Y)] {
        move_at(window.window(), x, y);
        pointer(
            window.window(),
            WindowEvent::PointerPressed {
                position: LogicalPosition::new(x, y),
                button: PointerEventButton::Left,
            },
        );
        for step in 1..=8 {
            move_at(window.window(), x + 12.0 * step as f32, y);
        }
        pointer(
            window.window(),
            WindowEvent::PointerReleased {
                position: LogicalPosition::new(x + 96.0, y),
                button: PointerEventButton::Left,
            },
        );
    }
    assert!(moves.borrow().is_empty(), "{:?}", moves.borrow());
    assert!(selected.borrow().is_empty(), "{:?}", selected.borrow());
    assert_eq!(toggled.borrow().len(), 4);

    // Drag the last tab (id 5) in front of the first unpinned one, and let the application's
    // sync reorder the rows: the speaker at the third place now belongs to id 5, not index 2.
    drag(window.window(), tab_x(2), tab_x(0));
    assert_eq!(*moves.borrow(), [(5, 2, false)]);
    let selected_by_the_drag = selected.borrow().len();
    let mut rows = sound_tabs();
    let moved = rows.remove(4);
    rows.insert(2, moved);
    with_rows(&window, |model| update_tab_rows(model, rows));
    click_at(window.window(), speaker_x(tab_left(0)), TAB_Y);
    click_at(window.window(), speaker_x(tab_left(2)), TAB_Y);
    assert_eq!(toggled.borrow()[4..], [5, 4]);
    assert_eq!(selected.borrow().len(), selected_by_the_drag);
    window.hide().expect("the window can be hidden");
}

// Resting the pointer on the speaker (or the pinned badge) keeps the tab hovered, so its preview
// stays; leaving the tab hides it.
#[test]
fn resting_on_the_speaker_keeps_the_hover_preview() {
    init_platform();
    let window = window_with(ChromeTheme::Dark, sound_tabs(), 2);

    for (left, body, speaker) in [
        (
            tab_left(2),
            (tab_left(2) + 60.0, TAB_Y),
            (speaker_x(tab_left(2)), TAB_Y),
        ),
        (pinned_left(0), (pinned_left(0) + 10.0, TAB_Y), badge_at(0)),
    ] {
        move_at(window.window(), 900.0, 100.0);
        rest(300);
        assert!(!preview_shows_at(&window, left), "no preview yet");
        if left > pinned_left(1) {
            // The tab is 184 px wide and its title is short: the speaker is the only thing that
            // can be drawn in its box.
            let speaker = (speaker_x(left) - 11.0, speaker_x(left) + 11.0);
            assert!(
                ink(&snapshot(&window), speaker, SPEAKER_Y) > 20,
                "a speaker"
            );
        }
        move_at(window.window(), body.0, body.1);
        rest(700);
        assert!(preview_shows_at(&window, left), "the preview shows");
        move_at(window.window(), speaker.0, speaker.1);
        rest(400);
        assert!(
            preview_shows_at(&window, left),
            "the preview stays while the pointer is on the speaker"
        );
        // A click on it gets the preview out of the way until the pointer leaves the tab.
        click_at(window.window(), speaker.0, speaker.1);
        rest(800);
        assert!(
            !preview_shows_at(&window, left),
            "no preview after a click on the speaker"
        );
        move_at(window.window(), 900.0, 100.0);
        rest(400);
        assert!(
            !preview_shows_at(&window, left),
            "leaving hides the preview"
        );
    }
    window.hide().expect("the window can be hidden");
}

// A row whose sound changes is updated in place, so a drag in progress survives it, and the
// dragged copy shows the new state.
#[test]
fn a_drag_survives_a_tab_whose_sound_changes() {
    init_platform();
    let window = window_with(ChromeTheme::Dark, sound_tabs(), 2);
    let moves = recorded_moves(&window);

    // Pick up the last tab (id 5, muted) and let it start to play, while the quiet tab (id 3)
    // is muted and the first pinned tab (id 1) falls silent.
    press_at(window.window(), tab_x(2), PointerEventButton::Left);
    move_to(window.window(), tab_x(2) - 40.0);
    with_rows(&window, |rows| {
        rows.set_row_data(4, sound(rows.row_data(4).unwrap(), TabAudioState::Playing));
        rows.set_row_data(2, sound(rows.row_data(2).unwrap(), TabAudioState::Muted));
        rows.set_row_data(0, sound(rows.row_data(0).unwrap(), TabAudioState::Silent));
    });
    move_to(window.window(), tab_x(0));
    // The copy under the pointer shows the speaker; the tab's own place shows none.
    let pixels = snapshot(&window);
    let ghost_left = tab_x(0) - 92.0;
    assert!(
        ink(&pixels, (ghost_left + 124.0, ghost_left + 146.0), SPEAKER_Y) > 20,
        "the dragged copy shows the speaker"
    );
    release_at(window.window(), tab_x(0));

    assert_eq!(*moves.borrow(), [(5, 2, false)]);
    assert!(preview_shows_after_resting(&window), "the drag ended");
    window.hide().expect("the window can be hidden");
}

// The dragged copy shows the indicator but takes no input, and the tab's own place hides it.
#[test]
fn the_dragged_copy_shows_the_speaker_but_takes_no_input() {
    init_platform();
    // A short title, so that nothing but the speaker can be drawn in its box.
    let mut tabs = sound_tabs();
    tabs[3].title = "Song".into();
    let window = window_with(ChromeTheme::Dark, tabs, 2);
    let toggled = recorded_toggles(&window);

    // Pick up the active tab (id 4, playing) and carry it left over the third tab.
    press_at(window.window(), tab_x(1), PointerEventButton::Left);
    for step in 1..=8 {
        move_to(window.window(), tab_x(1) - 12.0 * step as f32);
    }
    let ghost_left = tab_left(1) - 96.0;
    let pixels = snapshot(&window);
    save(&pixels, "tab-strip-dragging-sound.png");
    assert!(
        ink(&pixels, (ghost_left + 124.0, ghost_left + 146.0), SPEAKER_Y) > 20,
        "the dragged copy shows the speaker"
    );
    let slot = tab_left(1);
    assert_eq!(
        ink(&pixels, (slot + 124.0, slot + 146.0), SPEAKER_Y),
        0,
        "the tab's place is an empty outline"
    );

    // A click on the copy's speaker mutes nothing.
    click_at(window.window(), speaker_x(ghost_left), TAB_Y);
    assert!(toggled.borrow().is_empty(), "{:?}", toggled.borrow());
    window.hide().expect("the window can be hidden");
}

// The narrow-tab rule: a tab with sound that is narrower than 160 px shows its speaker where the
// page icon goes, so it never moves when the pointer arrives and the title keeps room; the close
// button then takes its own place at the right, shown for the active tab and under the pointer.
// From 160 px the speaker sits before the close button, which is always shown.
//
// `count` tabs: the first is active and quiet, the second plays, the third is muted. 5 tabs are
// 182 px wide, 6 are 150.7 px, 7 are 128.3 px and 15 are 112 px.
fn a_tab_with_sound_keeps_its_speaker_and_close_button(count: usize) {
    let tabs = (1..=count as i32)
        .map(|id| match id {
            2 => sound(tab(id, "Page 2", false, false), TabAudioState::Playing),
            3 => sound(tab(id, "Page 3", false, false), TabAudioState::Muted),
            _ => tab(id, &format!("Page {id}"), id == 1, false),
        })
        .collect::<Vec<_>>();
    let window = window_with(ChromeTheme::Dark, tabs, 0);
    let toggled = recorded_toggles(&window);
    let selected = recorded_selects(&window);
    let closed = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&closed);
    window.on_close_tab(move |index| recorded.borrow_mut().push(index));

    // The strip is 934 px wide, or 894 px with the "all tabs" button.
    let room = if count * 118 - 6 > 934 { 894.0 } else { 934.0 };
    let width = ((room + 6.0) / count as f32 - 6.0).clamp(112.0, 184.0);
    let compact = width < 160.0;
    let mut expected_toggles = Vec::new();
    let mut expected_closes = Vec::new();
    for index in [1usize, 2] {
        let left = STRIP_X + index as f32 * (width + 6.0);
        let speaker = if compact {
            (left + 8.0, left + 30.0)
        } else {
            (left + width - 60.0, left + width - 38.0)
        };
        let close = (left + width - 30.0, left + width - 8.0);
        assert!(speaker.1 <= close.0, "the speaker and × never overlap");
        let title = if compact {
            (left + 36.0, close.0 - 6.0)
        } else {
            (left + 38.0, speaker.0 - 8.0)
        };
        let middle = |range: (f32, f32)| (range.0 + range.1) / 2.0;
        let label = format!("{width} px, tab {}", index + 1);

        // At rest.
        move_at(window.window(), 500.0, 100.0);
        let pixels = snapshot(&window);
        assert!(ink(&pixels, speaker, SPEAKER_Y) > 20, "{label}: speaker");
        assert!(
            ink(&pixels, title, (18.0, 34.0)) > 5,
            "{label}: the title keeps room"
        );
        if compact {
            assert_eq!(ink(&pixels, close, SPEAKER_Y), 0, "{label}: no × at rest");
        } else {
            assert!(ink(&pixels, close, SPEAKER_Y) > 0, "{label}: × at rest");
        }

        // Under the pointer: the × shows and nothing moves.
        move_at(window.window(), left + 50.0, TAB_Y);
        let pixels = snapshot(&window);
        if index == 1 {
            save(&pixels, &format!("tab-strip-sound-{count}-tabs.png"));
        }
        assert!(ink(&pixels, close, SPEAKER_Y) > 0, "{label}: × on hover");
        assert!(ink(&pixels, speaker, SPEAKER_Y) > 20, "{label}: speaker");
        move_at(window.window(), middle(speaker), TAB_Y);
        let pixels = snapshot(&window);
        assert!(ink(&pixels, close, SPEAKER_Y) > 0, "{label}: × stays");
        assert!(ink(&pixels, speaker, SPEAKER_Y) > 20, "{label}: speaker");

        // Each button does its own thing, and the tab is not selected.
        click_at(window.window(), middle(speaker), TAB_Y);
        expected_toggles.push(index as i32 + 1);
        assert_eq!(*toggled.borrow(), expected_toggles, "{label}");
        move_at(window.window(), left + 50.0, TAB_Y);
        move_at(window.window(), middle(close), TAB_Y);
        click_at(window.window(), middle(close), TAB_Y);
        expected_closes.push(index as i32);
        assert_eq!(*closed.borrow(), expected_closes, "{label}");
        assert!(
            selected.borrow().is_empty(),
            "{label}: {:?}",
            selected.borrow()
        );
    }
    window.hide().expect("the window can be hidden");
}

#[test]
fn a_tab_with_sound_keeps_its_speaker_and_close_button_at_every_width() {
    init_platform();
    for count in [5, 6, 7, 15] {
        a_tab_with_sound_keeps_its_speaker_and_close_button(count);
    }
}

#[test]
fn the_tab_strip_renders_and_responds() {
    init_platform();

    for (theme, name) in [
        (ChromeTheme::Light, "light"),
        (ChromeTheme::Dark, "dark"),
        (ChromeTheme::System, "system"),
    ] {
        render_pinned_tabs(theme, name);
        the_tab_menu_pins_and_closes(theme, name);
        the_overflow_button_opens_tab_search(theme, name);
        hovering_shows_the_preview(theme, name);
    }
    dragging_reorders_and_pins();
    a_drag_survives_a_tab_opening_elsewhere();
    a_drag_whose_tab_closes_is_cancelled();
    a_drag_survives_a_sync_that_closes_and_opens_tabs();
    a_drop_left_of_the_strip_does_not_pin();
    the_close_button_of_a_narrow_tab_can_be_clicked();
    the_tab_menu_duplicates();
}
