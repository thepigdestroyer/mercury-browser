// SPDX-License-Identifier: MPL-2.0
mod address;
mod browser;
// certificate exceptions
mod certificate_exception;
// media-capture
mod capture;
mod clipboard;
mod controller;
mod controls;
mod cursor;
mod find;
// groups-core
mod groups;
mod input;
mod media;
// openurl
mod navigation_policy;
// passwords
mod login_fill;
mod page_icon;
mod page_request;
// sh-pages
mod site_images;
mod site_security;
mod tab;
// txt-1: IME input and the focused editable's text state
mod text_input;
// sh-pages
mod view_source;
mod visits;
mod web_view;
mod zoom;

pub use address::{
    AddressResolutionError, NavigationTarget, SearchUrlBuilder, classify_address, resolve_address,
};
pub use browser::{
    BrowserCommand, BrowserEvent, BrowserState, DownloadOffer, DownloadSignal, FieldFocusInfo,
    FieldInputType, LoadState, PasswordSubmittedInfo, PointerButton, PointerButtonAction,
    display_origin, placeholder_title,
};
// media-capture
pub use capture::CaptureAccess;
pub use clipboard::{
    ASYNC_CLIPBOARD_OFFERED, AsyncAccess, CLIPBOARD_READ_FRAME_GATE, ClipboardGate,
    ClipboardReadDecision, ClipboardRequestOrigin, ClipboardSourceKind, MemoryPasteboard,
    Pasteboard, PasteboardImage, PasteboardItem, RefusalReason, USER_CLIPBOARD_ACTION_WINDOW,
    async_read_access, async_write_access, clipboard_shortcut, context_menu_clipboard_command,
};
pub use controller::{BrowserController, CloseTabOutcome};
// tabs-plus
pub use controller::{PinnedCloseOutcome, tab_matches_query};
pub use controls::{
    ContextMenuAction, ContextMenuEntry, ContextMenuInfo, ControlAnswer, ControlId, ControlRect,
    ControlRequest, DateTimeKind, MediaAction, Rgb, SelectEntry, SelectOption,
    context_menu_search_text,
};
pub use cursor::{ColorScheme, PointerCursor};
pub use find::{
    CONTINUATION_ROUNDS, EndFindRequest, FindMatches, FindOutcome, FindRequest, FindSelection,
    FindState, FindStatus, FindStep,
};
// groups-core
pub use groups::{
    ClosedGroup, DEFAULT_GROUP_KEY_SEQUENCE, GroupBook, GroupColour, GroupError, GroupId, GroupKey,
    GroupPreset, GroupSummary, MAX_GROUP_ID, MAX_GROUP_NAME_BYTES, MAX_GROUPS, RestoredGroup,
    TabGroup, TabStatus, clean_group_name, load_progress_estimate,
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
// certificate exceptions
pub use certificate_exception::{certificate_exception_allowed, certificate_exception_host};
// site security
pub use site_security::{
    ErrorPageKind, HttpsOnlyInterstitial, HttpsPolicy, HttpsUpgradeChain, MainFrameLoad,
    SecurityIndicator, is_local_host, main_frame_load, security_indicator, typed_https_upgrade,
};
pub use tab::{RestoredTab, Tab, TabActivation, TabCommand, TabEvent, TabId};
// txt-1
pub use text_input::{
    ImeClause, ImeClauseKind, ImeReplacement, TextInputState, Utf16Range, utf8_offset_of_utf16,
    utf16_len, utf16_offset_of_byte,
};
pub use visits::{NavigationOrigin, VisitKey, VisitUpdate};
pub use web_view::{WebView, dispatch_command};
pub use zoom::{DEFAULT_ZOOM_PERCENT, ZOOM_STEPS, zoom_in, zoom_out};
// openurl
pub use navigation_policy::{
    ExternalLinkRequest, NavigationVerdict, navigation_verdict, refuses_local_file_load,
    remembered_origin,
};
// sh-pages
pub use site_images::{MAX_IMAGE_BLOCKED_SITES, SiteImages, SiteImagesError, image_site_key};
pub use view_source::{
    VIEW_SOURCE_SCHEME, is_view_source, view_source_address, view_source_normalised,
    view_source_target,
};
