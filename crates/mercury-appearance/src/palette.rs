// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Colour roles, schemes and the shape of a palette.
//!
//! A [`Role`] is one colour of the browser's own interface ("the toolbar's background", "the
//! text in a menu"). A [`Palette`] says what colour every role has in the light and the dark
//! [`Scheme`]. The roles are exactly the properties of `ChromePalette` in
//! `crates/mercury-ui/ui/theme/chrome-palette.slint` and keep its kebab-case names
//! ([`Role::slint_name`]), because those names are the contract with the interface code: it asks
//! for `"window-background"` and receives a colour, and never sees Slint syntax in a palette.
//!
//! The lookup is **total**: a palette is two functions, each an exhaustive `match` over [`Role`]
//! returning a literal, so a palette that forgets a role does not compile, and nothing is parsed
//! or computed when Mercury starts. The tables are in [`crate::palettes`]; the rules that every
//! palette a user can pick must pass are in [`crate::contrast`].

use crate::colour::Colour;

/// Which half of a palette is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scheme {
    /// The light half.
    Light,
    /// The dark half.
    Dark,
}

impl Scheme {
    /// Both schemes, light first.
    pub const ALL: [Self; 2] = [Self::Light, Self::Dark];

    /// `true` for [`Scheme::Dark`], the `root.dark` of `ChromePalette`.
    pub const fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }

    /// `"light"` or `"dark"`, for messages.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

/// Declares [`Role`] and everything derived from its list, so that the enum, [`Role::ALL`] and
/// [`Role::slint_name`] cannot disagree: a role added here is in all three.
macro_rules! roles {
    ($( $(#[$meta:meta])* $variant:ident => $name:literal, )+) => {
        /// One colour of the browser's own interface; see the [module documentation](self).
        ///
        /// There is one variant per property of `ChromePalette` (54 today), in the order of that
        /// file. [`Role::slint_name`] is the property's exact name.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Role {
            $( $(#[$meta])* $variant, )+
        }

        impl Role {
            /// How many roles there are.
            pub const COUNT: usize = [$(Self::$variant),+].len();

            /// Every role, in the order of `chrome-palette.slint`.
            pub const ALL: [Self; Self::COUNT] = [$(Self::$variant),+];

            /// The name of the `ChromePalette` property for this role, for example
            /// `"window-background"`.
            pub const fn slint_name(self) -> &'static str {
                match self {
                    $( Self::$variant => $name, )+
                }
            }
        }
    };
}

roles! {
    // Surfaces and hairlines.
    /// The window frame and the tab strip: the darker of the two chrome surfaces.
    WindowBackground => "window-background",
    /// The toolbar: the lighter chrome surface.
    ToolbarBackground => "toolbar-background",
    /// The line between the toolbar and the page.
    ToolbarDivider => "toolbar-divider",
    /// Full-window browser pages (bookmarks, history, settings) behind their rows.
    ContentBackground => "content-background",
    /// The address field and other text fields.
    FieldBackground => "field-background",
    /// The outline of a text field.
    FieldBorder => "field-border",
    /// The find-in-page and search fields.
    SearchFieldBackground => "search-field-background",
    /// The outline of a search field.
    SearchFieldBorder => "search-field-border",

    // Text.
    /// Ordinary text.
    Text => "text",
    /// Secondary lines: addresses under titles, descriptions.
    TextSecondary => "text-secondary",
    /// The quietest text that is still meant to be read: captions, counts, dates.
    TextMuted => "text-muted",
    /// The hint inside an empty field.
    Placeholder => "placeholder",

    // Accent text and buttons.
    /// The accent where it is drawn as text or a mark on the chrome: links, checked boxes, the
    /// active-tab underline, the loading bar. (Fills use [`Role::PrimaryButtonBackground`].)
    Accent => "accent",
    /// The label of a toolbar or sheet button.
    ButtonLabel => "button-label",
    /// The label of a button the pointer is over.
    ButtonLabelHover => "button-label-hover",
    /// The fill of a button the pointer is over.
    ButtonHover => "button-hover",
    /// The fill of a button being pressed.
    ButtonPressed => "button-pressed",

    // Tabs.
    /// The selected tab.
    TabActive => "tab-active",
    /// A tab the pointer is over.
    TabHover => "tab-hover",
    /// The title of a tab that is not selected.
    TabTitleInactive => "tab-title-inactive",
    /// The fill behind a tab's close button when the pointer is over it.
    TabCloseHover => "tab-close-hover",
    /// The fill behind a tab's close button while it is pressed.
    TabClosePressed => "tab-close-pressed",
    /// The close button's cross.
    TabCloseLabel => "tab-close-label",

    // Status bubble.
    /// The bubble that names a link under the pointer.
    StatusBackground => "status-background",
    /// The outline of that bubble.
    StatusBorder => "status-border",
    /// The text in that bubble.
    StatusText => "status-text",

    // Notice and information bars.
    /// A warning bar, such as the settings notice.
    NoticeBackground => "notice-background",
    /// The outline of a warning bar.
    NoticeBorder => "notice-border",
    /// An information bar, such as "restore your tabs?". Never blue.
    InfoBackground => "info-background",
    /// The outline of an information bar.
    InfoBorder => "info-border",
    /// Text on an information bar.
    InfoText => "info-text",

    // Popups and menus.
    /// The browser's own menus and popovers.
    PopupBackground => "popup-background",
    /// The outline of a popup.
    PopupBorder => "popup-border",
    /// A menu item, suggestion or list row the pointer is over.
    ItemHover => "item-hover",
    /// The fill of the selected row of a list (a suggestion, a bookmark folder).
    ItemSelected => "item-selected",
    /// Everything drawn on [`Role::ItemSelected`].
    ItemSelectedText => "item-selected-text",
    /// A line between menu items.
    Separator => "separator",
    /// The shortcut shown beside a menu item.
    ShortcutText => "shortcut-text",
    /// Text of something that cannot be used now.
    DisabledText => "disabled-text",

    // Dialog sheets, primary buttons, errors and focus.
    /// Laid over the page behind a sheet. Translucent.
    Scrim => "scrim",
    /// A dialog sheet.
    SheetBackground => "sheet-background",
    /// The shadow of a popup or sheet. Translucent.
    Shadow => "shadow",
    /// The fill of a primary button: the brand's fill accent.
    PrimaryButtonBackground => "primary-button-background",
    /// A primary button the pointer is over.
    PrimaryButtonHover => "primary-button-hover",
    /// A primary button being pressed.
    PrimaryButtonPressed => "primary-button-pressed",
    /// The label of a primary button.
    PrimaryButtonText => "primary-button-text",
    /// Errors and destructive actions.
    Danger => "danger",
    /// The outline of the control that has keyboard focus.
    FocusRing => "focus-ring",
    /// The part of a suggestion that matches what was typed.
    MatchHighlight => "match-highlight",

    // Page popups.
    /// Select lists, colour pickers and context menus a page opens: neutral grey, so they never
    /// pass for the browser's own menus.
    PagePopupBackground => "page-popup-background",
    /// The outline of a page popup.
    PagePopupBorder => "page-popup-border",
    /// The line naming the page above a select list or colour picker.
    PagePopupCaption => "page-popup-caption",

    // Media capture.
    /// The mark of a page that may use the camera or microphone: a recording red.
    CaptureIndicator => "capture-indicator",
    /// A glyph drawn on [`Role::CaptureIndicator`].
    CaptureIndicatorGlyph => "capture-indicator-glyph",
}

impl Role {
    /// The role whose `ChromePalette` property is called `name`, or `None`.
    pub fn from_slint_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.slint_name() == name)
    }
}

/// What a palette has for one scheme: an exhaustive `match` over [`Role`] that returns a literal.
pub type Table = fn(Role) -> Colour;

/// The colours of every [`Role`] in the light and in the dark scheme.
///
/// `appearance.palette_light` names the palette whose `light` table is used in the light scheme
/// and `appearance.palette_dark` the palette whose `dark` table is used in the dark scheme, so
/// the two halves of different palettes can be combined.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// The stored id: lower case letters and digits and hyphens, for example `"mercury-b2"`.
    pub id: &'static str,
    /// The name shown to the user.
    pub name: &'static str,
    /// Whether a user may choose it. A palette that is not selectable is never offered in a
    /// picker and is never a default.
    pub selectable: bool,
    /// The light half.
    pub light: Table,
    /// The dark half.
    pub dark: Table,
}

impl Palette {
    /// The colour of `role` in `scheme`. Total: every role has a colour in every scheme.
    pub fn colour(&self, scheme: Scheme, role: Role) -> Colour {
        self.table(scheme)(role)
    }

    /// The table for `scheme`.
    pub fn table(&self, scheme: Scheme) -> Table {
        match scheme {
            Scheme::Light => self.light,
            Scheme::Dark => self.dark,
        }
    }

    /// Every role with its colour in `scheme`, in [`Role::ALL`] order.
    pub fn colours(&self, scheme: Scheme) -> impl Iterator<Item = (Role, Colour)> + '_ {
        let table = self.table(scheme);
        Role::ALL.into_iter().map(move |role| (role, table(role)))
    }
}

/// The four stops of B2's Sun gradient, from the pale core to the rim (`#FFF3C4`, `#FFC857`,
/// `#FF8A2F`, `#EC5B27`). The sun in Orbit and the app icon use it; it is brand, not a palette
/// role, and the same in both schemes.
pub const SUN_STOPS: [Colour; 4] = [
    Colour::hex(0xfff3c4),
    Colour::hex(0xffc857),
    Colour::hex(0xff8a2f),
    Colour::hex(0xec5b27),
];

/// The ink drawn on the Sun (`#1E1233`).
pub const SUN_INK: Colour = Colour::hex(0x1e1233);
