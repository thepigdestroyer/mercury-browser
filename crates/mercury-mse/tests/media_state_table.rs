// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The pure decisions of the media element (med-1b review fix, finding 4): `update_media_state`'s play and
//! pause action and the `waiting` predicate of "ready state change". Without a MediaSource they must
//! reproduce Servo 0.8 exactly (so the default build, where MSE is off, does not change); with one they are
//! the fixed rules. The element's two hunks call these functions, so these tables are the evidence for
//! both.

use mercury_mse::api::ElementReadyState::{self, *};
use mercury_mse::media_state::MediaStateAction::{self, *};
use mercury_mse::media_state::{WaitingInputs, media_state_action, waiting_fires};

const BOOLS: [bool; 2] = [false, true];

/// The code of `update_media_state` in Servo 0.8 and on current `main`, transcribed:
///
/// ```text
/// if potentially_playing && !is_playing { play } else if is_playing { pause }
/// ```
fn servo_0_8(potentially_playing: bool, is_playing: bool) -> MediaStateAction {
    if potentially_playing && !is_playing {
        Play
    } else if is_playing {
        Pause
    } else {
        Keep
    }
}

// ---- media_state_action ----

#[test]
fn media_state_without_a_media_source_is_servo_0_8_for_all_sixteen_inputs() {
    for potentially_playing in BOOLS {
        for is_playing in BOOLS {
            for stalled in BOOLS {
                assert_eq!(
                    media_state_action(potentially_playing, is_playing, stalled, false),
                    servo_0_8(potentially_playing, is_playing),
                    "potentially_playing {potentially_playing}, is_playing {is_playing}, \
                     stalled {stalled}"
                );
            }
        }
    }
}

#[test]
fn media_state_without_a_media_source_pauses_a_playing_element_whatever_else_holds() {
    // Servo's own behaviour, kept on purpose while the preference is off: `else if is_playing` is reached
    // for every state but "may play and is not playing", so even a potentially playing element is paused.
    assert_eq!(media_state_action(true, true, false, false), Pause);
    assert_eq!(media_state_action(false, true, false, false), Pause);
    assert_eq!(media_state_action(true, false, false, false), Play);
    assert_eq!(media_state_action(false, false, false, false), Keep);
}

#[test]
fn media_state_with_a_media_source_table() {
    // (potentially_playing, is_playing, stalled) -> action
    let table: [((bool, bool, bool), MediaStateAction); 8] = [
        ((false, false, false), Keep),
        ((false, false, true), Keep),
        ((false, true, false), Pause), // the page paused, ended, an error, below HAVE_FUTURE_DATA
        ((false, true, true), Pause),
        ((true, false, false), Play), // may play and the pipeline does not run: start it
        ((true, false, true), Keep),  // the underrun guard holds the pipeline
        ((true, true, false), Keep),  // playing and may play on: left alone (the fix)
        ((true, true, true), Pause),  // the guard stops a running pipeline
    ];
    for ((potentially_playing, is_playing, stalled), want) in table {
        assert_eq!(
            media_state_action(potentially_playing, is_playing, stalled, true),
            want,
            "potentially_playing {potentially_playing}, is_playing {is_playing}, stalled {stalled}"
        );
    }
}

#[test]
fn media_state_with_a_media_source_never_pauses_a_healthy_playing_element() {
    // The defect of the published code: readiness moving between HAVE_FUTURE_DATA and HAVE_ENOUGH_DATA
    // calls update_media_state while the element is playing, and it paused the pipeline each time.
    assert_eq!(media_state_action(true, true, false, true), Keep);
}

#[test]
fn media_state_with_a_media_source_pauses_exactly_when_it_must() {
    for potentially_playing in BOOLS {
        for stalled in BOOLS {
            let got = media_state_action(potentially_playing, true, stalled, true);
            assert_eq!(
                got == Pause,
                stalled || !potentially_playing,
                "potentially_playing {potentially_playing}, stalled {stalled}"
            );
        }
    }
}

#[test]
fn media_state_with_a_media_source_never_plays_while_stalled_or_while_playing() {
    for potentially_playing in BOOLS {
        for is_playing in BOOLS {
            for stalled in BOOLS {
                let got = media_state_action(potentially_playing, is_playing, stalled, true);
                if got == Play {
                    assert!(
                        potentially_playing && !is_playing && !stalled,
                        "Play for ({potentially_playing}, {is_playing}, {stalled})"
                    );
                }
            }
        }
    }
}

// ---- waiting_fires ----

const READY: [ElementReadyState; 5] = [
    HaveNothing,
    HaveMetadata,
    HaveCurrentData,
    HaveFutureData,
    HaveEnoughData,
];

/// A change from HAVE_ENOUGH_DATA to HAVE_CURRENT_DATA of an element that was playing normally: the one
/// case `waiting` is for.
fn underrun() -> WaitingInputs {
    WaitingInputs {
        old: HaveEnoughData,
        new: HaveCurrentData,
        was_potentially_playing: true,
        ended_playback: false,
        has_error: false,
        paused_for_user_interaction: false,
        paused_for_in_band_content: false,
    }
}

#[test]
fn waiting_fires_for_the_ordinary_underrun_of_plain_media_and_of_a_media_source() {
    // The ready state steps do not know whether a MediaSource is attached: plain media that stalls on the
    // network and an MSE element that plays out its data are the same change of readyState. With the
    // inverted test of Servo 0.8 (`!error.is_none()`) neither fired `waiting`.
    assert!(waiting_fires(&underrun()));
}

#[test]
fn waiting_does_not_fire_when_playback_stopped_due_to_an_error() {
    // The error test has the opposite polarity of the published one: an error suppresses `waiting`.
    let with_error = WaitingInputs {
        has_error: true,
        ..underrun()
    };
    assert!(!waiting_fires(&with_error));
}

#[test]
fn waiting_needs_each_of_its_conditions() {
    let base = underrun();
    let cases: [(&str, WaitingInputs); 5] = [
        (
            "was not potentially playing",
            WaitingInputs {
                was_potentially_playing: false,
                ..base
            },
        ),
        (
            "has ended playback",
            WaitingInputs {
                ended_playback: true,
                ..base
            },
        ),
        (
            "error",
            WaitingInputs {
                has_error: true,
                ..base
            },
        ),
        (
            "paused for user interaction",
            WaitingInputs {
                paused_for_user_interaction: true,
                ..base
            },
        ),
        (
            "paused for in-band content",
            WaitingInputs {
                paused_for_in_band_content: true,
                ..base
            },
        ),
    ];
    for (what, inputs) in cases {
        assert!(!waiting_fires(&inputs), "fired although the element {what}");
    }
}

#[test]
fn waiting_depends_on_the_ready_state_step_exactly_as_the_spec_says() {
    // "previous ready state HAVE_FUTURE_DATA or more, new ready state HAVE_CURRENT_DATA or less".
    for old in READY {
        for new in READY {
            let got = waiting_fires(&WaitingInputs {
                old,
                new,
                ..underrun()
            });
            let want = old >= HaveFutureData && new <= HaveCurrentData;
            assert_eq!(got, want, "{old:?} -> {new:?}");
        }
    }
}

#[test]
fn waiting_over_all_inputs_is_the_conjunction_of_the_spec_steps() {
    for old in READY {
        for new in READY {
            for bits in 0u8..32 {
                let flag = |n: u8| bits & (1 << n) != 0;
                let inputs = WaitingInputs {
                    old,
                    new,
                    was_potentially_playing: flag(0),
                    ended_playback: flag(1),
                    has_error: flag(2),
                    paused_for_user_interaction: flag(3),
                    paused_for_in_band_content: flag(4),
                };
                let want = old >= HaveFutureData
                    && new <= HaveCurrentData
                    && flag(0)
                    && !flag(1)
                    && !flag(2)
                    && !flag(3)
                    && !flag(4);
                assert_eq!(waiting_fires(&inputs), want, "{inputs:?}");
            }
        }
    }
}

// ---- forward_position (finding 2, the pure part) ----

#[test]
fn a_position_is_forwarded_when_no_seek_is_pending() {
    use mercury_mse::media_state::forward_position;
    assert_eq!(forward_position(false, 12.5), Some(12.5));
    assert_eq!(forward_position(false, 0.0), Some(0.0));
}

#[test]
fn a_position_is_never_forwarded_while_a_seek_is_pending() {
    use mercury_mse::media_state::forward_position;
    for position in [0.0, 0.001, 12.5, 1e9] {
        assert_eq!(forward_position(true, position), None, "{position}");
    }
}
