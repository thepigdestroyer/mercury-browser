# Patched servo-default-resources 0.8.0

This directory vendors `servo-default-resources` 0.8.0, the files Servo reads through its
resource reader (the public-suffix list, the HSTS preload list, the built-in pages), under its
original MPL-2.0 license. It is the crate exactly as published on crates.io (checksum
`8bd5df0724a6b49aba27fbfaeb124abd646cc8c362a013c6bf87d9cc8d2315b4`), built from
https://github.com/servo/servo at commit `4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path
`components/default-resources`. The MPL-2.0 text in the repository's `LICENSES/MPL-2.0.txt`
applies to this directory.

`third_party/patches/servo-default-resources.patch` is the difference from the published crate
(`mercury-servo-patches diff servo-default-resources` prints it from the directory as it is
now), and `make servo-patches-check` fails if the two disagree. It holds every change, and only
the change listed below, marked with a `Mercury:` comment.

## Changes

### 1. Do not register the default resource reader

- **What:** the last line of `lib.rs`, `embedder_traits::submit_resource_reader!(&DefaultResourceReader);`,
  is removed. `DefaultResourceReader` is unchanged and still public.
- **Why:** Servo reads its error pages (`Resource::BadCertHTML`, `NetErrorHTML`, `CrashHTML`)
  through the one registered resource reader. Mercury registers its own
  (`crates/talaria-servo/src/resources.rs`) to serve Mercury's error pages, and that reader
  serves every other resource from `DefaultResourceReader`. With both registered, Servo picks
  one of them at random (`servo-embedder-traits` 0.8.0 `resources.rs:61-77`). Servo links
  this crate through its default `baked-in-resources` feature and `servo-net`'s dependency, so
  turning the feature off would not remove it, and would change Servo's feature list, which
  other work also edits.
- **Upstream:** `lib.rs:40`.
- **Proof:** `cargo test -p talaria-servo resources` (Mercury's reader serves its pages and
  Servo's files); the error pages show in `--screenshot` runs (see the site-security report).
- **Drop when:** Servo lets an embedder replace single resources, or registers its default
  reader only when no other is registered.
