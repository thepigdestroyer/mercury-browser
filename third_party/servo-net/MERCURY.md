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
  above, which aborts the test process on a 2 MiB stack unless the large stack is used). They run in a scratch crate that
  includes the file by `#[path]` (cargo cannot test `servo-net` itself, see `docs/developer-guide/vendored-patches.md`); the
  three mutants (small stack, plain default resolver, no depth count) were run and fail. And the page
  `tests/pages/svg-deep-nesting.html` (600 `g`, 800 nested `svg`, an entity, a nested `data:` image and the `<use>` document,
  none of which usvg alone refuses).
- **Drop when:** the Servo bump that takes resvg 0.50: set `max_depth: 500` where `usvg::Options` is built in
  `impl ImageCacheFactory for ImageCacheFactoryImpl`, as upstream does, and delete `mercury_svg.rs` and the two calls
  (keep the large-stack thread only if usvg 0.50's `max_depth` of 500 still overflows 2 MiB on nested `<svg>`, which it
  should not: 500 levels pass in 2 MiB).

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-net.patch` and this directory
once Mercury no longer needs to change the crate.
