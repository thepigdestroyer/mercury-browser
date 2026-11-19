// SPDX-License-Identifier: MPL-2.0
mod address;
mod browser;
// media-capture
mod capture;
mod clipboard;
mod controller;
mod controls;
mod cursor;
mod find;
mod input;
mod media;
// openurl
mod navigation_policy;
// passwords
mod login_fill;
mod page_icon;
mod page_request;
mod site_security;
mod tab;
mod visits;
mod web_view;
mod zoom;

pub use address::{
    AddressResolutionError, NavigationTarget, SearchUrlBuilder, classify_address, resolve_address,
};
pub use browser::{
    BrowserCommand, BrowserEvent, BrowserState, DownloadOffer, DownloadSignal, LoadState,
    PointerButton, PointerButtonAction, display_origin, placeholder_title,
};
// media-capture
pub use capture::CaptureAccess;
pub use clipboard::{
    ASYNC_CLIPBOARD_OFFERED, CLIPBOARD_READ_FRAME_GATE, ClipboardGate, ClipboardReadDecision,
    MemoryPasteboard, Pasteboard, USER_CLIPBOARD_ACTION_WINDOW, clipboard_shortcut,
    context_menu_clipboard_command,
};
pub use controller::{BrowserController, CloseTabOutcome};
// tabs-plus
pub use controller::{PinnedCloseOutcome, tab_matches_query};
pub use controls::{
    ContextMenuAction, ContextMenuEntry, ControlAnswer, ControlId, ControlRect, ControlRequest,
    Rgb, SelectEntry, SelectOption,
};
pub use cursor::{ColorScheme, PointerCursor};
pub use find::{
    CONTINUATION_ROUNDS, EndFindRequest, FindMatches, FindOutcome, FindRequest, FindSelection,
    FindState, FindStatus, FindStep,
};
pub use input::{
    ClipboardCommand, CompositionState, ImeInput, KeyLocation, KeyState, KeyboardInput,
    KeyboardModifiers,
};
// media session
pub use media::{
    MediaMetadata, MediaPlaybackState, MediaPosition, MediaProbe, MediaSessionAction, MediaState,
    TabAudio,
};
// passwords
pub use login_fill::LoginFill;
pub use page_icon::PageIcon;
pub use page_request::{
    GEOLOCATION_FRAME_GATE, GEOLOCATION_OFFERED, HttpCredentials, MAX_COMBINING_MARKS,
    MAX_MESSAGE_CHARS, MAX_PROMPT_VALUE_CHARS, PageOrigin, PagePermission, PageRequest,
    PageRequestAnswer, PageRequestId, PageRequestKind, PageRequestReply, PendingPageRequest,
    PermissionDecision, PermissionDecisions, SIGN_IN_RETRY_INTERVAL, SUCCESSIVE_DIALOG_INTERVAL,
    SessionPermissionDecisions, sanitize_page_text, strip_user_info,
};
// site security
pub use site_security::{
    ErrorPageKind, HttpsOnlyInterstitial, HttpsPolicy, HttpsUpgradeChain, MainFrameLoad,
    SecurityIndicator, is_local_host, main_frame_load, security_indicator, typed_https_upgrade,
};
pub use tab::{RestoredTab, Tab, TabActivation, TabCommand, TabEvent, TabId};
pub use visits::{NavigationOrigin, VisitKey, VisitUpdate};
pub use web_view::{WebView, dispatch_command};
pub use zoom::{DEFAULT_ZOOM_PERCENT, ZOOM_STEPS, zoom_in, zoom_out};
// openurl
pub use navigation_policy::{
    ExternalLinkRequest, NavigationVerdict, navigation_verdict, refuses_local_file_load,
    remembered_origin,
};
