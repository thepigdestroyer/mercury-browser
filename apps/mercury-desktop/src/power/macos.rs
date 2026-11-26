// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Low Power Mode from `NSProcessInfo`: `isLowPowerModeEnabled`, and the
//! `NSProcessInfoPowerStateDidChangeNotification` the system posts when it changes. Only
//! created in an interactive launch (see `power_source`); nothing here runs in tests (the one
//! test that reads the real system is `#[ignore]`d).

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{msg_send, sel};
use objc2_foundation::NSString;

use super::{ChangeListener, PowerSource};
use crate::media_session::ServiceKind;

/// The notification's value is its own name (`NSProcessInfoPowerStateDidChangeNotification`).
const POWER_STATE_CHANGED: &str = "NSProcessInfoPowerStateDidChangeNotification";

/// What the observer block shares with the source. The block runs on the thread that posts the
/// notification, so it touches only an atomic and the `Send + Sync` listener.
struct Shared {
    low_power: AtomicBool,
    listener: Mutex<Option<ChangeListener>>,
}

impl Shared {
    fn refresh(&self) {
        let now = read_low_power_mode();
        let before = self.low_power.swap(now, Ordering::SeqCst);
        if now != before
            && let Ok(listener) = self.listener.lock()
            && let Some(listener) = listener.as_ref()
        {
            listener();
        }
    }
}

pub(super) struct SystemPowerSource {
    shared: Arc<Shared>,
    /// What `-addObserverForName:...` returned; removed on drop.
    observer: Option<Retained<AnyObject>>,
}

impl SystemPowerSource {
    pub(super) fn new() -> Self {
        let shared = Arc::new(Shared {
            low_power: AtomicBool::new(read_low_power_mode()),
            listener: Mutex::new(None),
        });
        let observer = observe_power_state(&shared);
        Self { shared, observer }
    }
}

impl PowerSource for SystemPowerSource {
    fn kind(&self) -> ServiceKind {
        ServiceKind::System
    }

    fn low_power_mode(&self) -> bool {
        self.shared.low_power.load(Ordering::SeqCst)
    }

    fn on_change(&mut self, listener: ChangeListener) {
        if let Ok(mut slot) = self.shared.listener.lock() {
            *slot = Some(listener);
        }
    }
}

impl Drop for SystemPowerSource {
    fn drop(&mut self) {
        let Some(observer) = self.observer.take() else {
            return;
        };
        let Some(center) = default_center() else {
            return;
        };
        // SAFETY: `observer` was returned by `-addObserverForName:object:queue:usingBlock:` on
        // this centre; `-removeObserver:` takes it.
        unsafe {
            let () = msg_send![&*center, removeObserver: &*observer];
        }
    }
}

fn read_low_power_mode() -> bool {
    let Some(class) = AnyClass::get(c"NSProcessInfo") else {
        return false;
    };
    // SAFETY: `+processInfo` takes no arguments and returns the shared process info.
    let info: Option<Retained<AnyObject>> = unsafe { msg_send![class, processInfo] };
    let Some(info) = info else {
        return false;
    };
    // `isLowPowerModeEnabled` exists from macOS 12; before that there is no Low Power Mode.
    // SAFETY: `-respondsToSelector:` takes a selector and returns a BOOL.
    let responds: Bool =
        unsafe { msg_send![&*info, respondsToSelector: sel!(isLowPowerModeEnabled)] };
    if !responds.as_bool() {
        return false;
    }
    // SAFETY: `-isLowPowerModeEnabled` takes no arguments and returns a BOOL.
    let enabled: Bool = unsafe { msg_send![&*info, isLowPowerModeEnabled] };
    enabled.as_bool()
}

fn default_center() -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"NSNotificationCenter")?;
    // SAFETY: `+defaultCenter` takes no arguments and returns the shared centre.
    unsafe { msg_send![class, defaultCenter] }
}

fn observe_power_state(shared: &Arc<Shared>) -> Option<Retained<AnyObject>> {
    let center = default_center()?;
    let name = NSString::from_str(POWER_STATE_CHANGED);
    let shared = Arc::clone(shared);
    // The block hands the change on; it captures nothing tied to a thread.
    let block = RcBlock::new(move |_notification: NonNull<AnyObject>| shared.refresh());
    // SAFETY: `-addObserverForName:object:queue:usingBlock:` takes a notification name, an
    // optional sender, an optional queue (nil: the block runs on the posting thread) and a block
    // `void (^)(NSNotification *)`, and returns the observer to remove later; the centre
    // copies the block.
    unsafe {
        msg_send![
            &*center,
            addObserverForName: &*name,
            object: None::<&AnyObject>,
            queue: None::<&AnyObject>,
            usingBlock: &*block
        ]
    }
}
