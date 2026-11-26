// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Maps the engine's form messages (a focused field, a submitted password form, a date picker,
//! a validation message) to the browser's core types (txt-2 T10).

use servo::{
    ContextMenuDetails, DateTimePickerRequest, FieldFocus, PasswordSubmission, ValidationMessage,
};
use talaria_core::{
    ControlId, ControlRequest, ContextMenuInfo, DateTimeKind, FieldFocusInfo, FieldInputType,
    PasswordSubmittedInfo,
};

/// The core event fact for a focused field. `control` is the browser's id for the input, which
/// the caller resolves from the engine's control store. No field text comes with a focus.
pub(crate) fn field_focus_info(focus: FieldFocus, control: ControlId) -> FieldFocusInfo {
    FieldFocusInfo {
        document: focus.document,
        control,
        input_type: field_input_type(focus.input_type),
        name: focus.name,
        id: focus.id,
        autocomplete: focus.autocomplete,
        origin: focus.origin,
        top_level_site: focus.top_level_site,
        is_cross_site_frame: focus.is_cross_site_frame,
        user_initiated: focus.user_initiated,
        form_has_password: focus.form_has_password,
    }
}

/// The core event fact for a submitted password form.
pub(crate) fn password_submitted_info(submission: PasswordSubmission) -> PasswordSubmittedInfo {
    PasswordSubmittedInfo {
        document: submission.document,
        origin: submission.origin,
        top_level_site: submission.top_level_site,
        is_cross_site_frame: submission.is_cross_site_frame,
        action_url: submission.action_url.map(|url| url.to_string()),
        username: submission.username,
        password: submission.password,
        new_password: submission.new_password,
        had_user_activation: submission.had_user_activation,
    }
}

/// A date, time, month, week or local date-time picker for the control at `rect`.
pub(crate) fn date_time_request(
    rect: talaria_core::ControlRect,
    request: DateTimePickerRequest,
) -> ControlRequest {
    ControlRequest::DateTime {
        rect,
        kind: date_time_kind(request.kind),
        value: request.value,
        min: request.min,
        max: request.max,
        step: request.step,
    }
}

/// The context menu's element facts. The engine's media details are not reduced to a
/// `MediaAction` here: `media` stays `None` until the desktop decides which media controls to
/// offer (Task 11). `editable` is the presence of editable details.
pub(crate) fn context_menu_info(details: ContextMenuDetails) -> ContextMenuInfo {
    ContextMenuInfo {
        selection_text: details.selection_text,
        link_text: details.link_text,
        page_url: details.page_url.map(|url| url.to_string()),
        frame_url: details.frame_url.map(|url| url.to_string()),
        is_top_level_frame: details.is_top_level_frame,
        media: None,
        editable: details.editable.is_some(),
        is_password: details.is_password,
    }
}

/// A validation message for the control at `rect`.
pub(crate) fn validation_request(
    rect: talaria_core::ControlRect,
    message: ValidationMessage,
) -> ControlRequest {
    ControlRequest::ValidationMessage {
        rect,
        message: message.message,
    }
}

fn date_time_kind(kind: servo::DateTimeKind) -> DateTimeKind {
    match kind {
        servo::DateTimeKind::Date => DateTimeKind::Date,
        servo::DateTimeKind::Time => DateTimeKind::Time,
        servo::DateTimeKind::Month => DateTimeKind::Month,
        servo::DateTimeKind::Week => DateTimeKind::Week,
        servo::DateTimeKind::DatetimeLocal => DateTimeKind::DatetimeLocal,
    }
}

fn field_input_type(input_type: servo::InputMethodType) -> FieldInputType {
    match input_type {
        servo::InputMethodType::Color => FieldInputType::Color,
        servo::InputMethodType::Date => FieldInputType::Date,
        servo::InputMethodType::DatetimeLocal => FieldInputType::DatetimeLocal,
        servo::InputMethodType::Email => FieldInputType::Email,
        servo::InputMethodType::Month => FieldInputType::Month,
        servo::InputMethodType::Number => FieldInputType::Number,
        servo::InputMethodType::Password => FieldInputType::Password,
        servo::InputMethodType::Search => FieldInputType::Search,
        servo::InputMethodType::Tel => FieldInputType::Tel,
        servo::InputMethodType::Text => FieldInputType::Text,
        servo::InputMethodType::Time => FieldInputType::Time,
        servo::InputMethodType::Url => FieldInputType::Url,
        servo::InputMethodType::Week => FieldInputType::Week,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_time_picker_becomes_a_control_request_and_the_answer_returns_its_value() {
        let request = DateTimePickerRequest {
            kind: servo::DateTimeKind::Date,
            value: "2026-11-25".to_owned(),
            min: Some("2026-01-01".to_owned()),
            max: None,
            step: None,
        };
        let rect = talaria_core::ControlRect {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        };
        let ControlRequest::DateTime {
            kind, value, min, ..
        } = date_time_request(rect, request)
        else {
            panic!("expected a date-time request");
        };
        assert_eq!(kind, DateTimeKind::Date);
        assert_eq!(value, "2026-11-25");
        assert_eq!(min.as_deref(), Some("2026-01-01"));
    }

    #[test]
    fn a_validation_message_becomes_a_control_request() {
        let rect = talaria_core::ControlRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let request = validation_request(
            rect,
            ValidationMessage {
                message: "Required".to_owned(),
            },
        );
        assert!(matches!(
            request,
            ControlRequest::ValidationMessage { message, .. } if message == "Required"
        ));
    }
}
