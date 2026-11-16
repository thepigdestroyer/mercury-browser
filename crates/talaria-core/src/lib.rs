mod address;
mod browser;
mod controller;
mod cursor;
mod input;
mod media;
mod page_icon;
mod tab;
mod web_view;
mod zoom;

pub use address::{AddressResolutionError, NavigationTarget, classify_address, resolve_address};
pub use browser::{
    BrowserCommand, BrowserEvent, BrowserState, LoadState, PointerButton, PointerButtonAction,
};
pub use controller::BrowserController;
pub use cursor::{ColorScheme, PointerCursor};
pub use input::{
    CompositionState, ImeInput, KeyLocation, KeyState, KeyboardInput, KeyboardModifiers,
};
// media session
pub use media::{
    MediaMetadata, MediaPlaybackState, MediaPosition, MediaProbe, MediaSessionAction, MediaState,
    TabAudio,
};
pub use page_icon::PageIcon;
pub use tab::{Tab, TabActivation, TabCommand, TabEvent, TabId};
pub use web_view::{WebView, dispatch_command};
pub use zoom::{DEFAULT_ZOOM_PERCENT, ZOOM_STEPS, zoom_in, zoom_out};
