// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Servo's media session and the script-level mute, probe and media-key fallback.

use std::cell::Cell;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::rc::Rc;
use std::sync::OnceLock;

use url::Url;

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

const AGENT_SCRIPT: &str = include_str!("../resources/media/media-agent.js");

/// The name of the property that holds a document's media helper: a keyed hash of the document's
/// origin under a secret chosen at random for each browser run. A page cannot claim it before the
/// browser installs the helper, and two sites see different names, so it cannot link them.
fn agent_name(origin: &str) -> String {
    // SipHash keyed with the random keys of one `RandomState`, kept for the whole run.
    static SECRET: OnceLock<RandomState> = OnceLock::new();
    let secret = SECRET.get_or_init(RandomState::new);
    let half = |salt: u8| {
        let mut hasher = secret.build_hasher();
        hasher.write_u8(salt);
        hasher.write(origin.as_bytes());
        hasher.finish()
    };
    format!("__mercury_media_{:016x}{:016x}", half(1), half(2))
}

fn agent_script(call: &str, origin: &str) -> String {
    AGENT_SCRIPT
        .replace("__TOKEN__", &agent_name(origin))
        .replace("__CALL__", call)
}

/// Page-controlled text in a log line: one line, at most this many characters.
pub(crate) fn loggable(text: &str) -> String {
    const LIMIT: usize = 64;
    let mut line = text
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(LIMIT)
        .collect::<String>();
    if text.chars().count() > LIMIT {
        line.push('…');
    }
    line
}

/// A media-session event for the log, with the page's metadata cut short.
pub(crate) fn describe_for_log(event: &BrowserEvent) -> String {
    match event {
        BrowserEvent::MediaMetadataChanged { metadata } => format!(
            "metadata {:?} / {:?} / {:?}",
            loggable(&metadata.title),
            loggable(&metadata.artist),
            loggable(&metadata.album)
        ),
        other => format!("{other:?}"),
    }
}

/// What the media helper's name is derived from for the page at `url`. A tuple origin is
/// enough: every document of a site shares the name, no other site sees it. Opaque origins
/// (`about:blank` popups a site opens and scripts, `data:`, `file:`) all serialise as "null", so
/// their documents get a key of their own from the web view's `serial` (unique in the run) and
/// its count of document starts (`generation`); see `DocumentStarts::identity`.
pub(crate) fn agent_key(url: Option<&Url>, serial: u64, generation: u64) -> String {
    match url.map(Url::origin) {
        Some(origin) if origin.is_tuple() => origin.ascii_serialization(),
        _ => format!("null#{serial}#{generation}"),
    }
}

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

pub(crate) fn route_action(
    action: MediaSessionAction,
    through_engine: bool,
    origin: &str,
) -> ActionRoute {
    if through_engine {
        return ActionRoute::Engine(servo_action(action));
    }
    match action {
        MediaSessionAction::Play => ActionRoute::Script(media_keys_script("play", origin)),
        MediaSessionAction::Pause => ActionRoute::Script(media_keys_script("pause", origin)),
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

fn media_keys_script(action: &str, origin: &str) -> String {
    agent_script(&format!("[\"key\", \"{action}\"]"), origin)
}

pub(crate) fn mute_script(muted: bool, origin: &str) -> String {
    agent_script(
        if muted {
            "[\"mute\", true]"
        } else {
            "[\"mute\", false]"
        },
        origin,
    )
}

pub(crate) fn probe_script(origin: &str) -> String {
    agent_script("[\"probe\"]", origin)
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
        JSValue::Boolean(unseen),
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
        unseen: *unseen,
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
                route_action(action, true, "https://a.test"),
                ActionRoute::Engine(servo_action)
            );
        }
    }

    #[test]
    fn another_tab_gets_play_and_pause_from_script_and_nothing_else() {
        let ActionRoute::Script(pause) =
            route_action(MediaSessionAction::Pause, false, "https://a.test")
        else {
            panic!("pause is played from script");
        };
        assert!(
            pause.trim_end().ends_with(", [\"key\", \"pause\"])"),
            "{pause}"
        );
        let ActionRoute::Script(play) =
            route_action(MediaSessionAction::Play, false, "https://a.test")
        else {
            panic!("play is played from script");
        };
        assert!(
            play.trim_end().ends_with(", [\"key\", \"play\"])"),
            "{play}"
        );
        for action in [
            MediaSessionAction::NextTrack,
            MediaSessionAction::PreviousTrack,
            MediaSessionAction::SeekBackward,
            MediaSessionAction::SeekForward,
            MediaSessionAction::Stop,
        ] {
            assert_eq!(
                route_action(action, false, "https://a.test"),
                ActionRoute::Drop
            );
        }
    }

    #[test]
    fn the_mute_script_is_given_the_choice() {
        let mute = mute_script(true, "https://a.test");
        assert!(mute.trim_end().ends_with(", [\"mute\", true])"), "{mute}");
        let unmute = mute_script(false, "https://a.test");
        assert!(
            unmute.trim_end().ends_with(", [\"mute\", false])"),
            "{unmute}"
        );
        assert!(
            probe_script("https://a.test")
                .trim_end()
                .ends_with(", [\"probe\"])")
        );
    }

    #[test]
    fn the_helpers_name_is_per_origin_random_per_run_and_not_a_registry_symbol() {
        let name = agent_name("https://a.test");
        assert!(name.starts_with("__mercury_media_"));
        assert_eq!(name.len(), "__mercury_media_".len() + 32);
        assert_ne!(&name["__mercury_media_".len()..], "0".repeat(32));
        assert_eq!(agent_name("https://a.test"), name, "stable within a run");
        // Two sites in one run cannot compare names to recognise the browser run.
        assert_ne!(agent_name("https://b.test"), name);
        assert_ne!(agent_name("https://a.test:8443"), name);
        assert_ne!(agent_name("http://a.test"), name);
        for script in [
            mute_script(true, "https://a.test"),
            probe_script("https://a.test"),
            media_keys_script("play", "https://a.test"),
        ] {
            assert!(script.contains(&format!("(\"{name}\", [")), "{script}");
            assert!(!script.contains("__TOKEN__") && !script.contains("__CALL__"));
            assert!(!script.contains("Symbol.for"));
        }
    }

    #[test]
    fn page_metadata_is_cut_short_and_kept_on_one_line_in_the_log() {
        assert_eq!(loggable("a\nb\u{7}c"), "a b c");
        let long = "x".repeat(100);
        assert_eq!(loggable(&long), format!("{}…", "x".repeat(64)));
        let event = BrowserEvent::MediaMetadataChanged {
            metadata: MediaMetadata {
                title: long,
                artist: "A".to_owned(),
                album: String::new(),
            },
        };
        let line = describe_for_log(&event);
        assert!(line.len() < 120, "{line}");
    }

    #[test]
    fn probe_answers_are_read() {
        assert_eq!(
            probe_from_js(&JSValue::Array(vec![
                JSValue::Number(2.0),
                JSValue::Number(1.0),
                JSValue::Boolean(true),
                JSValue::Boolean(true),
            ])),
            Some(MediaProbe {
                playing: 2,
                audible: 1,
                unseen: true,
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
                JSValue::Number(1.0),
                JSValue::Number(1.0),
                JSValue::Boolean(false),
            ]),
            JSValue::Array(vec![
                JSValue::Number(-1.0),
                JSValue::Number(0.0),
                JSValue::Boolean(false),
                JSValue::Boolean(false),
            ]),
            JSValue::Array(vec![
                JSValue::Number(f64::NAN),
                JSValue::Number(0.0),
                JSValue::Boolean(false),
                JSValue::Boolean(false),
            ]),
            JSValue::Array(vec![
                JSValue::String("1".to_owned()),
                JSValue::Number(0.0),
                JSValue::Boolean(false),
                JSValue::Boolean(false),
            ]),
        ] {
            assert_eq!(probe_from_js(&value), None, "{value:?}");
        }
    }

    #[test]
    fn opaque_documents_get_a_name_of_their_own() {
        let url = |text: &str| Url::parse(text).unwrap();
        let blank = url("about:blank");

        // Tuple origins: one name per origin and run, whichever web view or document.
        assert_eq!(
            agent_key(Some(&url("https://a.test/x")), 1, 0),
            agent_key(Some(&url("https://a.test/y")), 2, 7)
        );
        assert_eq!(
            agent_key(Some(&url("https://a.test/")), 1, 0),
            "https://a.test"
        );

        // Opaque origins (`about:blank` popups, `data:`, `file:`): never shared between web
        // views, nor between documents of one web view.
        let popup = agent_key(Some(&blank), 1, 0);
        assert_ne!(popup, agent_key(Some(&blank), 2, 0));
        assert_ne!(popup, agent_key(Some(&blank), 1, 1));
        assert_eq!(popup, agent_key(Some(&url("data:text/html,x")), 1, 0));
        assert_ne!(
            agent_name(&popup),
            agent_name(&agent_key(Some(&blank), 1, 1))
        );
        assert_ne!(agent_key(None, 1, 0), agent_key(None, 2, 0));
    }
}
