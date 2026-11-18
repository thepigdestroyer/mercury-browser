use talaria_core::{
    BrowserCommand, BrowserController, BrowserEvent, BrowserState, LoadState, MediaMetadata,
    MediaPlaybackState, MediaPosition, MediaProbe, MediaState, TabAudio, TabCommand, TabId,
};

fn playing() -> BrowserEvent {
    BrowserEvent::MediaPlaybackChanged {
        state: MediaPlaybackState::Playing,
    }
}

fn paused() -> BrowserEvent {
    BrowserEvent::MediaPlaybackChanged {
        state: MediaPlaybackState::Paused,
    }
}

fn metadata(title: &str) -> MediaMetadata {
    MediaMetadata {
        title: title.to_owned(),
        artist: "Artist".to_owned(),
        album: "Album".to_owned(),
    }
}

fn probe(playing: u32, audible: u32, fullscreen_video: bool) -> BrowserEvent {
    BrowserEvent::MediaProbed {
        probe: MediaProbe {
            playing,
            audible,
            unseen: false,
            fullscreen_video,
        },
    }
}

fn commit(url: &str) -> BrowserEvent {
    BrowserEvent::AddressChanged {
        url: url.to_owned(),
    }
}

fn loaded() -> BrowserEvent {
    BrowserEvent::LoadStateChanged {
        state: LoadState::Loaded,
    }
}

fn loading() -> BrowserEvent {
    BrowserEvent::LoadStateChanged {
        state: LoadState::Loading,
    }
}

fn ready_controller() -> (BrowserController, TabId) {
    let mut controller = BrowserController::default();
    let tab_id = controller.active_tab_id();
    controller.handle_event(tab_id, BrowserEvent::AddressChanged { url: String::new() });
    (controller, tab_id)
}

fn drain(controller: &mut BrowserController) -> Vec<TabCommand> {
    std::iter::from_fn(|| controller.next_command()).collect()
}

fn tab_state(controller: &BrowserController, tab_id: TabId) -> &BrowserState {
    controller
        .tabs()
        .find(|tab| tab.id() == tab_id)
        .expect("the tab exists")
        .state()
}

#[test]
fn a_new_tab_has_no_media_and_is_not_muted() {
    let state = BrowserState::default();

    assert_eq!(state.media(), &MediaState::default());
    assert_eq!(state.media().playback(), MediaPlaybackState::None);
    assert!(!state.is_audio_muted());
    assert_eq!(state.audio_indicator(), TabAudio::Silent);
}

#[test]
fn playback_changes_are_recorded() {
    let mut state = BrowserState::default();

    state.apply(playing());
    assert!(state.media().is_playing());
    assert_eq!(state.audio_indicator(), TabAudio::Playing);

    state.apply(paused());
    assert_eq!(state.media().playback(), MediaPlaybackState::Paused);
    assert!(!state.media().is_playing());
    assert_eq!(state.audio_indicator(), TabAudio::Silent);
}

#[test]
fn metadata_and_position_are_kept_as_reported() {
    let mut state = BrowserState::default();
    let position = MediaPosition::new(4.0, 1.0, 1.5);

    state.apply(BrowserEvent::MediaMetadataChanged {
        metadata: metadata("Song"),
    });
    state.apply(BrowserEvent::MediaPositionChanged { position });

    assert_eq!(state.media().metadata(), Some(&metadata("Song")));
    assert_eq!(state.media().position(), Some(position));
    assert_eq!(position.duration(), 4.0);
    assert_eq!(position.playback_rate(), 1.0);
    assert_eq!(position.position(), 1.5);
}

#[test]
fn positions_compare_bit_for_bit() {
    assert_eq!(
        MediaPosition::new(f64::INFINITY, 1.0, 0.0),
        MediaPosition::new(f64::INFINITY, 1.0, 0.0)
    );
    assert_eq!(
        MediaPosition::new(f64::NAN, 1.0, 0.0),
        MediaPosition::new(f64::NAN, 1.0, 0.0)
    );
    assert_ne!(
        MediaPosition::new(4.0, 1.0, 0.0),
        MediaPosition::new(4.0, 1.0, 0.1)
    );
}

#[test]
fn the_none_state_forgets_the_session() {
    let mut state = BrowserState::default();
    state.apply(BrowserEvent::MediaMetadataChanged {
        metadata: metadata("Song"),
    });
    state.apply(playing());
    state.apply(probe(1, 1, false));

    state.apply(BrowserEvent::MediaPlaybackChanged {
        state: MediaPlaybackState::None,
    });

    assert_eq!(state.media(), &MediaState::default());
}

fn playing_loaded_page() -> BrowserState {
    let mut state = BrowserState::default();
    state.apply(loading());
    state.apply(BrowserEvent::DocumentStarted);
    state.apply(commit("https://a.test/"));
    state.apply(loaded());
    state.apply(playing());
    state.apply(BrowserEvent::MediaMetadataChanged {
        metadata: metadata("Song"),
    });
    state
}

#[test]
fn a_new_document_starts_without_media() {
    // Servo's `Started` comes from the new top-level document as it is initialised; a reload
    // sends no address change at all.
    let mut state = playing_loaded_page();

    state.apply(loading());
    state.apply(BrowserEvent::DocumentStarted);

    assert_eq!(state.media(), &MediaState::default());
}

#[test]
fn load_states_and_address_changes_alone_keep_the_media() {
    // `HeadParsed` also arrives when a page inserts a `<body>`; address changes without a new
    // document are `pushState` or a fragment. Neither may let a page hide its sound.
    let mut state = playing_loaded_page();

    state.apply(loading());
    state.apply(loaded());
    state.apply(commit("https://a.test/next-track"));
    state.apply(commit("https://a.test/next-track#t=10"));

    assert!(state.media().is_playing());
    assert_eq!(state.media().metadata(), Some(&metadata("Song")));
}

#[test]
fn media_the_new_document_starts_is_kept() {
    let mut state = BrowserState::default();
    state.apply(loading());
    state.apply(BrowserEvent::DocumentStarted);
    state.apply(playing());

    state.apply(loading());
    state.apply(commit("https://a.test/"));
    state.apply(loaded());

    assert!(state.media().is_playing());
}

#[test]
fn a_muted_tab_is_muted_again_when_a_document_starts() {
    let (mut controller, tab_id) = ready_controller();
    controller.toggle_audio_muted(tab_id);
    drain(&mut controller);

    controller.handle_event(tab_id, BrowserEvent::DocumentStarted);

    assert_eq!(
        drain(&mut controller),
        vec![TabCommand::new(
            tab_id,
            BrowserCommand::SetAudioMuted { muted: true }
        )]
    );
}

#[test]
fn a_tab_whose_web_view_is_replaced_forgets_its_media() {
    let mut controller = BrowserController::default();
    let tab_id = controller.active_tab_id();
    controller.handle_event(tab_id, playing());

    // Not engine-ready: the navigation replaces the tab's web view.
    controller
        .queue_navigation("b.test")
        .expect("the address resolves");
    assert_eq!(
        controller.take_initial_url(tab_id),
        Some("https://b.test/".to_owned())
    );

    assert_eq!(
        tab_state(&controller, tab_id).media(),
        &MediaState::default()
    );
}

#[test]
fn a_failed_or_crashed_page_has_no_media() {
    let mut state = BrowserState::default();
    state.apply(playing());

    state.apply(BrowserEvent::LoadStateChanged {
        state: LoadState::Failed {
            message: "crashed".to_owned(),
        },
    });

    assert_eq!(state.media(), &MediaState::default());
}

#[test]
fn a_probe_that_finds_only_silenced_media_hides_the_indicator() {
    let mut state = BrowserState::default();
    state.apply(playing());

    state.apply(probe(2, 0, false));
    assert!(!state.media().is_audible());
    assert_eq!(state.audio_indicator(), TabAudio::Silent);

    state.apply(probe(2, 1, false));
    assert!(state.media().is_audible());
    assert_eq!(state.audio_indicator(), TabAudio::Playing);
}

#[test]
fn media_in_places_the_probe_cannot_see_counts_as_audible() {
    // A muted background video next to a frame (or a closed shadow root) that may play sound.
    let mut state = BrowserState::default();
    state.apply(playing());

    state.apply(BrowserEvent::MediaProbed {
        probe: MediaProbe {
            playing: 1,
            audible: 0,
            unseen: true,
            fullscreen_video: false,
        },
    });

    assert!(state.media().is_audible());
    assert_eq!(state.audio_indicator(), TabAudio::Playing);
}

#[test]
fn media_the_probe_cannot_see_counts_as_audible() {
    // Media in a frame, or an element that is not in the document, plays while the top-level
    // document has nothing playing.
    let mut state = BrowserState::default();
    state.apply(playing());

    state.apply(probe(0, 0, false));

    assert!(state.media().is_audible());
}

#[test]
fn fullscreen_video_needs_playback_and_a_probe_that_saw_it() {
    let mut state = BrowserState::default();
    state.apply(probe(1, 1, true));
    assert!(!state.media().plays_fullscreen_video());

    state.apply(playing());
    assert!(state.media().plays_fullscreen_video());

    state.apply(probe(1, 1, false));
    assert!(!state.media().plays_fullscreen_video());

    state.apply(probe(1, 1, true));
    state.apply(paused());
    assert!(!state.media().plays_fullscreen_video());
}

#[test]
fn toggling_mute_records_the_choice_and_tells_the_page() {
    let (mut controller, tab_id) = ready_controller();

    assert!(controller.toggle_audio_muted(tab_id));
    assert!(tab_state(&controller, tab_id).is_audio_muted());
    assert_eq!(
        tab_state(&controller, tab_id).audio_indicator(),
        TabAudio::Muted
    );
    assert_eq!(
        drain(&mut controller),
        vec![TabCommand::new(
            tab_id,
            BrowserCommand::SetAudioMuted { muted: true }
        )]
    );

    assert!(controller.toggle_audio_muted(tab_id));
    assert!(!tab_state(&controller, tab_id).is_audio_muted());
    assert_eq!(
        drain(&mut controller),
        vec![TabCommand::new(
            tab_id,
            BrowserCommand::SetAudioMuted { muted: false }
        )]
    );
}

#[test]
fn toggling_mute_of_a_missing_tab_does_nothing() {
    let (mut controller, _) = ready_controller();

    assert!(!controller.toggle_audio_muted(TabId::new(99)));
    assert!(drain(&mut controller).is_empty());
}

#[test]
fn a_muted_tab_shows_the_muted_indicator_while_playing_or_not() {
    let (mut controller, tab_id) = ready_controller();
    controller.toggle_audio_muted(tab_id);

    controller.handle_event(tab_id, playing());
    assert_eq!(
        tab_state(&controller, tab_id).audio_indicator(),
        TabAudio::Muted
    );
    controller.handle_event(tab_id, paused());
    assert_eq!(
        tab_state(&controller, tab_id).audio_indicator(),
        TabAudio::Muted
    );
}

#[test]
fn mute_survives_navigation_and_is_applied_again_to_each_new_document() {
    let (mut controller, tab_id) = ready_controller();
    controller.toggle_audio_muted(tab_id);
    drain(&mut controller);

    controller.handle_event(tab_id, loading());
    controller.handle_event(
        tab_id,
        BrowserEvent::AddressChanged {
            url: "https://next.test/".to_owned(),
        },
    );
    controller.handle_event(
        tab_id,
        BrowserEvent::LoadStateChanged {
            state: LoadState::Loaded,
        },
    );
    controller.handle_event(tab_id, playing());

    assert!(tab_state(&controller, tab_id).is_audio_muted());
    let mute = TabCommand::new(tab_id, BrowserCommand::SetAudioMuted { muted: true });
    assert_eq!(
        drain(&mut controller),
        vec![mute.clone(), mute.clone(), mute]
    );
}

#[test]
fn an_unmuted_tab_is_left_alone_by_page_events() {
    let (mut controller, tab_id) = ready_controller();

    controller.handle_event(
        tab_id,
        BrowserEvent::LoadStateChanged {
            state: LoadState::Loaded,
        },
    );
    controller.handle_event(tab_id, playing());

    assert!(drain(&mut controller).is_empty());
}

#[test]
fn pause_and_position_events_do_not_reapply_mute() {
    let (mut controller, tab_id) = ready_controller();
    controller.toggle_audio_muted(tab_id);
    drain(&mut controller);

    controller.handle_event(tab_id, paused());
    controller.handle_event(
        tab_id,
        BrowserEvent::MediaPositionChanged {
            position: MediaPosition::new(4.0, 1.0, 1.0),
        },
    );

    assert!(drain(&mut controller).is_empty());
}

#[test]
fn mute_is_per_tab() {
    let (mut controller, first) = ready_controller();
    let second = controller.open_tab();
    controller.handle_event(second, BrowserEvent::AddressChanged { url: String::new() });

    controller.toggle_audio_muted(first);

    assert!(tab_state(&controller, first).is_audio_muted());
    assert!(!tab_state(&controller, second).is_audio_muted());
}

#[test]
fn a_muted_tab_whose_page_unmuted_its_media_is_muted_again() {
    let (mut controller, tab_id) = ready_controller();
    controller.toggle_audio_muted(tab_id);
    drain(&mut controller);

    controller.handle_event(tab_id, probe(1, 0, false));
    assert!(drain(&mut controller).is_empty());

    controller.handle_event(tab_id, probe(1, 1, false));
    assert_eq!(
        drain(&mut controller),
        vec![TabCommand::new(
            tab_id,
            BrowserCommand::SetAudioMuted { muted: true }
        )]
    );
}

#[test]
fn an_unmuted_tab_is_not_muted_by_a_probe() {
    let (mut controller, tab_id) = ready_controller();

    controller.handle_event(tab_id, probe(1, 1, false));

    assert!(drain(&mut controller).is_empty());
}

#[test]
fn only_positive_evidence_of_sound_counts_for_the_media_keys() {
    let mut state = BrowserState::default();
    state.apply(playing());
    assert!(!state.media().has_audible_evidence(), "not probed yet");

    // A muted autoplay video next to a frame: the speaker shows, the media keys stay away.
    state.apply(BrowserEvent::MediaProbed {
        probe: MediaProbe {
            playing: 1,
            audible: 0,
            unseen: true,
            fullscreen_video: false,
        },
    });
    assert!(state.media().is_audible());
    assert!(!state.media().has_audible_evidence());

    state.apply(probe(1, 1, false));
    assert!(state.media().has_audible_evidence());

    // Servo says it plays and the page shows nothing playing: the sound is out of sight.
    state.apply(probe(0, 0, false));
    assert!(state.media().has_audible_evidence());

    state.apply(paused());
    assert!(!state.media().has_audible_evidence());
}

#[test]
fn a_late_pause_from_the_previous_document_does_not_revive_its_media() {
    let mut state = playing_loaded_page();

    state.apply(BrowserEvent::DocumentStarted);
    // The old document's elements report their pause as they are torn down; Servo does not say
    // which document an event came from.
    state.apply(paused());

    assert_eq!(state.media(), &MediaState::default());
}

#[test]
fn a_document_reports_pauses_once_its_media_has_played() {
    let mut state = BrowserState::default();
    state.apply(BrowserEvent::DocumentStarted);
    // GStreamer reports Paused while it prepares an element, before it plays.
    state.apply(paused());
    assert_eq!(state.media().playback(), MediaPlaybackState::None);

    state.apply(playing());
    state.apply(paused());
    assert_eq!(state.media().playback(), MediaPlaybackState::Paused);
}
