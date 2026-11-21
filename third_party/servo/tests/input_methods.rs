/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! WebView input method tests
mod common;

use std::rc::Rc;

use embedder_traits::InputMethodType;
// Mercury: the clause and replacement types, the preferences and the state message of the IME
// tests (EB11).
use servo::{
    CompositionEvent, CompositionState, EmbedderControl, ImeClause, ImeClauseKind, ImeEvent,
    ImeReplacement, InputEvent, JSValue, Key, KeyState, KeyboardEvent, Preferences,
    TextInputState, WebView, WebViewBuilder,
};
use url::Url;

use crate::common::{
    ServoTest, WebViewDelegateImpl, evaluate_javascript,
    show_webview_and_wait_for_rendering_to_be_ready,
};

#[derive(Debug, Clone, PartialEq)]
enum InputMethodEvent {
    CompositionStart,
    CompositionUpdate(String),
    CompositionEnd(String),
    Input(String, Option<String>),
    BeforeInput(String, Option<String>),
}

struct InputMethodTest {
    servo_test: ServoTest,
    webview: WebView,
    delegate: Rc<WebViewDelegateImpl>,
}

impl InputMethodTest {
    fn new(body: &str) -> Self {
        Self::with_servo_test(ServoTest::new(), body)
    }

    // Mercury: a Servo with `execCommand` on, which editing hosts (contenteditable) need (EB11).
    fn new_with_editing_hosts(body: &str) -> Self {
        Self::with_servo_test(
            ServoTest::new_with_builder(|builder| builder.preferences(editing_preferences())),
            body,
        )
    }

    fn with_servo_test(servo_test: ServoTest, body: &str) -> Self {
        let delegate = Rc::new(WebViewDelegateImpl::default());
        let webview = WebViewBuilder::new(servo_test.servo(), servo_test.rendering_context.clone())
            .delegate(delegate.clone())
            .url(Url::parse(format!("data:text/html,{body}").as_str()).unwrap())
            .build();
        show_webview_and_wait_for_rendering_to_be_ready(&servo_test, &webview, &delegate);
        evaluate_javascript(
            &servo_test,
            webview.clone(),
            "window.input_method_test_received = [];",
        )
        .expect("JS evaluation should not fail");
        Self {
            servo_test,
            webview,
            delegate,
        }
    }

    fn send_input_event(&self, event: InputEvent) {
        *self.delegate.input_event_result.borrow_mut() = None;
        self.webview.notify_input_event(event);
        self.servo_test
            .spin(move || self.delegate.input_event_result.borrow().is_none());
    }

    fn focus(&self, id: &str) {
        evaluate_javascript(
            &self.servo_test,
            self.webview.clone(),
            format!("document.getElementById('{id}').focus();"),
        )
        .expect("JS evaluation should not fail");
    }

    fn install_event_listeners(&self, id: &str) {
        for event in &["beforeinput", "input"] {
            evaluate_javascript(
                &self.servo_test,
                self.webview.clone(),
                format!("document.getElementById('{id}').addEventListener('{event}', (event) => {{ \
                        window.input_method_test_received.push({{ name: '{event}', inputType: event.inputType, data: event.data }}); \
                    }});"),
            ).expect("JS evaluation should not fail");
        }
        for event in &["compositionstart", "compositionupdate", "compositionend"] {
            evaluate_javascript(
                &self.servo_test,
                self.webview.clone(),
                format!("document.getElementById('{id}').addEventListener('{event}', (event) => {{ \
                        window.input_method_test_received.push({{ name: '{event}', data: event.data }}); \
                    }});"),
            ).expect("JS evaluation should not fail");
        }
    }

    fn get_text_control_value(&self, id: &str) -> String {
        let value = evaluate_javascript(
            &self.servo_test,
            self.webview.clone(),
            format!("document.getElementById('{id}').value"),
        );
        let Ok(JSValue::String(content)) = value else {
            panic!("Should get a string from JS");
        };
        content
    }

    fn drain_received_events(&self) -> Vec<InputMethodEvent> {
        let raw_events = evaluate_javascript(
            &self.servo_test,
            self.webview.clone(),
            "let received = window.input_method_test_received; \
            window.input_method_test_received = []; \
            received",
        )
        .expect("JS evaluation should not fail");
        let js_events = match raw_events {
            JSValue::Array(values) => values,
            _ => return Vec::new(),
        };
        js_events
            .into_iter()
            .filter_map(|js_value| {
                let JSValue::Object(mut object) = js_value else {
                    panic!("Should not put non-object into window.input_method_test_received");
                };
                let Some(JSValue::String(name)) = object.remove("name") else {
                    panic!("name should be a string");
                };
                let event = match name.as_str() {
                    "beforeinput" | "input" => {
                        let Some(JSValue::String(input_type)) = object.remove("inputType") else {
                            panic!("inputType should be a string");
                        };
                        let data = match object.remove("data").unwrap() {
                            JSValue::String(data) => Some(data),
                            JSValue::Null => None,
                            _ => panic!("data should be null or a string"),
                        };
                        if name == "beforeinput" {
                            InputMethodEvent::BeforeInput(input_type, data)
                        } else {
                            InputMethodEvent::Input(input_type, data)
                        }
                    },
                    "compositionstart" => InputMethodEvent::CompositionStart,
                    "compositionupdate" | "compositionend" => {
                        let Some(JSValue::String(data)) = object.remove("data") else {
                            panic!("data should be null or a string");
                        };
                        if name == "compositionupdate" {
                            InputMethodEvent::CompositionUpdate(data)
                        } else {
                            InputMethodEvent::CompositionEnd(data)
                        }
                    },
                    _ => panic!("Shouldn't receive any other event"),
                };
                Some(event)
            })
            .collect()
    }
}

#[test]
fn test_simple_composition() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" />",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.send_input_event(InputEvent::Ime(ImeEvent::Composition(CompositionEvent {
        state: CompositionState::Start,
        data: String::new(),
    })));
    test.send_input_event(InputEvent::Ime(ImeEvent::Composition(CompositionEvent {
        state: CompositionState::Update,
        data: "abc".to_owned(),
    })));
    assert_eq!(test.get_text_control_value("my-input"), "abc");
    test.send_input_event(InputEvent::Ime(ImeEvent::Composition(CompositionEvent {
        state: CompositionState::Update,
        data: "r".to_owned(),
    })));
    assert_eq!(test.get_text_control_value("my-input"), "r");
    test.send_input_event(InputEvent::Ime(ImeEvent::Composition(CompositionEvent {
        state: CompositionState::End,
        data: "r".to_owned(),
    })));
    assert_eq!(test.get_text_control_value("my-input"), "r");
    assert_eq!(
        test.drain_received_events().as_slice(),
        &[
            InputMethodEvent::CompositionStart,
            InputMethodEvent::CompositionUpdate("abc".to_owned()),
            InputMethodEvent::BeforeInput(
                "insertCompositionText".to_owned(),
                Some("abc".to_owned())
            ),
            InputMethodEvent::Input("insertCompositionText".to_owned(), Some("abc".to_owned())),
            InputMethodEvent::CompositionUpdate("r".to_owned()),
            InputMethodEvent::BeforeInput("insertCompositionText".to_owned(), Some("r".to_owned())),
            InputMethodEvent::Input("insertCompositionText".to_owned(), Some("r".to_owned())),
            InputMethodEvent::CompositionEnd("r".to_owned()),
        ]
    );
}

#[test]
fn test_attempting_to_cancel_beforeinput_composition() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" /> \
        <script> \
            document.getElementById('my-input').addEventListener('beforeinput', (event) => { \
                event.preventDefault(); \
            }); \
        </script>",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.send_input_event(InputEvent::Keyboard(KeyboardEvent::from_state_and_key(
        KeyState::Down,
        Key::Character("a".to_owned()),
    )));
    test.send_input_event(InputEvent::Keyboard(KeyboardEvent::from_state_and_key(
        KeyState::Up,
        Key::Character("a".to_owned()),
    )));
    assert!(test.get_text_control_value("my-input").is_empty());
    test.send_input_event(InputEvent::Ime(ImeEvent::Composition(CompositionEvent {
        state: CompositionState::Start,
        data: String::new(),
    })));
    test.send_input_event(InputEvent::Ime(ImeEvent::Composition(CompositionEvent {
        state: CompositionState::Update,
        data: "abc".to_owned(),
    })));
    test.send_input_event(InputEvent::Ime(ImeEvent::Composition(CompositionEvent {
        state: CompositionState::End,
        data: "abc".to_owned(),
    })));
    assert_eq!(test.get_text_control_value("my-input"), "abc");
    assert_eq!(
        test.drain_received_events().as_slice(),
        &[
            InputMethodEvent::BeforeInput("insertText".to_owned(), Some("a".to_owned())),
            InputMethodEvent::CompositionStart,
            InputMethodEvent::CompositionUpdate("abc".to_owned()),
            InputMethodEvent::BeforeInput(
                "insertCompositionText".to_owned(),
                Some("abc".to_owned())
            ),
            InputMethodEvent::Input("insertCompositionText".to_owned(), Some("abc".to_owned())),
            InputMethodEvent::CompositionEnd("abc".to_owned()),
        ]
    );
}

#[test]
fn test_ime_multiline_for_textarea() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <textarea id=my-text-area style=\"width: 200px; height: 200px;\">servo</textarea>",
    );
    test.focus("my-text-area");
    test.servo_test
        .spin(|| test.delegate.controls_shown.borrow().is_empty());

    let controls = test.delegate.controls_shown.borrow();
    let EmbedderControl::InputMethod(ime) = &controls[0] else {
        panic!("Expected embedder control to be an InputMethod");
    };
    assert_eq!(ime.input_method_type(), InputMethodType::Text);
    assert!(ime.multiline());
}

// Mercury: backport of servo/servo#49400 (13fbe153e). Drop at the bump that has it.
#[test]
fn test_commit_without_composition() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" />",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.send_input_event(InputEvent::Ime(ImeEvent::Commit("hello".to_owned())));
    assert_eq!(test.get_text_control_value("my-input"), "hello");
    test.send_input_event(InputEvent::Ime(ImeEvent::Commit("世界".to_owned())));
    assert_eq!(test.get_text_control_value("my-input"), "hello世界");
    assert_eq!(
        test.drain_received_events().as_slice(),
        &[
            InputMethodEvent::BeforeInput("insertText".to_owned(), Some("hello".to_owned())),
            InputMethodEvent::Input("insertText".to_owned(), Some("hello".to_owned())),
            InputMethodEvent::BeforeInput("insertText".to_owned(), Some("世界".to_owned())),
            InputMethodEvent::Input("insertText".to_owned(), Some("世界".to_owned())),
        ]
    );
}

// ---- Mercury: IME sequences (EB11, the text lane) -------------------------------------------
//
// These tests drive the new `ImeEvent`s (`SetMarkedText`, `Commit`, `InsertText`) and read the
// text state messages the engine sends (`TextMsg::InputState`). They need no window: Servo runs on
// a `SoftwareRenderingContext`. What they cannot show is what a person sees and hears from the
// real application: the underline under the marked text and the thicker one under the selected
// clause, the caret inside the marked text, the candidate window at the caret, and everything the
// macOS input methods themselves do. Those are open items for a run on a Mac with a real keyboard.

fn editing_preferences() -> Preferences {
    let mut preferences = Preferences::default();
    preferences.network_http_proxy_uri = String::new();
    preferences.network_https_proxy_uri = String::new();
    preferences.dom_exec_command_enabled = true;
    preferences
}

/// `beforeinput` and `input` events carry `isComposing` and `cancelable`; the composition events
/// carry `cancelable`.
#[derive(Debug, Clone, PartialEq)]
struct EventFlagsSeen {
    name: String,
    composing: bool,
    cancelable: bool,
}

impl InputMethodTest {
    fn ime(&self, event: ImeEvent) {
        self.send_input_event(InputEvent::Ime(event));
    }

    fn set_marked(&self, text: &str) {
        self.set_marked_with(text, None, vec![], None);
    }

    fn set_marked_with(
        &self,
        text: &str,
        selected: Option<std::ops::Range<u32>>,
        clauses: Vec<ImeClause>,
        replacement: Option<ImeReplacement>,
    ) {
        self.ime(ImeEvent::SetMarkedText {
            text: text.to_owned(),
            selected,
            clauses,
            replacement,
        });
    }

    fn commit(&self, text: &str) {
        self.ime(ImeEvent::Commit(text.to_owned()));
    }

    fn insert_text(&self, text: &str, replacement: Option<ImeReplacement>) {
        self.ime(ImeEvent::InsertText {
            text: text.to_owned(),
            replacement,
        });
    }

    fn eval(&self, script: impl ToString) -> JSValue {
        evaluate_javascript(&self.servo_test, self.webview.clone(), script)
            .expect("JS evaluation should not fail")
    }

    fn eval_string(&self, script: impl ToString) -> String {
        let JSValue::String(value) = self.eval(script) else {
            panic!("Should get a string from JS");
        };
        value
    }

    fn get_text_content(&self, id: &str) -> String {
        self.eval_string(format!("document.getElementById('{id}').textContent"))
    }

    /// Focus a contenteditable element and put the caret at its end (the selection is the
    /// document's, and an editing host needs one).
    fn focus_editing_host(&self, id: &str) {
        self.eval(format!(
            "const el = document.getElementById('{id}'); el.focus(); \
             getSelection().collapse(el, el.childNodes.length);"
        ));
    }

    /// Record `isComposing` and `cancelable` of the input method events at the element.
    fn install_flag_listeners(&self, id: &str) {
        self.eval(format!(
            "window.input_method_test_flags = []; \
             for (const name of ['compositionstart', 'compositionupdate', 'compositionend', \
                                 'beforeinput', 'input']) {{ \
                 document.getElementById('{id}').addEventListener(name, (event) => {{ \
                     window.input_method_test_flags.push({{ \
                         name: name, composing: !!event.isComposing, cancelable: event.cancelable }}); \
                 }}); \
             }}"
        ));
    }

    fn drain_flags(&self) -> Vec<EventFlagsSeen> {
        let raw = self.eval(
            "let flags = window.input_method_test_flags; window.input_method_test_flags = []; flags",
        );
        let JSValue::Array(values) = raw else {
            return Vec::new();
        };
        values
            .into_iter()
            .map(|value| {
                let JSValue::Object(mut object) = value else {
                    panic!("flags should be objects");
                };
                let Some(JSValue::String(name)) = object.remove("name") else {
                    panic!("name should be a string");
                };
                let Some(JSValue::Boolean(composing)) = object.remove("composing") else {
                    panic!("composing should be a boolean");
                };
                let Some(JSValue::Boolean(cancelable)) = object.remove("cancelable") else {
                    panic!("cancelable should be a boolean");
                };
                EventFlagsSeen {
                    name,
                    composing,
                    cancelable,
                }
            })
            .collect()
    }

    /// Spin the event loop until `done` (or fail after ten seconds instead of hanging).
    fn wait_until(&self, what: &str, done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        self.servo_test.spin(|| {
            assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
            !done()
        });
    }

    fn wait_for_state_messages(&self, count: usize) {
        self.wait_until("the text state messages", || {
            self.delegate.text_states.borrow().len() >= count
        });
    }

    fn state_messages(&self) -> Vec<Option<TextInputState>> {
        self.delegate.text_states.borrow().clone()
    }

    /// The last text state message that carries a state (not the "nothing editable" one).
    fn last_state(&self) -> TextInputState {
        self.state_messages()
            .into_iter()
            .rev()
            .flatten()
            .next()
            .expect("a text state message with a state")
    }

    fn key(&self, key: Key) {
        self.send_input_event(InputEvent::Keyboard(KeyboardEvent::from_state_and_key(
            KeyState::Down,
            key.clone(),
        )));
        self.send_input_event(InputEvent::Keyboard(KeyboardEvent::from_state_and_key(
            KeyState::Up,
            key,
        )));
    }
}

fn compositionupdate(text: &str) -> InputMethodEvent {
    InputMethodEvent::CompositionUpdate(text.to_owned())
}

fn compositionend(text: &str) -> InputMethodEvent {
    InputMethodEvent::CompositionEnd(text.to_owned())
}

/// The events of one update of the marked text, as the engine fires them (D5).
fn update_events(text: &str) -> Vec<InputMethodEvent> {
    vec![
        compositionupdate(text),
        InputMethodEvent::BeforeInput("insertCompositionText".to_owned(), Some(text.to_owned())),
        InputMethodEvent::Input("insertCompositionText".to_owned(), Some(text.to_owned())),
    ]
}

/// The events of a Pinyin sequence `n`, `ni`, then the commit of `你`.
fn pinyin_events() -> Vec<InputMethodEvent> {
    let mut events = vec![InputMethodEvent::CompositionStart];
    events.extend(update_events("n"));
    events.extend(update_events("ni"));
    // The commit replaces the marked text (`beforeinput` and `input`), then the composition ends.
    events.push(InputMethodEvent::BeforeInput(
        "insertCompositionText".to_owned(),
        Some("你".to_owned()),
    ));
    events.push(InputMethodEvent::Input(
        "insertCompositionText".to_owned(),
        Some("你".to_owned()),
    ));
    events.push(compositionend("你"));
    events
}

fn count_composition_ends(events: &[InputMethodEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, InputMethodEvent::CompositionEnd(_)))
        .count()
}

#[test]
fn test_pinyin_in_input() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" />",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.set_marked("n");
    assert_eq!(test.get_text_control_value("my-input"), "n");
    test.set_marked("ni");
    assert_eq!(test.get_text_control_value("my-input"), "ni");
    test.commit("你");
    assert_eq!(test.get_text_control_value("my-input"), "你");
    let events = test.drain_received_events();
    assert_eq!(events, pinyin_events());
    assert_eq!(count_composition_ends(&events), 1);
}

#[test]
fn test_pinyin_in_textarea() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <textarea id=\"my-textarea\" style=\"width: 200px; height: 100px;\"></textarea>",
    );
    test.install_event_listeners("my-textarea");
    test.focus("my-textarea");
    test.set_marked("n");
    test.set_marked("ni");
    assert_eq!(test.get_text_control_value("my-textarea"), "ni");
    test.commit("你");
    assert_eq!(test.get_text_control_value("my-textarea"), "你");
    let events = test.drain_received_events();
    assert_eq!(events, pinyin_events());
    assert_eq!(count_composition_ends(&events), 1);
}

#[test]
fn test_pinyin_in_contenteditable() {
    let test = InputMethodTest::new_with_editing_hosts(
        "<!DOCTYPE html> \
        <div id=\"my-editable\" contenteditable style=\"width: 200px; height: 50px;\"></div>",
    );
    test.install_event_listeners("my-editable");
    test.focus_editing_host("my-editable");
    test.set_marked("n");
    assert_eq!(test.get_text_content("my-editable"), "n");
    test.set_marked("ni");
    assert_eq!(test.get_text_content("my-editable"), "ni");
    // The state message follows the marked text of the editing host.
    let state = test.last_state();
    assert_eq!(state.text, "ni");
    assert_eq!(state.marked, Some(0..2));
    assert!(state.multiline);
    test.commit("你");
    assert_eq!(test.get_text_content("my-editable"), "你");
    let events = test.drain_received_events();
    assert_eq!(events, pinyin_events());
    assert_eq!(count_composition_ends(&events), 1);
    assert_eq!(test.last_state().marked, None);
}

#[test]
fn test_composition_events_are_composing_and_not_cancelable() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" />",
    );
    test.install_flag_listeners("my-input");
    test.focus("my-input");
    test.set_marked("ni");
    test.commit("你");
    let flag = |name: &str, composing: bool, cancelable: bool| EventFlagsSeen {
        name: name.to_owned(),
        composing,
        cancelable,
    };
    assert_eq!(
        test.drain_flags(),
        vec![
            // `compositionstart` is the one cancelable composition event.
            flag("compositionstart", false, true),
            flag("compositionupdate", false, false),
            // `beforeinput` of a composition is not cancelable; both are composing.
            flag("beforeinput", true, false),
            flag("input", true, false),
            flag("beforeinput", true, false),
            flag("input", true, false),
            flag("compositionend", false, false),
        ]
    );
}

#[test]
fn test_japanese_two_clauses_with_a_selected_clause() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" />",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    // The first clause is converted, the second is the one the user works on.
    test.set_marked_with(
        "にほんご",
        Some(2..4),
        vec![
            ImeClause {
                range: 0..2,
                kind: ImeClauseKind::Converted,
            },
            ImeClause {
                range: 2..4,
                kind: ImeClauseKind::SelectedConverted,
            },
        ],
        None,
    );
    assert_eq!(test.get_text_control_value("my-input"), "にほんご");
    // The marked text is not the selection: the caret is at the start of the selected clause.
    let state = test.last_state();
    assert_eq!(state.text, "にほんご");
    assert_eq!(state.marked, Some(0..4));
    assert_eq!(state.selection, 2..2);

    test.set_marked_with(
        "日本語",
        Some(0..3),
        vec![ImeClause {
            range: 0..3,
            kind: ImeClauseKind::SelectedConverted,
        }],
        None,
    );
    assert_eq!(test.get_text_control_value("my-input"), "日本語");
    assert_eq!(test.last_state().marked, Some(0..3));
    test.commit("日本語");
    assert_eq!(test.get_text_control_value("my-input"), "日本語");

    let mut expected = vec![InputMethodEvent::CompositionStart];
    expected.extend(update_events("にほんご"));
    expected.extend(update_events("日本語"));
    expected.push(InputMethodEvent::BeforeInput(
        "insertCompositionText".to_owned(),
        Some("日本語".to_owned()),
    ));
    expected.push(InputMethodEvent::Input(
        "insertCompositionText".to_owned(),
        Some("日本語".to_owned()),
    ));
    expected.push(compositionend("日本語"));
    assert_eq!(test.drain_received_events(), expected);
}

#[test]
fn test_korean_replaces_the_marked_text_in_place() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" value=\"abc\" />",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.eval("document.getElementById('my-input').setSelectionRange(1, 1);");
    for (marked, value) in [("ㅎ", "aㅎbc"), ("하", "a하bc"), ("한", "a한bc")] {
        test.set_marked(marked);
        assert_eq!(test.get_text_control_value("my-input"), value);
    }
    // The next syllable starts after the one that is finished.
    test.commit("한");
    test.set_marked("ㄱ");
    assert_eq!(test.get_text_control_value("my-input"), "a한ㄱbc");
    let starts = test
        .drain_received_events()
        .into_iter()
        .filter(|event| *event == InputMethodEvent::CompositionStart)
        .count();
    assert_eq!(starts, 2);
}

#[test]
fn test_dead_key() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" />",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.set_marked("´");
    assert_eq!(test.get_text_control_value("my-input"), "´");
    test.commit("é");
    assert_eq!(test.get_text_control_value("my-input"), "é");
    let mut expected = vec![InputMethodEvent::CompositionStart];
    expected.extend(update_events("´"));
    expected.push(InputMethodEvent::BeforeInput(
        "insertCompositionText".to_owned(),
        Some("é".to_owned()),
    ));
    expected.push(InputMethodEvent::Input(
        "insertCompositionText".to_owned(),
        Some("é".to_owned()),
    ));
    expected.push(compositionend("é"));
    assert_eq!(test.drain_received_events(), expected);
}

#[test]
fn test_an_empty_commit_cancels_the_composition() {
    for body in [
        "<input id=\"my-input\" />",
        "<textarea id=\"my-input\"></textarea>",
    ] {
        let test = InputMethodTest::new(&format!("<!DOCTYPE html> {body}"));
        test.install_event_listeners("my-input");
        test.focus("my-input");
        test.set_marked("abc");
        test.commit("");
        assert_eq!(test.get_text_control_value("my-input"), "");
        let events = test.drain_received_events();
        // The marked text is removed (the page sees an `input` with empty data), and the
        // composition ends with empty data, once.
        assert_eq!(
            &events[events.len() - 3..],
            &[
                InputMethodEvent::BeforeInput("insertCompositionText".to_owned(), Some(String::new())),
                InputMethodEvent::Input("insertCompositionText".to_owned(), Some(String::new())),
                InputMethodEvent::CompositionEnd(String::new()),
            ]
        );
        assert_eq!(count_composition_ends(&events), 1);
    }
}

#[test]
fn test_empty_marked_text_ends_the_composition_too() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" />",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.set_marked("a");
    test.set_marked("");
    assert_eq!(test.get_text_control_value("my-input"), "");
    let events = test.drain_received_events();
    assert_eq!(count_composition_ends(&events), 1);
    // Empty marked text without a composition does nothing.
    test.set_marked("");
    assert!(test.drain_received_events().is_empty());
}

#[test]
fn test_dismissed_ends_the_composition_and_keeps_the_text() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" />",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.set_marked("abc");
    test.ime(ImeEvent::Dismissed);
    assert_eq!(test.get_text_control_value("my-input"), "abc");
    assert_eq!(
        test.eval_string("document.activeElement.id"),
        "",
        "Dismissed moves focus to the viewport"
    );
    let events = test.drain_received_events();
    assert_eq!(count_composition_ends(&events), 1);
    assert_eq!(
        events.last(),
        Some(&InputMethodEvent::CompositionEnd("abc".to_owned()))
    );
}

#[test]
fn test_insert_text_without_a_composition() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" />",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.insert_text("😀", None);
    assert_eq!(test.get_text_control_value("my-input"), "😀");
    assert_eq!(
        test.drain_received_events(),
        vec![
            InputMethodEvent::BeforeInput("insertText".to_owned(), Some("😀".to_owned())),
            InputMethodEvent::Input("insertText".to_owned(), Some("😀".to_owned())),
        ]
    );
}

#[test]
fn test_insert_text_in_contenteditable() {
    let test = InputMethodTest::new_with_editing_hosts(
        "<!DOCTYPE html> \
        <div id=\"my-editable\" contenteditable></div>",
    );
    test.install_event_listeners("my-editable");
    test.focus_editing_host("my-editable");
    test.insert_text("😀", None);
    assert_eq!(test.get_text_content("my-editable"), "😀");
    assert_eq!(
        test.drain_received_events(),
        vec![
            InputMethodEvent::BeforeInput("insertText".to_owned(), Some("😀".to_owned())),
            InputMethodEvent::Input("insertText".to_owned(), Some("😀".to_owned())),
        ]
    );
}

#[test]
fn test_insert_text_with_a_replacement_for_the_current_sequence() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" value=\"cafe\" />",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.wait_for_state_messages(1);
    let state = test.last_state();
    assert_eq!(state.text, "cafe");
    // The accent menu replaces the `e`.
    test.insert_text(
        "é",
        Some(ImeReplacement {
            range: 3..4,
            sequence: state.sequence,
        }),
    );
    assert_eq!(test.get_text_control_value("my-input"), "café");
    assert_eq!(
        test.drain_received_events(),
        vec![
            InputMethodEvent::BeforeInput("insertReplacementText".to_owned(), Some("é".to_owned())),
            InputMethodEvent::Input("insertReplacementText".to_owned(), Some("é".to_owned())),
        ]
    );
}

#[test]
fn test_insert_text_with_a_replacement_for_a_stale_sequence() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" value=\"cafe\" />",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.wait_for_state_messages(1);
    let first = test.last_state();
    test.eval("document.getElementById('my-input').setSelectionRange(4, 4);");
    test.insert_text("!", None);
    // A message for the new text was sent, so the old sequence is stale.
    assert!(test.last_state().sequence > first.sequence);
    test.drain_received_events();
    test.insert_text(
        "x",
        Some(ImeReplacement {
            range: 0..1,
            sequence: first.sequence,
        }),
    );
    // Not replaced: inserted at the selection, as plain text.
    assert_eq!(test.get_text_control_value("my-input"), "cafe!x");
    assert_eq!(
        test.drain_received_events()[0],
        InputMethodEvent::BeforeInput("insertText".to_owned(), Some("x".to_owned()))
    );
}

#[test]
fn test_a_replacement_is_ignored_when_the_text_changed_since_the_message() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" value=\"cafe\" />",
    );
    test.focus("my-input");
    test.wait_for_state_messages(1);
    let state = test.last_state();
    // A script changes the text without any input event.
    test.eval("document.getElementById('my-input').value = 'tea';");
    test.insert_text(
        "é",
        Some(ImeReplacement {
            range: 3..4,
            sequence: state.sequence,
        }),
    );
    assert_eq!(test.get_text_control_value("my-input"), "teaé");
}

#[test]
fn test_a_replacement_out_of_range_or_inside_a_surrogate_pair_does_not_crash() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" value=\"a😀b\" />",
    );
    test.focus("my-input");
    test.wait_for_state_messages(1);
    test.eval("document.getElementById('my-input').setSelectionRange(4, 4);");
    let sequence = test.last_state().sequence;
    // Past the end of the text.
    test.insert_text(
        "x",
        Some(ImeReplacement {
            range: 0..99,
            sequence,
        }),
    );
    assert_eq!(test.get_text_control_value("my-input"), "a😀bx");
    // Inside the surrogate pair of the emoji (the state sent after the insertion is the one to name).
    let sequence = test.last_state().sequence;
    test.insert_text(
        "y",
        Some(ImeReplacement {
            range: 2..3,
            sequence,
        }),
    );
    assert_eq!(test.get_text_control_value("my-input"), "a😀bxy");
    // The same for marked text, and a reversed range.
    let sequence = test.last_state().sequence;
    test.set_marked_with(
        "z",
        None,
        vec![],
        Some(ImeReplacement {
            range: 3..1,
            sequence,
        }),
    );
    assert_eq!(test.get_text_control_value("my-input"), "a😀bxyz");
    // Servo is still alive.
    assert_eq!(test.eval_string("'alive'"), "alive");
}

#[test]
fn test_beforeinput_prevent_default_does_not_stop_a_composition() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" /> \
        <script> \
            document.getElementById('my-input').addEventListener('beforeinput', (event) => { \
                event.preventDefault(); \
            }); \
        </script>",
    );
    test.install_event_listeners("my-input");
    test.focus("my-input");
    test.set_marked("ni");
    test.commit("你");
    // `beforeinput` is not cancelable during a composition, so the text went in.
    assert_eq!(test.get_text_control_value("my-input"), "你");
    // Plain insertion is cancelable, and the page canceled it.
    test.insert_text("x", None);
    assert_eq!(test.get_text_control_value("my-input"), "你");
}

#[test]
fn test_text_state_messages_for_a_text_control() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" />",
    );
    test.focus("my-input");
    // On focus: the empty control, and where its caret is.
    test.wait_for_state_messages(1);
    let focused = test.last_state();
    assert_eq!(focused.text, "");
    assert_eq!(focused.selection, 0..0);
    assert_eq!(focused.marked, None);
    assert!(!focused.multiline);
    assert!(!focused.is_password);
    assert!(
        focused.caret_rect.is_some(),
        "a newly focused editable gets a caret rectangle"
    );

    // Typing: the text and the selection.
    test.key(Key::Character("a".to_owned()));
    let typed = test.last_state();
    assert_eq!(typed.text, "a");
    assert_eq!(typed.selection, 1..1);
    assert!(typed.sequence > focused.sequence);

    // A selection change without a text change.
    test.key(Key::Named(servo::NamedKey::ArrowLeft));
    let moved = test.last_state();
    assert_eq!(moved.text, "a");
    assert_eq!(moved.selection, 0..0);

    // An event that changes nothing sends nothing.
    let count = test.state_messages().len();
    test.eval("1 + 1");
    test.send_input_event(InputEvent::Keyboard(KeyboardEvent::from_state_and_key(
        KeyState::Up,
        Key::Named(servo::NamedKey::Shift),
    )));
    assert_eq!(test.state_messages().len(), count);

    // The composition shows in the state.
    test.set_marked("ni");
    let composing = test.last_state();
    assert_eq!(composing.text, "ani");
    assert_eq!(composing.marked, Some(0..2));

    // On blur: nothing editable has focus.
    test.eval("document.getElementById('my-input').blur();");
    test.wait_until("the message that nothing is focused", || {
        test.state_messages().last() == Some(&None)
    });
}

#[test]
fn test_a_password_field_sends_no_text() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <input id=\"my-input\" type=\"password\" value=\"hunter2\" />",
    );
    test.focus("my-input");
    test.wait_for_state_messages(1);
    let state = test.last_state();
    assert!(state.is_password);
    assert_eq!(state.text, "");
    assert_eq!(state.selection, 0..0);
    assert_eq!(state.marked, None);
    // Typing does not change that.
    test.key(Key::Character("x".to_owned()));
    let typed = test.last_state();
    assert!(typed.is_password);
    assert_eq!(typed.text, "");
}

#[test]
fn test_a_long_text_sends_a_window_of_4096_units() {
    let test = InputMethodTest::new(
        "<!DOCTYPE html> \
        <textarea id=\"my-input\" style=\"width: 200px; height: 100px;\"></textarea>",
    );
    test.eval(
        "const area = document.getElementById('my-input'); \
         area.value = 'x'.repeat(10000); area.setSelectionRange(10000, 10000); area.focus();",
    );
    test.wait_for_state_messages(1);
    let state = test.last_state();
    assert!(state.multiline);
    assert_eq!(state.text.encode_utf16().count(), 4096);
    // The caret is at the end of the text; the window ends there.
    assert_eq!(state.text_start, 10_000 - 4096);
    assert_eq!(state.selection, 4096..4096);
}

#[test]
fn test_the_old_composition_events_still_work_in_contenteditable() {
    let test = InputMethodTest::new_with_editing_hosts(
        "<!DOCTYPE html> \
        <div id=\"my-editable\" contenteditable></div>",
    );
    test.install_event_listeners("my-editable");
    test.focus_editing_host("my-editable");
    for (state, data) in [
        (CompositionState::Start, ""),
        (CompositionState::Update, "ab"),
        (CompositionState::Update, "c"),
        (CompositionState::End, "c"),
    ] {
        test.send_input_event(InputEvent::Ime(ImeEvent::Composition(CompositionEvent {
            state,
            data: data.to_owned(),
        })));
    }
    assert_eq!(test.get_text_content("my-editable"), "c");
    let events = test.drain_received_events();
    assert_eq!(count_composition_ends(&events), 1);
    assert_eq!(events.first(), Some(&InputMethodEvent::CompositionStart));
}
