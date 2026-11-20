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
- **Tested by:** the unit tests in `mercury_log.rs` (29 with the SVG ones below), run in a scratch crate that includes the two files with `#[path]` (see "Testing a vendored crate" in `docs/developer-guide/vendored-patches.md`: cargo refuses `-p servo-net` outside the crate's own workspace, because it has dev-dependencies). They are: redaction of each
  name and of mixed case, other headers unchanged, an empty value, a full map with two `Set-Cookie` entries; and
  `include_str!` guards: `the_cookie_files_do_not_log_a_cookie_value` fails if a logging call in `cookie_storage.rs` or
  `cookie.rs` logs anything but the cookie's name, domain, path or the sent count;
  `the_http_loader_logs_headers_only_through_the_redacting_helpers` fails if `http_loader.rs` logs a header or header map
  outside the two helpers; `the_guard_finds_the_lines_servo_used_to_have` proves the guard flags Servo's old lines.
- **Drop when:** Servo no longer logs these values; then delete `mercury_log.rs` and its call sites.

### 3. SVG nesting guard (decision D8) — `image_cache.rs`, `lib.rs`, new `mercury_svg.rs`

- **What:** Mercury's replacement for servo/servo#49835, which sets `usvg::Options::max_depth` (it exists from usvg 0.50;
  the tree has 0.48.1, and moving resvg also touches the non-vendored `servo-malloc-size-of`).
  `parse_svg_document_in_memory` (0.8.0 line about 85) calls `mercury_svg::guard(bytes)` before `usvg::Tree::from_data`.
  `guard` unpacks gzip (SVGZ) with `usvg::decompress_svgz` so compression is no way round it, scans the XML in one pass
  without recursion, and returns `Err("SVG is nested too deeply")` for a document nested deeper than `MAX_SVG_DEPTH` = 500
  elements (upstream's constant). Comments, CDATA, processing instructions and DOCTYPE (with its internal subset) are
  skipped, `<` and `>` in quoted attribute values are text, and a self-closing element counts as a level that does not
  deepen what follows. Non-XML and cut-off input returns "not too deep" and is left to usvg. Known limit: entities declared
  in a DOCTYPE can expand to elements, which the scan does not follow; roxmltree limits entity expansion to 10 levels, so
  that adds at most 10.
- **Why:** usvg walks the tree with one stack frame per level, so a tiny image nested 100,000 deep (an `<img>` from a Blob
  URL) can overflow the decoder thread's stack and crash the browser.
- **Tested by:** the same scratch crate (19 tests in `mercury_svg.rs`: 499 and 500 deep ok, 501
  and 502 refused, 100,000 deep refused with and without close tags, wide documents, self-closing edge cases, comments,
  CDATA, PIs, DOCTYPE with subset, quoted brackets, namespaces, non-XML and cut-off input, svgz) and the page
  `tests/pages/svg-deep-nesting.html`.
- **Drop when:** the Servo bump that takes resvg 0.50: set `max_depth: 500` where `usvg::Options` is built in
  `impl ImageCacheFactory for ImageCacheFactoryImpl`, as upstream does, and delete `mercury_svg.rs` and the call.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-net.patch` and this directory
once Mercury no longer needs to change the crate.
