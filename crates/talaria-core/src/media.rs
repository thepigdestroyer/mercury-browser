/// What the engine last said about a page's media: from Servo's media session, which every
/// `<audio>` and `<video>` element reports to, whether or not the page uses the Media Session API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaPlaybackState {
    #[default]
    None,
    Playing,
    Paused,
}

/// The page's media metadata. Servo reports no artwork.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// Seconds as the engine reported them. Compared bit for bit, so an infinite or NaN duration
/// still equals itself.
#[derive(Debug, Clone, Copy)]
pub struct MediaPosition {
    duration: f64,
    playback_rate: f64,
    position: f64,
}

impl MediaPosition {
    pub const fn new(duration: f64, playback_rate: f64, position: f64) -> Self {
        Self {
            duration,
            playback_rate,
            position,
        }
    }

    pub const fn duration(&self) -> f64 {
        self.duration
    }

    pub const fn playback_rate(&self) -> f64 {
        self.playback_rate
    }

    pub const fn position(&self) -> f64 {
        self.position
    }

    fn bits(&self) -> [u64; 3] {
        [
            self.duration.to_bits(),
            self.playback_rate.to_bits(),
            self.position.to_bits(),
        ]
    }
}

impl PartialEq for MediaPosition {
    fn eq(&self, other: &Self) -> bool {
        self.bits() == other.bits()
    }
}

impl Eq for MediaPosition {}

/// What a look into the page's top-level document found: media elements that are playing,
/// how many of those are neither muted nor at volume 0, and whether the fullscreen element
/// is (or holds a playing) video. Media inside frames and elements outside the document are
/// not seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MediaProbe {
    pub playing: u32,
    pub audible: u32,
    pub fullscreen_video: bool,
}

/// A Media Session action, as media keys and the system's Now Playing controls send them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSessionAction {
    Play,
    Pause,
    PreviousTrack,
    NextTrack,
    SeekBackward,
    SeekForward,
    Stop,
}

/// The tab strip's sound indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabAudio {
    Silent,
    Playing,
    /// The user muted the tab; shown whether or not it plays, so it can be unmuted.
    Muted,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaState {
    playback: MediaPlaybackState,
    metadata: Option<MediaMetadata>,
    position: Option<MediaPosition>,
    probe: Option<MediaProbe>,
}

impl MediaState {
    pub const fn playback(&self) -> MediaPlaybackState {
        self.playback
    }

    pub fn metadata(&self) -> Option<&MediaMetadata> {
        self.metadata.as_ref()
    }

    pub const fn position(&self) -> Option<MediaPosition> {
        self.position
    }

    pub const fn probe(&self) -> Option<MediaProbe> {
        self.probe
    }

    pub const fn is_playing(&self) -> bool {
        matches!(self.playback, MediaPlaybackState::Playing)
    }

    /// Playing, and not silenced as far as the top-level document shows. When the probe sees
    /// nothing playing, the media is somewhere it cannot look (a frame, a detached element),
    /// so it counts as audible; so does media not probed yet.
    pub fn is_audible(&self) -> bool {
        self.is_playing()
            && self
                .probe
                .is_none_or(|probe| probe.playing == 0 || probe.audible > 0)
    }

    pub fn plays_fullscreen_video(&self) -> bool {
        self.is_playing() && self.probe.is_some_and(|probe| probe.fullscreen_video)
    }

    pub(crate) fn set_playback(&mut self, playback: MediaPlaybackState) {
        if playback == MediaPlaybackState::None {
            *self = Self::default();
        } else {
            self.playback = playback;
        }
    }

    pub(crate) fn set_metadata(&mut self, metadata: MediaMetadata) {
        self.metadata = Some(metadata);
    }

    pub(crate) fn set_position(&mut self, position: MediaPosition) {
        self.position = Some(position);
    }

    pub(crate) fn set_probe(&mut self, probe: MediaProbe) {
        self.probe = Some(probe);
    }
}
