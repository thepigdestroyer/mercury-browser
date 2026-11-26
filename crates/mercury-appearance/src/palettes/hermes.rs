// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `hermes`: the chrome's colours as they were before Mercury B2, **kept only as a regression
//! baseline**.
//!
//! Every value here is the literal in `crates/mercury-ui/ui/theme/chrome-palette.slint`
//! (`tests/hermes_matches_chrome_palette.rs` fails if the two drift apart). The owner dropped
//! the Hermes look (decision O-02), so this palette is `selectable: false`: it is never a default,
//! never offered in a picker, and `signal` is what every profile gets. It exists because the
//! theme-plumbing lane's pixel-diff gate ("Hermes palette with Roomy metrics renders what the
//! browser renders today", R12 stage 2) needs today's colours as data to prove that moving the
//! chrome onto palettes changed nothing.
//!
//! **Delete this module (and `tests/hermes_matches_chrome_palette.rs`) in the layout-switch lane**,
//! once that gate has passed. It is not held to the contrast rules: its `placeholder`, for one,
//! is 3.6:1 on a white field.

use crate::colour::Colour;
use crate::palette::{Palette, Role};

/// The palette's id.
pub const ID: &str = "hermes";

/// Hermes, as the chrome drew it before Mercury B2.
pub const PALETTE: Palette = Palette {
    id: ID,
    name: "Hermes (legacy)",
    selectable: false,
    light,
    dark,
};

/// The light scheme: the `: #…` side of each `root.dark ? … : …`.
const fn light(role: Role) -> Colour {
    match role {
        // Surfaces and hairlines.
        Role::WindowBackground => Colour::hex(0xe9edf2),
        Role::ToolbarBackground => Colour::hex(0xf7f8fa),
        Role::ToolbarDivider => Colour::hex(0xd6dce3),
        Role::ContentBackground => Colour::hex(0xf4f6f8),
        Role::FieldBackground => Colour::hex(0xffffff),
        Role::FieldBorder => Colour::hex(0xc5ccd5),
        Role::SearchFieldBackground => Colour::hex(0xffffff),
        Role::SearchFieldBorder => Colour::hex(0xc5ccd5),
        // Text.
        Role::Text => Colour::hex(0x141a21),
        Role::TextSecondary => Colour::hex(0x5a6573),
        Role::TextMuted => Colour::hex(0x6a7480),
        Role::Placeholder => Colour::hex(0x7d8794),
        // Accent text and buttons.
        Role::Accent => Colour::hex(0xb4410c),
        Role::ButtonLabel => Colour::hex(0x4d5866),
        Role::ButtonLabelHover => Colour::hex(0x9e390a),
        Role::ButtonHover => Colour::hex(0xe2e7ed),
        Role::ButtonPressed => Colour::hex(0xd3dae2),
        // Tabs.
        Role::TabActive => Colour::hex(0xffffff),
        Role::TabHover => Colour::hex(0xdde3ea),
        Role::TabTitleInactive => Colour::hex(0x5a6573),
        Role::TabCloseHover => Colour::hex(0xd8dee6),
        Role::TabClosePressed => Colour::hex(0xc8d0da),
        Role::TabCloseLabel => Colour::hex(0x6a7480),
        // Status bubble.
        Role::StatusBackground => Colour::hex(0xffffff),
        Role::StatusBorder => Colour::hex(0xc5ccd5),
        Role::StatusText => Colour::hex(0x2d3540),
        // Notice and information bars.
        Role::NoticeBackground => Colour::hex(0xfff4d6),
        Role::NoticeBorder => Colour::hex(0xe8cf87),
        Role::InfoBackground => Colour::hex(0xf6f1fb),
        Role::InfoBorder => Colour::hex(0xcfc3e6),
        Role::InfoText => Colour::hex(0x141a21),
        // Popups and menus.
        Role::PopupBackground => Colour::hex(0xffffff),
        Role::PopupBorder => Colour::hex(0xc5ccd5),
        Role::ItemHover => Colour::hex(0xe8edf3),
        Role::ItemSelected => Colour::hex(0xb4410c),
        Role::ItemSelectedText => Colour::hex(0xffffff),
        Role::Separator => Colour::hex(0xd6dce3),
        Role::ShortcutText => Colour::hex(0x6a7480),
        Role::DisabledText => Colour::hex(0x9aa3ad),
        // Dialog sheets, primary buttons, errors and focus.
        Role::Scrim => Colour::hex_alpha(0x0000004d),
        Role::SheetBackground => Colour::hex(0xffffff),
        Role::Shadow => Colour::hex_alpha(0x00000040),
        Role::PrimaryButtonBackground => Colour::hex(0xb4410c),
        Role::PrimaryButtonHover => Colour::hex(0x9e390a),
        Role::PrimaryButtonPressed => Colour::hex(0x873009),
        Role::PrimaryButtonText => Colour::hex(0xffffff),
        Role::Danger => Colour::hex(0xb3261e),
        Role::FocusRing => Colour::hex(0x9e390a),
        Role::MatchHighlight => Colour::hex(0x9e390a),
        // Page popups.
        Role::PagePopupBackground => Colour::hex(0xfdfdfd),
        Role::PagePopupBorder => Colour::hex(0xb9b9be),
        Role::PagePopupCaption => Colour::hex(0x6e6e73),
        // Media capture.
        Role::CaptureIndicator => Colour::hex(0xd93025),
        Role::CaptureIndicatorGlyph => Colour::hex(0xffffff),
        Role::PinnedBeadLetter => Colour::hex(0xffffff),
    }
}

/// The dark scheme: the `? #… :` side of each `root.dark ? … : …`.
const fn dark(role: Role) -> Colour {
    match role {
        // Surfaces and hairlines.
        Role::WindowBackground => Colour::hex(0x0b0f14),
        Role::ToolbarBackground => Colour::hex(0x11171f),
        Role::ToolbarDivider => Colour::hex(0x202832),
        Role::ContentBackground => Colour::hex(0x0c1117),
        Role::FieldBackground => Colour::hex(0x1a222c),
        Role::FieldBorder => Colour::hex(0x29323d),
        Role::SearchFieldBackground => Colour::hex(0x171f29),
        Role::SearchFieldBorder => Colour::hex(0x29333e),
        // Text.
        Role::Text => Colour::hex(0xedf4fa),
        Role::TextSecondary => Colour::hex(0x8c99a9),
        Role::TextMuted => Colour::hex(0x727f8f),
        Role::Placeholder => Colour::hex(0x717e8e),
        // Accent text and buttons.
        Role::Accent => Colour::hex(0xffb547),
        Role::ButtonLabel => Colour::hex(0x9aa6b5),
        Role::ButtonLabelHover => Colour::hex(0xffc46b),
        Role::ButtonHover => Colour::hex(0x19222c),
        Role::ButtonPressed => Colour::hex(0x222d39),
        // Tabs.
        Role::TabActive => Colour::hex(0x161e28),
        Role::TabHover => Colour::hex(0x111820),
        Role::TabTitleInactive => Colour::hex(0x8b98a8),
        Role::TabCloseHover => Colour::hex(0x222c37),
        Role::TabClosePressed => Colour::hex(0x2b3744),
        Role::TabCloseLabel => Colour::hex(0x7f8c9c),
        // Status bubble.
        Role::StatusBackground => Colour::hex(0x1a222c),
        Role::StatusBorder => Colour::hex(0x29323d),
        Role::StatusText => Colour::hex(0xc9d3de),
        // Notice and information bars.
        Role::NoticeBackground => Colour::hex(0x2a2212),
        Role::NoticeBorder => Colour::hex(0x5c4a1f),
        Role::InfoBackground => Colour::hex(0x1e1830),
        Role::InfoBorder => Colour::hex(0x3d3160),
        Role::InfoText => Colour::hex(0xedf4fa),
        // Popups and menus.
        Role::PopupBackground => Colour::hex(0x161e28),
        Role::PopupBorder => Colour::hex(0x2b3744),
        Role::ItemHover => Colour::hex(0x1f2a36),
        Role::ItemSelected => Colour::hex(0xe8892e),
        Role::ItemSelectedText => Colour::hex(0x1a1233),
        Role::Separator => Colour::hex(0x29323d),
        Role::ShortcutText => Colour::hex(0x7f8c9c),
        Role::DisabledText => Colour::hex(0x5d6a7a),
        // Dialog sheets, primary buttons, errors and focus.
        Role::Scrim => Colour::hex_alpha(0x00000099),
        Role::SheetBackground => Colour::hex(0x141b24),
        Role::Shadow => Colour::hex_alpha(0x00000080),
        Role::PrimaryButtonBackground => Colour::hex(0xffb547),
        Role::PrimaryButtonHover => Colour::hex(0xffc46b),
        Role::PrimaryButtonPressed => Colour::hex(0xf59e2f),
        Role::PrimaryButtonText => Colour::hex(0x1a1233),
        Role::Danger => Colour::hex(0xff9e94),
        Role::FocusRing => Colour::hex(0xffd08a),
        Role::MatchHighlight => Colour::hex(0xffc46b),
        // Page popups.
        Role::PagePopupBackground => Colour::hex(0x2b2b2e),
        Role::PagePopupBorder => Colour::hex(0x4b4b50),
        Role::PagePopupCaption => Colour::hex(0xa2a2a8),
        // Media capture.
        Role::CaptureIndicator => Colour::hex(0xff5f57),
        Role::CaptureIndicatorGlyph => Colour::hex(0xffffff),
        Role::PinnedBeadLetter => Colour::hex(0xffffff),
    }
}
