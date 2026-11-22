// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Which of a tab's input events were wheel events, so that Servo's answer
//! (`WebViewDelegate::notify_input_event_handled`) can be told to the browser as
//! `BrowserEvent::WheelHandled`: a page whose `wheel` listener cancels the event owns the scroll,
//! and the two-finger swipe (gestures lane) must leave it alone.
//!
//! Servo answers every input event, but Mercury only needs the wheel's answer, so the web view
//! notes each wheel event's id as it sends it and the delegate asks whether an answer is one of
//! them. The ledger keeps the newest [`CAPACITY`] ids: an answer for an older one is simply not
//! reported (the swipe then proceeds as if the page had not cancelled, which is the safe side).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use servo::{InputEventId, InputEventResult};

/// How many unanswered wheel events are remembered.
const CAPACITY: usize = 256;

#[derive(Default)]
pub(crate) struct WheelLedger {
    sent: RefCell<VecDeque<InputEventId>>,
}

impl WheelLedger {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Notes a wheel event that was sent to the engine.
    pub(crate) fn sent(&self, id: InputEventId) {
        let mut sent = self.sent.borrow_mut();
        if sent.len() == CAPACITY {
            sent.pop_front();
        }
        sent.push_back(id);
    }

    /// Whether `id` was one of ours; it is forgotten when it was.
    pub(crate) fn answered(&self, id: InputEventId) -> bool {
        let mut sent = self.sent.borrow_mut();
        match sent.iter().position(|sent_id| *sent_id == id) {
            Some(index) => {
                sent.remove(index);
                true
            }
            None => false,
        }
    }
}

/// Whether the page took the wheel event for itself: its script cancelled it. `Consumed` is the
/// engine's own default handling and `DispatchFailed` means the page saw nothing, so neither is
/// the page's.
pub(crate) fn wheel_prevented(result: InputEventResult) -> bool {
    result.contains(InputEventResult::DefaultPrevented)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Event ids cannot be built by hand: they come from the engine's counter. Real events
    /// make fresh, distinct ones.
    fn ids(count: usize) -> Vec<InputEventId> {
        (0..count)
            .map(|_| {
                servo::InputEventAndId::from(servo::InputEvent::MouseLeftViewport(
                    servo::MouseLeftViewportEvent::default(),
                ))
                .id
            })
            .collect()
    }

    #[test]
    fn an_unknown_id_is_not_ours() {
        let ledger = WheelLedger::default();
        let ids = ids(2);
        ledger.sent(ids[0]);
        assert!(!ledger.answered(ids[1]));
    }

    #[test]
    fn an_answered_id_is_forgotten() {
        let ledger = WheelLedger::default();
        let ids = ids(1);
        ledger.sent(ids[0]);
        assert!(ledger.answered(ids[0]));
        assert!(!ledger.answered(ids[0]));
    }

    #[test]
    fn the_ledger_keeps_the_newest_256() {
        let ledger = WheelLedger::default();
        let ids = ids(300);
        for id in &ids {
            ledger.sent(*id);
        }
        assert!(ids[..44].iter().all(|id| !ledger.answered(*id)));
        assert!(ids[44..].iter().all(|id| ledger.answered(*id)));
    }

    #[test]
    fn only_default_prevented_counts() {
        for bits in 0u8..8 {
            let result = InputEventResult::from_bits_truncate(bits);
            assert_eq!(
                wheel_prevented(result),
                bits & 1 != 0,
                "flags {bits:#05b}"
            );
        }
    }
}
