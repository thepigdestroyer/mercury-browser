// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `archive.rs`: the zip of a package, with its directory validated up front and every inflate counted.

mod package_common;

use std::fs;
use std::io::Write;
use std::path::Path;

use mercury_extensions::archive::{ArchiveError, Limits, NameProblem, ZipPackage};
use package_common::{RawEntry, XorShift, crc32, raw_zip, temp_dir, zip_of};

fn open(bytes: Vec<u8>) -> Result<ZipPackage, ArchiveError> {
    ZipPackage::open(bytes, &Limits::default())
}

fn open_with(bytes: Vec<u8>, limits: &Limits) -> Result<ZipPackage, ArchiveError> {
    ZipPackage::open(bytes, limits)
}

fn sample() -> Vec<u8> {
    zip_of(&[
        ("manifest.json", br#"{"manifest_version":3}"#),
        ("js/app.js", b"console.log('app');\n"),
        ("css/style.css", &[b'x'; 5000]),
    ])
}

fn unsafe_reason(result: Result<ZipPackage, ArchiveError>) -> NameProblem {
    match result.unwrap_err() {
        ArchiveError::UnsafeName { reason, .. } => reason,
        other => panic!("expected an unsafe name, got {other:?}"),
    }
}

fn one_named(name: &[u8]) -> Vec<u8> {
    raw_zip(&[RawEntry::stored("placeholder", b"x").raw_name(name)])
}

fn dir_is_empty(path: &Path) -> bool {
    fs::read_dir(path)
        .map(|mut d| d.next().is_none())
        .unwrap_or(false)
}

// ---- well-formed archives ----------------------------------------------------------------------

#[test]
fn valid_zip_lists_entries_and_reads_files() {
    let pkg = open(sample()).unwrap();
    let names: Vec<&str> = pkg.file_names().collect();
    assert_eq!(names, vec!["manifest.json", "js/app.js", "css/style.css"]);
    assert_eq!(
        pkg.read_file("manifest.json").unwrap(),
        br#"{"manifest_version":3}"#
    );
    assert_eq!(pkg.read_file("css/style.css").unwrap(), vec![b'x'; 5000]);
    assert!(pkg.contains("js/app.js"));
    assert_eq!(pkg.entry("js/app.js").unwrap().size, 20);
}

#[test]
fn stored_and_deflated_entries_both_read() {
    let bytes = raw_zip(&[
        RawEntry::stored("stored.txt", b"stored bytes"),
        RawEntry::deflated("deflated.txt", &b"deflated bytes ".repeat(50)),
    ]);
    let pkg = open(bytes).unwrap();
    assert_eq!(pkg.read_file("stored.txt").unwrap(), b"stored bytes");
    assert_eq!(
        pkg.read_file("deflated.txt").unwrap(),
        b"deflated bytes ".repeat(50)
    );
}

#[test]
fn empty_zip_opens_with_no_entries() {
    let pkg = open(raw_zip(&[])).unwrap();
    assert!(pkg.entries().is_empty());
    assert_eq!(pkg.read_file("x").unwrap_err(), ArchiveError::NotFound);
}

#[test]
fn zero_bytes_are_not_a_zip() {
    assert!(open(Vec::new()).is_err());
}

#[test]
fn garbage_is_not_a_zip() {
    assert_eq!(
        open(b"definitely not a zip archive".to_vec()).unwrap_err(),
        ArchiveError::Malformed
    );
    assert!(open(vec![0u8; 4096]).is_err());
    assert!(open(vec![0xFFu8; 4096]).is_err());
}

#[test]
fn every_truncation_of_a_valid_zip_is_refused() {
    let bytes = sample();
    for length in 0..bytes.len() {
        let truncated = bytes[..length].to_vec();
        assert!(open(truncated).is_err(), "length {length} was accepted");
    }
}

#[test]
fn a_prefix_before_the_zip_is_refused_by_open_and_read_by_open_at() {
    let zip = sample();
    let mut prefixed = b"Cr24 pretend header ".to_vec();
    let offset = prefixed.len() as u64;
    prefixed.extend_from_slice(&zip);
    assert!(open(prefixed.clone()).is_err());
    let pkg = ZipPackage::open_at(prefixed, offset, &Limits::default()).unwrap();
    assert_eq!(pkg.entries().len(), 3);
    assert_eq!(
        pkg.read_file("js/app.js").unwrap(),
        b"console.log('app');\n"
    );
}

#[test]
fn open_at_past_the_end_is_refused() {
    let zip = sample();
    let past = zip.len() as u64 + 1;
    assert!(ZipPackage::open_at(zip, past, &Limits::default()).is_err());
}

#[test]
fn zip64_archive_is_accepted() {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    writer.start_file("big.txt", options).unwrap();
    writer.write_all(b"zip64 content").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let pkg = open(bytes).unwrap();
    assert_eq!(pkg.read_file("big.txt").unwrap(), b"zip64 content");
}

#[test]
fn a_zero_dos_date_does_not_matter() {
    let mut entry = RawEntry::stored("manifest.json", b"{}");
    entry.dos_date = 0;
    let pkg = open(raw_zip(&[entry])).unwrap();
    assert_eq!(pkg.read_file("manifest.json").unwrap(), b"{}");
}

// ---- names -------------------------------------------------------------------------------------

#[test]
fn path_traversal_name_is_refused() {
    assert_eq!(
        unsafe_reason(open(one_named(b"../evil"))),
        NameProblem::ParentComponent
    );
}

#[test]
fn nested_traversal_name_is_refused() {
    assert_eq!(
        unsafe_reason(open(one_named(b"a/../../b"))),
        NameProblem::ParentComponent
    );
    assert_eq!(
        unsafe_reason(open(one_named(b"a/.."))),
        NameProblem::ParentComponent
    );
}

#[test]
fn absolute_name_is_refused() {
    assert_eq!(
        unsafe_reason(open(one_named(b"/etc/passwd"))),
        NameProblem::Absolute
    );
}

#[test]
fn windows_drive_names_are_refused() {
    assert_eq!(
        unsafe_reason(open(one_named(b"C:\\x"))),
        NameProblem::Backslash
    );
    assert_eq!(
        unsafe_reason(open(one_named(b"C:/x"))),
        NameProblem::DriveLetter
    );
    assert_eq!(
        unsafe_reason(open(one_named(b"z:x"))),
        NameProblem::DriveLetter
    );
}

#[test]
fn backslash_names_are_refused() {
    assert_eq!(
        unsafe_reason(open(one_named(b"a\\..\\b"))),
        NameProblem::Backslash
    );
    assert_eq!(
        unsafe_reason(open(one_named(b"dir\\file"))),
        NameProblem::Backslash
    );
}

#[test]
fn nul_in_a_name_is_refused() {
    assert_eq!(unsafe_reason(open(one_named(b"a\0b"))), NameProblem::Nul);
}

#[test]
fn control_characters_in_a_name_are_refused() {
    assert_eq!(
        unsafe_reason(open(one_named(b"a\x01b"))),
        NameProblem::ControlCharacter
    );
    assert_eq!(
        unsafe_reason(open(one_named(b"a\nb"))),
        NameProblem::ControlCharacter
    );
    assert_eq!(
        unsafe_reason(open(one_named(b"a\x7fb"))),
        NameProblem::ControlCharacter
    );
    assert_eq!(
        unsafe_reason(open(one_named("a\u{202e}b".as_bytes()))),
        NameProblem::ControlCharacter
    );
}

#[test]
fn a_name_that_is_not_utf8_is_refused() {
    assert_eq!(
        unsafe_reason(open(one_named(&[b'a', 0xFF, b'b']))),
        NameProblem::NotUtf8
    );
}

#[test]
fn dot_and_empty_components_are_refused_not_normalised() {
    assert_eq!(
        unsafe_reason(open(one_named(b"./a"))),
        NameProblem::CurrentComponent
    );
    assert_eq!(
        unsafe_reason(open(one_named(b"a/./b"))),
        NameProblem::CurrentComponent
    );
    assert_eq!(
        unsafe_reason(open(one_named(b"a//b"))),
        NameProblem::EmptyComponent
    );
}

#[test]
fn an_empty_name_is_refused() {
    assert!(open(one_named(b"")).is_err());
}

#[test]
fn a_name_of_512_bytes_is_accepted_and_513_is_refused() {
    let at = "a".repeat(512);
    assert!(open(one_named(at.as_bytes())).is_ok());
    let over = "a".repeat(513);
    assert_eq!(
        unsafe_reason(open(one_named(over.as_bytes()))),
        NameProblem::TooLong
    );
}

#[test]
fn a_path_of_32_components_is_accepted_and_33_is_refused() {
    let at = vec!["d"; 32].join("/");
    assert!(open(one_named(at.as_bytes())).is_ok());
    let over = vec!["d"; 33].join("/");
    assert_eq!(
        unsafe_reason(open(one_named(over.as_bytes()))),
        NameProblem::TooDeep
    );
}

#[test]
fn duplicate_names_are_refused() {
    let bytes = raw_zip(&[
        RawEntry::stored("manifest.json", b"{}"),
        RawEntry::stored("manifest.json", b"{\"evil\":1}"),
    ]);
    assert!(matches!(
        open(bytes).unwrap_err(),
        ArchiveError::DuplicateName { .. }
    ));
}

#[test]
fn names_that_differ_only_in_case_are_refused() {
    let bytes = raw_zip(&[
        RawEntry::stored("README", b"a"),
        RawEntry::stored("readme", b"b"),
    ]);
    assert!(matches!(
        open(bytes).unwrap_err(),
        ArchiveError::CaseCollision { .. }
    ));
    let nested = raw_zip(&[
        RawEntry::stored("A/b.js", b"a"),
        RawEntry::stored("a/B.js", b"b"),
    ]);
    assert!(matches!(
        open(nested).unwrap_err(),
        ArchiveError::CaseCollision { .. }
    ));
    let distinct = raw_zip(&[
        RawEntry::stored("a/b.js", b"a"),
        RawEntry::stored("a/c.js", b"b"),
    ]);
    assert!(open(distinct).is_ok());
}

#[test]
fn a_file_and_a_directory_with_one_path_are_refused() {
    let file_then_child = raw_zip(&[RawEntry::stored("a", b"x"), RawEntry::stored("a/b", b"y")]);
    assert!(matches!(
        open(file_then_child).unwrap_err(),
        ArchiveError::PathConflict { .. }
    ));
    let child_then_file = raw_zip(&[RawEntry::stored("a/b", b"y"), RawEntry::stored("a", b"x")]);
    assert!(matches!(
        open(child_then_file).unwrap_err(),
        ArchiveError::PathConflict { .. }
    ));
    let dir_then_file = raw_zip(&[RawEntry::directory("a/"), RawEntry::stored("a", b"x")]);
    assert!(open(dir_then_file).is_err());
}

// ---- entry kinds -------------------------------------------------------------------------------

#[test]
fn symbolic_link_entry_is_refused() {
    let bytes = raw_zip(&[RawEntry::symlink("link", "/etc/passwd")]);
    assert!(matches!(
        open(bytes).unwrap_err(),
        ArchiveError::SymlinkEntry { .. }
    ));
}

#[test]
fn device_and_fifo_entries_are_refused() {
    for mode in [0o020_644, 0o060_644, 0o010_644, 0o140_644] {
        let bytes = raw_zip(&[RawEntry::stored("node", b"").mode(mode)]);
        assert!(
            matches!(open(bytes).unwrap_err(), ArchiveError::SpecialEntry { .. }),
            "mode {mode:o}"
        );
    }
}

#[test]
fn a_directory_mode_on_a_file_name_is_refused() {
    let bytes = raw_zip(&[RawEntry::stored("looks-like-a-file", b"").mode(0o040_755)]);
    assert!(matches!(
        open(bytes).unwrap_err(),
        ArchiveError::SpecialEntry { .. }
    ));
}

#[test]
fn encrypted_entry_is_refused() {
    let mut entry = RawEntry::stored("secret.txt", b"cipher");
    entry.flags = 1;
    assert!(matches!(
        open(raw_zip(&[entry])).unwrap_err(),
        ArchiveError::Encrypted { .. }
    ));
}

#[test]
fn compression_methods_other_than_stored_and_deflate_are_refused() {
    for method in [1u16, 6, 9, 12, 14, 93, 95, 98, 99] {
        let mut entry = RawEntry::stored("x", b"data");
        entry.method = method;
        let err = open(raw_zip(&[entry])).unwrap_err();
        assert!(
            matches!(err, ArchiveError::UnsupportedCompression { method: m, .. } if m == method),
            "method {method}: {err:?}"
        );
    }
}

// ---- counts and sizes (the D12 limits, at the edge and one over) ---------------------------------

fn empty_entries(count: usize) -> Vec<u8> {
    let entries: Vec<RawEntry> = (0..count)
        .map(|i| RawEntry::stored(&format!("f{i}"), b""))
        .collect();
    raw_zip(&entries)
}

#[test]
fn twenty_thousand_entries_are_accepted_and_twenty_thousand_and_one_are_refused() {
    assert_eq!(open(empty_entries(20_000)).unwrap().entries().len(), 20_000);
    assert_eq!(
        open(empty_entries(20_001)).unwrap_err(),
        ArchiveError::TooManyEntries {
            limit: 20_000,
            found: 20_001
        }
    );
}

#[test]
fn package_size_edge() {
    let bytes = sample();
    let exact = Limits {
        max_package_bytes: bytes.len() as u64,
        ..Limits::default()
    };
    assert!(open_with(bytes.clone(), &exact).is_ok());
    let under = Limits {
        max_package_bytes: bytes.len() as u64 - 1,
        ..Limits::default()
    };
    assert_eq!(
        open_with(bytes.clone(), &under).unwrap_err(),
        ArchiveError::PackageTooLarge {
            limit: bytes.len() as u64 - 1,
            found: bytes.len() as u64
        }
    );
}

/// A deflated entry that declares `size` bytes while its data is tiny: nothing is inflated at open.
fn declaring(name: &str, size: u32) -> RawEntry {
    let mut entry = RawEntry::deflated(name, b"x");
    entry.declared_size = size;
    entry
}

fn small_limits() -> Limits {
    Limits {
        max_entry_bytes: 1000,
        max_total_bytes: 3000,
        ratio_min_entry_bytes: u64::MAX,
        ..Limits::default()
    }
}

#[test]
fn entry_size_edge() {
    let limits = small_limits();
    assert!(open_with(raw_zip(&[declaring("a", 1000)]), &limits).is_ok());
    assert!(matches!(
        open_with(raw_zip(&[declaring("a", 1001)]), &limits).unwrap_err(),
        ArchiveError::EntryTooLarge { limit: 1000, .. }
    ));
}

#[test]
fn total_size_edge() {
    let limits = small_limits();
    let three = raw_zip(&[
        declaring("a", 1000),
        declaring("b", 1000),
        declaring("c", 1000),
    ]);
    assert!(open_with(three, &limits).is_ok());
    let over = raw_zip(&[
        declaring("a", 1000),
        declaring("b", 1000),
        declaring("c", 1000),
        declaring("d", 1),
    ]);
    assert_eq!(
        open_with(over, &limits).unwrap_err(),
        ArchiveError::TotalTooLarge { limit: 3000 }
    );
}

#[test]
fn default_entry_cap_is_one_hundred_mebibytes() {
    let at = u32::try_from(100 * 1024 * 1024).unwrap();
    let limits = Limits {
        ratio_min_entry_bytes: u64::MAX,
        ..Limits::default()
    };
    assert!(open_with(raw_zip(&[declaring("a", at)]), &limits).is_ok());
    assert!(matches!(
        open_with(raw_zip(&[declaring("a", at + 1)]), &limits).unwrap_err(),
        ArchiveError::EntryTooLarge { .. }
    ));
}

#[test]
fn default_total_cap_is_five_hundred_mebibytes() {
    let limits = Limits {
        ratio_min_entry_bytes: u64::MAX,
        ..Limits::default()
    };
    let hundred = u32::try_from(100 * 1024 * 1024).unwrap();
    let five: Vec<RawEntry> = (0..5)
        .map(|i| declaring(&format!("f{i}"), hundred))
        .collect();
    assert!(open_with(raw_zip(&five), &limits).is_ok());
    let mut six = five.clone();
    six.push(declaring("f5", 1));
    assert!(matches!(
        open_with(raw_zip(&six), &limits).unwrap_err(),
        ArchiveError::TotalTooLarge { .. }
    ));
}

#[test]
fn compression_ratio_edge_is_two_hundred_to_one() {
    let limits = Limits {
        ratio_min_entry_bytes: 100,
        ..Limits::default()
    };
    let compressed = RawEntry::deflated("a", b"x").declared_compressed;
    let at = declaring("a", compressed * 200);
    assert!(open_with(raw_zip(&[at]), &limits).is_ok());
    let over = declaring("a", compressed * 200 + 1);
    assert!(matches!(
        open_with(raw_zip(&[over]), &limits).unwrap_err(),
        ArchiveError::CompressionRatio { .. }
    ));
}

#[test]
fn the_ratio_rule_starts_above_one_mebibyte() {
    let mib = 1024 * 1024;
    let limits = Limits {
        max_entry_bytes: 1 << 30,
        max_total_bytes: 1 << 30,
        ..Limits::default()
    };
    assert!(open_with(raw_zip(&[declaring("a", mib)]), &limits).is_ok());
    assert!(matches!(
        open_with(raw_zip(&[declaring("a", mib + 1)]), &limits).unwrap_err(),
        ArchiveError::CompressionRatio { .. }
    ));
}

#[test]
fn a_gibibyte_of_zeros_is_refused_by_ratio_and_by_cap_without_inflating() {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(1))
        .large_file(true);
    writer.start_file("zeros.bin", options).unwrap();
    let chunk = vec![0u8; 1024 * 1024];
    for _ in 0..1024 {
        writer.write_all(&chunk).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    assert!(
        bytes.len() < 5 * 1024 * 1024,
        "the bomb is small: {} bytes",
        bytes.len()
    );
    assert!(matches!(
        open(bytes.clone()).unwrap_err(),
        ArchiveError::EntryTooLarge { .. } | ArchiveError::CompressionRatio { .. }
    ));
    let roomy = Limits {
        max_entry_bytes: 2 << 30,
        max_total_bytes: 2 << 30,
        ..Limits::default()
    };
    assert!(matches!(
        open_with(bytes, &roomy).unwrap_err(),
        ArchiveError::CompressionRatio { .. }
    ));
}

// ---- lying directories -----------------------------------------------------------------------

#[test]
fn a_directory_that_declares_fewer_bytes_than_the_data_inflates_to_is_not_trusted() {
    let data = vec![b'A'; 100];
    for crc in [crc32(&data), crc32(&data[..10])] {
        let mut entry = RawEntry::deflated("lie.txt", &data);
        entry.declared_size = 10;
        entry.crc = crc;
        let pkg = match open(raw_zip(&[entry])) {
            Ok(pkg) => pkg,
            Err(_) => continue,
        };
        if let Ok(bytes) = pkg.read_file("lie.txt") {
            assert!(bytes.len() <= 10, "read {} bytes", bytes.len())
        }
    }
}

#[test]
fn a_stored_entry_whose_data_is_longer_than_declared_fails_to_read() {
    let data = vec![b'B'; 100];
    let mut entry = RawEntry::stored("lie.txt", &data);
    entry.declared_size = 10;
    entry.declared_compressed = 10;
    if let Ok(pkg) = open(raw_zip(&[entry])) {
        assert!(pkg.read_file("lie.txt").is_err());
    }
}

#[test]
fn a_directory_that_declares_more_bytes_than_there_are_fails_to_read() {
    let mut entry = RawEntry::deflated("lie.txt", b"short");
    entry.declared_size = 100;
    let pkg = open_with(
        raw_zip(&[entry]),
        &Limits {
            ratio_min_entry_bytes: u64::MAX,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        pkg.read_file("lie.txt").unwrap_err(),
        ArchiveError::SizeMismatch { .. } | ArchiveError::Io(_)
    ));
}

#[test]
fn a_corrupt_crc_fails_to_read() {
    let mut entry = RawEntry::stored("a.txt", b"hello");
    entry.crc ^= 1;
    let pkg = open(raw_zip(&[entry])).unwrap();
    assert!(pkg.read_file("a.txt").is_err());
}

// ---- reading -----------------------------------------------------------------------------------

#[test]
fn reading_a_missing_file_or_a_directory_is_an_error() {
    let bytes = raw_zip(&[RawEntry::directory("d/"), RawEntry::stored("d/f", b"x")]);
    let pkg = open(bytes).unwrap();
    assert_eq!(pkg.read_file("nope").unwrap_err(), ArchiveError::NotFound);
    assert_eq!(pkg.read_file("d/").unwrap_err(), ArchiveError::IsDirectory);
    assert_eq!(pkg.read_file("d/f").unwrap(), b"x");
}

#[test]
fn read_file_capped_refuses_before_inflating() {
    let pkg = open(sample()).unwrap();
    assert!(matches!(
        pkg.read_file_capped("css/style.css", 4999).unwrap_err(),
        ArchiveError::EntryTooLarge { limit: 4999, .. }
    ));
    assert_eq!(
        pkg.read_file_capped("css/style.css", 5000).unwrap().len(),
        5000
    );
}

#[test]
fn visit_file_streams_the_whole_file_in_chunks() {
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let pkg = open(raw_zip(&[RawEntry::deflated("big.bin", &data)])).unwrap();
    let mut seen = Vec::new();
    let mut chunks = 0;
    let total = pkg
        .visit_file("big.bin", u64::MAX, &mut |chunk| {
            chunks += 1;
            seen.extend_from_slice(chunk);
        })
        .unwrap();
    assert_eq!(total, data.len() as u64);
    assert_eq!(seen, data);
    assert!(chunks > 1);
}

// ---- extraction -------------------------------------------------------------------------------

fn tree() -> Vec<u8> {
    raw_zip(&[
        RawEntry::directory("empty/"),
        RawEntry::stored("manifest.json", b"{}"),
        RawEntry::deflated("js/app.js", b"app();\n"),
        RawEntry::stored("js/lib/util.js", b"util();\n"),
        RawEntry::stored("bin/tool", b"#!/bin/sh\n").mode(0o100_755),
    ])
}

#[test]
fn extraction_writes_exactly_the_entries() {
    let pkg = open(tree()).unwrap();
    let parent = temp_dir();
    let dest = parent.path().join("out");
    let stats = pkg.extract_to(&dest).unwrap();
    assert_eq!(stats.files, 4);
    assert_eq!(fs::read(dest.join("manifest.json")).unwrap(), b"{}");
    assert_eq!(fs::read(dest.join("js/app.js")).unwrap(), b"app();\n");
    assert_eq!(fs::read(dest.join("js/lib/util.js")).unwrap(), b"util();\n");
    assert!(dest.join("empty").is_dir());
    let mut found = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            out.push(rel);
            if path.is_dir() {
                walk(root, &path, out);
            }
        }
    }
    walk(&dest, &dest, &mut found);
    found.sort();
    assert_eq!(
        found,
        vec![
            "bin",
            "bin/tool",
            "empty",
            "js",
            "js/app.js",
            "js/lib",
            "js/lib/util.js",
            "manifest.json"
        ]
    );
}

#[cfg(unix)]
#[test]
fn extraction_sets_mode_0644_or_0755_and_never_a_set_id_bit() {
    use std::os::unix::fs::PermissionsExt;
    let bytes = raw_zip(&[
        RawEntry::stored("plain", b"p").mode(0o100_600),
        RawEntry::stored("setuid", b"s").mode(0o104_755),
        RawEntry::stored("setgid", b"g").mode(0o102_644),
        RawEntry::stored("exec", b"x").mode(0o100_700),
    ]);
    let pkg = open(bytes).unwrap();
    let parent = temp_dir();
    let dest = parent.path().join("out");
    pkg.extract_to(&dest).unwrap();
    let mode = |n: &str| fs::metadata(dest.join(n)).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode("plain"), 0o644);
    assert_eq!(mode("setuid"), 0o755);
    assert_eq!(mode("setgid"), 0o644);
    assert_eq!(mode("exec"), 0o755);
}

#[test]
fn extraction_into_an_existing_empty_directory_works() {
    let pkg = open(tree()).unwrap();
    let parent = temp_dir();
    let dest = parent.path().join("existing");
    fs::create_dir(&dest).unwrap();
    pkg.extract_to(&dest).unwrap();
    assert!(dest.join("manifest.json").is_file());
}

#[test]
fn extraction_refuses_a_destination_that_is_not_empty() {
    let pkg = open(tree()).unwrap();
    let parent = temp_dir();
    let dest = parent.path().join("busy");
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("precious"), b"keep").unwrap();
    assert_eq!(
        pkg.extract_to(&dest).unwrap_err(),
        ArchiveError::DestinationNotEmpty
    );
    assert_eq!(fs::read(dest.join("precious")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
}

#[test]
fn extraction_refuses_a_destination_that_is_a_file() {
    let pkg = open(tree()).unwrap();
    let parent = temp_dir();
    let dest = parent.path().join("file");
    fs::write(&dest, b"x").unwrap();
    assert_eq!(
        pkg.extract_to(&dest).unwrap_err(),
        ArchiveError::DestinationNotEmpty
    );
    assert_eq!(fs::read(&dest).unwrap(), b"x");
}

#[cfg(unix)]
#[test]
fn extraction_refuses_a_destination_that_is_a_symbolic_link() {
    let pkg = open(tree()).unwrap();
    let parent = temp_dir();
    let target = parent.path().join("target");
    fs::create_dir(&target).unwrap();
    let link = parent.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(pkg.extract_to(&link).is_err());
    assert!(dir_is_empty(&target));
}

fn failing_third_entry() -> Vec<u8> {
    let mut bad = RawEntry::stored("c/bad.txt", b"corrupt");
    bad.crc ^= 0xFFFF;
    raw_zip(&[
        RawEntry::stored("a.txt", b"a"),
        RawEntry::stored("b/b.txt", b"b"),
        bad,
        RawEntry::stored("d.txt", b"d"),
    ])
}

#[test]
fn a_failed_extraction_removes_everything_it_created() {
    let pkg = open(failing_third_entry()).unwrap();
    let parent = temp_dir();
    let dest = parent.path().join("out");
    assert!(pkg.extract_to(&dest).is_err());
    assert!(
        !dest.exists(),
        "the destination this call created is removed"
    );
    assert!(dir_is_empty(parent.path()));
}

#[test]
fn a_failed_extraction_leaves_an_existing_empty_destination_empty() {
    let pkg = open(failing_third_entry()).unwrap();
    let parent = temp_dir();
    let dest = parent.path().join("out");
    fs::create_dir(&dest).unwrap();
    assert!(pkg.extract_to(&dest).is_err());
    assert!(dest.is_dir());
    assert!(dir_is_empty(&dest));
}

#[test]
fn a_failed_extraction_creates_missing_parents_only_to_remove_them() {
    let pkg = open(failing_third_entry()).unwrap();
    let parent = temp_dir();
    let dest = parent.path().join("x").join("y").join("out");
    assert!(pkg.extract_to(&dest).is_err());
    assert!(dir_is_empty(parent.path()));
}

#[test]
fn extraction_counts_the_bytes_it_writes_not_the_declared_sizes() {
    let mut entry = RawEntry::deflated("lie.txt", &vec![b'z'; 500]);
    entry.declared_size = 10;
    entry.crc = crc32(&[b'z'; 10]);
    let pkg = match open(raw_zip(&[entry])) {
        Ok(pkg) => pkg,
        Err(_) => return,
    };
    let parent = temp_dir();
    let dest = parent.path().join("out");
    match pkg.extract_to(&dest) {
        Ok(_) => assert!(fs::read(dest.join("lie.txt")).unwrap().len() <= 10),
        Err(_) => assert!(dir_is_empty(parent.path())),
    }
}

#[test]
fn names_that_a_volume_may_fold_together_never_overwrite_each_other() {
    // "é" as one code point and as "e" + combining accent: different bytes, one name on a volume that
    // ignores normalisation. Either both files exist, or the second create_new fails and everything is
    // rolled back.
    let bytes = raw_zip(&[
        RawEntry::stored("caf\u{e9}.txt", b"nfc"),
        RawEntry::stored("cafe\u{301}.txt", b"nfd"),
    ]);
    let pkg = open(bytes).unwrap();
    let parent = temp_dir();
    let dest = parent.path().join("out");
    match pkg.extract_to(&dest) {
        Ok(stats) => assert_eq!(stats.files, 2),
        Err(_) => assert!(dir_is_empty(parent.path())),
    }
}

// ---- mutation loop -----------------------------------------------------------------------------

#[test]
fn a_deterministic_mutation_loop_never_panics() {
    let base = sample();
    let mut rng = XorShift(0x2545_F491_4F6C_DD1D);
    let parent = temp_dir();
    for round in 0..2000 {
        let mut bytes = base.clone();
        match rng.below(4) {
            0 => {
                for _ in 0..=rng.below(3) {
                    let i = rng.below(bytes.len());
                    bytes[i] ^= 1 << rng.below(8);
                }
            }
            1 => bytes.truncate(rng.below(bytes.len())),
            2 => {
                let i = rng.below(bytes.len());
                bytes.insert(i, rng.next() as u8);
            }
            _ => {
                let i = rng.below(bytes.len());
                let j = rng.below(bytes.len());
                bytes.swap(i, j);
            }
        }
        if let Ok(pkg) = open(bytes) {
            let names: Vec<String> = pkg.file_names().map(str::to_string).collect();
            for name in names {
                let _ = pkg.read_file(&name);
            }
            if round % 50 == 0 {
                let dest = parent.path().join(format!("out{round}"));
                let _ = pkg.extract_to(&dest);
            }
        }
    }
}
