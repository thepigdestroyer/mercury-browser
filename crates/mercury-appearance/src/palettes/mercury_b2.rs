// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `mercury-b2`: Mercury's own look as designed on 18 Nov 2026 (SPEC §1). **Not selectable and not
//! the default** since 22 Nov 2026: the owner rejected it and chose Signal, Forest, Abyss and
//! Glacier instead. Kept as data (its tests and the `Sun` brand colours stay).
//!
//! Light is warm paper with plum-black ink and the brand's orange; dark is the "night plate",
//! a deep violet with the Sun's amber. Where the owner gave a value it is used as given:
//!
//! | | Light | Dark |
//! | --- | --- | --- |
//! | chrome | `#F8F4EE` (toolbar), `#EFE8DF` (window frame, tab strip) | `#1C1534` (toolbar), `#140F27` (window frame, tab strip) |
//! | text | `#1A1230` | `#F3EEF8` |
//! | accent fill | `#C2491A` | `#FFB547` |
//!
//! The rest are read from the mockups in `design/ui-final/{ring,bar,system}/` (light panel
//! `#F1ECE5`, hairlines `#E2DACF` and `#DDD3C6`, secondary text `#5B5168`; dark panels `#1E1838`
//! and `#2A2347`, hairline `#382E58`, secondary text `#B4AAC6`, page `#110C21`) or derived from
//! them. The mockups' muted greys (`#8D8499`, `#7F7596`) measure only 2.9:1 to 4.1:1 as text, so
//! `text-muted`, `placeholder` and `shortcut-text` are the same hue family, darkened (light) or
//! lightened (dark) until they reach 4.5:1 on every surface they are drawn on.
//!
//! **The accent comes in two forms.** `#C2491A` on `#F8F4EE` measures 4.497:1 (4.05:1 on
//! `#EFE8DF`), just under the 4.5:1 that text needs, so the owner's colour is the *fill*
//! accent: the primary button, the selected row (white text on it is 4.93:1) and every mark
//! (marks need 3:1). The roles that are drawn as text on the chrome (`accent`,
//! `button-label-hover`, `match-highlight`) are the same hue, deepened. The accent is also drawn
//! as text on the hover and pressed fills of buttons and tabs (the pop-up bar's Allow label, the
//! bookmark star, the download arrow), the palest of which, `#E0D7CA`, would leave `#B24318` at
//! 3.97:1; so the light accent is `#A03C14`, 4.68:1 or more on every surface it is drawn on, and
//! `button-label-hover` is a step deeper (`#8A320F`) so that a hovered link or button label still
//! shows. In dark, `#FFB547` is readable as text and as a fill.
//!
//! Every pair is checked in both schemes by [`crate::contrast`]; the tables below hold literals,
//! nothing is computed at run time.

use crate::colour::Colour;
use crate::palette::{Palette, Role};

/// The palette's id, as stored in `appearance.palette_light` and `appearance.palette_dark`.
pub const ID: &str = "mercury-b2";

/// Mercury B2.
pub const PALETTE: Palette = Palette {
    id: ID,
    name: "Mercury",
    selectable: false,
    light,
    dark,
};

/// The light scheme.
const fn light(role: Role) -> Colour {
    match role {
        // Surfaces and hairlines.
        Role::WindowBackground => Colour::hex(0xefe8df),
        Role::ToolbarBackground => Colour::hex(0xf8f4ee),
        Role::ToolbarDivider => Colour::hex(0xddd3c6),
        Role::ContentBackground => Colour::hex(0xf8f4ee),
        Role::FieldBackground => Colour::hex(0xffffff),
        Role::FieldBorder => Colour::hex(0xd6cbbc),
        Role::SearchFieldBackground => Colour::hex(0xffffff),
        Role::SearchFieldBorder => Colour::hex(0xddd3c6),
        // Text.
        Role::Text => Colour::hex(0x1a1230),
        Role::TextSecondary => Colour::hex(0x5b5168),
        Role::TextMuted => Colour::hex(0x635970),
        Role::Placeholder => Colour::hex(0x736982),
        // Accent text and buttons.
        Role::Accent => Colour::hex(0xa03c14),
        Role::ButtonLabel => Colour::hex(0x4b4160),
        Role::ButtonLabelHover => Colour::hex(0x8a320f),
        Role::ButtonHover => Colour::hex(0xe7dfd3),
        Role::ButtonPressed => Colour::hex(0xe0d7ca),
        // Tabs.
        Role::TabActive => Colour::hex(0xffffff),
        Role::TabHover => Colour::hex(0xe6ddd1),
        Role::TabTitleInactive => Colour::hex(0x5b5168),
        Role::TabCloseHover => Colour::hex(0xddd3c6),
        Role::TabClosePressed => Colour::hex(0xd0c5b5),
        Role::TabCloseLabel => Colour::hex(0x6b6178),
        // Status bubble.
        Role::StatusBackground => Colour::hex(0xffffff),
        Role::StatusBorder => Colour::hex(0xddd3c6),
        Role::StatusText => Colour::hex(0x2e2540),
        // Notice and information bars.
        Role::NoticeBackground => Colour::hex(0xfbefcb),
        Role::NoticeBorder => Colour::hex(0xe3c87e),
        Role::InfoBackground => Colour::hex(0xf1eaf7),
        Role::InfoBorder => Colour::hex(0xd3c6e5),
        Role::InfoText => Colour::hex(0x1a1230),
        // Popups and menus.
        Role::PopupBackground => Colour::hex(0xfcfaf6),
        Role::PopupBorder => Colour::hex(0xd6cbbc),
        Role::ItemHover => Colour::hex(0xefe8df),
        Role::ItemSelected => Colour::hex(0xc2491a),
        Role::ItemSelectedText => Colour::hex(0xffffff),
        Role::Separator => Colour::hex(0xddd3c6),
        Role::ShortcutText => Colour::hex(0x6b6178),
        Role::DisabledText => Colour::hex(0xa59db0),
        // Dialog sheets, primary buttons, errors and focus.
        Role::Scrim => Colour::hex_alpha(0x0000004d),
        Role::SheetBackground => Colour::hex(0xfcfaf6),
        Role::Shadow => Colour::hex_alpha(0x00000040),
        Role::PrimaryButtonBackground => Colour::hex(0xc2491a),
        Role::PrimaryButtonHover => Colour::hex(0xae4016),
        Role::PrimaryButtonPressed => Colour::hex(0x97370f),
        Role::PrimaryButtonText => Colour::hex(0xffffff),
        Role::Danger => Colour::hex(0xb3261e),
        Role::FocusRing => Colour::hex(0x9e390f),
        Role::MatchHighlight => Colour::hex(0xa23c14),
        // Page popups.
        Role::PagePopupBackground => Colour::hex(0xfdfdfd),
        Role::PagePopupBorder => Colour::hex(0xb9b9be),
        Role::PagePopupCaption => Colour::hex(0x6e6e73),
        // Media capture.
        Role::CaptureIndicator => Colour::hex(0xd93025),
        Role::CaptureIndicatorGlyph => Colour::hex(0xffffff),
    }
}

/// The dark scheme.
const fn dark(role: Role) -> Colour {
    match role {
        // Surfaces and hairlines.
        Role::WindowBackground => Colour::hex(0x140f27),
        Role::ToolbarBackground => Colour::hex(0x1c1534),
        Role::ToolbarDivider => Colour::hex(0x382e58),
        Role::ContentBackground => Colour::hex(0x110c21),
        Role::FieldBackground => Colour::hex(0x2a2347),
        Role::FieldBorder => Colour::hex(0x413769),
        Role::SearchFieldBackground => Colour::hex(0x241d40),
        Role::SearchFieldBorder => Colour::hex(0x382e58),
        // Text.
        Role::Text => Colour::hex(0xf3eef8),
        Role::TextSecondary => Colour::hex(0xb4aac6),
        Role::TextMuted => Colour::hex(0xa198b7),
        Role::Placeholder => Colour::hex(0x9a90b0),
        // Accent text and buttons.
        Role::Accent => Colour::hex(0xffb547),
        Role::ButtonLabel => Colour::hex(0xbdb3d0),
        Role::ButtonLabelHover => Colour::hex(0xffc46b),
        Role::ButtonHover => Colour::hex(0x2a2347),
        Role::ButtonPressed => Colour::hex(0x352c58),
        // Tabs.
        Role::TabActive => Colour::hex(0x2a2347),
        Role::TabHover => Colour::hex(0x211a3c),
        Role::TabTitleInactive => Colour::hex(0xa99fbd),
        Role::TabCloseHover => Colour::hex(0x352c58),
        Role::TabClosePressed => Colour::hex(0x42386a),
        Role::TabCloseLabel => Colour::hex(0x9e94b4),
        // Status bubble.
        Role::StatusBackground => Colour::hex(0x2a2347),
        Role::StatusBorder => Colour::hex(0x413769),
        Role::StatusText => Colour::hex(0xddd6ea),
        // Notice and information bars.
        Role::NoticeBackground => Colour::hex(0x33260f),
        Role::NoticeBorder => Colour::hex(0x6b531f),
        Role::InfoBackground => Colour::hex(0x271f4a),
        Role::InfoBorder => Colour::hex(0x43376e),
        Role::InfoText => Colour::hex(0xf3eef8),
        // Popups and menus.
        Role::PopupBackground => Colour::hex(0x241d42),
        Role::PopupBorder => Colour::hex(0x3e3363),
        Role::ItemHover => Colour::hex(0x2e2652),
        Role::ItemSelected => Colour::hex(0xf59a3c),
        Role::ItemSelectedText => Colour::hex(0x1e1233),
        Role::Separator => Colour::hex(0x382e58),
        Role::ShortcutText => Colour::hex(0x9e94b4),
        Role::DisabledText => Colour::hex(0x6b6185),
        // Dialog sheets, primary buttons, errors and focus.
        Role::Scrim => Colour::hex_alpha(0x00000099),
        Role::SheetBackground => Colour::hex(0x211a3d),
        Role::Shadow => Colour::hex_alpha(0x00000080),
        Role::PrimaryButtonBackground => Colour::hex(0xffb547),
        Role::PrimaryButtonHover => Colour::hex(0xffc46b),
        Role::PrimaryButtonPressed => Colour::hex(0xf59e2f),
        Role::PrimaryButtonText => Colour::hex(0x1e1233),
        Role::Danger => Colour::hex(0xff9e94),
        Role::FocusRing => Colour::hex(0xffd08a),
        Role::MatchHighlight => Colour::hex(0xffc46b),
        // Page popups.
        Role::PagePopupBackground => Colour::hex(0x2b2b2e),
        Role::PagePopupBorder => Colour::hex(0x4b4b50),
        Role::PagePopupCaption => Colour::hex(0xa2a2a8),
        // Media capture.
        Role::CaptureIndicator => Colour::hex(0xf2544c),
        Role::CaptureIndicatorGlyph => Colour::hex(0xffffff),
    }
}
