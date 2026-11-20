/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: carried from servo/servo pull request #47981 (draft, by xbwwj, Xubai Wang), branch
// `weblocks` of xbwwj/servo, head commit a900121b (`components/script/dom/weblock/lock.rs`),
// MPL-2.0. Changed for Servo 0.8's bindings: the name is a `String` that is not traced, the
// object is made with `reflect_dom_object`, and the lock's id is not kept (the manager that
// granted it knows it).

use dom_struct::dom_struct;
use js::context::JSContext;
use script_bindings::reflector::{Reflector, reflect_dom_object};
use storage_traits::weblocks::LockMsg;

use crate::conversions::Convert;
use crate::dom::bindings::codegen::Bindings::WebLockBinding::{LockMethods, LockMode};
use crate::dom::bindings::root::DomRoot;
use crate::dom::bindings::str::DOMString;
use crate::dom::globalscope::GlobalScope;

/// <https://w3c.github.io/web-locks/#api-lock>
///
/// Handed to the callback of a granted request. It is a receipt, not a handle: the lock is
/// released when the promise the callback returned settles, so there is nothing to call on it.
#[dom_struct]
pub(crate) struct Lock {
    reflector_: Reflector,
    #[no_trace]
    name: String,
    mode: LockMode,
}

impl Lock {
    pub(crate) fn new_inherited(msg: LockMsg) -> Self {
        Lock {
            reflector_: Reflector::new(),
            name: msg.name,
            mode: msg.mode.convert(),
        }
    }

    pub(crate) fn new(cx: &mut JSContext, global: &GlobalScope, msg: LockMsg) -> DomRoot<Self> {
        reflect_dom_object(cx, Box::new(Self::new_inherited(msg)), global)
    }
}

impl LockMethods<crate::DomTypeHolder> for Lock {
    /// <https://w3c.github.io/web-locks/#dom-lock-name>
    fn Name(&self) -> DOMString {
        DOMString::from(self.name.clone())
    }

    /// <https://w3c.github.io/web-locks/#dom-lock-mode>
    fn Mode(&self) -> LockMode {
        self.mode
    }
}
