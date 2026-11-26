// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The system's power state behind a trait (energy-1 lane): Low Power Mode today, and the
//! place the Energy saver (energy-2) reads the rest from.
//!
//! The macOS source (`NSProcessInfo`) is only created in an interactive launch. Automated runs
//! and other platforms get a constant source that says "off" and never calls back, and tests
//! use `FakePowerSource`, so no test or screenshot run asks the system anything.

#[cfg(target_os = "macos")]
mod macos;

use crate::media_session::ServiceKind;

/// Called after the power state changed, on whichever thread the system reports it from.
pub(crate) type ChangeListener = Box<dyn Fn() + Send + Sync>;

pub(crate) trait PowerSource {
    fn kind(&self) -> ServiceKind;
    /// Cheap (an atomic read): called on every engine pump.
    fn low_power_mode(&self) -> bool;
    /// Replaces the listener. It is called after `low_power_mode` has changed.
    fn on_change(&mut self, listener: ChangeListener);
}

/// A source whose state never changes: automated runs and platforms without the service.
pub(crate) struct ConstantPowerSource(pub(crate) bool);

impl PowerSource for ConstantPowerSource {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Logging
    }

    fn low_power_mode(&self) -> bool {
        self.0
    }

    fn on_change(&mut self, _listener: ChangeListener) {}
}

/// `interactive` comes from `media_session::uses_system_services`: only an interactive launch
/// on macOS asks the system.
pub(crate) fn power_source(interactive: bool) -> Box<dyn PowerSource> {
    #[cfg(target_os = "macos")]
    if interactive {
        let source = macos::SystemPowerSource::new();
        log::info!(
            "power: {:?} source, Low Power Mode {}",
            source.kind(),
            if source.low_power_mode() { "on" } else { "off" }
        );
        return Box::new(source);
    }
    let _ = interactive;
    Box::new(ConstantPowerSource(false))
}

#[cfg(test)]
pub(crate) use fake::FakePowerSource;

#[cfg(test)]
mod fake {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use super::{ChangeListener, PowerSource};
    use crate::media_session::ServiceKind;

    #[derive(Default)]
    struct State {
        low_power: AtomicBool,
        listener: Mutex<Option<ChangeListener>>,
    }

    /// A power source the test drives. Clones share their state, so a test keeps one and hands
    /// the other to the code under test.
    #[derive(Clone, Default)]
    pub(crate) struct FakePowerSource {
        state: Arc<State>,
    }

    impl FakePowerSource {
        pub(crate) fn set_low_power(&self, on: bool) {
            let changed = self.state.low_power.swap(on, Ordering::SeqCst) != on;
            if changed && let Some(listener) = self.state.listener.lock().unwrap().as_ref() {
                listener();
            }
        }
    }

    impl PowerSource for FakePowerSource {
        fn kind(&self) -> ServiceKind {
            ServiceKind::Test
        }

        fn low_power_mode(&self) -> bool {
            self.state.low_power.load(Ordering::SeqCst)
        }

        fn on_change(&mut self, listener: ChangeListener) {
            *self.state.listener.lock().unwrap() = Some(listener);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn counting_listener() -> (Arc<AtomicUsize>, ChangeListener) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        (
            calls,
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        )
    }

    #[test]
    fn the_fake_follows_what_is_set_and_calls_the_listener_after_each_change() {
        let handle = FakePowerSource::default();
        let mut source = handle.clone();
        let (calls, listener) = counting_listener();
        source.on_change(listener);
        assert!(!source.low_power_mode());

        handle.set_low_power(true);
        assert!(source.low_power_mode());
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        handle.set_low_power(true);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no change, no call");

        handle.set_low_power(false);
        assert!(!source.low_power_mode());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_change_with_no_listener_is_harmless() {
        let handle = FakePowerSource::default();

        handle.set_low_power(true);

        assert!(handle.low_power_mode());
    }

    #[test]
    fn the_listener_sees_the_new_state_when_it_is_called() {
        let handle = FakePowerSource::default();
        let mut source = handle.clone();
        let seen = Arc::new(AtomicUsize::new(0));
        let (observed, probe) = (Arc::clone(&seen), handle.clone());
        source.on_change(Box::new(move || {
            observed.store(usize::from(probe.low_power_mode()) + 1, Ordering::SeqCst);
        }));

        handle.set_low_power(true);

        assert_eq!(
            seen.load(Ordering::SeqCst),
            2,
            "on, as seen from the listener"
        );
    }

    #[test]
    fn the_constant_source_never_changes_and_never_calls_its_listener() {
        let mut off = ConstantPowerSource(false);
        let mut on = ConstantPowerSource(true);
        let (calls, listener) = counting_listener();
        off.on_change(listener);
        on.on_change(Box::new(|| {}));

        assert!(!off.low_power_mode());
        assert!(on.low_power_mode());
        assert_eq!(off.kind(), ServiceKind::Logging);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn an_automated_launch_gets_the_constant_source() {
        let source = power_source(false);

        assert_eq!(source.kind(), ServiceKind::Logging);
        assert!(!source.low_power_mode());
    }

    /// Reads the real system once; by hand only (`-- --ignored`), see deferred-hands-on.md.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "asks the system; run by hand"]
    fn the_real_source_reads_low_power_mode_without_panicking() {
        let mut source = power_source(true);
        let (_calls, listener) = counting_listener();
        source.on_change(listener);

        assert_eq!(source.kind(), ServiceKind::System);
        let _ = source.low_power_mode();
    }
}
