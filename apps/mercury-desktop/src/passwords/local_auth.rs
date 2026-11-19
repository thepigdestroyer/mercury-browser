// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Touch ID or the login password before a saved password is shown, copied or exported:
//! `LAContext.evaluatePolicy(.deviceOwnerAuthentication)` from Apple's LocalAuthentication
//! framework, called through the Objective-C runtime (the framework needs no entitlement).
//!
//! Only normal interactive runs use this; tests and automated runs use a fake that never asks.

use std::cell::RefCell;
use std::collections::HashMap;

use block2::RcBlock;
use mercury_password_manager::{AuthOutcome, Authenticator};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_foundation::NSString;

#[link(name = "LocalAuthentication", kind = "framework")]
unsafe extern "C" {}

/// `LAPolicyDeviceOwnerAuthentication`: biometrics, or the user's password when they are not
/// available or the user chooses it.
const DEVICE_OWNER_AUTHENTICATION: isize = 2;

type Done = Box<dyn FnOnce(AuthOutcome)>;

thread_local! {
    /// Authentications in progress: the context (kept alive until the reply) and who waits.
    static WAITING: RefCell<HashMap<u64, (Retained<AnyObject>, Done)>> =
        RefCell::new(HashMap::new());
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub(crate) struct LocalAuthentication;

impl Authenticator for LocalAuthentication {
    fn authenticate(&self, reason: &str, done: Done) {
        let Some(class) = AnyClass::get(c"LAContext") else {
            log::warn!("passwords: LocalAuthentication is not available");
            done(AuthOutcome::Unavailable);
            return;
        };
        // SAFETY: `+new` returns a retained `LAContext`, an `NSObject` subclass.
        let context: Retained<AnyObject> = unsafe { msg_send![class, new] };
        let mut error: *mut AnyObject = std::ptr::null_mut();
        // SAFETY: `canEvaluatePolicy:error:` takes an `LAPolicy` (NSInteger) and an
        // `NSError **` that it may fill; the error is not used.
        let can: Bool = unsafe {
            msg_send![&*context, canEvaluatePolicy: DEVICE_OWNER_AUTHENTICATION, error: &mut error]
        };
        if !can.as_bool() {
            log::info!("passwords: this Mac cannot authenticate its user");
            done(AuthOutcome::Unavailable);
            return;
        }

        let id = NEXT_ID.with(|next| {
            next.set(next.get() + 1);
            next.get()
        });
        WAITING.with(|waiting| {
            waiting.borrow_mut().insert(id, (context.clone(), done));
        });
        // Called once, on a private queue of the framework; the answer goes to the UI thread.
        let reply = RcBlock::new(move |success: Bool, _error: *mut AnyObject| {
            let outcome = if success.as_bool() {
                AuthOutcome::Allowed
            } else {
                AuthOutcome::Denied
            };
            let _queued = slint::invoke_from_event_loop(move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                let waiting = WAITING.with(|waiting| waiting.borrow_mut().remove(&id));
                if let Some((_context, done)) = waiting {
                    done(outcome);
                }
            });
        });
        let reason = NSString::from_str(reason);
        // SAFETY: `evaluatePolicy:localizedReason:reply:` takes an `LAPolicy`, an `NSString`
        // and a block `void (^)(BOOL, NSError *)`, which it copies.
        let () = unsafe {
            msg_send![
                &*context,
                evaluatePolicy: DEVICE_OWNER_AUTHENTICATION,
                localizedReason: &*reason,
                reply: &*reply
            ]
        };
    }
}
