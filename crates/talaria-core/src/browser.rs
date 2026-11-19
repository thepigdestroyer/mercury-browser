// SPDX-License-Identifier: MPL-2.0
use url::Url;

use crate::{
    ClipboardCommand, ColorScheme, ControlAnswer, ControlId, ControlRect, ControlRequest,
    DEFAULT_ZOOM_PERCENT, ImeInput, KeyboardInput, PageIcon, PointerCursor,
};
use crate::{EndFindRequest, FindOutcome, FindRequest, FindSelection, FindState};
use crate::{PageRequest, PageRequestAnswer, PageRequestId};
// media session
use crate::{
    MediaMetadata, MediaPlaybackState, MediaPosition, MediaProbe, MediaSessionAction, MediaState,
    TabAudio,
};
// passwords
use crate::LoginFill;

const DEFAULT_PAGE_TITLE: &str = "New Tab";
const INITIAL_ADDRESS: &str = "";

/// Longest placeholder title made from a whole URL, ellipsis included.
const MAX_URL_TITLE_CHARS: usize = 80;

/// Title shown until a document supplies its own: the URL's host (a `blob:` URL's inner host),
/// else the URL shortened to `MAX_URL_TITLE_CHARS`, else "New Tab". A `data:` URL shows only
/// its scheme, since the rest is the document itself.
pub fn placeholder_title(url: &str) -> String {
    if url.is_empty() {
        return DEFAULT_PAGE_TITLE.to_owned();
    }
    let Ok(parsed) = Url::parse(url) else {
        return shortened(url);
    };
    let host = match parsed.scheme() {
        "data" => return "data:".to_owned(),
        "blob" => match parsed.origin() {
            url::Origin::Tuple(_, host, _) => Some(host.to_string()),
            url::Origin::Opaque(_) => None,
        },
        _ => parsed.host_str().map(str::to_owned),
    };
    host.filter(|host| !host.is_empty())
        .unwrap_or_else(|| shortened(url))
}

/// The origin a page's request is shown and remembered under: `scheme://host[:port]` for web
/// pages, `file://` for local files, None for anything else (opaque origins).
pub fn display_origin(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() == "file" {
        return Some("file://".to_owned());
    }
    match parsed.origin() {
        url::Origin::Tuple(..) => Some(parsed.origin().ascii_serialization()),
        url::Origin::Opaque(_) => None,
    }
}

fn shortened(text: &str) -> String {
    if text.chars().count() <= MAX_URL_TITLE_CHARS {
        return text.to_owned();
    }
    let mut short = text
        .chars()
        .take(MAX_URL_TITLE_CHARS - 1)
        .collect::<String>();
    short.push('…');
    short
}

/// Whether two addresses differ at most in their fragment, which keeps the same document.
fn same_document(current: &str, next: &str) -> bool {
    match (Url::parse(current), Url::parse(next)) {
        (Ok(mut current), Ok(mut next)) => {
            current.set_fragment(None);
            next.set_fragment(None);
            current == next
        }
        _ => current == next,
    }
}

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
    // controls
    /// Answers (or drops) a control the engine asked for; see `BrowserEvent::ControlShown`.
    AnswerControl {
        id: ControlId,
        answer: ControlAnswer,
    },
    // dialogs
    /// Answers a page request (see `page_request`); `Discard` gives the engine's default.
    AnswerPageRequest {
        id: PageRequestId,
        answer: PageRequestAnswer,
    },
    // menus
    /// Cut, Copy or Paste chosen in the Edit menu while the page has focus.
    Clipboard(ClipboardCommand),
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
    // find
    /// The find bar opened: read the page's selection (answered by `FindSelectionRead`).
    BeginFind {
        request_id: u64,
    },
    /// Search the page and select a match (answered by `FindResult`).
    Find(FindRequest),
    EndFind(EndFindRequest),
    // passwords
    /// Fills a login the user chose into the focused password field of the top-level document,
    /// if that document has the login's origin.
    FillLogin(LoginFill),
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

    /// How a navigation this command starts was reached, when the command alone says so.
    /// `Navigate` does not: `queue_navigation` and its callers name the origin.
    pub(crate) const fn navigation_origin(&self) -> Option<crate::NavigationOrigin> {
        match self {
            Self::Reload => Some(crate::NavigationOrigin::Reload),
            Self::GoBack | Self::GoForward => Some(crate::NavigationOrigin::History),
            _ => None,
        }
    }

    /// Answers go to the web view that asked even before its first page has committed: a
    /// sign-in for that page holds the commit until it is answered.
    pub(crate) const fn is_dispatched_before_engine_ready(&self) -> bool {
        matches!(self, Self::AnswerPageRequest { .. })
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
    // downloads
    /// The top-level page at `url` is the engine's own "Unknown content type" page: the
    /// response was of a type it cannot show (`mime_type`). `posted` says whether the request
    /// that fetched it was not a GET (a form, which cannot be fetched again); `None` when that
    /// request is not known. `referrer` is the page that request came from, if the engine sent
    /// one.
    DownloadOffered {
        url: String,
        mime_type: String,
        posted: Option<bool>,
        referrer: Option<String>,
    },
    /// A top-level request is starting: a new navigation, or (`redirect`) the next hop of one.
    /// `from` is the page the tab showed when it started, if any.
    /// `user_started`: the embedder decided, synchronously as the request started, that the
    /// user started this navigation (a gesture in the tab, or the address they typed).
    DocumentRequestStarted {
        url: String,
        redirect: bool,
        from: Option<String>,
        user_started: bool,
    },
    // tabs opened and closed by pages
    /// The page closed its own tab (`window.close()`).
    CloseRequested,
    // controls
    /// The engine asks for a `<select>` popup, a picker or a context menu. It waits for exactly
    /// one `BrowserCommand::AnswerControl` with this id.
    ControlShown {
        id: ControlId,
        request: ControlRequest,
    },
    /// An editable element has focus; the input-method window belongs at `rect`.
    InputMethodShown {
        id: ControlId,
        rect: ControlRect,
    },
    /// The engine closed a control or the input method itself; no answer is expected.
    ControlHidden {
        id: ControlId,
    },
    // openurl
    /// The page asked for a URL the engine cannot load (`mailto:`, `tel:`, an app's scheme) and
    /// the engine was refused. The controller queues it for
    /// `BrowserController::take_external_link_requests`.
    ExternalLinkRequested(crate::ExternalLinkRequest),
    // dialogs
    /// The page asks the user something; see `page_request`.
    PageRequested(PageRequest),
    /// A new document started loading in the tab (not a same-document navigation).
    PageLoadStarted,
    // media-capture
    /// The page's content process crashed (sent after the crash's `LoadStateChanged`). Unlike a
    /// `Failed` load state alone, which also reports an address the browser could not use, it
    /// means the page is gone.
    PageCrashed,
    // history
    /// The tab's session history changed (a new document, a fragment change, `pushState` or
    /// `replaceState`, a reload, back or forward, or a subframe navigating): `url` is the
    /// top-level URL now shown, at `history_index` of `history_length` entries. Sent after the
    /// matching `AddressChanged`; only history recording reads it.
    PageCommitted {
        url: String,
        history_length: usize,
        history_index: usize,
    },
    /// The page's title, with the URL the web view showed when the title was reported. Sent
    /// after the matching `PageTitleChanged`; only history recording reads it.
    PageTitleAt {
        url: String,
        title: Option<String>,
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
    // find
    FindResult {
        request_id: u64,
        outcome: FindOutcome,
    },
    /// The answer to `BeginFind`; `None` if the page has no selection or could not be read.
    FindSelectionRead {
        request_id: u64,
        selection: Option<FindSelection>,
    },
    // site security
    /// The tab shows one of Mercury's own error pages for `url`.
    ErrorPageShown {
        kind: crate::ErrorPageKind,
        url: String,
    },
    /// HTTPS-only mode: the engine replaced a top-level load of `from` with one of `to`.
    HttpsUpgraded {
        from: String,
        to: String,
    },
    /// HTTPS-only mode: the engine refused a top-level load of `url`, which it could not
    /// upgrade (a form submission, or a site that sends its HTTPS address back to HTTP).
    HttpsOnlyBlocked {
        url: String,
    },
    // passwords
    /// The context menu `menu` was opened in the same frame as the focused password field
    /// `field` (an input-method control of a password input). Sent right after the menu's
    /// `ControlShown`.
    PasswordFieldMenu {
        menu: ControlId,
        field: ControlId,
    },
}

/// A page the engine could not show, offered to the browser as a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOffer {
    pub url: String,
    pub mime_type: String,
    pub posted: Option<bool>,
    pub referrer: Option<String>,
}

/// What the downloads work learns from a tab's engine events, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadSignal {
    /// A top-level request started (`redirect`: the next hop of the same navigation).
    /// `from` is the page the tab showed when it started.
    RequestStarted {
        url: String,
        redirect: bool,
        from: Option<String>,
        user_started: bool,
    },
    /// A page the engine could not show.
    Offered(DownloadOffer),
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
    // controls
    pending_control: Option<(ControlId, ControlRequest)>,
    input_method: Option<(ControlId, ControlRect)>,
    // media session
    media: MediaState,
    /// The user's choice; kept across navigations in the tab.
    audio_muted: bool,
    // find
    find: FindState,
    // passwords: the context menu and the password field it was opened over.
    password_field_menu: Option<(ControlId, ControlId)>,
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
            pending_control: None,
            input_method: None,
            // media session
            media: MediaState::default(),
            audio_muted: false,
            // find
            find: FindState::default(),
            password_field_menu: None,
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

    pub(crate) fn set_page_title(&mut self, title: String) {
        self.page_title = title;
    }

    // find
    pub const fn find(&self) -> &FindState {
        &self.find
    }

    pub(crate) fn find_mut(&mut self) -> &mut FindState {
        &mut self.find
    }

    pub(crate) fn set_zoom_percent(&mut self, percent: u16) {
        self.zoom_percent = percent;
    }

    /// The control waiting for the user's answer, if any.
    pub fn pending_control(&self) -> Option<(ControlId, &ControlRequest)> {
        self.pending_control
            .as_ref()
            .map(|(id, request)| (*id, request))
    }

    /// Where the focused editable element is, while the input method is active.
    pub fn input_method(&self) -> Option<ControlRect> {
        self.input_method.map(|(_, rect)| rect)
    }

    pub(crate) fn forget_controls(&mut self) {
        self.pending_control = None;
        self.input_method = None;
    }

    pub(crate) fn take_pending_control(&mut self) -> Option<ControlId> {
        self.pending_control.take().map(|(id, _)| id)
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
            BrowserEvent::PageLoadStarted
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
                // dialogs: Servo writes credentials typed into a sign-in into the request URL,
                // and so into the page's address; they are never shown or kept.
                let url = crate::strip_user_info(&url);
                // Cursor and status text are deliberately not reset here: the engine only
                // reports changes against its own cached values, so the engine's notifications
                // must stay the only source of both.
                if self.url != url {
                    if !same_document(&self.url, &url) {
                        self.page_title = placeholder_title(&url);
                        self.page_icon = None;
                        // find
                        self.find.page_changed();
                    }
                    self.url = url;
                }
            }
            BrowserEvent::PageTitleChanged { title } => {
                self.page_title = title
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| placeholder_title(&self.url));
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
                    // The engine dropped every request of the crashed page.
                    self.pending_control = None;
                    self.input_method = None;
                    // find
                    self.find.page_changed();
                }
                // find: a load starting (a reload too) replaces the document
                if state == LoadState::Loading && self.load_state != LoadState::Loading {
                    self.find.page_changed();
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
            // downloads: queued by the controller, not shown.
            BrowserEvent::DownloadOffered { .. } | BrowserEvent::DocumentRequestStarted { .. } => {}
            // Closing is the controller's business; the tab's own state does not change.
            BrowserEvent::CloseRequested => {}
            BrowserEvent::ControlShown { id, request } => {
                self.pending_control = Some((id, request));
            }
            BrowserEvent::InputMethodShown { id, rect } => {
                self.input_method = Some((id, rect));
            }
            BrowserEvent::ControlHidden { id } => {
                if self
                    .pending_control
                    .as_ref()
                    .is_some_and(|(pending_id, _)| *pending_id == id)
                {
                    self.pending_control = None;
                }
                if self
                    .input_method
                    .is_some_and(|(input_method_id, _)| input_method_id == id)
                {
                    self.input_method = None;
                }
            }
            // openurl: not part of what the tab shows; see `BrowserController::handle_event`.
            BrowserEvent::ExternalLinkRequested(_) => {}
            // dialogs: kept by the tab's page requests, not here.
            BrowserEvent::PageRequested(_) | BrowserEvent::PageLoadStarted => {}
            // media-capture: kept by the tab's page requests.
            BrowserEvent::PageCrashed => {}
            // history: read by the controller's visit tracker, not shown.
            BrowserEvent::PageCommitted { .. } | BrowserEvent::PageTitleAt { .. } => {}
            // media session
            BrowserEvent::MediaPlaybackChanged { state } => self.media.set_playback(state),
            BrowserEvent::MediaMetadataChanged { metadata } => self.media.set_metadata(metadata),
            BrowserEvent::MediaPositionChanged { position } => self.media.set_position(position),
            BrowserEvent::MediaProbed { probe } => self.media.set_probe(probe),
            // find
            BrowserEvent::FindResult {
                request_id,
                outcome,
            } => self.find.apply_result(request_id, outcome),
            BrowserEvent::FindSelectionRead {
                request_id,
                selection,
            } => self.find.apply_selection(request_id, selection),
            // site security: kept by the tab's site security, not here.
            BrowserEvent::ErrorPageShown { .. }
            | BrowserEvent::HttpsUpgraded { .. }
            | BrowserEvent::HttpsOnlyBlocked { .. } => {}
            // passwords
            BrowserEvent::PasswordFieldMenu { menu, field } => {
                self.password_field_menu = Some((menu, field));
            }
        }
    }

    // passwords
    /// The control id of the focused editable element, while the input method is active.
    pub fn input_method_id(&self) -> Option<ControlId> {
        self.input_method.map(|(id, _)| id)
    }

    /// The focused password field, when the pending control is a context menu opened over it
    /// (in its frame) and the field still has focus.
    pub fn password_field_menu(&self) -> Option<ControlId> {
        let (menu, field) = self.password_field_menu?;
        let menu_pending = self.pending_control.as_ref().is_some_and(|(id, request)| {
            *id == menu && matches!(request, ControlRequest::ContextMenu { .. })
        });
        (menu_pending && self.input_method_id() == Some(field)).then_some(field)
    }
}
