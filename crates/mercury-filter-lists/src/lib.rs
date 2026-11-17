//! Mercury Browser's ad and tracker filter lists as data files.
//!
//! The lists ship beside the executable with a manifest (`manifest.json`). At start-up the
//! content-blocking engine is built off the UI thread from the newest valid copy of each list,
//! the bundled one or an updated one in the profile's cache, and kept compiled in that cache
//! so later starts are fast. Updated copies are fetched in the background from each list's
//! publisher. Only network rules are used; cosmetic (element-hiding) rules are not.

pub mod cache;
pub mod engine;
pub mod fetch;
pub mod list_text;
pub mod manifest;
pub mod schedule;
pub mod service;
pub mod timestamp;
pub mod update;

#[cfg(test)]
mod test_server;

pub use list_text::{ListSummary, MAX_LIST_BYTES, Rejection, check_download, summarize};
pub use manifest::{ListEntry, MANIFEST_FILE, Manifest, ManifestError, parse_manifest};

/// SHA-256 of `bytes` in lower-case hexadecimal.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}
