// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
mod common;

use common::test_search;
use talaria_core::{
    BrowserCommand, BrowserController, BrowserEvent, BrowserState, ClipboardCommand, ColorScheme,
    ControlAnswer, ControlId, Direction, EndFindRequest, FindRequest, ImeInput, KeyboardInput,
    LoginFill, MediaSessionAction, PageRequestAnswer, PageRequestId, PointerButton,
    PointerButtonAction, WebView, dispatch_command,
};

fn history(urls: &[&str], current: usize) -> BrowserEvent {
    BrowserEvent::HistoryEntries {
        urls: urls.iter().map(|url| (*url).to_owned()).collect(),
        current,
    }
}

#[derive(Debug, Default, PartialEq)]
struct Recorded {
    calls: Vec<String>,
}

impl WebView for Recorded {
    type Error = std::convert::Infallible;

    fn navigate(&mut self, url: &str) -> Result<(), Self::Error> {
        self.calls.push(format!("navigate {url}"));
        Ok(())
    }
    fn go_back(&mut self) -> Result<(), Self::Error> {
        self.calls.push("go_back".to_owned());
        Ok(())
    }
    fn go_forward(&mut self) -> Result<(), Self::Error> {
        self.calls.push("go_forward".to_owned());
        Ok(())
    }
    fn reload(&mut self) -> Result<(), Self::Error> {
        self.calls.push("reload".to_owned());
        Ok(())
    }
    fn pointer_moved(&mut self, _: f32, _: f32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn pointer_button(
        &mut self,
        _: PointerButtonAction,
        _: PointerButton,
        _: f32,
        _: f32,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn scroll(&mut self, _: f32, _: f32, _: f32, _: f32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn set_focus(&mut self, _: bool) -> Result<(), Self::Error> {
        Ok(())
    }
    fn keyboard_input(&mut self, _: KeyboardInput) -> Result<(), Self::Error> {
        Ok(())
    }
    fn ime_input(&mut self, _: ImeInput) -> Result<(), Self::Error> {
        Ok(())
    }
    fn set_page_zoom(&mut self, _: u16) -> Result<(), Self::Error> {
        Ok(())
    }
    fn pinch_zoom(&mut self, _: f32, _: f32, _: f32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn set_color_scheme(&mut self, _: ColorScheme) -> Result<(), Self::Error> {
        Ok(())
    }
    fn exit_fullscreen(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn pointer_left(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn answer_control(&mut self, _: ControlId, _: ControlAnswer) -> Result<(), Self::Error> {
        Ok(())
    }
    fn answer_page_request(
        &mut self,
        _: PageRequestId,
        _: PageRequestAnswer,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn clipboard(&mut self, _: ClipboardCommand) -> Result<(), Self::Error> {
        Ok(())
    }
    fn set_audio_muted(&mut self, _: bool) -> Result<(), Self::Error> {
        Ok(())
    }
    fn media_session_action(&mut self, _: MediaSessionAction, _: bool) -> Result<(), Self::Error> {
        Ok(())
    }
    fn probe_media(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn begin_find(&mut self, _: u64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn find(&mut self, _: FindRequest) -> Result<(), Self::Error> {
        Ok(())
    }
    fn end_find(&mut self, _: EndFindRequest) -> Result<(), Self::Error> {
        Ok(())
    }
    fn fill_login(&mut self, _: LoginFill) -> Result<(), Self::Error> {
        Ok(())
    }
    fn traverse_history(&mut self, steps: i32) -> Result<(), Self::Error> {
        self.calls.push(format!("traverse {steps}"));
        Ok(())
    }
    fn stop(&mut self) -> Result<(), Self::Error> {
        self.calls.push("stop".to_owned());
        Ok(())
    }
    fn reload_bypassing_cache(&mut self) -> Result<(), Self::Error> {
        self.calls.push("reload_bypassing_cache".to_owned());
        Ok(())
    }
    fn read_selection(&mut self, request_id: u64) -> Result<(), Self::Error> {
        self.calls.push(format!("read_selection {request_id}"));
        Ok(())
    }
}

#[test]
fn the_four_commands_reach_the_matching_web_view_methods() {
    let mut web_view = Recorded::default();

    for command in [
        BrowserCommand::TraverseHistory { steps: -2 },
        BrowserCommand::Stop,
        BrowserCommand::ReloadBypassingCache,
        BrowserCommand::ReadSelection { request_id: 9 },
    ] {
        assert_eq!(dispatch_command(&mut web_view, command), Ok(()));
    }

    assert_eq!(
        web_view.calls,
        [
            "traverse -2",
            "stop",
            "reload_bypassing_cache",
            "read_selection 9",
        ]
    );
}

#[test]
fn a_hard_reload_held_for_a_tab_that_is_not_ready_is_dropped_by_a_new_navigation_like_reload() {
    let mut controller = BrowserController::default();
    controller.set_search_url_builder(test_search());
    controller.queue_command(BrowserCommand::ReloadBypassingCache);
    controller.queue_command(BrowserCommand::Stop);
    assert!(!controller.has_pending_commands());

    controller
        .queue_navigation("https://example.test/")
        .expect("a valid address");
    assert!(!controller.has_pending_commands());

    let tab_id = controller.active_tab_id();
    assert!(controller.take_initial_url(tab_id).is_some());
    assert!(controller.handle_event(
        tab_id,
        BrowserEvent::AddressChanged {
            url: "https://example.test/".to_owned(),
        },
    ));
    assert!(controller.has_pending_commands());

    let queued = controller
        .next_command()
        .expect("stop is not about the current page and is released");
    assert_eq!(queued.into_command(), BrowserCommand::Stop);
    assert!(controller.next_command().is_none());
}

#[test]
fn history_entries_fill_the_back_list_and_the_booleans_agree_with_history_changed() {
    let mut state = BrowserState::default();
    state.apply(history(
        &["https://a.test/", "https://b.test/", "https://c.test/"],
        1,
    ));

    assert_eq!(state.back_list().entries().len(), 3);
    assert_eq!(state.back_list().current(), 1);
    assert!(state.back_list().can_go_back());
    assert!(state.back_list().can_go_forward());

    state.apply(history(&["https://a.test/"], 0));
    assert!(!state.back_list().can_go_back());
    assert!(!state.back_list().can_go_forward());
}

#[test]
fn a_page_title_after_history_sets_the_current_entry_title() {
    let mut state = BrowserState::default();
    state.apply(history(&["https://a.test/", "https://b.test/"], 1));
    state.apply(BrowserEvent::PageTitleChanged {
        title: Some("B page".to_owned()),
    });

    assert_eq!(
        state.back_list().entries()[1].title.as_deref(),
        Some("B page")
    );
    assert_eq!(
        state.back_list().popup(Direction::Back).rows[0].label,
        "a.test/"
    );
}

#[test]
fn a_selection_read_answer_is_replaced_by_the_next_one() {
    let mut state = BrowserState::default();
    assert_eq!(state.selection_read(), None);

    state.apply(BrowserEvent::SelectionRead {
        request_id: 5,
        text: Some("x".to_owned()),
    });
    assert_eq!(state.selection_read(), Some((5, "x")));

    state.apply(BrowserEvent::SelectionRead {
        request_id: 6,
        text: Some("later".to_owned()),
    });
    assert_eq!(state.selection_read(), Some((6, "later")));
}
