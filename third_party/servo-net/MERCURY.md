# Patched servo-net 0.8.0

This directory vendors `servo-net` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`e274e25650db0dacd2a290d13f51effd12735c17d5448403c2c77fea530e034d`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/net`,
plus Mercury's changes in `third_party/patches/servo-net.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-net` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

One change: **certificate exceptions are bound to the pair (host name, certificate)**
and fail closed (R14 `EC27`; the site-security review's follow-up to C1). Everything new is in
`certificate_exceptions.rs` (new, PolyForm Noncommercial, copies nothing from Servo or another
project); the upstream files get hook lines only, each marked `// Mercury:`. The same change also keeps
the HSTS list from learning over a connection that only an exception made valid (below).

**What it does.** In Servo 0.8 a user decision ("continue anyway") adds the bare certificate to a
list that `verify_server_cert` accepts for **every** host name until the process ends, so whoever
holds the key of a certificate the user once accepted for `nas.local` can present it as
`bank.example` and the connection verifies. Now:

- the verifier accepts a certificate that fails normal verification only if the exact pair
  `(server name, end-entity certificate)` is in a `HashSet`; another host name, another certificate on
  the same host, a subdomain and a parent domain are all refused; the signature checks
  (`verify_tls12_signature`, `verify_tls13_signature`) stay with the main verifier, so the server must
  still hold the accepted certificate's key;
- a user decision becomes an exception only through `HttpState::accept_failed_certificate(host, der)`,
  which accepts only the certificate the host **actually failed with** (a second, bounded record of the
  last failure of each host, at most 64 hosts, oldest dropped, the newest failure of a host replaces
  the older one) and never for a host on the HSTS list (learned or preloaded). Refusals are
  `CertificateExceptionRefusal::{InvalidHost, HstsHost, NoFailureRecorded, CertificateChanged}` with the
  stable `code()` texts `invalid-host`, `hsts-host`, `no-failure-recorded`, `certificate-changed`. Eviction
  fails closed: the user reloads the page and gets a fresh record. A successful accept clears the host
  from both maps (the record and Servo's pending map);
- `server_name_for_host` is the one place that turns a host string into the name both maps and the
  exceptions use: it trims the brackets of an IPv6 literal, lower-cases, drops one trailing dot and
  parses with `ServerName::try_from`; anything else (an unbracketed IPv6 address, a second trailing
  dot, brackets around a non-address, non-ASCII) is `None`. `Uri::host()` keeps the brackets and
  `ServerName::try_from("[::1]")` fails, so before this change no certificate was reported for an IPv6
  literal and the page could not offer "Continue" there;
- `chrome:allowcert` (Servo's own entry point, a second way to add a bare certificate) is refused with a
  `warn!` and answers with the blank page it always answered with; `handle_allowcert_request` and the
  imports it needed are deleted;
- `HttpState::revoke_certificate_exception(host)` and `HttpState::certificate_exceptions()` exist
  (tested for new connections only). A new connection must make a full handshake: rustls does not call
  `verify_server_cert` when it resumes a TLS session, so a ticket stored while the exception was in force
  would let the host back in after a revoke (found by the revoke test: the first version of it failed with
  a `200`). `create_tls_config` therefore takes each client's session cache from the override manager
  (same size as rustls' default, one cache per `ClientConfig` as before), and a revoke empties the
  host's TLS 1.2 session and TLS 1.3 tickets. Revoking does **not** close keep-alive or HTTP/2
  connections that were opened under the exception, because hyper's pool has no per-host eviction
  (`create_http_client`: up to 6 idle connections per host), so Mercury shows no "Remove exception"
  button yet (`EC27b`).

- **no HSTS from a host that has had an exception.** Servo updates the HSTS list from every response over
  `https` or `wss`, including one over a connection that verified only through the user's exception
  (`http_loader.rs` and `websocket_loader.rs` each call `HstsList::update_hsts_list_from_response`). RFC 6797
  section 8.1 does not process the header of a response whose secure transport had an error, and two things
  went wrong when Mercury did: a self-signed device that sends `Strict-Transport-Security` (many routers and
  NAS pages do) became an HSTS host after one accepted visit, and because Mercury keeps `hsts_list.json`
  between sessions and clearing site data does not remove it, `accept_failed_certificate` then answered
  `hsts-host` for it from the next session on, so "Continue" was refused for good (until `max-age` ended);
  and whoever held the accepted certificate's key could plant a persistent HSTS entry for the name (with
  `includeSubDomains`, for its subdomains too). `HttpState::learn_hsts_from_response(url, headers)` is now the
  only place that updates the list from a response, and it does nothing for a host that has had an exception
  in this process (`HttpState::may_learn_hsts_from`, one `overridden` set of names in the store, filled by
  every way an exception is added and **not** emptied by a revoke, because a connection opened under the
  exception can outlive it). The rule is by name and not by connection: the connection a response came over
  is not known to the code that learns HSTS (and a WebSocket handshake has no record at all), and a rule by
  name fails closed. Its price: a name that had an exception does not learn its own HSTS policy for the rest
  of the process, even over a later connection that verified normally; the next process learns it. The name is
  the one `accept_failed_certificate` uses (`server_name_for_host`), so `NAS.LOCAL.` and `nas.local` are one.
  A neighbouring name is not affected. `tls_security_info.hsts` of a response (a flag in Servo's page-info
  record, which Mercury does not read) still reports the header; nothing is stored from it.

The key has no port and no error kind: rustls hands the verifier a name, not a port, and the error
kind adds nothing against someone who holds the accepted certificate's key. Chrome keys by host and
certificate too. The exception lasts until the process ends. A host that becomes HSTS **after** an
exception exists keeps that exception for that exact certificate: the refusal is at acceptance.

**Why Mercury needs it.** The release blocker of `docs/developer-guide/releases.md` ("Before the first
public release", item 6): Mercury's certificate page ("Continue anyway", local names only) called the
bare-certificate override, so one click exposed every site to the holder of that certificate's key, with
the normal padlock. Servo PR 36946 only moves the `chrome:allowcert` transport and still pushes a bare
certificate, so it is not carried.

**Upstream files and lines** (servo-net 0.8.0 as published; Mercury's lines in
`third_party/patches/servo-net.patch`):

| File | Upstream lines | Hook |
| --- | --- | --- |
| `certificate_exceptions.rs` | new file | The store (`CertificateExceptionStore`), `server_name_for_host`, `CertificateExceptionRefusal`, `CertificateExceptionEntry`, `CertificateErrorOverrideManager::add_override(host, certificate)`, `is_accepted` and `session_resumption`, and the three `HttpState` methods. Behind the `test-util` feature: `has_exception`, `note_certificate_failure_for_test`, `report_certificate_failure_for_test` (the tests cannot reach the verifier). |
| `lib.rs` | 8 | One `pub mod certificate_exceptions;` line. |
| `http_loader.rs` | 2483-2485 | The HSTS update after a response, `hsts_list.write().update_hsts_list_from_response(&url, &response.headers)`, becomes `state.learn_hsts_from_response(&url, &response.headers)` (one comment line and the call). |
| `websocket_loader.rs` | 157-159 | The same in `process_ws_response`. |
| `connector.rs` | 303-310 | `CertificateErrorOverrideManagerInternal` is `pub(crate)` with `pub(crate)` fields, and `overrides: Vec<CertificateDer>` becomes `exceptions: CertificateExceptionStore`. |
| `connector.rs` | 324-328 | The one-argument `add_override(&cert)` is removed (a stale caller does not compile) and a `pub(crate) lock()` accessor is added for the new file. |
| `connector.rs` | 365-381 | `create_tls_config` sets `config.resumption` to the manager's session cache (see the revoke note above). |
| `connector.rs` | 333-348 | `remove_certificate_failing_verification` keeps remove-on-report (an entry that is left behind would turn a later unrelated network error into a certificate error), uses `server_name_for_host`, and copies the removed certificate into the bounded record. |
| `connector.rs` | 561-566 | `verify_server_cert` asks `is_accepted(server_name, end_entity)` instead of looping over bare certificates. |
| `connector.rs` | 7 | The `use std::convert::TryFrom;` that the removed `ServerName::try_from(host)` made the only user of. |
| `fetch/methods.rs` | 7, 9-10, 17, 25-29, 40 | The imports that only `handle_allowcert_request` used. |
| `fetch/methods.rs` | 1017-1078 | `handle_allowcert_request` is deleted. |
| `fetch/methods.rs` | 1101-1106 | The `chrome:allowcert` arm refuses with a `warn!` and answers the blank page. |
| `tests/main.rs` | 7 | `mod certificate_exceptions;`. |
| `tests/fetch.rs` | 798, 945 | `add_override(certificate)` becomes `add_override("localhost", certificate).expect(..)`. |
| `tests/fetch.rs` | 840, 858-862 | `test_load_adds_host_to_hsts_list_when_url_is_https` reached its self-signed server with an exception and expected the HSTS list to learn the host, which is the behaviour this patch removes (found when the guard was added: the test failed). It now uses `state_that_ignores_certificate_errors()` (a state whose client does not verify, defined in `tests/certificate_exceptions.rs`), so the connection needs no exception and the test still checks what it is named for. |
| `tests/http_loader.rs` | 2012 | The same. |
| `tests/certificate_exceptions.rs` | new file | The tests below. |

**How it is tested.** `tests/certificate_exceptions.rs` (PolyForm Noncommercial, all Rust):
key semantics; the host-form table (the same rows as `HOST_FORMS` in
`crates/talaria-core/tests/certificate_exceptions.rs`, case, trailing dots, `[::1]`, `127.0.0.1`,
garbage); every refusal and the success; learned and preloaded HSTS; the record's bound, its
replacement and refresh rules; both maps cleared on accept; revoke and listing (SHA-256 pinned with the
FIPS 180-2 vector for `abc`); `chrome:allowcert` with the real secret and body adds nothing; and, with the
loopback TLS server of `test_util` (self-signed, one certificate under `localhost`, `127.0.0.1` and
`[::1]`): a failed load carries the server's certificate, after `accept_failed_certificate` the same host
loads, **the other name for the same server still fails**, a revoke makes a new connection fail again (this needs the session-cache eviction above),
another certificate for the same host does not help, and an IPv6 literal gets a certificate error and can
be accepted. The HSTS guard has eight tests: with the same loopback server answering
`Strict-Transport-Security`, the list learns the host from a state whose client verifies nothing (the
control: the header does reach the list through the fetch path) and does **not** learn it over a connection
that only an exception made valid, and after a revoke and a new failure the user can accept again (the
next session reads the same list, so "Continue" is not refused for good); without a network, that a host
with an exception (in every spelling: case, a trailing dot, an IPv6 literal) and a revoked one teach nothing
while a neighbouring name, a subdomain and a parent do, that a user's decision blocks the host and a
refused one does not, and a scan of the crate's sources that fails if `update_hsts_list_from_response` is
called anywhere but the guard, or if the HTTP fetch or the WebSocket handshake stops calling the guard
(a Servo upgrade or a merge could bring the old call back). 30 tests in all; none touches a real trust store or the network beyond loopback. The test "the same certificate on another host name still fails" was written first and run against the unpatched crate with
the old global `add_override(&cert)`: it failed (the second name loaded, `get_network_error()` was
`None`), and its patched counterpart passes. The HSTS e2e test was written first too and failed against
the crate without the guard (`the list learned a host from a connection that only the user's exception made
valid`, with its control passing); with the two hook lines of `http_loader.rs` and `websocket_loader.rs`
put back to Servo's own call, the e2e test and the source scan fail again.

**Drop when.** Servo binds certificate overrides to a host (and refuses HSTS hosts) itself. Then delete
`certificate_exceptions.rs`, restore the `connector.rs` and `fetch/methods.rs` hunks, and keep only what
Servo's own API cannot express (the recorded failure).

## Running the tests

`servo-net` is a vendored dependency with dev-dependencies, so `cargo test -p servo-net` from the
workspace root does not work (cargo refuses a package that is not a workspace member). The root `Cargo.toml`
lists `third_party/servo-net` under `exclude`, as it does `third_party/gaol`, so cargo accepts the crate's own
manifest, and `make test-servo-net` runs the crate's whole `main` target on its own `Cargo.lock`, against the
published sibling crates (the same code as Mercury's copies, because only `servo-net` changes; not
Mercury's vendored copies and not Mercury's `Cargo.lock`: of the crates it links against, `servo-net` is the only one
Mercury changes in a way that matters to it; the others are byte-identical to the published ones, except
`servo-default-resources`, which Mercury patches only to stop it registering the default resource reader, and
the tests need that reader registered). `make test` runs it, so do the full gates, and the
CI job "Vendored servo-net tests" does. Two tests fail on the unpatched crate as well and are skipped by
name: `fetch::test_file` (it reads `../../components/net/tests/test.css`, which exists only in Servo's
repository) and `filemanager_thread::test_filemanager` (it panics on the process-wide async runtime when it
runs with the others). To run only this change's tests, from the repository root (the tests need the
`devtools` feature as well as `test-util`):

```bash
MERCURY_SLOT_POOL=light ~/mercury-browser/.superpowers/tools/guarded.sh "$PWD" \
  "RUST_TEST_THREADS=2 cargo test --locked --release --target-dir target \
   --manifest-path third_party/servo-net/Cargo.toml \
   --features test-util,devtools --test main certificate_exceptions"
```

The first run builds the published dependencies (about 11 minutes on a 12-thread Intel Mac, a few
hundred MB under `target/`; `--target-dir` keeps the build out of the vendored crate). After an
edit, format only the files you changed with the command in
`docs/developer-guide/vendored-patches.md` ("No formatting churn"), run `export` and
`make servo-patches-check`.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-net.patch` and this directory
once Mercury no longer needs to change the crate.
