/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::cell::Cell;

use dom_struct::dom_struct;
use js::context::JSContext;
use js::realm::CurrentRealm;
use script_bindings::inheritance::Castable;
use script_bindings::reflector::reflect_dom_object;
use servo_constellation_traits::ScriptToConstellationMessage;

use crate::dom::bindings::codegen::Bindings::WakeLockBinding::{
    WakeLockSentinelMethods, WakeLockType,
};
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::DomRoot;
use crate::dom::bindings::str::DOMString;
use crate::dom::eventtarget::EventTarget;
use crate::dom::globalscope::GlobalScope;
use crate::dom::promise::{Promise, RootedPromise};

/// <https://w3c.github.io/screen-wake-lock/#the-wakelocksentinel-interface>
#[dom_struct]
pub(crate) struct WakeLockSentinel {
    eventtarget: EventTarget,
    released: Cell<bool>,
    type_: WakeLockType,
}

impl WakeLockSentinel {
    pub(crate) fn new_inherited(type_: WakeLockType) -> Self {
        Self {
            eventtarget: EventTarget::new_inherited(),
            released: Cell::new(false),
            type_,
        }
    }

    pub(crate) fn new(
        cx: &mut js::context::JSContext,
        global: &GlobalScope,
        type_: WakeLockType,
    ) -> DomRoot<Self> {
        reflect_dom_object(cx, Box::new(Self::new_inherited(type_)), global)
    }
}

// Mercury: the sentinel's release (H22), wired to the constellation's ReleaseWakeLock.
impl WakeLockSentinel {
    /// Releases the lock once: sends `ReleaseWakeLock` and fires `release`. A second call only
    /// resolves its promise.
    pub(crate) fn release_lock(&self, cx: &mut JSContext) {
        if self.released.replace(true) {
            return;
        }
        let global = self.global();
        global
            .as_window()
            .send_to_constellation(ScriptToConstellationMessage::ReleaseWakeLock(
                match self.type_ {
                    WakeLockType::Screen => embedder_traits::WakeLockType::Screen,
                },
            ));
        self.upcast::<EventTarget>()
            .fire_event(cx, DOMString::from("release").into());
    }
}

impl WakeLockSentinelMethods<crate::DomTypeHolder> for WakeLockSentinel {
    /// <https://w3c.github.io/screen-wake-lock/#dom-wakelocksentinel-released>
    fn Released(&self) -> bool {
        self.released.get()
    }

    /// <https://w3c.github.io/screen-wake-lock/#dom-wakelocksentinel-type>
    fn Type(&self) -> WakeLockType {
        self.type_
    }

    /// <https://w3c.github.io/screen-wake-lock/#dom-wakelocksentinel-release>
    fn Release(&self, cx: &mut CurrentRealm) -> RootedPromise {
        let promise = Promise::new_in_realm(cx);
        self.release_lock(cx);
        promise.resolve_native(cx, &());
        promise
    }

    event_handler!(release, GetOnrelease, SetOnrelease);
}
