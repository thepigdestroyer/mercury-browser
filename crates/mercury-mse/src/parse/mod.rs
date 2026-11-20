// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Item 1 `parse`: incremental byte-stream parsers for fragmented MP4 and WebM.
//!
//! OWNER: item 1 of lane med-1 (brief section 6). The signature of [`new_parser`] is the contract with
//! item 3; the trait it returns is `crate::api::ByteStreamParser`.

pub mod bmff;
pub mod config;
pub mod ebml;
pub mod mp4;
pub mod webm;

use crate::api::{ByteStreamParser, Container};

/// A parser for `container`, ready for its first byte.
pub fn new_parser(container: Container) -> Box<dyn ByteStreamParser> {
    let _ = container;
    todo!("item 1: the parsers")
}
