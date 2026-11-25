/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// https://w3c.github.io/web-locks/#api
//
// Mercury: carried from servo/servo pull request #47981 (draft, by xbwwj, Xubai Wang, branch
// `weblocks`, head commit a900121b, `components/script_bindings/webidls/WebLock.webidl`), MPL-2.0.
// The pull request has no `Pref`: here the interfaces and the `locks` attribute are behind
// `dom_web_locks_enabled` (Mercury keeps it off until `tests/pages/compat-locks.html` has run in a
// real window), `[SameObject]` is the specification's, and the two methods are `[NewObject]`.

[SecureContext]
interface mixin NavigatorLocks {
  [SameObject, Pref="dom_web_locks_enabled"] readonly attribute LockManager locks;
};
Navigator includes NavigatorLocks;
WorkerNavigator includes NavigatorLocks;

[SecureContext, Exposed=(Window,Worker), Pref="dom_web_locks_enabled"]
interface LockManager {
  [NewObject]
  Promise<any> request(DOMString name,
                       LockGrantedCallback callback);
  [NewObject]
  Promise<any> request(DOMString name,
                       LockOptions options,
                       LockGrantedCallback callback);

  [NewObject]
  Promise<LockManagerSnapshot> query();
};

callback LockGrantedCallback = Promise<any> (Lock? lock);

enum LockMode { "shared", "exclusive" };

dictionary LockOptions {
  LockMode mode = "exclusive";
  boolean ifAvailable = false;
  boolean steal = false;
  AbortSignal signal;
};

dictionary LockManagerSnapshot {
  sequence<LockInfo> held;
  sequence<LockInfo> pending;
};

dictionary LockInfo {
  DOMString name;
  LockMode mode;
  DOMString clientId;
};

[SecureContext, Exposed=(Window,Worker), Pref="dom_web_locks_enabled"]
interface Lock {
  readonly attribute DOMString name;
  readonly attribute LockMode mode;
};
