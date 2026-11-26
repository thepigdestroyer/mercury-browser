// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Generator of the six palettes designed on 22 Nov 2026.
//!
//! Each palette is a short list of design tokens per scheme (surfaces, ink, accent fill). The
//! 54 roles are derived from them, and every role that a contrast rule holds to a ratio is
//! tuned (a few percent at a time, toward ink) until it passes the crate's own `RULES` with a
//! small margin. The result is written as ordinary palette data (two exhaustive tables of
//! literals), which is what the crate ships; nothing here runs in the browser.
//!
//! Usage: `cargo run --example palette_gen -- <crate dir>` rewrites
//! `<crate dir>/src/palettes/<id>.rs` for each of them. `palettes/mod.rs` (the list and its
//! order) is written by hand.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use mercury_appearance::{Colour, EXEMPT, RULES, Role};

type Map = BTreeMap<Role, Colour>;

const TEXT_MARGIN: f64 = 0.1;

/// The palettes the owner chose to ship (22 Nov 2026): the only ones a user may select.
const SELECTABLE: [&str; 5] = ["signal", "forest", "abyss", "glacier", "ember"];

/// The design tokens of one scheme.
struct Scheme {
    dark: bool,
    frame: u32,
    bar: u32,
    content: u32,
    field: u32,
    field_border: u32,
    line: u32,
    popup: u32,
    tab_active: u32,
    tab_hover: u32,
    btn_hover: u32,
    btn_pressed: u32,
    item_hover: u32,
    ink: u32,
    fill: u32,
    on_fill: u32,
    notice_bg: u32,
    notice_border: u32,
    info_bg: u32,
    info_border: u32,
    danger: u32,
    /// How far down the text tiers go: 1.0 is a soft hierarchy, lower is stronger secondary text.
    soft: f64,
}

struct Candidate {
    id: &'static str,
    name: &'static str,
    mood: &'static str,
    light: Scheme,
    dark: Scheme,
}

fn c(hex: u32) -> Colour {
    Colour::hex(hex)
}

fn candidates() -> Vec<Candidate> {
    vec![
        Candidate {
            id: "abyss",
            name: "Abyss",
            mood: "Deep ocean: sea-glass white by day, a trench of blue-green by night, with a bright cyan-turquoise accent.",
            light: Scheme {
                dark: false,
                frame: 0xd3e6e8,
                bar: 0xeef7f7,
                content: 0xe9f3f4,
                field: 0xffffff,
                field_border: 0x9fbfc3,
                line: 0xbcd5d8,
                popup: 0xf8fcfc,
                tab_active: 0xffffff,
                tab_hover: 0xc6dde0,
                btn_hover: 0xdcecee,
                btn_pressed: 0xcce2e5,
                item_hover: 0xdcecee,
                ink: 0x062a31,
                fill: 0x00998a,
                on_fill: 0xffffff,
                notice_bg: 0xfdf0cb,
                notice_border: 0xe0c677,
                info_bg: 0xefe9f7,
                info_border: 0xd0c6e6,
                danger: 0xc62828,
                soft: 1.0,
            },
            dark: Scheme {
                dark: true,
                frame: 0x04141a,
                bar: 0x09222a,
                content: 0x061a20,
                field: 0x11353f,
                field_border: 0x23596a,
                line: 0x1a4350,
                popup: 0x0d2c36,
                tab_active: 0x11353f,
                tab_hover: 0x0c2a33,
                btn_hover: 0x11353f,
                btn_pressed: 0x18444f,
                item_hover: 0x14333d,
                ink: 0xe4f7f5,
                fill: 0x2ee6c0,
                on_fill: 0x021719,
                notice_bg: 0x2e2a10,
                notice_border: 0x6a5c1f,
                info_bg: 0x2a2147,
                info_border: 0x463a78,
                danger: 0xff8a80,
                soft: 1.0,
            },
        },
        Candidate {
            id: "glacier",
            name: "Glacier",
            mood: "Arctic ice: frosted blue-white chrome with a vivid cyan accent; steel-blue polar night in the dark.",
            light: Scheme {
                dark: false,
                frame: 0xd8e9f5,
                bar: 0xf2f9fd,
                content: 0xecf5fb,
                field: 0xffffff,
                field_border: 0xa2c1d6,
                line: 0xc0d7e6,
                popup: 0xf9fcfe,
                tab_active: 0xffffff,
                tab_hover: 0xc9e0f0,
                btn_hover: 0xdeedf8,
                btn_pressed: 0xcee3f2,
                item_hover: 0xdeedf8,
                ink: 0x082233,
                fill: 0x0096c7,
                on_fill: 0xffffff,
                notice_bg: 0xfdf0cb,
                notice_border: 0xe0c677,
                info_bg: 0xf0eaf8,
                info_border: 0xd3c8e8,
                danger: 0xc62828,
                soft: 1.0,
            },
            dark: Scheme {
                dark: true,
                frame: 0x0a1a28,
                bar: 0x112a3d,
                content: 0x0c2031,
                field: 0x1b3a52,
                field_border: 0x2f5878,
                line: 0x264a65,
                popup: 0x153248,
                tab_active: 0x1b3a52,
                tab_hover: 0x142f44,
                btn_hover: 0x1b3a52,
                btn_pressed: 0x25496a,
                item_hover: 0x1d3d56,
                ink: 0xe9f7fe,
                fill: 0x27d0ff,
                on_fill: 0x03202c,
                notice_bg: 0x2e2a10,
                notice_border: 0x6a5c1f,
                info_bg: 0x2a2147,
                info_border: 0x463a78,
                danger: 0xff8a80,
                soft: 1.0,
            },
        },
        Candidate {
            id: "graphite",
            name: "Graphite",
            mood: "Studio graphite: true neutral greys, no tint, with one electric-violet accent.",
            light: Scheme {
                dark: false,
                frame: 0xe2e3e6,
                bar: 0xf5f5f7,
                content: 0xf0f0f2,
                field: 0xffffff,
                field_border: 0xb4b6bd,
                line: 0xcdced3,
                popup: 0xfafafb,
                tab_active: 0xffffff,
                tab_hover: 0xd5d6da,
                btn_hover: 0xe5e6e9,
                btn_pressed: 0xd8d9de,
                item_hover: 0xe7e8eb,
                ink: 0x16171b,
                fill: 0x6a52ff,
                on_fill: 0xffffff,
                notice_bg: 0xfdf0cb,
                notice_border: 0xe0c677,
                info_bg: 0xf6ecf0,
                info_border: 0xe0cdd6,
                danger: 0xc62828,
                soft: 1.0,
            },
            dark: Scheme {
                dark: true,
                frame: 0x121316,
                bar: 0x1c1d21,
                content: 0x17181b,
                field: 0x27292e,
                field_border: 0x3f424a,
                line: 0x30333a,
                popup: 0x212327,
                tab_active: 0x27292e,
                tab_hover: 0x1f2024,
                btn_hover: 0x27292e,
                btn_pressed: 0x31343b,
                item_hover: 0x2b2d33,
                ink: 0xf2f3f5,
                fill: 0x8a7dff,
                on_fill: 0x0c0933,
                notice_bg: 0x2e2a10,
                notice_border: 0x6a5c1f,
                info_bg: 0x2d2127,
                info_border: 0x5a3f4b,
                danger: 0xff8a80,
                soft: 1.0,
            },
        },
        Candidate {
            id: "midnight",
            name: "Midnight",
            mood: "Midnight indigo with an aurora-green accent: periwinkle paper by day, a saturated deep blue night by night.",
            light: Scheme {
                dark: false,
                frame: 0xdde0f5,
                bar: 0xf4f5ff,
                content: 0xeceffc,
                field: 0xffffff,
                field_border: 0xabb1d9,
                line: 0xc6cae8,
                popup: 0xfafaff,
                tab_active: 0xffffff,
                tab_hover: 0xd0d4f0,
                btn_hover: 0xe1e4f8,
                btn_pressed: 0xd3d7f2,
                item_hover: 0xe3e6f9,
                ink: 0x10143a,
                fill: 0x0f9a45,
                on_fill: 0xffffff,
                notice_bg: 0xfdf0cb,
                notice_border: 0xe0c677,
                info_bg: 0xf4e8f5,
                info_border: 0xe0c6e4,
                danger: 0xc62828,
                soft: 1.0,
            },
            dark: Scheme {
                dark: true,
                frame: 0x060922,
                bar: 0x0d1240,
                content: 0x090d33,
                field: 0x192063,
                field_border: 0x2e3688,
                line: 0x242b73,
                popup: 0x131951,
                tab_active: 0x192063,
                tab_hover: 0x111752,
                btn_hover: 0x192063,
                btn_pressed: 0x252d78,
                item_hover: 0x1c2468,
                ink: 0xecefff,
                fill: 0x5cf28a,
                on_fill: 0x04142b,
                notice_bg: 0x2e2a10,
                notice_border: 0x6a5c1f,
                info_bg: 0x34204a,
                info_border: 0x5a3a7c,
                danger: 0xff8a80,
                soft: 1.0,
            },
        },
        Candidate {
            id: "forest",
            name: "Forest",
            mood: "Moss and mint: sage-green paper by day, a dark forest floor by night, fresh lime-mint accent.",
            light: Scheme {
                dark: false,
                frame: 0xdbe7d4,
                bar: 0xf2f7ee,
                content: 0xecf3e7,
                field: 0xffffff,
                field_border: 0xaabf9f,
                line: 0xc3d4ba,
                popup: 0xf8fbf5,
                tab_active: 0xffffff,
                tab_hover: 0xcddec3,
                btn_hover: 0xdfe9d8,
                btn_pressed: 0xd0decb,
                item_hover: 0xe0ebd9,
                ink: 0x13261a,
                fill: 0x4f9412,
                on_fill: 0xffffff,
                notice_bg: 0xfdf0cb,
                notice_border: 0xe0c677,
                info_bg: 0xf1ebf6,
                info_border: 0xd5cae4,
                danger: 0xc62828,
                soft: 1.0,
            },
            dark: Scheme {
                dark: true,
                frame: 0x09130c,
                bar: 0x11201a,
                content: 0x0c1a11,
                field: 0x1b3123,
                field_border: 0x31503b,
                line: 0x28422f,
                popup: 0x15271b,
                tab_active: 0x1b3123,
                tab_hover: 0x14261b,
                btn_hover: 0x1b3123,
                btn_pressed: 0x26412f,
                item_hover: 0x1f3626,
                ink: 0xe8f6ec,
                fill: 0xa8e86a,
                on_fill: 0x121f08,
                notice_bg: 0x2e2a10,
                notice_border: 0x6a5c1f,
                info_bg: 0x2a2147,
                info_border: 0x463a78,
                danger: 0xff8a80,
                soft: 1.0,
            },
        },
        Candidate {
            id: "signal",
            name: "Signal",
            mood: "Bold and stark: paper white and ink black with hard hairlines and one hot electric-pink accent.",
            light: Scheme {
                dark: false,
                frame: 0xe4e4e8,
                bar: 0xffffff,
                content: 0xffffff,
                field: 0xffffff,
                field_border: 0x55555e,
                line: 0x3a3a42,
                popup: 0xffffff,
                tab_active: 0xffffff,
                tab_hover: 0xd2d2d8,
                btn_hover: 0xe8e8ec,
                btn_pressed: 0xd6d6dc,
                item_hover: 0xececf0,
                ink: 0x000000,
                fill: 0xe0007e,
                on_fill: 0xffffff,
                notice_bg: 0xfff2a8,
                notice_border: 0x8a7600,
                info_bg: 0xece4ff,
                info_border: 0x5b3fd0,
                danger: 0xc40018,
                soft: 0.55,
            },
            dark: Scheme {
                dark: true,
                frame: 0x000000,
                bar: 0x0b0b0d,
                content: 0x000000,
                field: 0x151518,
                field_border: 0x8c8c96,
                line: 0x55555e,
                popup: 0x101013,
                tab_active: 0x222226,
                tab_hover: 0x151518,
                btn_hover: 0x1f1f23,
                btn_pressed: 0x2c2c32,
                item_hover: 0x1f1f23,
                ink: 0xffffff,
                fill: 0xff4fb6,
                on_fill: 0x000000,
                notice_bg: 0x3a3200,
                notice_border: 0xa68f00,
                info_bg: 0x2a1f55,
                info_border: 0x7a5cf0,
                danger: 0xff8a8a,
                soft: 0.55,
            },
        },
        Candidate {
            id: "ember",
            name: "Ember",
            mood: "Mercury orange on white by day, orange on true OLED black by night: the brand's own sun colour.",
            light: Scheme {
                dark: false,
                frame: 0xf5f3f1,
                bar: 0xffffff,
                content: 0xffffff,
                field: 0xffffff,
                field_border: 0xb5aca3,
                line: 0xe0dad4,
                popup: 0xffffff,
                tab_active: 0xffffff,
                tab_hover: 0xebe7e3,
                btn_hover: 0xf3f0ed,
                btn_pressed: 0xeeeae6,
                item_hover: 0xf4f1ee,
                ink: 0x1b1511,
                // The brand's sun rim, `#EC5B27`; tuned darker until a white label reads on it
                // (4.6:1) and it clears 3:1 on every surface.
                fill: 0xec5b27,
                on_fill: 0xffffff,
                notice_bg: 0xfff3cd,
                notice_border: 0xd8b84a,
                info_bg: 0xf1eafa,
                info_border: 0xbfa9e3,
                danger: 0xc62828,
                soft: 0.9,
            },
            dark: Scheme {
                dark: true,
                frame: 0x000000,
                bar: 0x0a0a0a,
                content: 0x000000,
                field: 0x0a0a0a,
                field_border: 0x3d3d3d,
                line: 0x1f1f1f,
                popup: 0x0a0a0a,
                tab_active: 0x0a0a0a,
                tab_hover: 0x111111,
                btn_hover: 0x121212,
                btn_pressed: 0x1a1a1a,
                item_hover: 0x121212,
                ink: 0xf6f1ec,
                // The brand's sun, the middle stop of `AppIcon.svg` (`#FF8A2F`).
                fill: 0xff8a2f,
                on_fill: 0x000000,
                notice_bg: 0x2a2106,
                notice_border: 0x7a6414,
                info_bg: 0x1c1230,
                info_border: 0x4a3a7a,
                danger: 0xff8a80,
                soft: 0.9,
            },
        },
    ]
}

// ---------------------------------------------------------------------------------------------

fn needs(role: Role) -> Vec<(Role, f64)> {
    let mut out = Vec::new();
    for rule in RULES {
        if rule.foreground == role {
            for &background in rule.backgrounds {
                out.push((background, rule.minimum.ratio()));
            }
        }
    }
    out
}

/// The smallest amount by which `colour` clears the rules held to `role`.
fn slack(map: &Map, role: Role, colour: Colour) -> f64 {
    needs(role)
        .into_iter()
        .map(|(background, minimum)| {
            let bg = map[&background];
            colour.over(bg).contrast_ratio(bg) - minimum
        })
        .fold(f64::MAX, f64::min)
}

/// `seed` moved toward `toward` one percent at a time until the rules of `role` hold with a
/// margin and `extra` is true.
fn tune(
    map: &Map,
    role: Role,
    seed: Colour,
    toward: Colour,
    extra: &dyn Fn(Colour) -> bool,
) -> Colour {
    for step in 0..=100 {
        let colour = seed.mix(toward, f64::from(step) / 100.0);
        if slack(map, role, colour) >= TEXT_MARGIN && extra(colour) {
            return colour;
        }
    }
    toward
}

fn build(s: &Scheme) -> Map {
    use Role as R;
    let ink = c(s.ink);
    let extreme = if s.dark { Colour::WHITE } else { Colour::BLACK };
    let frame = c(s.frame);
    let bar = c(s.bar);
    let field = c(s.field);
    let popup = c(s.popup);
    let on_fill = c(s.on_fill);
    let mut m = Map::new();

    // Surfaces and fills that are given, not tuned.
    m.insert(R::WindowBackground, frame);
    m.insert(R::ToolbarBackground, bar);
    m.insert(R::ToolbarDivider, c(s.line));
    m.insert(R::ContentBackground, c(s.content));
    m.insert(R::FieldBackground, field);
    m.insert(R::FieldBorder, c(s.field_border));
    m.insert(R::SearchFieldBackground, field);
    m.insert(R::SearchFieldBorder, c(s.line));
    m.insert(R::Text, ink);
    m.insert(R::ButtonHover, c(s.btn_hover));
    m.insert(R::ButtonPressed, c(s.btn_pressed));
    m.insert(R::TabActive, c(s.tab_active));
    m.insert(R::TabHover, c(s.tab_hover));
    m.insert(R::TabCloseHover, frame.mix(ink, 0.10));
    m.insert(R::TabClosePressed, frame.mix(ink, 0.18));
    m.insert(R::StatusBackground, popup);
    m.insert(R::StatusBorder, c(s.field_border));
    m.insert(R::NoticeBackground, c(s.notice_bg));
    m.insert(R::NoticeBorder, c(s.notice_border));
    m.insert(R::InfoBackground, c(s.info_bg));
    m.insert(R::InfoBorder, c(s.info_border));
    m.insert(R::InfoText, ink);
    m.insert(R::PopupBackground, popup);
    m.insert(R::PopupBorder, c(s.field_border));
    m.insert(R::ItemHover, c(s.item_hover));
    m.insert(R::Separator, c(s.line));
    m.insert(R::DisabledText, popup.mix(ink, 0.38));
    m.insert(R::SheetBackground, popup);
    let (scrim, shadow) = if s.dark {
        (Colour::rgba(0, 0, 0, 0x99), Colour::rgba(0, 0, 0, 0x80))
    } else {
        (
            Colour::rgba(ink.r, ink.g, ink.b, 0x4d),
            Colour::rgba(ink.r, ink.g, ink.b, 0x40),
        )
    };
    m.insert(R::Scrim, scrim);
    m.insert(R::Shadow, shadow);
    // Page popups are the same neutral grey in every palette, so they never pass for the
    // browser's own menus.
    if s.dark {
        m.insert(R::PagePopupBackground, c(0x2b2b2e));
        m.insert(R::PagePopupBorder, c(0x4b4b50));
        m.insert(R::PagePopupCaption, c(0xa2a2a8));
    } else {
        m.insert(R::PagePopupBackground, c(0xfdfdfd));
        m.insert(R::PagePopupBorder, c(0xb9b9be));
        m.insert(R::PagePopupCaption, c(0x6e6e73));
    }
    m.insert(R::CaptureIndicatorGlyph, Colour::WHITE);

    // The primary fill: a mark (3:1) that its label reads on (4.5:1).
    let label_ok = |fill: Colour| fill.contrast_ratio(on_fill) >= 4.6;
    let seed = c(s.fill);
    // The fill's own rule is on `PrimaryButtonBackground`.
    let fill = tune(&m, R::PrimaryButtonBackground, seed, extreme, &label_ok);
    m.insert(R::PrimaryButtonBackground, fill);
    let (hover, pressed) = if s.dark {
        (fill.lighter(0.12), fill.darker(0.10))
    } else {
        (fill.darker(0.10), fill.darker(0.20))
    };
    m.insert(R::PrimaryButtonHover, hover);
    m.insert(R::PrimaryButtonPressed, pressed);
    m.insert(R::PrimaryButtonText, on_fill);
    m.insert(R::ItemSelected, fill);
    m.insert(R::ItemSelectedText, on_fill);

    // The accent as text: the fill's hue, moved until it reads on every surface it is drawn on and
    // the primary label reads on it.
    let accent = tune(&m, R::Accent, fill, extreme, &label_ok);
    m.insert(R::Accent, accent);
    let hover_label = tune(
        &m,
        R::ButtonLabelHover,
        accent.mix(extreme, 0.15),
        extreme,
        &|_| true,
    );
    m.insert(R::ButtonLabelHover, hover_label);
    let highlight = tune(&m, R::MatchHighlight, accent, extreme, &|_| true);
    m.insert(R::MatchHighlight, highlight);
    let ring = tune(&m, R::FocusRing, fill, extreme, &|_| true);
    m.insert(R::FocusRing, ring);

    let danger = tune(&m, R::Danger, c(s.danger), extreme, &|_| true);
    m.insert(R::Danger, danger);
    let capture_seed = if s.dark { c(0xf2544c) } else { c(0xd93025) };
    let capture = tune(&m, R::CaptureIndicator, capture_seed, extreme, &|colour| {
        colour.contrast_ratio(Colour::WHITE) >= 3.1
    });
    m.insert(R::CaptureIndicator, capture);

    // The text tiers: as soft as the hierarchy wants, then moved toward ink until they pass.
    let soft = s.soft;
    let secondary = tune(
        &m,
        R::TextSecondary,
        ink.mix(bar, 0.30 * soft),
        ink,
        &|_| true,
    );
    let muted = tune(&m, R::TextMuted, ink.mix(bar, 0.42 * soft), ink, &|_| true);
    m.insert(R::TextMuted, muted);
    // Muted text has to clear the pressed fills, which can make it the stronger of the two: keep
    // the order (text, secondary, muted) by strengthening the secondary line.
    let mut secondary = secondary;
    while secondary.contrast_ratio(bar) < muted.contrast_ratio(bar) + 0.3 && secondary != ink {
        secondary = secondary.mix(ink, 0.05);
    }
    m.insert(R::TextSecondary, secondary);
    let placeholder = tune(
        &m,
        R::Placeholder,
        ink.mix(field, 0.48 * soft),
        ink,
        &|_| true,
    );
    m.insert(R::Placeholder, placeholder);
    let button_label = tune(&m, R::ButtonLabel, ink.mix(bar, 0.22 * soft), ink, &|_| {
        true
    });
    m.insert(R::ButtonLabel, button_label);
    let tab_title = tune(
        &m,
        R::TabTitleInactive,
        ink.mix(bar, 0.30 * soft),
        ink,
        &|_| true,
    );
    m.insert(R::TabTitleInactive, tab_title);
    let shortcut = tune(
        &m,
        R::ShortcutText,
        ink.mix(popup, 0.42 * soft),
        ink,
        &|_| true,
    );
    m.insert(R::ShortcutText, shortcut);
    let status = tune(&m, R::StatusText, ink.mix(popup, 0.12), ink, &|_| true);
    m.insert(R::StatusText, status);
    let close = tune(
        &m,
        R::TabCloseLabel,
        ink.mix(frame, 0.45 * soft),
        ink,
        &|_| true,
    );
    m.insert(R::TabCloseLabel, close);
    m
}

/// Every pair of RULES that fails, as text.
fn violations(map: &Map) -> Vec<String> {
    let mut out = Vec::new();
    for rule in RULES {
        for &background in rule.backgrounds {
            let (fg, bg) = (map[&rule.foreground], map[&background]);
            let ratio = fg.over(bg).contrast_ratio(bg);
            if ratio < rule.minimum.ratio() {
                out.push(format!(
                    "{} {} on {} {}: {ratio:.2} (needs {:.1})",
                    rule.foreground.slint_name(),
                    fg.to_hex(),
                    background.slint_name(),
                    bg.to_hex(),
                    rule.minimum.ratio()
                ));
            }
        }
    }
    out
}

fn literal(colour: Colour) -> String {
    if colour.a == 255 {
        format!(
            "Colour::hex(0x{:02x}{:02x}{:02x})",
            colour.r, colour.g, colour.b
        )
    } else {
        format!(
            "Colour::hex_alpha(0x{:02x}{:02x}{:02x}{:02x})",
            colour.r, colour.g, colour.b, colour.a
        )
    }
}

fn variant_name(role: Role) -> String {
    format!("{role:?}")
}

fn table(out: &mut String, name: &str, doc: &str, map: &Map) {
    let _ = writeln!(out, "/// {doc}");
    let _ = writeln!(out, "const fn {name}(role: Role) -> Colour {{");
    let _ = writeln!(out, "    match role {{");
    for role in Role::ALL {
        let _ = writeln!(
            out,
            "        Role::{} => {},",
            variant_name(role),
            literal(map[&role])
        );
    }
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");
}

fn module(candidate: &Candidate, light: &Map, dark: &Map) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0"
    );
    let _ = writeln!(
        out,
        "// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors"
    );
    let _ = writeln!(out, "//! `{}`: {}", candidate.id, candidate.mood);
    let _ = writeln!(out, "//!");
    let _ = writeln!(
        out,
        "//! Generated by `examples/palette_gen.rs` from a few design tokens; the tables below are plain"
    );
    let _ = writeln!(
        out,
        "//! literals. Accent fill: light `{}`, dark `{}`.",
        light[&Role::PrimaryButtonBackground].to_hex(),
        dark[&Role::PrimaryButtonBackground].to_hex()
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "use crate::colour::Colour;");
    let _ = writeln!(out, "use crate::palette::{{Palette, Role}};");
    let _ = writeln!(out);
    let _ = writeln!(out, "/// The palette's id.");
    let _ = writeln!(out, "pub const ID: &str = \"{}\";", candidate.id);
    let _ = writeln!(out);
    let _ = writeln!(out, "/// {}.", candidate.name);
    let _ = writeln!(out, "pub const PALETTE: Palette = Palette {{");
    let _ = writeln!(out, "    id: ID,");
    let _ = writeln!(out, "    name: \"{}\",", candidate.name);
    let _ = writeln!(
        out,
        "    selectable: {},",
        SELECTABLE.contains(&candidate.id)
    );
    let _ = writeln!(out, "    light,");
    let _ = writeln!(out, "    dark,");
    let _ = writeln!(out, "}};");
    let _ = writeln!(out);
    table(&mut out, "light", "The light scheme.", light);
    let _ = writeln!(out);
    table(&mut out, "dark", "The dark scheme.", dark);
    out
}

fn main() {
    let root: PathBuf = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("src/palettes");
    std::fs::create_dir_all(&root).expect("the directory can be made");
    let all = candidates();
    let mut failed = false;

    for candidate in &all {
        let light = build(&candidate.light);
        let dark = build(&candidate.dark);
        for (name, map) in [("light", &light), ("dark", &dark)] {
            assert_eq!(
                map.len(),
                Role::COUNT,
                "{} {name}: a role is missing",
                candidate.id
            );
            for failure in violations(map) {
                failed = true;
                println!("FAIL {} {name}: {failure}", candidate.id);
            }
        }
        std::fs::write(
            root.join(format!("{}.rs", candidate.id)),
            module(candidate, &light, &dark),
        )
        .expect("the module can be written");
        println!(
            "{:9} light fill {} accent {} | dark fill {} accent {}",
            candidate.id,
            light[&Role::PrimaryButtonBackground].to_hex(),
            light[&Role::Accent].to_hex(),
            dark[&Role::PrimaryButtonBackground].to_hex(),
            dark[&Role::Accent].to_hex(),
        );
    }
    let _ = EXEMPT;
    if failed {
        std::process::exit(1);
    }
}
