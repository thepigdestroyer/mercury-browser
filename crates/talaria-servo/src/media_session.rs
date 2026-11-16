//! Servo's media session and the script-level mute, probe and media-key fallback.

use std::cell::Cell;
use std::rc::Rc;

use servo::{
    JSValue, MediaSessionActionType, MediaSessionEvent, MediaSessionPlaybackState as ServoPlayback,
};
use talaria_core::{
    BrowserEvent, MediaMetadata, MediaPlaybackState, MediaPosition, MediaProbe, MediaSessionAction,
    TabId,
};

/// The tab whose page last reported a media-session event the constellation forwarded. Servo
/// keeps one active media session for all web views (the pipeline that sent the last such
/// event) and sends every media-session action there, whichever web view it is given to.
pub(crate) type EngineMediaSession = Rc<Cell<Option<TabId>>>;

const MUTE_SCRIPT: &str = include_str!("../resources/media/tab-mute.js");
const PROBE_SCRIPT: &str = include_str!("../resources/media/probe.js");
const MEDIA_KEYS_SCRIPT: &str = include_str!("../resources/media/media-keys.js");

pub(crate) fn browser_event_from_servo(event: MediaSessionEvent) -> BrowserEvent {
    match event {
        MediaSessionEvent::SetMetadata(metadata) => BrowserEvent::MediaMetadataChanged {
            metadata: MediaMetadata {
                title: metadata.title,
                artist: metadata.artist,
                album: metadata.album,
            },
        },
        MediaSessionEvent::PlaybackStateChange(state) => BrowserEvent::MediaPlaybackChanged {
            state: match state {
                ServoPlayback::None_ => MediaPlaybackState::None,
                ServoPlayback::Playing => MediaPlaybackState::Playing,
                ServoPlayback::Paused => MediaPlaybackState::Paused,
            },
        },
        MediaSessionEvent::SetPositionState(state) => BrowserEvent::MediaPositionChanged {
            position: MediaPosition::new(state.duration, state.playback_rate, state.position),
        },
    }
}

/// How a media-session action reaches a tab's page.
#[derive(Debug, PartialEq)]
pub(crate) enum ActionRoute {
    /// Through Servo, which runs the page's action handler or its default action.
    Engine(MediaSessionActionType),
    /// Servo would deliver it to another tab: play or pause this tab's media from script.
    Script(String),
    /// Nothing can deliver it to this tab.
    Drop,
}

pub(crate) fn route_action(action: MediaSessionAction, owns_engine_session: bool) -> ActionRoute {
    if owns_engine_session {
        return ActionRoute::Engine(servo_action(action));
    }
    match action {
        MediaSessionAction::Play => ActionRoute::Script(media_keys_script("play")),
        MediaSessionAction::Pause => ActionRoute::Script(media_keys_script("pause")),
        _ => ActionRoute::Drop,
    }
}

fn servo_action(action: MediaSessionAction) -> MediaSessionActionType {
    match action {
        MediaSessionAction::Play => MediaSessionActionType::Play,
        MediaSessionAction::Pause => MediaSessionActionType::Pause,
        MediaSessionAction::PreviousTrack => MediaSessionActionType::PreviousTrack,
        MediaSessionAction::NextTrack => MediaSessionActionType::NextTrack,
        MediaSessionAction::SeekBackward => MediaSessionActionType::SeekBackward,
        MediaSessionAction::SeekForward => MediaSessionActionType::SeekForward,
        MediaSessionAction::Stop => MediaSessionActionType::Stop,
    }
}

fn media_keys_script(action: &str) -> String {
    MEDIA_KEYS_SCRIPT.replace("__ACTION__", &format!("\"{action}\""))
}

pub(crate) fn mute_script(muted: bool) -> String {
    MUTE_SCRIPT.replace("__MUTED__", if muted { "true" } else { "false" })
}

pub(crate) fn probe_script() -> &'static str {
    PROBE_SCRIPT
}

/// Reads the probe script's answer; anything else (a page that broke the script's built-ins,
/// a document gone mid-way) gives `None`.
pub(crate) fn probe_from_js(value: &JSValue) -> Option<MediaProbe> {
    let JSValue::Array(items) = value else {
        return None;
    };
    let [
        JSValue::Number(playing),
        JSValue::Number(audible),
        JSValue::Boolean(fullscreen_video),
    ] = items.as_slice()
    else {
        return None;
    };
    let count = |number: f64| {
        (number.is_finite() && number >= 0.0 && number <= f64::from(u32::MAX))
            .then_some(number as u32)
    };
    Some(MediaProbe {
        playing: count(*playing)?,
        audible: count(*audible)?,
        fullscreen_video: *fullscreen_video,
    })
}

#[cfg(test)]
mod tests {
    use servo::{MediaMetadata as ServoMetadata, MediaPositionState};

    use super::*;

    #[test]
    fn media_session_events_become_browser_events() {
        assert_eq!(
            browser_event_from_servo(MediaSessionEvent::PlaybackStateChange(
                ServoPlayback::Playing
            )),
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Playing
            }
        );
        assert_eq!(
            browser_event_from_servo(MediaSessionEvent::PlaybackStateChange(
                ServoPlayback::Paused
            )),
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Paused
            }
        );
        assert_eq!(
            browser_event_from_servo(MediaSessionEvent::PlaybackStateChange(ServoPlayback::None_)),
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::None
            }
        );
        assert_eq!(
            browser_event_from_servo(MediaSessionEvent::SetMetadata(ServoMetadata {
                title: "Title".to_owned(),
                artist: "Artist".to_owned(),
                album: "Album".to_owned(),
            })),
            BrowserEvent::MediaMetadataChanged {
                metadata: MediaMetadata {
                    title: "Title".to_owned(),
                    artist: "Artist".to_owned(),
                    album: "Album".to_owned(),
                }
            }
        );
        assert_eq!(
            browser_event_from_servo(MediaSessionEvent::SetPositionState(
                MediaPositionState::new(4.0, 1.0, 2.5)
            )),
            BrowserEvent::MediaPositionChanged {
                position: MediaPosition::new(4.0, 1.0, 2.5)
            }
        );
    }

    #[test]
    fn the_tab_owning_the_engine_session_gets_every_action_through_servo() {
        let pairs = [
            (MediaSessionAction::Play, MediaSessionActionType::Play),
            (MediaSessionAction::Pause, MediaSessionActionType::Pause),
            (
                MediaSessionAction::PreviousTrack,
                MediaSessionActionType::PreviousTrack,
            ),
            (
                MediaSessionAction::NextTrack,
                MediaSessionActionType::NextTrack,
            ),
            (
                MediaSessionAction::SeekBackward,
                MediaSessionActionType::SeekBackward,
            ),
            (
                MediaSessionAction::SeekForward,
                MediaSessionActionType::SeekForward,
            ),
            (MediaSessionAction::Stop, MediaSessionActionType::Stop),
        ];
        for (action, servo_action) in pairs {
            assert_eq!(
                route_action(action, true),
                ActionRoute::Engine(servo_action)
            );
        }
    }

    #[test]
    fn another_tab_gets_play_and_pause_from_script_and_nothing_else() {
        let ActionRoute::Script(pause) = route_action(MediaSessionAction::Pause, false) else {
            panic!("pause is played from script");
        };
        assert!(pause.contains("})(\"pause\")"), "{pause}");
        assert!(!pause.contains("__ACTION__"));
        let ActionRoute::Script(play) = route_action(MediaSessionAction::Play, false) else {
            panic!("play is played from script");
        };
        assert!(play.contains("})(\"play\")"), "{play}");
        for action in [
            MediaSessionAction::NextTrack,
            MediaSessionAction::PreviousTrack,
            MediaSessionAction::SeekBackward,
            MediaSessionAction::SeekForward,
            MediaSessionAction::Stop,
        ] {
            assert_eq!(route_action(action, false), ActionRoute::Drop);
        }
    }

    #[test]
    fn the_mute_script_is_given_the_choice() {
        let mute = mute_script(true);
        assert!(mute.trim_end().ends_with("})(true)"), "{mute}");
        let unmute = mute_script(false);
        assert!(unmute.trim_end().ends_with("})(false)"), "{unmute}");
        assert!(!mute.contains("__MUTED__") && !unmute.contains("__MUTED__"));
    }

    #[test]
    fn probe_answers_are_read() {
        assert_eq!(
            probe_from_js(&JSValue::Array(vec![
                JSValue::Number(2.0),
                JSValue::Number(1.0),
                JSValue::Boolean(true),
            ])),
            Some(MediaProbe {
                playing: 2,
                audible: 1,
                fullscreen_video: true,
            })
        );
    }

    #[test]
    fn malformed_probe_answers_are_ignored() {
        for value in [
            JSValue::Undefined,
            JSValue::Array(vec![JSValue::Number(1.0), JSValue::Number(1.0)]),
            JSValue::Array(vec![
                JSValue::Number(-1.0),
                JSValue::Number(0.0),
                JSValue::Boolean(false),
            ]),
            JSValue::Array(vec![
                JSValue::Number(f64::NAN),
                JSValue::Number(0.0),
                JSValue::Boolean(false),
            ]),
            JSValue::Array(vec![
                JSValue::String("1".to_owned()),
                JSValue::Number(0.0),
                JSValue::Boolean(false),
            ]),
        ] {
            assert_eq!(probe_from_js(&value), None, "{value:?}");
        }
    }
}
