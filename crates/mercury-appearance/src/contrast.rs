// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The contrast rules every palette a user can pick must pass, in both schemes.
//!
//! The rules are WCAG 2.2 success criteria 1.4.3 (text: 4.5:1) and 1.4.11 (the parts of an
//! interface and graphics: 3:1), written as data. A [`Rule`] says that one role is drawn on a
//! list of other roles and what ratio it needs there; [`RULES`] is every such pair the browser's
//! interface draws. [`check`] measures a palette against them and returns what fails. The
//! tests run it for every [`selectable`](crate::palettes::selectable) palette, so a palette that
//! cannot be read does not ship, and the completeness guard in `tests/contrast_rules.rs` fails
//! when a new [`Role`] is neither in a rule nor in [`EXEMPT`], so a role cannot be forgotten.
//!
//! The pairs of `crates/mercury-ui/tests/palette_contrast.rs`, which draws the real windows, are
//! all here; that test stays as the check that the windows really use those roles.

use std::fmt;

use crate::colour::Colour;
use crate::palette::{Palette, Role, Scheme};

use Role as R;

/// How much contrast a pair needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Minimum {
    /// Text: 4.5:1 (WCAG 2.2 success criterion 1.4.3, level AA).
    Text,
    /// A part of the interface or a graphic, such as a focus ring, a mark or a button's shape:
    /// 3:1 (success criterion 1.4.11).
    Ui,
}

impl Minimum {
    /// The smallest contrast ratio that passes.
    pub const fn ratio(self) -> f64 {
        match self {
            Self::Text => 4.5,
            Self::Ui => 3.0,
        }
    }
}

/// One role drawn on a list of other roles.
#[derive(Clone, Copy, Debug)]
pub struct Rule {
    /// The role that is drawn (the text, the mark, the outline).
    pub foreground: Role,
    /// The roles it is drawn on. Never empty.
    pub backgrounds: &'static [Role],
    /// What it needs on each of them.
    pub minimum: Minimum,
    /// Where the pair occurs, for the message that names a failure.
    pub why: &'static str,
}

/// A measured pair: the ratio, both colours, and what was required.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement {
    /// The role that is drawn.
    pub foreground: Role,
    /// The role it is drawn on.
    pub background: Role,
    /// The foreground's colour in the measured scheme.
    pub foreground_colour: Colour,
    /// The background's colour in the measured scheme.
    pub background_colour: Colour,
    /// The WCAG contrast ratio, 1.0 to 21.0. A translucent foreground is composited over the
    /// background first.
    pub ratio: f64,
    /// What the pair needs.
    pub minimum: Minimum,
    /// The [`Rule::why`] of the rule the pair came from.
    pub why: &'static str,
}

/// A [`Measurement`] below its minimum.
pub type Violation = Measurement;

impl Measurement {
    /// Whether the ratio reaches the minimum.
    pub fn passes(&self) -> bool {
        self.ratio >= self.minimum.ratio()
    }
}

impl fmt::Display for Measurement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} on {} {}: {:.2} (needs {:.1}; {})",
            self.foreground.slint_name(),
            self.foreground_colour.to_hex(),
            self.background.slint_name(),
            self.background_colour.to_hex(),
            self.ratio,
            self.minimum.ratio(),
            self.why,
        )
    }
}

/// The surfaces most things are drawn on (the seven the interface's contrast test has always
/// used), followed by any extra ones.
macro_rules! on_surfaces {
    ($($extra:expr),* $(,)?) => {
        &[
            R::WindowBackground,
            R::ToolbarBackground,
            R::ContentBackground,
            R::FieldBackground,
            R::PopupBackground,
            R::SheetBackground,
            R::TabActive
            $(, $extra)*
        ]
    };
}

const fn text(foreground: Role, backgrounds: &'static [Role], why: &'static str) -> Rule {
    Rule {
        foreground,
        backgrounds,
        minimum: Minimum::Text,
        why,
    }
}

const fn ui(foreground: Role, backgrounds: &'static [Role], why: &'static str) -> Rule {
    Rule {
        foreground,
        backgrounds,
        minimum: Minimum::Ui,
        why,
    }
}

/// Every pair of roles the interface draws, with what each needs.
///
/// A role that is not the foreground or a background of a rule must be in [`EXEMPT`].
pub const RULES: &[Rule] = &[
    // --- Text: 4.5:1. ---
    text(
        R::Text,
        on_surfaces![
            R::SearchFieldBackground,
            R::TabHover,
            R::ButtonHover,
            R::ButtonPressed,
            R::ItemHover,
            R::NoticeBackground,
            R::InfoBackground
        ],
        "ordinary text on every surface, the hover fills and the warning and information bars",
    ),
    text(
        R::TextSecondary,
        on_surfaces![R::SearchFieldBackground, R::ItemHover, R::NoticeBackground],
        "secondary lines (addresses, descriptions) on the surfaces and a hovered row",
    ),
    text(
        R::TextMuted,
        on_surfaces![R::SearchFieldBackground, R::ItemHover],
        "captions, counts and dates on the surfaces, the fields and a hovered row",
    ),
    text(
        R::Placeholder,
        &[R::FieldBackground, R::SearchFieldBackground],
        "the hint in an empty address, search or find field",
    ),
    text(
        R::Accent,
        on_surfaces![R::ItemHover],
        "links, checked boxes, the active-tab underline and the loading bar, on every surface",
    ),
    text(
        R::ButtonLabel,
        on_surfaces![R::ButtonHover, R::ButtonPressed],
        "a button's label at rest, and on its hover and pressed fills",
    ),
    text(
        R::ButtonLabelHover,
        on_surfaces![R::ButtonHover, R::ButtonPressed],
        "a hovered button's label, on the surfaces and on the hover and pressed fills",
    ),
    text(
        R::TabTitleInactive,
        &[R::WindowBackground, R::ToolbarBackground, R::TabHover],
        "the title of a tab that is not selected, in the strip and under the pointer",
    ),
    text(
        R::StatusText,
        &[R::StatusBackground],
        "the link bubble over the page",
    ),
    text(
        R::ShortcutText,
        &[R::PopupBackground, R::SheetBackground, R::ItemHover],
        "the shortcut beside a menu item, also when the item is hovered",
    ),
    text(
        R::InfoText,
        &[R::InfoBackground],
        "the text of an information bar",
    ),
    text(
        R::Danger,
        on_surfaces![R::ItemHover],
        "errors and destructive actions, on the surfaces and a hovered row",
    ),
    text(
        R::PrimaryButtonText,
        &[
            R::PrimaryButtonBackground,
            R::PrimaryButtonHover,
            R::PrimaryButtonPressed,
            R::Accent,
        ],
        "a primary button's label on its three fills, and on the accent colour",
    ),
    text(
        R::ItemSelectedText,
        &[R::ItemSelected],
        "everything drawn on the selected row of a list",
    ),
    text(
        R::PagePopupCaption,
        &[R::PagePopupBackground],
        "the line that names the page above a select list or colour picker",
    ),
    text(
        R::MatchHighlight,
        &[R::PopupBackground, R::FieldBackground, R::ItemHover],
        "the part of a suggestion that matches what was typed",
    ),
    // --- Interface parts and graphics: 3:1. ---
    ui(
        R::TabCloseLabel,
        &[
            R::WindowBackground,
            R::TabActive,
            R::TabHover,
            R::TabCloseHover,
            R::TabClosePressed,
        ],
        "a tab's close cross, a graphic, also on the fills behind it",
    ),
    ui(
        R::FocusRing,
        on_surfaces![R::SearchFieldBackground],
        "the outline of the control that has keyboard focus",
    ),
    ui(
        R::PrimaryButtonBackground,
        on_surfaces![],
        "the shape of a primary button, which has no outline",
    ),
    ui(
        R::PrimaryButtonHover,
        on_surfaces![],
        "the shape of a primary button under the pointer",
    ),
    ui(
        R::PrimaryButtonPressed,
        on_surfaces![],
        "the shape of a primary button while pressed",
    ),
    ui(
        R::ItemSelected,
        &[
            R::PopupBackground,
            R::ContentBackground,
            R::PagePopupBackground,
        ],
        "the fill of the selected row of a list, which must stand out from the list",
    ),
    ui(
        R::CaptureIndicator,
        &[
            R::WindowBackground,
            R::ToolbarBackground,
            R::FieldBackground,
            R::TabActive,
            R::TabHover,
        ],
        "the camera and microphone mark on a tab and beside the address",
    ),
    ui(
        R::CaptureIndicatorGlyph,
        &[R::CaptureIndicator],
        "the glyph drawn on the camera and microphone mark",
    ),
];

/// Roles that no rule holds to a ratio, each with the reason.
///
/// Every [`Role`] is either in a rule or here (`tests/contrast_rules.rs` checks), so a new role
/// forces a decision.
pub const EXEMPT: &[(Role, &str)] = &[
    (
        R::DisabledText,
        "WCAG 1.4.3 exempts text of inactive components: looking dimmed is how they show it",
    ),
    (
        R::ToolbarDivider,
        "a hairline between regions; the regions differ by fill and position as well",
    ),
    (
        R::FieldBorder,
        "a hairline; a field is also identified by its fill, its text and its place",
    ),
    (
        R::SearchFieldBorder,
        "a hairline; a field is also identified by its fill, its text and its place",
    ),
    (
        R::StatusBorder,
        "a hairline round a bubble that is also set off by its fill",
    ),
    (
        R::NoticeBorder,
        "a hairline round a bar that is also set off by its fill",
    ),
    (
        R::InfoBorder,
        "a hairline round a bar that is also set off by its fill",
    ),
    (
        R::PopupBorder,
        "a hairline round a popup that is also set off by its fill and its shadow",
    ),
    (
        R::PagePopupBorder,
        "a hairline round a page popup that is also set off by its fill and its shadow",
    ),
    (
        R::Separator,
        "a hairline between menu items; the items are separate controls with their own labels",
    ),
    (
        R::Scrim,
        "a translucent veil over the page, not drawn against; contrast is not defined for it",
    ),
    (
        R::Shadow,
        "a translucent shadow, decoration with no information in it",
    ),
];

/// Measures every pair of [`RULES`] in `palette`'s `scheme`, in rule order.
pub fn measure(palette: &Palette, scheme: Scheme) -> Vec<Measurement> {
    let table = palette.table(scheme);
    let mut measurements = Vec::new();
    for rule in RULES {
        let foreground_colour = table(rule.foreground);
        for &background in rule.backgrounds {
            let background_colour = table(background);
            let ratio = foreground_colour
                .over(background_colour)
                .contrast_ratio(background_colour);
            measurements.push(Measurement {
                foreground: rule.foreground,
                background,
                foreground_colour,
                background_colour,
                ratio,
                minimum: rule.minimum,
                why: rule.why,
            });
        }
    }
    measurements
}

/// The pairs of [`RULES`] that `palette` fails in `scheme`. Empty means the palette passes.
pub fn check(palette: &Palette, scheme: Scheme) -> Vec<Violation> {
    measure(palette, scheme)
        .into_iter()
        .filter(|measurement| !measurement.passes())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_minimums_are_the_wcag_ones() {
        assert!((Minimum::Text.ratio() - 4.5).abs() < f64::EPSILON);
        assert!((Minimum::Ui.ratio() - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_measurement_names_both_roles_and_the_ratio() {
        let measurement = Measurement {
            foreground: Role::Text,
            background: Role::WindowBackground,
            foreground_colour: Colour::hex(0x808080),
            background_colour: Colour::hex(0x909090),
            ratio: 1.2,
            minimum: Minimum::Text,
            why: "for the test",
        };
        assert!(!measurement.passes());
        assert_eq!(
            measurement.to_string(),
            "text #808080 on window-background #909090: 1.20 (needs 4.5; for the test)"
        );
    }
}
