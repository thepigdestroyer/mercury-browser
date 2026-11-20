// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The track buffer: one sorted store of coded frames per track kind.
//!
//! OWNER: item 2 of lane med-1. Empty until item 2 fills it; nothing outside `processor.rs` needs its
//! API (item 3 goes through `SourceBufferProcessor`). If this file translates Ladybird's `TrackBuffer`
//! or Firefox's `TrackBuffersManager` it carries that source's SPDX line and header (brief D1).
