// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `site-images.json`: the store of the sites whose images are off. The table of entries is
//! repeated in `crates/talaria-core/tests/site_images.rs`: the two crates normalise a host by the
//! same rule without sharing code.

use std::fs;
use std::path::{Path, PathBuf};

use mercury_settings::{
    Loaded, MAX_SITE_IMAGE_HOSTS, MAX_SITE_IMAGES_FILE_BYTES, SITE_IMAGES_FORMAT_VERSION,
    SettingsError, SiteImagesFile, SiteImagesStore,
};
use serde_json::{Value, json};

fn profile() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn file_path(dir: &Path) -> PathBuf {
    dir.join("site-images.json")
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("the profile should be readable")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn file_of(items: &[&str]) -> SiteImagesFile {
    SiteImagesFile {
        blocked_hosts: strings(items),
    }
}

/// Seeds the file with `bytes`, loads it, and checks it was treated as damaged: none blocked, the
/// original moved to the backup and readable there, nothing at the original path.
fn assert_damaged(bytes: &[u8]) -> PathBuf {
    let dir = profile();
    let path = file_path(dir.path());
    fs::write(&path, bytes).expect("seed the file");

    let loaded = SiteImagesStore::new(dir.path()).load().expect("load");

    let Loaded::Damaged { value, backup } = loaded else {
        panic!("expected Damaged, got {loaded:?}");
    };
    assert_eq!(value, SiteImagesFile::default());
    assert_eq!(
        backup.parent(),
        Some(dir.path()),
        "the backup sits beside it"
    );
    assert_eq!(fs::read(&backup).expect("the backup is readable"), bytes);
    assert!(!path.exists(), "the damaged file was moved, not copied");
    backup
}

/// Seeds the file with `bytes`, loads it and returns the value and repairs of an `Ok` load.
fn load_ok(bytes: &[u8]) -> (SiteImagesFile, usize) {
    let dir = profile();
    fs::write(file_path(dir.path()), bytes).expect("seed the file");
    match SiteImagesStore::new(dir.path()).load().expect("load") {
        Loaded::Ok { value, repairs } => {
            assert!(repairs.iter().all(|repair| repair.field == "blocked_hosts"));
            (value, repairs.len())
        }
        other => panic!("expected Ok, got {other:?}"),
    }
}

/// (stored entry, key it becomes), the same table as in `talaria-core`'s tests.
const ENTRIES: &[(&str, Option<&str>)] = &[
    ("example.com", Some("example.com")),
    ("Example.COM", Some("example.com")),
    ("example.com.", Some("example.com")),
    ("www.example.com", Some("www.example.com")),
    ("xn--bcher-kva.example", Some("xn--bcher-kva.example")),
    ("b\u{fc}cher.example", Some("xn--bcher-kva.example")),
    ("[::1]", Some("[::1]")),
    ("[0:0:0:0:0:0:0:1]", Some("[::1]")),
    ("[2001:DB8::1]", Some("[2001:db8::1]")),
    ("127.0.0.1", Some("127.0.0.1")),
    ("127.1", Some("127.0.0.1")),
    ("localhost", Some("localhost")),
    ("", None),
    (" ", None),
    (" example.com", None),
    ("example.com ", None),
    ("exa mple.com", None),
    ("exa\tmple.com", None),
    ("exa\nmple.com", None),
    ("exa\u{0}mple.com", None),
    ("https://example.com", None),
    ("https://example.com/", None),
    ("//example.com", None),
    ("example.com/", None),
    ("example.com/path", None),
    ("example.com\\path", None),
    ("example.com?x=1", None),
    ("example.com#frag", None),
    ("user@example.com", None),
    ("user:pw@example.com", None),
    ("example.com:80", None),
    ("example.com:8080", None),
    ("[::1]:8080", None),
    ("%65xample.com", None),
    ("example%2Ecom", None),
    ("[fe80::1%25eth0]", None),
    ("[", None),
    ("]", None),
    ("[]", None),
    ("[zzz]", None),
    ("[::1", None),
    ("::1", None),
    (".", None),
    ("..", None),
];

#[test]
fn an_entry_is_normalised_like_a_page_host_or_dropped() {
    for (entry, expected) in ENTRIES {
        let file = SiteImagesFile::new([(*entry).to_owned()]);
        assert_eq!(
            file.blocked_hosts,
            expected
                .iter()
                .map(|key| (*key).to_owned())
                .collect::<Vec<_>>(),
            "the entry {entry:?}"
        );
    }
}

#[test]
fn no_file_is_a_first_run() {
    let dir = profile();

    let loaded = SiteImagesStore::new(dir.path()).load().expect("load");

    assert_eq!(loaded, Loaded::Missing(SiteImagesFile::default()));
    assert!(names_in(dir.path()).is_empty(), "loading creates nothing");
}

#[test]
fn a_saved_list_loads_back() {
    let dir = profile();
    let store = SiteImagesStore::new(dir.path());
    let file = file_of(&["example.com", "[::1]", "www.example.com"]);
    store.save(&file).expect("save");

    let loaded = store.load().expect("load");

    // Sorted: "[" sorts before letters.
    assert_eq!(
        loaded,
        Loaded::Ok {
            value: file_of(&["[::1]", "example.com", "www.example.com"]),
            repairs: Vec::new()
        }
    );
    assert_eq!(
        names_in(dir.path()),
        ["site-images.json"],
        "no temporary left"
    );
}

#[test]
fn the_file_has_the_frozen_shape() {
    let dir = profile();
    let store = SiteImagesStore::new(dir.path());
    store.save(&file_of(&["b.test", "a.test"])).expect("save");

    let text = fs::read_to_string(file_path(dir.path())).expect("the file");
    let json: Value = serde_json::from_str(&text).expect("JSON");

    assert_eq!(
        json,
        json!({ "version": 1, "blocked_hosts": ["a.test", "b.test"] })
    );
    assert_eq!(SITE_IMAGES_FORMAT_VERSION, 1);
    assert!(
        text.trim_start().starts_with("{\n  \"version\": 1,"),
        "the version comes first: {text}"
    );
    assert!(text.ends_with("}\n"));
}

#[test]
fn an_empty_list_is_saved_and_loaded() {
    let dir = profile();
    let store = SiteImagesStore::new(dir.path());
    store.save(&SiteImagesFile::default()).expect("save");

    let json: Value =
        serde_json::from_slice(&fs::read(file_path(dir.path())).expect("the file")).expect("JSON");
    assert_eq!(json, json!({ "version": 1, "blocked_hosts": [] }));
    assert_eq!(
        store.load().expect("load"),
        Loaded::Ok {
            value: SiteImagesFile::default(),
            repairs: Vec::new()
        }
    );
}

#[cfg(unix)]
#[test]
fn the_file_is_private() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = profile();
    let store = SiteImagesStore::new(dir.path());
    store.save(&file_of(&["example.com"])).expect("save");
    let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode(&file_path(dir.path())), 0o600);

    // Saving again over a wider file narrows it, because the file is replaced.
    fs::set_permissions(file_path(dir.path()), fs::Permissions::from_mode(0o644)).expect("chmod");
    store.save(&file_of(&["example.org"])).expect("save again");
    assert_eq!(mode(&file_path(dir.path())), 0o600);
}

#[test]
fn a_hand_edited_list_is_repaired_on_load() {
    let (value, repairs) = load_ok(
        br#"{"version": 1, "blocked_hosts": ["B.test", "a.test", "A.TEST.", "b.test", "no good", "https://c.test/"]}"#,
    );
    assert_eq!(value, file_of(&["a.test", "b.test"]));
    assert_eq!(repairs, 1, "one repair for the field, however many entries");

    // Only unsorted: still reported, so the caller saves it in order once.
    let (value, repairs) = load_ok(br#"{"version": 1, "blocked_hosts": ["b.test", "a.test"]}"#);
    assert_eq!(value, file_of(&["a.test", "b.test"]));
    assert_eq!(repairs, 1);

    // Entries that are not text are skipped.
    let (value, repairs) =
        load_ok(br#"{"version": 1, "blocked_hosts": ["a.test", 7, null, {"x": 1}, ["b.test"]]}"#);
    assert_eq!(value, file_of(&["a.test"]));
    assert_eq!(repairs, 1);

    // A clean file has none.
    let (value, repairs) = load_ok(br#"{"version": 1, "blocked_hosts": ["a.test", "b.test"]}"#);
    assert_eq!(value, file_of(&["a.test", "b.test"]));
    assert_eq!(repairs, 0);
}

#[test]
fn a_file_without_a_version_or_a_list_is_read_as_version_one() {
    assert_eq!(
        load_ok(br#"{"blocked_hosts": ["a.test"]}"#),
        (file_of(&["a.test"]), 0)
    );
    assert_eq!(
        load_ok(br#"{"version": 1}"#),
        (SiteImagesFile::default(), 0)
    );
    assert_eq!(load_ok(b"{}"), (SiteImagesFile::default(), 0));
    assert_eq!(
        load_ok(br#"{"version": 1, "blocked_hosts": null}"#),
        (SiteImagesFile::default(), 0)
    );
    assert_eq!(
        load_ok(br#"{"version": 0, "blocked_hosts": ["a.test"], "later": {"x": [1, 2]}}"#),
        (file_of(&["a.test"]), 0),
        "version 0 and members it does not know are read"
    );
}

#[test]
fn a_damaged_file_is_moved_aside() {
    assert_damaged(b"");
    assert_damaged(b"not json");
    assert_damaged(br#"{"version": 1, "blocked_hosts": [""#);
    assert_damaged(br#"["a.test"]"#);
    assert_damaged(b"null");
    assert_damaged(br#"{"version": 1, "blocked_hosts": "a.test"}"#);
    assert_damaged(br#"{"version": 1, "blocked_hosts": {"a.test": true}}"#);
    assert_damaged(br#"{"version": "1", "blocked_hosts": []}"#);
    assert_damaged(br#"{"version": -1, "blocked_hosts": []}"#);
    assert_damaged(br#"{"version": 1.5, "blocked_hosts": []}"#);
    assert_damaged(br#"{"version": null, "blocked_hosts": []}"#);
    assert_damaged(b"\xEF\xBB\xBF{\"version\": 1, \"blocked_hosts\": []}");
    assert_damaged(&[0xFF, 0xFE, 0x00, 0x01]);
}

#[test]
fn a_deeply_nested_file_is_damaged_not_a_crash() {
    let mut bytes = br#"{"version": 1, "blocked_hosts": "#.to_vec();
    bytes.extend(std::iter::repeat_n(b'[', 100_000));
    assert_damaged(&bytes);
}

#[test]
fn a_second_damaged_file_gets_its_own_backup() {
    let dir = profile();
    let store = SiteImagesStore::new(dir.path());
    fs::write(file_path(dir.path()), b"first").expect("seed");
    let Loaded::Damaged { backup: first, .. } = store.load().expect("load") else {
        panic!("expected Damaged");
    };
    fs::write(file_path(dir.path()), b"second").expect("seed again");
    let Loaded::Damaged { backup: second, .. } = store.load().expect("load") else {
        panic!("expected Damaged");
    };

    assert_ne!(first, second);
    assert_eq!(fs::read(first).expect("first"), b"first");
    assert_eq!(fs::read(second).expect("second"), b"second");
}

#[test]
fn a_file_of_300_kib_is_damaged() {
    let mut bytes = br#"{"version": 1, "blocked_hosts": ["#.to_vec();
    while bytes.len() < 300 * 1024 {
        bytes.extend_from_slice(b"\"a.test\", ");
    }
    bytes.extend_from_slice(b"\"z.test\"]}");
    assert!(u64::try_from(bytes.len()).expect("a length") > MAX_SITE_IMAGES_FILE_BYTES);

    assert_damaged(&bytes);
}

#[test]
fn a_file_just_under_the_limit_is_read() {
    let mut bytes = br#"{"version": 1, "blocked_hosts": ["a.test"]"#.to_vec();
    // Padding in a member the reader ignores, up to the limit exactly.
    bytes.extend_from_slice(br#", "pad": ""#);
    let room = usize::try_from(MAX_SITE_IMAGES_FILE_BYTES).expect("a size") - bytes.len() - 2;
    bytes.extend(std::iter::repeat_n(b'x', room));
    bytes.extend_from_slice(b"\"}");
    assert_eq!(
        u64::try_from(bytes.len()).expect("a length"),
        MAX_SITE_IMAGES_FILE_BYTES
    );
    assert_eq!(load_ok(&bytes), (file_of(&["a.test"]), 0));

    bytes.push(b' ');
    assert_damaged(&bytes);
}

#[test]
fn a_newer_version_is_left_alone_and_never_saved_over() {
    let dir = profile();
    let path = file_path(dir.path());
    let original = br#"{"version": 2, "blocked_hosts": [{"host": "a.test", "since": 5}], "x": 1}"#;
    fs::write(&path, original).expect("seed");
    let store = SiteImagesStore::new(dir.path());

    let loaded = store.load().expect("load");
    assert_eq!(
        loaded,
        Loaded::Newer {
            value: SiteImagesFile::default(),
            found_version: 2
        }
    );
    assert_eq!(fs::read(&path).expect("untouched"), original);

    let error = store
        .save(&file_of(&["b.test"]))
        .expect_err("a save over a newer file");
    assert!(
        matches!(
            error,
            SettingsError::NewerVersion {
                found: 2,
                supported: 1,
                ..
            }
        ),
        "{error:?}"
    );
    assert_eq!(fs::read(&path).expect("still untouched"), original);
    assert_eq!(names_in(dir.path()), ["site-images.json"]);
}

#[test]
fn a_huge_version_number_is_newer() {
    for text in [
        r#"{"version": 99999999999, "blocked_hosts": []}"#,
        r#"{"version": 99999999999999999999999999, "blocked_hosts": []}"#,
    ] {
        let dir = profile();
        fs::write(file_path(dir.path()), text).expect("seed");
        let loaded = SiteImagesStore::new(dir.path()).load().expect("load");
        assert!(
            matches!(
                loaded,
                Loaded::Newer {
                    found_version: u32::MAX,
                    ..
                }
            ),
            "{text}: {loaded:?}"
        );
    }
}

#[test]
fn a_newer_file_over_the_size_limit_is_still_newer() {
    let dir = profile();
    let path = file_path(dir.path());
    let mut bytes = br#"{"version": 2, "blocked_hosts": ["#.to_vec();
    bytes.extend(std::iter::repeat_n(b'x', 400 * 1024));
    fs::write(&path, &bytes).expect("seed");

    let loaded = SiteImagesStore::new(dir.path()).load().expect("load");

    assert!(
        matches!(
            loaded,
            Loaded::Newer {
                found_version: 2,
                ..
            }
        ),
        "{loaded:?}"
    );
    assert_eq!(fs::read(&path).expect("untouched"), bytes);
}

#[test]
fn an_oversize_file_whose_version_is_not_first_is_damaged() {
    // Too large, and the version is not the first member: damaged, whatever it says.
    let mut bytes = br#"{"blocked_hosts": [], "version": 2, "pad": ""#.to_vec();
    bytes.extend(std::iter::repeat_n(b'x', 400 * 1024));
    bytes.extend_from_slice(b"\"}");
    assert_damaged(&bytes);
}

#[test]
fn saving_over_a_damaged_file_keeps_it_aside_first() {
    let dir = profile();
    let path = file_path(dir.path());
    fs::write(&path, b"{ not json").expect("seed");
    let store = SiteImagesStore::new(dir.path());

    store.save(&file_of(&["a.test"])).expect("save");

    assert_eq!(
        store.load().expect("load"),
        Loaded::Ok {
            value: file_of(&["a.test"]),
            repairs: Vec::new()
        }
    );
    let names = names_in(dir.path());
    assert_eq!(names, ["site-images.json", "site-images.json.damaged-1"]);
    assert_eq!(
        fs::read(dir.path().join("site-images.json.damaged-1")).expect("backup"),
        b"{ not json"
    );
}

#[test]
fn a_save_normalises_sorts_and_deduplicates() {
    let dir = profile();
    let store = SiteImagesStore::new(dir.path());

    store
        .save(&file_of(&[
            "B.test",
            "a.test",
            "A.test.",
            "bad host",
            "https://c.test/",
            "b.test",
        ]))
        .expect("save");

    let json: Value =
        serde_json::from_slice(&fs::read(file_path(dir.path())).expect("the file")).expect("JSON");
    assert_eq!(json["blocked_hosts"], json!(["a.test", "b.test"]));
}

#[test]
fn an_entry_of_10000_bytes_is_dropped() {
    let giant = "a".repeat(10_000);
    let bytes = serde_json::to_vec(&json!({
        "version": 1,
        "blocked_hosts": [giant, "kept.test"]
    }))
    .expect("JSON");

    assert_eq!(load_ok(&bytes), (file_of(&["kept.test"]), 1));

    let dir = profile();
    let store = SiteImagesStore::new(dir.path());
    store
        .save(&SiteImagesFile {
            blocked_hosts: vec!["a".repeat(10_000), "kept.test".to_owned()],
        })
        .expect("save");
    let on_disk = fs::read(file_path(dir.path())).expect("the file");
    assert!(on_disk.len() < 200, "the giant entry was not written");
}

#[test]
fn five_thousand_entries_keep_the_first_thousand() {
    let entries: Vec<String> = (0..5000)
        .map(|number| format!("site{number}.test"))
        .collect();
    let bytes =
        serde_json::to_vec(&json!({ "version": 1, "blocked_hosts": entries })).expect("JSON");
    assert!(u64::try_from(bytes.len()).expect("a length") < MAX_SITE_IMAGES_FILE_BYTES);

    let (value, repairs) = load_ok(&bytes);

    assert_eq!(value.blocked_hosts.len(), MAX_SITE_IMAGE_HOSTS);
    assert_eq!(MAX_SITE_IMAGE_HOSTS, 1000);
    assert!(value.blocked_hosts.contains(&"site0.test".to_owned()));
    assert!(value.blocked_hosts.contains(&"site999.test".to_owned()));
    assert!(!value.blocked_hosts.contains(&"site1000.test".to_owned()));
    assert!(
        value.blocked_hosts.windows(2).all(|pair| pair[0] < pair[1]),
        "sorted"
    );
    assert_eq!(repairs, 1);

    // The same cap on the way out.
    let dir = profile();
    let store = SiteImagesStore::new(dir.path());
    store
        .save(&SiteImagesFile {
            blocked_hosts: entries,
        })
        .expect("save");
    let Loaded::Ok { value, repairs } = store.load().expect("load") else {
        panic!("expected Ok");
    };
    assert_eq!(value.blocked_hosts.len(), MAX_SITE_IMAGE_HOSTS);
    assert!(repairs.is_empty(), "what the store wrote needs no repair");
}

#[test]
fn a_thousand_longest_names_fit_in_the_file() {
    // 63 + 62 + 62 + 63 letters and three dots make 253 bytes,
    // the longest name DNS allows.
    let entries: Vec<String> = (0..MAX_SITE_IMAGE_HOSTS)
        .map(|number| {
            format!(
                "{number:0>63}.{}.{}.{}b",
                "b".repeat(62),
                "c".repeat(62),
                "d".repeat(62)
            )
        })
        .collect();
    assert!(entries.iter().all(|entry| entry.len() == 253));
    let dir = profile();
    let store = SiteImagesStore::new(dir.path());

    store
        .save(&SiteImagesFile {
            blocked_hosts: entries.clone(),
        })
        .expect("save");

    let size = fs::metadata(file_path(dir.path())).expect("metadata").len();
    assert!(size < MAX_SITE_IMAGES_FILE_BYTES, "{size} bytes");
    let Loaded::Ok { value, repairs } = store.load().expect("load") else {
        panic!("expected Ok");
    };
    assert_eq!(value.blocked_hosts, entries);
    assert!(repairs.is_empty());
}

#[cfg(unix)]
#[test]
fn a_symlink_at_the_path_is_replaced_not_written_through() {
    let dir = profile();
    let elsewhere = profile();
    let victim = elsewhere.path().join("victim.json");
    fs::write(
        &victim,
        br#"{"version": 1, "blocked_hosts": ["victim.test"]}"#,
    )
    .expect("victim");
    std::os::unix::fs::symlink(&victim, file_path(dir.path())).expect("symlink");
    let store = SiteImagesStore::new(dir.path());

    // The link is followed to read (as the settings file's is).
    assert_eq!(
        store.load().expect("load"),
        Loaded::Ok {
            value: file_of(&["victim.test"]),
            repairs: Vec::new()
        }
    );

    store.save(&file_of(&["mine.test"])).expect("save");

    let metadata = fs::symlink_metadata(file_path(dir.path())).expect("the path");
    assert!(
        metadata.file_type().is_file(),
        "the symlink was replaced by a file"
    );
    assert_eq!(
        fs::read(&victim).expect("victim"),
        br#"{"version": 1, "blocked_hosts": ["victim.test"]}"#,
        "nothing was written through the link"
    );
    assert_eq!(
        store.load().expect("load"),
        Loaded::Ok {
            value: file_of(&["mine.test"]),
            repairs: Vec::new()
        }
    );
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_is_a_first_run_and_is_replaced_by_a_save() {
    let dir = profile();
    std::os::unix::fs::symlink(dir.path().join("nowhere"), file_path(dir.path())).expect("symlink");
    let store = SiteImagesStore::new(dir.path());

    assert_eq!(
        store.load().expect("load"),
        Loaded::Missing(SiteImagesFile::default())
    );

    store.save(&file_of(&["a.test"])).expect("save");
    assert!(
        fs::symlink_metadata(file_path(dir.path()))
            .expect("the path")
            .file_type()
            .is_file()
    );
    assert!(!dir.path().join("nowhere").exists());
}

#[test]
fn a_directory_at_the_path_is_an_error_not_a_panic() {
    let dir = profile();
    fs::create_dir(file_path(dir.path())).expect("a directory");
    let store = SiteImagesStore::new(dir.path());

    assert!(matches!(store.load(), Err(SettingsError::Io { .. })));
    assert!(matches!(
        store.save(&file_of(&["a.test"])),
        Err(SettingsError::Io { .. })
    ));
    assert!(file_path(dir.path()).is_dir(), "left as it was");
}

#[test]
fn a_missing_profile_directory_is_an_error_on_save() {
    let dir = profile();
    let store = SiteImagesStore::new(&dir.path().join("not-there"));

    assert_eq!(
        store.load().expect("load"),
        Loaded::Missing(SiteImagesFile::default())
    );
    assert!(matches!(
        store.save(&file_of(&["a.test"])),
        Err(SettingsError::Io { .. })
    ));
}

#[test]
fn new_normalises_and_normalised_is_idempotent() {
    let file = SiteImagesFile::new(strings(&["B.test", "a.test", "a.test.", "bad host"]));
    assert_eq!(file, file_of(&["a.test", "b.test"]));
    assert_eq!(file.clone().normalised(), file);
    assert_eq!(
        SiteImagesFile {
            blocked_hosts: strings(&["Z.test", "q"])
        }
        .normalised(),
        file_of(&["q", "z.test"])
    );
}
