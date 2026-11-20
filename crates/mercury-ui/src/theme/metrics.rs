// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Pushes the sizes preset into `ChromeMetrics` (the metrics-layer item of the theme-plumbing
//! lane replaces this stub).

use mercury_appearance::Metrics;
use slint::{ComponentHandle, Global};

use crate::ChromeMetrics;

/// Sets every property of `ChromeMetrics` from `metrics`.
pub(super) fn push<C>(component: &C, metrics: &Metrics)
where
    C: ComponentHandle,
    for<'a> ChromeMetrics<'a>: Global<'a, C>,
{
    let _ = (component, metrics);
}
