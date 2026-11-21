// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The HSTS preload list tool (`hsts-preload`): turns Chromium's `transport_security_state_static.json`
//! into `crates/talaria-servo/resources/hsts_preload.fstmap`, the `fst` map Servo reads as
//! `Resource::HstsPreloadList` (key = host name, value = `2 * rank + include_subdomains`, so the
//! parity is the flag), with a manifest that records where the data came from. See
//! `.superpowers/sdd/lane-ec16-tool-brief.md` for the contract; `docs/developer-guide/releases.md`
//! for how a maintainer refreshes the list.
//!
//! - [`source`]: Chromium's JSON (comments, trailing commas, Gitiles' base64) to [`Entry`] values.
//! - [`fstmap`]: writes and reads the data file, and checks it the way Servo reads it.
//! - [`manifest`]: the manifest (`hsts_preload.manifest.json`), SHA-256 and dates.
//! - [`review`]: `check` (is the file fit to ship) and `diff` (what a refresh changed), and the
//!   guards that stop a bad refresh from replacing a good file.
//! - [`cli`]: the command line of the `hsts-preload` binary.
//!
//! Nothing here touches the network; the download is `scripts/dev/update-hsts-preload.sh`.

pub mod cli;
pub mod fstmap;
pub mod manifest;
pub mod review;
pub mod source;

/// One preloaded host: lower-case ASCII, no dot at either end.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    pub host: String,
    pub include_subdomains: bool,
}

/// Hosts every refreshed list must contain with `include_subdomains` (long-standing preloads, and
/// the TLD `dev`, so that a parse that drops TLD entries is caught). Each one is listed with the
/// flag in the list Servo 0.8.0 ships, so the shipped file meets the rule itself; `paypal.com` is
/// deliberately not here: that list has it (and `www.paypal.com`) without the flag.
pub const CANARIES: [&str; 4] = ["google.com", "github.com", "dropbox.com", "dev"];
/// A refreshed list with fewer entries is refused (the list Servo 0.8.0 shipped has 166,958).
pub const DEFAULT_MIN_ENTRIES: usize = 100_000;
/// A refreshed list with more than this share fewer entries than the file it replaces is refused.
pub const DEFAULT_MAX_SHRINK_PERCENT: usize = 10;
/// A refreshed list in which fewer than this share of entries include subdomains is refused
/// (the shipped list has 99.85 %).
pub const MIN_INCLUDE_PERCENT: usize = 90;
/// Where the data file lives, relative to the repository root, and where `build` writes by default.
pub const DEFAULT_DATA: &str = "crates/talaria-servo/resources/hsts_preload.fstmap";
/// Chromium's file, relative to `src/`.
pub const CHROMIUM_FILE: &str = "net/http/transport_security_state_static.json";
/// Gitiles' base: the download URL is this, then `/<ref>/`, [`CHROMIUM_FILE`] and `?format=TEXT`.
pub const CHROMIUM_HOST_URL: &str = "https://chromium.googlesource.com/chromium/src/+";
