// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Type strings: `addSourceBuffer(type)`, `changeType(type)` and `MediaSource.isTypeSupported(type)`.
//!
//! OWNER: item 1 of lane med-1 (EA2 core). The signatures are the contract with item 3 and with the
//! glue (`MseBackend::is_type_supported`). Decision D4 says which containers and codecs are accepted.

use crate::api::{CodecId, Container, MseError, TrackKind, TypeSupport};

/// A parsed MIME type with codecs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedType {
    pub container: Container,
    /// The kinds of track the type promises (from the codecs, or from the major type when `codecs` is
    /// absent).
    pub kinds: Vec<TrackKind>,
    /// The codecs named, in order; empty when the type has no `codecs` parameter.
    pub codecs: Vec<CodecId>,
    /// The type as written, trimmed (the DOM does not need it normalised).
    pub original: String,
}

/// Parses a type string. `MseError::Type` for a syntax error or an empty string,
/// `MseError::NotSupported` for a container or codec that decision D4 excludes, and for two video
/// codecs, a video codec in `audio/*` and the other combinations that cannot be one source buffer.
pub fn parse_type(mime: &str) -> Result<ParsedType, MseError> {
    let _ = mime;
    todo!("item 1: the type parser")
}

/// `MediaSource.isTypeSupported`: [`parse_type`] succeeds and `types` has a decoder for every codec.
pub fn is_type_supported(mime: &str, types: &dyn TypeSupport) -> bool {
    let _ = (mime, types);
    todo!("item 1: isTypeSupported")
}
