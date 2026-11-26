// SPDX-License-Identifier: MPL-2.0
//! Form controls and menus the engine asks the browser to show: `<select>` popups, colour and
//! file pickers, the page context menu and the input-method area. Each request carries an id;
//! the browser answers it exactly once, or drops it, which gives the engine's default answer.

use std::path::PathBuf;

/// Identifies one engine request; ids are never reused within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlId(u64);

impl ControlId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

/// The element (or, for a context menu, the point) a control belongs to, as Servo 0.8.0
/// reports it: CSS pixels relative to the page's viewport. A point has zero size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    /// What the engine uses to select the option.
    pub id: usize,
    pub label: String,
    pub disabled: bool,
}

/// Options outside an option group come as single entries, in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectEntry {
    Option(SelectOption),
    Group {
        label: String,
        options: Vec<SelectOption>,
    },
}

/// The context-menu actions the engine carries out itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuAction {
    GoBack,
    GoForward,
    Reload,
    CopyLink,
    OpenLinkInNewView,
    CopyImageLink,
    OpenImageInNewView,
    Cut,
    Copy,
    Paste,
    SelectAll,
    /// Copies the image's pixels to the clipboard (rich pasteboard).
    CopyImage,
    /// A media control on the video or audio under the menu.
    Media(MediaAction),
}

/// A media control on the element under a context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    PlayPause,
    Mute,
    Loop,
    Controls,
}

/// The kind of a date or time input, for its picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeKind {
    Date,
    Time,
    Month,
    Week,
    DatetimeLocal,
}

/// What the page's selection, link and element are, for a context menu. Text fields are empty
/// for a password field (`is_password`), and their `Debug` prints no text.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct ContextMenuInfo {
    pub selection_text: Option<String>,
    pub link_text: Option<String>,
    pub page_url: Option<String>,
    pub frame_url: Option<String>,
    pub is_top_level_frame: bool,
    pub media: Option<MediaAction>,
    pub editable: bool,
    pub is_password: bool,
}

impl std::fmt::Debug for ContextMenuInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextMenuInfo")
            .field(
                "selection_text",
                &self.selection_text.as_ref().map(String::len),
            )
            .field("link_text", &self.link_text.as_ref().map(String::len))
            .field("is_top_level_frame", &self.is_top_level_frame)
            .field("media", &self.media)
            .field("editable", &self.editable)
            .field("is_password", &self.is_password)
            .finish_non_exhaustive()
    }
}

/// Longest text Look Up and Search send from a context menu, in characters.
const MAX_SEARCH_TEXT_CHARS: usize = 200;

/// The text a context menu's Look Up and Search use: the selection when it is not empty and the
/// element is not a password field, else the link's text, trimmed; at most 200 characters.
pub fn context_menu_search_text(info: &ContextMenuInfo) -> Option<String> {
    if info.is_password {
        return None;
    }
    let selection = info
        .selection_text
        .as_deref()
        .filter(|text| !text.is_empty());
    let text = match selection {
        Some(text) => text,
        None => info.link_text.as_deref()?.trim(),
    };
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(MAX_SEARCH_TEXT_CHARS).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextMenuEntry {
    Item {
        label: String,
        action: ContextMenuAction,
        enabled: bool,
    },
    Separator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlRequest {
    Select {
        rect: ControlRect,
        entries: Vec<SelectEntry>,
        /// Ids of the selected options.
        selected: Vec<usize>,
        multiple: bool,
    },
    Color {
        rect: ControlRect,
        current: Option<Rgb>,
    },
    File {
        multiple: bool,
        /// File-name extensions without the dot; empty accepts every file.
        extensions: Vec<String>,
    },
    ContextMenu {
        rect: ControlRect,
        entries: Vec<ContextMenuEntry>,
        link_url: Option<String>,
        image_url: Option<String>,
        details: Option<ContextMenuInfo>,
    },
    /// A date, time, month, week or local date-and-time picker for the input at `rect`. `value`
    /// is in the input's format ("" when empty).
    DateTime {
        rect: ControlRect,
        kind: DateTimeKind,
        value: String,
        min: Option<String>,
        max: Option<String>,
        step: Option<String>,
    },
    /// A validation message for the control at `rect`, at most 300 characters.
    ValidationMessage { rect: ControlRect, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlAnswer {
    /// Ids of the options to select.
    Select(Vec<usize>),
    Color(Rgb),
    /// Only files the user chose. An empty list leaves the input unchanged.
    Files(Vec<PathBuf>),
    ContextMenu(ContextMenuAction),
    /// The picked value in the input's format, or `None` to leave the input unchanged.
    DateTime(Option<String>),
    /// Closes the control with the engine's default answer: the unchanged selection or colour,
    /// no files, no menu action.
    Dismiss,
}
