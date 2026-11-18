// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Camera and microphone (media-capture lane): the permission prompt's camera and microphone
//! form (asked together, "Remember for this site until you quit", the notice when macOS refuses),
//! the mark of a page that may capture with its Stop button, and the diagnostics of fake-device
//! runs.
//!
//! Servo does not tell the browser when a page starts or stops capturing, so the marks show what
//! the page may use: from the grant until the page changes (`talaria_core::CaptureAccess`).
//! macOS's own menu-bar indicator shows actual device use.

#[cfg(target_os = "macos")]
mod fake_devices;
mod system_access;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use mercury_ui::{CaptureView, MainWindow, PageRequestStyle, PageRequestView};
use slint::{ComponentHandle, ModelRc, VecModel};
use talaria_core::{
    BrowserController, CaptureAccess, PagePermission, PageRequestAnswer, PageRequestKind,
    PageRequestReply, PendingPageRequest,
};
use talaria_servo::page_request_clock;

#[cfg(target_os = "macos")]
pub(crate) use fake_devices::select_fake_media_devices;
pub(crate) use system_access::{
    SystemAccess, SystemAccessSource, open_privacy_settings, system_access,
};

use crate::cli::PromptAnswer;
use crate::ui_sync::sync_ui;

/// The choice under a camera or microphone prompt. Offered only because Mercury's engine refuses
/// these permissions to frames of another origin than the top-level page and to workers, so the
/// site named is the one that asks.
pub(crate) const REMEMBER_CHOICE: &str = "Remember for this site until you quit";

/// Where the answer to "may Mercury use the camera" comes from in this run.
pub(crate) fn set_system_access_source(source: SystemAccessSource) {
    system_access::set_source(source);
}

const fn is_capture(permission: PagePermission) -> bool {
    matches!(
        permission,
        PagePermission::Camera | PagePermission::Microphone
    )
}

/// "camera", "microphone" or "camera and microphone".
fn devices_phrase(camera: bool, microphone: bool) -> &'static str {
    match (camera, microphone) {
        (true, true) => "camera and microphone",
        (true, false) => "camera",
        (false, true) => "microphone",
        (false, false) => "",
    }
}

/// The kinds a camera or microphone prompt asks for (both when two requests are shown together).
fn prompt_devices(permission: PagePermission, paired: Option<PagePermission>) -> (bool, bool) {
    let kinds = [Some(permission), paired];
    (
        kinds.contains(&Some(PagePermission::Camera)),
        kinds.contains(&Some(PagePermission::Microphone)),
    )
}

/// The prompt for a camera or microphone request, or `None` for any other request. `site` is the
/// asking origin as shown; `system` says what macOS allows for each kind asked.
pub(crate) fn permission_view(
    pending: &PendingPageRequest,
    system: impl Fn(PagePermission) -> SystemAccess,
) -> Option<PageRequestView> {
    let PageRequestKind::Permission(permission) = *pending.kind() else {
        return None;
    };
    if !is_capture(permission) {
        return None;
    }
    let site = pending.origin().label();
    let (camera, microphone) = prompt_devices(permission, pending.paired_permission());
    let heading = format!(
        "{site} wants to use your {}",
        devices_phrase(camera, microphone)
    );
    let denied_camera = camera && system(PagePermission::Camera) == SystemAccess::Denied;
    let denied_microphone =
        microphone && system(PagePermission::Microphone) == SystemAccess::Denied;
    let view = if denied_camera || denied_microphone {
        let devices = devices_phrase(denied_camera, denied_microphone);
        let panes = match (denied_camera, denied_microphone) {
            (true, true) => "Camera and Microphone",
            (true, false) => "Camera",
            _ => "Microphone",
        };
        PageRequestView {
            heading: heading.into(),
            message: format!(
                "macOS does not allow Mercury Browser to use the {devices}. You can allow it in \
                 System Settings, Privacy & Security, {panes}."
            )
            .into(),
            accept_label: "Open Privacy Settings".into(),
            decline_label: "Close".into(),
            ..PageRequestView::default()
        }
    } else {
        PageRequestView {
            heading: heading.into(),
            accept_label: "Allow".into(),
            decline_label: "Block".into(),
            choice_label: if pending.origin().key().is_some() {
                REMEMBER_CHOICE
            } else {
                ""
            }
            .into(),
            ..PageRequestView::default()
        }
    };
    Some(PageRequestView {
        style: PageRequestStyle::Permission,
        ..view
    })
}

/// What the user's click on a camera or microphone prompt means, or `None` for any other
/// request: the reply for the page, and which privacy pane to open (when macOS refuses, the
/// prompt's main button opens System Settings and the page is refused).
pub(crate) fn capture_reply(
    pending: &PendingPageRequest,
    accepted: bool,
    checked: bool,
    system: impl Fn(PagePermission) -> SystemAccess,
) -> Option<(PageRequestReply, Option<PagePermission>)> {
    let PageRequestKind::Permission(permission) = *pending.kind() else {
        return None;
    };
    if !is_capture(permission) {
        return None;
    }
    let (camera, microphone) = prompt_devices(permission, pending.paired_permission());
    let denied = [
        (camera, PagePermission::Camera),
        (microphone, PagePermission::Microphone),
    ]
    .into_iter()
    .find(|(asked, kind)| *asked && system(*kind) == SystemAccess::Denied)
    .map(|(_, kind)| kind);
    if let Some(denied) = denied {
        let reply = PageRequestReply::new(PageRequestAnswer::Decline);
        return Some((reply, accepted.then_some(denied)));
    }
    let reply = PageRequestReply::new(if accepted {
        PageRequestAnswer::Accept
    } else {
        PageRequestAnswer::Decline
    });
    let reply = if checked && pending.origin().key().is_some() {
        reply.remembering_for_site()
    } else {
        reply
    };
    Some((reply, None))
}

/// `--answer-permission-prompts`: the reply given to a permission prompt without anyone at the
/// keyboard.
pub(crate) fn automatic_reply(answer: PromptAnswer) -> PageRequestReply {
    PageRequestReply::new(match answer {
        PromptAnswer::Allow => PageRequestAnswer::Accept,
        PromptAnswer::Deny => PageRequestAnswer::Decline,
    })
}

/// The automatic reply to `pending`: as a click on its main button (`Allow`) or on Block would
/// answer it, except that System Settings is never opened. A camera or microphone prompt that
/// macOS refuses is refused whatever the answer.
pub(crate) fn automatic_reply_to(
    pending: &PendingPageRequest,
    answer: PromptAnswer,
    system: impl Fn(PagePermission) -> SystemAccess,
) -> PageRequestReply {
    match capture_reply(pending, answer == PromptAnswer::Allow, false, system) {
        Some((reply, _privacy_pane)) => reply,
        None => automatic_reply(answer),
    }
}

/// The mark of the tab whose page may use `access`, named by `site`.
/// `stop_refused`: the page kept itself from being reloaded after Stop.
pub(crate) fn capture_view(
    access: CaptureAccess,
    site: Option<&str>,
    stop_refused: bool,
) -> CaptureView {
    if access.is_empty() {
        return CaptureView::default();
    }
    let devices = devices_phrase(access.camera, access.microphone);
    CaptureView {
        camera: access.camera,
        microphone: access.microphone,
        heading: format!("{} can use your {devices}", site.unwrap_or("This page")).into(),
        note: if stop_refused {
            format!(
                "The page did not reload, so it can still use your {devices}. Close the tab to \
                 stop it."
            )
        } else {
            String::new()
        }
        .into(),
    }
}

/// Shows which tabs' pages may capture, and the active tab's mark beside the address bar.
pub(crate) fn sync_capture(window: &MainWindow, controller: &BrowserController) {
    let view_of = |tab_id| {
        capture_view(
            controller.capture_access(tab_id),
            controller.capture_origin(tab_id).as_deref(),
            controller.capture_stop_refused(tab_id),
        )
    };
    let active = view_of(controller.active_tab_id());
    if active.heading.is_empty() && window.get_capture_bubble_open() {
        window.set_capture_bubble_open(false);
    }
    if window.get_capture() != active {
        window.set_capture(active);
    }
    let tabs = controller
        .tabs()
        .map(|tab| view_of(tab.id()))
        .collect::<Vec<_>>();
    window.set_capture_tabs(ModelRc::new(VecModel::from(tabs)));
}

pub(crate) fn connect_capture_callbacks(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    answer_prompts: Option<PromptAnswer>,
) {
    let weak_window = window.as_weak();
    let stop_controller = Rc::clone(&controller);
    window.on_capture_stop(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let tab_id = stop_controller.borrow().active_tab_id();
        stop_controller.borrow_mut().stop_capture(tab_id);
        if let Some(window) = weak_window.upgrade() {
            sync_ui(&window, &stop_controller.borrow());
            window.window().request_redraw();
        }
        crate::engine::request_engine_pump();
    });

    if let Some(answer) = answer_prompts {
        start_answering_prompts(window, controller, answer);
    }
}

thread_local! {
    /// The timer of `--answer-permission-prompts`; kept for the life of the window.
    static ANSWER_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
}

/// How long a permission prompt stays on screen before the diagnostic answers it.
const AUTOMATIC_ANSWER_DELAY: Duration = Duration::from_millis(600);

/// `--answer-permission-prompts`: answers the active tab's permission prompt once it has been
/// shown for a moment.
fn start_answering_prompts(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    answer: PromptAnswer,
) {
    let weak_window = window.as_weak();
    let shown: RefCell<Option<(talaria_core::PageRequestId, Duration)>> = RefCell::new(None);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            let tab_id = controller.borrow().active_tab_id();
            let pending = controller
                .borrow()
                .pending_page_request(tab_id)
                .filter(|pending| matches!(pending.kind(), PageRequestKind::Permission(_)));
            let Some(pending) = pending else {
                shown.replace(None);
                return;
            };
            let now = page_request_clock();
            let previously_shown = *shown.borrow();
            let since = match previously_shown {
                Some((id, since)) if id == pending.id() => since,
                _ => {
                    shown.replace(Some((pending.id(), now)));
                    return;
                }
            };
            if now.saturating_sub(since) < AUTOMATIC_ANSWER_DELAY {
                return;
            }
            log::info!(
                "answering permission prompt {} automatically ({answer:?})",
                pending.id().value()
            );
            controller.borrow_mut().answer_page_request(
                tab_id,
                pending.id(),
                automatic_reply_to(&pending, answer, system_access),
                now,
            );
            shown.replace(None);
            if let Some(window) = weak_window.upgrade() {
                sync_ui(&window, &controller.borrow());
                window.window().request_redraw();
            }
            // A screenshot run's window is covered and does not render, so pump explicitly.
            crate::engine::request_engine_pump();
        },
    );
    ANSWER_TIMER.with(|slot| *slot.borrow_mut() = Some(timer));
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "macos"))]
    use talaria_core::BrowserCommand;
    use talaria_core::{BrowserEvent, PageRequest, PageRequestId};

    use super::*;

    fn pending(requests: &[(u64, &str, PagePermission)]) -> PendingPageRequest {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        controller.handle_event(
            tab_id,
            BrowserEvent::AddressChanged {
                url: requests[0].1.to_owned(),
            },
        );
        for (id, url, permission) in requests {
            controller.handle_event(
                tab_id,
                BrowserEvent::PageRequested(PageRequest {
                    id: PageRequestId::new(*id),
                    url: (*url).to_owned(),
                    kind: PageRequestKind::Permission(*permission),
                    received_at: Duration::ZERO,
                }),
            );
        }
        controller
            .pending_page_request(tab_id)
            .expect("the request is pending")
    }

    const PAGE: &str = "http://127.0.0.1:8123/pages/capture.html";

    fn allowed(_: PagePermission) -> SystemAccess {
        SystemAccess::Allowed
    }

    #[test]
    fn other_permissions_keep_the_dialogs_prompt() {
        let pending = pending(&[(1, PAGE, PagePermission::PersistentStorage)]);
        assert_eq!(permission_view(&pending, allowed), None);
        assert_eq!(capture_reply(&pending, true, true, allowed), None);
    }

    #[test]
    fn automatic_answers_allow_or_refuse() {
        assert_eq!(
            automatic_reply(PromptAnswer::Allow),
            PageRequestReply::new(PageRequestAnswer::Accept)
        );
        assert_eq!(
            automatic_reply(PromptAnswer::Deny),
            PageRequestReply::new(PageRequestAnswer::Decline)
        );
    }

    #[test]
    fn the_mark_names_the_site_and_the_devices() {
        assert_eq!(
            capture_view(CaptureAccess::default(), Some("a.example"), false),
            CaptureView::default()
        );
        let view = capture_view(
            CaptureAccess {
                camera: true,
                microphone: true,
            },
            Some("meet.example"),
            false,
        );
        assert!(view.camera && view.microphone);
        assert_eq!(
            view.heading,
            "meet.example can use your camera and microphone"
        );
        assert_eq!(
            capture_view(
                CaptureAccess {
                    camera: false,
                    microphone: true,
                },
                None,
                false
            )
            .heading,
            "This page can use your microphone"
        );
    }

    #[test]
    fn a_page_that_stayed_after_stop_is_said_to_still_have_access() {
        let access = CaptureAccess {
            camera: true,
            microphone: false,
        };
        assert_eq!(capture_view(access, Some("meet.example"), false).note, "");
        let view = capture_view(access, Some("meet.example"), true);
        assert_eq!(view.heading, "meet.example can use your camera");
        assert_eq!(
            view.note,
            "The page did not reload, so it can still use your camera. Close the tab to stop it."
        );
    }

    // Camera and microphone are asked for only on macOS; elsewhere they never wait for an answer.
    #[cfg(target_os = "macos")]
    mod macos_prompts {
        use super::*;

        fn camera_denied(permission: PagePermission) -> SystemAccess {
            if permission == PagePermission::Camera {
                SystemAccess::Denied
            } else {
                SystemAccess::Allowed
            }
        }

        #[test]
        fn camera_and_microphone_are_asked_in_one_prompt_with_the_remember_choice() {
            let pending = pending(&[
                (1, PAGE, PagePermission::Microphone),
                (2, PAGE, PagePermission::Camera),
            ]);
            let view = permission_view(&pending, allowed).expect("a capture prompt");

            assert_eq!(view.style, PageRequestStyle::Permission);
            assert_eq!(
                view.heading,
                "127.0.0.1:8123 wants to use your camera and microphone"
            );
            assert_eq!(view.choice_label, REMEMBER_CHOICE);
            assert_eq!(view.accept_label, "Allow");
            assert_eq!(view.decline_label, "Block");
            assert_eq!(view.message, "");
        }

        #[test]
        fn a_single_kind_names_only_that_device() {
            let pending = pending(&[(1, PAGE, PagePermission::Camera)]);
            assert_eq!(
                permission_view(&pending, allowed)
                    .expect("a capture prompt")
                    .heading,
                "127.0.0.1:8123 wants to use your camera"
            );
        }

        #[test]
        fn allow_with_the_choice_ticked_remembers_for_the_site() {
            let pending = pending(&[(1, PAGE, PagePermission::Camera)]);

            assert_eq!(
                capture_reply(&pending, true, true, allowed),
                Some((
                    PageRequestReply::new(PageRequestAnswer::Accept).remembering_for_site(),
                    None
                ))
            );
            assert_eq!(
                capture_reply(&pending, false, false, allowed),
                Some((PageRequestReply::new(PageRequestAnswer::Decline), None))
            );
        }

        #[test]
        fn a_page_without_an_origin_is_never_remembered() {
            let pending = pending(&[(1, "file:///tmp/capture.html", PagePermission::Camera)]);
            let view = permission_view(&pending, allowed).expect("a capture prompt");

            assert_eq!(view.choice_label, "");
            assert_eq!(
                capture_reply(&pending, true, true, allowed),
                Some((PageRequestReply::new(PageRequestAnswer::Accept), None))
            );
        }

        #[test]
        fn when_macos_refuses_the_prompt_explains_and_the_page_is_refused() {
            let pending = pending(&[
                (1, PAGE, PagePermission::Camera),
                (2, PAGE, PagePermission::Microphone),
            ]);
            let view = permission_view(&pending, camera_denied).expect("a capture prompt");

            assert_eq!(
                view.heading,
                "127.0.0.1:8123 wants to use your camera and microphone"
            );
            assert_eq!(
                view.message,
                "macOS does not allow Mercury Browser to use the camera. You can allow it in System \
                 Settings, Privacy & Security, Camera."
            );
            assert_eq!(view.accept_label, "Open Privacy Settings");
            assert_eq!(view.decline_label, "Close");
            assert_eq!(view.choice_label, "");

            let refused = PageRequestReply::new(PageRequestAnswer::Decline);
            assert_eq!(
                capture_reply(&pending, true, true, camera_denied),
                Some((refused.clone(), Some(PagePermission::Camera)))
            );
            assert_eq!(
                capture_reply(&pending, false, false, camera_denied),
                Some((refused, None))
            );
        }

        #[test]
        fn an_automatic_allow_is_refused_when_macos_refuses() {
            let pending = pending(&[(1, PAGE, PagePermission::Camera)]);
            let refused = PageRequestReply::new(PageRequestAnswer::Decline);

            assert_eq!(
                automatic_reply_to(&pending, PromptAnswer::Allow, camera_denied),
                refused
            );
            assert_eq!(
                automatic_reply_to(&pending, PromptAnswer::Allow, allowed),
                PageRequestReply::new(PageRequestAnswer::Accept)
            );
            let storage = super::pending(&[(2, PAGE, PagePermission::PersistentStorage)]);
            assert_eq!(
                automatic_reply_to(&storage, PromptAnswer::Allow, camera_denied),
                PageRequestReply::new(PageRequestAnswer::Accept)
            );
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_a_camera_request_is_refused_with_nothing_waiting() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        controller.handle_event(
            tab_id,
            BrowserEvent::AddressChanged {
                url: PAGE.to_owned(),
            },
        );
        controller.handle_event(
            tab_id,
            BrowserEvent::PageRequested(PageRequest {
                id: PageRequestId::new(1),
                url: PAGE.to_owned(),
                kind: PageRequestKind::Permission(PagePermission::Camera),
                received_at: Duration::ZERO,
            }),
        );

        assert!(controller.pending_page_request(tab_id).is_none());
        let answers: Vec<_> = std::iter::from_fn(|| controller.next_command())
            .map(|command| command.command().clone())
            .filter(|command| matches!(command, BrowserCommand::AnswerPageRequest { .. }))
            .collect();
        assert_eq!(
            answers,
            [BrowserCommand::AnswerPageRequest {
                id: PageRequestId::new(1),
                answer: PageRequestAnswer::Decline,
            }]
        );
    }
}
