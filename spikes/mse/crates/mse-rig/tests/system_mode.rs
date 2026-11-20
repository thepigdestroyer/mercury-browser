// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The `system` plugin mode: the framework's plugins are scanned in process into a private registry
//! (through a directory of symlinks that leaves out the Vulkan plugin, which hangs a fresh scan on
//! some Macs), and the result plays the same fixture as the `servo` mode does. A process of its own
//! because GStreamer is initialised once per process.

mod common;

use mse_rig::backend::MseBackend;
use mse_rig::env::{PluginMode, registry_file};
use mse_rig::rig::Shape;

use common::{colour_letters, init, one_rig_at_a_time, play_fixture, scratch};

#[test]
fn the_system_scan_finds_msesrc_and_plays_a_muxed_fixture() {
    let _one = one_rig_at_a_time();
    let info = init(PluginMode::SystemPath);
    assert_eq!(info.plugin_mode, "system-path");
    assert!(info.msesrc_available, "{info:?}");
    assert!(info.failed_plugins.is_empty(), "{:?}", info.failed_plugins);
    assert!(
        registry_file(scratch()).is_file(),
        "the private registry was written"
    );
    assert!(MseBackend::is_type_supported(
        "video/mp4; codecs=\"avc1.42C00C\""
    ));

    let cycle = play_fixture(Shape::PlayBin, "h264-aac");
    assert!(cycle.eos);
    let video = cycle.rig.video.lock().unwrap().clone();
    assert!((198..=202).contains(&video.count), "{} frames", video.count);
    let letters = colour_letters(&video.colour);
    assert!(
        letters.starts_with('R') && letters.ends_with('B'),
        "{letters}"
    );
    cycle.rig.shutdown().expect("teardown");
}
