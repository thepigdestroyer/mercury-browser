//! What the browser does with a navigation a page asks for (a link, a form, `location = …`):
//! the engine loads it, it is refused, or it belongs to another app.

use url::Url;

use crate::TabId;

/// A page's request to open a URL in another app, queued by the controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLinkRequest {
    pub tab_id: TabId,
    pub url: String,
    /// The page shown in the tab when it asked, if one had committed.
    pub page_url: Option<String>,
}

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
    /// The engine cannot load the scheme (`mailto:`, `tel:`, an app's own scheme). The engine is
    /// refused; the system's handler for the scheme may open it, after a user gesture and the
    /// user's confirmation.
    OpenExternally,
}

/// `page` is the URL of the page shown in the tab when it asked, if one has committed.
pub fn navigation_verdict(page: Option<&Url>, target: &Url) -> NavigationVerdict {
    let scheme = target.scheme();
    if !ENGINE_SCHEMES.contains(&scheme) {
        return NavigationVerdict::OpenExternally;
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

/// How a page is named to the user when it asks for something: its host, or "This page" when it
/// has none (a local file, a `data:` page).
pub fn page_display_name(page: Option<&Url>) -> String {
    page.and_then(Url::host_str)
        .filter(|host| !host.is_empty())
        .map_or_else(|| "This page".to_owned(), str::to_owned)
}

/// The key under which "always allow" is remembered for the session: the page's origin, or
/// `None` when the origin is opaque (a local file, a `data:` page), which is never remembered.
pub fn remembered_origin(page: Option<&Url>) -> Option<String> {
    let origin = page?.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

/// The confirmation shown before a page's link opens in another app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalOpenPrompt {
    /// "example.com wants to open Mail".
    pub message: String,
    /// The address being opened, shown under the message.
    pub detail: String,
    /// The button that opens it: "Open Mail", or "Open" when the app is not known.
    pub open_label: String,
    /// "Always allow example.com to open mailto links", or `None` when the page's origin cannot
    /// be remembered.
    pub always_label: Option<String>,
}

/// `app` is the name of the app the system would use, when it is known.
pub fn external_open_prompt(
    page: Option<&Url>,
    target: &Url,
    app: Option<&str>,
) -> ExternalOpenPrompt {
    let page_name = page_display_name(page);
    let scheme = target.scheme();
    let (message, open_label) = match app {
        Some(app) => (
            format!("{page_name} wants to open {app}"),
            format!("Open {app}"),
        ),
        None => (
            format!("{page_name} wants to open a {scheme} link in another app"),
            "Open".to_owned(),
        ),
    };
    let always_label =
        remembered_origin(page).map(|_| format!("Always allow {page_name} to open {scheme} links"));
    ExternalOpenPrompt {
        message,
        detail: target.as_str().to_owned(),
        open_label,
        always_label,
    }
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
            "data:text/html,hello",
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
            "ftp://example.com/file",
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
    fn pages_are_named_by_their_host() {
        assert_eq!(
            page_display_name(Some(&url("https://mail.example.com:8443/inbox"))),
            "mail.example.com"
        );
        assert_eq!(
            page_display_name(Some(&url("file:///tmp/a.html"))),
            "This page"
        );
        assert_eq!(
            page_display_name(Some(&url("data:text/html,x"))),
            "This page"
        );
        assert_eq!(page_display_name(None), "This page");
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
    fn the_prompt_names_the_page_and_the_app() {
        assert_eq!(
            external_open_prompt(
                Some(&url("https://example.com/contact")),
                &url("mailto:someone@example.com"),
                Some("Mail")
            ),
            ExternalOpenPrompt {
                message: "example.com wants to open Mail".to_owned(),
                detail: "mailto:someone@example.com".to_owned(),
                open_label: "Open Mail".to_owned(),
                always_label: Some("Always allow example.com to open mailto links".to_owned()),
            }
        );
    }

    #[test]
    fn the_prompt_names_the_scheme_when_the_app_is_unknown() {
        let prompt = external_open_prompt(
            Some(&url("file:///tmp/a.html")),
            &url("tel:+15555550123"),
            None,
        );
        assert_eq!(
            prompt.message,
            "This page wants to open a tel link in another app"
        );
        assert_eq!(prompt.open_label, "Open");
        assert_eq!(prompt.always_label, None);
    }
}
