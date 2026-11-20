# Patched servo-config 0.8.0

This directory vendors `servo-config` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`cdc1c3fa6136a6d3337ba6297118e3ce5194a9e5a17d41ef91a4973fde68d74e`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/config`,
plus Mercury's changes in `third_party/patches/servo-config.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-config` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane K0 (engine base, 19 Nov 2026). Mercury's own additions, not upstream backports.

### 1. Twenty preferences for the first two waves, one block — `prefs.rs`

- **What:** one block of 20 new fields at the end of `Preferences` and the matching block at the end of
  `const_default()`, both headed `// Mercury (K0)`. Bool, default `false`: `dom_media_source_enabled`,
  `dom_media_capabilities_enabled`, `dom_eme_enabled`, `dom_picture_in_picture_enabled`,
  `dom_video_frame_callback_enabled`, `dom_webcodecs_enabled`, `dom_display_capture_enabled`,
  `dom_audio_output_enabled`, `dom_audioworklet_enabled`, `dom_media_recorder_enabled`,
  `dom_webrtc_peerconnection_enabled`, `media_eme_clearkey_enabled`, `privacy_gpc_enabled`,
  `network_cookie_samesite_lax_default`, `dom_web_locks_enabled`. String, default `""` (the 0.8 behaviour,
  meaning "allow" or "default"): `media_autoplay_policy`, `media_autoplay_allow_origins`,
  `media_autoplay_block_origins`, `media_webrtc_ip_policy`, `stylo_prefs`. No field has a reader in Servo yet:
  each is read by the lane named in its doc comment (EA1, EA2, ..., EC25, EC6, ED4), which adds the reader in its
  own file. `dom_webrtc_transceiver_enabled` is not touched (the media-capture pin stays).
- **Why:** R14 section 4.1: hot-crate edits happen only in numbered batches, so a wave-1 lane never edits this file
  and Servo rebuilds once. A preference set in `prefs::set` is seen by every content process, and `set_value` and
  `get_value` panic on an unknown name, so every name must exist before a lane sets it. Mercury's values live in
  `crates/talaria-servo/src/engine_prefs.rs`.
- **Upstream:** `components/config/prefs.rs` (v0.8.0): the end of the struct after `expose_servointernals_globally`
  (line 427) and the end of `const_default()` (line 617).
- **Tested by:** `talaria-servo` `engine_prefs::tests` (every name exists and has its Mercury value; every feature
  switch stays off) and the `runtime.rs` preference tests.
- **Drop when:** Servo adds a preference of the same name (check with `Preferences::exists` at each bump) or the
  lane that owns a field is removed.

### 2. Stylo preferences by name — `prefs.rs`, `lib.rs`, new `mercury_stylo.rs` (ED2)

- **What:** Servo forwards seven Stylo preferences, each by hand in `set()`. After those seven `set_pref!` lines,
  one line calls `crate::mercury_stylo::apply_stylo_prefs(&preferences.stylo_prefs)`, and one `pub use` makes
  `apply_stylo_prefs` and `stylo_bool_pref` reachable as `servo::prefs::...`. `mercury_stylo.rs` (new) parses a
  comma-separated `name=true|false` list (spaces trimmed, empty entries skipped, a repeated name: last wins),
  applies each with `stylo_static_prefs::set_bool`, and returns the names it could not apply (malformed
  entries whole). `stylo_bool_pref(name)` reads one back. `lib.rs`: `mod mercury_stylo;`. No `log` dependency.
- **Why:** every Stylo feature Mercury turns on (`:nth-child(of)` now, `:has` and others from wave 3) would otherwise
  need a Servo-struct edit and a rebuild. A name list needs no further edit of this file. A name in the list wins
  over the seven fixed mappings (the hook comes after them). `set()` applies the list on every call that changes a
  preference, in each content process too. A name removed from the list keeps its last value: the list sets, it does
  not reset. Unknown names are ignored by the engine; Mercury's test asserts there are none.
- **Upstream:** `components/config/prefs.rs` `set()` line 74 (after the `layout.css.writing-mode.enabled` call, before
  `*PREFERENCES.write()`); `components/config/lib.rs`: one `mod` line.
- **Tested by:** `cargo test --release -p servo-config` (`mercury_stylo::tests`, 9 tests: values, spaces and empty
  entries, `false`, last value wins, unknown and malformed entries returned, empty list, other names, `prefs::set`
  applying the list) and `engine_prefs::tests::every_stylo_name_exists_and_is_applied_by_servo`.
- **Drop when:** Stylo or Servo forwards preferences by name (nothing like it in Servo main on 19 Nov 2026).

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-config.patch` and this directory
once Mercury no longer needs to change the crate.
