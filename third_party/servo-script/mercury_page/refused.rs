/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the page lane's refused navigation. Stub until the lane's tasks fill it.

use embedder_traits::{EmbedderMsg, LoadStatus};
use js::context::JSContext;
use log::warn;
use servo_base::id::PipelineId;
use servo_url::ServoUrl;

use crate::event_loop::script_thread::ScriptThread;

/// Whether `url` has a scheme that does not fetch: anything except `http`, `https`, `file`, `data`,
/// `blob`, `about`, `javascript`, `resource`, `urlinfo` and `servo`.
pub(crate) fn is_non_fetch_scheme(url: &ServoUrl) -> bool {
    !matches!(
        url.scheme(),
        "http"
            | "https"
            | "file"
            | "data"
            | "blob"
            | "about"
            | "javascript"
            | "resource"
            | "urlinfo"
            | "servo"
    )
}

/// The navigation of `pipeline_id` was refused: a top-level page goes back to `Stopped`, so it does
/// not stay "loading" after a link to a scheme the browser does not fetch.
pub(crate) fn handle(_cx: &mut JSContext, pipeline_id: PipelineId) {
    let Some(window) = ScriptThread::find_window(pipeline_id) else {
        warn!("page lane: refused navigation for a pipeline that is gone");
        return;
    };
    if window.is_top_level() {
        window.send_to_embedder(EmbedderMsg::NotifyLoadStatusChanged(
            window.webview_id(),
            LoadStatus::Stopped,
        ));
    }
}

#[cfg(test)]
mod tests {
    use servo_url::ServoUrl;

    use super::is_non_fetch_scheme;

    #[test]
    fn non_fetch_schemes() {
        for url in [
            "mailto:someone@example.com",
            "tel:+15555550100",
            "x-app:open",
        ] {
            let url = ServoUrl::parse(url).unwrap();
            assert!(is_non_fetch_scheme(&url), "{url} does not fetch");
        }
    }

    #[test]
    fn fetching_schemes() {
        for url in [
            "https://example.com/",
            "http://example.com/",
            "file:///tmp/page.html",
            "data:text/html,hello",
            "blob:https://example.com/5f0c",
            "about:blank",
            "javascript:void(0)",
            "resource://servo/index.html",
            "urlinfo:about",
            "servo:devtools",
        ] {
            let url = ServoUrl::parse(url).unwrap();
            assert!(!is_non_fetch_scheme(&url), "{url} fetches");
        }
    }
}
