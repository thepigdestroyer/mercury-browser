// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Pure decisions the media element takes (med-1b review fix, findings 2 and 4): the play/pause action of
//! `update_media_state`, the `waiting` predicate of "ready state change", and the position gate. The
//! element (`htmlmediaelement.rs` and `dom/mse/mediaelement.rs`) calls these and does only the player
//! calls, so the decisions are tested here without Servo or GStreamer (`tests/media_state_table.rs`,
//! `tests/position_gate_*.rs`).

use crate::api::ElementReadyState;

/// What `update_media_state` does to the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaStateAction {
    /// Set the playback rate and volume and start the player.
    Play,
    /// Pause the player.
    Pause,
    /// Leave the player as it is.
    Keep,
}

/// The decision of `update_media_state`.
///
/// * `potentially_playing`: HTML "potentially playing" (not paused, not ended, no error, not a blocked
///   media element).
/// * `is_playing`: the player is running (not paused).
/// * `stalled`: the underrun guard of an attached MediaSource holds the pipeline paused.
/// * `media_source_attached`: the element plays a MediaSource.
///
/// Without a MediaSource this is **exactly** what Servo 0.8 (and `main`) does:
/// `if potentially_playing && !is_playing { play } else if is_playing { pause }`, so `Pause` whenever the
/// player runs and the first branch was not taken, even for an element that is potentially playing (an
/// defect of Servo 0.8, kept for a plain element so that the default build does not change until the
/// F6 window check of `sdd/deferred-hands-on.md` has passed). `stalled` does not exist there and is
/// ignored.
///
/// With a MediaSource: `Play` if `potentially_playing && !stalled && !is_playing`; `Pause` if
/// `is_playing && (stalled || !potentially_playing)`; otherwise `Keep`. A playing element that may play on
/// is left alone; the guard's pause is not undone while it holds.
pub fn media_state_action(
    potentially_playing: bool,
    is_playing: bool,
    stalled: bool,
    media_source_attached: bool,
) -> MediaStateAction {
    if !media_source_attached {
        return if potentially_playing && !is_playing {
            MediaStateAction::Play
        } else if is_playing {
            MediaStateAction::Pause
        } else {
            MediaStateAction::Keep
        };
    }
    if potentially_playing && !stalled && !is_playing {
        MediaStateAction::Play
    } else if is_playing && (stalled || !potentially_playing) {
        MediaStateAction::Pause
    } else {
        MediaStateAction::Keep
    }
}

/// The position the element passes on to the backend (`set_position`), or `None` to drop the report.
///
/// While a seek is pending the player still reports the position it had before the seek: the backend
/// already knows the target (`media_source_seeking` set it), and the stale value would make the data
/// around the target look old enough to evict (a backward seek) and would start a late track at the old
/// place. Reports resume when the seek has completed.
pub fn forward_position(seeking: bool, position: f64) -> Option<f64> {
    if seeking { None } else { Some(position) }
}

/// The inputs of the `waiting` condition in the "ready state change" steps of the HTML specification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitingInputs {
    /// The readyState before the change.
    pub old: ElementReadyState,
    /// The readyState after it.
    pub new: ElementReadyState,
    /// The element was potentially playing before its readyState changed.
    pub was_potentially_playing: bool,
    /// The element has ended playback (looping included).
    pub ended_playback: bool,
    /// The element has an error (`error` is not null).
    pub has_error: bool,
    pub paused_for_user_interaction: bool,
    pub paused_for_in_band_content: bool,
}

/// Whether `timeupdate` and `waiting` are queued for this readyState change: the previous state was
/// HAVE_FUTURE_DATA or more, the new one HAVE_CURRENT_DATA or less, and the element was potentially
/// playing and has not ended playback and "playback has not stopped due to errors, paused for user
/// interaction, or paused for in-band content".
///
/// Servo 0.8 tests `!error.is_none()` where the specification says there is no error, so it queued
/// `waiting` only for an element with an error. The polarity here is the specified one, for plain media
/// and for MediaSource alike.
pub fn waiting_fires(inputs: &WaitingInputs) -> bool {
    inputs.old >= ElementReadyState::HaveFutureData
        && inputs.new <= ElementReadyState::HaveCurrentData
        && inputs.was_potentially_playing
        && !inputs.ended_playback
        && !inputs.has_error
        && !inputs.paused_for_user_interaction
        && !inputs.paused_for_in_band_content
}
