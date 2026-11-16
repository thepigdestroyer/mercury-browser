mod address;
mod browser;
mod controller;
mod cursor;
mod input;
// openurl
mod navigation_policy;
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
pub use page_icon::PageIcon;
pub use tab::{Tab, TabActivation, TabCommand, TabEvent, TabId};
pub use web_view::{WebView, dispatch_command};
pub use zoom::{DEFAULT_ZOOM_PERCENT, ZOOM_STEPS, zoom_in, zoom_out};
// openurl
pub use navigation_policy::{
    ExternalLinkRequest, ExternalOpenPrompt, NavigationVerdict, external_open_prompt,
    navigation_verdict, page_display_name, refuses_local_file_load, remembered_origin,
};
