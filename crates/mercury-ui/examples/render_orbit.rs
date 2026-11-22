// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the Orbit overview to PNG files without a display, through Slint's software renderer
//! (1440 x 900 logical px at 2x). Every frame is built through the real pipeline (the fixtures, the
//! `OrbitMachine`, `OrbitData::build`, `OrbitModels::update`) by `tests/orbit_scenes/mod.rs`, with a
//! synthetic skeleton picture so that the image path is the real one.
//!
//! `cargo run --release -p mercury-ui --features test-support --example render_orbit -- <output directory>`
//!
//! Writes the five mockup frames in light and dark (`orbit-1-overview`, `orbit-2-group`,
//! `orbit-3-tab`, `orbit-new-group`, `orbit-move-tab`) and four extras: `orbit-1-many-groups-light`
//! (12 groups), `orbit-2-two-rings-light` (a 20-tab group), `orbit-1-searching-light` and
//! `orbit-1-empty-dark` (one group and no pictures: the placeholder cards, at level 2).

#[cfg(feature = "test-support")]
#[allow(dead_code)]
#[path = "../tests/orbit_scenes/mod.rs"]
mod scenes;

#[cfg(feature = "test-support")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use scenes::{Frame, file_name, init_software, save, scene};
    use slint::ComponentHandle;

    let output = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_orbit <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;
    init_software();
    let mut jobs: Vec<(Frame, bool)> = Vec::new();
    for frame in [
        Frame::Overview,
        Frame::Group,
        Frame::Tab,
        Frame::NewGroup,
        Frame::MoveTab,
    ] {
        jobs.push((frame, false));
        jobs.push((frame, true));
    }
    jobs.push((Frame::ManyGroups, false));
    jobs.push((Frame::TwoRings, false));
    jobs.push((Frame::Searching, false));
    jobs.push((Frame::Empty, true));
    for (frame, dark) in jobs {
        let scene = scene(frame, dark);
        let name = format!(
            "{}-{}.png",
            file_name(frame),
            if dark { "dark" } else { "light" }
        );
        save(&scene.render(), &output.join(name));
        scene.window.hide()?;
    }
    Ok(())
}

#[cfg(not(feature = "test-support"))]
fn main() {
    eprintln!("render_orbit needs --features test-support (it draws the lane's fixtures)");
}
