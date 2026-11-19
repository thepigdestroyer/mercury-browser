# Patched stylo_static_prefs 0.23.0

This directory vendors `stylo_static_prefs` 0.23.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`8e06c605a1d428d159798ebfed3f81f3a145f1838872360ebb24ec6d85993fe1`), built from https://github.com/servo/stylo at commit
`2dca34592ced37c6aa01733e94206a767194cf9d`, path `stylo_static_prefs`,
plus Mercury's changes in `third_party/patches/stylo_static_prefs.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff stylo_static_prefs` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane K0 (engine base, 19 Nov 2026), part of the Stylo preference bridge (ED2); the other half is
`servo-config` (`mercury_stylo.rs`).

### Name-based setters for boolean preferences — `build.rs`, new `mercury_bridge.rs`, `src/lib.rs`

- **What:** `build.rs` gets `mod mercury_bridge;` and one call, `mercury_bridge::append_bridge(&mut output, <names of the
  boolean preferences>)`, before the generated code is written. `mercury_bridge.rs` (new) writes into the generated code a
  private `mercury_bool_index(name) -> Option<usize>` (a `match` over the boolean names, in `BOOLS` order),
  `pub fn set_bool(name: &str, value: bool) -> bool` (false, and no effect, for a name that is not a boolean preference,
  integer ones included) and `pub fn get_bool(name: &str) -> Option<bool>`. It also prints
  `cargo::rerun-if-changed=mercury_bridge.rs`. `src/lib.rs`: four tests.
- **Why:** upstream's `set_pref!("name", value)` needs the name as a literal in the source, so an embedder cannot forward a
  list of names it reads at run time. The accessors use the same `BOOLS` array as `pref!`, so Stylo reads the new value
  at once.
- **Upstream:** `build.rs` (stylo `2dca34592c`): `use toml::...` (line 7, the `mod` line follows it) and the end of
  `generate_code`, after the closing `}}` of `set_pref!` and before `output.join("\n")` (line 126). `src/lib.rs`: tests
  appended after `test_basic_preferences`.
- **Tested by:** `cargo test --release -p stylo_static_prefs`: `mercury_set_bool_changes_what_pref_reads`,
  `mercury_get_bool_reads_the_defaults`, `mercury_set_bool_refuses_unknown_names_and_integer_preferences`,
  `mercury_every_boolean_in_preferences_toml_has_an_accessor` (reads `preferences.toml`).
- **Drop when:** stylo publishes name-based setters (none on 19 Nov 2026) or `servo-config` stops needing the list. A new
  stylo that changes `build.rs` heavily may conflict at the two hook lines: re-apply them.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/stylo_static_prefs.patch` and this directory
once Mercury no longer needs to change the crate.
