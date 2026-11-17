---
title: Contributing
description: Prepare focused, reviewable contributions to Mercury Browser.
---

# Contributing

Good Mercury contributions make one behavior easier to understand, test, or trust. Browser work can spread across UI, platform, engine, graphics, media, and networking quickly, so a narrow change is a real advantage.

## Before writing code

- Search existing issues and recent commits.
- Reproduce the behavior with the locked dependency graph.
- Decide which layer owns the policy.
- For website failures, compare the pinned Servo `servoshell` when possible.
- For a dependency patch, read [Vendored patches](vendored-patches.md) first.
- Write product code and tools in Rust and Slint only; see [the rules that apply everywhere](index.md#rules-that-apply-everywhere).

## Keep boundaries intact

Put engine-independent browser behavior in `talaria-core`. Keep Slint presentation in `mercury-ui`, desktop event translation and composition in `mercury-desktop`, and Servo-specific types in `talaria-servo`.

If a change requires Servo internals to leak into the core model, pause and look for a smaller command, event, or adapter abstraction.

## Write tests near the behavior

- State and policy changes belong in core tests.
- Servo type conversions belong in adapter tests.
- Shortcut and native event behavior belongs in desktop tests.
- Dependency regressions belong beside the vendored patch.

Use manual website testing as additional evidence, not as a replacement for a deterministic test when one is possible.

## Run the checks

```bash
make qa CARGO_FLAGS=--release
make docs
```

If the change affects rendering, media or anything a person sees, also run the release build (with `--screenshot` for automated runs) and record what you tested. Changes under `scripts/` or `.github/workflows/` also need `make lint-scripts`, and dependency changes `make deny`.

## Commit style

The project uses short Conventional Commit-style subjects in English:

```text
feat(core): initial tab URLs, engine-ready command gating and file URLs
fix(desktop): keep Servo running when the window is covered
feat: use the system proxy and keep an HTTP disk cache in the profile
refactor(desktop): split main.rs into focused modules
docs: add a hands-on checklist
```

Choose the smallest useful scope, keep the subject lowercase after the colon, and explain surprising tradeoffs in the commit body or code comments.

## Open a pull request

A useful pull request describes:

- The user-visible or developer-visible problem
- Why the chosen layer owns the fix
- Automated checks that cover it
- Manual pages, platforms, GPUs, or media files tested
- Known limitations left intentionally unresolved
- Any upstream issue or patch that should eventually replace local code

Honest limits are welcome. Mercury is a preview, and clear evidence is worth more than broad compatibility claims.
