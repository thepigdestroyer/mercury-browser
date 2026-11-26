// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The A11Y-2 feature switches. They stay here until the preferences for them exist (K1 and B1-5);
//! then the engine reads the preferences and these constants go.

/// Which A11Y-2 features are on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    pub text_runs: bool,
    pub live_regions: bool,
    pub aria_notify: bool,
}

// stub
pub const FEATURES: Features = Features {
    text_runs: true,
    live_regions: true,
    aria_notify: true,
};
