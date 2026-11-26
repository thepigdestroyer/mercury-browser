/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the page lane. Add variants (and the payload types they need) here
// only, never in another lane's file or in a shared enum.

use std::fmt;

use serde::{Deserialize, Serialize};
use servo_base::id::{BrowsingContextId, PipelineId};
use strum::IntoStaticStr;

/// The longest origin string Mercury accepts; longer ones are reported as `null`.
pub const MAX_ORIGIN_BYTES: usize = 2048;
/// The longest status text Mercury keeps.
pub const MAX_STATUS_TEXT_BYTES: usize = 128;
/// The response headers Mercury keeps, by lower-case name.
pub const ALLOWED_RESPONSE_HEADERS: [&str; 7] = [
    "content-type",
    "content-disposition",
    "content-length",
    "content-language",
    "last-modified",
    "cache-control",
    "etag",
];
/// The most response headers Mercury keeps.
pub const MAX_RESPONSE_HEADERS: usize = 16;
/// The longest response header value Mercury keeps.
pub const MAX_HEADER_VALUE_BYTES: usize = 512;
/// The longest `window.open` features string that is inspected.
const MAX_FEATURES_BYTES: usize = 1024;

/// Who a document is: the origin of the document and of its top-level document.
///
/// The origins are ASCII serialisations, `null` when opaque or unknown. `Debug` prints no field.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RequesterInfo {
    pub pipeline_id: PipelineId,
    pub browsing_context_id: BrowsingContextId,
    /// ASCII serialisation of the document's origin; `null` when opaque.
    pub origin: String,
    /// The same for the top-level document; `null` when opaque or unknown.
    pub top_level_origin: String,
    pub is_top_level: bool,
}

impl fmt::Debug for RequesterInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("RequesterInfo")
    }
}

/// The main document's response: its final URL, status and the allow-listed headers.
///
/// `Debug` prints no field.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MainDocumentResponse {
    pub pipeline_id: PipelineId,
    /// The final URL after redirects, with the user-info stripped.
    pub url: String,
    pub status: u16,
    /// Capped at [`MAX_STATUS_TEXT_BYTES`].
    pub status_text: String,
    pub content_type: Option<String>,
    /// Allow-listed headers with lower-case names.
    pub headers: Vec<(String, String)>,
}

impl fmt::Debug for MainDocumentResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("MainDocumentResponse")
    }
}

/// Keeps the allow-listed headers (case-insensitive name), lower-cases the name, cuts a value at
/// [`MAX_HEADER_VALUE_BYTES`] on a char boundary, keeps the first [`MAX_RESPONSE_HEADERS`] and
/// drops every other header.
pub fn allow_listed_headers<'a>(
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<(String, String)> {
    headers
        .into_iter()
        .filter_map(|(name, value)| {
            let name = name.to_ascii_lowercase();
            if !ALLOWED_RESPONSE_HEADERS.contains(&name.as_str()) {
                return None;
            }
            Some((name, truncate_at_char_boundary(value, MAX_HEADER_VALUE_BYTES)))
        })
        .take(MAX_RESPONSE_HEADERS)
        .collect()
}

fn truncate_at_char_boundary(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

/// Where a window opened by `window.open`, a link or a middle click goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum OpenDisposition {
    ForegroundTab,
    BackgroundTab,
    Popup,
}

impl OpenDisposition {
    /// `Popup` when the features string asks for `popup` (not `popup=0`, `popup=false` or
    /// `popup=no`) or for a size or position; otherwise `ForegroundTab`. Only the first
    /// [`MAX_FEATURES_BYTES`] bytes are inspected.
    pub fn from_features(features: &str) -> OpenDisposition {
        let features = truncate_at_char_boundary(features, MAX_FEATURES_BYTES);
        let popup = features.split(',').any(|feature| {
            let (name, value) = match feature.split_once('=') {
                Some((name, value)) => (name.trim(), Some(value.trim())),
                None => (feature.trim(), None),
            };
            match name.to_ascii_lowercase().as_str() {
                "popup" => !matches!(
                    value.map(str::to_ascii_lowercase).as_deref(),
                    Some("0" | "false" | "no")
                ),
                "width" | "height" | "left" | "top" | "screenx" | "screeny" => true,
                _ => false,
            }
        });
        if popup {
            OpenDisposition::Popup
        } else {
            OpenDisposition::ForegroundTab
        }
    }
}

/// How a window opening request was made.
///
/// `Default` is a foreground tab, no activation, no features.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpenHints {
    /// Whether the opener had user activation when the request was made.
    pub user_activation: bool,
    pub disposition: OpenDisposition,
    pub features: Option<String>,
}

impl Default for OpenHints {
    fn default() -> Self {
        Self {
            user_activation: false,
            disposition: OpenDisposition::ForegroundTab,
            features: None,
        }
    }
}

impl fmt::Debug for OpenHints {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("OpenHints")
    }
}

/// Why a page is asked whether it may unload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum UnloadReason {
    CloseTab,
    Quit,
    Navigation,
    Traversal,
}

/// Messages from the engine to Mercury's page lane. `Debug` prints only the variant name.
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum PageMsg {
    DocumentIdentity(RequesterInfo),
    MainDocumentResponse(MainDocumentResponse),
    LoadStopped { had_pending_navigation: bool },
    UnloadPermission { request_id: u64, allowed: bool },
    OpenWebViewRequested(OpenHints),
}

impl fmt::Debug for PageMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}

#[cfg(test)]
mod tests {
    use servo_base::id::{BrowsingContextId, PipelineId};

    use super::*;

    fn pipeline() -> PipelineId {
        PipelineId::new()
    }

    #[test]
    fn allow_listed_headers_drops_set_cookie_and_caps() {
        let long = "x".repeat(600);
        let etag = format!("\"{}\"", "e".repeat(10));
        let mut headers: Vec<(&str, &str)> = vec![("Set-Cookie", "a=b")];
        for _ in 0..20 {
            headers.push(("ETag", &etag));
        }
        headers.insert(0, ("Content-Type", &long));

        let kept = allow_listed_headers(headers);

        assert_eq!(kept.len(), MAX_RESPONSE_HEADERS);
        assert!(kept.iter().all(|(name, _)| name != "set-cookie"));
        assert!(kept.iter().all(|(name, _)| name.chars().all(|c| !c.is_ascii_uppercase())));
        assert_eq!(kept[0].0, "content-type");
        assert_eq!(kept[0].1.len(), MAX_HEADER_VALUE_BYTES);
    }

    #[test]
    fn allow_listed_headers_cuts_on_a_char_boundary() {
        let value = "é".repeat(300);
        let kept = allow_listed_headers([("Content-Type", value.as_str())]);
        assert_eq!(kept[0].1.len(), MAX_HEADER_VALUE_BYTES);
        assert!(kept[0].1.chars().all(|c| c == 'é'));
    }

    #[test]
    fn from_features_popup_and_sizes() {
        use OpenDisposition::{ForegroundTab, Popup};
        assert_eq!(OpenDisposition::from_features("popup"), Popup);
        assert_eq!(OpenDisposition::from_features("popup=0"), ForegroundTab);
        assert_eq!(OpenDisposition::from_features("popup=false"), ForegroundTab);
        assert_eq!(OpenDisposition::from_features("popup=no"), ForegroundTab);
        assert_eq!(OpenDisposition::from_features("width=300,height=200"), Popup);
        assert_eq!(OpenDisposition::from_features("noopener"), ForegroundTab);
        assert_eq!(OpenDisposition::from_features(""), ForegroundTab);
        assert_eq!(OpenDisposition::from_features(&"junk,".repeat(1024)), ForegroundTab);
    }

    #[test]
    fn page_messages_debug_names_only() {
        let message = PageMsg::MainDocumentResponse(MainDocumentResponse {
            pipeline_id: pipeline(),
            url: "https://secret.example/private".to_owned(),
            status: 200,
            status_text: "OK".to_owned(),
            content_type: None,
            headers: Vec::new(),
        });
        assert_eq!(format!("{message:?}"), "MainDocumentResponse");
        let message = PageMsg::LoadStopped { had_pending_navigation: true };
        assert_eq!(format!("{message:?}"), "LoadStopped");
    }

    #[test]
    fn requester_info_debug_has_no_origin() {
        let info = RequesterInfo {
            pipeline_id: pipeline(),
            browsing_context_id: BrowsingContextId::new(),
            origin: "https://secret.example".to_owned(),
            top_level_origin: "https://secret.example".to_owned(),
            is_top_level: true,
        };
        let printed = format!("{info:?}");
        assert!(!printed.contains("secret"), "{printed}");
    }
}
