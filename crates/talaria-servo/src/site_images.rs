// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The engine's guard for images per site: [`SiteImagesHandle`] holds the sites whose images are
//! off, and [`refuse_blocked_image`] answers an image load of such a site the way the content
//! blocker answers a blocked image (see `adblocker.rs`): the load ends in a network error and
//! the page goes on loading.
//!
//! `runtime.rs` calls `refuse_blocked_image` from `BrowserDelegate::load_web_resource`, with the
//! tab's top-level page as `page` (never a frame's: a third-party frame on a blocked site loses
//! its images too, and a frame of another site on an allowed page keeps them). While the tab
//! navigates, that page is the last main-frame document load, not `web_view.url()` (see
//! [`NavigatingPage`]). Loads that name no web view (`BrowserServoDelegate::load_web_resource`:
//! workers, say) are not judged: they belong to no tab's page. The set itself is `talaria_core::SiteImages`; the desktop owns the file and pushes the
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

/// The page a tab's loads belong to while it navigates: the address of the last main-frame
/// document load the delegate saw, until the engine reports the new address.
///
/// `WebView::url()` is the back-forward list entry, and it changes only when the new document's
/// pipeline is activated. An image the new document asks for before then (the parser's prefetch
/// starts as soon as the first bytes arrive) would be judged by the page being left, so a
/// navigation from `b.test` to a site with images off would load its first images. The delegate
/// records each main-frame document load here when it starts, so that the images of that
/// navigation are judged by the site it goes to; `settle` is called when the address has
/// changed and `web_view.url()` is right again.
///
/// A main-frame load that never commits (a download, a `204`, a cancelled load) leaves its
/// address here until the tab's address next changes; a late image of the page that stays is
/// then judged by the site that was refused. That errs on the side of the user's choice, and is
/// accepted.
#[derive(Debug, Default)]
pub(crate) struct NavigatingPage {
    last_document: RefCell<Option<Url>>,
}

impl NavigatingPage {
    /// A load starts: a main-frame document load becomes the page of what follows.
    pub(crate) fn note_load(&self, is_for_main_frame: bool, url: &Url) {
        if is_for_main_frame {
            self.last_document.replace(Some(url.clone()));
        }
    }

    /// The tab's address changed: `web_view.url()` says where the tab is.
    pub(crate) fn settle(&self) {
        self.last_document.replace(None);
    }

    /// The page to judge an image load by: the navigation in progress, else `current`.
    pub(crate) fn page(&self, current: Option<Url>) -> Option<Url> {
        self.last_document.borrow().clone().or(current)
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
    fn the_images_of_a_cross_site_navigation_are_judged_by_the_site_it_goes_to() {
        let handle = handle_with(&["a.test"]);
        let navigating = NavigatingPage::default();
        let on_b = url("https://b.test/start");
        let a = url("https://a.test/article");
        let refused = |current: &Url| {
            handle.refuses(
                Destination::Image,
                navigating.page(Some(current.clone())).as_ref(),
            )
        };

        // On `b.test`, whose images are on.
        assert!(!refused(&on_b));
        // The tab starts a navigation to `a.test`: `web_view.url()` still says `b.test`, but the
        // images the new document asks for first are `a.test`'s and are refused.
        navigating.note_load(true, &a);
        assert!(
            refused(&on_b),
            "an early image of the blocked site was let through"
        );
        // The address changes: `web_view.url()` is `a.test`.
        navigating.settle();
        assert!(refused(&a));
        // And back to `b.test`: the first images of that navigation are `b.test`'s.
        let back = url("https://b.test/back");
        navigating.note_load(true, &back);
        assert!(!refused(&a), "images of the allowed site were refused");
        navigating.settle();
        assert!(!refused(&back));
    }

    #[test]
    fn a_frames_document_does_not_change_the_page() {
        let handle = handle_with(&["a.test"]);
        let navigating = NavigatingPage::default();
        let on_b = url("https://b.test/");

        navigating.note_load(false, &url("https://a.test/frame"));

        assert_eq!(navigating.page(Some(on_b.clone())), Some(on_b.clone()));
        assert!(!handle.refuses(Destination::Image, navigating.page(Some(on_b)).as_ref()));
        // A tab that has no address yet (a pop-up) is judged by its first document.
        assert_eq!(navigating.page(None), None);
        navigating.note_load(true, &url("https://a.test/popup"));
        assert!(handle.refuses(Destination::Image, navigating.page(None).as_ref()));
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
