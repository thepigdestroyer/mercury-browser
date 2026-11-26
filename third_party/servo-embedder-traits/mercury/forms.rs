/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the text lane (TXT-2). The form, picker, validation, context-menu
// data and password-hook types live here; the `TextMsg` variants that carry some of them are in
// `text.rs`, and the picker and validation requests are `EmbedderControlRequest` variants added
// in `embedder_controls.rs`.

//! Form controls and page data for Mercury: date and time pickers, validation messages, the
//! context menu's page details, and the password-field hooks.
//!
//! Every type that holds a typed value, a password or a username prints only its flags and sizes
//! in `Debug`.

use std::fmt;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{EmbedderControlId, InputMethodType};

/// The most characters of a validation message; a longer message is cut.
pub const MAX_VALIDATION_MESSAGE_CHARS: usize = 300;
/// The most characters of selected text in the context menu details.
pub const MAX_CONTEXT_SELECTION_CHARS: usize = 2_000;
/// The most characters of link text in the context menu details.
pub const MAX_LINK_TEXT_CHARS: usize = 500;

/// The kind of value a date or time picker chooses.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum DateTimeKind {
    Date,
    Time,
    Month,
    Week,
    DatetimeLocal,
}

/// A request to show a date or time picker for an `<input>` of one of the date or time types.
///
/// `value`, `min`, `max` and `step` are in the HTML value format of the kind (`2026-11-22`,
/// `13:45`, `2026-11`, `2026-W47`, `2026-11-22T13:45`), as the page wrote them.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct DateTimePickerRequest {
    pub kind: DateTimeKind,
    /// The current value, empty when there is none.
    pub value: String,
    pub min: Option<String>,
    pub max: Option<String>,
    pub step: Option<String>,
}

/// The message of an interactive validation bubble. `Debug` prints the length only.
#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct ValidationMessage {
    pub message: String,
}

impl fmt::Debug for ValidationMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("ValidationMessage")
            .field("message_len", &self.message.len())
            .finish()
    }
}

/// What the page's context menu needs beyond the element flags: the selection, the link text,
/// the URLs, the media state and the editable state. Password fields carry no text here.
///
/// `Debug` prints the flags and lengths only, never the text or the URLs.
#[derive(Clone, Default, Deserialize, PartialEq, Serialize)]
pub struct ContextMenuDetails {
    /// The selected text, at most [`MAX_CONTEXT_SELECTION_CHARS`]; `None` for a password field.
    pub selection_text: Option<String>,
    /// The text of the link the element is in, at most [`MAX_LINK_TEXT_CHARS`].
    pub link_text: Option<String>,
    /// The top-level page's URL when the frame is same-origin with it, otherwise `None`.
    pub page_url: Option<Url>,
    /// The URL of the frame the element is in.
    pub frame_url: Option<Url>,
    pub is_top_level_frame: bool,
    pub media: Option<MediaDetails>,
    pub editable: Option<EditableDetails>,
    pub is_password: bool,
}

impl fmt::Debug for ContextMenuDetails {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("ContextMenuDetails")
            .field("has_selection", &self.selection_text.is_some())
            .field("has_link_text", &self.link_text.is_some())
            .field("has_page_url", &self.page_url.is_some())
            .field("has_frame_url", &self.frame_url.is_some())
            .field("is_top_level_frame", &self.is_top_level_frame)
            .field("media", &self.media.is_some())
            .field("editable", &self.editable.is_some())
            .field("is_password", &self.is_password)
            .finish()
    }
}

/// The state of the audio or video element the context menu was opened on.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct MediaDetails {
    pub kind: MediaKind,
    pub src: Option<Url>,
    pub paused: bool,
    pub muted: bool,
    pub looping: bool,
    pub controls: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum MediaKind {
    Audio,
    Video,
}

/// The editable state of the element the context menu was opened on.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EditableDetails {
    pub input_type: Option<InputMethodType>,
    pub spellcheck: Option<bool>,
    pub multiline: bool,
    pub read_only: bool,
}

/// An action on the element's media, run by the engine (the page's own controls).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum MediaContextAction {
    PlayPause,
    Mute,
    Loop,
    Controls,
}

/// A password-capable field got focus. It carries no value: `Debug` prints the flags only.
///
/// `origin` is the focused document's own origin, taken in the content process; a page cannot
/// claim another origin here. `control` is the `InputMethod` control raised for the same focus,
/// so the embedder can pair the two.
#[derive(Clone, Deserialize, PartialEq, Serialize)]
pub struct FieldFocus {
    /// The document's pipeline id, as `TextInputState::document`.
    pub document: u64,
    pub control: EmbedderControlId,
    pub input_type: InputMethodType,
    pub name: Option<String>,
    pub id: Option<String>,
    pub autocomplete: Option<String>,
    pub origin: String,
    pub top_level_site: Option<String>,
    pub is_cross_site_frame: bool,
    pub user_initiated: bool,
    pub form_has_password: bool,
}

impl fmt::Debug for FieldFocus {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("FieldFocus")
            .field("document", &self.document)
            .field("input_type", &self.input_type)
            .field("has_name", &self.name.is_some())
            .field("has_id", &self.id.is_some())
            .field("has_autocomplete", &self.autocomplete.is_some())
            .field("origin", &self.origin)
            .field("is_cross_site_frame", &self.is_cross_site_frame)
            .field("user_initiated", &self.user_initiated)
            .field("form_has_password", &self.form_has_password)
            .finish()
    }
}

/// A password form was submitted. It carries the passwords: `Debug` prints lengths and flags only.
///
/// `password` is the first password field of the form with a value; `new_password` is a second
/// one, with a different value (sign-up, change-password forms). `origin` is the document's own.
#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct PasswordSubmission {
    pub document: u64,
    pub origin: String,
    pub top_level_site: Option<String>,
    pub is_cross_site_frame: bool,
    pub action_url: Option<Url>,
    pub username: Option<String>,
    pub password: String,
    pub new_password: Option<String>,
    pub had_user_activation: bool,
}

impl fmt::Debug for PasswordSubmission {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("PasswordSubmission")
            .field("document", &self.document)
            .field("origin", &self.origin)
            .field("is_cross_site_frame", &self.is_cross_site_frame)
            .field("has_action_url", &self.action_url.is_some())
            .field("has_username", &self.username.is_some())
            .field("password_len", &self.password.len())
            .field("has_new_password", &self.new_password.is_some())
            .field("had_user_activation", &self.had_user_activation)
            .finish()
    }
}

/// A command from the embedder to the engine: put this text into the field that the
/// `InputMethod` control named. The engine checks the element before it sets anything.
#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
pub enum InputMethodResponse {
    Fill { text: String },
}

impl fmt::Debug for InputMethodResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Fill { text } => formatter
                .debug_struct("Fill")
                .field("text_len", &text.len())
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control_id() -> EmbedderControlId {
        servo_base::id::PipelineNamespace::install(servo_base::id::PipelineNamespaceId(1));
        EmbedderControlId {
            webview_id: servo_base::id::WebViewId::new(
                servo_base::id::PainterId::next(),
            ),
            pipeline_id: servo_base::id::PipelineId::new(),
            index: servo_base::Epoch(7),
        }
    }

    #[test]
    fn debug_never_prints_a_typed_value_or_password() {
        let focus = FieldFocus {
            document: 9,
            control: control_id(),
            input_type: InputMethodType::Password,
            name: Some("pw".to_owned()),
            id: Some("hunter2-field".to_owned()),
            autocomplete: Some("current-password".to_owned()),
            origin: "https://bank.test".to_owned(),
            top_level_site: Some("bank.test".to_owned()),
            is_cross_site_frame: false,
            user_initiated: true,
            form_has_password: true,
        };
        let printed = format!("{focus:?}");
        assert!(printed.contains("FieldFocus"), "{printed}");
        assert!(!printed.contains("hunter2-field"), "{printed}");
    }

    #[test]
    fn debug_of_a_password_submission_never_prints_the_passwords() {
        let submission = PasswordSubmission {
            document: 9,
            origin: "https://bank.test".to_owned(),
            top_level_site: None,
            is_cross_site_frame: true,
            action_url: None,
            username: Some("ann".to_owned()),
            password: "hunter2".to_owned(),
            new_password: Some("hunter3".to_owned()),
            had_user_activation: true,
        };
        let printed = format!("{submission:?}");
        assert!(printed.contains("password_len"), "{printed}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(!printed.contains("hunter3"), "{printed}");
        assert!(!printed.contains("ann"), "{printed}");
    }

    #[test]
    fn debug_of_an_input_method_fill_never_prints_the_text() {
        let fill = InputMethodResponse::Fill {
            text: "hunter2".to_owned(),
        };
        let printed = format!("{fill:?}");
        assert!(printed.contains("text_len"), "{printed}");
        assert!(!printed.contains("hunter2"), "{printed}");
    }

    #[test]
    fn debug_of_a_validation_message_never_prints_it() {
        let message = ValidationMessage {
            message: "Please fill in SECRETFIELD".to_owned(),
        };
        let printed = format!("{message:?}");
        assert!(printed.contains("message_len"), "{printed}");
        assert!(!printed.contains("SECRETFIELD"), "{printed}");
    }

    #[test]
    fn debug_of_context_menu_details_never_prints_the_selection_or_urls() {
        let details = ContextMenuDetails {
            selection_text: Some("SECRETSELECTION".to_owned()),
            link_text: Some("SECRETLINK".to_owned()),
            page_url: Url::parse("https://secret.test/private").ok(),
            ..ContextMenuDetails::default()
        };
        let printed = format!("{details:?}");
        assert!(printed.contains("has_selection"), "{printed}");
        assert!(!printed.contains("SECRET"), "{printed}");
        assert!(!printed.contains("secret.test"), "{printed}");
    }

    #[test]
    fn the_form_types_cross_the_process_boundary() {
        fn assert_serde<T: Serialize + for<'de> Deserialize<'de>>() {}
        assert_serde::<DateTimePickerRequest>();
        assert_serde::<ValidationMessage>();
        assert_serde::<ContextMenuDetails>();
        assert_serde::<FieldFocus>();
        assert_serde::<PasswordSubmission>();
        assert_serde::<InputMethodResponse>();
    }

    #[test]
    fn context_menu_details_default_has_no_details() {
        let printed = format!("{:?}", ContextMenuDetails::default());
        assert!(printed.contains("has_selection"), "{printed}");
        assert!(printed.contains("false"), "{printed}");
    }
}
