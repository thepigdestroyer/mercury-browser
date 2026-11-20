// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! IME input and the focused editable's text state (txt-1 lane).

use talaria_core::{
    BrowserCommand, BrowserController, BrowserEvent, ControlId, ControlRect, ImeClause,
    ImeClauseKind, ImeInput, ImeReplacement, LoadState, TabCommand, TabId, TextInputState,
    Utf16Range, utf8_offset_of_utf16, utf16_len, utf16_offset_of_byte,
};

const CARET: ControlRect = ControlRect {
    x: 40,
    y: 100,
    width: 2,
    height: 18,
};

fn state(sequence: u64, text: &str) -> TextInputState {
    let end = utf16_len(text);
    TextInputState {
        sequence,
        text: text.to_owned(),
        text_start: 0,
        selection: Utf16Range::collapsed(end),
        marked: None,
        caret_rect: Some(CARET),
        multiline: false,
        is_password: false,
    }
}

fn ready_controller() -> (BrowserController, TabId) {
    let mut controller = BrowserController::default();
    let tab_id = controller.active_tab_id();
    controller.handle_event(tab_id, BrowserEvent::AddressChanged { url: String::new() });
    (controller, tab_id)
}

fn queued(controller: &mut BrowserController) -> Vec<TabCommand> {
    std::iter::from_fn(|| controller.next_command()).collect()
}

#[test]
fn the_text_state_follows_the_engines_events() {
    let (mut controller, tab_id) = ready_controller();
    assert_eq!(controller.active_state().text_input(), None);

    controller.handle_event(
        tab_id,
        BrowserEvent::TextInputState(Some(state(1, "hello"))),
    );
    assert_eq!(
        controller.active_state().text_input(),
        Some(&state(1, "hello"))
    );

    // The next one replaces it, whatever its sequence.
    controller.handle_event(
        tab_id,
        BrowserEvent::TextInputState(Some(state(2, "hello w"))),
    );
    assert_eq!(
        controller.active_state().text_input(),
        Some(&state(2, "hello w"))
    );

    // Nothing editable has focus any more.
    controller.handle_event(tab_id, BrowserEvent::TextInputState(None));
    assert_eq!(controller.active_state().text_input(), None);
    assert!(queued(&mut controller).is_empty());
}

#[test]
fn a_crash_forgets_the_text_state() {
    let (mut controller, tab_id) = ready_controller();
    controller.handle_event(tab_id, BrowserEvent::TextInputState(Some(state(1, "a"))));

    controller.handle_event(
        tab_id,
        BrowserEvent::LoadStateChanged {
            state: LoadState::Failed {
                message: "crashed".to_owned(),
            },
        },
    );

    assert_eq!(controller.active_state().text_input(), None);
}

#[test]
fn a_new_document_has_no_text_state() {
    let (mut controller, tab_id) = ready_controller();
    controller.handle_event(tab_id, BrowserEvent::TextInputState(Some(state(1, "a"))));

    controller.handle_event(tab_id, BrowserEvent::PageLoadStarted);

    assert_eq!(controller.active_state().text_input(), None);
}

#[test]
fn a_replaced_web_view_leaves_no_text_state() {
    let (mut controller, tab_id) = ready_controller();
    controller.handle_event(tab_id, BrowserEvent::TextInputState(Some(state(1, "a"))));
    assert!(
        controller
            .set_initial_address(tab_id, "https://a.test/")
            .expect("the address resolves")
    );

    assert!(controller.take_initial_url(tab_id).is_some());

    assert_eq!(controller.active_state().text_input(), None);
}

#[test]
fn the_text_state_goes_with_its_tab() {
    let (mut controller, first_tab) = ready_controller();
    let second_tab = controller.open_tab();
    controller.handle_event(
        second_tab,
        BrowserEvent::AddressChanged { url: String::new() },
    );
    controller.select_tab(second_tab);
    controller.handle_event(
        second_tab,
        BrowserEvent::TextInputState(Some(state(1, "second"))),
    );
    assert_eq!(
        controller.active_state().text_input(),
        Some(&state(1, "second"))
    );

    let _outcome = controller.close_tab(second_tab);

    assert!(controller.tabs().all(|tab| tab.id() != second_tab));
    assert_eq!(controller.active_tab_id(), first_tab);
    // The tab that took over has its own, which is none.
    assert_eq!(controller.active_state().text_input(), None);
}

#[test]
fn a_background_tab_keeps_its_own_text_state() {
    let (mut controller, background_tab) = ready_controller();
    controller.open_tab();

    controller.handle_event(
        background_tab,
        BrowserEvent::TextInputState(Some(state(7, "kept"))),
    );

    let kept = controller
        .tabs()
        .find(|tab| tab.id() == background_tab)
        .and_then(|tab| tab.state().text_input().cloned());
    assert_eq!(kept, Some(state(7, "kept")));
    // The active tab is another one and shows none.
    assert_eq!(controller.active_state().text_input(), None);
    assert!(queued(&mut controller).is_empty());
}

#[test]
fn the_text_state_is_independent_of_the_input_method_area() {
    let (mut controller, tab_id) = ready_controller();
    controller.handle_event(
        tab_id,
        BrowserEvent::InputMethodShown {
            id: ControlId::new(6),
            rect: CARET,
        },
    );
    controller.handle_event(tab_id, BrowserEvent::TextInputState(Some(state(1, "x"))));

    controller.handle_event(
        tab_id,
        BrowserEvent::ControlHidden {
            id: ControlId::new(6),
        },
    );

    // The engine's own `None` ends the text state, not the hidden control: the two messages
    // travel on different paths and may arrive in either order.
    assert_eq!(controller.active_state().input_method(), None);
    assert_eq!(controller.active_state().text_input(), Some(&state(1, "x")));
}

#[test]
fn the_new_ime_inputs_travel_through_the_command_queue_unchanged() {
    let (mut controller, tab_id) = ready_controller();
    let inputs = [
        ImeInput::SetMarkedText {
            text: "ni".to_owned(),
            selected: Some(Utf16Range::collapsed(2)),
            clauses: vec![ImeClause {
                range: Utf16Range::new(0, 2),
                kind: ImeClauseKind::Raw,
            }],
            replacement: None,
        },
        ImeInput::Commit("你".to_owned()),
        ImeInput::InsertText {
            text: "é".to_owned(),
            replacement: Some(ImeReplacement {
                range: Utf16Range::new(3, 4),
                sequence: 9,
            }),
        },
    ];

    for input in inputs.clone() {
        controller.queue_command_for(tab_id, BrowserCommand::ImeInput(input));
    }

    let commands = queued(&mut controller);
    assert_eq!(commands.len(), 3);
    for (command, input) in commands.iter().zip(inputs) {
        assert_eq!(command.tab_id(), tab_id);
        assert_eq!(command.command(), &BrowserCommand::ImeInput(input));
    }
}

#[test]
fn debug_output_never_carries_the_text() {
    let secret = "correct horse battery staple";
    let text_state = TextInputState {
        is_password: false,
        ..state(3, secret)
    };
    let printed = format!("{text_state:?}");
    assert!(!printed.contains("horse"), "{printed}");
    assert!(printed.contains("text_units"), "{printed}");

    let printed = format!("{:?}", BrowserEvent::TextInputState(Some(text_state)));
    assert!(!printed.contains("horse"), "{printed}");

    for input in [
        ImeInput::Commit(secret.to_owned()),
        ImeInput::InsertText {
            text: secret.to_owned(),
            replacement: None,
        },
        ImeInput::SetMarkedText {
            text: secret.to_owned(),
            selected: None,
            clauses: Vec::new(),
            replacement: None,
        },
        ImeInput::Composition {
            state: talaria_core::CompositionState::Update,
            data: secret.to_owned(),
        },
    ] {
        let printed = format!("{input:?}");
        assert!(!printed.contains("horse"), "{printed}");
    }
    assert_eq!(format!("{:?}", ImeInput::Dismissed), "Dismissed");
}

#[test]
fn utf8_byte_offsets_become_utf16_units() {
    // "a" is 1 byte and 1 unit; "ç" 2 bytes, 1 unit; "你" 3 bytes, 1 unit; "😀" 4 bytes, 2 units.
    let text = "aç你😀z";
    assert_eq!(utf16_offset_of_byte(text, 0), 0);
    assert_eq!(utf16_offset_of_byte(text, 1), 1);
    assert_eq!(utf16_offset_of_byte(text, 3), 2);
    assert_eq!(utf16_offset_of_byte(text, 6), 3);
    assert_eq!(utf16_offset_of_byte(text, 10), 5);
    assert_eq!(utf16_offset_of_byte(text, 11), 6);
    assert_eq!(utf16_len(text), 6);
}

#[test]
fn a_byte_offset_inside_a_character_is_the_start_of_that_character() {
    let text = "a你😀";
    // Bytes 2 and 3 are inside "你" (bytes 1..4); 5, 6 and 7 inside "😀" (bytes 4..8).
    assert_eq!(utf16_offset_of_byte(text, 2), 1);
    assert_eq!(utf16_offset_of_byte(text, 3), 1);
    assert_eq!(utf16_offset_of_byte(text, 5), 2);
    assert_eq!(utf16_offset_of_byte(text, 7), 2);
    // Past the end is the end.
    assert_eq!(utf16_offset_of_byte(text, 99), 4);
    assert_eq!(utf16_offset_of_byte("", 5), 0);
}

#[test]
fn utf16_offsets_become_byte_offsets_and_back() {
    let text = "aç你😀z";
    for (unit, byte) in [(0, 0), (1, 1), (2, 3), (3, 6), (5, 10), (6, 11)] {
        assert_eq!(utf8_offset_of_utf16(text, unit), byte, "unit {unit}");
        assert_eq!(utf16_offset_of_byte(text, byte), unit, "byte {byte}");
    }
    // Inside the surrogate pair: the start of the emoji. Past the end: the end.
    assert_eq!(utf8_offset_of_utf16(text, 4), 6);
    assert_eq!(utf8_offset_of_utf16(text, 99), text.len());
}

#[test]
fn text_in_a_range_is_cut_to_the_window_and_to_whole_characters() {
    // The window starts at 100 of the whole text: "x😀y" is units 100..104.
    let window = TextInputState {
        text_start: 100,
        selection: Utf16Range::new(0, 1),
        ..state(4, "x😀y")
    };
    assert_eq!(window.window_end(), 104);
    assert_eq!(window.absolute_selection(), Utf16Range::new(100, 101));

    assert_eq!(
        window.text_in(Utf16Range::new(100, 104)),
        Some((Utf16Range::new(100, 104), "x😀y".to_owned()))
    );
    // Wider than the window: cut.
    assert_eq!(
        window.text_in(Utf16Range::new(90, 200)),
        Some((Utf16Range::new(100, 104), "x😀y".to_owned()))
    );
    // Ends inside the pair: the pair is left out; starts inside it: left out too.
    assert_eq!(
        window.text_in(Utf16Range::new(100, 102)),
        Some((Utf16Range::new(100, 101), "x".to_owned()))
    );
    assert_eq!(
        window.text_in(Utf16Range::new(102, 104)),
        Some((Utf16Range::new(103, 104), "y".to_owned()))
    );
    // A caret inside a pair is after the pair, not half a character.
    assert_eq!(
        window.text_in(Utf16Range::new(102, 102)),
        Some((Utf16Range::new(103, 103), String::new()))
    );
    // Outside the window altogether.
    assert_eq!(window.text_in(Utf16Range::new(0, 50)), None);
    assert_eq!(window.text_in(Utf16Range::new(105, 110)), None);
    assert_eq!(window.text_in(Utf16Range::new(103, 101)), None);
}

#[test]
fn a_replacement_names_the_state_and_is_relative_to_its_window() {
    let window = TextInputState {
        text_start: 100,
        ..state(12, "hello")
    };

    assert_eq!(
        window.replacement_for(Utf16Range::new(102, 104)),
        Some(ImeReplacement {
            range: Utf16Range::new(2, 4),
            sequence: 12,
        })
    );
    // Cut to the end of the window.
    assert_eq!(
        window.replacement_for(Utf16Range::new(103, 500)),
        Some(ImeReplacement {
            range: Utf16Range::new(3, 5),
            sequence: 12,
        })
    );
    // Starting before or after the window: the engine never saw it.
    assert_eq!(window.replacement_for(Utf16Range::new(99, 102)), None);
    assert_eq!(window.replacement_for(Utf16Range::new(106, 107)), None);
    // A caret at the very end is fine.
    assert_eq!(
        window.replacement_for(Utf16Range::collapsed(105)),
        Some(ImeReplacement {
            range: Utf16Range::collapsed(5),
            sequence: 12,
        })
    );
}

#[test]
fn a_password_field_state_has_no_text_to_hand_out() {
    let password = TextInputState {
        text: String::new(),
        selection: Utf16Range::collapsed(0),
        is_password: true,
        ..state(1, "")
    };

    assert_eq!(password.window_end(), 0);
    assert_eq!(
        password.text_in(Utf16Range::new(0, 10)),
        Some((Utf16Range::new(0, 0), String::new()))
    );
}

#[test]
fn utf16_ranges_report_their_size() {
    assert!(Utf16Range::collapsed(3).is_empty());
    assert_eq!(Utf16Range::new(2, 5).len(), 3);
    assert_eq!(Utf16Range::new(5, 2).len(), 0);
    assert!(Utf16Range::new(5, 2).is_empty());
    assert!(ImeClauseKind::SelectedRaw.is_selected());
    assert!(ImeClauseKind::SelectedConverted.is_selected());
    assert!(!ImeClauseKind::Raw.is_selected());
    assert!(!ImeClauseKind::Converted.is_selected());
}
