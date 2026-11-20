// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Media Source Extensions for Mercury, by Plan B of R14a (the GstMSE spike said no, R17): Mercury's own
//! byte-stream parsers, coded-frame processing and track buffers, with a feed into GStreamer's `appsrc`
//! (the `mercury-mse-gst` crate) and a trait for the DOM ([`MediaSourceBackend`]).
//!
//! Lane brief: `.superpowers/sdd/lane-med-1-brief.md`. The contract is [`api`]. Who owns which module:
//!
//! | module | item |
//! |---|---|
//! | `api` | the lead (frozen: add, never rename) |
//! | `parse`, `mime` | 1 `parse` |
//! | `ranges`, `trackbuffer`, `processor` | 2 `frames` |
//! | `source_buffer`, `media_source`, `backend` | 3 `model` |
//!
//! Each item edits only its own block of `pub use` lines below.

pub mod api;
pub mod backend;
pub mod media_source;
pub mod mime;
pub mod parse;
pub mod processor;
pub mod ranges;
pub mod source_buffer;
pub mod trackbuffer;

pub use api::*;

// ---- item 1 `parse` exports ----
pub use mime::{ParsedType, is_type_supported, parse_type};
pub use parse::new_parser;

// ---- item 2 `frames` exports ----
pub use processor::{
    FeedCursor, InitOutcome, ProcessError, Processed, Removed, SourceBufferProcessor,
};

// ---- item 3 `model` exports ----
pub use backend::MseBackend;
