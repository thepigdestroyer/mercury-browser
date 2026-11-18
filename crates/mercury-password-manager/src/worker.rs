// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The one place the password store is used: a worker thread that owns it, so a Keychain call
//! that waits for the user (an unlock or access dialog) never blocks the browser's interface.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;

use mercury_passwords::PasswordStore;

use crate::AnyBackend;

/// Whether a job may make the system ask the user (unlock the keychain, allow access to an
/// item).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interaction {
    /// Only for jobs that follow a gesture of the user in Mercury's own interface.
    Allowed,
    /// For everything else (start-up, anything a page caused): an operation that would ask fails
    /// with `PasswordError::Locked` instead.
    Never,
}

/// A job's result, on its way back to the interface thread.
pub type JobResult = Box<dyn Any + Send>;

/// Hands a finished job's id and result to the interface thread, which then calls
/// [`StoreWorker::finish`] (in the browser: through `slint::invoke_from_event_loop`).
pub type Deliver = Arc<dyn Fn(u64, JobResult) + Send + Sync>;

type Job = Box<dyn FnOnce(&PasswordStore<AnyBackend>) -> JobResult + Send>;
type Reply = Box<dyn FnOnce(JobResult)>;

enum Mode {
    /// Jobs run at once, on the caller's thread: the memory store, which never waits.
    Inline(PasswordStore<AnyBackend>),
    /// Jobs wait until [`StoreWorker::run_queued`]: for tests of what happens meanwhile.
    Deferred {
        store: PasswordStore<AnyBackend>,
        queue: RefCell<VecDeque<(u64, Interaction, Job)>>,
    },
    /// Jobs run in order on the worker thread.
    Thread(mpsc::Sender<(u64, Interaction, Job)>),
}

/// Runs store jobs and calls each job's reply on the interface thread, once.
pub struct StoreWorker {
    mode: Mode,
    persistent: bool,
    next_id: Cell<u64>,
    last_interaction: Cell<Option<Interaction>>,
    on_failure: RefCell<Option<Rc<dyn Fn()>>>,
    waiting: RefCell<HashMap<u64, Reply>>,
}

impl StoreWorker {
    /// Runs jobs inline. For the memory store.
    pub fn inline(backend: AnyBackend) -> Self {
        Self::with_mode(
            backend.is_persistent(),
            Mode::Inline(PasswordStore::new(backend)),
        )
    }

    /// Queues jobs until [`StoreWorker::run_queued`]. For tests.
    pub fn deferred(backend: AnyBackend) -> Self {
        Self::with_mode(
            backend.is_persistent(),
            Mode::Deferred {
                store: PasswordStore::new(backend),
                queue: RefCell::default(),
            },
        )
    }

    /// Starts the worker thread, which owns the store from now on.
    pub fn spawn(backend: AnyBackend, deliver: Deliver) -> std::io::Result<Self> {
        let persistent = backend.is_persistent();
        let (sender, receiver) = mpsc::channel::<(u64, Interaction, Job)>();
        std::thread::Builder::new()
            .name("password-store".to_owned())
            .spawn(move || {
                let store = PasswordStore::new(backend);
                store.backend().bind_to_current_thread();
                for (id, interaction, job) in receiver {
                    let result = run_job(&store, interaction, job);
                    deliver(id, result);
                }
            })?;
        Ok(Self::with_mode(persistent, Mode::Thread(sender)))
    }

    fn with_mode(persistent: bool, mode: Mode) -> Self {
        Self {
            mode,
            persistent,
            next_id: Cell::new(0),
            last_interaction: Cell::new(None),
            on_failure: RefCell::default(),
            waiting: RefCell::default(),
        }
    }

    /// Called on this thread when a job panicked (its reply never runs) or the worker is gone.
    pub fn set_on_failure(&self, on_failure: Box<dyn Fn()>) {
        *self.on_failure.borrow_mut() = Some(on_failure.into());
    }

    fn failed(&self) {
        let on_failure = self.on_failure.borrow().clone();
        if let Some(on_failure) = on_failure {
            on_failure();
        }
    }

    /// Whether logins outlive this run (the Keychain).
    pub fn is_persistent(&self) -> bool {
        self.persistent
    }

    /// Runs `job` with the store, then `reply` with its result on this (the interface) thread.
    /// Inline, `reply` runs before this returns.
    pub fn run<R: Send + 'static>(
        &self,
        interaction: Interaction,
        job: impl FnOnce(&PasswordStore<AnyBackend>) -> R + Send + 'static,
        reply: impl FnOnce(R) + 'static,
    ) {
        let job: Job = Box::new(move |store| Box::new(job(store)) as JobResult);
        let reply: Reply = Box::new(move |result| {
            if let Ok(result) = result.downcast::<R>() {
                reply(*result);
            }
        });
        self.last_interaction.set(Some(interaction));
        let id = self.next_id.get() + 1;
        self.next_id.set(id);
        match &self.mode {
            Mode::Inline(store) => {
                self.waiting.borrow_mut().insert(id, reply);
                let result = run_job(store, interaction, job);
                self.finish(id, result);
            }
            Mode::Deferred { queue, .. } => {
                self.waiting.borrow_mut().insert(id, reply);
                queue.borrow_mut().push_back((id, interaction, job));
            }
            Mode::Thread(sender) => {
                self.waiting.borrow_mut().insert(id, reply);
                if sender.send((id, interaction, job)).is_err() {
                    // The thread is gone: the reply never comes.
                    self.waiting.borrow_mut().remove(&id);
                    self.failed();
                }
            }
        }
    }

    /// A job's result arrived (see [`Deliver`]); calls its reply. Unknown ids are ignored.
    pub fn finish(&self, id: u64, result: JobResult) {
        // Taken out first: the reply may start another job.
        let reply = self.waiting.borrow_mut().remove(&id);
        let Some(reply) = reply else {
            return;
        };
        if result.is::<JobPanicked>() {
            self.failed();
        } else {
            reply(result);
        }
    }

    /// Deferred mode: runs every queued job and its reply, in order; returns how many ran.
    pub fn run_queued(&self) -> usize {
        let Mode::Deferred { store, queue } = &self.mode else {
            return 0;
        };
        let mut ran = 0;
        loop {
            let next = queue.borrow_mut().pop_front();
            let Some((id, interaction, job)) = next else {
                return ran;
            };
            let result = run_job(store, interaction, job);
            self.finish(id, result);
            ran += 1;
        }
    }

    /// The interaction allowed to the job started last, for tests.
    pub fn last_interaction(&self) -> Option<Interaction> {
        self.last_interaction.get()
    }

    /// Jobs started whose reply has not run yet.
    pub fn pending(&self) -> usize {
        self.waiting.borrow().len()
    }
}

/// What a job that panicked delivers instead of its result.
struct JobPanicked;

/// Runs one job. A panic is caught (and reported as `JobPanicked`), so the worker goes on with
/// the next job, and quiet mode is switched off again whatever happens.
fn run_job(store: &PasswordStore<AnyBackend>, interaction: Interaction, job: Job) -> JobResult {
    struct Loud<'a>(&'a AnyBackend);
    impl Drop for Loud<'_> {
        fn drop(&mut self) {
            self.0.set_quiet(false);
        }
    }

    store.backend().set_quiet(interaction == Interaction::Never);
    let _loud = Loud(store.backend());
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(store))).unwrap_or_else(|_| {
        log::error!("passwords: a store job failed");
        Box::new(JobPanicked)
    })
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use mercury_passwords::{LoginKey, Origin, Password};

    use super::*;

    fn key() -> LoginKey {
        LoginKey::form(Origin::parse("https://a.test").expect("valid"), "alice")
    }

    fn memory() -> AnyBackend {
        AnyBackend::memory()
    }

    #[test]
    fn the_backend_can_move_to_the_worker_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<AnyBackend>();
    }

    #[test]
    fn inline_jobs_reply_before_run_returns() {
        let worker = StoreWorker::inline(memory());
        let seen = Rc::new(Cell::new(None));
        let reply = Rc::clone(&seen);

        worker.run(
            Interaction::Allowed,
            |store| {
                store
                    .save(&key(), &Password::new("fake".to_owned()))
                    .is_ok()
            },
            move |saved| reply.set(Some(saved)),
        );

        assert_eq!(seen.get(), Some(true));
        assert!(!worker.is_persistent());
        assert_eq!(worker.pending(), 0);
    }

    #[test]
    fn the_last_jobs_interaction_is_known() {
        let worker = StoreWorker::inline(memory());
        assert_eq!(worker.last_interaction(), None);

        worker.run(Interaction::Never, |_| (), |()| {});

        assert_eq!(worker.last_interaction(), Some(Interaction::Never));
    }

    #[test]
    fn deferred_jobs_wait_and_then_reply_in_order() {
        let worker = StoreWorker::deferred(memory());
        let order = Rc::new(RefCell::new(Vec::new()));
        for number in [1, 2] {
            let order = Rc::clone(&order);
            worker.run(
                Interaction::Never,
                move |_| number,
                move |n| order.borrow_mut().push(n),
            );
        }
        assert_eq!(worker.pending(), 2);
        assert!(order.borrow().is_empty());

        assert_eq!(worker.run_queued(), 2);

        assert_eq!(*order.borrow(), vec![1, 2]);
        assert_eq!(worker.pending(), 0);
    }

    #[test]
    fn thread_jobs_reply_once_their_result_is_delivered() {
        let (sender, receiver) = mpsc::channel();
        let deliver: Deliver = Arc::new(move |id, result| {
            let _ = sender.send((id, result));
        });
        let worker = StoreWorker::spawn(memory(), deliver).expect("a thread");
        let seen = Rc::new(Cell::new(None));
        let reply = Rc::clone(&seen);

        worker.run(
            Interaction::Allowed,
            |store| {
                store
                    .save(&key(), &Password::new("fake".to_owned()))
                    .expect("saved");
                store.all().expect("listed").len()
            },
            move |count| reply.set(Some(count)),
        );
        assert_eq!(seen.get(), None, "not before the result is delivered");
        let (id, result) = receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("delivered");
        worker.finish(id, result);
        worker.finish(id, Box::new(0_usize));

        assert_eq!(seen.get(), Some(1), "replied exactly once");
    }
}

#[cfg(test)]
mod panic_tests {
    use std::rc::Rc;

    use super::*;

    fn quiet_panicking_job(store: &PasswordStore<AnyBackend>) -> u8 {
        let _ = store;
        panic!("a job failed (expected in this test)");
    }

    fn check(worker: &StoreWorker, settle: impl Fn(&StoreWorker)) {
        let failures = Rc::new(Cell::new(0));
        let counted = Rc::clone(&failures);
        worker.set_on_failure(Box::new(move || counted.set(counted.get() + 1)));
        let replied = Rc::new(Cell::new(false));
        let seen = Rc::clone(&replied);

        worker.run(Interaction::Never, quiet_panicking_job, move |_| seen.set(true));
        settle(worker);
        assert!(!replied.get(), "no reply for a job that panicked");
        assert_eq!(failures.get(), 1);
        assert_eq!(worker.pending(), 0);

        let quiet = Rc::new(Cell::new(None));
        let after = Rc::clone(&quiet);
        worker.run(
            Interaction::Allowed,
            |store| store.backend().is_quiet(),
            move |was_quiet| after.set(Some(was_quiet)),
        );
        settle(worker);
        assert_eq!(quiet.get(), Some(false), "the worker still runs jobs, not quietly");
    }

    #[test]
    fn a_job_that_panics_reports_a_failure_inline() {
        check(&StoreWorker::inline(AnyBackend::memory()), |_| {});
    }

    #[test]
    fn a_job_that_panics_reports_a_failure_deferred() {
        check(&StoreWorker::deferred(AnyBackend::memory()), |worker| {
            worker.run_queued();
        });
    }

    #[test]
    fn a_job_that_panics_leaves_the_worker_thread_running() {
        let (sender, receiver) = mpsc::channel();
        let deliver: Deliver = Arc::new(move |id, result| {
            let _ = sender.send((id, result));
        });
        let worker = StoreWorker::spawn(AnyBackend::memory(), deliver).expect("a thread");
        check(&worker, |worker| {
            let (id, result) = receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("delivered");
            worker.finish(id, result);
        });
    }
}
