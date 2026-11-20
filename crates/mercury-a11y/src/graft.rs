// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The order in which a page's AccessKit subtrees may reach the window's adapter: a tree is pushed
//! only after a graft that names it has been pushed. Pure state (no winit, no Slint), so the
//! vendored Slint backend only calls it and its tests run in the light pool against
//! `accesskit_consumer`, which panics where AccessKit would.
//!
//! Owner: item 1 (`graft`).
