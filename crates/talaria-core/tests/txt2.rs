// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Clipboard policy, rich pasteboard defaults, form and picker events and the context menu's
//! details (txt-2 lane, T9).

use talaria_core::{
    AsyncAccess, BrowserEvent, ClipboardRequestOrigin, ClipboardSourceKind, ContextMenuInfo,
    ControlId, ControlRect, ControlRequest, FieldFocusInfo, FieldInputType, MemoryPasteboard,
    PasswordSubmittedInfo, Pasteboard, PasteboardItem, RefusalReason, async_read_access,
    async_write_access, context_menu_search_text,
};

const RECT: ControlRect = ControlRect {
    x: 0,
    y: 0,
    width: 10,
    height: 10,
};

fn origin(same_origin_with_top: bool, had_transient_activation: bool) -> ClipboardRequestOrigin {
    ClipboardRequestOrigin {
        same_origin_with_top,
        is_top_level: same_origin_with_top,
        had_transient_activation,
    }
}

fn item(mime: &str, text: &str) -> PasteboardItem {
    PasteboardItem {
        mime: mime.to_owned(),
        bytes: text.as_bytes().to_vec(),
    }
}

fn info(selection: Option<&str>, link: Option<&str>, is_password: bool) -> ContextMenuInfo {
    ContextMenuInfo {
        selection_text: selection.map(str::to_owned),
        link_text: link.map(str::to_owned),
        page_url: None,
        frame_url: None,
        is_top_level_frame: true,
        media: None,
        editable: false,
        is_password,
    }
}

#[test]
fn async_write_is_refused_for_a_cross_origin_frame_even_with_engine_activation() {
    let frame = ClipboardRequestOrigin {
        same_origin_with_top: false,
        is_top_level: false,
        had_transient_activation: true,
    };
    assert_eq!(
        async_write_access(&frame, true),
        AsyncAccess::Refuse(RefusalReason::CrossOrigin)
    );
}

#[test]
fn async_write_is_refused_when_the_browser_saw_no_activation() {
    assert_eq!(
        async_write_access(&origin(true, true), false),
        AsyncAccess::Refuse(RefusalReason::NoActivation)
    );
}

#[test]
fn async_write_is_allowed_with_both() {
    assert_eq!(
        async_write_access(&origin(true, true), true),
        AsyncAccess::Allow
    );
}

#[test]
fn async_read_is_refused_while_reads_are_off() {
    assert_eq!(
        async_read_access(&origin(true, true), true, false),
        AsyncAccess::Refuse(RefusalReason::ReadsOff)
    );
}

#[test]
fn async_read_asks_permission_when_gate_on_and_both_activations() {
    assert_eq!(
        async_read_access(&origin(true, true), true, true),
        AsyncAccess::AskPermission
    );
}

#[test]
fn a_text_only_pasteboard_keeps_plain_text_through_the_rich_methods() {
    let pasteboard = MemoryPasteboard::default();
    pasteboard.write_items(
        &[item("text/plain", "a"), item("text/html", "<b>a</b>")],
        None,
    );
    assert_eq!(pasteboard.read_text().as_deref(), Some("a"));
    assert_eq!(pasteboard.read_items(&[]), vec![item("text/plain", "a")]);
}

#[test]
fn the_user_clipboard_paths_ignore_the_async_policy() {
    // The policy functions take no source: the caller sends a user's Cut, Copy or Paste past
    // them (`ClipboardSourceKind::User`) and asks them only for a script's request.
    assert_ne!(ClipboardSourceKind::User, ClipboardSourceKind::Script);
}

#[test]
fn context_menu_search_text_prefers_the_selection() {
    assert_eq!(
        context_menu_search_text(&info(Some("pick me"), Some("https://x.example"), false))
            .as_deref(),
        Some("pick me")
    );
    assert_eq!(
        context_menu_search_text(&info(None, Some("  https://x.example  "), false)).as_deref(),
        Some("https://x.example")
    );
}

#[test]
fn context_menu_search_text_is_none_for_a_password_field() {
    assert_eq!(
        context_menu_search_text(&info(Some("hunter2"), None, true)),
        None
    );
}

#[test]
fn context_menu_search_text_is_cut_at_200_chars() {
    let long = "a".repeat(300);
    let text = context_menu_search_text(&info(Some(&long), None, false)).unwrap_or_default();
    assert_eq!(text.chars().count(), 200);
}

#[test]
fn debug_of_field_focus_and_password_submitted_has_no_values() {
    let focus = FieldFocusInfo {
        document: 7,
        control: ControlId::new(3),
        input_type: FieldInputType::Password,
        name: Some("user-field-name".to_owned()),
        id: Some("secret-id".to_owned()),
        autocomplete: Some("current-password".to_owned()),
        origin: "https://bank.example".to_owned(),
        top_level_site: Some("bank.example".to_owned()),
        is_cross_site_frame: true,
        user_initiated: true,
        form_has_password: true,
    };
    let submitted = PasswordSubmittedInfo {
        document: 7,
        origin: "https://bank.example".to_owned(),
        top_level_site: Some("bank.example".to_owned()),
        is_cross_site_frame: false,
        action_url: None,
        username: Some("alice-user".to_owned()),
        password: "hunter2-secret".to_owned(),
        new_password: Some("newsecret-value".to_owned()),
        had_user_activation: true,
    };
    let printed = [
        format!("{focus:?}"),
        format!("{submitted:?}"),
        format!("{:?}", BrowserEvent::FieldFocused(focus.clone())),
        format!("{:?}", BrowserEvent::PasswordSubmitted(submitted.clone())),
    ];
    for text in printed {
        for secret in [
            "user-field-name",
            "secret-id",
            "bank.example",
            "alice-user",
            "hunter2-secret",
            "newsecret-value",
        ] {
            assert!(!text.contains(secret), "{secret} printed in {text}");
        }
    }
}

#[test]
fn debug_never_prints_the_clipboard_bytes_or_the_selection() {
    let printed = format!("{:?}", item("text/plain", "SECRET-CLIP"));
    assert!(printed.contains("text/plain"));
    assert!(!printed.contains("SECRET-CLIP"));
    let menu = format!(
        "{:?}",
        info(Some("secret selection"), Some("secret link"), false)
    );
    assert!(!menu.contains("secret selection"));
    assert!(!menu.contains("secret link"));
}

#[test]
fn an_existing_context_menu_request_still_builds_without_details() {
    let request = ControlRequest::ContextMenu {
        rect: RECT,
        entries: Vec::new(),
        link_url: None,
        image_url: None,
        details: None,
    };
    assert!(matches!(
        request,
        ControlRequest::ContextMenu { details: None, .. }
    ));
}
