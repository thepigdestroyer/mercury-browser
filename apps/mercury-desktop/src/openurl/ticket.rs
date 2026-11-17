//! Who decides the fate of a second launch's addresses: the window, which opens them, or the
//! listening thread, which gives up waiting. Exactly one of them wins, so a late window never
//! opens tabs the second launch was told were not opened, and the thread never reports a
//! failure for tabs the window did open.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

const WAITING: u8 = 0;
const OPENING: u8 = 1;
const ABANDONED: u8 = 2;

#[derive(Clone, Debug, Default)]
pub(crate) struct HandOverTicket(Arc<AtomicU8>);

impl HandOverTicket {
    /// The window is about to open the addresses. False if the listening thread has given up:
    /// then nothing may be opened.
    pub(crate) fn claim_for_opening(&self) -> bool {
        self.0
            .compare_exchange(WAITING, OPENING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// The listening thread gives up waiting. False if the window has already started opening
    /// them: the thread must then wait for its result.
    pub(crate) fn abandon(&self) -> bool {
        self.0
            .compare_exchange(WAITING, ABANDONED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_that_comes_first_opens_and_the_thread_waits_for_it() {
        let ticket = HandOverTicket::default();

        assert!(ticket.clone().claim_for_opening());
        assert!(!ticket.abandon());
    }

    #[test]
    fn a_late_window_opens_nothing() {
        let ticket = HandOverTicket::default();

        assert!(ticket.abandon());
        assert!(!ticket.claim_for_opening());
        assert!(!ticket.claim_for_opening(), "never twice");
    }

    #[test]
    fn the_window_opens_at_most_once() {
        let ticket = HandOverTicket::default();

        assert!(ticket.claim_for_opening());
        assert!(!ticket.claim_for_opening());
    }

    #[test]
    fn only_one_side_wins_a_race() {
        for _ in 0..200 {
            let ticket = HandOverTicket::default();
            let window = ticket.clone();
            let opened = std::thread::spawn(move || window.claim_for_opening());
            let abandoned = ticket.abandon();
            let opened = opened.join().expect("the thread ends");
            assert_ne!(opened, abandoned);
        }
    }
}
