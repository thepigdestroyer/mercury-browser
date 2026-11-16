//! When and in what order the session is written, and when the recovery list may be cleared:
//! the part of the `mercury-settings` contract (items 5, 8, 9, 11) that is about ordering. Pure:
//! the writer and the time are given, so every rule is tested with a fake.

use std::collections::VecDeque;
use std::time::Duration;

use mercury_settings::{SaveThrottle, SaveToken, SessionSnapshot, SettingsError};

use super::model::only_new_tab_pages;

/// The longest any single wait for a write may take: for the write in flight, then for the
/// write that follows it. Quitting waits at most twice this (2 s) before the window goes and
/// the engine's own 5 s shutdown watchdog starts (`shutdown.rs`); restoring from the bar waits
/// as long once a click.
pub(crate) const WAIT_BUDGET: Duration = Duration::from_secs(1);

/// Writes the session file and edits the recovery list. The real one is a thread
/// (`saver::SaveWorker`) that writes one snapshot at a time, in order.
pub(crate) trait SessionWriter {
    /// Starts writing `snapshot`. False if it could not be started.
    fn begin_save(&mut self, snapshot: SessionSnapshot, exited_cleanly: bool) -> bool;
    /// The outcome of the write started, if it has finished.
    fn poll(&mut self) -> Option<Result<(), SettingsError>>;
    /// Waits at most `budget` for the write started to finish. `None` if it did not.
    fn wait(&mut self, budget: Duration) -> Option<Result<(), SettingsError>>;
    fn remove_recovery(&mut self, index: usize) -> Result<bool, SettingsError>;
    fn resolve_recovery(&mut self) -> Result<(), SettingsError>;
}

/// What becomes of the recovery list once a restore is safely on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resolve {
    /// The entry at this index (of the list as it is after the earlier resolutions) was restored.
    Entry(usize),
    /// The user dismissed every entry.
    All,
}

/// How a write that was waited for ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Written {
    Saved,
    /// The session file is from a newer version; nothing is written this run.
    Refused,
    /// It did not finish within the budget. It may still land later.
    TimedOut,
    Failed,
}

struct InFlight {
    /// The throttle's permission, for a throttled save.
    token: Option<SaveToken>,
    /// The change count the snapshot covers.
    generation: u64,
}

pub(crate) struct SaveState<W> {
    writer: W,
    throttle: SaveThrottle,
    /// Counts changes, like the throttle, so that a resolution waits for a snapshot that holds it.
    changes: u64,
    in_flight: Option<InFlight>,
    saving_refused: bool,
    refusal_to_tell: bool,
    /// Resolutions waiting for a snapshot of at least this change count to be written.
    pending: VecDeque<(Resolve, u64)>,
    /// A clean previous session that was not restored is held: a session of new-tab pages
    /// only is not written over it until the quit.
    holding_previous: bool,
    last_error: Option<String>,
}

impl<W: SessionWriter> SaveState<W> {
    pub(crate) fn new(writer: W, min_interval_ms: i64) -> Self {
        Self {
            writer,
            throttle: SaveThrottle::new(min_interval_ms),
            changes: 0,
            in_flight: None,
            saving_refused: false,
            refusal_to_tell: false,
            pending: VecDeque::new(),
            holding_previous: false,
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

    /// From the timer: collects a finished write, and starts a throttled one when it is due and
    /// nothing is in flight. `snapshot` is only called when a write starts.
    pub(crate) fn tick(&mut self, now_ms: i64, snapshot: impl FnOnce() -> SessionSnapshot) {
        if self.in_flight.is_some()
            && let Some(outcome) = self.writer.poll()
        {
            self.take_outcome(outcome, now_ms);
        }
        if self.saving_refused || self.in_flight.is_some() {
            return;
        }
        let Some(token) = self.throttle.should_save(now_ms) else {
            return;
        };
        let snapshot = snapshot();
        if self.holding_previous && only_new_tab_pages(&snapshot) {
            // Offered again after the interval; still dirty.
            drop(token);
            return;
        }
        if self.writer.begin_save(snapshot, false) {
            self.in_flight = Some(InFlight {
                token: Some(token),
                generation: self.changes,
            });
        } else {
            self.warn_once("the session saver is not running; the session was not saved");
        }
    }

    /// After tabs were restored: writes `snapshot` (which holds them) now, after any write in
    /// flight, and only once it is on disk applies `resolve`. If the write does not land, the
    /// resolution waits for the next write that does.
    pub(crate) fn persist_restore(
        &mut self,
        now_ms: i64,
        snapshot: SessionSnapshot,
        resolve: Option<Resolve>,
    ) -> Written {
        self.mark_dirty();
        if let Some(resolve) = resolve {
            self.pending.push_back((resolve, self.changes));
        }
        self.save_and_wait(now_ms, snapshot, false)
    }

    /// The user dismissed every recovered session: clears the list now, or after the restores
    /// still waiting to be written.
    pub(crate) fn dismiss_all(&mut self) {
        match self.pending.back().map(|(_, generation)| *generation) {
            Some(generation) => self.pending.push_back((Resolve::All, generation)),
            None => self.resolve(Resolve::All),
        }
    }

    /// The final save of an orderly quit, made even when nothing changed: waits for the write
    /// in flight, then writes with `exited_cleanly: true`. Each wait is bounded by
    /// `WAIT_BUDGET`; if one runs out, the run stays marked as running (a crash, the safe state).
    pub(crate) fn finish(&mut self, now_ms: i64, snapshot: SessionSnapshot) -> Written {
        let written = self.save_and_wait(now_ms, snapshot, true);
        if written == Written::TimedOut {
            log::warn!(
                "the session could not be saved at quit within {} ms; it will be offered as a crash",
                WAIT_BUDGET.as_millis()
            );
        }
        written
    }

    fn save_and_wait(
        &mut self,
        now_ms: i64,
        snapshot: SessionSnapshot,
        exited_cleanly: bool,
    ) -> Written {
        if self.in_flight.is_some() {
            match self.writer.wait(WAIT_BUDGET) {
                Some(outcome) => self.take_outcome(outcome, now_ms),
                None => return Written::TimedOut,
            }
        }
        if self.saving_refused {
            return Written::Refused;
        }
        let generation = self.changes;
        if !self.writer.begin_save(snapshot, exited_cleanly) {
            self.warn_once("the session saver is not running; the session was not saved");
            return Written::Failed;
        }
        self.in_flight = Some(InFlight {
            token: None,
            generation,
        });
        let Some(outcome) = self.writer.wait(WAIT_BUDGET) else {
            return Written::TimedOut;
        };
        let saved = outcome.is_ok();
        self.take_outcome(outcome, now_ms);
        if saved {
            Written::Saved
        } else if self.saving_refused {
            Written::Refused
        } else {
            Written::Failed
        }
    }

    fn take_outcome(&mut self, outcome: Result<(), SettingsError>, now_ms: i64) {
        let Some(in_flight) = self.in_flight.take() else {
            return;
        };
        match outcome {
            Ok(()) => {
                match in_flight.token {
                    Some(token) => self.throttle.saved(token, now_ms),
                    // A write made without asking holds every change up to its generation.
                    None if self.changes == in_flight.generation => self.throttle.flushed(now_ms),
                    None => {}
                }
                self.last_error = None;
                while let Some(&(resolve, generation)) = self.pending.front()
                    && generation <= in_flight.generation
                {
                    self.pending.pop_front();
                    self.resolve(resolve);
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
            }
            Err(error) => self.warn_once(&format!("could not save the session: {error}")),
        }
    }

    fn resolve(&mut self, resolve: Resolve) {
        let result = match resolve {
            Resolve::Entry(index) => self.writer.remove_recovery(index).map(|_| ()),
            Resolve::All => self.writer.resolve_recovery(),
        };
        if let Err(error) = result {
            log::warn!("could not update the list of sessions to recover: {error}");
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use mercury_settings::{TabSnapshot, WindowSnapshot};

    use super::*;

    const INTERVAL: i64 = 5_000;

    /// One step of what the fake writer was asked to do, in order.
    #[derive(Debug, PartialEq, Eq)]
    enum Call {
        Save { url: String, exited_cleanly: bool },
        Remove(usize),
        ResolveAll,
    }

    /// A writer whose writes finish only when the test says, and end as the test says.
    #[derive(Default)]
    struct FakeWriter {
        calls: Vec<Call>,
        writing: bool,
        /// Whether the write in progress has finished (for `poll`) and whether `wait` sees it
        /// finish within the budget.
        finished: bool,
        stalled: bool,
        /// How the next writes end; `Ok` when empty.
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

        fn outcome(&mut self) -> Result<(), SettingsError> {
            self.writing = false;
            self.finished = false;
            self.outcomes.pop_front().unwrap_or(Ok(()))
        }
    }

    impl SessionWriter for FakeWriter {
        fn begin_save(&mut self, snapshot: SessionSnapshot, exited_cleanly: bool) -> bool {
            if self.stopped {
                return false;
            }
            assert!(
                !self.writing,
                "a second write was started while one was in flight"
            );
            self.writing = true;
            self.calls.push(Call::Save {
                url: snapshot
                    .windows
                    .first()
                    .and_then(|window| window.tabs.first())
                    .map(|tab| tab.url.clone())
                    .unwrap_or_default(),
                exited_cleanly,
            });
            true
        }

        fn poll(&mut self) -> Option<Result<(), SettingsError>> {
            (self.writing && self.finished).then(|| self.outcome())
        }

        fn wait(&mut self, budget: Duration) -> Option<Result<(), SettingsError>> {
            assert_eq!(budget, WAIT_BUDGET);
            (self.writing && !self.stalled).then(|| self.outcome())
        }

        fn remove_recovery(&mut self, index: usize) -> Result<bool, SettingsError> {
            self.calls.push(Call::Remove(index));
            Ok(true)
        }

        fn resolve_recovery(&mut self) -> Result<(), SettingsError> {
            self.calls.push(Call::ResolveAll);
            Ok(())
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
        state.writer.finished = true;

        // The failed write is collected; the change is still unsaved and offered again.
        state.tick(INTERVAL, || at("https://a.test/"));
        assert_eq!(state.writer().saves().len(), 2);

        state.writer.finished = true;
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
        state.writer.finished = true;

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
        state.writer.finished = true;
        state.tick(INTERVAL, || at("https://a.test/"));

        assert!(state.take_refusal_notice());
        assert!(!state.take_refusal_notice());
        state.mark_dirty();
        state.tick(10 * INTERVAL, || at("https://b.test/"));
        assert_eq!(
            state.finish(11 * INTERVAL, at("https://b.test/")),
            Written::Refused
        );
        assert_eq!(state.writer().saves().len(), 1);
    }

    #[test]
    fn the_final_save_is_made_even_when_nothing_changed() {
        let mut state = state();

        assert_eq!(state.finish(0, at("https://a.test/")), Written::Saved);

        assert_eq!(state.writer().saves(), vec![("https://a.test/", true)]);
    }

    #[test]
    fn the_final_save_follows_the_write_in_flight() {
        let mut state = state();
        state.mark_dirty();
        state.tick(0, || at("https://a.test/"));

        assert_eq!(state.finish(100, at("https://b.test/")), Written::Saved);

        assert_eq!(
            state.writer().saves(),
            vec![("https://a.test/", false), ("https://b.test/", true)]
        );
    }

    #[test]
    fn a_stalled_write_in_flight_leaves_the_run_marked_as_running() {
        let mut state = state();
        state.mark_dirty();
        state.tick(0, || at("https://a.test/"));
        state.writer.stalled = true;

        assert_eq!(state.finish(100, at("https://b.test/")), Written::TimedOut);

        assert_eq!(state.writer().saves(), vec![("https://a.test/", false)]);
    }

    #[test]
    fn a_restored_entry_is_removed_only_after_the_restored_tabs_are_written() {
        let mut state = state();

        let written =
            state.persist_restore(0, at("https://restored.test/"), Some(Resolve::Entry(1)));

        assert_eq!(written, Written::Saved);
        assert_eq!(
            state.writer().calls,
            vec![
                Call::Save {
                    url: "https://restored.test/".to_owned(),
                    exited_cleanly: false
                },
                Call::Remove(1),
            ]
        );
    }

    #[test]
    fn a_restore_waits_for_the_write_in_flight_so_an_older_one_cannot_land_after_it() {
        let mut state = state();
        state.mark_dirty();
        state.tick(0, || at("https://blank.test/"));

        state.persist_restore(10, at("https://restored.test/"), Some(Resolve::Entry(0)));

        assert_eq!(
            state.writer().calls,
            vec![
                Call::Save {
                    url: "https://blank.test/".to_owned(),
                    exited_cleanly: false
                },
                Call::Save {
                    url: "https://restored.test/".to_owned(),
                    exited_cleanly: false
                },
                Call::Remove(0),
            ]
        );
    }

    #[test]
    fn a_restore_whose_write_fails_keeps_the_entry_until_a_later_write_lands() {
        let mut state = state();
        state.writer.outcomes.push_back(Err(failed()));

        let written =
            state.persist_restore(0, at("https://restored.test/"), Some(Resolve::Entry(0)));

        assert_eq!(written, Written::Failed);
        assert!(!state.writer().calls.contains(&Call::Remove(0)));
        // The next throttled write holds the restored tabs, so it settles the entry.
        state.tick(INTERVAL, || at("https://restored.test/"));
        assert!(!state.writer().calls.contains(&Call::Remove(0)));
        state.writer.finished = true;
        state.tick(INTERVAL + 1, || at("https://restored.test/"));
        assert_eq!(state.writer().calls.last(), Some(&Call::Remove(0)));
    }

    #[test]
    fn a_restore_whose_write_stalls_is_settled_when_that_write_lands() {
        let mut state = state();
        state.writer.stalled = true;

        let written =
            state.persist_restore(0, at("https://restored.test/"), Some(Resolve::Entry(2)));

        assert_eq!(written, Written::TimedOut);
        assert_eq!(state.writer().calls.len(), 1);
        state.writer.finished = true;
        state.tick(1, || at("https://restored.test/"));
        assert_eq!(state.writer().calls.last(), Some(&Call::Remove(2)));
    }

    #[test]
    fn an_older_write_landing_does_not_settle_a_newer_restore() {
        let mut state = state();
        state.mark_dirty();
        state.tick(0, || at("https://blank.test/"));
        state.writer.stalled = true;

        // The wait for the blank write runs out; the restore is not written yet.
        let written =
            state.persist_restore(10, at("https://restored.test/"), Some(Resolve::Entry(0)));
        assert_eq!(written, Written::TimedOut);

        state.writer.finished = true;
        state.tick(20, || at("https://restored.test/"));
        assert!(
            !state.writer().calls.contains(&Call::Remove(0)),
            "the blank write does not hold the restored tabs"
        );
    }

    #[test]
    fn dismissing_waits_behind_a_restore_not_yet_written() {
        let mut state = state();
        state.writer.stalled = true;
        let _ = state.persist_restore(0, at("https://restored.test/"), Some(Resolve::Entry(1)));

        state.dismiss_all();
        assert!(!state.writer().calls.contains(&Call::ResolveAll));

        state.writer.finished = true;
        state.tick(1, || at("https://restored.test/"));
        assert_eq!(
            &state.writer().calls[1..],
            &[Call::Remove(1), Call::ResolveAll]
        );
    }

    #[test]
    fn dismissing_with_nothing_waiting_resolves_at_once() {
        let mut state = state();

        state.dismiss_all();

        assert_eq!(state.writer().calls, vec![Call::ResolveAll]);
    }

    #[test]
    fn a_held_previous_session_is_not_overwritten_by_new_tab_pages_until_quit() {
        let mut state = state();
        state.set_holding_previous(true);
        state.mark_dirty();

        state.tick(0, || at("about:newtab"));
        assert!(state.writer().calls.is_empty());

        state.tick(INTERVAL, || at("https://a.test/"));
        assert_eq!(state.writer().saves(), vec![("https://a.test/", false)]);
    }

    #[test]
    fn a_held_previous_session_still_gets_the_final_save() {
        let mut state = state();
        state.set_holding_previous(true);

        assert_eq!(state.finish(0, at("about:newtab")), Written::Saved);

        assert_eq!(state.writer().saves(), vec![("about:newtab", true)]);
    }

    #[test]
    fn a_saver_that_is_gone_saves_nothing_and_says_so() {
        let mut state = state();
        state.writer.stopped = true;
        state.mark_dirty();

        state.tick(0, || at("https://a.test/"));

        assert!(state.writer().calls.is_empty());
        assert_eq!(state.finish(1, at("https://a.test/")), Written::Failed);
    }
}
