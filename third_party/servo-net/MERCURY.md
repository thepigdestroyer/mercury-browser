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

Changes made by lane K0 (engine base, 19 Nov 2026).

### 1. HTTP/2 header list of 64 KiB (YouTube) — `connector.rs`

- **What:** backport of servo/servo#49981 (merge commit `552c43da1`). `create_http_client` sets
  `http2_max_header_list_size(64 * 1024)` on the hyper client builder (new constant `HTTP2_MAX_HEADER_LIST_SIZE`;
  0.8.0 lines about 45 and 670).
- **Why:** hyper's default of 16 KiB is too small for YouTube, whose responses fail with "client error (SendRequest)".
- **Tested by:** nothing automatic (it needs a server that sends a header list over 16 KiB); check with a YouTube watch page.
- **Drop when:** the Servo bump that contains PR #49981.

### 2. Cookie and credential values out of the logs (EC13) — `cookie_storage.rs`, `cookie.rs`, `http_loader.rs`, `lib.rs`, new `mercury_log.rs`

- **What:** Mercury's own change, not a backport. (a) `CookieStorage::cookies_for_url` (0.8.0 line about 243) logged the
  whole cookie string at `info`; it now logs how many cookies were sent. (b) `ServoCookie::appropriate_for_url` (line about
  339) logged the cookie's value at `debug`; it now logs name, domain and path. (c) `http_network_fetch` (lines about
  2188 and 2284) logged every request and response header at `debug`, and `cors_preflight_fetch` (line about 2638)
  logged the whole request header map; they now go through `header_for_log` and `headers_for_log` in `mercury_log.rs`,
  which replace the value of `Cookie`, `Set-Cookie`, `Authorization` and `Proxy-Authorization` (case-insensitively) with
  `"<redacted>"` and print other headers as before.
  (d) Train-22 security review, finding 5: the URLs that `http_loader.rs` logged (`http_network_fetch`, lines about 1976,
  2188, 2285, 2336, 2418, 2447, 2453) and that `websocket_loader.rs` logged (lines about 129, 311, 363, 366) now go through
  `url_for_log` in `mercury_log.rs`: scheme, host, port and path, with no user name, password, query string or fragment
  (a `data:`, `blob:` or `javascript:` URL shows its scheme only). The WebSocket DOM actions, DOM messages and stream
  messages (`trace`, lines about 178, 217, 241) print only the names of their variants through `variant_for_log`, never the
  message body.
- **Why:** session cookies and credentials reached the log at `info` and `debug`; query strings (tokens) and
  `user:password@` URLs reached it at `info`, and WebSocket message bodies at `trace`. Not changed: other URL logs outside
  these two files (the `fetch/` and `connector.rs` ones are at `debug`/`trace` and print what the page asked for; a
  follow-up if the log policy wants them redacted too).
- **Tested by:** the unit tests in `mercury_log.rs` (the log helpers' tests and the SVG ones below), run by `make test-servo-net`: `tests/mercury_unit.rs` includes the two files with `#[path]` into the `main` test target, because the library has `test = false`. They are: redaction of each
  name and of mixed case, other headers unchanged, an empty value, a full map with two `Set-Cookie` entries; and
  `include_str!` guards: `the_cookie_files_do_not_log_a_cookie_value` fails if a logging call in `cookie_storage.rs` or
  `cookie.rs` logs anything but the cookie's name, domain, path or the sent count;
  `the_http_loader_logs_headers_only_through_the_redacting_helpers` fails if `http_loader.rs` logs a header or header map
  outside the two helpers; `the_guard_finds_the_lines_servo_used_to_have` proves the guard flags Servo's old lines. For (d):
  `a_url_is_shown_without_credentials_query_and_fragment`, `a_url_that_carries_its_content_shows_only_its_scheme`,
  `a_message_is_shown_by_its_variant_not_its_content`, and the guard
  `the_http_and_websocket_loaders_log_urls_and_messages_only_through_the_helpers` (with
  `the_url_guard_finds_the_lines_servo_used_to_have`).
- **Drop when:** Servo no longer logs these values; then delete `mercury_log.rs` and its call sites.

### 3. SVG nesting guard (decision D8) — `image_cache.rs`, `lib.rs`, new `mercury_svg.rs`

- **What:** Mercury's replacement for servo/servo#49835, which sets `usvg::Options::max_depth` to 500 (an option that exists from
  usvg 0.50; the tree has 0.48.1, and moving resvg also touches the non-vendored `servo-malloc-size-of`). Four parts:
  1. `parse_svg_document_in_memory` (0.8.0 line about 85) calls `mercury_svg::parse_tree(bytes, &usvg_options)` instead of
     `usvg::Tree::from_data`. `parse_tree` does what `from_data` does (gunzip SVGZ, UTF-8, roxmltree with `allow_dtd`), counts
     the nesting of the elements **on the parsed document** (so entities declared in a `<!DOCTYPE>` that expand to elements are
     counted; one pass over the nodes with a stack of the open elements, no recursion, stops at the first depth over the limit),
     refuses more than `MAX_SVG_DEPTH` = 500 (upstream's constant) with `Err("SVG is nested too deeply")`, and hands the parsed
     document to `usvg::Tree::from_xmltree`. Not XML, not UTF-8 and bad gzip come back as `Err("Not a valid SVG document")`.
  2. `ImageCacheFactoryImpl::create` sets `resolve_data: mercury_svg::data_resolver()`, usvg's own resolver with the same count
     applied to an SVG inside an SVG (`<image href="data:image/svg+xml;...">`, which usvg parses by itself, with the same
     functions and on the same stack). The documents being converted at once on a thread (a thread-local chain) may add up to
     500 levels and be at most `MAX_NESTED_DOCUMENTS` = 4; a nested image over budget is left out and the outer one loads.
  3. `parse_tree` runs on a scoped thread of its own with a 32 MiB stack (`PARSE_STACK_BYTES`; if the thread cannot be started it
     parses on the calling thread and logs it). This is what makes the limit hold against `<use>`: usvg's recursion depth is the
     depth of the tree **after** `<use>` expansion (up to its own limit of 1024), not the depth in the file.
  4. SVGZ size limit (train-22 security review, finding 2): `unpack` no longer calls `usvg::decompress_svgz`, which reads gzip to
     the end with no limit in usvg 0.48.1 (a gzip bomb of a few kilobytes fills the memory). It reads through
     `flate2::read::GzDecoder` limited to `MAX_SVG_BYTES` + 1 = 32 MiB + 1 and refuses a larger result (top level: `Err("Not a
     valid SVG document")`, logged; nested `data:` image: left out before usvg's own resolver can unpack it again). This is the
     only reason for `flate2` in `[dependencies]` of `Cargo.toml` (it was a dev-dependency; marked `# Mercury:`). Tests: a
     1 GiB gzip bomb (about 5 MB of gzip) is refused, exactly 32 MiB loads and 32 MiB + 1 is refused, and a bomb in a nested
     `data:` image leaves the image out. Dropped together with the file at the resvg 0.50 bump (check then whether usvg's
     `decompress_svgz` is still unbounded, and keep the limit if so).
- **What was measured (usvg 0.48.1, a 2 MiB thread, as a rayon worker's, release build; probe and numbers in
  `.superpowers/sdd/k0-engine-base-logs/svg-probe-*.txt`):** usvg refuses more than 1024 levels by itself (`NodesLimitReached`,
  initial depth 0), so the first version's 100,000-level test documents never reached the converter; the crash is in the range
  501 to 1024. Nested `<svg>` elements overflow the stack from about 700 levels (650 passes, 700 aborts; 2 to 3 MiB at 1024);
  `g`, `a`, `switch`, opacity, transform, clip path, mask and filter chains pass 1024 levels in 2 MiB. Four groups of 200 nested
  `<svg>` elements, each ending in a `<use>` of the previous group, are 202 levels in the file and abort a 2 MiB thread
  (about 800 for usvg), which is why a byte or XML depth count alone cannot be the whole fix. `resvg::render` needs 512 KiB at
  1024 levels.
- **The first version's flaws, fixed here:** it lexed the bytes and skipped the whole DOCTYPE, claiming an entity "adds at most
  10 levels" (roxmltree's 10 limits how deep entity *references* nest, not the markup inside one entity value, which it parses
  as elements); it looked only at the top-level bytes, so a nested `data:` SVG was unguarded; and its tests and page used
  100,000 levels, which usvg rejects without the guard.
- **Tested by:** 20 tests in `mercury_svg.rs` (boundary 499/500/501 for `g` and for nested `svg`, the 600 to 1024 range that usvg
  alone accepts and the guard refuses, 100,000 levels on a 2 MiB stack, entities (600 deep, 400 twice, five nested of 120),
  SVGZ, nested data images by `image/svg+xml`, `text/plain` and gzip, the shared budget (299 + 300 refused, 100 + 100 and 249 + 250
  allowed), the document limit, the chain cleaned after every parse, raster data untouched, and the `<use>` document
  above, which aborts the test process on a 2 MiB stack unless the large stack is used). They run in `make test-servo-net`
  (`tests/mercury_unit.rs` includes the file by `#[path]` into the `main` test target; they ran in a scratch crate in K0); the
  three mutants (small stack, plain default resolver, no depth count) were run and fail. And the page
  `tests/pages/svg-deep-nesting.html` (600 `g`, 800 nested `svg`, an entity, a nested `data:` image and the `<use>` document,
  none of which usvg alone refuses).
- **Drop when:** the Servo bump that takes resvg 0.50: set `max_depth: 500` where `usvg::Options` is built in
  `impl ImageCacheFactory for ImageCacheFactoryImpl`, as upstream does, and delete `mercury_svg.rs` and the two calls
  (keep the large-stack thread only if usvg 0.50's `max_depth` of 500 still overflows 2 MiB on nested `<svg>`, which it
  should not: 500 levels pass in 2 MiB).

### 4. Certificate exceptions bound to the pair (host name, certificate) (EC27) — `certificate_exceptions.rs` (new), `connector.rs`, `fetch/methods.rs`, `lib.rs`, `http_loader.rs`, `websocket_loader.rs`

Made by lane cert-exceptions. Certificate exceptions are bound to the pair (host name, certificate)
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

`Cargo.lock`, which `make test-servo-net` uses, takes `url` 2.5.9 and `form_urlencoded` 1.2.3 (RUSTSEC-2026-0592
in `url` 2.5.8) instead of the versions the published crate ships. As a generated file it carries no `Mercury:`
comment and is listed under `unmarked` in `third_party/vendor.toml`.

## Running the tests

`servo-net` is a vendored dependency with dev-dependencies, so `cargo test -p servo-net` from the
workspace root does not work (cargo refuses a package that is not a workspace member). The root `Cargo.toml`
lists `third_party/servo-net` under `exclude`, as it does `third_party/gaol`, so cargo accepts the crate's own
manifest, and `make test-servo-net` runs the crate's whole `main` target on its own `Cargo.lock`, against the
published sibling crates (not Mercury's vendored copies and not Mercury's `Cargo.lock`; Mercury's K0 changes to
`servo-embedder-traits` and `servo-config` are not in this build, and `servo-net` is the only crate Mercury changes in
a way that matters to these tests; the others it links against are byte-identical to the published ones, except
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

`make test-servo-net` adds `fonts/bundled_freetype` to the features, which builds FreeType from source: on Linux
the crate's own graph needs FreeType 24.3.18 or later (`freetype-sys`) and Ubuntu 22.04 has 24.1.18; on macOS the
feature does nothing. Add `,fonts/bundled_freetype` to the command above on such a system.

The first run builds the published dependencies (about 11 minutes on a 12-thread Intel Mac, a few
hundred MB under `target/`; `--target-dir` keeps the build out of the vendored crate). After an
edit, format only the files you changed with the command in
`docs/developer-guide/vendored-patches.md` ("No formatting churn"), run `export` and
`make servo-patches-check`.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-net.patch` and this directory
once Mercury no longer needs to change the crate.
