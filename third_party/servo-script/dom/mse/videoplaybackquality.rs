// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

// Mercury: (MED-1b) `VideoPlaybackQuality` and `HTMLVideoElement.getVideoPlaybackQuality()` (decision D5
// of lane med-1b): total and dropped frames of the video sink of the element's player, which dash.js and
// Shaka read for their bitrate decisions. Written by Mercury; the interface and the method are behind
// `dom_media_source_enabled` (the interface is part of the Media Source Extensions specification).
// <https://w3c.github.io/media-playback-quality/>

use dom_struct::dom_struct;
use js::context::JSContext;
use script_bindings::reflector::{Reflector, reflect_dom_object};

use crate::dom::bindings::codegen::Bindings::PerformanceBinding::PerformanceMethods;
use crate::dom::bindings::codegen::Bindings::VideoPlaybackQualityBinding::VideoPlaybackQualityMethods;
use crate::dom::bindings::num::Finite;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::DomRoot;
use crate::dom::html::htmlmediaelement::HTMLMediaElement;
use crate::dom::window::Window;

/// <https://w3c.github.io/media-playback-quality/#videoplaybackquality>
#[dom_struct]
pub(crate) struct VideoPlaybackQuality {
    reflector_: Reflector,
    creation_time: Finite<f64>,
    total_video_frames: u32,
    dropped_video_frames: u32,
    corrupted_video_frames: u32,
}

impl VideoPlaybackQuality {
    fn new_inherited(
        creation_time: Finite<f64>,
        total_video_frames: u32,
        dropped_video_frames: u32,
    ) -> VideoPlaybackQuality {
        VideoPlaybackQuality {
            reflector_: Reflector::new(),
            creation_time,
            total_video_frames,
            dropped_video_frames,
            // Nothing in the pipeline tells a corrupted frame from a good one.
            corrupted_video_frames: 0,
        }
    }

    /// The counters of `element`'s player now: `getVideoPlaybackQuality()` of a video element.
    /// <https://w3c.github.io/media-playback-quality/#dom-htmlvideoelement-getvideoplaybackquality>
    pub(crate) fn for_element(
        cx: &mut JSContext,
        window: &Window,
        element: &HTMLMediaElement,
    ) -> DomRoot<VideoPlaybackQuality> {
        let (total, dropped) = element.player_video_playback_quality().unwrap_or((0, 0));
        let creation_time = element.global().performance(cx).Now();
        let total = u32::try_from(total).unwrap_or(u32::MAX);
        // Dropped frames are a part of the total.
        let dropped = u32::try_from(dropped).unwrap_or(u32::MAX).min(total);
        reflect_dom_object(
            cx,
            Box::new(VideoPlaybackQuality::new_inherited(
                creation_time,
                total,
                dropped,
            )),
            window,
        )
    }
}

impl VideoPlaybackQualityMethods<crate::DomTypeHolder> for VideoPlaybackQuality {
    /// <https://w3c.github.io/media-playback-quality/#dom-videoplaybackquality-creationtime>
    fn CreationTime(&self) -> Finite<f64> {
        self.creation_time
    }

    /// <https://w3c.github.io/media-playback-quality/#dom-videoplaybackquality-totalvideoframes>
    fn TotalVideoFrames(&self) -> u32 {
        self.total_video_frames
    }

    /// <https://w3c.github.io/media-playback-quality/#dom-videoplaybackquality-droppedvideoframes>
    fn DroppedVideoFrames(&self) -> u32 {
        self.dropped_video_frames
    }

    /// <https://w3c.github.io/media-playback-quality/#dom-videoplaybackquality-corruptedvideoframes>
    fn CorruptedVideoFrames(&self) -> u32 {
        self.corrupted_video_frames
    }
}
