//! Which tab the system's Now Playing shows, and where media keys go.

use std::collections::HashSet;

use talaria_core::{
    BrowserController, BrowserState, MediaPlaybackState, MediaSessionAction, TabId,
};

/// A consecutive position report this far from the last one is a seek, not playback (Servo
/// reports the position about ten times a second while playing).
const SEEK_JUMP_SECONDS: f64 = 1.5;

/// What the system's media controls ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteCommand {
    Play,
    Pause,
    TogglePlayPause,
    NextTrack,
    PreviousTrack,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NowPlayingInfo {
    pub tab_id: TabId,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    /// Seconds; `None` when unknown or infinite (a live stream).
    pub duration: Option<f64>,
    pub elapsed: Option<f64>,
    pub playback_rate: f64,
}

impl NowPlayingInfo {
    /// Equal apart from the elapsed time, which the system extrapolates from the rate.
    fn same_item_and_state(&self, other: &Self) -> bool {
        Self {
            elapsed: None,
            ..self.clone()
        } == Self {
            elapsed: None,
            ..other.clone()
        }
    }
}

/// The system's Now Playing information and remote commands. The macOS implementation is
/// only created in an interactive launch; automated runs and tests use fakes.
pub(crate) trait NowPlayingCenter {
    /// Shows `info` and (on the first call after `clear`) starts taking remote commands.
    fn publish(&mut self, info: &NowPlayingInfo);
    /// Removes the information and stops taking remote commands.
    fn clear(&mut self);
}

#[derive(Debug, PartialEq)]
pub(crate) enum NowPlayingChange {
    Publish(NowPlayingInfo),
    Clear,
}

/// Now Playing follows the tab that most recently started playing audibly, for as long as it
/// keeps a media session (paused included, so that Play resumes it). When that tab stops, is
/// navigated or closed, the tab that started before it takes over if it still has a session.
#[derive(Debug, Default)]
pub(crate) struct NowPlayingPolicy {
    /// Tabs in the order they last started playing audibly; the last one is shown.
    started: Vec<TabId>,
    playing: HashSet<TabId>,
    published: Option<NowPlayingInfo>,
    last_position: Option<(TabId, f64)>,
}

impl NowPlayingPolicy {
    pub(crate) fn update(&mut self, controller: &BrowserController) -> Option<NowPlayingChange> {
        let mut playing_now = HashSet::new();
        for tab in controller.tabs() {
            if plays_audibly(tab.state()) {
                playing_now.insert(tab.id());
                if !self.playing.contains(&tab.id()) {
                    self.started.retain(|tab_id| *tab_id != tab.id());
                    self.started.push(tab.id());
                }
            }
        }
        self.playing = playing_now;
        self.started.retain(|tab_id| {
            controller.tabs().any(|tab| {
                tab.id() == *tab_id && tab.state().media().playback() != MediaPlaybackState::None
            })
        });

        let owner = self.started.last().and_then(|tab_id| {
            controller
                .tabs()
                .find(|tab| tab.id() == *tab_id)
                .map(|tab| (tab.id(), tab.state()))
        });
        let Some((tab_id, state)) = owner else {
            self.last_position = None;
            return self.published.take().map(|_| NowPlayingChange::Clear);
        };

        let info = now_playing_info(tab_id, state);
        let sought = match (self.last_position, info.elapsed) {
            (Some((last_tab_id, last)), Some(elapsed)) if last_tab_id == tab_id => {
                (elapsed - last).abs() > SEEK_JUMP_SECONDS
            }
            _ => false,
        };
        self.last_position = info.elapsed.map(|elapsed| (tab_id, elapsed));
        let changed = self
            .published
            .as_ref()
            .is_none_or(|published| !published.same_item_and_state(&info));
        if !changed && !sought {
            return None;
        }
        self.published = Some(info.clone());
        Some(NowPlayingChange::Publish(info))
    }

    /// The tab whose media is in Now Playing.
    pub(crate) fn owner(&self) -> Option<TabId> {
        self.published.as_ref().map(|info| info.tab_id)
    }

    /// Where a remote command goes: always the tab shown in Now Playing.
    pub(crate) fn action_for(&self, command: RemoteCommand) -> Option<(TabId, MediaSessionAction)> {
        let info = self.published.as_ref()?;
        let action = match command {
            RemoteCommand::Play => MediaSessionAction::Play,
            RemoteCommand::Pause => MediaSessionAction::Pause,
            RemoteCommand::TogglePlayPause if info.playing => MediaSessionAction::Pause,
            RemoteCommand::TogglePlayPause => MediaSessionAction::Play,
            RemoteCommand::NextTrack => MediaSessionAction::NextTrack,
            RemoteCommand::PreviousTrack => MediaSessionAction::PreviousTrack,
        };
        Some((info.tab_id, action))
    }
}

/// Playing, and a probe has confirmed that the top-level document's playing media is not all
/// muted. A muted autoplaying background video therefore never takes the media keys.
fn plays_audibly(state: &BrowserState) -> bool {
    let media = state.media();
    media.probe().is_some() && media.is_audible()
}

fn now_playing_info(tab_id: TabId, state: &BrowserState) -> NowPlayingInfo {
    let media = state.media();
    let metadata = media.metadata();
    // Servo uses the document's address as the title of media that has none.
    let title = metadata
        .map(|metadata| metadata.title.as_str())
        .filter(|title| !title.trim().is_empty() && *title != state.url())
        .unwrap_or(state.page_title())
        .to_owned();
    let playing = media.is_playing();
    let position = media.position();
    let finite = |seconds: f64| (seconds.is_finite() && seconds >= 0.0).then_some(seconds);
    NowPlayingInfo {
        tab_id,
        title,
        artist: metadata
            .map(|metadata| metadata.artist.clone())
            .unwrap_or_default(),
        album: metadata
            .map(|metadata| metadata.album.clone())
            .unwrap_or_default(),
        playing,
        duration: position
            .and_then(|position| finite(position.duration()))
            .filter(|duration| *duration > 0.0),
        elapsed: position.and_then(|position| finite(position.position())),
        playback_rate: match (playing, position) {
            (false, _) => 0.0,
            (true, Some(position)) if position.playback_rate().is_finite() => {
                position.playback_rate()
            }
            (true, _) => 1.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use talaria_core::{BrowserEvent, LoadState, MediaMetadata, MediaPosition, MediaProbe};

    use super::*;

    fn event(controller: &mut BrowserController, tab_id: TabId, event: BrowserEvent) {
        assert!(controller.handle_event(tab_id, event));
    }

    fn page(controller: &mut BrowserController, tab_id: TabId, url: &str, title: &str) {
        event(
            controller,
            tab_id,
            BrowserEvent::AddressChanged {
                url: url.to_owned(),
            },
        );
        event(
            controller,
            tab_id,
            BrowserEvent::PageTitleChanged {
                title: Some(title.to_owned()),
            },
        );
    }

    fn playback(controller: &mut BrowserController, tab_id: TabId, state: MediaPlaybackState) {
        event(
            controller,
            tab_id,
            BrowserEvent::MediaPlaybackChanged { state },
        );
    }

    fn probed(controller: &mut BrowserController, tab_id: TabId, playing: u32, audible: u32) {
        event(
            controller,
            tab_id,
            BrowserEvent::MediaProbed {
                probe: MediaProbe {
                    playing,
                    audible,
                    fullscreen_video: false,
                },
            },
        );
    }

    fn start_audible(controller: &mut BrowserController, tab_id: TabId) {
        playback(controller, tab_id, MediaPlaybackState::Playing);
        probed(controller, tab_id, 1, 1);
    }

    fn position(controller: &mut BrowserController, tab_id: TabId, seconds: f64) {
        event(
            controller,
            tab_id,
            BrowserEvent::MediaPositionChanged {
                position: MediaPosition::new(240.0, 1.0, seconds),
            },
        );
    }

    fn published(change: Option<NowPlayingChange>) -> NowPlayingInfo {
        match change {
            Some(NowPlayingChange::Publish(info)) => info,
            other => panic!("expected a publication, got {other:?}"),
        }
    }

    fn two_tabs() -> (BrowserController, TabId, TabId) {
        let mut controller = BrowserController::default();
        let first = controller.active_tab_id();
        page(&mut controller, first, "https://a.test/", "First page");
        let second = controller.open_tab();
        page(&mut controller, second, "https://b.test/", "Second page");
        (controller, first, second)
    }

    #[test]
    fn nothing_is_published_while_nothing_plays() {
        let (controller, _, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();

        assert_eq!(policy.update(&controller), None);
        assert_eq!(policy.owner(), None);
        assert_eq!(policy.action_for(RemoteCommand::Play), None);
    }

    #[test]
    fn a_playing_tab_is_published_with_its_metadata() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        event(
            &mut controller,
            first,
            BrowserEvent::MediaMetadataChanged {
                metadata: MediaMetadata {
                    title: "Song".to_owned(),
                    artist: "Band".to_owned(),
                    album: "Record".to_owned(),
                },
            },
        );
        start_audible(&mut controller, first);
        position(&mut controller, first, 12.0);

        assert_eq!(
            published(policy.update(&controller)),
            NowPlayingInfo {
                tab_id: first,
                title: "Song".to_owned(),
                artist: "Band".to_owned(),
                album: "Record".to_owned(),
                playing: true,
                duration: Some(240.0),
                elapsed: Some(12.0),
                playback_rate: 1.0,
            }
        );
        assert_eq!(policy.owner(), Some(first));
    }

    #[test]
    fn media_without_a_title_is_shown_under_the_page_title() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        // What Servo sends for a media element without a title.
        event(
            &mut controller,
            first,
            BrowserEvent::MediaMetadataChanged {
                metadata: MediaMetadata {
                    title: "https://a.test/".to_owned(),
                    ..MediaMetadata::default()
                },
            },
        );
        start_audible(&mut controller, first);

        let info = published(policy.update(&controller));
        assert_eq!(info.title, "First page");
        assert_eq!(info.duration, None);
        assert_eq!(info.elapsed, None);
    }

    #[test]
    fn a_tab_is_not_published_until_a_probe_confirms_it_is_audible() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        playback(&mut controller, first, MediaPlaybackState::Playing);

        assert_eq!(policy.update(&controller), None);
        // A muted background video.
        probed(&mut controller, first, 1, 0);
        assert_eq!(policy.update(&controller), None);

        probed(&mut controller, first, 1, 1);
        assert_eq!(published(policy.update(&controller)).tab_id, first);
    }

    #[test]
    fn the_most_recently_started_tab_wins() {
        let (mut controller, first, second) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, first);
        assert_eq!(published(policy.update(&controller)).tab_id, first);

        start_audible(&mut controller, second);
        assert_eq!(published(policy.update(&controller)).tab_id, second);

        // The first tab keeps playing; it does not take Now Playing back.
        position(&mut controller, first, 3.0);
        assert_eq!(policy.update(&controller), None);
        assert_eq!(policy.owner(), Some(second));

        // Starting again after a pause makes it the most recent.
        playback(&mut controller, first, MediaPlaybackState::Paused);
        assert_eq!(policy.update(&controller), None);
        playback(&mut controller, first, MediaPlaybackState::Playing);
        assert_eq!(published(policy.update(&controller)).tab_id, first);
    }

    #[test]
    fn a_paused_tab_stays_published_as_paused() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, first);
        policy.update(&controller);

        playback(&mut controller, first, MediaPlaybackState::Paused);
        let info = published(policy.update(&controller));

        assert_eq!(info.tab_id, first);
        assert!(!info.playing);
        assert_eq!(info.playback_rate, 0.0);
    }

    #[test]
    fn stopping_clears_now_playing_or_hands_it_to_the_previous_tab() {
        let (mut controller, first, second) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, first);
        policy.update(&controller);
        start_audible(&mut controller, second);
        policy.update(&controller);

        playback(&mut controller, second, MediaPlaybackState::None);
        assert_eq!(published(policy.update(&controller)).tab_id, first);

        playback(&mut controller, first, MediaPlaybackState::None);
        assert_eq!(policy.update(&controller), Some(NowPlayingChange::Clear));
        assert_eq!(policy.owner(), None);
        assert_eq!(policy.update(&controller), None);
    }

    #[test]
    fn closing_or_navigating_the_tab_clears_now_playing() {
        let (mut controller, first, second) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, second);
        policy.update(&controller);

        assert!(controller.close_tab(second));
        assert_eq!(policy.update(&controller), Some(NowPlayingChange::Clear));

        start_audible(&mut controller, first);
        policy.update(&controller);
        event(
            &mut controller,
            first,
            BrowserEvent::LoadStateChanged {
                state: LoadState::Loading,
            },
        );
        assert_eq!(policy.update(&controller), Some(NowPlayingChange::Clear));
    }

    #[test]
    fn steady_playback_is_not_republished_but_a_seek_is() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, first);
        position(&mut controller, first, 10.0);
        policy.update(&controller);

        position(&mut controller, first, 10.1);
        assert_eq!(policy.update(&controller), None);
        position(&mut controller, first, 10.2);
        assert_eq!(policy.update(&controller), None);

        position(&mut controller, first, 95.0);
        assert_eq!(published(policy.update(&controller)).elapsed, Some(95.0));
    }

    #[test]
    fn remote_commands_go_to_the_published_tab() {
        let (mut controller, first, second) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, first);
        policy.update(&controller);
        start_audible(&mut controller, second);
        policy.update(&controller);

        assert_eq!(
            policy.action_for(RemoteCommand::TogglePlayPause),
            Some((second, MediaSessionAction::Pause))
        );
        assert_eq!(
            policy.action_for(RemoteCommand::NextTrack),
            Some((second, MediaSessionAction::NextTrack))
        );
        assert_eq!(
            policy.action_for(RemoteCommand::PreviousTrack),
            Some((second, MediaSessionAction::PreviousTrack))
        );
        assert_eq!(
            policy.action_for(RemoteCommand::Play),
            Some((second, MediaSessionAction::Play))
        );

        playback(&mut controller, second, MediaPlaybackState::Paused);
        policy.update(&controller);
        assert_eq!(
            policy.action_for(RemoteCommand::TogglePlayPause),
            Some((second, MediaSessionAction::Play))
        );
        assert_eq!(
            policy.action_for(RemoteCommand::Pause),
            Some((second, MediaSessionAction::Pause))
        );
    }
}
