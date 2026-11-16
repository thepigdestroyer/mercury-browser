//! Writes the running session off the UI thread, one write at a time.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use mercury_settings::{SessionSnapshot, SessionStore, SettingsError};

/// A thread that writes snapshots with `exited_cleanly: false`, in the order given, and reports
/// each outcome.
pub(crate) struct SaveWorker {
    requests: Option<Sender<SessionSnapshot>>,
    outcomes: Receiver<Result<(), SettingsError>>,
    thread: Option<JoinHandle<()>>,
}

impl SaveWorker {
    pub(crate) fn start(store: SessionStore) -> std::io::Result<Self> {
        let (requests, requested) = mpsc::channel::<SessionSnapshot>();
        let (report, outcomes) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("session saver".to_owned())
            .spawn(move || {
                for snapshot in requested {
                    if report.send(store.save(&snapshot, false)).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests: Some(requests),
            outcomes,
            thread: Some(thread),
        })
    }

    /// Hands a snapshot to the thread. False if the thread is gone.
    pub(crate) fn save(&self, snapshot: SessionSnapshot) -> bool {
        self.requests
            .as_ref()
            .is_some_and(|requests| requests.send(snapshot).is_ok())
    }

    /// The outcome of a write that has finished, if any.
    pub(crate) fn try_outcome(&self) -> Option<Result<(), SettingsError>> {
        self.outcomes.try_recv().ok()
    }

    /// Waits for every write handed over to finish and stops the thread. Returns the outcomes
    /// not yet collected.
    pub(crate) fn finish(mut self) -> Vec<Result<(), SettingsError>> {
        // Closing the channel ends the thread's loop once the queued writes are done.
        drop(self.requests.take());
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("the session saver panicked");
        }
        self.outcomes.try_iter().collect()
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
    fn finishing_waits_for_the_writes_in_flight_and_reports_them() {
        let directory =
            std::env::temp_dir().join(format!("mercury-session-saver-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let worker = SaveWorker::start(SessionStore::new(&directory)).unwrap();

        assert!(worker.save(one_tab("https://a.test/")));
        assert!(worker.save(one_tab("https://b.test/")));
        let outcomes = worker.finish();

        let written = std::fs::read_to_string(directory.join("session.json")).unwrap();
        let _ = std::fs::remove_dir_all(&directory);
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(Result::is_ok));
        assert!(written.contains("https://b.test/"), "{written}");
        assert!(written.contains("\"exited_cleanly\": false"), "{written}");
    }
}
