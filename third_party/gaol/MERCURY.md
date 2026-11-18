# Mercury gaol patch

This directory vendors `gaol` 0.2.1 under its MIT/Apache-2.0 license because
Servo uses its Linux namespace and seccomp implementation for content
processes. The patch was introduced by Hermes Browser, the project Mercury
Browser is derived from, and Mercury continues to carry it.
`third_party/patches/gaol.patch` is the whole difference from the crate as
published on crates.io; each change is marked with a `Mercury:` comment.

The patch extends the legacy seccomp allowlist only where required by current
Rust, glibc, SpiderMonkey, and Servo. It also leaves `RLIMIT_FSIZE` unchanged so
Servo's anonymous `memfd` IPC regions can be sized with `ftruncate`; the
original zero-byte limit killed content processes with `SIGXFSZ`. Filesystem
access remains constrained by gaol's mount namespace and chroot profile;
network access remains absent from content processes.

The content process no longer gets its own PID namespace (`namespace.rs`):
Servo pairs content processes with their memory reporters by OS PID, and
inside a PID namespace every child reported itself as PID 1. The user, mount,
IPC, UTS and (without network access) network namespaces remain.

`tests/shared-memory.rs` is Mercury's regression test for the `RLIMIT_FSIZE`
change; `Cargo.toml` declares it and requires libc 0.2.177 for the system-call
constants the allowlist uses. `Cargo.lock`, which the crate's own test build
(`cargo test --manifest-path third_party/gaol/Cargo.toml`) uses, was regenerated
by Cargo in the current lock format; as a generated file it
carries no `Mercury:` comment and is listed under `unmarked` in
`third_party/vendor.toml`.

Mercury also made three warning-only edits so builds print no warnings:
`try!(…)` became `…?` in `sandbox.rs`, the `extern` block in
`platform/macos/mod.rs` names the `"C"` ABI, and an unnecessary `unsafe` block
around the wait-status macros in `platform/unix/process.rs` was removed.

Remove the `[patch.crates-io]` entry, the `third_party/vendor.toml` entry,
`third_party/patches/gaol.patch` and this directory after equivalent sandbox
updates land in the Servo release used by Mercury.
