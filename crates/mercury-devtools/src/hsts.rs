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
/// flag both in the list Servo 0.8.0 ships and in Chromium's list of 22 Nov 2026, so the shipped
/// file and a real refresh both meet the rule. Two hosts are deliberately not here: `paypal.com`
/// is listed without the flag in both (as is `www.paypal.com`), and `google.com` itself is no
/// longer listed in Chromium's file (only `accounts.google.com`, `mail.google.com` and the
/// like, and the TLD `google`).
pub const CANARIES: [&str; 4] = ["accounts.google.com", "github.com", "dropbox.com", "dev"];
/// A refreshed list with fewer entries is refused. The live list of 22 Nov 2026 has 95,544
/// entries and the older snapshot Servo 0.8.0 ships has 166,958, so this is about half the size
/// of the list today: a truncated download or a half-parsed file falls under it, while Chromium
/// would have to prune nearly half of its list again before an honest refresh does.
pub const DEFAULT_MIN_ENTRIES: usize = 50_000;
/// A refreshed list with more than this share fewer entries than the file it replaces is refused.
/// (The first refresh, on 22 Nov 2026, broke this on purpose and used `--allow-shrink`: see
/// `docs/developer-guide/releases.md`.)
pub const DEFAULT_MAX_SHRINK_PERCENT: usize = 10;
/// A refreshed list in which more than this share of the hosts it has in common with the file it
/// replaces lose `include_subdomains` is refused. Today's Chromium list differs from Servo's
/// snapshot in no flag at all, so one percent leaves room for a handful of honest changes and
/// stops a parse that reads the flag wrongly for a slice of the list.
pub const DEFAULT_MAX_FLAG_LOSS_PERCENT: usize = 1;
/// A refreshed list in which fewer than this share of entries include subdomains is refused
/// (the shipped list has 99.85 %, Chromium's of 22 Nov 2026 99.74 %).
pub const MIN_INCLUDE_PERCENT: usize = 90;
/// `build` refuses an input larger than this many bytes (Chromium's file is about 10.6 MB as
/// JSON and 14.2 MB as Gitiles' base64), and the download script's `curl` has the same cap.
pub const MAX_INPUT_BYTES: u64 = 100_000_000;
/// Where the data file lives, relative to the repository root, and where `build` writes by default.
pub const DEFAULT_DATA: &str = "crates/talaria-servo/resources/hsts_preload.fstmap";
/// Chromium's file, relative to `src/`.
pub const CHROMIUM_FILE: &str = "net/http/transport_security_state_static.json";
/// Gitiles' base: the download URL is this, then `/<ref>/`, [`CHROMIUM_FILE`] and `?format=TEXT`.
pub const CHROMIUM_HOST_URL: &str = "https://chromium.googlesource.com/chromium/src/+";
