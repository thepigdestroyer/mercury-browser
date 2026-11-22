// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! How Mercury Browser looks, as data.
//!
//! A library with no engine, no Slint and no I/O. The interface receives only resolved values
//! (colours, lengths, flags and models); nothing here, and nothing a theme or extension could
//! supply, is Slint syntax or a Slint type (Slint's royalty-free licence does not permit
//! distributing an application that exposes its APIs; see `SPEC.md` §2).
//!
//! - [`colour`]: hex forms, compositing, mixing, WCAG contrast.
//! - [`oklab`]: perceptual colour distance, for telling tab-group colours apart.
//! - [`palette`] and [`palettes`]: a palette is a table of the chrome's colour roles for the
//!   light and the dark scheme. `mercury-b2` is the default look; `hermes` is today's values,
//!   kept only as a regression baseline.
//! - [`contrast`]: the rules that every palette that can be chosen must pass.
//! - [`group_colour`]: the 14 tab-group colours, custom colours, and tab shades.
//! - [`keymap`]: the one-letter ⌥ group keys and their validation.
//! - [`metrics`]: the sizes of the chrome's rows, buttons and tabs, as a preset.
//! - [`resolve`](mod@resolve): turns the stored [`mercury_settings::AppearanceSettings`] into what the
//!   interface needs, with notes for anything it had to fall back from.
//! - [`panels`]: what the settings window's Appearance and Tab groups sections show and do.

#![deny(missing_docs)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod colour;
pub mod contrast;
pub mod group_colour;
pub mod keymap;
pub mod metrics;
pub mod oklab;
pub mod palette;
pub mod palettes;
// settings-ui: what the Appearance and Tab groups sections of the settings window show and do.
pub mod panels;
pub mod resolve;

pub use colour::Colour;
pub use contrast::{EXEMPT, Measurement, Minimum, RULES, Rule, Violation, check, measure};
pub use group_colour::{GroupColour, PRESETS, Preset, Swatch, auto_colour, tab_shade};
pub use keymap::{
    Assign, DeadKey, DeadKeyTable, KeyIssue, KeyMap, LetterKey, OptionChar, dead_key_note,
    parse_sequence,
};
pub use metrics::Metrics;
pub use oklab::{Oklab, delta_e};
pub use palette::{Palette, Role, SUN_INK, SUN_STOPS, Scheme};
pub use palettes::default_palette;
pub use resolve::{KeyPolicy, Layout, Resolved, TabGroupPolicy, resolve};
