// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the pointer, the wheel and the keys mean in the viewer, as pure functions and small state
//! machines: click counts, drag against click, what a link does, how far a key scrolls. The
//! window's callbacks (`mod.rs`) feed them and carry out the answers.

use std::time::{Duration, Instant};

use mercury_pdf::types::LinkTarget;
use mercury_ui::PdfScrollKey;
use url::Url;

/// Two presses within this time and distance count as a double (or triple) click.
const MULTI_CLICK_TIME: Duration = Duration::from_millis(500);
const MULTI_CLICK_DISTANCE: f32 = 4.0;
/// A press that moves farther than this before the release is a drag, not a click.
const CLICK_SLOP: f32 = 4.0;

/// How far an arrow key scrolls, in logical pixels.
pub(super) const LINE: f32 = 40.0;

/// Presses, drags and releases of the primary button in the page area, from which it makes click
/// counts (1 caret, 2 word, 3 line, then round again) and tells a click from a drag. Positions
/// are the page area's own, so scrolling does not matter.
#[derive(Debug, Default)]
pub(super) struct PointerTracker {
    last_press: Option<(Instant, f32, f32)>,
    clicks: u8,
    pressed_at: Option<(f32, f32)>,
    dragged: bool,
}

impl PointerTracker {
    /// A press; returns the number of clicks in a row it makes.
    pub(super) fn down(&mut self, now: Instant, x: f32, y: f32) -> u8 {
        let continues = self.last_press.is_some_and(|(at, last_x, last_y)| {
            now.saturating_duration_since(at) <= MULTI_CLICK_TIME
                && (x - last_x).abs() <= MULTI_CLICK_DISTANCE
                && (y - last_y).abs() <= MULTI_CLICK_DISTANCE
        });
        self.clicks = if continues { self.clicks % 3 + 1 } else { 1 };
        self.last_press = Some((now, x, y));
        self.pressed_at = Some((x, y));
        self.dragged = false;
        self.clicks
    }

    /// The pointer moved. True when it moved with the button down, which is a drag from now on.
    pub(super) fn moved(&mut self, x: f32, y: f32) -> bool {
        let Some((start_x, start_y)) = self.pressed_at else {
            return false;
        };
        if (x - start_x).abs() > CLICK_SLOP || (y - start_y).abs() > CLICK_SLOP {
            self.dragged = true;
        }
        true
    }

    pub(super) fn is_pressed(&self) -> bool {
        self.pressed_at.is_some()
    }

    /// The release. `Some(position)` when the press and release made a plain single click (not a
    /// drag, not the second click of a double): where a link would be followed.
    pub(super) fn up(&mut self) -> Option<(f32, f32)> {
        let pressed_at = self.pressed_at.take()?;
        let click = !self.dragged && self.clicks == 1;
        self.dragged = false;
        click.then_some(pressed_at)
    }

    /// The press was taken away (the window lost the pointer).
    pub(super) fn cancel(&mut self) {
        self.pressed_at = None;
        self.dragged = false;
    }
}

/// What a click on a link does.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum LinkAction {
    /// Scroll to a place in the document: the page (counted from 0) and the y position on it.
    GoToPage { page: u32, y: Option<f32> },
    /// An ordinary navigation of the tab, as a click on a link in a web page.
    Navigate(Url),
    /// Anything else: nothing happens (`mailto:`, `javascript:`, `file:`, an address that does
    /// not parse, a page the document does not have).
    Ignore,
}

/// A link's target as the viewer follows it. Only `http` and `https` addresses leave the
/// document: a PDF must not be able to open a file, start a program or run a script.
pub(super) fn link_action(target: &LinkTarget, page_count: u32) -> LinkAction {
    match target {
        LinkTarget::Page { page, y } if *page < page_count => {
            LinkAction::GoToPage { page: *page, y: *y }
        }
        LinkTarget::Page { .. } => LinkAction::Ignore,
        LinkTarget::Uri(address) => match Url::parse(address.trim()) {
            Ok(url) if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() => {
                LinkAction::Navigate(url)
            }
            _ => LinkAction::Ignore,
        },
    }
}

/// How a scrolling key moves the pages.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ScrollMove {
    /// By this many logical pixels, right and down.
    By(f32, f32),
    Top,
    Bottom,
}

/// The move for a key, given the page area's height.
pub(super) fn scroll_move(key: PdfScrollKey, area_height: f32) -> ScrollMove {
    // A page key leaves a few lines of the old view on screen, as other viewers do.
    let page = (area_height - 2.0 * LINE).max(area_height * 0.5).max(LINE);
    match key {
        PdfScrollKey::LineDown => ScrollMove::By(0.0, LINE),
        PdfScrollKey::LineUp => ScrollMove::By(0.0, -LINE),
        PdfScrollKey::LineLeft => ScrollMove::By(-LINE, 0.0),
        PdfScrollKey::LineRight => ScrollMove::By(LINE, 0.0),
        PdfScrollKey::PageDown => ScrollMove::By(0.0, page),
        PdfScrollKey::PageUp => ScrollMove::By(0.0, -page),
        PdfScrollKey::Top => ScrollMove::Top,
        PdfScrollKey::Bottom => ScrollMove::Bottom,
    }
}

/// The zoom step nearest to the zoom a pinch asks for (a tie goes to the smaller step, so a pinch
/// that has barely begun does not move the zoom).
pub(super) fn nearest_zoom_step(percent: f32) -> u16 {
    if !percent.is_finite() {
        return talaria_core::DEFAULT_ZOOM_PERCENT;
    }
    talaria_core::ZOOM_STEPS
        .into_iter()
        .min_by(|a, b| {
            (f32::from(*a) - percent)
                .abs()
                .total_cmp(&(f32::from(*b) - percent).abs())
        })
        .unwrap_or(talaria_core::DEFAULT_ZOOM_PERCENT)
}

/// A wheel or trackpad delta as a scroll: Slint's deltas are the way the content moves, so the
/// view moves the other way.
pub(super) fn wheel_scroll(delta_x: f32, delta_y: f32) -> (f32, f32) {
    (-delta_x, -delta_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, milliseconds: u64) -> Instant {
        start + Duration::from_millis(milliseconds)
    }

    #[test]
    fn presses_in_a_row_count_up_to_three_and_start_again() {
        let start = Instant::now();
        let mut tracker = PointerTracker::default();
        assert_eq!(tracker.down(at(start, 0), 10.0, 10.0), 1);
        assert_eq!(tracker.down(at(start, 200), 11.0, 10.0), 2);
        assert_eq!(tracker.down(at(start, 400), 10.0, 12.0), 3);
        assert_eq!(tracker.down(at(start, 600), 10.0, 10.0), 1);
    }

    #[test]
    fn a_slow_or_distant_press_starts_a_new_count() {
        let start = Instant::now();
        let mut tracker = PointerTracker::default();
        assert_eq!(tracker.down(at(start, 0), 10.0, 10.0), 1);
        assert_eq!(tracker.down(at(start, 900), 10.0, 10.0), 1, "too slow");
        assert_eq!(tracker.down(at(start, 1000), 30.0, 10.0), 1, "too far");
        assert_eq!(tracker.down(at(start, 1100), 31.0, 10.0), 2);
    }

    #[test]
    fn a_press_and_release_in_place_is_a_click_but_a_drag_or_a_double_is_not() {
        let start = Instant::now();
        let mut tracker = PointerTracker::default();
        tracker.down(at(start, 0), 50.0, 60.0);
        assert!(tracker.is_pressed());
        assert!(tracker.moved(51.0, 61.0), "a small move is still a click");
        assert_eq!(tracker.up(), Some((50.0, 60.0)));
        assert!(!tracker.is_pressed());

        tracker.down(at(start, 2000), 50.0, 60.0);
        assert!(tracker.moved(80.0, 60.0));
        assert_eq!(tracker.up(), None, "a drag selects");

        tracker.down(at(start, 4000), 50.0, 60.0);
        tracker.up();
        tracker.down(at(start, 4100), 50.0, 60.0);
        assert_eq!(
            tracker.up(),
            None,
            "the second click of a double selects a word"
        );
    }

    #[test]
    fn moves_without_a_press_are_not_drags_and_a_cancel_forgets_the_press() {
        let start = Instant::now();
        let mut tracker = PointerTracker::default();
        assert!(!tracker.moved(5.0, 5.0));
        assert_eq!(tracker.up(), None);
        tracker.down(start, 5.0, 5.0);
        tracker.cancel();
        assert!(!tracker.is_pressed());
        assert_eq!(tracker.up(), None);
    }

    #[test]
    fn links_inside_the_document_scroll_and_unknown_pages_are_ignored() {
        assert_eq!(
            link_action(
                &LinkTarget::Page {
                    page: 2,
                    y: Some(100.0)
                },
                3
            ),
            LinkAction::GoToPage {
                page: 2,
                y: Some(100.0)
            }
        );
        assert_eq!(
            link_action(&LinkTarget::Page { page: 3, y: None }, 3),
            LinkAction::Ignore
        );
    }

    #[test]
    fn only_http_and_https_links_leave_the_document() {
        let navigate = |address: &str| link_action(&LinkTarget::Uri(address.to_owned()), 1);
        assert_eq!(
            navigate("https://example.com/a?b=1"),
            LinkAction::Navigate(Url::parse("https://example.com/a?b=1").expect("a URL"))
        );
        assert_eq!(
            navigate("  http://example.com/  "),
            LinkAction::Navigate(Url::parse("http://example.com/").expect("a URL"))
        );
        for refused in [
            "mailto:someone@example.com",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://example.com/",
            "data:text/html,hi",
            "https://",
            "not an address",
            "",
        ] {
            assert_eq!(navigate(refused), LinkAction::Ignore, "{refused:?}");
        }
    }

    #[test]
    fn keys_scroll_by_lines_and_pages() {
        assert_eq!(
            scroll_move(PdfScrollKey::LineDown, 600.0),
            ScrollMove::By(0.0, LINE)
        );
        assert_eq!(
            scroll_move(PdfScrollKey::LineLeft, 600.0),
            ScrollMove::By(-LINE, 0.0)
        );
        assert_eq!(
            scroll_move(PdfScrollKey::PageDown, 600.0),
            ScrollMove::By(0.0, 520.0)
        );
        assert_eq!(
            scroll_move(PdfScrollKey::PageUp, 600.0),
            ScrollMove::By(0.0, -520.0)
        );
        // A very short area still moves.
        assert_eq!(
            scroll_move(PdfScrollKey::PageDown, 60.0),
            ScrollMove::By(0.0, 40.0)
        );
        assert_eq!(scroll_move(PdfScrollKey::Top, 600.0), ScrollMove::Top);
        assert_eq!(scroll_move(PdfScrollKey::Bottom, 600.0), ScrollMove::Bottom);
    }

    #[test]
    fn a_pinch_lands_on_the_nearest_zoom_step() {
        assert_eq!(nearest_zoom_step(100.0), 100);
        assert_eq!(nearest_zoom_step(104.0), 100);
        assert_eq!(
            nearest_zoom_step(105.0),
            100,
            "a tie goes to the smaller step"
        );
        assert_eq!(nearest_zoom_step(106.0), 110);
        assert_eq!(nearest_zoom_step(1.0), 30);
        assert_eq!(nearest_zoom_step(9000.0), 500);
        assert_eq!(nearest_zoom_step(f32::NAN), 100);
    }

    #[test]
    fn the_wheel_moves_the_view_against_the_content() {
        assert_eq!(wheel_scroll(0.0, 12.0), (0.0, -12.0));
        assert_eq!(wheel_scroll(-5.0, 0.0), (5.0, 0.0));
    }
}
