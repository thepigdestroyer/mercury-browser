// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The HSTS preload data file (`hsts::fstmap`): building it, reading it back the way Servo does,
//! refusing damaged files, and replacing a file atomically. The last group reads the file Servo
//! 0.8.0 ships (`third_party/servo-default-resources/resources/hsts_preload.fstmap`), the
//! baseline the tool must agree with.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use fst::{Map, MapBuilder, Streamer};
use mercury_devtools::hsts::fstmap::{Preload, build, write_atomic};
use mercury_devtools::hsts::{CANARIES, Entry};
use sha2::{Digest, Sha256};

fn entry(host: &str, include_subdomains: bool) -> Entry {
    Entry {
        host: host.to_string(),
        include_subdomains,
    }
}

/// Six hosts that exercise every case of the lookup: include and non-include entries, a TLD
/// entry, a non-include parent with an include child, a host that sorts first (digits).
fn small_list() -> Vec<Entry> {
    vec![
        entry("google.com", true),
        entry("example.org", false),
        entry("dev", true),
        entry("mail.example.net", false),
        entry("deep.mail.example.net", true),
        entry("0--1.de", true),
    ]
}

/// The same hosts in key (byte) order.
fn small_list_sorted() -> Vec<Entry> {
    let mut sorted = small_list();
    sorted.sort();
    sorted
}

/// A file written with the raw `fst` builder, the way another tool (or a bug) could write one.
/// The keys must already be in byte order.
fn raw(pairs: &[(&[u8], u64)]) -> Vec<u8> {
    let mut builder = MapBuilder::memory();
    for (key, value) in pairs {
        builder.insert(key, *value).expect("keys in order");
    }
    builder.into_inner().expect("finish")
}

fn plain_map(bytes: &[u8]) -> Map<Vec<u8>> {
    Map::new(bytes.to_vec()).expect("a valid fst map")
}

/// Servo's `HstsPreloadList::is_host_secure` (`third_party/servo-net/hsts.rs:123-147`), copied,
/// with the start of the walk given: Servo passes `reg_suffix(host)` (the registrable domain).
fn servo_walk(map: &Map<Vec<u8>>, host: &str, base_domain: &str) -> bool {
    let parts = host[..host.len() - base_domain.len()].rsplit_terminator('.');
    let mut domain_to_test = base_domain.to_owned();

    if map.get(&domain_to_test).is_some_and(|id| {
        // The FST map ids were constructed such that the parity represents the includeSubdomain flag
        id % 2 == 1 || domain_to_test == host
    }) {
        return true;
    }

    // Check all further subdomains up to the passed host
    for part in parts {
        domain_to_test = format!("{}.{}", part, domain_to_test);
        if map.get(&domain_to_test).is_some_and(|id| {
            // The FST map ids were constructed such that the parity represents the includeSubdomain flag
            id % 2 == 1 || domain_to_test == host
        }) {
            return true;
        }
    }
    false
}

/// Servo's rule with `reg_suffix` replaced by the last label, as the contract says: every
/// parent of the host down to the last label is tried.
fn servo_rule(map: &Map<Vec<u8>>, host: &str) -> bool {
    let last_label = host.rsplit('.').next().unwrap_or(host);
    servo_walk(map, host, last_label)
}

/// A model of what Servo really does for a host below a one-label public suffix such as `dev`:
/// `reg_suffix` (`third_party/servo-net-traits/pub_domains.rs:147`) is the registrable domain,
/// the public suffix plus one label, so for `example.dev` it is `example.dev` itself and the
/// walk never tests `dev`. Here the last two labels stand for it.
fn servo_model(map: &Map<Vec<u8>>, host: &str) -> bool {
    let registrable = match host.rmatch_indices('.').nth(1) {
        Some((index, _)) => &host[index + 1..],
        None => host,
    };
    servo_walk(map, host, registrable)
}

/// Host names that cover every outcome for `small_list`.
const HOSTS: [&str; 24] = [
    // listed exactly: include, non-include, TLD
    "google.com",
    "example.org",
    "dev",
    "0--1.de",
    "mail.example.net",
    "deep.mail.example.net",
    // below an include entry
    "www.google.com",
    "a.b.google.com",
    "x.0--1.de",
    "x.deep.mail.example.net",
    // below a non-include entry
    "www.example.org",
    "a.www.example.org",
    "other.mail.example.net",
    // below the TLD entry
    "example.dev",
    "a.b.dev",
    // unrelated, or only a parent of an entry
    "example.com",
    "evilgoogle.com",
    "google.com.evil.org",
    "com",
    "org",
    "net",
    "example.net",
    "de",
    "oogle.com",
];

/// Strings that are not host names (or not lower case): nothing is found and nothing panics.
const ODD_HOSTS: [&str; 8] = [
    "",
    ".",
    "google.com.",
    "a..b",
    ".google.com",
    "GOOGLE.COM",
    "www.GOOGLE.com",
    "dev.",
];

// ---- building and reading back -------------------------------------------------------------

#[test]
fn a_built_list_reads_back_entry_for_entry() {
    let bytes = build(&small_list()).expect("build");
    let preload = Preload::open(bytes).expect("open");
    assert_eq!(preload.len(), 6);
    assert!(!preload.is_empty());
    assert_eq!(preload.include_subdomains_count(), 4);
    assert_eq!(preload.entries(), small_list_sorted());
    assert_eq!(preload.get("google.com"), Some(true));
    assert_eq!(preload.get("example.org"), Some(false));
    assert_eq!(preload.get("dev"), Some(true));
    assert_eq!(preload.get("mail.example.net"), Some(false));
    // get is exact: a parent or a child of an entry is not an entry.
    assert_eq!(preload.get("www.google.com"), None);
    assert_eq!(preload.get("example.net"), None);
    assert_eq!(preload.get("Google.com"), None);
    assert_eq!(preload.get(""), None);
}

#[test]
fn the_values_are_two_times_the_rank_plus_the_flag_and_plain_fst_reads_them() {
    let bytes = build(&small_list()).expect("build");
    // Keys in byte order: 0--1.de, deep.mail.example.net, dev, example.org, google.com, mail.example.net.
    let mut expected = vec![
        ("0--1.de", 1_u64),
        ("deep.mail.example.net", 3),
        ("dev", 5),
        ("example.org", 6),
        ("google.com", 9),
        ("mail.example.net", 10),
    ]
    .into_iter();
    let map = plain_map(&bytes);
    let mut stream = map.stream();
    while let Some((key, value)) = stream.next() {
        let (host, id) = expected.next().expect("no more keys than expected");
        assert_eq!(std::str::from_utf8(key).expect("utf-8"), host);
        assert_eq!(value, id, "the value of {host}");
    }
    assert!(expected.next().is_none(), "every expected key was present");
}

#[test]
fn the_bytes_are_an_fst_version_3_file_servo_reads() {
    let bytes = build(&small_list()).expect("build");
    // Version 3 (the one with a checksum), little endian, as the shipped file starts.
    assert_eq!(&bytes[..8], &[3, 0, 0, 0, 0, 0, 0, 0]);
    // What Servo does: Map::new, nothing more; and the checksum verifies.
    let map = Map::new(bytes).expect("Servo's Map::new");
    map.as_fst().verify().expect("the checksum");
    assert_eq!(map.len(), 6);
}

#[test]
fn the_same_entries_give_the_same_bytes_in_any_order() {
    let list = small_list();
    let forward = build(&list).expect("build");
    let mut reversed = list.clone();
    reversed.reverse();
    let mut rotated = list.clone();
    rotated.rotate_left(2);
    assert_eq!(forward, build(&reversed).expect("build reversed"));
    assert_eq!(forward, build(&rotated).expect("build rotated"));
    assert_eq!(forward, build(&small_list_sorted()).expect("build sorted"));
    assert_eq!(forward, build(&list).expect("build again"));
}

#[test]
fn a_flag_changes_the_bytes() {
    let mut other = small_list();
    other[1].include_subdomains = true; // example.org
    assert_ne!(
        build(&small_list()).expect("build"),
        build(&other).expect("build changed")
    );
}

#[test]
fn a_larger_generated_list_round_trips_and_follows_the_scheme() {
    let mut list = Vec::new();
    for i in 0..2000_u32 {
        list.push(entry(
            &format!("h{i}.example{}.com", i % 7),
            i % 3 == 0 || i % 11 == 0,
        ));
    }
    // A cheap deterministic shuffle: the order the entries arrive in must not matter.
    let mut shuffled = list.clone();
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    for i in (1..shuffled.len()).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        shuffled.swap(i, (state >> 33) as usize % (i + 1));
    }
    let bytes = build(&shuffled).expect("build");
    assert_eq!(bytes, build(&list).expect("build unshuffled"));
    let preload = Preload::open(bytes.clone()).expect("open");
    assert_eq!(preload.len(), 2000);
    list.sort();
    assert_eq!(preload.entries(), list);
    let expected_includes = list.iter().filter(|e| e.include_subdomains).count();
    assert_eq!(preload.include_subdomains_count(), expected_includes);
    assert_eq!(preload.id_scheme_violation(), None);
    assert!(preload.rebuilds_identically(&bytes));
}

#[test]
fn an_empty_list_builds_and_opens() {
    let bytes = build(&[]).expect("build");
    assert_eq!(&bytes[..8], &[3, 0, 0, 0, 0, 0, 0, 0]);
    let preload = Preload::open(bytes.clone()).expect("open");
    assert_eq!(preload.len(), 0);
    assert!(preload.is_empty());
    assert_eq!(preload.include_subdomains_count(), 0);
    assert!(preload.entries().is_empty());
    assert_eq!(preload.get("google.com"), None);
    assert!(!preload.is_secure("google.com"));
    assert!(!preload.is_secure(""));
    assert_eq!(preload.id_scheme_violation(), None);
    assert!(preload.rebuilds_identically(&bytes));
}

#[test]
fn a_host_listed_twice_is_an_error_whatever_the_flags() {
    for flags in [(true, true), (false, false), (true, false), (false, true)] {
        let list = vec![
            entry("a.example", true),
            entry("twice.example", flags.0),
            entry("z.example", false),
            entry("twice.example", flags.1),
        ];
        let error = build(&list).expect_err("a duplicate host");
        assert!(error.contains("twice.example"), "{error}");
        assert!(error.contains("more than once"), "{error}");
    }
}

#[test]
fn an_invalid_host_is_an_error_that_names_it() {
    let too_long_label = format!("{}.example", "a".repeat(64));
    let too_long_name = format!("{}.com", ["abcdefghi"; 28].join("."));
    assert!(too_long_name.len() > 253);
    let bad = [
        "",
        "Example.com",
        "exa mple.com",
        "a_b.example",
        "a*.example",
        "trailing.example.",
        ".leading.example",
        "double..dot.example",
        "b\u{fc}cher.example",
        &too_long_label,
        &too_long_name,
    ];
    for host in bad {
        let list = vec![entry("fine.example", true), entry(host, true)];
        let error = build(&list).expect_err(host);
        assert!(
            error.starts_with("cannot write the HSTS list: "),
            "{host:?}: {error}"
        );
        if host.len() < 100 {
            assert!(error.contains(&format!("{host:?}")), "{host:?}: {error}");
        }
    }
    let error = build(&[entry("Example.com", true)]).expect_err("upper case");
    assert!(error.contains("lower-case"), "{error}");
}

// ---- looking hosts up ----------------------------------------------------------------------

#[test]
fn is_secure_agrees_with_servos_rule_over_a_grid_of_hosts() {
    let bytes = build(&small_list()).expect("build");
    let preload = Preload::open(bytes.clone()).expect("open");
    let map = plain_map(&bytes);
    for host in HOSTS.into_iter().chain(ODD_HOSTS) {
        assert_eq!(
            preload.is_secure(host),
            servo_rule(&map, host),
            "is_secure and Servo's rule disagree on {host:?}"
        );
    }
}

#[test]
fn is_secure_answers_the_cases_it_is_meant_for() {
    let preload = Preload::open(build(&small_list()).expect("build")).expect("open");
    let secure = [
        "google.com",
        "www.google.com",
        "a.b.google.com",
        "example.org", // listed without include_subdomains: the host itself is covered
        "dev",
        "example.dev",
        "a.b.dev",
        "0--1.de",
        "x.0--1.de",
        "mail.example.net",
        "deep.mail.example.net",
        "x.deep.mail.example.net", // a non-include parent does not hide an include child
    ];
    for host in secure {
        assert!(preload.is_secure(host), "{host} should be secure");
    }
    let insecure = [
        "www.example.org", // below a non-include entry
        "a.www.example.org",
        "other.mail.example.net",
        "example.com",
        "evilgoogle.com",
        "google.com.evil.org",
        "com",
        "org",
        "net",
        "example.net", // the parent of an entry is not covered by it
        "de",
        "oogle.com",
        "",
        ".",
        "google.com.", // a trailing dot is not stripped
        "GOOGLE.COM",  // the host of a URL is lower case already: no folding here
    ];
    for host in insecure {
        assert!(!preload.is_secure(host), "{host:?} should not be secure");
    }
}

#[test]
fn servo_never_looks_at_a_listed_public_suffix_and_is_secure_does() {
    // `servo_rule` above starts at the last label, as the contract says. Real Servo starts at the
    // registrable domain, so the listed public suffix `dev` covers only the bare host there.
    // This documents the one difference (see the doc comment of `Preload::is_secure`): it is
    // what happens to the TLD entries of the shipped list (`dev`, `app`, ...) in Servo 0.8.0.
    let bytes = build(&small_list()).expect("build");
    let preload = Preload::open(bytes.clone()).expect("open");
    let map = plain_map(&bytes);
    let differing: BTreeSet<&str> = HOSTS
        .into_iter()
        .filter(|host| servo_model(&map, host) != preload.is_secure(host))
        .collect();
    assert_eq!(
        differing,
        BTreeSet::from(["a.b.dev", "example.dev"]),
        "only the hosts below the listed public suffix differ"
    );
    // The bare TLD and every name that is not below a listed public suffix agree.
    assert!(servo_model(&map, "dev") && preload.is_secure("dev"));
}

// ---- damaged and foreign files -------------------------------------------------------------

#[test]
fn an_empty_file_and_a_short_one_are_refused() {
    let error = Preload::open(Vec::new()).expect_err("empty");
    assert!(error.contains("not an fst map"), "{error}");
    let error = Preload::open(vec![3, 0, 0, 0, 0, 0, 0, 0, 0, 0]).expect_err("ten bytes");
    assert!(error.contains("not an fst map"), "{error}");
}

#[test]
fn a_truncated_file_is_refused() {
    let bytes = build(&small_list()).expect("build");
    for cut in [1, 2, 4, 5, 20, bytes.len() / 2, bytes.len() - 20] {
        let truncated = bytes[..bytes.len() - cut].to_vec();
        assert!(
            Preload::open(truncated).is_err(),
            "a file {cut} bytes short must be refused"
        );
    }
    let mut longer = bytes.clone();
    longer.extend_from_slice(&[0, 0, 0, 0]);
    assert!(Preload::open(longer).is_err(), "trailing bytes are refused");
}

#[test]
fn a_flipped_byte_in_the_body_is_caught_by_the_checksum() {
    let bytes = build(&small_list()).expect("build");
    let mut damaged = bytes.clone();
    let middle = bytes.len() / 2;
    damaged[middle] ^= 0x01;
    // Why open verifies: Servo's Map::new reads only the header and footer, so it loads this.
    assert!(
        Map::new(damaged.clone()).is_ok(),
        "Map::new does not notice"
    );
    let error = Preload::open(damaged).expect_err("the checksum fails");
    assert!(error.contains("checksum"), "{error}");
    // The checksum itself damaged.
    let mut damaged = bytes.clone();
    *damaged.last_mut().expect("bytes") ^= 0xff;
    assert!(
        Map::new(damaged.clone()).is_ok(),
        "Map::new does not notice"
    );
    let error = Preload::open(damaged).expect_err("the checksum fails");
    assert!(error.contains("checksum"), "{error}");
}

#[test]
fn a_file_without_a_checksum_is_refused_though_servo_reads_it() {
    // Format version 2 (fst before 0.4) is version 3 without the last four bytes.
    let mut old = build(&small_list()).expect("build");
    old.truncate(old.len() - 4);
    old[0] = 2;
    assert!(Map::new(old.clone()).is_ok(), "Servo can read version 2");
    let error = Preload::open(old).expect_err("no checksum to verify");
    assert!(error.contains("checksum"), "{error}");
}

#[test]
fn a_file_that_is_not_an_fst_is_refused() {
    let text = b"This is a text file, not an fst map, and it is longer than thirty-six bytes.";
    let error = Preload::open(text.to_vec()).expect_err("text");
    assert!(error.contains("not an fst map"), "{error}");
    let error = Preload::open(vec![0; 64]).expect_err("zeros");
    assert!(error.contains("not an fst map"), "{error}");
    let json = br#"{"entries": [{"name": "google.com", "mode": "force-https"}], "pinsets": []}"#;
    assert!(Preload::open(json.to_vec()).is_err());
}

#[test]
fn a_key_that_is_not_a_lower_case_host_name_is_refused_and_named() {
    // Byte order: upper case sorts before lower case.
    let error = Preload::open(raw(&[(b"Example.com", 1), (b"a.com", 3)])).expect_err("upper case");
    assert!(error.contains("Example.com"), "{error}");
    assert!(error.contains("lower-case"), "{error}");
    assert!(error.contains("key number 1"), "{error}");

    let error = Preload::open(raw(&[(b"a b.com", 1)])).expect_err("a space");
    assert!(error.contains("a b.com"), "{error}");

    let error = Preload::open(raw(&[(b"a.com", 1), (b"b_c.com", 3)])).expect_err("underscore");
    assert!(error.contains("b_c.com"), "{error}");
    assert!(error.contains("key number 2"), "{error}");

    let error = Preload::open(raw(&[(b"", 1)])).expect_err("empty key");
    assert!(error.contains("empty"), "{error}");

    let error = Preload::open(raw(&[(b"a.", 1)])).expect_err("trailing dot");
    assert!(error.contains("a."), "{error}");

    let error = Preload::open(raw(&[(b"a.com", 1), (&[0xff, 0xfe], 3)])).expect_err("not UTF-8");
    assert!(error.contains("not UTF-8"), "{error}");
}

#[test]
fn the_first_bad_key_in_key_order_is_the_one_reported() {
    let bytes = raw(&[
        (b"a.com", 1),
        (b"b c.com", 3),
        (b"d_e.com", 5),
        (b"fine.com", 7),
    ]);
    let error = Preload::open(bytes).expect_err("bad keys");
    assert!(error.contains("b c.com"), "{error}");
    assert!(!error.contains("d_e.com"), "{error}");
}

// ---- the value scheme ----------------------------------------------------------------------

#[test]
fn a_file_with_only_parity_values_opens_and_reports_the_scheme_breach() {
    // What the `fst` command line would write: 0 or 1 as the value, no ranks.
    let bytes = raw(&[(b"a.com", 1), (b"b.com", 0), (b"c.com", 1)]);
    let preload = Preload::open(bytes.clone()).expect("Servo can read it, so open accepts it");
    assert_eq!(preload.len(), 3);
    assert_eq!(preload.include_subdomains_count(), 2);
    assert_eq!(preload.get("a.com"), Some(true));
    assert_eq!(preload.get("b.com"), Some(false));
    assert!(preload.is_secure("www.a.com"));
    assert!(!preload.is_secure("www.b.com"));
    let violation = preload.id_scheme_violation().expect("a breach");
    // a.com (position 0, flag 1) is right: 2 * 0 + 1. b.com (position 1, flag 0) should be 2.
    assert!(violation.contains("b.com"), "{violation}");
    assert!(violation.contains("position 1"), "{violation}");
    assert!(violation.contains("value 0"), "{violation}");
    assert!(violation.contains("gives 2"), "{violation}");
    assert!(!preload.rebuilds_identically(&bytes));
}

#[test]
fn the_scheme_breach_is_found_at_the_first_key_that_has_one() {
    // The first key's value is 2 * 0 + flag: 3 is a breach there.
    let preload = Preload::open(raw(&[(b"a.com", 3)])).expect("open");
    let violation = preload.id_scheme_violation().expect("a breach");
    assert!(violation.contains("a.com"), "{violation}");
    assert!(violation.contains("gives 1"), "{violation}");
    // Parity right but a gap in the ranks: 1, then 5 where 3 belongs.
    let preload = Preload::open(raw(&[(b"a.com", 1), (b"b.com", 5), (b"c.com", 7)])).expect("open");
    let violation = preload.id_scheme_violation().expect("a gap");
    assert!(violation.contains("b.com"), "{violation}");
    assert!(violation.contains("value 5"), "{violation}");
    assert!(violation.contains("gives 3"), "{violation}");
    // A file that follows the scheme reports nothing.
    let preload = Preload::open(raw(&[(b"a.com", 1), (b"b.com", 2), (b"c.com", 5)])).expect("open");
    assert_eq!(preload.id_scheme_violation(), None);
}

#[test]
fn a_built_file_follows_the_scheme_and_rebuilds_to_the_same_bytes() {
    let bytes = build(&small_list()).expect("build");
    let preload = Preload::open(bytes.clone()).expect("open");
    assert_eq!(preload.id_scheme_violation(), None);
    assert!(preload.rebuilds_identically(&bytes));
    // Different bytes are not "identical", even when the entries are.
    let mut other = bytes.clone();
    other.push(0);
    assert!(!preload.rebuilds_identically(&other));
    assert!(!preload.rebuilds_identically(&[]));
}

#[test]
fn the_debug_output_gives_counts_and_not_the_bytes() {
    let preload = Preload::open(build(&small_list()).expect("build")).expect("open");
    let text = format!("{preload:?}");
    assert!(text.contains("Preload"), "{text}");
    assert!(text.contains("entries: 6"), "{text}");
    assert!(text.len() < 100, "{text}");
}

// ---- write_atomic --------------------------------------------------------------------------

/// The names in a directory, sorted.
fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("read the directory")
        .map(|item| {
            item.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn write_atomic_writes_a_new_file_and_leaves_nothing_else() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("hsts_preload.fstmap");
    let bytes = build(&small_list()).expect("build");
    write_atomic(&path, &bytes).expect("write");
    assert_eq!(std::fs::read(&path).expect("read back"), bytes);
    assert_eq!(names(directory.path()), ["hsts_preload.fstmap"]);
}

#[test]
fn write_atomic_replaces_an_existing_file_and_leaves_no_temporary_file() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("hsts_preload.fstmap");
    std::fs::write(&path, b"the old file").expect("seed");
    let bytes = build(&small_list()).expect("build");
    write_atomic(&path, &bytes).expect("replace");
    assert_eq!(std::fs::read(&path).expect("read back"), bytes);
    // A second and a shorter write, to be sure nothing of the longer one stays.
    write_atomic(&path, b"x").expect("replace again");
    assert_eq!(std::fs::read(&path).expect("read back"), b"x");
    assert_eq!(names(directory.path()), ["hsts_preload.fstmap"]);
}

#[test]
fn write_atomic_needs_the_directory_to_exist_and_does_not_create_it() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("missing").join("hsts_preload.fstmap");
    let error = write_atomic(&path, b"data").expect_err("no directory");
    assert!(error.contains("does not exist"), "{error}");
    assert!(error.contains("missing"), "{error}");
    assert!(names(directory.path()).is_empty(), "nothing was created");
}

#[test]
fn write_atomic_leaves_the_destination_alone_when_the_rename_cannot_happen() {
    let directory = tempfile::tempdir().expect("tempdir");
    // The destination is a directory that holds a file: a file cannot replace it.
    let path = directory.path().join("hsts_preload.fstmap");
    std::fs::create_dir(&path).expect("directory");
    std::fs::write(path.join("kept.txt"), b"kept").expect("seed");
    let error = write_atomic(&path, b"new bytes").expect_err("cannot replace a directory");
    assert!(error.contains("cannot replace"), "{error}");
    assert_eq!(
        std::fs::read(path.join("kept.txt")).expect("still there"),
        b"kept"
    );
    assert_eq!(
        names(directory.path()),
        ["hsts_preload.fstmap"],
        "no temporary file"
    );
    assert_eq!(names(&path), ["kept.txt"]);
}

#[test]
fn write_atomic_refuses_a_path_that_names_no_file() {
    let directory = tempfile::tempdir().expect("tempdir");
    let error = write_atomic(&directory.path().join(".."), b"x").expect_err("no file name");
    assert!(error.contains("does not name a file"), "{error}");
    let error = write_atomic(Path::new("/"), b"x").expect_err("the root");
    assert!(error.contains("does not name a file"), "{error}");
}

#[cfg(unix)]
#[test]
fn write_atomic_keeps_an_existing_file_when_the_directory_is_read_only() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("hsts_preload.fstmap");
    std::fs::write(&path, b"the good file").expect("seed");
    let read_only = std::fs::Permissions::from_mode(0o555);
    std::fs::set_permissions(directory.path(), read_only).expect("chmod");
    let result = write_atomic(&path, b"replacement");
    // Restore first, so the temporary directory can be removed whatever happens next.
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755))
        .expect("chmod back");
    match result {
        Err(error) => {
            assert!(error.contains("cannot write"), "{error}");
            assert_eq!(std::fs::read(&path).expect("read"), b"the good file");
        }
        // Running as root: the directory permissions do not apply, and the write is right.
        Ok(()) => assert_eq!(std::fs::read(&path).expect("read"), b"replacement"),
    }
    assert_eq!(names(directory.path()), ["hsts_preload.fstmap"]);
}

// ---- the file Servo 0.8.0 ships ------------------------------------------------------------

fn shipped_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../third_party/servo-default-resources/resources/hsts_preload.fstmap")
}

/// The shipped bytes, read once, opened with `Preload` and with plain `fst::Map` (Servo's reader).
fn shipped() -> &'static (Vec<u8>, Preload, Map<Vec<u8>>) {
    static SHIPPED: OnceLock<(Vec<u8>, Preload, Map<Vec<u8>>)> = OnceLock::new();
    SHIPPED.get_or_init(|| {
        let bytes = std::fs::read(shipped_path()).expect("read the shipped hsts_preload.fstmap");
        let preload = Preload::open(bytes.clone()).expect("the shipped file opens");
        let map = Map::new(bytes.clone()).expect("Servo's Map::new reads it");
        (bytes, preload, map)
    })
}

#[test]
fn the_shipped_file_is_the_one_that_was_measured() {
    let (bytes, preload, _) = shipped();
    assert_eq!(bytes.len(), 1_949_725);
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "b94818a7e0c3515f9fd6a50af51e32019cbb50ee1015e1e56471fb3b06d72c42"
    );
    assert_eq!(&bytes[..8], &[3, 0, 0, 0, 0, 0, 0, 0], "fst version 3");
    assert_eq!(preload.len(), 166_958);
    assert_eq!(preload.include_subdomains_count(), 166_708);
    assert_eq!(preload.entries().len(), 166_958);
    let without_flag = preload
        .entries()
        .iter()
        .filter(|e| !e.include_subdomains)
        .count();
    assert_eq!(without_flag, 250);
}

#[test]
fn every_entry_of_the_shipped_file_follows_the_id_scheme() {
    // The baseline follows `2 * rank + include_subdomains` for all 166,958 entries, so the id
    // scheme is a hard rule for Mercury's own file (decision D6 needs no fallback).
    let (_, preload, _) = shipped();
    assert_eq!(preload.id_scheme_violation(), None);
}

#[test]
fn the_shipped_file_has_the_canaries_and_the_hosts_the_tool_relies_on() {
    let (_, preload, _) = shipped();
    // These carry include_subdomains in the shipped file.
    for host in [
        "google.com",
        "github.com",
        "dev",
        "dropbox.com",
        "stripe.com",
        "cloudflare.com",
    ] {
        assert_eq!(preload.get(host), Some(true), "{host}");
    }
    // Every canary the tool checks is listed and secure.
    for host in CANARIES {
        assert!(preload.get(host).is_some(), "canary {host} is listed");
        assert!(preload.is_secure(host), "canary {host}");
    }
    for host in [
        "facebook.com",
        "app",
        "mail.google.com",
        "pinningtest.appspot.com",
    ] {
        assert!(preload.get(host).is_some(), "{host} is listed");
    }
    // Chromium's test host is kept (its "policy" is "test"): the old script did not drop it.
    assert_eq!(preload.get("pinningtest.appspot.com"), Some(true));
    for host in ["example.com", "gov"] {
        assert_eq!(preload.get(host), None, "{host} is not listed");
    }
}

#[test]
fn paypal_com_is_listed_without_include_subdomains_in_the_shipped_file() {
    // Measured on the file Servo 0.8.0 ships: paypal.com and www.paypal.com are both listed with
    // the flag off (PayPal asked for the exact names only). A guard that requires
    // `include_subdomains` on the canary `paypal.com` therefore refuses this very file.
    let (_, preload, _) = shipped();
    assert_eq!(preload.get("paypal.com"), Some(false));
    assert_eq!(preload.get("www.paypal.com"), Some(false));
    assert!(preload.is_secure("paypal.com"));
    assert!(!preload.is_secure("login.paypal.com"));
    // So do facebook.com, twitter.com and gmail.com: the 250 hosts without the flag are real.
    for host in ["facebook.com", "twitter.com", "gmail.com"] {
        assert_eq!(preload.get(host), Some(false), "{host}");
    }
}

#[test]
fn the_shipped_file_has_51_one_label_entries_and_24_other_public_suffix_entries() {
    // Why `is_secure` differs from Servo's walk (see its doc comment): Servo starts at the
    // registrable domain of the host, so an entry that is itself a public suffix covers only the
    // bare host in Servo 0.8.0. In the shipped file these are all the one-label entries (`dev`,
    // `app`, `google`, ...) and 24 more (`appspot.com`, `blogspot.com`, `now.sh`, ...), counted
    // with the public suffix list Servo ships (`public_domains.txt`, rules, wildcards, exceptions,
    // read as `third_party/servo-net-traits/pub_domains.rs` reads them).
    let (_, preload, _) = shipped();
    let list = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../third_party/servo-default-resources/resources/public_domains.txt"),
    )
    .expect("read the public suffix list");
    let (mut rules, mut wildcards, mut exceptions) =
        (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for line in list.lines().map(str::trim) {
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(rest) = line.strip_prefix('!') {
            exceptions.insert(rest);
        } else if let Some(rest) = line.strip_prefix("*.") {
            wildcards.insert(rest);
        } else {
            rules.insert(line);
        }
    }
    let is_public_suffix = |name: &str| match name.split_once('.') {
        None => !name.is_empty(),
        Some((_, parent)) => {
            (!exceptions.contains(name) && wildcards.contains(parent)) || rules.contains(name)
        }
    };
    let entries = preload.entries();
    let one_label: Vec<&Entry> = entries.iter().filter(|e| !e.host.contains('.')).collect();
    let others: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.host.contains('.') && is_public_suffix(&e.host))
        .collect();
    assert_eq!(one_label.len(), 51);
    assert!(one_label.iter().all(|e| e.include_subdomains));
    assert!(one_label.iter().any(|e| e.host == "dev"));
    assert_eq!(others.len(), 24);
    for host in ["appspot.com", "blogspot.com", "googleapis.com", "now.sh"] {
        assert!(others.iter().any(|e| e.host == host), "{host}");
    }
}

#[test]
fn the_shipped_file_secures_the_hosts_it_should() {
    let (_, preload, _) = shipped();
    for host in [
        "google.com",
        "www.google.com",
        "a.b.c.google.com",
        "dev",
        "example.dev",
    ] {
        assert!(preload.is_secure(host), "{host}");
    }
    for host in [
        "example.com",
        "www.example.com",
        "notgoogle.com",
        "google.com.example",
        "com",
        "",
    ] {
        assert!(!preload.is_secure(host), "{host:?}");
    }
}

#[test]
fn is_secure_agrees_with_servos_rule_over_hosts_around_the_shipped_entries() {
    let (_, preload, map) = shipped();
    let entries = preload.entries();
    let mut checked = 0_usize;
    for entry in entries.iter().step_by(7) {
        let host = entry.host.as_str();
        let parent = host.split_once('.').map_or("", |(_, rest)| rest);
        let candidates = [
            host.to_string(),
            format!("www.{host}"),
            format!("a.b.{host}"),
            parent.to_string(),
            format!("x{host}"),
            format!("{host}.invalid"),
        ];
        for candidate in &candidates {
            assert_eq!(
                preload.is_secure(candidate),
                servo_rule(map, candidate),
                "is_secure and Servo's rule disagree on {candidate:?}"
            );
            checked += 1;
        }
        // What the entry itself means: the host is covered, and a name below it is covered when
        // it is listed itself or when the host or one of its parents includes subdomains.
        assert!(preload.is_secure(host), "{host}");
        let below = format!("sub.{host}");
        let expected = preload.get(&below).is_some() || has_flagged_parent(preload, host);
        assert_eq!(preload.is_secure(&below), expected, "{below}");
    }
    assert!(checked > 100_000, "{checked} lookups");
}

/// Whether `host` or one of its parents is listed with `include_subdomains`.
fn has_flagged_parent(preload: &Preload, host: &str) -> bool {
    let mut candidate = host;
    loop {
        if preload.get(candidate) == Some(true) {
            return true;
        }
        match candidate.split_once('.') {
            Some((_, rest)) => candidate = rest,
            None => return false,
        }
    }
}

#[test]
fn a_flipped_byte_in_the_shipped_file_is_caught() {
    let (bytes, _, _) = shipped();
    let mut damaged = bytes.clone();
    damaged[bytes.len() / 2] ^= 0x01;
    let error = Preload::open(damaged).expect_err("the checksum fails");
    assert!(error.contains("checksum"), "{error}");
}

#[test]
fn rebuilding_the_shipped_file_from_its_own_entries_writes_the_same_bytes() {
    // THE BYTE QUESTION, answered by measurement (20 Nov 2026, fst 0.4.7): `build` fed the 166,958
    // entries of Servo 0.8.0's file writes **exactly the same 1,949,725 bytes**. So the encoder
    // and the value scheme (2 * rank + flag) are the ones that produced Servo's file, and a
    // refresh with this tool changes the file only where the list changed. If an `fst` upgrade
    // ever changes the encoding this test fails, and `rebuilds_identically` (which `check`
    // reports as a note) is false for the shipped file: the file would still be valid.
    let (bytes, preload, _) = shipped();
    let rebuilt = build(&preload.entries()).expect("rebuild");
    let reopened = Preload::open(rebuilt.clone()).expect("the rebuilt file opens");
    assert_eq!(
        reopened.entries(),
        preload.entries(),
        "same entries and flags"
    );
    assert_eq!(reopened.id_scheme_violation(), None);
    assert_eq!(preload.rebuilds_identically(bytes), rebuilt == *bytes);
    assert!(
        preload.rebuilds_identically(bytes),
        "build writes different bytes from the shipped file"
    );
    assert_eq!(rebuilt.len(), 1_949_725);
}

#[test]
fn write_atomic_can_carry_the_whole_shipped_file() {
    let (bytes, _, _) = shipped();
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("hsts_preload.fstmap");
    write_atomic(&path, bytes).expect("write 1.9 MB");
    let read_back = std::fs::read(&path).expect("read");
    assert_eq!(&read_back, bytes);
    assert!(Preload::open(read_back).is_ok());
    assert_eq!(names(directory.path()), ["hsts_preload.fstmap"]);
}
