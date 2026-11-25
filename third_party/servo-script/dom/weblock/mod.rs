/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Web Locks: <https://w3c.github.io/web-locks/>
//!
//! Mercury: carried from servo/servo pull request #47981 "storages: implement Web Locks API"
//! (draft, by xbwwj, Xubai Wang), branch `weblocks` of xbwwj/servo, head commit a900121b
//! (`components/script/dom/weblock/`), MPL-2.0, with the `WorkerNavigator` exposure after
//! LunarWerxs `pr/web-locks` (commit 23583d753, L0garithmic, MPL-2.0). The lock state lives in
//! the storage thread (`servo-storage`, `weblocks/mod.rs`), so that windows and workers of one
//! origin share it across content processes. `lockmanager.rs` says what differs from the pull
//! request: it never released a lock, and never settled the promise of `request()`.

use storage_traits::weblocks::{LockInfoMsg, LockManagerSnapshotMsg, LockModeMsg};

use crate::conversions::Convert;
use crate::dom::bindings::codegen::Bindings::WebLockBinding::{
    LockInfo, LockManagerSnapshot, LockMode,
};

pub(crate) mod lock;
pub(crate) mod lockmanager;

impl Convert<LockManagerSnapshot> for LockManagerSnapshotMsg {
    fn convert(self) -> LockManagerSnapshot {
        let mut snapshot = LockManagerSnapshot::empty();
        snapshot.held = Some(self.held.into_iter().map(Convert::convert).collect());
        snapshot.pending = Some(self.pending.into_iter().map(Convert::convert).collect());
        snapshot
    }
}

impl Convert<LockInfo> for LockInfoMsg {
    fn convert(self) -> LockInfo {
        let mut lock_info = LockInfo::empty();
        lock_info.name = Some(self.name.into());
        lock_info.mode = Some(self.mode.convert());
        lock_info.clientId = Some(self.client_id.into());
        lock_info
    }
}

impl Convert<LockMode> for LockModeMsg {
    fn convert(self) -> LockMode {
        match self {
            LockModeMsg::Shared => LockMode::Shared,
            LockModeMsg::Exclusive => LockMode::Exclusive,
        }
    }
}

impl Convert<LockModeMsg> for LockMode {
    fn convert(self) -> LockModeMsg {
        match self {
            LockMode::Shared => LockModeMsg::Shared,
            LockMode::Exclusive => LockModeMsg::Exclusive,
        }
    }
}

/// Mercury: the options `request()` refuses, in the order of steps 5 to 8 of
/// <https://w3c.github.io/web-locks/#dom-lockmanager-request>. The message is the one of the
/// `NotSupportedError`. (The pull request tested "steal and ifAvailable" inside a branch that
/// only ran when both were set, so that its other two checks never ran.)
pub(crate) fn unsupported_request(
    name: &str,
    mode: LockMode,
    steal: bool,
    if_available: bool,
    has_signal: bool,
) -> Option<&'static str> {
    // Step 5. If name starts with U+002D HYPHEN-MINUS (-), then return a promise rejected with a
    // "NotSupportedError" DOMException.
    if name.starts_with('-') {
        return Some("the name of a lock may not start with U+002D HYPHEN-MINUS");
    }
    // Step 6. If both options’ steal dictionary member and options’ ifAvailable dictionary member
    // are true, then return a promise rejected with a "NotSupportedError" DOMException.
    if steal && if_available {
        return Some("steal and ifAvailable cannot both be true");
    }
    // Step 7. If options’ steal dictionary member is true and options’ mode dictionary member is
    // not "exclusive", then return a promise rejected with a "NotSupportedError" DOMException.
    if steal && mode != LockMode::Exclusive {
        return Some("steal is true but mode is not exclusive");
    }
    // Step 8. If options’ signal dictionary member is present, and either of options’ steal
    // dictionary member or options’ ifAvailable dictionary member is true, then return a promise
    // rejected with a "NotSupportedError" DOMException.
    if has_signal && (steal || if_available) {
        return Some("signal is present but steal or ifAvailable is true");
    }
    None
}

/// Mercury: what the callback of a lock request does with an event of the storage thread, which
/// it receives on the router thread (or on the storage thread itself in a single process).
#[derive(Debug, PartialEq)]
pub(crate) enum ClientAction {
    /// Queue a task for the event on the global's event loop.
    Deliver,
    /// Nothing: the event only asks whether the client is alive, and it is.
    Ignore,
    /// The global scope is closed (its window was discarded or its worker closed): tell the
    /// storage thread that the client is gone, so that it releases what the client holds.
    ReportGone,
}

pub(crate) fn client_action(event_is_probe: bool, global_closed: bool) -> ClientAction {
    match (event_is_probe, global_closed) {
        (_, true) => ClientAction::ReportGone,
        (true, false) => ClientAction::Ignore,
        (false, false) => ClientAction::Deliver,
    }
}

#[cfg(test)]
mod tests {
    //! No JS engine here: the checks that do not need one.

    use super::*;

    #[test]
    fn modes_convert_both_ways() {
        assert_eq!(LockModeMsg::Shared.convert(), LockMode::Shared);
        assert_eq!(LockModeMsg::Exclusive.convert(), LockMode::Exclusive);
        assert_eq!(LockMode::Shared.convert(), LockModeMsg::Shared);
        assert_eq!(LockMode::Exclusive.convert(), LockModeMsg::Exclusive);
    }

    #[test]
    fn a_plain_request_is_supported() {
        assert_eq!(
            unsupported_request("x", LockMode::Exclusive, false, false, false),
            None
        );
        assert_eq!(
            unsupported_request("x", LockMode::Shared, false, true, false),
            None
        );
        assert_eq!(
            unsupported_request("x", LockMode::Shared, false, false, true),
            None
        );
        assert_eq!(
            unsupported_request("x", LockMode::Exclusive, true, false, false),
            None
        );
    }

    #[test]
    fn a_name_that_starts_with_a_hyphen_is_not_supported() {
        assert!(unsupported_request("-x", LockMode::Exclusive, false, false, false).is_some());
        assert_eq!(
            unsupported_request("x-", LockMode::Exclusive, false, false, false),
            None
        );
        assert_eq!(
            unsupported_request("", LockMode::Exclusive, false, false, false),
            None,
            "the empty name is a name"
        );
    }

    #[test]
    fn steal_and_if_available_exclude_each_other() {
        assert!(unsupported_request("x", LockMode::Exclusive, true, true, false).is_some());
    }

    #[test]
    fn steal_needs_the_exclusive_mode() {
        assert!(unsupported_request("x", LockMode::Shared, true, false, false).is_some());
        assert_eq!(
            unsupported_request("x", LockMode::Exclusive, true, false, false),
            None
        );
    }

    #[test]
    fn a_signal_excludes_steal_and_if_available() {
        assert!(unsupported_request("x", LockMode::Exclusive, true, false, true).is_some());
        assert!(unsupported_request("x", LockMode::Exclusive, false, true, true).is_some());
        assert_eq!(
            unsupported_request("x", LockMode::Exclusive, false, false, true),
            None
        );
    }

    #[test]
    fn a_closed_global_is_reported_gone_whatever_the_event_is() {
        assert_eq!(client_action(true, true), ClientAction::ReportGone);
        assert_eq!(client_action(false, true), ClientAction::ReportGone);
    }

    #[test]
    fn a_probe_of_a_live_client_is_ignored_and_any_other_event_is_delivered() {
        assert_eq!(client_action(true, false), ClientAction::Ignore);
        assert_eq!(client_action(false, false), ClientAction::Deliver);
    }
}
