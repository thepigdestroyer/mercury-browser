//! What a page may navigate to and load, as the engine side of the openurl lane: page-initiated
//! navigations (`request_navigation`) are judged by `talaria_core::navigation_verdict`, and
//! `file:` frames and resources of pages that are not local files are refused in
//! `load_web_resource`, since a frame's `src` never reaches `request_navigation` (Servo sends it
//! as `ScriptLoadedURLInIFrame`).

use http::StatusCode;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use log::info;
use servo::{NavigationRequest, WebResourceLoad, WebResourceResponse};
use talaria_core::{
    BrowserEvent, ExternalLinkRequest, NavigationVerdict, TabId, navigation_verdict,
    refuses_local_file_load,
};
use url::Url;

use crate::page_requests::{new_page_request_id, page_request_clock};

/// Answers the page's navigation request at once. Returns the event to send when the URL
/// belongs to another app.
pub(crate) fn judge_navigation(
    tab_id: TabId,
    page: Option<Url>,
    request: NavigationRequest,
) -> Option<BrowserEvent> {
    let page_name = || {
        page.as_ref()
            .map_or("a page that has not loaded", Url::as_str)
            .to_owned()
    };
    match navigation_verdict(page.as_ref(), &request.url) {
        NavigationVerdict::Allow => {
            request.allow();
            None
        }
        NavigationVerdict::DenyLocalFile => {
            info!(
                "tab {}: refused a navigation from {} to the local file {}",
                tab_id.value(),
                page_name(),
                request.url
            );
            request.deny();
            None
        }
        NavigationVerdict::DenyData => {
            info!(
                "tab {}: refused a navigation from {} to a data: URL",
                tab_id.value(),
                page_name()
            );
            request.deny();
            None
        }
        NavigationVerdict::DenyScheme => {
            info!(
                "tab {}: refused a navigation from {} to a {}: URL; pages may not open those",
                tab_id.value(),
                page_name(),
                request.url.scheme()
            );
            request.deny();
            None
        }
        NavigationVerdict::OpenExternally => {
            info!(
                "tab {}: the engine does not load {}: links; the browser decides whether to offer another app",
                tab_id.value(),
                request.url.scheme()
            );
            let url = request.url.to_string();
            request.deny();
            Some(BrowserEvent::ExternalLinkRequested(ExternalLinkRequest {
                id: new_page_request_id(),
                url,
                page_url: page.map(String::from),
                received_at: page_request_clock(),
            }))
        }
    }
}

/// Refuses `load` if it fetches a local file into a frame or as a resource of a page that is not
/// one (see `refuses_local_file_load`); otherwise gives it back. `page` is the tab's top-level
/// page; `None` for loads without a web view, which are refused too.
pub(crate) fn refuse_local_file_load(
    tab_id: Option<TabId>,
    page: Option<Url>,
    load: WebResourceLoad,
) -> Option<WebResourceLoad> {
    let request = load.request();
    if !refuses_local_file_load(
        page.as_ref(),
        request.url.as_url(),
        request.is_for_main_frame,
    ) {
        return Some(load);
    }
    let url = request.url.as_url().clone();
    info!(
        "{}: refused loading the local file {url} into {}",
        tab_id.map_or_else(|| "no tab".to_owned(), |tab| format!("tab {}", tab.value())),
        page.as_ref()
            .map_or("a page that has not loaded", Url::as_str)
    );
    answer_forbidden(load, url);
    None
}

/// Answers a refused load with an empty `403 Forbidden`, the same whether the resource exists or
/// not: a page's `fetch()` rejects or reports `ok: false`, and a frame gets an empty document, so
/// its parent's `load` event still fires. `cancel()` would leave a frame without a document, and
/// its parent would never finish loading (tried).
pub(crate) fn answer_forbidden(load: WebResourceLoad, url: Url) {
    load.intercept(forbidden_response(url)).finish();
}

fn forbidden_response(url: Url) -> WebResourceResponse {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    WebResourceResponse::new(url)
        .status_code(StatusCode::FORBIDDEN)
        .status_message(b"Forbidden".to_vec())
        .headers(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refused_loads_are_forbidden_plain_text() {
        let response = forbidden_response(Url::parse("file:///etc/passwd").expect("valid"));

        assert_eq!(response.status_code, StatusCode::FORBIDDEN);
        assert_eq!(response.status_message, b"Forbidden");
        assert_eq!(
            response.headers.get(CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/plain"))
        );
    }
}
