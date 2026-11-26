// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Per-tab counters and the list of what was blocked on the tab's page (MH14).

use std::collections::HashMap;

use crate::level::Category;

/// How many distinct (host, category) pairs a tab keeps for the panel's list.
pub const MAX_BLOCKED_HOSTS: usize = 200;

/// The longest host kept, in bytes (a DNS name).
const MAX_HOST_BYTES: usize = 253;

/// How many tabs are counted at once; the least recently updated is evicted.
const MAX_TABS: usize = 512;

/// A tab, as the caller names it (any stable number).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabKey(pub u64);

/// What kind of resource a blocked request was for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    /// A page or frame.
    Document,
    /// A script.
    Script,
    /// An image.
    Image,
    /// A style sheet.
    Stylesheet,
    /// A font.
    Font,
    /// Audio or video.
    Media,
    /// `fetch` or `XMLHttpRequest`.
    Xhr,
    /// A WebSocket.
    WebSocket,
    /// Anything else.
    Other,
}

/// One request the protections blocked on a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockEvent {
    /// The tab the request came from.
    pub tab: TabKey,
    /// The category that blocked it.
    pub category: Category,
    /// The host the request went to.
    pub host: String,
    /// What the request was for.
    pub kind: ResourceKind,
}

/// What was blocked on one tab's current page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabCounters {
    /// Every block on the page.
    pub total: u64,
    /// Blocks per category, in [`Category::ALL`] order.
    pub by_category: [u64; Category::COUNT],
    hosts: Vec<HostCount>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HostCount {
    host: String,
    category: Category,
    count: u64,
}

/// One row of a tab's blocked-host list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockedHost<'a> {
    /// The host, lower-cased.
    pub host: &'a str,
    /// The category that blocked it.
    pub category: Category,
    /// How many requests to it were blocked.
    pub count: u64,
}

impl TabCounters {
    /// The blocked hosts, in the order they were first seen. At most [`MAX_BLOCKED_HOSTS`] rows.
    pub fn hosts(&self) -> impl Iterator<Item = BlockedHost<'_>> {
        self.hosts.iter().map(|h| BlockedHost {
            host: &h.host,
            category: h.category,
            count: h.count,
        })
    }

    fn add(&mut self, host: String, category: Category) {
        self.total = self.total.saturating_add(1);
        let slot = &mut self.by_category[category.index()];
        *slot = slot.saturating_add(1);
        let room = self.hosts.len() < MAX_BLOCKED_HOSTS;
        match self
            .hosts
            .iter_mut()
            .find(|h| h.category == category && h.host == host)
        {
            Some(existing) => existing.count = existing.count.saturating_add(1),
            None if room => self.hosts.push(HostCount {
                host,
                category,
                count: 1,
            }),
            None => {}
        }
    }
}

#[derive(Debug)]
struct Slot {
    counters: TabCounters,
    touched: u64,
}

/// The blocks counted per tab. Each tab's counts are its own; they reset when its page changes.
#[derive(Debug, Default)]
pub struct TabProtection {
    tabs: HashMap<TabKey, Slot>,
    seq: u64,
}

impl TabProtection {
    /// Counts one blocked request on its tab.
    pub fn record(&mut self, e: &BlockEvent) {
        self.seq += 1;
        if !self.tabs.contains_key(&e.tab) && self.tabs.len() >= MAX_TABS {
            self.evict_least_recent();
        }
        let slot = self.tabs.entry(e.tab).or_insert_with(|| Slot {
            counters: TabCounters::default(),
            touched: 0,
        });
        slot.touched = self.seq;
        slot.counters.add(normal_host(&e.host), e.category);
    }

    /// The counts of one tab, if it has any.
    pub fn counters(&self, t: TabKey) -> Option<&TabCounters> {
        self.tabs.get(&t).map(|slot| &slot.counters)
    }

    /// A new document loaded in the tab's main frame: its counts start again. A same-document navigation does not
    /// call this.
    pub fn document_changed(&mut self, t: TabKey) {
        if let Some(slot) = self.tabs.get_mut(&t) {
            slot.counters = TabCounters::default();
        }
    }

    /// The tab is gone: its counts are dropped.
    pub fn close(&mut self, t: TabKey) {
        self.tabs.remove(&t);
    }

    fn evict_least_recent(&mut self) {
        if let Some(tab) = self
            .tabs
            .iter()
            .min_by_key(|(_, slot)| slot.touched)
            .map(|(tab, _)| *tab)
        {
            self.tabs.remove(&tab);
        }
    }
}

fn normal_host(host: &str) -> String {
    let lower = host.to_ascii_lowercase();
    let mut end = lower.len().min(MAX_HOST_BYTES);
    while !lower.is_char_boundary(end) {
        end -= 1;
    }
    lower[..end].to_owned()
}
