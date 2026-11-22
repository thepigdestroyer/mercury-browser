// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The scenes of the orbit-view lane, shared by `examples/render_orbit.rs` (which writes the PNGs),
//! `tests/orbit_render.rs` (pixel probes), `orbit_a11y.rs` and `orbit_pointer.rs`: an `OrbitPreview`
//! filled through the REAL pipeline: `orbit_view::fixtures` rows -> `OrbitMachine` (driven to the
//! level, focus, drag and popover state of each frame with its public methods) -> `OrbitData::build`
//! -> `OrbitModels::update` -> the window's properties, with the B2 palette pushed. Included with
//! `#[path]` (a directory module under `tests/` is not a test target).
//!
//! A `Scene` installs no Slint platform: the renderers call [`init_software`] first, the a11y and
//! pointer tests use the testing backend. Pictures are the lane's skeleton picture (the image path is
//! the real one); a scene without pictures draws the placeholder cards.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use mercury_appearance::resolve::AppearanceSettings;
use mercury_ui::orbit_view::data::{Options, OrbitData, OrbitModels};
use mercury_ui::orbit_view::drag::{DropTarget, Source};
use mercury_ui::orbit_view::fixtures::{
    generated_input, orbit_groups, orbit_input, orbit_tabs, skeleton_picture, tab_id,
};
use mercury_ui::orbit_view::machine::OrbitMachine;
use mercury_ui::orbit_view::{DragView, Level, OrbitInput, Stage, Target, View, search};
use mercury_ui::ring_bands::RingColours;
use mercury_ui::{ChromePalette, ChromeTheme, Look, OrbitLevel, OrbitPreview, apply_look};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize, Rgba8Pixel, SharedPixelBuffer};

pub const SCALE: f32 = 2.0;
/// The mockup's window.
pub const STAGE: Stage = Stage::MOCKUP;

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

thread_local! {
    static SOFTWARE_READY: Cell<bool> = const { Cell::new(false) };
}

/// Installs the headless software platform on this thread (once; a second call does nothing).
pub fn init_software() {
    SOFTWARE_READY.with(|ready| {
        if !ready.replace(true) {
            let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
        }
    });
}

pub type Pixels = SharedPixelBuffer<Rgba8Pixel>;

/// The frames: the five of the mockup, then the four extras.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    /// Level 1 with Research lit (`orbit-1-overview`).
    Overview,
    /// Level 2 of Research, a preview lifted (`orbit-2-group`).
    Group,
    /// Level 3 of "AccessKit guide" (`orbit-3-tab`).
    Tab,
    /// A tab dragged over "New group", the popover open (`orbit-new-group`).
    NewGroup,
    /// A tab dragged over Reading's ring (`orbit-move-tab`).
    MoveTab,
    /// Level 1 with twelve groups: the planets shrink (`orbit-1-many-groups`).
    ManyGroups,
    /// Level 2 of a group of 20 tabs: two preview rings (`orbit-2-two-rings`).
    TwoRings,
    /// Level 1 with the query "map" (`orbit-1-searching`).
    Searching,
    /// One group and no pictures at level 2: placeholder cards (`orbit-1-empty`, drawn at level 2).
    Empty,
}

/// What `OrbitPreview` shows for a state of the machine.
pub struct Scene {
    pub window: OrbitPreview,
    pub machine: OrbitMachine,
    pub input: OrbitInput,
    pub models: OrbitModels,
    pub dark: bool,
    pub pictures: bool,
    /// A drag to show on top of the machine's own state (the mockup's new-group frame shows the
    /// ghost and the popover at once; the machine allows only one of them).
    pub ghost: Option<DragView>,
    pub data: OrbitData,
}

impl Scene {
    /// A window with the palette pushed and the machine opened on `input` (level 1); the first
    /// frame is built.
    pub fn new(input: OrbitInput, dark: bool, pictures: bool) -> Self {
        let window = OrbitPreview::new().expect("the preview can be created");
        let (look, _) = Look::from_appearance(&AppearanceSettings::default());
        apply_look(&window, &look);
        window.global::<ChromePalette>().set_theme(if dark {
            ChromeTheme::Dark
        } else {
            ChromeTheme::Light
        });
        window.set_reduce_motion(true);
        window.set_animate_spinners(false);
        let mut machine = OrbitMachine::new(7);
        machine.open(&input);
        let models = OrbitModels::new();
        let data = OrbitData::build(
            STAGE,
            &input,
            &machine.view(),
            &|_| None,
            &Options::default(),
        );
        let mut scene = Self {
            window,
            machine,
            input,
            models,
            dark,
            pictures,
            ghost: None,
            data,
        };
        scene.refresh();
        scene
    }

    /// The view the data is built from.
    pub fn view(&self) -> View {
        let mut view = self.machine.view();
        if let Some(ghost) = &self.ghost {
            view.drag = Some(ghost.clone());
        }
        // The machine opens with the keyboard focus on the active group and the layout lights
        // the focused planet; a drag is driven by the pointer (mockup `orbit-move-tab`), so the
        // scene shows no focus while one is in flight.
        if view.drag.is_some() {
            view.focus = None;
        }
        view
    }

    /// Rebuilds the data from the machine and the rows and puts it into the window.
    pub fn refresh(&mut self) {
        let view = self.view();
        let picture = skeleton_picture(self.dark);
        let with_pictures = self.pictures;
        let snapshots = move |_tab: i32| with_pictures.then(|| picture.clone());
        let options = Options {
            dark: self.dark,
            colours: RingColours::from_palette(&self.window.global::<ChromePalette>()),
            ..Options::default()
        };
        self.data = OrbitData::build(STAGE, &self.input, &view, &snapshots, &options);
        self.models.update(&self.data);
        let window = &self.window;
        window.set_level(match view.level {
            Level::Groups => OrbitLevel::Groups,
            Level::Group => OrbitLevel::Group,
            Level::Tab => OrbitLevel::Tab,
        });
        window.set_planets(self.models.planets());
        window.set_centre(self.models.centre());
        window.set_has_centre(self.models.has_centre());
        window.set_previews(self.models.previews());
        window.set_more_previews(self.models.more_previews());
        window.set_chips(self.models.chips());
        window.set_neighbours(self.models.neighbours());
        window.set_card(self.models.card());
        window.set_sun(self.models.sun());
        window.set_new_group(self.models.new_group());
        window.set_drag(self.models.drag());
        window.set_popover(self.models.popover());
        window.set_crumbs(self.models.crumbs());
        window.set_container_label(self.container_label(&view).into());
        window.set_query_active(!view.query.is_empty());
        window.set_search_text(view.query.as_str().into());
        window.set_focus_tab(match view.focus {
            Some(Target::Preview(tab)) => tab,
            _ => -1,
        });
    }

    /// "Orbit, every group", "Orbit, Research, 10 tabs", "Orbit, AccessKit guide, tab 4 of 10".
    fn container_label(&self, view: &View) -> String {
        let group = view.group.and_then(|g| self.input.group(g));
        let tab = view.tab.and_then(|t| self.input.tab(t));
        match (view.level, group, tab) {
            (Level::Group, Some(group), _) => {
                format!("Orbit, {}, {} tabs", group.name, group.count)
            }
            (Level::Tab, Some(group), Some(tab)) => {
                let own = self.input.tabs_of(group.id);
                let place = own.iter().position(|t| t.id == tab.id).map_or(0, |i| i + 1);
                format!("Orbit, {}, tab {place} of {}", tab.title, own.len())
            }
            _ => "Orbit, every group".to_string(),
        }
    }

    /// The slint row of the planet of this group.
    pub fn planet(&self, group: i32) -> &mercury_ui::OrbitPlanet {
        &self
            .data
            .planets
            .iter()
            .find(|p| p.row.group == group)
            .expect("the planet of that group")
            .row
    }

    /// Draws the window at 1440 x 900 logical px and scale 2 (software renderer).
    pub fn render(&self) -> Pixels {
        let window = self.window.window();
        self.window.show().expect("the preview can be shown");
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: SCALE,
        });
        window.set_size(PhysicalSize::new(
            (STAGE.width * SCALE) as u32,
            (STAGE.height * SCALE) as u32,
        ));
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            window.request_redraw();
            let _ = window.take_snapshot();
        }
        window.take_snapshot().expect("the window renders")
    }
}

/// The scene of a frame, light or dark. `Empty` is the one-group, no-picture level-2 scene.
pub fn scene(frame: Frame, dark: bool) -> Scene {
    let research = 1;
    let mut scene = match frame {
        Frame::Overview
        | Frame::Group
        | Frame::Tab
        | Frame::NewGroup
        | Frame::MoveTab
        | Frame::Searching => Scene::new(orbit_input(dark), dark, true),
        Frame::ManyGroups => Scene::new(generated_input(12, 60, dark), dark, true),
        Frame::TwoRings => Scene::new(generated_input(3, 60, dark), dark, true),
        Frame::Empty => {
            let groups = vec![orbit_groups(dark)[0].clone()];
            let tabs: Vec<_> = orbit_tabs().into_iter().filter(|t| t.group == 1).collect();
            let input = OrbitInput::new(&groups, &tabs, "Personal", "Window 1", 101, dark);
            Scene::new(input, dark, false)
        }
    };
    match frame {
        Frame::Overview => scene.machine.hover(Target::Planet(research)),
        Frame::Group => {
            scene.machine.click(Target::Planet(research));
            scene.machine.hover(Target::Preview(tab_id(0, 3)));
        }
        Frame::Tab => {
            scene.machine.click(Target::Planet(research));
            scene.machine.click(Target::Preview(tab_id(0, 3)));
        }
        Frame::MoveTab => {
            let source = Source::Tab(tab_id(1, 1));
            scene.machine.drag_started(source);
            scene.machine.drag_over(Some(DropTarget::Group {
                group: 3,
                slot: Some(3),
            }));
        }
        Frame::NewGroup => {
            let source = Source::Tab(tab_id(1, 2));
            scene.machine.drag_started(source);
            scene.machine.drag_over(Some(DropTarget::NewGroup));
            let over = Some(DropTarget::NewGroup);
            scene.machine.drag_released();
            scene.machine.popover_edited("Untitled group", "blue");
            scene.ghost = Some(DragView { source, over });
        }
        Frame::ManyGroups => scene.machine.hover(Target::Planet(1)),
        Frame::TwoRings => {
            scene.machine.click(Target::Planet(1));
        }
        Frame::Searching => {
            let query = "map";
            let mut input = scene.input.clone();
            for tab in &mut input.tabs {
                tab.matches_query = search::matches(query, &tab.title, &tab.host);
            }
            scene.machine.rows_changed(&input);
            scene.input = input;
            scene.machine.search_changed(query);
        }
        Frame::Empty => {
            scene.machine.click(Target::Planet(research));
        }
    }
    scene.refresh();
    scene
}

pub fn file_name(frame: Frame) -> &'static str {
    match frame {
        Frame::Overview => "orbit-1-overview",
        Frame::Group => "orbit-2-group",
        Frame::Tab => "orbit-3-tab",
        Frame::NewGroup => "orbit-new-group",
        Frame::MoveTab => "orbit-move-tab",
        Frame::ManyGroups => "orbit-1-many-groups",
        Frame::TwoRings => "orbit-2-two-rings",
        Frame::Searching => "orbit-1-searching",
        Frame::Empty => "orbit-1-empty",
    }
}

pub fn save(pixels: &Pixels, path: &Path) {
    image::save_buffer_with_format(
        path,
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .expect("the rendering can be saved");
    println!("wrote {}", path.display());
}

/// The pixel at a stage point (logical px) of a frame drawn at scale 2.
pub fn at(pixels: &Pixels, x: f32, y: f32) -> Rgba8Pixel {
    let (px, py) = ((x * SCALE) as u32, (y * SCALE) as u32);
    pixels.as_slice()[(py * pixels.width() + px) as usize]
}

pub fn rgb(pixel: Rgba8Pixel) -> (u8, u8, u8) {
    (pixel.r, pixel.g, pixel.b)
}
