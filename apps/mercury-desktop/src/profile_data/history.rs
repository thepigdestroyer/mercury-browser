//! Browsing history: recording what the controller decided were visits.

use mercury_store::{
    ClearRange, HistoryCursor, HistoryEntry, MAX_SUGGESTIONS, Suggestion, VisitId, VisitTransition,
};
use talaria_core::{BrowserController, NavigationOrigin, VisitKey, VisitUpdate};

use super::{
    Area, Message, ProfileData, ProfileDataError, RECENT_VISITS, handle, now_ms, write_private,
};

/// Hands the controller's history updates to the service, stamped with the current time. Call
/// after applying engine events; with no service the updates are dropped.
pub(crate) fn record_visits(controller: &mut BrowserController) {
    let updates = controller.take_visit_updates();
    if updates.is_empty() {
        return;
    }
    if let Some(data) = handle() {
        data.record_updates(updates, now_ms());
    }
}

/// The stored transition for a navigation's origin. Back and forward count as following a link
/// to the page again: the store has no transition of its own for them. `Restored` pages are
/// never recorded; should one reach here, it counts as a link.
pub(super) fn transition(origin: NavigationOrigin) -> VisitTransition {
    match origin {
        NavigationOrigin::Typed => VisitTransition::Typed,
        NavigationOrigin::Link | NavigationOrigin::History | NavigationOrigin::Restored => {
            VisitTransition::Link
        }
        NavigationOrigin::Bookmark => VisitTransition::Bookmark,
        NavigationOrigin::Reload => VisitTransition::Reload,
    }
}

impl ProfileData {
    /// Records `updates` in order, as having happened at `at_ms`. No reply.
    pub(crate) fn record_updates(&self, updates: Vec<VisitUpdate>, at_ms: i64) {
        for update in updates {
            match update {
                VisitUpdate::Visit { key, url, origin } => {
                    self.record_visit(key, url, transition(origin), at_ms)
                }
                VisitUpdate::Title { url, title } => self.set_page_title(url, title),
                VisitUpdate::Forget { key } => self.forget_visit(key),
            }
        }
    }

    /// Records a visit to `url` at `at_ms`; `key` lets [`forget_visit`](Self::forget_visit)
    /// take it back. No reply.
    fn record_visit(&self, key: VisitKey, url: String, transition: VisitTransition, at_ms: i64) {
        self.submit(
            Some(Area::History),
            "recording a visit",
            move |database, _now| {
                let id = database.store.record_visit(&url, None, transition, at_ms)?;
                if database.recent_visits.len() == RECENT_VISITS {
                    database.recent_visits.pop_front();
                }
                database.recent_visits.push_back((key, id));
                Ok(())
            },
        );
    }

    /// Removes a visit recorded with `key` moments ago (an error page, a replaced entry), and
    /// its page if that has no other visit and no bookmark. Older visits are kept. No reply.
    fn forget_visit(&self, key: VisitKey) {
        self.submit(
            Some(Area::History),
            "forgetting a visit",
            move |database, now| {
                let index = database.recent_visits.iter().position(|(k, _)| *k == key);
                let Some((_, id)) = index.and_then(|index| database.recent_visits.remove(index))
                else {
                    log::debug!("visit {} is no longer recent; kept", key.value());
                    return Ok(());
                };
                database.store.delete_visit(id, now)?;
                Ok(())
            },
        );
    }

    /// Sets the title of the page at `url`, if it is known; debounced per URL. No reply.
    pub(super) fn set_page_title(&self, url: String, title: String) {
        self.send(Message::Title { url, title });
    }

    /// Address-bar suggestions for `input`, best first: at most `limit`, itself at most
    /// [`MAX_SUGGESTIONS`]. Ranked as of the service's current time.
    pub(crate) fn suggest(
        &self,
        input: String,
        limit: u32,
        reply: impl FnOnce(Result<Vec<Suggestion>, ProfileDataError>) + 'static,
    ) {
        let limit = limit.min(MAX_SUGGESTIONS);
        self.request(
            None,
            move |db, now| db.store().suggest(&input, limit, now),
            reply,
        );
    }

    /// Visits, newest first: `limit` of them after `before` (from [`HistoryEntry::cursor`]),
    /// matching `query` (words of the URL or title) if given.
    pub(crate) fn history(
        &self,
        query: Option<String>,
        before: Option<HistoryCursor>,
        limit: u32,
        reply: impl FnOnce(Result<Vec<HistoryEntry>, ProfileDataError>) + 'static,
    ) {
        self.request(
            None,
            move |db, _| db.store().history(query.as_deref(), before, limit),
            reply,
        );
    }

    /// Deletes one visit; `false` if it did not exist. The deleted rows are purged from the
    /// files shortly after (when the service is idle).
    pub(crate) fn delete_visit(
        &self,
        visit: VisitId,
        reply: impl FnOnce(Result<bool, ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::History),
            move |db, now| {
                db.purge_pending = true;
                db.store().delete_visit(visit, now)
            },
            reply,
        );
    }

    /// Deletes every visit of the site `host` (any scheme, port, and a leading `www.`) and
    /// replies with how many; the deleted rows are purged from the files at once.
    pub(crate) fn delete_site(
        &self,
        host: String,
        reply: impl FnOnce(Result<u64, ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::History),
            move |db, now| {
                let removed = db.store().delete_site(&host, now)?;
                db.store().purge_deleted()?;
                Ok(removed)
            },
            reply,
        );
    }

    /// Deletes the visits in `range` and replies with how many; the deleted rows are purged
    /// from the files at once.
    pub(crate) fn clear_history(
        &self,
        range: ClearRange,
        reply: impl FnOnce(Result<u64, ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::History),
            move |db, now| {
                let removed = db.store().clear_history(range, now)?;
                db.store().purge_deleted()?;
                Ok(removed)
            },
            reply,
        );
    }

    /// "Clear history on exit": writes the pending-clear marker first, then queues clearing
    /// all history, which removes the marker when it has finished. If the quit deadline cuts
    /// the clear short, the next start finds the marker and clears the history before
    /// anything else. Call it before [`shut_down`](super::shut_down).
    pub(crate) fn clear_history_on_exit(&self) {
        let Some(marker) = self.shared.pending_clear.clone() else {
            return;
        };
        if let Err(error) = write_private(&marker, b"history\n") {
            log::error!("could not write {}: {error}", marker.display());
        }
        self.submit(
            Some(Area::History),
            "clearing the history at exit",
            move |db, now| {
                db.store().clear_history(ClearRange::All, now)?;
                db.store().purge_deleted()?;
                if let Err(error) = std::fs::remove_file(&marker) {
                    log::warn!("could not remove {}: {error}", marker.display());
                }
                Ok(())
            },
        );
    }
}
