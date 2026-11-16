//! Media-session lane: which tab plays, muting a tab, the system's Now Playing and media keys,
//! and keeping the display awake during fullscreen video.
//!
//! The system services (macOS `MPNowPlayingInfoCenter`, `MPRemoteCommandCenter` and an IOKit
//! power assertion) are only used in an interactive launch. Screenshot and background runs get
//! fakes that log what would have been published, so automated runs never take the user's
//! media keys, appear in Control Centre or keep the display awake.

mod display_sleep;
#[cfg(target_os = "macos")]
mod macos;
mod now_playing;
mod probe;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use mercury_ui::{MainWindow, TabAudioState};
use slint::ComponentHandle;
use talaria_core::{BrowserCommand, BrowserController, TabAudio};

pub(crate) use display_sleep::{DisplaySleepPolicy, PowerAssertions};
pub(crate) use now_playing::{
    NowPlayingCenter, NowPlayingChange, NowPlayingInfo, NowPlayingPolicy, RemoteCommand,
};
use probe::ProbeSchedule;

use crate::ui_sync::sync_ui;

pub(crate) struct MediaSessionHub {
    probes: ProbeSchedule,
    now_playing: NowPlayingPolicy,
    display_sleep: DisplaySleepPolicy,
    center: Box<dyn NowPlayingCenter>,
    power: Box<dyn PowerAssertions>,
}

impl MediaSessionHub {
    pub(crate) fn new(center: Box<dyn NowPlayingCenter>, power: Box<dyn PowerAssertions>) -> Self {
        Self {
            probes: ProbeSchedule::default(),
            now_playing: NowPlayingPolicy::default(),
            display_sleep: DisplaySleepPolicy::default(),
            center,
            power,
        }
    }

    /// Brings the system services in line with the tabs. Returns true if it queued commands.
    pub(crate) fn update(&mut self, controller: &mut BrowserController, now: Instant) -> bool {
        let due = self.probes.due(controller, now);
        for tab_id in &due {
            controller.queue_command_for(*tab_id, BrowserCommand::ProbeMedia);
        }
        match self.now_playing.update(controller) {
            Some(NowPlayingChange::Publish(info)) => self.center.publish(&info),
            Some(NowPlayingChange::Clear) => self.center.clear(),
            None => {}
        }
        self.display_sleep
            .update(DisplaySleepPolicy::wanted(controller), self.power.as_mut());
        !due.is_empty()
    }

    /// Sends a media key to the tab in Now Playing. Returns true if it queued a command.
    pub(crate) fn remote_command(
        &mut self,
        controller: &mut BrowserController,
        command: RemoteCommand,
    ) -> bool {
        let Some((tab_id, action)) = self.now_playing.action_for(command) else {
            log::info!("media key {command:?}: no tab is in Now Playing");
            return false;
        };
        log::info!(
            "media key {command:?}: {action:?} to tab {}",
            tab_id.value()
        );
        controller.queue_command_for(tab_id, BrowserCommand::MediaSessionAction(action))
    }

    /// Releases everything held for the system; the browser is quitting.
    pub(crate) fn shut_down(&mut self) {
        self.display_sleep.release(self.power.as_mut());
        if self.now_playing.owner().is_some() {
            self.center.clear();
        }
        self.now_playing = NowPlayingPolicy::default();
    }
}

/// Only read in screenshot and background runs.
const START_MUTED_VARIABLE: &str = "MERCURY_START_MUTED";

struct Installed {
    hub: MediaSessionHub,
    controller: Rc<RefCell<BrowserController>>,
}

thread_local! {
    static INSTALLED: RefCell<Option<Installed>> = const { RefCell::new(None) };
}

/// Sets the lane up for this run; `interactive` is false for screenshot and background runs.
pub(crate) fn install(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    interactive: bool,
) {
    let (center, power) = system_services(interactive);
    if !interactive && std::env::var_os(START_MUTED_VARIABLE).is_some_and(|value| value == "1") {
        // Diagnostic for automated runs: the start-up tabs begin muted, as if the user had
        // muted them, so a test page can check what muting does without input.
        let tab_ids = controller
            .borrow()
            .tabs()
            .map(|tab| tab.id())
            .collect::<Vec<_>>();
        for tab_id in tab_ids {
            controller.borrow_mut().toggle_audio_muted(tab_id);
        }
        log::info!("{START_MUTED_VARIABLE}=1: the start-up tabs are muted");
    }
    INSTALLED.with(|installed| {
        *installed.borrow_mut() = Some(Installed {
            hub: MediaSessionHub::new(center, power),
            controller: Rc::clone(&controller),
        });
    });

    let weak_window = window.as_weak();
    window.on_toggle_tab_muted(move |index| {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        let tab_id = usize::try_from(index)
            .ok()
            .and_then(|index| controller.borrow().tabs().nth(index).map(|tab| tab.id()));
        if let Some(tab_id) = tab_id {
            toggle_mute(&window, &controller, tab_id);
        }
    });
}

pub(crate) fn toggle_mute(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
    tab_id: talaria_core::TabId,
) {
    if controller.borrow_mut().toggle_audio_muted(tab_id) {
        sync_ui(window, &controller.borrow());
        window.window().request_redraw();
    }
}

/// Called by the engine after each pump that applied browser events. Returns true if it
/// queued commands for the engine.
pub(crate) fn after_pump(controller: &mut BrowserController) -> bool {
    INSTALLED.with(|installed| {
        installed
            .borrow_mut()
            .as_mut()
            .is_some_and(|installed| installed.hub.update(controller, Instant::now()))
    })
}

/// A media key or Now Playing control; runs on the event loop.
pub(crate) fn on_remote_command(command: RemoteCommand) {
    let queued = INSTALLED.with(|installed| {
        let mut installed = installed.borrow_mut();
        let Some(installed) = installed.as_mut() else {
            return false;
        };
        let Ok(mut controller) = installed.controller.try_borrow_mut() else {
            // Busy: try again from a fresh turn of the event loop.
            let _queued = slint::invoke_from_event_loop(move || on_remote_command(command));
            return false;
        };
        installed.hub.remote_command(&mut controller, command)
    });
    if queued {
        crate::engine::request_pump();
    }
}

/// Clears Now Playing and releases the display; call once when the browser quits.
pub(crate) fn shut_down() {
    INSTALLED.with(|installed| {
        if let Some(mut installed) = installed.borrow_mut().take() {
            installed.hub.shut_down();
        }
    });
}

pub(crate) fn tab_audio_state(audio: TabAudio) -> TabAudioState {
    match audio {
        TabAudio::Silent => TabAudioState::Silent,
        TabAudio::Playing => TabAudioState::Playing,
        TabAudio::Muted => TabAudioState::Muted,
    }
}

fn system_services(interactive: bool) -> (Box<dyn NowPlayingCenter>, Box<dyn PowerAssertions>) {
    #[cfg(target_os = "macos")]
    if interactive {
        return (
            Box::new(macos::SystemNowPlaying::new(|command| {
                // The command centre may call from any thread; hand the command to the event loop.
                let _queued = slint::invoke_from_event_loop(move || on_remote_command(command));
            })),
            Box::new(macos::SystemPowerAssertions::default()),
        );
    }
    let _ = interactive;
    (
        Box::new(LoggingNowPlaying),
        Box::new(LoggingPowerAssertions),
    )
}

/// Stands in for the system's Now Playing in automated runs (and where there is none): logs
/// what would have been published, registers nothing.
struct LoggingNowPlaying;

impl NowPlayingCenter for LoggingNowPlaying {
    fn publish(&mut self, info: &NowPlayingInfo) {
        log::info!(
            "now playing (not published to the system): tab {} {:?} by {:?} on {:?}, {}, \
             duration {:?}, elapsed {:?}, rate {}",
            info.tab_id.value(),
            info.title,
            info.artist,
            info.album,
            if info.playing { "playing" } else { "paused" },
            info.duration,
            info.elapsed,
            info.playback_rate
        );
    }

    fn clear(&mut self) {
        log::info!("now playing (not published to the system): cleared");
    }
}

/// Stands in for the power assertion in automated runs (and where there is none).
struct LoggingPowerAssertions;

impl PowerAssertions for LoggingPowerAssertions {
    fn prevent_display_sleep(&mut self) -> bool {
        log::info!("display sleep: would be prevented now (no power assertion in this run)");
        true
    }

    fn allow_display_sleep(&mut self) {
        log::info!("display sleep: would be allowed again now");
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use talaria_core::{
        BrowserEvent, MediaPlaybackState, MediaProbe, MediaSessionAction, TabCommand, TabId,
    };

    use super::*;

    #[derive(Debug, PartialEq)]
    enum Call {
        Publish(u64, String, bool),
        Clear,
        Prevent,
        Allow,
    }

    type Calls = Rc<RefCell<Vec<Call>>>;

    struct FakeCenter(Calls);

    impl NowPlayingCenter for FakeCenter {
        fn publish(&mut self, info: &NowPlayingInfo) {
            self.0.borrow_mut().push(Call::Publish(
                info.tab_id.value(),
                info.title.clone(),
                info.playing,
            ));
        }
        fn clear(&mut self) {
            self.0.borrow_mut().push(Call::Clear);
        }
    }

    struct FakePower(Calls);

    impl PowerAssertions for FakePower {
        fn prevent_display_sleep(&mut self) -> bool {
            self.0.borrow_mut().push(Call::Prevent);
            true
        }
        fn allow_display_sleep(&mut self) {
            self.0.borrow_mut().push(Call::Allow);
        }
    }

    fn hub() -> (MediaSessionHub, Calls) {
        let calls = Calls::default();
        (
            MediaSessionHub::new(
                Box::new(FakeCenter(Rc::clone(&calls))),
                Box::new(FakePower(Rc::clone(&calls))),
            ),
            calls,
        )
    }

    fn ready(controller: &mut BrowserController, tab_id: TabId, title: &str) {
        controller.handle_event(
            tab_id,
            BrowserEvent::AddressChanged {
                url: format!("https://{title}.test/"),
            },
        );
        controller.handle_event(
            tab_id,
            BrowserEvent::PageTitleChanged {
                title: Some(title.to_owned()),
            },
        );
    }

    fn drain(controller: &mut BrowserController) -> Vec<TabCommand> {
        std::iter::from_fn(|| controller.next_command()).collect()
    }

    #[test]
    fn a_playing_tab_is_probed_then_published_then_answers_media_keys() {
        let (mut hub, calls) = hub();
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        ready(&mut controller, tab_id, "song");
        let now = Instant::now();

        controller.handle_event(
            tab_id,
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Playing,
            },
        );
        assert!(hub.update(&mut controller, now));
        assert_eq!(
            drain(&mut controller),
            [TabCommand::new(tab_id, BrowserCommand::ProbeMedia)]
        );
        assert!(calls.borrow().is_empty(), "nothing before the probe");

        controller.handle_event(
            tab_id,
            BrowserEvent::MediaProbed {
                probe: MediaProbe {
                    playing: 1,
                    audible: 1,
                    fullscreen_video: false,
                },
            },
        );
        assert!(!hub.update(&mut controller, now));
        assert_eq!(
            *calls.borrow(),
            [Call::Publish(tab_id.value(), "song".to_owned(), true)]
        );

        assert!(hub.remote_command(&mut controller, RemoteCommand::TogglePlayPause));
        assert_eq!(
            drain(&mut controller),
            [TabCommand::new(
                tab_id,
                BrowserCommand::MediaSessionAction(MediaSessionAction::Pause)
            )]
        );
    }

    #[test]
    fn media_keys_without_now_playing_do_nothing() {
        let (mut hub, _) = hub();
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        ready(&mut controller, tab_id, "quiet");

        assert!(!hub.remote_command(&mut controller, RemoteCommand::Play));
        assert!(drain(&mut controller).is_empty());
    }

    #[test]
    fn quitting_clears_now_playing_and_releases_the_display() {
        let (mut hub, calls) = hub();
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        ready(&mut controller, tab_id, "film");
        for event in [
            BrowserEvent::FullscreenChanged { fullscreen: true },
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Playing,
            },
            BrowserEvent::MediaProbed {
                probe: MediaProbe {
                    playing: 1,
                    audible: 1,
                    fullscreen_video: true,
                },
            },
        ] {
            controller.handle_event(tab_id, event);
        }
        hub.update(&mut controller, Instant::now());

        hub.shut_down();
        hub.shut_down();

        assert_eq!(
            *calls.borrow(),
            [
                Call::Publish(tab_id.value(), "film".to_owned(), true),
                Call::Prevent,
                Call::Allow,
                Call::Clear,
            ]
        );
    }

    #[test]
    fn quitting_with_nothing_held_touches_nothing() {
        let (mut hub, calls) = hub();

        hub.shut_down();

        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn tab_audio_maps_to_the_ui() {
        assert_eq!(tab_audio_state(TabAudio::Silent), TabAudioState::Silent);
        assert_eq!(tab_audio_state(TabAudio::Playing), TabAudioState::Playing);
        assert_eq!(tab_audio_state(TabAudio::Muted), TabAudioState::Muted);
    }
}
