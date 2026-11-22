// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

// Mercury: (MED-1b) the contents of `SourceBuffer.audioTracks` and `videoTracks` and of the media
// element's lists (decision D6 of lane med-1b). One `AudioTrack` or `VideoTrack` per track of an
// initialization segment (id, kind, label and language from `TrackInfo`; enabled or selected when it is
// the default), the same object in the source buffer's list and in the element's list, an `addtrack`
// event at each list, `removetrack` when the source buffer is removed. The lists are informational: the
// pipeline plays one track per kind (decision D3 of round 1), so toggling `enabled` or `selected` changes
// the flag and fires `change` and nothing else (the player ignores the request for a MediaSource).
//
// The track lists of this Servo have `add` and `clear` and no `remove`, and the shared files are not
// ours to change, so a removal or a replacement rebuilds the list from the tracks that stay.

use servo_media::player::mse::{InitInfo, TrackInfo, TrackKind};
use stylo_atoms::Atom;

use crate::dom::audio::audiotrack::AudioTrack;
use crate::dom::audio::audiotracklist::AudioTrackList;
use crate::dom::bindings::codegen::Bindings::AudioTrackBinding::AudioTrackMethods;
use crate::dom::bindings::codegen::Bindings::VideoTrackBinding::VideoTrackMethods;
use crate::dom::bindings::codegen::UnionTypes::VideoTrackOrAudioTrackOrTextTrack;
use crate::dom::bindings::inheritance::Castable;
use crate::dom::bindings::root::DomRoot;
use crate::dom::bindings::str::DOMString;
use crate::dom::event::Event;
use crate::dom::eventtarget::EventTarget;
use crate::dom::trackevent::TrackEvent;
use crate::dom::videotrack::VideoTrack;
use crate::dom::videotracklist::VideoTrackList;
use crate::dom::window::Window;
use js::context::JSContext;

/// The two lists a track of an initialization segment goes into, for one kind.
pub(crate) struct TrackLists<'a> {
    pub(crate) own_audio: &'a AudioTrackList,
    pub(crate) own_video: &'a VideoTrackList,
    pub(crate) element_audio: Option<&'a AudioTrackList>,
    pub(crate) element_video: Option<&'a VideoTrackList>,
}

/// The kind attribute of a track: "main" for the default track of its kind, empty otherwise (the
/// initialization segment carries no role).
fn kind_of(info: &TrackInfo) -> DOMString {
    if info.default {
        DOMString::from_static("main")
    } else {
        DOMString::new()
    }
}

/// The "initialization segment received" algorithm's track steps, for the tracks of `init`: a track
/// the buffer already has (same id) stays, or is replaced without an event when its kind, label or
/// language changed (a quality switch keeps the count); a new one is created, added to both lists and
/// announced with `addtrack`.
pub(crate) fn apply_init(
    cx: &mut JSContext,
    window: &Window,
    init: &InitInfo,
    lists: &TrackLists<'_>,
) {
    for info in &init.tracks {
        match info.kind {
            TrackKind::Audio => apply_audio(cx, window, info, lists),
            TrackKind::Video => apply_video(cx, window, info, lists),
            TrackKind::Text => {}
        }
    }
}

/// `removeSourceBuffer` steps 3 to 8: the buffer's tracks leave the element's lists and its own, each
/// removal announced with `removetrack`.
pub(crate) fn remove_all(cx: &mut JSContext, window: &Window, lists: &TrackLists<'_>) {
    for track in audio_tracks(lists.own_audio) {
        if let Some(element) = lists.element_audio {
            remove_audio(cx, window, element, &track);
        }
        remove_audio(cx, window, lists.own_audio, &track);
    }
    for track in video_tracks(lists.own_video) {
        if let Some(element) = lists.element_video {
            remove_video(cx, window, element, &track);
        }
        remove_video(cx, window, lists.own_video, &track);
    }
}

fn audio_tracks(list: &AudioTrackList) -> Vec<DomRoot<AudioTrack>> {
    (0..list.len())
        .filter_map(|index| list.item(index))
        .collect()
}

fn video_tracks(list: &VideoTrackList) -> Vec<DomRoot<VideoTrack>> {
    (0..list.len())
        .filter_map(|index| list.item(index))
        .collect()
}

fn find_audio(list: &AudioTrackList, id: &DOMString) -> Option<DomRoot<AudioTrack>> {
    audio_tracks(list)
        .into_iter()
        .find(|track| track.id() == *id)
}

fn find_video(list: &VideoTrackList, id: &DOMString) -> Option<DomRoot<VideoTrack>> {
    video_tracks(list)
        .into_iter()
        .find(|track| track.id() == *id)
}

/// Rebuilds `list` with `tracks`; `replace` swaps one of them for another at the same place.
fn rebuild_audio(
    list: &AudioTrackList,
    tracks: Vec<DomRoot<AudioTrack>>,
    replace: Option<(&AudioTrack, &DomRoot<AudioTrack>)>,
) {
    list.clear();
    for track in tracks {
        match replace {
            Some((old, new)) if &*track == old => list.add(new),
            _ => list.add(&track),
        }
    }
}

fn rebuild_video(
    list: &VideoTrackList,
    tracks: Vec<DomRoot<VideoTrack>>,
    replace: Option<(&VideoTrack, &DomRoot<VideoTrack>)>,
) {
    // `VideoTrackList::add` deselects a selected track it is given (the list keeps one selected): add
    // unselected, then restore.
    list.clear();
    for track in tracks {
        let track = match replace {
            Some((old, new)) if &*track == old => new.clone(),
            _ => track,
        };
        let selected = track.selected();
        track.set_selected(false);
        list.add(&track);
        track.set_selected(selected);
    }
}

fn announce_audio(
    cx: &mut JSContext,
    window: &Window,
    name: Atom,
    list: &AudioTrackList,
    track: &DomRoot<AudioTrack>,
) {
    let event = TrackEvent::new(
        cx,
        window,
        name,
        false,
        false,
        &Some(VideoTrackOrAudioTrackOrTextTrack::AudioTrack(track.clone())),
    );
    event
        .upcast::<Event>()
        .fire(cx, list.upcast::<EventTarget>());
}

fn announce_video(
    cx: &mut JSContext,
    window: &Window,
    name: Atom,
    list: &VideoTrackList,
    track: &DomRoot<VideoTrack>,
) {
    let event = TrackEvent::new(
        cx,
        window,
        name,
        false,
        false,
        &Some(VideoTrackOrAudioTrackOrTextTrack::VideoTrack(track.clone())),
    );
    event
        .upcast::<Event>()
        .fire(cx, list.upcast::<EventTarget>());
}

/// A track belongs to one list in this Servo (`track_list`, which `enabled = ...` goes through); the
/// element's list is the one that reports `change` for it, so it gets the track last.
fn repoint_audio(lists: &TrackLists<'_>) {
    if let Some(element) = lists.element_audio {
        for track in audio_tracks(element) {
            track.add_track_list(element);
        }
    }
}

fn repoint_video(lists: &TrackLists<'_>) {
    if let Some(element) = lists.element_video {
        for track in video_tracks(element) {
            track.add_track_list(element);
        }
    }
}

fn remove_audio(
    cx: &mut JSContext,
    window: &Window,
    list: &AudioTrackList,
    track: &DomRoot<AudioTrack>,
) {
    let rest: Vec<_> = audio_tracks(list)
        .into_iter()
        .filter(|other| **other != **track)
        .collect();
    if rest.len() == list.len() {
        return;
    }
    rebuild_audio(list, rest, None);
    announce_audio(cx, window, Atom::from("removetrack"), list, track);
}

fn remove_video(
    cx: &mut JSContext,
    window: &Window,
    list: &VideoTrackList,
    track: &DomRoot<VideoTrack>,
) {
    let rest: Vec<_> = video_tracks(list)
        .into_iter()
        .filter(|other| **other != **track)
        .collect();
    if rest.len() == list.len() {
        return;
    }
    rebuild_video(list, rest, None);
    announce_video(cx, window, Atom::from("removetrack"), list, track);
}

fn apply_audio(cx: &mut JSContext, window: &Window, info: &TrackInfo, lists: &TrackLists<'_>) {
    let id = DOMString::from(info.id.to_string());
    let kind = kind_of(info);
    let label = DOMString::from(info.label.clone());
    let language = DOMString::from(info.language.clone());

    if let Some(existing) = find_audio(lists.own_audio, &id) {
        if existing.kind() == kind && existing.Label() == label && existing.Language() == language {
            return;
        }
        let track = AudioTrack::new(cx, window, id, kind, label, language, None);
        track.set_enabled(existing.enabled());
        rebuild_audio(
            lists.own_audio,
            audio_tracks(lists.own_audio),
            Some((&existing, &track)),
        );
        if let Some(element) = lists.element_audio {
            rebuild_audio(element, audio_tracks(element), Some((&existing, &track)));
        }
        repoint_audio(lists);
        return;
    }

    let track = AudioTrack::new(cx, window, id, kind, label, language, None);
    let enable = info.default || lists.own_audio.enabled_index().is_none();
    track.set_enabled(enable);
    lists.own_audio.add(&track);
    announce_audio(cx, window, Atom::from("addtrack"), lists.own_audio, &track);
    if let Some(element) = lists.element_audio {
        element.add(&track);
        announce_audio(cx, window, Atom::from("addtrack"), element, &track);
    }
    repoint_audio(lists);
}

fn apply_video(cx: &mut JSContext, window: &Window, info: &TrackInfo, lists: &TrackLists<'_>) {
    let id = DOMString::from(info.id.to_string());
    let kind = kind_of(info);
    let label = DOMString::from(info.label.clone());
    let language = DOMString::from(info.language.clone());

    if let Some(existing) = find_video(lists.own_video, &id) {
        if existing.kind() == kind && existing.Label() == label && existing.Language() == language {
            return;
        }
        let track = VideoTrack::new(cx, window, id, kind, label, language, None);
        track.set_selected(existing.selected());
        rebuild_video(
            lists.own_video,
            video_tracks(lists.own_video),
            Some((&existing, &track)),
        );
        if let Some(element) = lists.element_video {
            rebuild_video(element, video_tracks(element), Some((&existing, &track)));
        }
        repoint_video(lists);
        return;
    }

    let track = VideoTrack::new(cx, window, id, kind, label, language, None);
    // The element's list keeps one selected video track: the first one that gets there.
    let select = (info.default || lists.own_video.selected_index().is_none())
        && lists
            .element_video
            .is_none_or(|element| element.selected_index().is_none());
    lists.own_video.add(&track);
    if let Some(element) = lists.element_video {
        element.add(&track);
    }
    track.set_selected(select);
    announce_video(cx, window, Atom::from("addtrack"), lists.own_video, &track);
    if let Some(element) = lists.element_video {
        announce_video(cx, window, Atom::from("addtrack"), element, &track);
    }
    repoint_video(lists);
}
