// SPDX-License-Identifier: MPL-2.0
use talaria_core::{
    BrowserCommand, ClipboardCommand, ColorScheme, CompositionState, ControlAnswer, ControlId,
    EndFindRequest, FindRequest, FindStep, ImeInput, KeyLocation, KeyState, KeyboardInput,
    KeyboardModifiers, PointerButton, PointerButtonAction, WebView, dispatch_command,
};
// media session
use talaria_core::MediaSessionAction;
use talaria_core::{PageRequestAnswer, PageRequestId};
// passwords
use talaria_core::LoginFill;

#[derive(Debug, PartialEq)]
enum FakeCall {
    Navigate(String),
    GoBack,
    GoForward,
    Reload,
    PointerMoved {
        x: f32,
        y: f32,
    },
    PointerButton {
        action: PointerButtonAction,
        button: PointerButton,
        x: f32,
        y: f32,
    },
    Scrolled {
        delta_x: f32,
        delta_y: f32,
        x: f32,
        y: f32,
    },
    FocusChanged(bool),
    KeyboardInput(KeyboardInput),
    ImeInput(ImeInput),
    SetPageZoom(u16),
    SetColorScheme(ColorScheme),
    ExitFullscreen,
    PointerLeft,
    PinchZoom {
        delta: f32,
        x: f32,
        y: f32,
    },
    AnswerControl(ControlId, ControlAnswer),
    FillInput(ControlId, String),
    // dialogs
    AnswerPageRequest(PageRequestId, PageRequestAnswer),
    // menus
    Clipboard(ClipboardCommand),
    // media session
    SetAudioMuted(bool),
    MediaSessionAction(MediaSessionAction, bool),
    ProbeMedia,
    // find
    BeginFind(u64),
    Find(FindRequest),
    EndFind(EndFindRequest),
    // passwords
    FillLogin(LoginFill),
}

#[derive(Debug, PartialEq, Eq)]
enum FakeError {
    CommandRejected,
}

#[derive(Debug, Default)]
struct FakeWebView {
    calls: Vec<FakeCall>,
    next_error: Option<FakeError>,
}

impl FakeWebView {
    fn reject_next_command(&mut self) {
        self.next_error = Some(FakeError::CommandRejected);
    }

    fn record(&mut self, call: FakeCall) -> Result<(), FakeError> {
        self.calls.push(call);

        match self.next_error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl WebView for FakeWebView {
    type Error = FakeError;

    fn navigate(&mut self, url: &str) -> Result<(), Self::Error> {
        self.record(FakeCall::Navigate(url.to_owned()))
    }

    fn go_back(&mut self) -> Result<(), Self::Error> {
        self.record(FakeCall::GoBack)
    }

    fn go_forward(&mut self) -> Result<(), Self::Error> {
        self.record(FakeCall::GoForward)
    }

    fn reload(&mut self) -> Result<(), Self::Error> {
        self.record(FakeCall::Reload)
    }

    fn pointer_moved(&mut self, x: f32, y: f32) -> Result<(), Self::Error> {
        self.record(FakeCall::PointerMoved { x, y })
    }

    fn pointer_button(
        &mut self,
        action: PointerButtonAction,
        button: PointerButton,
        x: f32,
        y: f32,
    ) -> Result<(), Self::Error> {
        self.record(FakeCall::PointerButton {
            action,
            button,
            x,
            y,
        })
    }

    fn scroll(&mut self, delta_x: f32, delta_y: f32, x: f32, y: f32) -> Result<(), Self::Error> {
        self.record(FakeCall::Scrolled {
            delta_x,
            delta_y,
            x,
            y,
        })
    }

    fn set_focus(&mut self, focused: bool) -> Result<(), Self::Error> {
        self.record(FakeCall::FocusChanged(focused))
    }

    fn keyboard_input(&mut self, event: KeyboardInput) -> Result<(), Self::Error> {
        self.record(FakeCall::KeyboardInput(event))
    }

    fn ime_input(&mut self, event: ImeInput) -> Result<(), Self::Error> {
        self.record(FakeCall::ImeInput(event))
    }

    fn set_page_zoom(&mut self, percent: u16) -> Result<(), Self::Error> {
        self.record(FakeCall::SetPageZoom(percent))
    }

    fn pinch_zoom(&mut self, delta: f32, x: f32, y: f32) -> Result<(), Self::Error> {
        self.record(FakeCall::PinchZoom { delta, x, y })
    }

    fn set_color_scheme(&mut self, scheme: ColorScheme) -> Result<(), Self::Error> {
        self.record(FakeCall::SetColorScheme(scheme))
    }

    fn exit_fullscreen(&mut self) -> Result<(), Self::Error> {
        self.record(FakeCall::ExitFullscreen)
    }

    fn pointer_left(&mut self) -> Result<(), Self::Error> {
        self.record(FakeCall::PointerLeft)
    }

    fn answer_control(&mut self, id: ControlId, answer: ControlAnswer) -> Result<(), Self::Error> {
        self.record(FakeCall::AnswerControl(id, answer))
    }

    fn fill_input(&mut self, id: ControlId, text: String) -> Result<(), Self::Error> {
        self.record(FakeCall::FillInput(id, text))
    }

    // dialogs
    fn answer_page_request(
        &mut self,
        id: PageRequestId,
        answer: PageRequestAnswer,
    ) -> Result<(), Self::Error> {
        self.record(FakeCall::AnswerPageRequest(id, answer))
    }

    // menus
    fn clipboard(&mut self, command: ClipboardCommand) -> Result<(), Self::Error> {
        self.record(FakeCall::Clipboard(command))
    }

    // media session
    fn set_audio_muted(&mut self, muted: bool) -> Result<(), Self::Error> {
        self.record(FakeCall::SetAudioMuted(muted))
    }

    fn media_session_action(
        &mut self,
        action: MediaSessionAction,
        through_engine: bool,
    ) -> Result<(), Self::Error> {
        self.record(FakeCall::MediaSessionAction(action, through_engine))
    }

    fn probe_media(&mut self) -> Result<(), Self::Error> {
        self.record(FakeCall::ProbeMedia)
    }

    // find
    fn begin_find(&mut self, request_id: u64) -> Result<(), Self::Error> {
        self.record(FakeCall::BeginFind(request_id))
    }

    fn find(&mut self, request: FindRequest) -> Result<(), Self::Error> {
        self.record(FakeCall::Find(request))
    }

    fn end_find(&mut self, request: EndFindRequest) -> Result<(), Self::Error> {
        self.record(FakeCall::EndFind(request))
    }

    // passwords
    fn fill_login(&mut self, fill: LoginFill) -> Result<(), Self::Error> {
        self.record(FakeCall::FillLogin(fill))
    }
}

#[test]
fn navigate_command_forwards_the_url() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(
        &mut web_view,
        BrowserCommand::Navigate {
            url: "https://example.com".to_owned(),
        },
    );

    assert_eq!(result, Ok(()));
    assert_eq!(
        web_view.calls,
        vec![FakeCall::Navigate("https://example.com".to_owned())]
    );
}

#[test]
fn go_back_command_calls_go_back() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(&mut web_view, BrowserCommand::GoBack);

    assert_eq!(result, Ok(()));
    assert_eq!(web_view.calls, vec![FakeCall::GoBack]);
}

#[test]
fn go_forward_command_calls_go_forward() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(&mut web_view, BrowserCommand::GoForward);

    assert_eq!(result, Ok(()));
    assert_eq!(web_view.calls, vec![FakeCall::GoForward]);
}

#[test]
fn reload_command_calls_reload() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(&mut web_view, BrowserCommand::Reload);

    assert_eq!(result, Ok(()));
    assert_eq!(web_view.calls, vec![FakeCall::Reload]);
}

#[test]
fn pointer_moved_command_forwards_logical_coordinates() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(
        &mut web_view,
        BrowserCommand::PointerMoved { x: 12.5, y: 24.0 },
    );

    assert_eq!(result, Ok(()));
    assert_eq!(
        web_view.calls,
        vec![FakeCall::PointerMoved { x: 12.5, y: 24.0 }]
    );
}

#[test]
fn pointer_button_command_forwards_action_button_and_coordinates() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(
        &mut web_view,
        BrowserCommand::PointerButton {
            action: PointerButtonAction::Down,
            button: PointerButton::Right,
            x: 30.0,
            y: 45.0,
        },
    );

    assert_eq!(result, Ok(()));
    assert_eq!(
        web_view.calls,
        vec![FakeCall::PointerButton {
            action: PointerButtonAction::Down,
            button: PointerButton::Right,
            x: 30.0,
            y: 45.0,
        }]
    );
}

#[test]
fn scrolled_command_forwards_delta_and_pointer_coordinates() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(
        &mut web_view,
        BrowserCommand::Scrolled {
            delta_x: -2.0,
            delta_y: 18.0,
            x: 100.0,
            y: 200.0,
        },
    );

    assert_eq!(result, Ok(()));
    assert_eq!(
        web_view.calls,
        vec![FakeCall::Scrolled {
            delta_x: -2.0,
            delta_y: 18.0,
            x: 100.0,
            y: 200.0,
        }]
    );
}

#[test]
fn focus_changed_command_forwards_focus_state() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(
        &mut web_view,
        BrowserCommand::FocusChanged { focused: true },
    );

    assert_eq!(result, Ok(()));
    assert_eq!(web_view.calls, vec![FakeCall::FocusChanged(true)]);
}

#[test]
fn keyboard_command_preserves_physical_and_logical_key_data() {
    let mut web_view = FakeWebView::default();
    let event = KeyboardInput {
        state: KeyState::Down,
        key: "ç".to_owned(),
        code: "Semicolon".to_owned(),
        location: KeyLocation::Standard,
        modifiers: KeyboardModifiers {
            shift: true,
            ..KeyboardModifiers::default()
        },
        repeat: false,
        is_composing: false,
    };

    let result = dispatch_command(&mut web_view, BrowserCommand::KeyboardInput(event.clone()));

    assert_eq!(result, Ok(()));
    assert_eq!(web_view.calls, vec![FakeCall::KeyboardInput(event)]);
}

#[test]
fn ime_command_preserves_composition_state_and_text() {
    let mut web_view = FakeWebView::default();
    let event = ImeInput::Composition {
        state: CompositionState::Update,
        data: "ação".to_owned(),
    };

    let result = dispatch_command(&mut web_view, BrowserCommand::ImeInput(event.clone()));

    assert_eq!(result, Ok(()));
    assert_eq!(web_view.calls, vec![FakeCall::ImeInput(event)]);
}

#[test]
fn web_view_errors_are_returned_without_conversion() {
    let mut web_view = FakeWebView::default();
    web_view.reject_next_command();

    let result = dispatch_command(&mut web_view, BrowserCommand::Reload);

    assert_eq!(result, Err(FakeError::CommandRejected));
    assert_eq!(web_view.calls, vec![FakeCall::Reload]);
}

#[test]
fn set_page_zoom_command_forwards_the_percentage() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(&mut web_view, BrowserCommand::SetPageZoom { percent: 133 });

    assert_eq!(result, Ok(()));
    assert_eq!(web_view.calls, vec![FakeCall::SetPageZoom(133)]);
}

#[test]
fn pinch_zoom_command_forwards_delta_and_centre() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(
        &mut web_view,
        BrowserCommand::PinchZoom {
            delta: 1.25,
            x: 40.0,
            y: 60.0,
        },
    );

    assert_eq!(result, Ok(()));
    assert_eq!(
        web_view.calls,
        vec![FakeCall::PinchZoom {
            delta: 1.25,
            x: 40.0,
            y: 60.0,
        }]
    );
}

#[test]
fn set_color_scheme_command_forwards_the_scheme() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(
        &mut web_view,
        BrowserCommand::SetColorScheme(ColorScheme::Dark),
    );

    assert_eq!(result, Ok(()));
    assert_eq!(
        web_view.calls,
        vec![FakeCall::SetColorScheme(ColorScheme::Dark)]
    );
}

#[test]
fn exit_fullscreen_command_calls_exit_fullscreen() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(&mut web_view, BrowserCommand::ExitFullscreen);

    assert_eq!(result, Ok(()));
    assert_eq!(web_view.calls, vec![FakeCall::ExitFullscreen]);
}

#[test]
fn pointer_left_command_calls_pointer_left() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(&mut web_view, BrowserCommand::PointerLeft);

    assert_eq!(result, Ok(()));
    assert_eq!(web_view.calls, vec![FakeCall::PointerLeft]);
}

#[test]
fn answer_control_command_forwards_the_id_and_answer() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(
        &mut web_view,
        BrowserCommand::AnswerControl {
            id: ControlId::new(3),
            answer: ControlAnswer::Select(vec![2]),
        },
    );

    assert_eq!(result, Ok(()));
    assert_eq!(
        web_view.calls,
        vec![FakeCall::AnswerControl(
            ControlId::new(3),
            ControlAnswer::Select(vec![2])
        )]
    );
}

#[test]
fn answer_page_request_command_forwards_the_id_and_answer() {
    let mut web_view = FakeWebView::default();

    let result = dispatch_command(
        &mut web_view,
        BrowserCommand::AnswerPageRequest {
            id: PageRequestId::new(7),
            answer: PageRequestAnswer::PromptText("typed".to_owned()),
        },
    );

    assert_eq!(result, Ok(()));
    assert_eq!(
        web_view.calls,
        vec![FakeCall::AnswerPageRequest(
            PageRequestId::new(7),
            PageRequestAnswer::PromptText("typed".to_owned())
        )]
    );
}

#[test]
fn clipboard_commands_reach_the_page_editing() {
    for command in [
        ClipboardCommand::Cut,
        ClipboardCommand::Copy,
        ClipboardCommand::Paste,
    ] {
        let mut web_view = FakeWebView::default();

        let result = dispatch_command(&mut web_view, BrowserCommand::Clipboard(command));

        assert_eq!(result, Ok(()));
        assert_eq!(web_view.calls, vec![FakeCall::Clipboard(command)]);
    }
}

#[test]
fn media_session_commands_reach_the_web_view() {
    let mut web_view = FakeWebView::default();

    for command in [
        BrowserCommand::SetAudioMuted { muted: true },
        BrowserCommand::SetAudioMuted { muted: false },
        BrowserCommand::MediaSessionAction {
            action: MediaSessionAction::Pause,
            through_engine: false,
        },
        BrowserCommand::MediaSessionAction {
            action: MediaSessionAction::NextTrack,
            through_engine: true,
        },
        BrowserCommand::ProbeMedia,
    ] {
        assert_eq!(dispatch_command(&mut web_view, command), Ok(()));
    }

    assert_eq!(
        web_view.calls,
        vec![
            FakeCall::SetAudioMuted(true),
            FakeCall::SetAudioMuted(false),
            FakeCall::MediaSessionAction(MediaSessionAction::Pause, false),
            FakeCall::MediaSessionAction(MediaSessionAction::NextTrack, true),
            FakeCall::ProbeMedia,
        ]
    );
}

#[test]
fn find_commands_reach_the_web_view() {
    let mut web_view = FakeWebView::default();
    let request = FindRequest {
        id: 2,
        query: "needle".to_owned(),
        match_case: true,
        step: FindStep::Previous,
        repeat: 2,
        thorough: true,
        current: Some(3),
    };
    let end = EndFindRequest {
        query: "needle".to_owned(),
        match_case: true,
        restore_selection: None,
        caret_left: true,
    };

    for command in [
        BrowserCommand::BeginFind { request_id: 1 },
        BrowserCommand::Find(request.clone()),
        BrowserCommand::EndFind(end.clone()),
    ] {
        assert_eq!(dispatch_command(&mut web_view, command), Ok(()));
    }

    assert_eq!(
        web_view.calls,
        vec![
            FakeCall::BeginFind(1),
            FakeCall::Find(request),
            FakeCall::EndFind(end),
        ]
    );
}

// passwords
#[test]
fn a_login_fill_reaches_the_web_view_unchanged() {
    let mut web_view = FakeWebView::default();
    let fill = LoginFill::new(
        "https://example.com".to_owned(),
        "alice".to_owned(),
        "fake-pw".to_owned(),
    );

    let result = dispatch_command(&mut web_view, BrowserCommand::FillLogin(fill.clone()));

    assert_eq!(result, Ok(()));
    assert_eq!(web_view.calls, vec![FakeCall::FillLogin(fill)]);
}
