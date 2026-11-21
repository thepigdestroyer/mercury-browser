// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The engine's guard for images per site: [`SiteImagesHandle`] holds the sites whose images are
//! off, and [`refuse_blocked_image`] answers an image load of such a site the way the content
//! blocker answers a blocked image (see `adblocker.rs`): the load ends in a network error and
//! the page goes on loading.
//!
//! `runtime.rs` calls `refuse_blocked_image` from `BrowserDelegate::load_web_resource`, with the
//! tab's own address as `page` (the top-level page, never a frame's: a third-party frame on a
//! blocked site loses its images too, and a frame of another site on an allowed page keeps
//! them). The set itself is `talaria_core::SiteImages`; the desktop owns the file and pushes the
//! set here with [`SiteImagesHandle::replace`].
//!
//! What the guard cannot tell apart: `Destination::Image` is also the destination of a page's
//! `<link rel=icon>`, a video poster, CSS images and notification icons, so a site with images
//! off loses its tab icon too (brief decision O9).

use std::cell::RefCell;
use std::rc::Rc;

use content_security_policy::Destination;
use servo::{WebResourceLoad, WebResourceResponse};
use talaria_core::SiteImages;
use url::Url;

/// The sites whose images are refused, shared between the browser (which replaces the set when
/// the user changes a site) and the engine's delegates (which ask it at every load). Cloning
/// shares the set. UI thread only, like `EngineSwitches`: every load decision and every
/// replacement happens on it, so a `RefCell` is enough.
#[derive(Clone, Debug, Default)]
pub struct SiteImagesHandle {
    images: Rc<RefCell<SiteImages>>,
}

impl SiteImagesHandle {
    /// Replaces the set of sites whose images are refused, for every load from now on. Images
    /// already on a page stay until the page is loaded again.
    pub fn replace(&self, images: SiteImages) {
        self.images.replace(images);
    }

    /// Whether a load with this `destination`, made by a tab showing `page`, is refused. One
    /// comparison for anything that is not an image; for an image, one more while no site is
    /// blocked.
    pub(crate) fn refuses(&self, destination: Destination, page: Option<&Url>) -> bool {
        matches!(destination, Destination::Image)
            && self.images.borrow().blocks_image_load(page, true)
    }
}

/// Answers an image load of a site that has images off with a network error and returns `None`;
/// any other load comes back as `Some` for the next step. `page` is the address of the tab the
/// load belongs to (`web_view.url()`).
pub(crate) fn refuse_blocked_image(
    handle: &SiteImagesHandle,
    page: Option<Url>,
    load: WebResourceLoad,
) -> Option<WebResourceLoad> {
    if !handle.refuses(load.request().destination, page.as_ref()) {
        return Some(load);
    }
    let refused = load.request().url.as_url().clone();
    log::trace!("images off for this site: refused an image load");
    load.intercept(WebResourceResponse::new(refused)).cancel();
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a valid URL")
    }

    fn handle_with(hosts: &[&str]) -> SiteImagesHandle {
        let handle = SiteImagesHandle::default();
        handle.replace(SiteImages::from_hosts(
            hosts.iter().map(|host| (*host).to_owned()),
        ));
        handle
    }

    #[test]
    fn only_an_image_of_a_blocked_site_is_refused() {
        let handle = handle_with(&["example.com"]);
        let page = url("https://example.com/article");

        assert!(handle.refuses(Destination::Image, Some(&page)));
        for other in [
            Destination::Document,
            Destination::IFrame,
            Destination::Frame,
            Destination::Script,
            Destination::Style,
            Destination::Font,
            Destination::Video,
            Destination::Audio,
            Destination::Json,
            Destination::None,
        ] {
            assert!(!handle.refuses(other, Some(&page)), "{other:?}");
        }
        assert!(!handle.refuses(Destination::Image, None));
        assert!(!handle.refuses(Destination::Image, Some(&url("https://example.org/"))));
        assert!(!handle.refuses(Destination::Image, Some(&url("https://cdn.example.com/")),));
        assert!(!handle.refuses(Destination::Image, Some(&url("about:blank"))));
    }

    #[test]
    fn an_empty_handle_refuses_nothing() {
        let handle = SiteImagesHandle::default();
        assert!(!handle.refuses(Destination::Image, Some(&url("https://example.com/"))));
    }

    #[test]
    fn replacing_the_set_changes_the_answer_for_every_clone() {
        let handle = SiteImagesHandle::default();
        let held_by_a_tab = handle.clone();
        let page = url("https://example.com/");
        assert!(!held_by_a_tab.refuses(Destination::Image, Some(&page)));

        handle.replace(SiteImages::from_hosts(["example.com".to_owned()]));
        assert!(held_by_a_tab.refuses(Destination::Image, Some(&page)));

        handle.replace(SiteImages::default());
        assert!(!held_by_a_tab.refuses(Destination::Image, Some(&page)));
    }
}
