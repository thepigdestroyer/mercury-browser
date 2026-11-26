// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The macOS content-process sandbox (R14 EC1).
//!
//! * [`content_profile`] builds the Seatbelt profile (`profiles/content.sb`, translated from
//!   Chromium's renderer profile) and its parameters, from paths the browser resolved.
//! * [`bundle_root`] and [`run_root_from_env`] find the application and the private run
//!   directory from what the process knows.

mod env;
mod layout;
mod profile;

pub use env::{content_environment, content_process_command};
pub use layout::{bundle_root, run_root_from_env};
pub use profile::{
    BUNDLE_IDENTIFIER, CONTENT_PROFILE, Profile, ProfileError, ProfileInputs, content_profile,
    report_profile,
};
