// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the browser does with a navigation a page asks for (a link, a form, `location = …`):
//! the engine loads it, it is refused, or it belongs to another app.

use url::Url;

use std::time::Duration;

use crate::PageRequestId;

/// A page's request to open a URL in another app, queued by the controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLinkRequest {
    /// The id the confirmation gets if the user is asked (`PageRequestKind::OpenExternally`).
    pub id: PageRequestId,
    pub url: String,
    /// The tab's top-level page when it asked, if one had committed.
    pub page_url: Option<String>,
    /// On the clock of `PageRequest::received_at`.
    pub received_at: Duration,
}

/// Schemes never handed to another app from a page, even after asking: network shares and file
/// transfer (`smb:`, `cifs:`, `afp:`, `nfs:`, `ftp:`, `sftp:`), remote shells and screens
/// (`ssh:`, `telnet:`, `vnc:`), System Settings, manual and help pages that can run commands or
/// open system UI (`x-man-page:`, `help:`), and app installation (`itms-services:`).
const NEVER_EXTERNAL_SCHEMES: [&str; 13] = [
    "smb",
    "cifs",
    "afp",
    "nfs",
    "ftp",
    "sftp",
    "ssh",
    "telnet",
    "vnc",
    "x-apple.systempreferences",
    "x-man-page",
    "help",
    "itms-services",
];

/// Schemes the engine loads itself. `javascript:` URLs are evaluated by the engine before a
/// navigation is requested, so they never arrive here as such, but they are the engine's.
const ENGINE_SCHEMES: [&str; 7] = [
    "http",
    "https",
    "file",
    "data",
    "blob",
    "about",
    "javascript",
];

/// The answer to a page's navigation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationVerdict {
    /// The engine loads it.
    Allow,
    /// A page that is not itself a local file asked for a `file:` URL. Typed addresses and files
    /// opened from other apps never ask, so they are not affected.
    DenyLocalFile,
    /// A page asked for a `data:` URL. Refused as Chrome and Firefox refuse top-level ones (a
    /// phishing vector); Servo's request does not say which frame asks, so a frame navigated to
    /// one by script is refused too. A frame's `src` and typed addresses never ask.
    DenyData,
    /// A scheme that is never handed to another app from a page, even after asking (see
    /// `NEVER_EXTERNAL_SCHEMES`).
    DenyScheme,
    /// The engine cannot load the scheme (`mailto:`, `tel:`, an app's own scheme). The engine is
    /// refused; the system's handler for the scheme may open it, after a user gesture and the
    /// user's confirmation.
    OpenExternally,
}

/// `page` is the URL of the page shown in the tab when it asked, if one has committed.
pub fn navigation_verdict(page: Option<&Url>, target: &Url) -> NavigationVerdict {
    let scheme = target.scheme();
    if NEVER_EXTERNAL_SCHEMES.contains(&scheme) {
        return NavigationVerdict::DenyScheme;
    }
    if !ENGINE_SCHEMES.contains(&scheme) {
        return NavigationVerdict::OpenExternally;
    }
    if scheme == "data" {
        return NavigationVerdict::DenyData;
    }
    if scheme == "file" && page.is_none_or(|page| page.scheme() != "file") {
        return NavigationVerdict::DenyLocalFile;
    }
    NavigationVerdict::Allow
}

/// Whether the engine must not fetch `target` for the tab whose top-level page is `page`: a
/// `file:` URL loaded into a frame or as a resource (image, script, style sheet) of a page that
/// is not itself a local file. The main frame's own load is not judged here: a navigation a page
/// asks for is judged by `navigation_verdict`, and a typed address or a file opened from another
/// app is the user's choice.
pub fn refuses_local_file_load(page: Option<&Url>, target: &Url, is_main_frame: bool) -> bool {
    !is_main_frame && target.scheme() == "file" && page.is_none_or(|page| page.scheme() != "file")
}

/// The key under which "always allow" is remembered for the session: the page's origin, or
/// `None` when the origin is opaque (a local file, a `data:` page), which is never remembered.
/// `page` is always the tab's top-level page: Servo's `NavigationRequest` carries no source
/// frame, so a link in a frame from another site is judged, named and remembered as the
/// top-level page's.
pub fn remembered_origin(page: Option<&Url>) -> Option<String> {
    let origin = page?.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).expect("the test URL is valid")
    }

    #[test]
    fn web_schemes_are_loaded_by_the_engine() {
        let page = url("https://example.com/");
        for target in [
            "https://example.com/next",
            "http://example.com/",
            "blob:https://example.com/0d6b2f4e-6d3c-4a8e-9a1d-1f1f1f1f1f1f",
            "about:blank",
        ] {
            assert_eq!(
                navigation_verdict(Some(&page), &url(target)),
                NavigationVerdict::Allow,
                "{target}"
            );
        }
    }

    #[test]
    fn schemes_the_engine_cannot_load_belong_to_other_apps() {
        let page = url("https://example.com/");
        for target in [
            "mailto:someone@example.com",
            "tel:+15555550123",
            "sms:+15555550123",
            "facetime:someone@example.com",
            "zoommtg://zoom.us/join?confno=1",
            "slack://open",
            "chrome:allowcert",
        ] {
            assert_eq!(
                navigation_verdict(Some(&page), &url(target)),
                NavigationVerdict::OpenExternally,
                "{target}"
            );
        }
    }

    #[test]
    fn a_web_page_cannot_reach_a_local_file() {
        let target = url("file:///etc/hosts");
        for page in [
            "https://example.com/",
            "http://127.0.0.1:8000/page.html",
            "about:blank",
            "data:text/html,hi",
            "blob:https://example.com/0d6b2f4e-6d3c-4a8e-9a1d-1f1f1f1f1f1f",
        ] {
            assert_eq!(
                navigation_verdict(Some(&url(page)), &target),
                NavigationVerdict::DenyLocalFile,
                "{page}"
            );
        }
        assert_eq!(
            navigation_verdict(None, &target),
            NavigationVerdict::DenyLocalFile
        );
    }

    #[test]
    fn a_local_file_may_link_to_another_local_file() {
        assert_eq!(
            navigation_verdict(
                Some(&url("file:///Users/me/notes/index.html")),
                &url("file:///Users/me/notes/page.html")
            ),
            NavigationVerdict::Allow
        );
    }

    #[test]
    fn scheme_names_are_compared_after_parsing() {
        let page = url("https://example.com/");
        assert_eq!(
            navigation_verdict(Some(&page), &url("FILE:///etc/hosts")),
            NavigationVerdict::DenyLocalFile
        );
        assert_eq!(
            navigation_verdict(Some(&page), &url("MailTo:a@example.com")),
            NavigationVerdict::OpenExternally
        );
    }

    #[test]
    fn web_pages_cannot_load_local_files_into_frames_or_as_resources() {
        let target = url("file:///etc/hosts");
        let web = url("https://example.com/");
        assert!(refuses_local_file_load(Some(&web), &target, false));
        assert!(refuses_local_file_load(None, &target, false));
        assert!(refuses_local_file_load(
            Some(&url("about:blank")),
            &target,
            false
        ));
    }

    #[test]
    fn local_pages_load_local_files_and_top_level_loads_are_decided_elsewhere() {
        let target = url("file:///Users/me/notes/picture.png");
        assert!(!refuses_local_file_load(
            Some(&url("file:///Users/me/notes/index.html")),
            &target,
            false
        ));
        // A typed address or a file from another app: the main frame's load is the user's.
        assert!(!refuses_local_file_load(
            Some(&url("https://example.com/")),
            &target,
            true
        ));
        assert!(!refuses_local_file_load(
            Some(&url("https://example.com/")),
            &url("https://example.com/a.png"),
            false
        ));
    }

    #[test]
    fn only_tuple_origins_are_remembered() {
        assert_eq!(
            remembered_origin(Some(&url("https://example.com:8443/a?b"))),
            Some("https://example.com:8443".to_owned())
        );
        assert_eq!(remembered_origin(Some(&url("file:///tmp/a.html"))), None);
        assert_eq!(remembered_origin(Some(&url("data:text/html,x"))), None);
        assert_eq!(remembered_origin(None), None);
    }

    #[test]
    fn pages_cannot_open_data_urls() {
        // Chrome and Firefox refuse top-level `data:` navigations a page starts; Servo does not
        // say which frame asks, so frames navigated by their own script are refused too. A
        // frame's `src` never asks and stays allowed.
        for page in ["https://example.com/", "file:///tmp/a.html", "about:blank"] {
            assert_eq!(
                navigation_verdict(Some(&url(page)), &url("data:text/html,<p>hi")),
                NavigationVerdict::DenyData,
                "{page}"
            );
        }
        assert_eq!(
            navigation_verdict(None, &url("data:,x")),
            NavigationVerdict::DenyData
        );
    }

    #[test]
    fn network_shares_and_system_settings_never_open_from_a_page() {
        let page = url("https://example.com/");
        for target in [
            "smb://server/share",
            "afp://server/share",
            "nfs://server/export",
            "vnc://server",
            "ftp://example.com/file",
            "x-apple.systempreferences:com.apple.preference.security",
            "SMB://server/share",
            "cifs://server/share",
            "sftp://server/file",
            "ssh://server",
            "telnet://server",
            "x-man-page://ls",
            "help:anchor=x",
            "itms-services://?action=download-manifest&url=https://example.com/a.plist",
        ] {
            assert_eq!(
                navigation_verdict(Some(&page), &url(target)),
                NavigationVerdict::DenyScheme,
                "{target}"
            );
        }
    }
}
