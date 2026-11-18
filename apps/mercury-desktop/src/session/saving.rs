// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! When and in what order the session is written, and when the recovery list may be changed:
//! the part of the `mercury-settings` contract (items 5, 8, 9, 11) that is about ordering. Pure:
//! the writer, the time and the deadlines are given, so every rule is tested with a fake.
//!
//! Every request goes through one queue that the writer carries out in order, so a write can
//! never land after one requested later. The UI thread never waits, except for the final save.

use std::collections::VecDeque;
use std::time::Instant;

use mercury_settings::{SaveThrottle, SaveToken, SessionSnapshot, SettingsError};

use super::model::only_new_tab_pages;

/// What the writer is asked to do. Carried out one at a time, in the order sent.
#[derive(Debug)]
pub(crate) enum Request {
    Save {
        snapshot: SessionSnapshot,
        exited_cleanly: bool,
    },
    /// Takes the restored session out of the recovery list (`SessionStore::remove_recovery`).
    RemoveRecovery(SessionSnapshot),
    /// Clears the recovery list (`SessionStore::resolve_recovery`).
    ResolveRecovery,
}

/// Carries out requests in order and reports one outcome per request, in the same order. The
/// real one is a thread (`saver::SaveWorker`).
pub(crate) trait SessionWriter {
    /// Queues `request`. False if it could not be queued.
    fn send(&mut self, request: Request) -> bool;
    /// The outcome of the oldest request not yet reported, if it has finished.
    fn poll(&mut self) -> Option<Result<(), SettingsError>>;
    /// The same, waiting until `deadline` at most. `None` if it did not finish by then.
    fn wait(&mut self, deadline: Instant) -> Option<Result<(), SettingsError>>;
}

/// What becomes of the recovery list once a restore is safely on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Resolve {
    /// This session was restored.
    Entry(SessionSnapshot),
    /// The user dismissed every session.
    All,
}

/// How the final save ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Written {
    Saved,
    /// The session file is from a newer version; nothing is written this run.
    Refused,
    /// It had not finished by the deadline. It may still land later.
    TimedOut,
    Failed,
}

/// A request sent and not yet reported.
enum Outstanding {
    Save {
        /// The throttle's permission, for a throttled save.
        token: Option<SaveToken>,
        /// The change count the snapshot covers.
        generation: u64,
        /// The snapshot, sanitized, holds a page other than a new-tab page.
        holds_pages: bool,
        is_final: bool,
    },
    Edit,
}

pub(crate) struct SaveState<W> {
    writer: W,
    throttle: SaveThrottle,
    /// Counts changes, like the throttle, so that a resolution waits for a snapshot that holds it.
    changes: u64,
    outstanding: VecDeque<Outstanding>,
    saving_refused: bool,
    refusal_to_tell: bool,
    /// Resolutions waiting for a snapshot of at least this change count to be written.
    pending: VecDeque<(Resolve, u64)>,
    /// A clean previous session that was not restored is held in the session file: until a
    /// session with a real page has been written, sessions of new-tab pages only are not
    /// written over it (except by the final save).
    holding_previous: bool,
    final_outcome: Option<Written>,
    last_error: Option<String>,
}

impl<W: SessionWriter> SaveState<W> {
    pub(crate) fn new(writer: W, min_interval_ms: i64) -> Self {
        Self {
            writer,
            throttle: SaveThrottle::new(min_interval_ms),
            changes: 0,
            outstanding: VecDeque::new(),
            saving_refused: false,
            refusal_to_tell: false,
            pending: VecDeque::new(),
            holding_previous: false,
            final_outcome: None,
            last_error: None,
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.throttle.mark_dirty();
        self.changes += 1;
    }

    pub(crate) fn set_holding_previous(&mut self, holding: bool) {
        self.holding_previous = holding;
    }

    /// True once, when saving has just been refused, so the user is told once.
    pub(crate) fn take_refusal_notice(&mut self) -> bool {
        std::mem::take(&mut self.refusal_to_tell)
    }

    /// From the timer: collects what the writer finished, and queues a throttled save when one
    /// is due and no save is outstanding. `snapshot` is only called when a save is due.
    pub(crate) fn tick(&mut self, now_ms: i64, snapshot: impl FnOnce() -> SessionSnapshot) {
        while !self.outstanding.is_empty()
            && let Some(outcome) = self.writer.poll()
        {
            self.take_outcome(outcome, now_ms);
        }
        let saving = self
            .outstanding
            .iter()
            .any(|request| matches!(request, Outstanding::Save { .. }));
        if self.saving_refused || saving {
            return;
        }
        let Some(token) = self.throttle.should_save(now_ms) else {
            return;
        };
        let snapshot = snapshot();
        let holds_pages = holds_pages(&snapshot);
        if self.holding_previous && !holds_pages {
            // Offered again after the interval; still dirty.
            drop(token);
            return;
        }
        self.send_save(snapshot, false, Some(token), holds_pages);
    }

    /// After tabs were restored: queues `snapshot` (which holds them) and returns. `resolve`
    /// is applied once a write holding the restore has succeeded — this one, or a later one if
    /// this one fails.
    pub(crate) fn persist_restore(&mut self, snapshot: SessionSnapshot, resolve: Option<Resolve>) {
        self.mark_dirty();
        if let Some(resolve) = resolve {
            self.pending.push_back((resolve, self.changes));
        }
        if self.saving_refused {
            self.warn_once(
                "the restored tabs cannot be saved this run (the session file is from a newer \
                 version); the session they came from stays offered at the next start",
            );
        } else {
            let holds_pages = holds_pages(&snapshot);
            self.send_save(snapshot, false, None, holds_pages);
        }
    }

    /// The user dismissed every recovered session: clears the list now, or after the restores
    /// still waiting to be written.
    pub(crate) fn dismiss_all(&mut self) {
        match self.pending.back().map(|(_, generation)| *generation) {
            Some(generation) => self.pending.push_back((Resolve::All, generation)),
            None => self.send_edit(Resolve::All),
        }
    }

    /// Queues the final save of an orderly quit, made even when nothing changed, with
    /// `exited_cleanly: true`, behind whatever is outstanding. Collect it with
    /// `collect_final`.
    pub(crate) fn begin_final(&mut self, snapshot: SessionSnapshot) {
        if self.saving_refused {
            self.final_outcome = Some(Written::Refused);
            return;
        }
        let holds_pages = holds_pages(&snapshot);
        if !self.send(Request::Save {
            snapshot,
            exited_cleanly: true,
        }) {
            self.final_outcome = Some(Written::Failed);
            return;
        }
        self.outstanding.push_back(Outstanding::Save {
            token: None,
            generation: self.changes,
            holds_pages,
            is_final: true,
        });
    }

    /// Waits until `deadline` at most for everything outstanding: the final save, and the
    /// recovery-list edits that a landed restore queued behind it. If the deadline passes before
    /// the final save lands, the run stays marked as running: a crash, the safe state.
    pub(crate) fn collect_final(&mut self, now_ms: i64, deadline: Instant) -> Written {
        while !self.outstanding.is_empty() {
            let Some(outcome) = self.writer.wait(deadline) else {
                if self.final_outcome.is_none() {
                    log::warn!("the session was not saved by the quit deadline");
                    return Written::TimedOut;
                }
                log::warn!("the list of sessions to recover was not updated by the quit deadline");
                break;
            };
            self.take_outcome(outcome, now_ms);
        }
        // Without `begin_final` there is no outcome.
        self.final_outcome.unwrap_or(Written::Failed)
    }

    /// `begin_final` and `collect_final` together. The application calls the two halves
    /// separately (`session::begin_final_save`, `session::collect_final_save`).
    #[cfg(test)]
    pub(crate) fn finish(
        &mut self,
        now_ms: i64,
        deadline: Instant,
        snapshot: SessionSnapshot,
    ) -> Written {
        self.begin_final(snapshot);
        self.collect_final(now_ms, deadline)
    }

    fn send_save(
        &mut self,
        snapshot: SessionSnapshot,
        exited_cleanly: bool,
        token: Option<SaveToken>,
        holds_pages: bool,
    ) {
        if self.send(Request::Save {
            snapshot,
            exited_cleanly,
        }) {
            self.outstanding.push_back(Outstanding::Save {
                token,
                generation: self.changes,
                holds_pages,
                is_final: false,
            });
        }
    }

    fn send_edit(&mut self, resolve: Resolve) {
        let request = match resolve {
            Resolve::Entry(snapshot) => Request::RemoveRecovery(snapshot),
            Resolve::All => Request::ResolveRecovery,
        };
        if self.send(request) {
            self.outstanding.push_back(Outstanding::Edit);
        }
    }

    fn send(&mut self, request: Request) -> bool {
        let sent = self.writer.send(request);
        if !sent {
            self.warn_once("the session saver is not running; the session was not saved");
        }
        sent
    }

    fn take_outcome(&mut self, outcome: Result<(), SettingsError>, now_ms: i64) {
        let Some(request) = self.outstanding.pop_front() else {
            return;
        };
        let Outstanding::Save {
            token,
            generation,
            holds_pages,
            is_final,
        } = request
        else {
            if let Err(error) = outcome {
                log::warn!("could not update the list of sessions to recover: {error}");
            }
            return;
        };
        match outcome {
            Ok(()) => {
                match token {
                    Some(token) => self.throttle.saved(token, now_ms),
                    // A write made without asking holds every change up to its generation.
                    None if self.changes == generation => self.throttle.flushed(now_ms),
                    None => {}
                }
                if holds_pages {
                    self.holding_previous = false;
                }
                self.last_error = None;
                if is_final {
                    self.final_outcome = Some(Written::Saved);
                }
                while let Some((_, pending)) = self.pending.front()
                    && *pending <= generation
                {
                    if let Some((resolve, _)) = self.pending.pop_front() {
                        self.send_edit(resolve);
                    }
                }
            }
            Err(SettingsError::NewerVersion { .. }) => {
                if !self.saving_refused {
                    log::warn!(
                        "the session file is from a newer version; tabs are not saved this run"
                    );
                    self.saving_refused = true;
                    self.refusal_to_tell = true;
                }
                if is_final {
                    self.final_outcome = Some(Written::Refused);
                }
            }
            Err(error) => {
                self.warn_once(&format!("could not save the session: {error}"));
                if is_final {
                    self.final_outcome = Some(Written::Failed);
                }
            }
        }
    }

    /// Logs a failure once until it changes or a write succeeds, so a full disk does not log
    /// every five seconds.
    fn warn_once(&mut self, message: &str) {
        if self.last_error.as_deref() != Some(message) {
            log::warn!("{message}");
            self.last_error = Some(message.to_owned());
        }
    }

    #[cfg(test)]
    fn writer(&self) -> &W {
        &self.writer
    }
}

/// Whether a snapshot, as it would be written (sanitized), holds a page other than a new-tab
/// page: a tab that sanitizing drops (`data:`, `javascript:`) does not count.
fn holds_pages(snapshot: &SessionSnapshot) -> bool {
    !only_new_tab_pages(&snapshot.clone().sanitized())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use mercury_settings::{TabSnapshot, WindowSnapshot};

    use super::*;

    const INTERVAL: i64 = 5_000;

    /// One request the fake writer received, in order.
    #[derive(Debug, PartialEq, Eq)]
    enum Call {
        Save { url: String, exited_cleanly: bool },
        Remove(String),
        ResolveAll,
    }

    /// A writer that finishes requests only when the test says, ending as the test says.
    #[derive(Default)]
    struct FakeWriter {
        calls: Vec<Call>,
        /// Requests received and not yet reported, oldest first; true for a save.
        queue: VecDeque<bool>,
        /// How many queued requests have finished, for `poll`.
        finished: usize,
        /// Whether `wait` sees nothing finish before the deadline.
        stalled: bool,
        /// How the next saves end; `Ok` when empty.
        outcomes: VecDeque<Result<(), SettingsError>>,
        stopped: bool,
    }

    impl FakeWriter {
        fn saves(&self) -> Vec<(&str, bool)> {
            self.calls
                .iter()
                .filter_map(|call| match call {
                    Call::Save {
                        url,
                        exited_cleanly,
                    } => Some((url.as_str(), *exited_cleanly)),
                    _ => None,
                })
                .collect()
        }

        fn report(&mut self) -> Option<Result<(), SettingsError>> {
            let is_save = self.queue.pop_front()?;
            Some(if is_save {
                self.outcomes.pop_front().unwrap_or(Ok(()))
            } else {
                Ok(())
            })
        }
    }

    fn first_url(snapshot: &SessionSnapshot) -> String {
        snapshot
            .windows
            .first()
            .and_then(|window| window.tabs.first())
            .map(|tab| tab.url.clone())
            .unwrap_or_default()
    }

    impl SessionWriter for FakeWriter {
        fn send(&mut self, request: Request) -> bool {
            if self.stopped {
                return false;
            }
            let (call, is_save) = match request {
                Request::Save {
                    snapshot,
                    exited_cleanly,
                } => (
                    Call::Save {
                        url: first_url(&snapshot),
                        exited_cleanly,
                    },
                    true,
                ),
                Request::RemoveRecovery(snapshot) => (Call::Remove(first_url(&snapshot)), false),
                Request::ResolveRecovery => (Call::ResolveAll, false),
            };
            self.calls.push(call);
            self.queue.push_back(is_save);
            true
        }

        fn poll(&mut self) -> Option<Result<(), SettingsError>> {
            if self.finished == 0 {
                return None;
            }
            self.finished -= 1;
            self.report()
        }

        fn wait(&mut self, _deadline: Instant) -> Option<Result<(), SettingsError>> {
            if self.stalled {
                return None;
            }
            self.report()
        }
    }

    fn at(url: &str) -> SessionSnapshot {
        SessionSnapshot {
            windows: vec![WindowSnapshot {
                tabs: vec![TabSnapshot {
                    url: url.to_owned(),
                    ..TabSnapshot::default()
                }],
                ..WindowSnapshot::default()
            }],
            ..SessionSnapshot::default()
        }
    }

    fn newer() -> SettingsError {
        SettingsError::NewerVersion {
            path: PathBuf::from("session.json"),
            found: 9,
            supported: 1,
        }
    }

    fn failed() -> SettingsError {
        SettingsError::Io {
            action: "write",
            path: PathBuf::from("session.json"),
            source: std::io::Error::other("disk full"),
        }
    }

    fn state() -> SaveState<FakeWriter> {
        SaveState::new(FakeWriter::default(), INTERVAL)
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    fn save(url: &str) -> Call {
        Call::Save {
            url: url.to_owned(),
            exited_cleanly: false,
        }
    }

    #[test]
    fn nothing_is_written_while_nothing_changed() {
        let mut state = state();

        state.tick(0, || at("https://a.test/"));

        assert!(state.writer().calls.is_empty());
    }

    #[test]
    fn no_second_save_starts_while_one_is_in_flight() {
        let mut state = state();
        state.mark_dirty();
        state.tick(0, || at("https://a.test/"));
        state.mark_dirty();

        state.tick(INTERVAL * 3, || at("https://b.test/"));

        assert_eq!(state.writer().saves(), vec![("https://a.test/", false)]);
    }

    #[test]
    fn a_save_counts_only_once_its_write_succeeded() {
        let mut state = state();
        state.writer.outcomes.push_back(Err(failed()));
        state.mark_dirty();
        state.tick(0, || at("https://a.test/"));
        state.writer.finished = 1;

        // The failed write is collected; the change is still unsaved and offered again.
        state.tick(INTERVAL, || at("https://a.test/"));
        assert_eq!(state.writer().saves().len(), 2);

        state.writer.finished = 1;
        state.tick(2 * INTERVAL, || at("https://a.test/"));
        state.tick(10 * INTERVAL, || at("https://a.test/"));
        assert_eq!(
            state.writer().saves().len(),
            2,
            "once saved, nothing is offered until the next change"
        );
    }

    #[test]
    fn changes_made_during_a_write_are_saved_by_the_next_one() {
        let mut state = state();
        state.mark_dirty();
        state.tick(0, || at("https://a.test/"));
        state.mark_dirty();
        state.writer.finished = 1;

        state.tick(INTERVAL, || at("https://b.test/"));
        // The interval runs from the save that just landed.
        state.tick(2 * INTERVAL, || at("https://b.test/"));

        assert_eq!(
            state.writer().saves(),
            vec![("https://a.test/", false), ("https://b.test/", false)]
        );
    }

    #[test]
    fn nothing_is_saved_after_a_newer_version_refused_it_and_the_user_is_told_once() {
        let mut state = state();
        state.writer.outcomes.push_back(Err(newer()));
        state.mark_dirty();
        state.tick(0, || at("https://a.test/"));
        state.writer.finished = 1;
        state.tick(INTERVAL, || at("https://a.test/"));

        assert!(state.take_refusal_notice());
        assert!(!state.take_refusal_notice());
        state.mark_dirty();
        state.tick(10 * INTERVAL, || at("https://b.test/"));
        state.persist_restore(at("https://r.test/"), None);
        assert_eq!(
            state.finish(11 * INTERVAL, deadline(), at("https://b.test/")),
            Written::Refused
        );
        assert_eq!(state.writer().saves().len(), 1);
    }

    #[test]
    fn the_final_save_is_made_even_when_nothing_changed() {
        let mut state = state();

        assert_eq!(
            state.finish(0, deadline(), at("https://a.test/")),
            Written::Saved
        );

        assert_eq!(state.writer().saves(), vec![("https://a.test/", true)]);
    }

    #[test]
    fn the_final_save_follows_the_write_in_flight() {
        let mut state = state();
        state.mark_dirty();
        state.tick(0, || at("https://a.test/"));

        assert_eq!(
            state.finish(100, deadline(), at("https://b.test/")),
            Written::Saved
        );

        assert_eq!(
            state.writer().saves(),
            vec![("https://a.test/", false), ("https://b.test/", true)]
        );
    }

    #[test]
    fn the_final_save_can_be_started_and_collected_later() {
        let mut state = state();

        state.begin_final(at("https://a.test/"));
        assert_eq!(state.writer().saves(), vec![("https://a.test/", true)]);

        assert_eq!(state.collect_final(1, deadline()), Written::Saved);
    }

    #[test]
    fn a_final_save_not_done_by_the_deadline_is_reported() {
        let mut state = state();
        state.mark_dirty();
        state.tick(0, || at("https://a.test/"));
        state.writer.stalled = true;

        assert_eq!(
            state.finish(100, Instant::now(), at("https://b.test/")),
            Written::TimedOut
        );
    }

    #[test]
    fn a_failed_final_save_is_reported() {
        let mut state = state();
        state.writer.outcomes.push_back(Err(failed()));

        assert_eq!(
            state.finish(0, deadline(), at("https://a.test/")),
            Written::Failed
        );
    }

    #[test]
    fn restoring_does_not_wait_and_the_entry_is_removed_after_the_restored_tabs_are_written() {
        let mut state = state();

        state.persist_restore(
            at("https://restored.test/"),
            Some(Resolve::Entry(at("https://crashed.test/"))),
        );
        assert_eq!(state.writer().calls, vec![save("https://restored.test/")]);

        state.writer.finished = 1;
        state.tick(1, || at("https://restored.test/"));

        assert_eq!(
            state.writer().calls,
            vec![
                save("https://restored.test/"),
                Call::Remove("https://crashed.test/".to_owned()),
            ]
        );
    }

    #[test]
    fn a_restore_queues_behind_the_write_in_flight_which_does_not_settle_it() {
        let mut state = state();
        state.mark_dirty();
        state.tick(0, || at("https://blank.test/"));

        state.persist_restore(
            at("https://restored.test/"),
            Some(Resolve::Entry(at("https://crashed.test/"))),
        );
        state.writer.finished = 1;
        state.tick(10, || at("https://restored.test/"));
        assert!(
            !state
                .writer()
                .calls
                .contains(&Call::Remove("https://crashed.test/".to_owned())),
            "the blank write does not hold the restored tabs"
        );

        state.writer.finished = 1;
        state.tick(20, || at("https://restored.test/"));
        assert_eq!(
            state.writer().calls,
            vec![
                save("https://blank.test/"),
                save("https://restored.test/"),
                Call::Remove("https://crashed.test/".to_owned()),
            ]
        );
    }

    #[test]
    fn a_restore_whose_write_fails_keeps_the_entry_until_a_later_write_lands() {
        let mut state = state();
        state.writer.outcomes.push_back(Err(failed()));

        state.persist_restore(
            at("https://restored.test/"),
            Some(Resolve::Entry(at("https://crashed.test/"))),
        );
        state.writer.finished = 1;
        // The failure is collected and, the change being unsaved, the session saved again.
        state.tick(0, || at("https://restored.test/"));
        assert_eq!(state.writer().saves().len(), 2);
        assert!(
            !state
                .writer()
                .calls
                .contains(&Call::Remove("https://crashed.test/".to_owned()))
        );

        state.writer.finished = 1;
        state.tick(1, || at("https://restored.test/"));
        assert_eq!(
            state.writer().calls.last(),
            Some(&Call::Remove("https://crashed.test/".to_owned()))
        );
    }

    #[test]
    fn a_restore_not_yet_written_at_quit_is_settled_by_the_final_save() {
        let mut state = state();
        state.persist_restore(
            at("https://restored.test/"),
            Some(Resolve::Entry(at("https://crashed.test/"))),
        );

        assert_eq!(
            state.finish(1, deadline(), at("https://restored.test/")),
            Written::Saved
        );

        assert_eq!(
            state.writer().calls,
            vec![
                save("https://restored.test/"),
                Call::Save {
                    url: "https://restored.test/".to_owned(),
                    exited_cleanly: true
                },
                Call::Remove("https://crashed.test/".to_owned()),
            ]
        );
        assert!(
            state.writer().queue.is_empty(),
            "the removal was waited for too"
        );
    }

    #[test]
    fn dismissing_waits_behind_a_restore_not_yet_written() {
        let mut state = state();
        state.persist_restore(
            at("https://restored.test/"),
            Some(Resolve::Entry(at("https://crashed.test/"))),
        );

        state.dismiss_all();
        assert!(!state.writer().calls.contains(&Call::ResolveAll));

        state.writer.finished = 1;
        state.tick(1, || at("https://restored.test/"));
        assert_eq!(
            &state.writer().calls[1..],
            &[
                Call::Remove("https://crashed.test/".to_owned()),
                Call::ResolveAll
            ]
        );
    }

    #[test]
    fn dismissing_with_nothing_waiting_is_queued_at_once() {
        let mut state = state();

        state.dismiss_all();

        assert_eq!(state.writer().calls, vec![Call::ResolveAll]);
    }

    #[test]
    fn a_held_previous_session_is_not_overwritten_by_new_tab_pages() {
        let mut state = state();
        state.set_holding_previous(true);
        state.mark_dirty();

        state.tick(0, || at("about:newtab"));
        assert!(state.writer().calls.is_empty());

        state.tick(INTERVAL, || at("https://a.test/"));
        assert_eq!(state.writer().saves(), vec![("https://a.test/", false)]);
    }

    #[test]
    fn a_tab_that_is_never_saved_does_not_count_as_a_page() {
        let mut state = state();
        state.set_holding_previous(true);
        state.mark_dirty();

        state.tick(0, || at("data:text/html,hello"));

        assert!(state.writer().calls.is_empty());
    }

    #[test]
    fn once_a_real_page_is_saved_new_tab_pages_are_saved_normally() {
        let mut state = state();
        state.set_holding_previous(true);
        state.mark_dirty();
        state.tick(0, || at("https://a.test/"));
        state.writer.finished = 1;
        state.tick(1, || at("https://a.test/"));

        // The page is closed again: the file must not keep offering it after a crash.
        state.mark_dirty();
        state.tick(INTERVAL + 1, || at("about:newtab"));

        assert_eq!(
            state.writer().saves(),
            vec![("https://a.test/", false), ("about:newtab", false)]
        );
    }

    #[test]
    fn a_held_previous_session_whose_page_write_failed_stays_held() {
        let mut state = state();
        state.set_holding_previous(true);
        state.writer.outcomes.push_back(Err(failed()));
        state.mark_dirty();
        state.tick(0, || at("https://a.test/"));
        state.writer.finished = 1;
        state.tick(1, || at("https://a.test/"));

        state.tick(INTERVAL, || at("about:newtab"));

        assert_eq!(state.writer().saves(), vec![("https://a.test/", false)]);
    }

    #[test]
    fn a_held_previous_session_still_gets_the_final_save() {
        let mut state = state();
        state.set_holding_previous(true);

        assert_eq!(
            state.finish(0, deadline(), at("about:newtab")),
            Written::Saved
        );

        assert_eq!(state.writer().saves(), vec![("about:newtab", true)]);
    }

    #[test]
    fn a_saver_that_is_gone_saves_nothing_and_says_so() {
        let mut state = state();
        state.writer.stopped = true;
        state.mark_dirty();

        state.tick(0, || at("https://a.test/"));

        assert!(state.writer().calls.is_empty());
        assert_eq!(
            state.finish(1, deadline(), at("https://a.test/")),
            Written::Failed
        );
    }
}
