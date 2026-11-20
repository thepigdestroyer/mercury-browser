// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Pushes a palette's colours into `ChromePalette` (the colour-layer item of the theme-plumbing
//! lane replaces this stub).

use mercury_appearance::Palette;
use slint::{ComponentHandle, Global};

use crate::ChromePalette;

/// Sets `ChromePalette`'s light table from `light` and its dark table from `dark`.
pub(super) fn push<C>(component: &C, light: &Palette, dark: &Palette)
where
    C: ComponentHandle,
    for<'a> ChromePalette<'a>: Global<'a, C>,
{
    let _ = (component, light, dark);
}
