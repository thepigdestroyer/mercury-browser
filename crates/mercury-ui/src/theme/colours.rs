// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Pushes a palette's colours into `ChromePalette`: one table of colours for each scheme.

use mercury_appearance::{Colour, Palette, Role, Scheme};
use slint::{Color, ComponentHandle, Global};

use crate::{ChromeColors, ChromePalette};

/// The colour as Slint holds it; the alpha is kept (`scrim` and `shadow` are translucent).
fn slint_colour(colour: Colour) -> Color {
    Color::from_argb_u8(colour.a, colour.r, colour.g, colour.b)
}

/// The colours of every role of `palette` in `scheme`. A struct literal with every field and no
/// default: a role added to `ChromeColors` without a line here does not compile.
fn table(palette: &Palette, scheme: Scheme) -> ChromeColors {
    let of = |role: Role| slint_colour(palette.colour(scheme, role));
    ChromeColors {
        window_background: of(Role::WindowBackground),
        toolbar_background: of(Role::ToolbarBackground),
        toolbar_divider: of(Role::ToolbarDivider),
        content_background: of(Role::ContentBackground),
        field_background: of(Role::FieldBackground),
        field_border: of(Role::FieldBorder),
        search_field_background: of(Role::SearchFieldBackground),
        search_field_border: of(Role::SearchFieldBorder),
        text: of(Role::Text),
        text_secondary: of(Role::TextSecondary),
        text_muted: of(Role::TextMuted),
        placeholder: of(Role::Placeholder),
        accent: of(Role::Accent),
        button_label: of(Role::ButtonLabel),
        button_label_hover: of(Role::ButtonLabelHover),
        button_hover: of(Role::ButtonHover),
        button_pressed: of(Role::ButtonPressed),
        tab_active: of(Role::TabActive),
        tab_hover: of(Role::TabHover),
        tab_title_inactive: of(Role::TabTitleInactive),
        tab_close_hover: of(Role::TabCloseHover),
        tab_close_pressed: of(Role::TabClosePressed),
        tab_close_label: of(Role::TabCloseLabel),
        status_background: of(Role::StatusBackground),
        status_border: of(Role::StatusBorder),
        status_text: of(Role::StatusText),
        notice_background: of(Role::NoticeBackground),
        notice_border: of(Role::NoticeBorder),
        info_background: of(Role::InfoBackground),
        info_border: of(Role::InfoBorder),
        info_text: of(Role::InfoText),
        popup_background: of(Role::PopupBackground),
        popup_border: of(Role::PopupBorder),
        item_hover: of(Role::ItemHover),
        item_selected: of(Role::ItemSelected),
        item_selected_text: of(Role::ItemSelectedText),
        separator: of(Role::Separator),
        shortcut_text: of(Role::ShortcutText),
        disabled_text: of(Role::DisabledText),
        scrim: of(Role::Scrim),
        sheet_background: of(Role::SheetBackground),
        shadow: of(Role::Shadow),
        primary_button_background: of(Role::PrimaryButtonBackground),
        primary_button_hover: of(Role::PrimaryButtonHover),
        primary_button_pressed: of(Role::PrimaryButtonPressed),
        primary_button_text: of(Role::PrimaryButtonText),
        danger: of(Role::Danger),
        focus_ring: of(Role::FocusRing),
        match_highlight: of(Role::MatchHighlight),
        page_popup_background: of(Role::PagePopupBackground),
        page_popup_border: of(Role::PagePopupBorder),
        page_popup_caption: of(Role::PagePopupCaption),
        capture_indicator: of(Role::CaptureIndicator),
        capture_indicator_glyph: of(Role::CaptureIndicatorGlyph),
        pinned_bead_letter: of(Role::PinnedBeadLetter),
    }
}

/// Sets `ChromePalette`'s light table from `light` and its dark table from `dark`.
///
/// The light table is the light half of `light` and the dark table the dark half of `dark`
/// (`appearance.palette_light` and `palette_dark` may name different palettes). The scheme in
/// use stays `ChromePalette.dark`'s business, so a change of the system's scheme needs no push.
pub(super) fn push<C>(component: &C, light: &Palette, dark: &Palette)
where
    C: ComponentHandle,
    for<'a> ChromePalette<'a>: Global<'a, C>,
{
    let palette = component.global::<ChromePalette>();
    palette.set_light_colours(table(light, Scheme::Light));
    palette.set_dark_colours(table(dark, Scheme::Dark));
}
