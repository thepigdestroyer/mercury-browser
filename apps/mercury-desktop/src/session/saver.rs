//! Carries out the session's writes off the UI thread, one at a time, in order.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Instant;

use mercury_settings::{SessionStore, SettingsError};

use super::saving::{Request, SessionWriter};

/// A thread that carries out the requests it is given, in order, and reports each outcome:
/// session writes and edits of the recovery list alike, so none waits on the UI thread and
/// none overtakes another.
pub(crate) struct SaveWorker {
    requests: Sender<Request>,
    outcomes: Receiver<Result<(), SettingsError>>,
}

impl SaveWorker {
    /// The thread ends when the worker is dropped, after the request it is doing; nothing
    /// waits for it beyond the quit deadline (a stalled disk must not hold up quitting).
    pub(crate) fn start(store: SessionStore) -> std::io::Result<Self> {
        let (requests, requested) = mpsc::channel::<Request>();
        let (report, outcomes) = mpsc::channel();
        thread::Builder::new()
            .name("session saver".to_owned())
            .spawn(move || {
                for request in requested {
                    let outcome = match request {
                        Request::Save {
                            snapshot,
                            exited_cleanly,
                        } => store.save(&snapshot, exited_cleanly),
                        Request::RemoveRecovery(snapshot) => {
                            store.remove_recovery(&snapshot).map(|_| ())
                        }
                        Request::ResolveRecovery => store.resolve_recovery(),
                    };
                    if report.send(outcome).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self { requests, outcomes })
    }
}

impl SessionWriter for SaveWorker {
    fn send(&mut self, request: Request) -> bool {
        self.requests.send(request).is_ok()
    }

    fn poll(&mut self) -> Option<Result<(), SettingsError>> {
        self.outcomes.try_recv().ok()
    }

    fn wait(&mut self, deadline: Instant) -> Option<Result<(), SettingsError>> {
        let budget = deadline.saturating_duration_since(Instant::now());
        match self.outcomes.recv_timeout(budget) {
            Ok(outcome) => Some(outcome),
            Err(RecvTimeoutError::Timeout) => None,
            // The thread is gone (it panicked): the request was not carried out.
            Err(RecvTimeoutError::Disconnected) => Some(Err(SettingsError::Io {
                action: "save",
                path: "session.json".into(),
                source: std::io::Error::other("the session saver stopped"),
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mercury_settings::{SessionSnapshot, TabSnapshot, WindowSnapshot};

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

        let deadline = Instant::now() + Duration::from_secs(5);
        assert!(worker.send(Request::Save {
            snapshot: one_tab("https://a.test/"),
            exited_cleanly: false
        }));
        assert!(worker.send(Request::Save {
            snapshot: one_tab("https://b.test/"),
            exited_cleanly: true
        }));
        // The list edit runs on the thread too; with no list there is nothing to clear.
        assert!(worker.send(Request::ResolveRecovery));
        for _ in 0..3 {
            assert!(worker.wait(deadline).unwrap().is_ok());
        }
        assert!(worker.poll().is_none());

        let written = std::fs::read_to_string(directory.path().join("session.json")).unwrap();
        assert!(written.contains("https://b.test/"), "{written}");
        assert!(written.contains("\"exited_cleanly\": true"), "{written}");
    }

    #[test]
    fn a_deadline_already_passed_returns_at_once_without_an_outcome() {
        let directory = tempfile::tempdir().unwrap();
        let mut worker = SaveWorker::start(SessionStore::new(directory.path())).unwrap();
        let started = Instant::now();

        let outcome = worker.wait(
            started
                .checked_sub(Duration::from_secs(1))
                .unwrap_or(started),
        );

        assert!(outcome.is_none());
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}
