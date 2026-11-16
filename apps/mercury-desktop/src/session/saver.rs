//! Writes the session file off the UI thread, one write at a time, in order.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use mercury_settings::{SessionSnapshot, SessionStore, SettingsError};

use super::saving::SessionWriter;

/// A thread that writes the snapshots it is given, in order, and reports each outcome. Edits
/// of the recovery list are made on the caller's thread: they are rare and follow a write the
/// caller has already waited for.
pub(crate) struct SaveWorker {
    store: SessionStore,
    requests: Sender<(SessionSnapshot, bool)>,
    outcomes: Receiver<Result<(), SettingsError>>,
}

impl SaveWorker {
    /// The thread ends when the worker is dropped, after the write it is doing; nothing waits
    /// for it (a stalled disk must not hold up quitting).
    pub(crate) fn start(store: SessionStore) -> std::io::Result<Self> {
        let (requests, requested) = mpsc::channel::<(SessionSnapshot, bool)>();
        let (report, outcomes) = mpsc::channel();
        let thread_store = store.clone();
        thread::Builder::new()
            .name("session saver".to_owned())
            .spawn(move || {
                for (snapshot, exited_cleanly) in requested {
                    if report
                        .send(thread_store.save(&snapshot, exited_cleanly))
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self {
            store,
            requests,
            outcomes,
        })
    }
}

impl SessionWriter for SaveWorker {
    fn begin_save(&mut self, snapshot: SessionSnapshot, exited_cleanly: bool) -> bool {
        self.requests.send((snapshot, exited_cleanly)).is_ok()
    }

    fn poll(&mut self) -> Option<Result<(), SettingsError>> {
        self.outcomes.try_recv().ok()
    }

    fn wait(&mut self, budget: Duration) -> Option<Result<(), SettingsError>> {
        match self.outcomes.recv_timeout(budget) {
            Ok(outcome) => Some(outcome),
            Err(RecvTimeoutError::Timeout) => None,
            // The thread is gone (it panicked): the write did not land.
            Err(RecvTimeoutError::Disconnected) => Some(Err(SettingsError::Io {
                action: "save",
                path: "session.json".into(),
                source: std::io::Error::other("the session saver stopped"),
            })),
        }
    }

    fn remove_recovery(&mut self, index: usize) -> Result<bool, SettingsError> {
        self.store.remove_recovery(index)
    }

    fn resolve_recovery(&mut self) -> Result<(), SettingsError> {
        self.store.resolve_recovery()
    }
}

#[cfg(test)]
mod tests {
    use mercury_settings::{TabSnapshot, WindowSnapshot};

    use super::*;

    fn one_tab(url: &str) -> SessionSnapshot {
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

    #[test]
    fn writes_land_in_order_with_their_mark_and_are_reported() {
        let directory = tempfile::tempdir().unwrap();
        let mut worker = SaveWorker::start(SessionStore::new(directory.path())).unwrap();

        assert!(worker.begin_save(one_tab("https://a.test/"), false));
        assert!(worker.wait(Duration::from_secs(5)).unwrap().is_ok());
        assert!(worker.begin_save(one_tab("https://b.test/"), true));
        assert!(worker.wait(Duration::from_secs(5)).unwrap().is_ok());
        assert!(worker.poll().is_none());

        let written = std::fs::read_to_string(directory.path().join("session.json")).unwrap();
        assert!(written.contains("https://b.test/"), "{written}");
        assert!(written.contains("\"exited_cleanly\": true"), "{written}");
    }
}
