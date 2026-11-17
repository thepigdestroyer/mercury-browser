use crate::{ColorScheme, DEFAULT_ZOOM_PERCENT, ImeInput, KeyboardInput, PageIcon, PointerCursor};
// media session
use crate::{
    MediaMetadata, MediaPlaybackState, MediaPosition, MediaProbe, MediaSessionAction, MediaState,
    TabAudio,
};

const DEFAULT_PAGE_TITLE: &str = "New Tab";
const INITIAL_ADDRESS: &str = "";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    Idle,
    Loading,
    Loaded,
    Failed { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButtonAction {
    Down,
    Up,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BrowserCommand {
    Navigate {
        url: String,
    },
    GoBack,
    GoForward,
    Reload,
    PointerMoved {
        x: f32,
        y: f32,
    },
    PointerButton {
        action: PointerButtonAction,
        button: PointerButton,
        x: f32,
        y: f32,
    },
    Scrolled {
        delta_x: f32,
        delta_y: f32,
        x: f32,
        y: f32,
    },
    FocusChanged {
        focused: bool,
    },
    KeyboardInput(KeyboardInput),
    ImeInput(ImeInput),
    /// Page zoom as a percentage of the default size.
    SetPageZoom {
        percent: u16,
    },
    /// Multiplies the pinch zoom by `delta`, centred on the given web-view point.
    PinchZoom {
        delta: f32,
        x: f32,
        y: f32,
    },
    SetColorScheme(ColorScheme),
    ExitFullscreen,
    /// The pointer left the web view, or the tab stopped being shown.
    PointerLeft,
    // media session
    /// Mutes (or unmutes) the page's media elements from script; the page can undo it.
    SetAudioMuted {
        muted: bool,
    },
    /// `through_engine`: Servo's single media session is known to be this tab's, so the action
    /// goes through Servo (page handlers, next/previous); otherwise play and pause are done from
    /// script and other actions are dropped.
    MediaSessionAction {
        action: MediaSessionAction,
        through_engine: bool,
    },
    /// Looks into the page's document; the engine answers with `BrowserEvent::MediaProbed`.
    ProbeMedia,
}

impl BrowserCommand {
    /// Pointer input only makes sense for the document the user currently sees. It is
    /// discarded, rather than held, while the tab's browsing context does not exist yet.
    pub(crate) const fn is_discarded_before_engine_ready(&self) -> bool {
        matches!(
            self,
            Self::PointerMoved { .. }
                | Self::PointerButton { .. }
                | Self::Scrolled { .. }
                | Self::PinchZoom { .. }
                | Self::PointerLeft
        )
    }

    /// Commands that act on the tab's current page; they are meaningless once that page is
    /// replaced before it ever committed.
    pub(crate) const fn targets_the_current_page(&self) -> bool {
        matches!(
            self,
            Self::Navigate { .. } | Self::Reload | Self::GoBack | Self::GoForward
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserEvent {
    AddressChanged {
        url: String,
    },
    PageTitleChanged {
        title: Option<String>,
    },
    PageIconChanged {
        icon: Option<PageIcon>,
    },
    LoadStateChanged {
        state: LoadState,
    },
    HistoryChanged {
        can_go_back: bool,
        can_go_forward: bool,
    },
    CursorChanged {
        cursor: PointerCursor,
    },
    /// Text the page wants shown in the status area, such as a hovered link's URL.
    StatusTextChanged {
        text: Option<String>,
    },
    FullscreenChanged {
        fullscreen: bool,
    },
    // media session
    MediaPlaybackChanged {
        state: MediaPlaybackState,
    },
    MediaMetadataChanged {
        metadata: MediaMetadata,
    },
    MediaPositionChanged {
        position: MediaPosition,
    },
    MediaProbed {
        probe: MediaProbe,
    },
    /// A new top-level document was created in the tab: the previous one, and its media, are
    /// gone. Unlike `LoadStateChanged`, a page cannot send this for its own document.
    DocumentStarted,
}

impl BrowserEvent {
    /// Events the engine only sends once the tab's browsing context exists.
    pub(crate) const fn shows_engine_ready(&self) -> bool {
        matches!(
            self,
            Self::AddressChanged { .. } | Self::HistoryChanged { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserState {
    url: String,
    page_title: String,
    page_icon: Option<PageIcon>,
    load_state: LoadState,
    can_go_back: bool,
    can_go_forward: bool,
    zoom_percent: u16,
    cursor: PointerCursor,
    status_text: Option<String>,
    fullscreen: bool,
    // media session
    media: MediaState,
    /// The user's choice; kept across navigations in the tab.
    audio_muted: bool,
}

impl Default for BrowserState {
    fn default() -> Self {
        Self {
            url: INITIAL_ADDRESS.to_owned(),
            page_title: DEFAULT_PAGE_TITLE.to_owned(),
            page_icon: None,
            load_state: LoadState::Idle,
            can_go_back: false,
            can_go_forward: false,
            zoom_percent: DEFAULT_ZOOM_PERCENT,
            cursor: PointerCursor::Default,
            status_text: None,
            fullscreen: false,
            // media session
            media: MediaState::default(),
            audio_muted: false,
        }
    }
}

impl BrowserState {
    pub fn page_title(&self) -> &str {
        &self.page_title
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn page_icon(&self) -> Option<&PageIcon> {
        self.page_icon.as_ref()
    }
    pub fn load_state(&self) -> &LoadState {
        &self.load_state
    }
    pub fn can_go_back(&self) -> bool {
        self.can_go_back
    }
    pub fn can_go_forward(&self) -> bool {
        self.can_go_forward
    }
    pub fn zoom_percent(&self) -> u16 {
        self.zoom_percent
    }

    pub fn cursor(&self) -> PointerCursor {
        self.cursor
    }
    pub fn status_text(&self) -> Option<&str> {
        self.status_text.as_deref()
    }
    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub(crate) fn set_zoom_percent(&mut self, percent: u16) {
        self.zoom_percent = percent;
    }

    // media session
    pub fn media(&self) -> &MediaState {
        &self.media
    }
    pub fn is_audio_muted(&self) -> bool {
        self.audio_muted
    }
    pub fn audio_indicator(&self) -> TabAudio {
        if self.audio_muted {
            TabAudio::Muted
        } else if self.media.is_audible() {
            TabAudio::Playing
        } else {
            TabAudio::Silent
        }
    }
    pub(crate) fn set_audio_muted(&mut self, muted: bool) {
        self.audio_muted = muted;
    }
    /// The tab's web view is about to be replaced by a new one.
    pub(crate) fn forget_media(&mut self) {
        self.media = MediaState::default();
    }
    /// Media belongs to a document: it is forgotten when a new document starts, and when the
    /// page fails or crashes. Load states and address changes alone keep it (a page can send
    /// `HeadParsed` by inserting a `<body>`; `pushState` changes the address).
    fn forget_media_of_a_replaced_document(&mut self, event: &BrowserEvent) {
        if matches!(
            event,
            BrowserEvent::DocumentStarted
                | BrowserEvent::LoadStateChanged {
                    state: LoadState::Failed { .. }
                }
        ) {
            self.forget_media();
        }
    }

    pub fn apply(&mut self, event: BrowserEvent) {
        // media session
        self.forget_media_of_a_replaced_document(&event);
        match event {
            BrowserEvent::AddressChanged { url } => {
                // Cursor and status text are deliberately not reset here: the engine only
                // reports changes against its own cached values, so the engine's notifications
                // must stay the only source of both.
                if self.url != url {
                    self.url = url;
                    self.page_icon = None;
                }
            }
            BrowserEvent::PageTitleChanged { title } => {
                self.page_title = title
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_PAGE_TITLE.to_owned());
            }
            BrowserEvent::PageIconChanged { icon } => {
                self.page_icon = icon;
            }
            BrowserEvent::LoadStateChanged { state } => {
                if matches!(state, LoadState::Failed { .. }) {
                    // A failed or crashed page can no longer leave fullscreen, so nothing else
                    // would clear these. The cursor is left to the engine: after a crash the
                    // web view still caches its old cursor and reports it again.
                    self.fullscreen = false;
                    self.status_text = None;
                }
                self.load_state = state;
            }
            BrowserEvent::HistoryChanged {
                can_go_back,
                can_go_forward,
            } => {
                self.can_go_back = can_go_back;
                self.can_go_forward = can_go_forward;
            }
            BrowserEvent::CursorChanged { cursor } => {
                self.cursor = cursor;
            }
            BrowserEvent::StatusTextChanged { text } => {
                self.status_text = text;
            }
            BrowserEvent::FullscreenChanged { fullscreen } => {
                self.fullscreen = fullscreen;
            }
            // media session
            BrowserEvent::MediaPlaybackChanged { state } => self.media.set_playback(state),
            BrowserEvent::MediaMetadataChanged { metadata } => self.media.set_metadata(metadata),
            BrowserEvent::MediaPositionChanged { position } => self.media.set_position(position),
            BrowserEvent::MediaProbed { probe } => self.media.set_probe(probe),
            BrowserEvent::DocumentStarted => {}
        }
    }
}
