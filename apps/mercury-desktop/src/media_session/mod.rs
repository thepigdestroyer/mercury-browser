// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
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

/// Which implementation stands behind a service trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceKind {
    /// The operating system's: only in an interactive launch.
    System,
    /// Logs what it would do: automated runs, and platforms without the service.
    Logging,
    #[cfg(test)]
    Test,
}
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

    /// Brings the system services in line with the tabs, and queues the probes that are due.
    /// `engine_session` is the tab Servo sends media-session actions to.
    pub(crate) fn update(
        &mut self,
        controller: &mut BrowserController,
        engine_session: Option<talaria_core::TabId>,
        now: Instant,
    ) {
        for tab_id in self.probes.due(controller, now) {
            controller.queue_command_for(tab_id, BrowserCommand::ProbeMedia);
        }
        match self.now_playing.update(controller, engine_session, now) {
            Some(NowPlayingChange::Publish(info)) => self.center.publish(&info),
            Some(NowPlayingChange::Clear) => self.center.clear(),
            None => {}
        }
        self.display_sleep.update(
            DisplaySleepPolicy::wanted(controller),
            self.power.as_mut(),
            now,
        );
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
        controller.queue_command_for(
            tab_id,
            BrowserCommand::MediaSessionAction {
                action,
                through_engine: self.now_playing.through_engine(),
            },
        )
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
/// Only read in screenshot and background runs: browser commands to give the active tab at
/// times after start-up, as `reload@4000,back@6500` (milliseconds), so a run can check what a
/// reload or Back does to the media state without input.
const DIAGNOSTIC_COMMANDS_VARIABLE: &str = "MERCURY_MEDIA_DIAGNOSTIC_COMMANDS";

fn diagnostic_commands(text: &str) -> Vec<(std::time::Duration, BrowserCommand)> {
    text.split(',')
        .filter_map(|entry| {
            let (name, millis) = entry.trim().split_once('@')?;
            let command = match name {
                "reload" => BrowserCommand::Reload,
                "back" => BrowserCommand::GoBack,
                "forward" => BrowserCommand::GoForward,
                _ => return None,
            };
            Some((
                std::time::Duration::from_millis(millis.parse().ok()?),
                command,
            ))
        })
        .collect()
}

struct Installed {
    hub: MediaSessionHub,
    controller: Rc<RefCell<BrowserController>>,
}

thread_local! {
    static INSTALLED: RefCell<Option<Installed>> = const { RefCell::new(None) };
}

/// The system's Now Playing, media keys and power assertions are only used in an interactive
/// launch, never in screenshot or background runs.
pub(crate) fn uses_system_services(options: &crate::cli::LaunchOptions) -> bool {
    !options.is_automated()
}

/// Sets the lane up for this run; `interactive` comes from `uses_system_services`.
pub(crate) fn install(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    interactive: bool,
) {
    let (center, power) = system_services(interactive);
    log::info!(
        "media session: Now Playing {:?}, display sleep {:?}",
        center.kind(),
        power.kind()
    );
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
    if !interactive && let Some(text) = std::env::var_os(DIAGNOSTIC_COMMANDS_VARIABLE) {
        for (delay, command) in diagnostic_commands(&text.to_string_lossy()) {
            let controller = Rc::downgrade(&controller);
            slint::Timer::single_shot(delay, move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                let Some(controller) = controller.upgrade() else {
                    return;
                };
                log::info!("{DIAGNOSTIC_COMMANDS_VARIABLE}: {command:?} to the active tab");
                controller.borrow_mut().queue_command(command);
                crate::engine::request_pump();
            });
        }
    }
    INSTALLED.with(|installed| {
        *installed.borrow_mut() = Some(Installed {
            hub: MediaSessionHub::new(center, power),
            controller: Rc::clone(&controller),
        });
    });

    let weak_window = window.as_weak();
    window.on_toggle_tab_muted(move |index| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
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

/// Called by the engine on every pump, before it dispatches commands (so the probes it queues
/// go out in the same pump).
pub(crate) fn after_pump(
    controller: &mut BrowserController,
    engine_session: Option<talaria_core::TabId>,
) {
    INSTALLED.with(|installed| {
        if let Some(installed) = installed.borrow_mut().as_mut() {
            installed
                .hub
                .update(controller, engine_session, Instant::now());
        }
    });
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
            let _queued = slint::invoke_from_event_loop(move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                on_remote_command(command);
            });
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
                let _queued = slint::invoke_from_event_loop(move || {
                    let _slint = crate::callback_scope::InSlintCallback::enter();
                    on_remote_command(command);
                });
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
    fn kind(&self) -> ServiceKind {
        ServiceKind::Logging
    }

    fn publish(&mut self, info: &NowPlayingInfo) {
        log::info!(
            "now playing (not published to the system): tab {}, {}, duration {:?}, \
             elapsed {:?}, rate {}, next/previous {}",
            info.tab_id.value(),
            if info.playing { "playing" } else { "paused" },
            info.duration,
            info.elapsed,
            info.playback_rate,
            if info.can_skip { "on" } else { "off" }
        );
        // The page's own text, cut short, only at debug level.
        log::debug!(
            "now playing (not published to the system): {:?} by {:?} on {:?}",
            log_excerpt(&info.title),
            log_excerpt(&info.artist),
            log_excerpt(&info.album)
        );
    }

    fn clear(&mut self) {
        log::info!("now playing (not published to the system): cleared");
    }
}

/// One line, at most 64 characters (the text is already free of controls).
fn log_excerpt(text: &str) -> String {
    let mut line = text.chars().take(64).collect::<String>();
    if text.chars().count() > 64 {
        line.push('…');
    }
    line
}

/// Stands in for the power assertion in automated runs (and where there is none).
struct LoggingPowerAssertions;

impl PowerAssertions for LoggingPowerAssertions {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Logging
    }

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
        fn kind(&self) -> ServiceKind {
            ServiceKind::Test
        }
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
        fn kind(&self) -> ServiceKind {
            ServiceKind::Test
        }
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
        hub.update(&mut controller, None, now);
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
                    unseen: false,
                    fullscreen_video: false,
                },
            },
        );
        hub.update(&mut controller, Some(tab_id), now);
        assert!(drain(&mut controller).is_empty());
        assert_eq!(
            *calls.borrow(),
            [Call::Publish(tab_id.value(), "song".to_owned(), true)]
        );

        assert!(hub.remote_command(&mut controller, RemoteCommand::TogglePlayPause));
        assert_eq!(
            drain(&mut controller),
            [TabCommand::new(
                tab_id,
                BrowserCommand::MediaSessionAction {
                    action: MediaSessionAction::Pause,
                    through_engine: true,
                }
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
                    unseen: false,
                    fullscreen_video: true,
                },
            },
        ] {
            controller.handle_event(tab_id, event);
        }
        hub.update(&mut controller, None, Instant::now());

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

    #[test]
    fn only_interactive_launches_use_the_system_services() {
        use crate::cli::{LaunchOptions, ScreenshotOptions};

        assert!(uses_system_services(&LaunchOptions::default()));
        assert!(!uses_system_services(&LaunchOptions {
            background: true,
            ..LaunchOptions::default()
        }));
        assert!(!uses_system_services(&LaunchOptions {
            screenshot: Some(ScreenshotOptions {
                path: "shot.png".into(),
                delay: std::time::Duration::ZERO,
            }),
            ..LaunchOptions::default()
        }));

        let (center, power) = system_services(false);
        assert_eq!(center.kind(), ServiceKind::Logging);
        assert_eq!(power.kind(), ServiceKind::Logging);
    }

    #[test]
    fn a_muted_tab_is_muted_again_after_a_navigation_and_never_published() {
        let (mut hub, calls) = hub();
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        ready(&mut controller, tab_id, "first");
        controller.toggle_audio_muted(tab_id);
        let start = Instant::now();
        hub.update(&mut controller, None, start);
        assert_eq!(
            drain(&mut controller),
            [TabCommand::new(
                tab_id,
                BrowserCommand::SetAudioMuted { muted: true }
            )]
        );

        // The tab goes to another page, which starts playing at once.
        controller.handle_event(
            tab_id,
            BrowserEvent::LoadStateChanged {
                state: talaria_core::LoadState::Loading,
            },
        );
        controller.handle_event(tab_id, BrowserEvent::PageLoadStarted);
        hub.update(&mut controller, None, start);
        ready(&mut controller, tab_id, "second");
        controller.handle_event(
            tab_id,
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Playing,
            },
        );
        hub.update(&mut controller, Some(tab_id), start);
        let mute = TabCommand::new(tab_id, BrowserCommand::SetAudioMuted { muted: true });
        assert_eq!(
            drain(&mut controller),
            [
                mute.clone(),
                mute.clone(),
                mute,
                TabCommand::new(tab_id, BrowserCommand::ProbeMedia),
            ]
        );

        // The page un-muted its audio: the probe hears it, the mute goes out again, and the
        // muted tab is not published.
        controller.handle_event(
            tab_id,
            BrowserEvent::MediaProbed {
                probe: MediaProbe {
                    playing: 1,
                    audible: 1,
                    unseen: false,
                    fullscreen_video: false,
                },
            },
        );
        hub.update(&mut controller, Some(tab_id), start);
        assert_eq!(
            drain(&mut controller),
            [TabCommand::new(
                tab_id,
                BrowserCommand::SetAudioMuted { muted: true }
            )]
        );
        assert!(calls.borrow().is_empty(), "{:?}", calls.borrow());
    }

    #[test]
    fn diagnostic_commands_are_read_and_bad_entries_skipped() {
        use std::time::Duration;

        assert_eq!(
            diagnostic_commands("reload@4000, back@6500,forward@7000"),
            [
                (Duration::from_millis(4000), BrowserCommand::Reload),
                (Duration::from_millis(6500), BrowserCommand::GoBack),
                (Duration::from_millis(7000), BrowserCommand::GoForward),
            ]
        );
        assert_eq!(diagnostic_commands("navigate@1, reload, reload@x, @5,"), []);
    }
}
