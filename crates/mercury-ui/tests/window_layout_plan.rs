// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `window_layout::plan`: what a window shows for the stored settings, the window's own session
//! values, whether the interface is live and the platform (layout-switch plan, task 2).

use mercury_settings::{AppearanceSettings, DialCorner, DockStart, TabGroupSettings};
use mercury_ui::window_layout::{ChromePlan, Layout, Platform, WindowChrome, plan};

fn appearance(layout: &str, tab_groups: TabGroupSettings) -> AppearanceSettings {
    AppearanceSettings {
        layout: layout.to_owned(),
        tab_groups,
        ..AppearanceSettings::default()
    }
}

fn live(appearance: &AppearanceSettings) -> ChromePlan {
    plan(appearance, &WindowChrome::default(), true, Platform::MacOs)
}

fn groups(f: impl FnOnce(&mut TabGroupSettings)) -> TabGroupSettings {
    let mut groups = TabGroupSettings::default();
    f(&mut groups);
    groups
}

#[test]
fn defaults_give_the_mercury_layout_with_dock_hub_and_dial() {
    let plan = live(&AppearanceSettings::default());
    assert_eq!(plan.layout, Layout::Mercury);
    assert!(!plan.show_tab_row);
    assert!(plan.show_hub);
    assert!(plan.show_dock);
    assert!(plan.show_dial);
    assert!(plan.orbit_enabled);
    assert!(plan.orbit_pinch);
    assert!(plan.integrated_title_bar);
    assert!(!plan.dock_starts_shrunk);
    assert_eq!(plan.dial_corner, DialCorner::BottomRight);
}

#[test]
fn classic_has_a_tab_row_and_no_dock_by_default() {
    let plan = live(&appearance("classic", TabGroupSettings::default()));
    assert_eq!(plan.layout, Layout::Classic);
    assert!(plan.show_tab_row);
    assert!(!plan.show_dock, "the dock in Classic is optional and off");
    assert!(plan.show_hub, "the hub shows in both layouts");
}

#[test]
fn classic_with_dock_in_classic_shows_the_dock() {
    let plan = live(&appearance(
        "classic",
        groups(|g| {
            g.dock_in_classic = true;
        }),
    ));
    assert!(plan.show_dock);
    assert!(plan.show_tab_row);
}

#[test]
fn the_dock_setting_of_the_other_layout_does_not_leak() {
    let mercury = live(&appearance(
        "mercury",
        groups(|g| {
            g.dock = false;
            g.dock_in_classic = true;
        }),
    ));
    assert!(
        !mercury.show_dock,
        "Mercury follows `dock`, not `dock_in_classic`"
    );
    let classic = live(&appearance(
        "classic",
        groups(|g| {
            g.dock = true;
            g.dock_in_classic = false;
        }),
    ));
    assert!(
        !classic.show_dock,
        "Classic follows `dock_in_classic`, not `dock`"
    );
}

#[test]
fn mercury_with_neither_dock_nor_hub_shows_the_tab_row() {
    let plan = live(&appearance(
        "mercury",
        groups(|g| {
            g.dock = false;
            g.ring = false;
        }),
    ));
    assert_eq!(plan.layout, Layout::Mercury);
    assert!(plan.show_tab_row);
    assert!(!plan.show_dock);
    assert!(!plan.show_hub);
}

#[test]
fn mercury_with_hub_only_has_no_tab_row() {
    let plan = live(&appearance(
        "mercury",
        groups(|g| {
            g.dock = false;
            g.ring = true;
        }),
    ));
    assert!(!plan.show_tab_row);
    assert!(plan.show_hub);
    assert!(!plan.show_dock);
}

#[test]
fn mercury_with_dock_only_has_no_tab_row() {
    let plan = live(&appearance(
        "mercury",
        groups(|g| {
            g.dock = true;
            g.ring = false;
        }),
    ));
    assert!(!plan.show_tab_row);
    assert!(plan.show_dock);
}

#[test]
fn an_unknown_layout_id_is_mercury() {
    for id in ["", "sidebar", "Classic", "orbit"] {
        let plan = live(&appearance(id, TabGroupSettings::default()));
        assert_eq!(plan.layout, Layout::Mercury, "{id:?}");
    }
    assert_eq!(Layout::from_id("classic"), Layout::Classic);
    assert_eq!(Layout::from_id("mercury"), Layout::Mercury);
}

#[test]
fn layout_ids_round_trip() {
    for layout in [Layout::Mercury, Layout::Classic] {
        assert_eq!(Layout::from_id(layout.id()), layout);
    }
}

#[test]
fn layouts_map_to_the_slint_enum() {
    use mercury_ui::WindowLayout;
    assert_eq!(Layout::Mercury.to_slint(), WindowLayout::Mercury);
    assert_eq!(Layout::Classic.to_slint(), WindowLayout::Classic);
}

#[test]
fn not_live_is_todays_window() {
    // Whatever the settings and session say, a window whose interface is not live is today's.
    let everything_on = appearance("mercury", TabGroupSettings::default());
    let session = WindowChrome {
        dock_shrunk: Some(true),
        dial_corner: Some("top_left".to_owned()),
    };
    for platform in [Platform::MacOs, Platform::Other] {
        let plan = plan(&everything_on, &session, false, platform);
        assert_eq!(plan.layout, Layout::Classic);
        assert!(plan.show_tab_row);
        assert!(!plan.show_hub);
        assert!(!plan.show_dock);
        assert!(!plan.show_dial);
        assert!(!plan.orbit_enabled);
        assert!(!plan.orbit_pinch);
        assert!(!plan.integrated_title_bar);
        assert!(!plan.dock_starts_shrunk);
        assert_eq!(plan, ChromePlan::todays_window());
    }
}

#[test]
fn session_dock_shrunk_beats_dock_starts() {
    for starts in [DockStart::Open, DockStart::Shrunk] {
        let settings = appearance("mercury", groups(|g| g.dock_starts = starts));
        let with = |session: Option<bool>| {
            let chrome = WindowChrome {
                dock_shrunk: session,
                dial_corner: None,
            };
            plan(&settings, &chrome, true, Platform::MacOs).dock_starts_shrunk
        };
        assert!(with(Some(true)), "{starts:?}: the window was shrunk");
        assert!(!with(Some(false)), "{starts:?}: the window was open");
        assert_eq!(
            with(None),
            starts == DockStart::Shrunk,
            "{starts:?}: follows the setting"
        );
    }
}

#[test]
fn session_dial_corner_beats_the_setting_and_an_unknown_name_falls_back() {
    let settings = appearance("mercury", groups(|g| g.dial_corner = DialCorner::TopRight));
    let with = |name: Option<&str>| {
        let chrome = WindowChrome {
            dock_shrunk: None,
            dial_corner: name.map(str::to_owned),
        };
        plan(&settings, &chrome, true, Platform::MacOs).dial_corner
    };
    assert_eq!(with(None), DialCorner::TopRight);
    assert_eq!(with(Some("bottom_left")), DialCorner::BottomLeft);
    assert_eq!(with(Some("top_left")), DialCorner::TopLeft);
    assert_eq!(with(Some("middle")), DialCorner::TopRight);
    assert_eq!(with(Some("")), DialCorner::TopRight);
}

#[test]
fn orbit_pinch_needs_orbit() {
    let settings = |orbit, pinch| {
        appearance(
            "mercury",
            groups(|g| {
                g.orbit = orbit;
                g.orbit_pinch = pinch;
            }),
        )
    };
    let both = live(&settings(true, true));
    assert!(both.orbit_enabled && both.orbit_pinch);
    let pinch_only = live(&settings(false, true));
    assert!(!pinch_only.orbit_enabled && !pinch_only.orbit_pinch);
    let orbit_only = live(&settings(true, false));
    assert!(orbit_only.orbit_enabled && !orbit_only.orbit_pinch);
}

#[test]
fn no_title_bar_integration_off_macos() {
    let settings = AppearanceSettings::default();
    let chrome = WindowChrome::default();
    assert!(plan(&settings, &chrome, true, Platform::MacOs).integrated_title_bar);
    assert!(!plan(&settings, &chrome, true, Platform::Other).integrated_title_bar);
}

#[test]
fn the_dial_follows_its_setting_in_both_layouts() {
    for layout in ["mercury", "classic"] {
        let on = live(&appearance(layout, groups(|g| g.dial = true)));
        let off = live(&appearance(layout, groups(|g| g.dial = false)));
        assert!(on.show_dial, "{layout}");
        assert!(!off.show_dial, "{layout}");
    }
}

#[test]
fn every_combination_always_has_a_way_to_see_tabs() {
    let bools = [false, true];
    for layout in ["mercury", "classic", "unknown"] {
        for dock in bools {
            for dock_in_classic in bools {
                for ring in bools {
                    for interface_live in bools {
                        let settings = appearance(
                            layout,
                            groups(|g| {
                                g.dock = dock;
                                g.dock_in_classic = dock_in_classic;
                                g.ring = ring;
                            }),
                        );
                        let plan = plan(
                            &settings,
                            &WindowChrome::default(),
                            interface_live,
                            Platform::MacOs,
                        );
                        assert!(
                            plan.show_tab_row || plan.show_hub || plan.show_dock,
                            "{layout} dock={dock} dock_in_classic={dock_in_classic} \
                             ring={ring} live={interface_live}: no way to see the tabs"
                        );
                        if !interface_live {
                            assert!(plan.show_tab_row, "a window that is not live has the row");
                        }
                    }
                }
            }
        }
    }
}
