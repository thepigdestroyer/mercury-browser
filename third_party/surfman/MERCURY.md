# Patched surfman 0.14.0

This directory vendors `surfman` 0.14.0 under its original MIT, Apache-2.0,
and MPL-2.0 licenses. The patch was introduced by Hermes Browser, the project
Mercury Browser is derived from, and Mercury continues to carry it.
`third_party/patches/surfman.patch` is the whole difference from the crate as
published on crates.io; each change is marked with a `Mercury:` comment.

Mercury uses Mesa's default hardware adapter on Linux instead of implementing
the hardware preference by setting the process-global `DRI_PRIME=1` before
each GL context creation. The upstream behavior assumes that GPU index 1
exists and races with GStreamer in a multithreaded browser. Mesa and the
desktop session remain responsible for automatic device selection.

The crate's own `.gitignore` ignores `*.lock`, so its published `Cargo.lock`
is committed with `git add -f`.

Remove the root `[patch.crates-io]` entry, the `third_party/vendor.toml` entry,
`third_party/patches/surfman.patch` and this directory after an equivalent
context-local adapter selection fix lands in the Surfman release used by
Mercury.
