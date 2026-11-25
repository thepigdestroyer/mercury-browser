// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The ⌥ group keys: when a held ⌥ and a letter belongs to Mercury and when it belongs to the page.
//!
//! Hold ⌥ and tap a letter you gave a tab group, then a digit for one of its tabs, and Mercury
//! switches there (`design/ui-final/SPEC.md` §5.1, the lane brief `lane-key-switcher-brief.md`
//! §5). The rule exists so that typing ⌥ characters (å, ß, ∂) and accent dead keys keeps working:
//! a quick ⌥ and letter in a text field is the page's; only a hold of 0.4 s (the setting
//! `hold_ms`) opens the "Switch group" card.
//!
//! This crate is the state machine and nothing else. It has no winit, no Slint, no clock and no
//! I/O. Everything it needs is handed in:
//!
//! - **keys and modifiers** as [`Event`]s (the desktop's `option_keys.rs` translates winit's);
//! - **time** as a [`std::time::Duration`] since an origin the caller chooses;
//! - **where the focus is** as [`Focus`] (a text field, an IME composition: the signal of the
//!   `txt-1` lane arrives here as a plain bool);
//! - **the groups** as [`KeyedGroup`]s in ring order, and the settings as a [`Config`].
//!
//! What it answers is an [`Outcome`]: whether to swallow the key (`consume`), where to switch
//! (`activate`) and whether the card changed (`card`); [`OptionKeys::view`] has what the card, the
//! dock and the ring show.
//!
//! # Where things are
//!
//! | Module | Content |
//! | --- | --- |
//! | [`types`] | every public type, **frozen** (the lead's skeleton; change only through the integrator) |
//! | [`machine`] | [`OptionKeys`], the state machine of the brief's section 5.4 |

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod machine;
pub mod types;

pub use machine::OptionKeys;
pub use types::{
    CardChange, CardView, Config, DEFAULT_HOLD, Event, Focus, Key, KeyCapture, KeyedGroup,
    LetterKey, MAX_DIGIT, MAX_HOLD, MAX_KEYED_GROUPS, MIN_HOLD, Mods, Outcome, Phase, Target,
};
