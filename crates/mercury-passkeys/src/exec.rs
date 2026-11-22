// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A minimal blocking executor for passkey-rs futures.
//!
//! passkey-rs' authenticator is `async`, but every callback this crate gives it (the store, the
//! user verifier) blocks until it has an answer, so its futures complete on the first poll. The
//! executor still handles a future that returns `Pending` and is woken later, so a change in a
//! dependency cannot turn into a hang or a busy loop.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

struct ThreadWaker {
    thread: Thread,
    woken: AtomicBool,
}

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::Release);
        self.thread.unpark();
    }
}

/// Runs `future` to completion on the calling thread, parking the thread while it is pending.
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let signal = Arc::new(ThreadWaker {
        thread: thread::current(),
        woken: AtomicBool::new(false),
    });
    let waker = Waker::from(Arc::clone(&signal));
    let mut context = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        // `park` may return without an unpark, so wait for the flag, not for the return.
        while !signal.woken.swap(false, Ordering::Acquire) {
            thread::park();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_ready_future_gives_its_value() {
        assert_eq!(block_on(async { 41 + 1 }), 42);
    }

    #[test]
    fn nested_async_calls_run_in_order() {
        async fn inner(value: u32) -> u32 {
            value * 2
        }
        async fn outer() -> u32 {
            inner(inner(3).await).await + 1
        }
        assert_eq!(block_on(outer()), 13);
    }

    /// Pending until polled `polls_needed` times, waking itself each time.
    struct SelfWaking {
        polls: u32,
        polls_needed: u32,
    }

    impl Future for SelfWaking {
        type Output = u32;

        fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<u32> {
            self.polls += 1;
            if self.polls >= self.polls_needed {
                Poll::Ready(self.polls)
            } else {
                context.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    #[test]
    fn a_future_that_wakes_itself_is_polled_again() {
        let output = block_on(SelfWaking {
            polls: 0,
            polls_needed: 5,
        });
        assert_eq!(output, 5);
    }

    /// Pending until another thread sets the value and calls the stored waker.
    struct Shared {
        value: Option<u32>,
        waker: Option<Waker>,
    }

    struct WaitForThread(Arc<Mutex<Shared>>);

    impl Future for WaitForThread {
        type Output = u32;

        fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<u32> {
            let mut shared = self.0.lock().expect("test lock");
            match shared.value {
                Some(value) => Poll::Ready(value),
                None => {
                    shared.waker = Some(context.waker().clone());
                    Poll::Pending
                }
            }
        }
    }

    #[test]
    fn a_future_woken_from_another_thread_completes() {
        let shared = Arc::new(Mutex::new(Shared {
            value: None,
            waker: None,
        }));
        let for_thread = Arc::clone(&shared);
        let helper = thread::spawn(move || {
            // Wait until the executor has parked on the future, then answer.
            loop {
                thread::sleep(Duration::from_millis(5));
                let mut shared = for_thread.lock().expect("test lock");
                if let Some(waker) = shared.waker.take() {
                    shared.value = Some(7);
                    drop(shared);
                    waker.wake();
                    break;
                }
            }
        });
        assert_eq!(block_on(WaitForThread(shared)), 7);
        helper.join().expect("helper thread");
    }

    #[test]
    fn a_wake_before_the_park_is_not_lost() {
        // The future wakes itself from another thread before returning `Pending`.
        struct WakeThenPend {
            done: bool,
        }
        impl Future for WakeThenPend {
            type Output = ();

            fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
                if self.done {
                    return Poll::Ready(());
                }
                self.done = true;
                let waker = context.waker().clone();
                thread::spawn(move || waker.wake())
                    .join()
                    .expect("waker thread");
                Poll::Pending
            }
        }
        block_on(WakeThenPend { done: false });
    }
}
