// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A watchdog for calls that could deadlock: the call runs on a helper thread and the caller waits
//! with a timeout. A stuck call is reported as a deadlock and the stuck thread is left behind (it can
//! never be joined); the scenario then ends, and the harness kills the child process.

use std::fmt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchError {
    /// The call did not return within the timeout: "deadlock in `what`".
    Deadlock { what: String, after: Duration },
    /// The helper thread panicked (a bug in the scenario or an `unwrap` inside the callee).
    Panicked { what: String },
}

impl fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchError::Deadlock { what, after } => {
                write!(
                    f,
                    "deadlock in {what} (no return after {} ms)",
                    after.as_millis()
                )
            }
            WatchError::Panicked { what } => write!(f, "{what} panicked on its helper thread"),
        }
    }
}

/// Runs `f` on a new thread named `watched:<what>` and waits at most `timeout` for its value.
pub fn call_with_timeout<T, F>(what: &str, timeout: Duration, f: F) -> Result<T, WatchError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name(format!("watched:{what}"))
        .spawn(move || {
            let _ = sender.send(f());
        });
    if spawned.is_err() {
        return Err(WatchError::Panicked {
            what: format!("{what} (thread could not start)"),
        });
    }
    match receiver.recv_timeout(timeout) {
        Ok(value) => Ok(value),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(WatchError::Deadlock {
            what: what.to_owned(),
            after: timeout,
        }),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(WatchError::Panicked {
            what: what.to_owned(),
        }),
    }
}

/// A flag a worker sets when it is done, with the time it took; the owner polls it with a deadline.
/// Used where the call runs inside someone else's callback and cannot be moved to a helper thread.
#[derive(Clone)]
pub struct Done {
    inner: std::sync::Arc<(std::sync::Mutex<Option<Duration>>, std::sync::Condvar)>,
    started: std::sync::Arc<std::sync::Mutex<Option<Instant>>>,
}

impl Default for Done {
    fn default() -> Self {
        Self::new()
    }
}

impl Done {
    pub fn new() -> Self {
        Done {
            inner: std::sync::Arc::new((std::sync::Mutex::new(None), std::sync::Condvar::new())),
            started: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// The worker calls this just before the call that might block.
    pub fn start(&self) {
        *self.started.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }

    /// Whether `start` has been called.
    pub fn has_started(&self) -> bool {
        self.started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// The worker calls this when the call returned.
    pub fn finish(&self) {
        let started = self
            .started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .unwrap_or_else(Instant::now);
        let (lock, changed) = &*self.inner;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(started.elapsed());
        changed.notify_all();
    }

    /// Waits until `finish` or the timeout; `Some(duration of the call)` when it finished.
    pub fn wait(&self, timeout: Duration) -> Option<Duration> {
        let (lock, changed) = &*self.inner;
        let deadline = Instant::now() + timeout;
        let mut value = lock.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(took) = *value {
                return Some(took);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            value = changed
                .wait_timeout(value, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_call_returns_its_value() {
        let value = call_with_timeout("quick", Duration::from_secs(2), || 41 + 1).unwrap();
        assert_eq!(value, 42);
    }

    #[test]
    fn a_stuck_call_is_reported_as_a_deadlock() {
        let error = call_with_timeout("stuck", Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(5));
        })
        .unwrap_err();
        assert!(matches!(&error, WatchError::Deadlock { what, .. } if what == "stuck"));
        assert!(error.to_string().starts_with("deadlock in stuck"));
    }

    #[test]
    fn a_panicking_call_is_reported() {
        let error =
            call_with_timeout::<(), _>("boom", Duration::from_secs(2), || panic!("on purpose"))
                .unwrap_err();
        assert_eq!(
            error,
            WatchError::Panicked {
                what: "boom".into()
            }
        );
    }

    #[test]
    fn done_reports_the_duration_or_none() {
        let done = Done::new();
        assert!(!done.has_started());
        assert_eq!(done.wait(Duration::from_millis(20)), None);
        let worker = done.clone();
        let handle = std::thread::spawn(move || {
            worker.start();
            std::thread::sleep(Duration::from_millis(30));
            worker.finish();
        });
        let took = done.wait(Duration::from_secs(5)).expect("finished");
        assert!(took >= Duration::from_millis(25), "{took:?}");
        handle.join().unwrap();
        assert!(done.has_started());
    }
}
