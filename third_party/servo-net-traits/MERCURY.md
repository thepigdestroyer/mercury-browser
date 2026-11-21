# Patched servo-net-traits 0.8.0

This directory vendors `servo-net-traits` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`80e6ac24573425319af0ac9dbc3ec7eec925b9da656902a15815506fd5e3e417`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/shared/net`,
plus Mercury's changes in `third_party/patches/servo-net-traits.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-net-traits` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane net-1 (21 Nov 2026; third-party cookies and the request path).

### `document.cookie` carries the document's site — `lib.rs`

- **What:** two new `CoreResourceMsg` variants after `GetCookieStringForUrl` (0.8.0 line about 812):
  `GetCookieStringForDocument(ServoUrl, NetworkPartitionKey, Option<WebViewId>, GenericSender<Option<String>>, CookieSource)`
  and `SetCookiesForDocument(ServoUrl, NetworkPartitionKey, Option<WebViewId>, Vec<Serde<Cookie<'static>>>, CookieSource)`.
  The old variants stay, unfiltered, for WebDriver and the tests.
- **Why:** the resource thread has to know which top-level site a document belongs to before it can block a third-party
  `document.cookie` (EC5, EC6). The old messages carry the URL only.
- **Tested by:** `servo-net/tests/cookie_policy.rs` (blocked and allowed document reads and writes, over the real
  resource thread); `servo-script` sends them from `Document::GetCookie` and `SetCookie`.
- **Drop when:** Servo filters `document.cookie` by site itself (PR #44469 is the only upstream work on it and does not
  cover it).

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-net-traits.patch` and this directory
once Mercury no longer needs to change the crate.
