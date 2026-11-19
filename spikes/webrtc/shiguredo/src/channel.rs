// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A data channel with an inbox of the messages it received.
//!
//! The wrapper offers `send`, `state`, `label`, `close` and an observer with two callbacks
//! (`on_state_change` without the state, `on_message`). It has no `id`, `protocol`, `ordered`,
//! `negotiated`, `max_retransmits` or `buffered_amount` getter, and `DataChannelInit` has only
//! `ordered` and `protocol` setters: those are missing from the C API, not just from the Rust
//! wrapper (see the trait-fit table of the report).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use shiguredo_webrtc::{
    DataChannel, DataChannelInit, DataChannelObserver, DataChannelObserverHandler, DataChannelState,
};

use crate::Result;

/// One received message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The payload.
    pub data: Vec<u8>,
    /// True for a binary message, false for a text message.
    pub binary: bool,
}

#[derive(Default)]
struct Inbox {
    messages: Mutex<Vec<Message>>,
    arrived: Condvar,
    state_changes: AtomicU64,
}

impl Inbox {
    fn lock(&self) -> MutexGuard<'_, Vec<Message>> {
        self.messages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct Handler(Arc<Inbox>);

impl DataChannelObserverHandler for Handler {
    fn on_state_change(&mut self) {
        self.0.state_changes.fetch_add(1, Ordering::Relaxed);
        self.0.arrived.notify_all();
    }

    fn on_message(&mut self, data: &[u8], is_binary: bool) {
        self.0.lock().push(Message {
            data: data.to_vec(),
            binary: is_binary,
        });
        self.0.arrived.notify_all();
    }
}

/// What can be set when a channel is created (all the wrapper offers).
#[derive(Debug, Clone, Default)]
pub struct ChannelOptions {
    /// `ordered = false` for an unordered channel.
    pub unordered: bool,
    /// The sub-protocol name.
    pub protocol: Option<String>,
}

impl ChannelOptions {
    pub(crate) fn to_init(&self) -> DataChannelInit {
        let mut init = DataChannelInit::new();
        init.set_ordered(!self.unordered);
        if let Some(protocol) = &self.protocol {
            init.set_protocol(protocol);
        }
        init
    }
}

/// A data channel (local or remote) with its inbox.
pub struct Channel {
    channel: DataChannel,
    inbox: Arc<Inbox>,
    observer: DataChannelObserver,
}

impl Channel {
    /// Registers an observer on `channel`. Messages that arrived before this call are delivered by
    /// libwebrtc when the observer is registered.
    pub fn attach(channel: DataChannel) -> Channel {
        let inbox = Arc::new(Inbox::default());
        let observer = DataChannelObserver::new_with_handler(Box::new(Handler(Arc::clone(&inbox))));
        channel.register_observer(&observer);
        Channel {
            channel,
            inbox,
            observer,
        }
    }

    /// The label the channel was created with (the remote side sees the creator's label).
    pub fn label(&self) -> Result<String> {
        self.channel.label()
    }

    /// The channel state.
    pub fn state(&self) -> DataChannelState {
        self.channel.state()
    }

    /// Sends a text message; false when libwebrtc refused it.
    pub fn send_text(&self, text: &str) -> bool {
        self.channel.send(text.as_bytes(), false)
    }

    /// Sends a binary message; false when libwebrtc refused it.
    pub fn send_binary(&self, data: &[u8]) -> bool {
        self.channel.send(data, true)
    }

    /// Closes the channel.
    pub fn close(&self) {
        self.channel.close();
    }

    /// Times the state changed (observer callbacks).
    pub fn state_changes(&self) -> u64 {
        self.inbox.state_changes.load(Ordering::Relaxed)
    }

    /// Waits until the channel is `Open`.
    pub fn wait_open(&self, timeout: Duration) -> bool {
        self.wait_state(timeout, |state| state == DataChannelState::Open)
    }

    /// Waits until the channel state satisfies `wanted` (polling every 5 ms).
    pub fn wait_state(&self, timeout: Duration, wanted: impl Fn(DataChannelState) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if wanted(self.state()) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The next message, or `None` after `timeout`.
    pub fn recv(&self, timeout: Duration) -> Option<Message> {
        let deadline = Instant::now() + timeout;
        let mut messages = self.inbox.lock();
        loop {
            if !messages.is_empty() {
                return Some(messages.remove(0));
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            messages = self
                .inbox
                .arrived
                .wait_timeout(messages, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Up to `count` messages, in arrival order, stopping early when `timeout` has passed.
    pub fn recv_many(&self, count: usize, timeout: Duration) -> Vec<Message> {
        let deadline = Instant::now() + timeout;
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.recv(left) {
                Some(message) => out.push(message),
                None => break,
            }
        }
        out
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        // libwebrtc keeps a raw pointer to the observer until it is unregistered.
        self.channel.unregister_observer();
        let _ = &self.observer;
    }
}
