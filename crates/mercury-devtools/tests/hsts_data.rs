// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The HSTS preload list that ships in the browser
//! (`crates/talaria-servo/resources/hsts_preload.fstmap`, with `hsts_preload.manifest.json` beside
//! it): it is valid, it matches its manifest, nothing a refresh could break is missing, and the
//! browser really reads the file this tool writes. These tests need no engine, so every
//! `cargo test -p mercury-devtools` guards the data: Servo fails open on a bad file (an empty
//! list, so no host is upgraded) and would not say so loudly.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use mercury_devtools::hsts::fstmap::Preload;
use mercury_devtools::hsts::manifest::{Date, FORMAT, Manifest, sha256_hex};
use mercury_devtools::hsts::review::{Limits, check};
use mercury_devtools::hsts::source::valid_host;
use mercury_devtools::hsts::{CANARIES, DEFAULT_DATA};

/// The repository root (this crate is `crates/mercury-devtools`).
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The shipped data file's bytes.
fn shipped_bytes() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| {
        let path = repository().join(DEFAULT_DATA);
        std::fs::read(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
    })
}

/// The shipped data file, opened and verified.
fn shipped() -> &'static Preload {
    static PRELOAD: OnceLock<Preload> = OnceLock::new();
    PRELOAD.get_or_init(|| {
        Preload::open(shipped_bytes().to_vec())
            .unwrap_or_else(|error| panic!("the shipped HSTS preload list does not open: {error}"))
    })
}

/// The text of the manifest next to the shipped data file.
fn manifest_text() -> String {
    let path = repository()
        .join(DEFAULT_DATA)
        .with_extension("manifest.json");
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

/// The manifest next to the shipped data file, parsed.
fn manifest() -> Manifest {
    Manifest::parse(&manifest_text())
        .unwrap_or_else(|error| panic!("the shipped manifest does not parse: {error}"))
}

#[test]
fn the_shipped_list_passes_the_check_with_its_manifest_and_the_default_limits() {
    let manifest = manifest();
    let verdict = check(
        shipped_bytes(),
        Some(&manifest),
        Date::today_utc(),
        &Limits::default(),
    );
    assert!(
        verdict.ok(),
        "problems: {:#?}\nnotes: {:#?}",
        verdict.problems,
        verdict.notes
    );
    if manifest.source.retrieved.is_none() {
        // The copy taken from Servo records no retrieval date, and the check says so.
        assert!(
            verdict
                .notes
                .iter()
                .any(|note| note.starts_with("age unknown")),
            "{:#?}",
            verdict.notes
        );
    }
}

#[test]
fn every_entry_is_a_valid_lower_case_host_name() {
    let entries = shipped().entries();
    assert_eq!(entries.len(), shipped().len());
    for entry in &entries {
        assert_eq!(
            valid_host(&entry.host),
            Ok(entry.host.clone()),
            "{:?}",
            entry.host
        );
    }
}

#[test]
fn the_canary_hosts_are_listed() {
    // Every canary is in the file and includes its subdomains. (`paypal.com` is not a canary: the
    // baseline lists it, and `www.paypal.com`, without the flag.)
    for host in CANARIES {
        assert_eq!(shipped().get(host), Some(true), "{host}");
    }
    // Lookups the browser makes: a listed host, a subdomain of one that includes them, and a
    // name that is not preloaded.
    assert!(shipped().is_secure("github.com"));
    assert!(shipped().is_secure("gist.github.com"));
    assert!(shipped().is_secure("anything.at.all.dev"));
    assert!(!shipped().is_secure("this-host-is-not-preloaded.example"));
}

#[test]
fn the_manifest_counts_and_hash_describe_the_shipped_file() {
    let manifest = manifest();
    assert_eq!(manifest.entries, shipped().len());
    assert_eq!(
        manifest.include_subdomains,
        shipped().include_subdomains_count()
    );
    assert_eq!(manifest.output_sha256, sha256_hex(shipped_bytes()));
    assert_eq!(manifest.format, FORMAT);
}

#[test]
fn the_manifest_is_written_the_way_the_tool_writes_it() {
    // A hand edit that sorts, indents or ends differently would show up as noise in the first
    // refresh's `git diff`; the text must be exactly what `Manifest::to_json` prints.
    let text = manifest_text();
    assert_eq!(manifest().to_json(), text);
}

#[test]
fn until_a_refresh_the_file_is_the_one_servo_publishes() {
    let manifest = manifest();
    if manifest.source.url.is_none() {
        // The baseline: a copy of `servo-default-resources` 0.8.0, which the manifest says.
        let servo =
            repository().join("third_party/servo-default-resources/resources/hsts_preload.fstmap");
        let published =
            std::fs::read(&servo).unwrap_or_else(|error| panic!("cannot read {servo:?}: {error}"));
        assert!(
            shipped_bytes() == published.as_slice(),
            "the manifest says this is Servo's file, but it differs from {servo:?}"
        );
        assert!(manifest.source.name.contains("servo-default-resources"));
        assert!(manifest.source.retrieved.is_none());
    } else {
        // A refresh names where it came from and what it read.
        let source = &manifest.source;
        assert!(source.retrieved.is_some(), "a refreshed list has a date");
        assert!(source.git_ref.is_some(), "a refreshed list names its ref");
        assert!(
            source.input_sha256.is_some(),
            "a refreshed list hashes its input"
        );
        assert!(source.objects.is_some() && source.force_https.is_some());
        assert!(
            manifest.tool.starts_with("hsts-preload "),
            "{}",
            manifest.tool
        );
    }
}

#[test]
fn the_tools_default_output_is_the_file_the_browser_compiles_in() {
    // The browser's reader compiles `crates/talaria-servo/resources/hsts_preload.fstmap` in, and
    // its own test (`mercury_serves_its_own_hsts_preload_list`) compares the bytes Servo is
    // served with the file at exactly that path. The tool's default output must be that path, or
    // a refresh would rewrite a file nobody reads.
    assert_eq!(
        DEFAULT_DATA,
        "crates/talaria-servo/resources/hsts_preload.fstmap"
    );
    assert!(repository().join(DEFAULT_DATA).is_file());
}

#[test]
fn the_data_file_and_its_manifest_sit_side_by_side() {
    // `hsts-preload check` finds the manifest by replacing `.fstmap` with `.manifest.json`.
    let data = repository().join(DEFAULT_DATA);
    let manifest_path = data.with_extension("manifest.json");
    assert!(data.is_file(), "{data:?}");
    assert!(manifest_path.is_file(), "{manifest_path:?}");
    assert_eq!(
        manifest_path.file_name().and_then(|name| name.to_str()),
        Some("hsts_preload.manifest.json")
    );
}
