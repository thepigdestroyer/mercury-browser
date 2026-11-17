//! Which tab the system's Now Playing shows, and where media keys go.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use talaria_core::{
    BrowserController, BrowserState, MediaPlaybackState, MediaSessionAction, TabId,
};

/// A consecutive position report this far from the last one is a seek, not playback (Servo
/// reports the position about ten times a second while playing).
const SEEK_JUMP_SECONDS: f64 = 1.5;

/// Servo reports a document's playback as its last element's, so a page with several elements
/// flaps between Playing and Paused. Playing again within this time of last playing is not a new
/// start, so such a tab does not take Now Playing back from another.
const FLAP_HOLD: Duration = Duration::from_secs(1);

/// Each text shown in Control Centre is cut to this many characters.
pub(crate) const MAX_TEXT_CHARACTERS: usize = 256;

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
    /// Next and previous can reach the page: Servo's single media session is this tab's.
    pub can_skip: bool,
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
    fn kind(&self) -> super::ServiceKind;
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
    last_playing: HashMap<TabId, Instant>,
    published: Option<NowPlayingInfo>,
    last_position: Option<(TabId, f64)>,
}

impl NowPlayingPolicy {
    /// `engine_session` is the tab Servo sends media-session actions to.
    pub(crate) fn update(
        &mut self,
        controller: &BrowserController,
        engine_session: Option<TabId>,
        now: Instant,
    ) -> Option<NowPlayingChange> {
        let mut playing_now = HashSet::new();
        for tab in controller.tabs() {
            if plays_audibly(tab.state()) {
                playing_now.insert(tab.id());
                let flapped = self
                    .last_playing
                    .get(&tab.id())
                    .is_some_and(|last| now.saturating_duration_since(*last) < FLAP_HOLD);
                if !self.playing.contains(&tab.id()) && !flapped {
                    self.started.retain(|tab_id| *tab_id != tab.id());
                    self.started.push(tab.id());
                }
                self.last_playing.insert(tab.id(), now);
            }
        }
        self.playing = playing_now;
        self.last_playing
            .retain(|tab_id, _| controller.tabs().any(|tab| tab.id() == *tab_id));
        self.started.retain(|tab_id| {
            controller.tabs().any(|tab| {
                tab.id() == *tab_id
                    && tab.state().media().playback() != MediaPlaybackState::None
                    && !tab.state().is_audio_muted()
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

        let info = now_playing_info(tab_id, state, engine_session == Some(tab_id));
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
            RemoteCommand::NextTrack if info.can_skip => MediaSessionAction::NextTrack,
            RemoteCommand::PreviousTrack if info.can_skip => MediaSessionAction::PreviousTrack,
            RemoteCommand::NextTrack | RemoteCommand::PreviousTrack => return None,
        };
        Some((info.tab_id, action))
    }
}

/// Playing in a tab the user has not muted, and a probe has been made and did not show all of
/// it muted. A muted autoplaying background video (on a page without frames) therefore never
/// takes the media keys, and neither does a muted tab whose page un-muted itself.
fn plays_audibly(state: &BrowserState) -> bool {
    let media = state.media();
    !state.is_audio_muted() && media.probe().is_some() && media.is_audible()
}

/// Page text for Control Centre: no control characters or bidirectional overrides, at most
/// `MAX_TEXT_CHARACTERS` characters.
pub(crate) fn clean_text(text: &str) -> String {
    let cleaned = text
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    character,
                    '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
                )
        })
        .collect::<String>();
    let cleaned = cleaned.trim();
    if cleaned.chars().count() <= MAX_TEXT_CHARACTERS {
        return cleaned.to_owned();
    }
    let mut capped = cleaned
        .chars()
        .take(MAX_TEXT_CHARACTERS - 1)
        .collect::<String>();
    capped.push('…');
    capped
}

/// Servo titles untitled media with its document's address, which for media in a frame is the
/// frame's address; such titles are not shown.
fn looks_like_an_address(title: &str) -> bool {
    !title.contains(char::is_whitespace) && title.contains("://")
}

fn host(state: &BrowserState) -> Option<String> {
    url::Url::parse(state.url())
        .ok()?
        .host_str()
        .map(clean_text)
}

fn now_playing_info(tab_id: TabId, state: &BrowserState, can_skip: bool) -> NowPlayingInfo {
    let media = state.media();
    let metadata = media.metadata();
    let title = metadata
        .map(|metadata| clean_text(&metadata.title))
        .filter(|title| !title.is_empty() && !looks_like_an_address(title))
        .unwrap_or_else(|| clean_text(state.page_title()));
    let artist = metadata
        .map(|metadata| clean_text(&metadata.artist))
        .unwrap_or_default();
    let album = metadata
        .map(|metadata| clean_text(&metadata.album))
        .unwrap_or_default();
    // Page-triggered UI names its origin: the host is the artist when the page gives none,
    // otherwise it follows the album.
    let (artist, album) = match host(state) {
        Some(host) if artist.is_empty() => (host, album),
        Some(host) if album.is_empty() => (artist, host),
        Some(host) => {
            // The host must stay visible: the album gives way.
            let suffix = format!(" — {host}");
            let room = MAX_TEXT_CHARACTERS.saturating_sub(suffix.chars().count());
            let album = if album.chars().count() > room {
                let mut cut = album
                    .chars()
                    .take(room.saturating_sub(1))
                    .collect::<String>();
                cut.push('…');
                cut
            } else {
                album
            };
            (artist, format!("{album}{suffix}"))
        }
        None => (artist, album),
    };
    let playing = media.is_playing();
    let position = media.position();
    let finite = |seconds: f64| (seconds.is_finite() && seconds >= 0.0).then_some(seconds);
    NowPlayingInfo {
        tab_id,
        title,
        artist,
        album,
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
        can_skip,
    }
}

#[cfg(test)]
mod tests {
    use talaria_core::{BrowserEvent, LoadState, MediaMetadata, MediaPosition, MediaProbe};

    use std::cell::Cell;
    use std::time::Duration;

    use super::*;

    thread_local! {
        static CLOCK: Cell<Option<Instant>> = const { Cell::new(None) };
        static ENGINE_SESSION: Cell<Option<TabId>> = const { Cell::new(None) };
    }

    /// Each call is two seconds after the previous one, past the flap hold; Servo's media
    /// session is on `ENGINE_SESSION`'s tab.
    fn update(
        policy: &mut NowPlayingPolicy,
        controller: &BrowserController,
    ) -> Option<NowPlayingChange> {
        let now = CLOCK.with(|clock| {
            let now = clock
                .get()
                .map_or_else(Instant::now, |last| last + Duration::from_secs(2));
            clock.set(Some(now));
            now
        });
        policy.update(controller, ENGINE_SESSION.with(Cell::get), now)
    }

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
                    unseen: false,
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

        assert_eq!(update(&mut policy, &controller), None);
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
            published(update(&mut policy, &controller)),
            NowPlayingInfo {
                tab_id: first,
                title: "Song".to_owned(),
                artist: "Band".to_owned(),
                album: "Record — a.test".to_owned(),
                playing: true,
                duration: Some(240.0),
                elapsed: Some(12.0),
                playback_rate: 1.0,
                can_skip: false,
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

        let info = published(update(&mut policy, &controller));
        assert_eq!(info.title, "First page");
        assert_eq!(info.duration, None);
        assert_eq!(info.elapsed, None);
    }

    #[test]
    fn a_tab_is_not_published_until_a_probe_confirms_it_is_audible() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        playback(&mut controller, first, MediaPlaybackState::Playing);

        assert_eq!(update(&mut policy, &controller), None);
        // A muted background video.
        probed(&mut controller, first, 1, 0);
        assert_eq!(update(&mut policy, &controller), None);

        probed(&mut controller, first, 1, 1);
        assert_eq!(published(update(&mut policy, &controller)).tab_id, first);
    }

    #[test]
    fn the_most_recently_started_tab_wins() {
        let (mut controller, first, second) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, first);
        assert_eq!(published(update(&mut policy, &controller)).tab_id, first);

        start_audible(&mut controller, second);
        assert_eq!(published(update(&mut policy, &controller)).tab_id, second);

        // The first tab keeps playing; it does not take Now Playing back.
        position(&mut controller, first, 3.0);
        assert_eq!(update(&mut policy, &controller), None);
        assert_eq!(policy.owner(), Some(second));

        // Starting again after a pause makes it the most recent.
        playback(&mut controller, first, MediaPlaybackState::Paused);
        assert_eq!(update(&mut policy, &controller), None);
        playback(&mut controller, first, MediaPlaybackState::Playing);
        assert_eq!(published(update(&mut policy, &controller)).tab_id, first);
    }

    #[test]
    fn a_paused_tab_stays_published_as_paused() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, first);
        update(&mut policy, &controller);

        playback(&mut controller, first, MediaPlaybackState::Paused);
        let info = published(update(&mut policy, &controller));

        assert_eq!(info.tab_id, first);
        assert!(!info.playing);
        assert_eq!(info.playback_rate, 0.0);
    }

    #[test]
    fn stopping_clears_now_playing_or_hands_it_to_the_previous_tab() {
        let (mut controller, first, second) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, first);
        update(&mut policy, &controller);
        start_audible(&mut controller, second);
        update(&mut policy, &controller);

        playback(&mut controller, second, MediaPlaybackState::None);
        assert_eq!(published(update(&mut policy, &controller)).tab_id, first);

        playback(&mut controller, first, MediaPlaybackState::None);
        assert_eq!(
            update(&mut policy, &controller),
            Some(NowPlayingChange::Clear)
        );
        assert_eq!(policy.owner(), None);
        assert_eq!(update(&mut policy, &controller), None);
    }

    #[test]
    fn closing_or_navigating_the_tab_clears_now_playing() {
        let (mut controller, first, second) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, second);
        update(&mut policy, &controller);

        assert!(controller.close_tab(second));
        assert_eq!(
            update(&mut policy, &controller),
            Some(NowPlayingChange::Clear)
        );

        start_audible(&mut controller, first);
        update(&mut policy, &controller);
        event(
            &mut controller,
            first,
            BrowserEvent::LoadStateChanged {
                state: LoadState::Loading,
            },
        );
        assert_eq!(
            update(&mut policy, &controller),
            None,
            "not before the commit"
        );
        page(&mut controller, first, "https://c.test/", "Next page");
        assert_eq!(
            update(&mut policy, &controller),
            Some(NowPlayingChange::Clear)
        );
    }

    #[test]
    fn steady_playback_is_not_republished_but_a_seek_is() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, first);
        position(&mut controller, first, 10.0);
        update(&mut policy, &controller);

        position(&mut controller, first, 10.1);
        assert_eq!(update(&mut policy, &controller), None);
        position(&mut controller, first, 10.2);
        assert_eq!(update(&mut policy, &controller), None);

        position(&mut controller, first, 95.0);
        assert_eq!(
            published(update(&mut policy, &controller)).elapsed,
            Some(95.0)
        );
    }

    #[test]
    fn remote_commands_go_to_the_published_tab() {
        let (mut controller, first, second) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        ENGINE_SESSION.with(|session| session.set(Some(second)));
        start_audible(&mut controller, first);
        update(&mut policy, &controller);
        start_audible(&mut controller, second);
        update(&mut policy, &controller);

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
        update(&mut policy, &controller);
        assert_eq!(
            policy.action_for(RemoteCommand::TogglePlayPause),
            Some((second, MediaSessionAction::Play))
        );
        assert_eq!(
            policy.action_for(RemoteCommand::Pause),
            Some((second, MediaSessionAction::Pause))
        );
    }

    #[test]
    fn a_muted_tab_never_takes_now_playing() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        controller.toggle_audio_muted(first);
        // The page un-muted its media: the probe hears it, the tab is still muted.
        start_audible(&mut controller, first);

        assert_eq!(update(&mut policy, &controller), None);
        assert_eq!(policy.action_for(RemoteCommand::Play), None);
    }

    #[test]
    fn muting_the_published_tab_clears_now_playing() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, first);
        update(&mut policy, &controller);

        controller.toggle_audio_muted(first);

        assert_eq!(
            update(&mut policy, &controller),
            Some(NowPlayingChange::Clear)
        );
    }

    fn set_metadata(controller: &mut BrowserController, tab_id: TabId, metadata: MediaMetadata) {
        event(
            controller,
            tab_id,
            BrowserEvent::MediaMetadataChanged { metadata },
        );
    }

    #[test]
    fn page_text_is_cleaned_capped_and_names_the_host() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        set_metadata(
            &mut controller,
            first,
            MediaMetadata {
                title: "Evil\u{202E}gnos\u{0}\u{1b}[31m\u{85}title\n".to_owned(),
                artist: String::new(),
                album: "y".repeat(400),
            },
        );
        start_audible(&mut controller, first);

        let info = published(update(&mut policy, &controller));
        assert_eq!(info.title, "Evilgnos[31mtitle");
        assert_eq!(
            info.artist, "a.test",
            "the host stands in for a missing artist"
        );
        assert_eq!(info.album.chars().count(), MAX_TEXT_CHARACTERS);
        assert!(info.album.ends_with('…'));

        set_metadata(
            &mut controller,
            first,
            MediaMetadata {
                title: "Song".to_owned(),
                artist: "Band".to_owned(),
                album: String::new(),
            },
        );
        let info = published(update(&mut policy, &controller));
        assert_eq!(info.artist, "Band");
        assert_eq!(
            info.album, "a.test",
            "the host stands in for a missing album"
        );

        set_metadata(
            &mut controller,
            first,
            MediaMetadata {
                title: "Song".to_owned(),
                artist: "Band".to_owned(),
                album: "Record".to_owned(),
            },
        );
        assert_eq!(
            published(update(&mut policy, &controller)).album,
            "Record — a.test"
        );
    }

    #[test]
    fn titles_that_look_like_addresses_give_way_to_the_page_title() {
        let (mut controller, first, _) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        // What Servo sends for untitled media in a frame: the frame's address.
        set_metadata(
            &mut controller,
            first,
            MediaMetadata {
                title: "https://player.example/embed/123?autoplay=1".to_owned(),
                ..MediaMetadata::default()
            },
        );
        start_audible(&mut controller, first);

        assert_eq!(
            published(update(&mut policy, &controller)).title,
            "First page"
        );
    }

    #[test]
    fn a_quick_pause_and_play_does_not_take_now_playing_back() {
        // Servo's per-document state is the last element's, so pages flap Playing/Paused.
        let (mut controller, first, second) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        let start = Instant::now();
        let at = |millis: u64| start + Duration::from_millis(millis);
        start_audible(&mut controller, first);
        policy.update(&controller, None, at(0));
        start_audible(&mut controller, second);
        policy.update(&controller, None, at(5000));
        assert_eq!(policy.owner(), Some(second));

        playback(&mut controller, first, MediaPlaybackState::Paused);
        policy.update(&controller, None, at(5100));
        playback(&mut controller, first, MediaPlaybackState::Playing);
        policy.update(&controller, None, at(5300));
        assert_eq!(policy.owner(), Some(second), "a flap is not a new start");

        playback(&mut controller, first, MediaPlaybackState::Paused);
        policy.update(&controller, None, at(6000));
        playback(&mut controller, first, MediaPlaybackState::Playing);
        policy.update(&controller, None, at(8000));
        assert_eq!(
            policy.owner(),
            Some(first),
            "playing again after the hold is a new start"
        );
    }

    #[test]
    fn next_and_previous_need_servos_media_session_on_the_published_tab() {
        let (mut controller, first, second) = two_tabs();
        let mut policy = NowPlayingPolicy::default();
        start_audible(&mut controller, first);
        let now = Instant::now();

        let info = published(policy.update(&controller, Some(second), now));
        assert!(!info.can_skip);
        assert_eq!(policy.action_for(RemoteCommand::NextTrack), None);
        assert_eq!(policy.action_for(RemoteCommand::PreviousTrack), None);
        assert_eq!(
            policy.action_for(RemoteCommand::TogglePlayPause),
            Some((first, MediaSessionAction::Pause))
        );

        let info = published(policy.update(&controller, Some(first), now));
        assert!(info.can_skip);
        assert_eq!(
            policy.action_for(RemoteCommand::NextTrack),
            Some((first, MediaSessionAction::NextTrack))
        );
    }
}
