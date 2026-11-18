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
    BrowserTab, ChromePalette, ChromeTheme, MainWindow, TabMenuCommand, TabSearchRow,
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

#[test]
fn the_tab_strip_renders_and_responds() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).expect("no platform is set yet");

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
    a_drop_left_of_the_strip_does_not_pin();
    the_close_button_of_a_narrow_tab_can_be_clicked();
    the_tab_menu_duplicates();
}
