// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The net lane's landing for the engine's `NetMsg` (`third_party/servo-embedder-traits/mercury/net.rs`).
//!
//! [`handle`] replaces the `Net` arm of `mercury_messages.rs`. The only message today is
//! `MixedContentBlocked` (EC17): the network thread refused a request of a secure page because it was not
//! potentially trustworthy and the user has not allowed mixed content on the page's site. Mercury counts
//! them per tab, so that a later indicator (the padlock, a "blocked content" button) can show how many, and
//! logs one line for each with the **host** of the request and nothing else (a URL has paths and query
//! strings that must not reach a log).
//!
//! * The count is for the page the tab shows: [`main_frame_load_started`] resets it at the tab's next
//!   main-frame load start (`LoadStatus::Started` in `runtime.rs`), so a reload or a new page begins at 0.
//! * It is bounded: at most [`MAX_TABS`] tabs are remembered (the least recently touched goes first) and
//!   a count stops at `u32::MAX`.
//! * `ServoRuntime::mixed_content_blocked(tab)` reads it. There is no interface for it yet.
//!
//! The state is per thread: the engine's delegates and the browser run on one thread (the delegates are
//! `Rc`), and a test that runs on its own thread has a state of its own.

use std::cell::RefCell;
use std::collections::HashMap;

use servo::NetMsg;
use talaria_core::TabId;

/// How many tabs' counts are kept.
pub(crate) const MAX_TABS: usize = 256;

#[derive(Clone, Copy)]
struct Entry {
    count: u32,
    /// Counts up with every touch; the smallest value is the least recently touched.
    touched: u64,
}

#[derive(Default)]
struct MixedContentCounts {
    tabs: HashMap<TabId, Entry>,
    ticks: u64,
}

impl MixedContentCounts {
    fn touch(&mut self) -> u64 {
        self.ticks = self.ticks.wrapping_add(1);
        self.ticks
    }

    fn add(&mut self, tab: TabId) {
        if !self.tabs.contains_key(&tab) && self.tabs.len() >= MAX_TABS {
            let oldest = self
                .tabs
                .iter()
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(tab, _)| *tab);
            if let Some(oldest) = oldest {
                self.tabs.remove(&oldest);
            }
        }
        let touched = self.touch();
        let entry = self.tabs.entry(tab).or_insert(Entry { count: 0, touched });
        entry.count = entry.count.saturating_add(1);
        entry.touched = touched;
    }

    fn count(&self, tab: TabId) -> u32 {
        self.tabs.get(&tab).map_or(0, |entry| entry.count)
    }

    fn reset(&mut self, tab: TabId) {
        self.tabs.remove(&tab);
    }
}

thread_local! {
    static COUNTS: RefCell<MixedContentCounts> = RefCell::default();
}

/// The net lane's engine messages. `tab` is the tab of the web view the message concerns, or `None` for a
/// message with no web view (a worker's request, say), which is logged but belongs to no count.
pub(crate) fn handle(tab: Option<TabId>, message: NetMsg) {
    match message {
        NetMsg::MixedContentBlocked { url, destination } => {
            let host = url.host_str().unwrap_or("(no host)");
            match tab {
                Some(tab) => {
                    COUNTS.with_borrow_mut(|counts| counts.add(tab));
                    log::info!(
                        "tab {}: blocked mixed content ({destination:?} from {host})",
                        tab.value()
                    );
                }
                None => {
                    log::info!("blocked mixed content ({destination:?} from {host}) for no tab");
                }
            }
        }
    }
}

/// How many requests of the page `tab` shows were blocked as mixed content.
pub(crate) fn mixed_content_blocked(tab: TabId) -> u32 {
    COUNTS.with_borrow(|counts| counts.count(tab))
}

/// A main frame of `tab` starts loading: the old page's count ends.
pub(crate) fn main_frame_load_started(tab: TabId) {
    COUNTS.with_borrow_mut(|counts| counts.reset(tab));
}

#[cfg(test)]
mod tests {
    use content_security_policy::Destination;
    use servo::ServoUrl;

    use super::*;

    fn blocked(url: &str, destination: Destination) -> NetMsg {
        NetMsg::MixedContentBlocked {
            url: ServoUrl::parse(url).expect("a URL"),
            destination,
        }
    }

    #[test]
    fn a_blocked_request_is_counted_for_its_tab() {
        let tab = TabId::new(1);
        let other = TabId::new(2);
        assert_eq!(mixed_content_blocked(tab), 0);

        handle(
            Some(tab),
            blocked("http://insecure.example/a.js", Destination::Script),
        );
        handle(
            Some(tab),
            blocked("http://insecure.example/b.png", Destination::Image),
        );
        handle(
            Some(other),
            blocked("http://insecure.example/c.css", Destination::Style),
        );

        assert_eq!(mixed_content_blocked(tab), 2);
        assert_eq!(mixed_content_blocked(other), 1);
        assert_eq!(mixed_content_blocked(TabId::new(3)), 0);
    }

    #[test]
    fn a_message_without_a_tab_is_handled_and_counted_nowhere() {
        handle(
            None,
            blocked("http://insecure.example/x.js", Destination::Script),
        );
        assert_eq!(mixed_content_blocked(TabId::new(1)), 0);
    }

    #[test]
    fn the_next_main_frame_load_starts_a_new_count() {
        let tab = TabId::new(4);
        handle(
            Some(tab),
            blocked("http://insecure.example/a.js", Destination::Script),
        );
        assert_eq!(mixed_content_blocked(tab), 1);

        main_frame_load_started(tab);
        assert_eq!(mixed_content_blocked(tab), 0);

        // Messages of the new page count again; another tab's page was not touched.
        handle(
            Some(tab),
            blocked("http://insecure.example/a.js", Destination::Script),
        );
        handle(
            Some(TabId::new(5)),
            blocked("http://insecure.example/a.js", Destination::Script),
        );
        main_frame_load_started(tab);
        assert_eq!(mixed_content_blocked(tab), 0);
        assert_eq!(mixed_content_blocked(TabId::new(5)), 1);
    }

    #[test]
    fn a_reset_for_a_tab_with_no_count_does_nothing() {
        main_frame_load_started(TabId::new(77));
        assert_eq!(mixed_content_blocked(TabId::new(77)), 0);
    }

    #[test]
    fn the_counts_are_bounded_and_the_least_recently_touched_tab_goes_first() {
        let mut counts = MixedContentCounts::default();
        for number in 0..MAX_TABS as u64 {
            counts.add(TabId::new(number));
        }
        assert_eq!(counts.tabs.len(), MAX_TABS);

        // Tab 0 is touched again, so tab 1 is now the least recently touched.
        counts.add(TabId::new(0));
        counts.add(TabId::new(MAX_TABS as u64));
        assert_eq!(counts.tabs.len(), MAX_TABS);
        assert_eq!(counts.count(TabId::new(0)), 2, "tab 0 kept its count");
        assert_eq!(counts.count(TabId::new(1)), 0, "tab 1 was dropped");
        assert_eq!(counts.count(TabId::new(MAX_TABS as u64)), 1);
    }

    #[test]
    fn a_count_stops_at_the_largest_value() {
        let mut counts = MixedContentCounts::default();
        let tab = TabId::new(9);
        counts.tabs.insert(
            tab,
            Entry {
                count: u32::MAX - 1,
                touched: 0,
            },
        );
        counts.add(tab);
        counts.add(tab);
        counts.add(tab);
        assert_eq!(counts.count(tab), u32::MAX);
    }

    #[test]
    fn the_log_line_names_the_host_and_never_the_path_or_query() {
        // `handle` builds its line from `host_str()` alone; this pins what that gives for a URL whose path
        // and query hold something that must stay out of a log.
        let url =
            ServoUrl::parse("http://user:secret@insecure.example:8080/path/token?session=abc#frag")
                .expect("a URL");
        assert_eq!(url.host_str(), Some("insecure.example"));
    }
}
