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
- **Why:** session cookies and credentials reached the log at `info` and `debug`. Not changed: URL logging at `info`
  (`http_network_fetch` still prints every URL, query strings included; a follow-up).
- **Tested by:** the unit tests in `mercury_log.rs` (10 of the 30 in the two files, the other 20 are the SVG ones below), run by `make test-servo-net`: `tests/mercury_unit.rs` includes the two files with `#[path]` into the `main` test target, because the library has `test = false`. They are: redaction of each
  name and of mixed case, other headers unchanged, an empty value, a full map with two `Set-Cookie` entries; and
  `include_str!` guards: `the_cookie_files_do_not_log_a_cookie_value` fails if a logging call in `cookie_storage.rs` or
  `cookie.rs` logs anything but the cookie's name, domain, path or the sent count;
  `the_http_loader_logs_headers_only_through_the_redacting_helpers` fails if `http_loader.rs` logs a header or header map
  outside the two helpers; `the_guard_finds_the_lines_servo_used_to_have` proves the guard flags Servo's old lines.
- **Drop when:** Servo no longer logs these values; then delete `mercury_log.rs` and its call sites.

### 3. SVG nesting guard (decision D8) — `image_cache.rs`, `lib.rs`, new `mercury_svg.rs`

- **What:** Mercury's replacement for servo/servo#49835, which sets `usvg::Options::max_depth` to 500 (an option that exists from
  usvg 0.50; the tree has 0.48.1, and moving resvg also touches the non-vendored `servo-malloc-size-of`). Three parts:
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

### 5. The network policy: third-party cookies blocked, SameSite, Global Privacy Control (EC5, EC6, EC25) — `net_policy.rs` (new), `mercury_cookies.rs` (new), `http_loader.rs`, `resource_thread.rs`, `cookie_storage.rs`, `cookie.rs`, `lib.rs`

Made by lane net-1 (owner decision O-18: "Standard + GPC on + third-party cookies blocked"). New files are PolyForm
Noncommercial and copy nothing from another project; the rules come from the specifications (RFC 6265bis, the Fetch
standard, <https://privacycg.github.io/gpc-spec/>). Servo 0.8 has no cookie policy at all: it sends every cookie to every
request and accepts every `Set-Cookie`. PR #44469 (the only upstream work) compares `request.origin` with the URL's origin, covers
only the `Set-Cookie` of responses and adds a global setter message; it is *considered, not carried*.

- **What:** `net_policy.rs` is one handle, `NetPolicy` (an `Arc`, cheap to clone). `NetPolicy::new()` behaves exactly as Servo
  0.8 in every method; `NetPolicy::process()` is the one process-wide instance both `HttpState`s (public and private) use and
  Mercury configures (`talaria-servo/src/net_policy.rs`). It has the switches (`block_third_party_cookies`,
  `same_site_enforcement`, `same_site_lax_default`, `global_privacy_control`, `referrer_cap`), per-site allow lists
  (`allow_third_party_cookies_on`, `set_mixed_content_allowed_on`) and a counter. `sites_of_request(&Request)` (and
  `sites_of_builder` for a WebSocket) says which site a request is for and whether it is first or third party, in this order: the
  client's network partition key (**script fills it for every document, every worker and worklet from the document that created
  it, and every nested-frame navigation from its parent document**: `servo-script` `Window::request_client`,
  `GlobalScope::network_partition_key`, `WorkerGlobalScopeInit.network_partition_key`, `ScriptThread::network_partition_key_of_load`);
  a top-level navigation (mode `Navigate`, destination `Document`, no initiator, a client that is not nested: the destination
  alone is not enough, because a frame navigated by a script or a link is fetched as `Document` too); otherwise unknown, which
  counts as third party for every decision. **The policy keeps no record of what a web view loads.** The first version recorded
  the site of every top-level navigation when it *started* and used it for requests without a key; a page could start a navigation
  to a tracker, let it be cancelled or answered 204, and its workers and frames were then first party to the tracker (net-1 review,
  finding 1). Third party means that the **scheme-less registrable domain** differs (`SiteKey`; an IP literal or a single label is
  its own key; an opaque origin has none). `mercury_cookies.rs` has the cookie rules: `cookies_allowed`, `cookie_may_be_sent` and
  `accept_set_cookie` (SameSite: Strict only on a same-site request, Lax also on a top-level navigation with a safe method,
  `SameSite=None` needs `Secure`; Lax-by-default stays off, `network_cookie_samesite_lax_default`), and the document and WebSocket
  helpers. A navigation from an opaque origin counts as the user's own (and so same-site) only when the **browser** started it:
  `servo-net-traits`' `RequestClient::browser_initiated_navigation`, set by `navigation.rs` for loads that did not come from a script
  (net-1 review, finding 2). The ancestor chain is not looked at: `a.test` inside `b.test` inside `a.test` is judged same-site
  (finding 9, a known gap; it needs the ancestor list of the document).
- **Blocked means:** no `Cookie` header (also not one the caller put in the header map), the `Set-Cookie` is ignored (at every
  redirect hop), `document.cookie` reads `""` and writes are dropped, the same for the WebSocket handshake, and the `*Async`
  `cookieStore` messages answer "no cookie" while blocking is on (the pref `dom_cookiestore_enabled` is pinned off anyway). The
  embedder messages (`Embedder*`: Mercury's site information, downloads, the PDF viewer) stay unfiltered. A `Partitioned` cookie in
  a third-party context is blocked like every other until NET-2 (CHIPS).
- **Global Privacy Control:** `Sec-GPC: 1` goes on every request that reaches `http_network_or_cache_fetch` (preflights and
  redirect hops included) and on the WebSocket handshake, from `net_policy.global_privacy_control()`.
- **Hook lines (upstream 0.8.0 line numbers, about):** `http_loader.rs` 101-106 (imports), 133 (`HttpState.net_policy`), 172-195
  (`request_authentication`, EC29 below), 359-425 (`set_request_cookies`, `set_cookie_for_url` and `set_cookies_from_headers` take
  a rule closure), 1534 (`Sec-GPC` after the User-Agent insert and the `note_main_frame` call), 1554-1575 (the cookie check before
  `set_request_cookies`, in place of the "XXXManishearth ... content blocking" comment), 2482 (the `Set-Cookie` check at step 15
  of `http_network_fetch`); `resource_thread.rs` 68 (imports), 229 and 246 (the two `HttpState` literals take
  `NetPolicy::process()`), 485-615 (the four `*Async` guards and the two document arms); `cookie_storage.rs` 221-308
  (`cookies_for_request` and `cookies_data_for_request`, the old methods delegate); `cookie.rs` 223 and 380 (the two "TODO"
  comments about same-site become pointers to `mercury_cookies.rs`); `lib.rs` (the `mod` lines of the lane).
- **Why:** third-party cookies are the main cross-site tracking channel; a browser that says "Standard" has to block them (O-18).
  SameSite is enforced because the same hooks decide it at no extra cost.
- **Known gaps:** third-party `localStorage`, IndexedDB and Cache Storage stay unpartitioned (EC8); the ancestor chain is not
  part of the SameSite test; no Lax-by-default and no 2-minute POST allowance; a nested frame whose parent document is in
  another script thread (cross-process frames) has no key for its navigation request and gets no cookies until its own
  document exists.
- **Tested by:** `tests/net_policy.rs` (the party matrix, nested navigations and workers with and without a key, navigations that
  are started and never finished, the allow list, `new()` = Servo's behaviour) and `tests/cookie_policy.rs` (real loopback servers:
  third-party `Set-Cookie` not stored, stored cookie not sent, first party works, per-site allow, a frame judged by its parent's
  key and a keyless one by nothing, a cross-site redirect hop, a top-level navigation answered 204, turned into a download or
  redirected into a 204 does not make the tracker first party for the page's workers and frames, the `*Async` messages,
  `document.cookie`, SameSite Strict/Lax/None, a script navigation from an opaque origin (GET and POST) is not a typed address,
  `Sec-GPC` on a request, a preflight and a redirect hop and absent when off, and a guard test that fails if
  `set_request_cookies` or `set_cookies_from_headers` is called outside the policy check). The mutation run (hooks forced open)
  and its result are in `.superpowers/sdd/lane-net-1-report.md`.
- **Drop when:** Servo has a cookie policy of its own (the Storage Access / third-party cookie work upstream); then keep only
  what its API cannot express.

### 6. The request path: frame identity, the rewrite hook, the referrer cap, mixed content (EC38, EC37, EC14, EC17) — `fetch/methods.rs`, `request_interceptor.rs`, `embedder.rs`, `mercury_rewrite.rs` (new), `mercury_mixed_content.rs` (new)

- **What:** (a) `intercept_request` (`request_interceptor.rs`) gets the policy and fills the three new `WebResourceRequest` fields
  (`top_level_site`, `initiator_origin`, `is_third_party`) from `sites_of_request`; `is_for_main_frame` is now true only for a
  top-level navigation and `url` is the current hop's URL. (b) A `WebResourceResponseMsg::Rewrite` answer is applied by
  `mercury_rewrite::apply`: all or nothing; the scheme must stay (HSTS may upgrade it afterwards); the origin must stay unless the
  request is a top-level `GET` navigation; the user name and password may not change; header changes refuse the Fetch standard's
  forbidden request-header names plus `Authorization`, `Proxy-Authorization` and the `Sec-` and `Proxy-` prefixes; refusals are
  logged at `warn` with the reason only. `main_fetch` applies HSTS a second time after the interceptor and reads `current_url` and
  `current_scheme` after it. (c) The referrer policy goes through `cap_referrer_policy`: with the cap on, `unsafe-url`,
  `no-referrer-when-downgrade` and `origin-when-cross-origin` become `strict-origin-when-cross-origin`; stricter policies are never
  loosened. (d) Mixed content (`should_request_be_blocked_as_mixed_content` and its response twin) lacks step 1.3 of the
  specification ("the user agent has been instructed to allow mixed content"): `mercury_mixed_content` lifts the block on a site the
  user allowed and sends one `NetToEmbedderMsg::Mercury(.., NetMsg::MixedContentBlocked)` per blocked request. The auto-upgrade of
  images, audio and video to `https` (step 6) still happens on an allowed site.
  After a rewrite that changed the URL, `main_fetch` runs the bad-port check and the Content Security Policy check again for the
  new URL (`network_error_after_rewrite`; they ran on the old one), and `mercury_rewrite::is_top_level_document` is the one
  definition of "top-level document" that the network policy's "top-level navigation" builds on (net-1 review, minors 6 and 8).
- **Hook lines:** `fetch/methods.rs` 504 (request-level mixed content), 525 (`cap_referrer_policy`), 543-568 (the interceptor call
  with the policy, HSTS and `current_url` after it), 704-732 (response-level mixed content); `request_interceptor.rs`
  (the signature, `FrameIdentity::of`, the `Rewrite` arm); `embedder.rs` 41 (the `NetToEmbedderMsg::Mercury` variant).
- **Why:** the content blocker needs the third-party flag and the frame without guessing (EC38), `$removeparam` needs a rewrite
  point (EC37, first user: Mercury's existing filter engine), the referrer cap is Firefox's and Brave's design (EC14), and a blocked
  mixed-content load has to be reported (EC17; Mercury counts them per tab, no UI yet).
- **Tested by:** `tests/mercury_request_path.rs` (28 tests, interceptor round trip with the fields for first-party, third-party,
  main-frame, nested and worker requests, the rewrite rules, HSTS after a rewrite, the referrer table, mixed content allowed and
  blocked) and the 17 unit tests in `mercury_rewrite.rs`.
- **Drop when:** Servo has frame identity and a rewrite point (items a and b), and implements step 1.3 of mixed content (d).

### 7. WebSockets through the proxy and sign-in prompts for `fetch` and XHR (EC23a, EC29) — `mercury_ws.rs` (new), `mercury_auth.rs` (new), `websocket_loader.rs`, `connector.rs`, `http_loader.rs`

- **What:** (a) `websocket_loader.rs` connected with a bare `TcpStream::connect`, so WebSockets went around the system proxy.
  `mercury_ws::connect_tcp(host, port, tls)` asks the same `ProxyConnector` as HTTP (`CONNECT host:port`, proxy basic auth,
  `no_proxy`) and connects directly when the matcher says so; `ProxyConnector::new` is `pub(crate)` (the only edit in `connector.rs`,
  0.8.0 line 634). The host-file mapping happens on the direct path. The handshake's `Cookie` header and the `Set-Cookie` of the 101
  answer follow the policy of item 5, and `Sec-GPC` is added. (b) `HttpState::request_authentication` returned `None` for every
  request that is not a navigation. `mercury_auth::subresource_prompt_allowed` lets a 401 prompt when the request is a `fetch` or
  XHR (destination none) from a document of the web view's top-level site to its own origin, with a web view id; `AuthLimiter` (in the
  policy) allows one prompt in flight per (web view, origin) and 60 s of silence after a refusal; a 407 and everything else get the
  response back as before. No realm line in the prompt (it needs a change of `PageRequestKind` in Mercury).
- **Tested by:** `tests/mercury_ws.rs` (a local WebSocket server behind a fake `CONNECT` proxy: the proxy sees the tunnel, `no_proxy`
  skips it, no proxy changes nothing, the handshake's `Sec-GPC` and cookies) and `tests/mercury_auth.rs` (the decision matrix and the
  limiter with a fake clock; a `PromptGuard` that is dropped with its fetch future silences the pair instead of leaving it "in
  flight" for good).
- **Drop when:** Servo sends WebSockets through its proxy and prompts for non-navigation 401s itself.

**`Cargo.lock` of this crate** (listed as `unmarked` in `third_party/vendor.toml`, a lock file takes no comments) now records `servo-net-traits` and `servo-embedder-traits` as path packages without a registry source or checksum, because `make test-servo-net` patches the vendored copies in (see below).

## Running the tests

`servo-net` is a vendored dependency with dev-dependencies, so `cargo test -p servo-net` from the
workspace root does not work (cargo refuses a package that is not a workspace member). The root `Cargo.toml`
lists `third_party/servo-net` under `exclude`, as it does `third_party/gaol`, so cargo accepts the crate's own
manifest, and `make test-servo-net` runs the crate's whole `main` target on its own `Cargo.lock`, against the
published sibling crates, **except `servo-net-traits` and `servo-embedder-traits`**, which the Makefile patches in
from `third_party/` with `--config 'patch.crates-io.<crate>.path=...'` (net-1: the lane's code uses the document cookie
messages, the request fields and the rewrite type that Mercury adds to them; `Cargo.lock` of `servo-net` records the two
path packages). It is not Mercury's `Cargo.lock`: the other crates it links against are byte-identical to the published
ones, except `servo-default-resources`, which Mercury patches only to stop it registering the default resource reader, and
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
