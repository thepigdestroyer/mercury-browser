// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The HSTS preload tool's manifest, dates, hashes, `check`, the refresh guards and `diff`
//! (`hsts::manifest` and `hsts::review`). Lists are built with `fstmap::build` from synthetic
//! hosts; the last test reads the file Servo 0.8.0 ships, the size the guards were chosen for.

use std::path::{Path, PathBuf};

use fst::MapBuilder;
use mercury_devtools::hsts::fstmap::{Preload, build};
use mercury_devtools::hsts::manifest::{Date, FORMAT, Manifest, Source, sha256_hex};
use mercury_devtools::hsts::review::{
    Diff, Limits, Verdict, check, diff, guard_flag_loss, guard_refresh, render_diff,
};
use mercury_devtools::hsts::{CANARIES, DEFAULT_MAX_SHRINK_PERCENT, Entry};
use serde_json::{Value, json};

fn entry(host: &str, include_subdomains: bool) -> Entry {
    Entry {
        host: host.to_string(),
        include_subdomains,
    }
}

/// The four hosts every refreshed list must contain.
fn canaries(include_subdomains: bool) -> Vec<Entry> {
    CANARIES
        .iter()
        .map(|host| entry(host, include_subdomains))
        .collect()
}

/// The data file for `entries`, and it opened.
fn file(entries: &[Entry]) -> (Vec<u8>, Preload) {
    let bytes = build(entries).expect("build the list");
    let preload = Preload::open(bytes.clone()).expect("open the list");
    (bytes, preload)
}

fn open(entries: &[Entry]) -> Preload {
    file(entries).1
}

/// A list of `total` entries (the four canaries, with the flag, then filler hosts) of which
/// exactly `included` include subdomains. `included` must be at least 4.
fn shaped(total: usize, included: usize) -> Vec<Entry> {
    let mut entries = canaries(true);
    for number in 0..total - CANARIES.len() {
        entries.push(entry(
            &format!("host-{number:05}.test"),
            CANARIES.len() + number < included,
        ));
    }
    entries
}

/// Eleven readable entries, ten of them with the flag (90.9 %), the canaries among them.
fn good_entries() -> Vec<Entry> {
    let mut entries = canaries(true);
    for (host, flag) in [
        ("example.org", true),
        ("example.net", true),
        ("mail.example.net", true),
        ("login.example.com", true),
        ("static.example.com", true),
        ("old.example.com", false),
        ("api.example.com", true),
    ] {
        entries.push(entry(host, flag));
    }
    entries
}

/// A list written with the raw `fst` builder (keys must already be in byte order).
fn raw(pairs: &[(&str, u64)]) -> Vec<u8> {
    let mut builder = MapBuilder::memory();
    for (key, value) in pairs {
        builder.insert(key, *value).expect("keys in order");
    }
    builder.into_inner().expect("finish")
}

fn today() -> Date {
    Date {
        year: 2026,
        month: 11,
        day: 20,
    }
}

fn date(year: i32, month: u32, day: u32) -> Date {
    Date { year, month, day }
}

/// Limits for small lists: any size passes, the rest as shipped.
fn small() -> Limits {
    Limits {
        min_entries: 1,
        ..Limits::default()
    }
}

fn source(retrieved: Option<Date>) -> Source {
    Source {
        name: "Chromium net/http/transport_security_state_static.json".to_string(),
        url: Some(
            "https://chromium.googlesource.com/chromium/src/+/main/x.json?format=TEXT".into(),
        ),
        git_ref: Some("main".to_string()),
        retrieved,
        input_sha256: Some("ab".repeat(32)),
        objects: Some(1234),
        force_https: Some(1200),
    }
}

/// The manifest `build` would write for `bytes`.
fn manifest_for(bytes: &[u8], retrieved: Option<Date>) -> Manifest {
    let preload = Preload::open(bytes.to_vec()).expect("open");
    Manifest::for_data(bytes, &preload, source(retrieved))
}

fn problems_of(verdict: &Verdict) -> String {
    verdict.problems.join(" | ")
}

fn has(texts: &[String], wanted: &str) -> bool {
    texts.iter().any(|text| text.contains(wanted))
}

// ---- Date ----------------------------------------------------------------------------------

#[test]
fn date_parse_accepts_real_dates() {
    assert_eq!(Date::parse("2026-11-20"), Ok(date(2026, 11, 20)));
    assert_eq!(Date::parse("1970-01-01"), Ok(date(1970, 1, 1)));
    assert_eq!(Date::parse("2026-12-31"), Ok(date(2026, 12, 31)));
    assert_eq!(Date::parse("9999-12-31"), Ok(date(9999, 12, 31)));
    assert_eq!(Date::parse("2026-04-30"), Ok(date(2026, 4, 30)));
}

#[test]
fn date_parse_refuses_anything_else() {
    let refused = [
        "2026-02-30", // no such day
        "2026-04-31",
        "2026-13-01", // no such month
        "2026-00-10",
        "2026-01-00",
        "2026-1-5", // not zero-padded
        "20261120",
        "2026/11/20",
        "2026-11-2x",
        "2026-11-200",
        "+026-11-20",
        " 2026-11-20",
        "2026-11-20 ",
        "2026-11-20T00:00:00Z",
        "26-11-20",
        "",
        "yesterday",
        "2026-11-2\u{e9}", // a multi-byte character where a digit belongs
    ];
    for text in refused {
        let error = Date::parse(text).expect_err(text);
        assert!(error.contains(&format!("{text:?}")), "{error}");
    }
}

#[test]
fn date_parse_errors_say_what_is_wrong() {
    let error = Date::parse("2026-02-30").unwrap_err();
    assert!(error.contains("February 2026 has 28 days"), "{error}");
    let error = Date::parse("2026-13-01").unwrap_err();
    assert!(error.contains("no month 13"), "{error}");
    let error = Date::parse("2026-1-5").unwrap_err();
    assert!(error.contains("YYYY-MM-DD"), "{error}");
}

#[test]
fn leap_days_exist_only_in_leap_years() {
    assert!(Date::parse("2024-02-29").is_ok());
    assert!(Date::parse("2000-02-29").is_ok()); // divisible by 400
    assert!(Date::parse("2400-02-29").is_ok());
    assert!(Date::parse("2023-02-29").is_err());
    assert!(Date::parse("1900-02-29").is_err()); // divisible by 100 but not 400
    assert!(Date::parse("2100-02-29").is_err());
    assert!(Date::parse("2026-02-29").is_err());
    assert_eq!(
        date(2024, 3, 1).days_since(date(2024, 2, 28)),
        2,
        "29 February 2024 lies between"
    );
    assert_eq!(date(2023, 3, 1).days_since(date(2023, 2, 28)), 1);
}

#[test]
fn date_displays_as_it_parses() {
    assert_eq!(date(2026, 1, 5).to_string(), "2026-01-05");
    assert_eq!(date(987, 12, 31).to_string(), "0987-12-31");
    assert_eq!(Date::parse("0987-12-31"), Ok(date(987, 12, 31)));
    assert_eq!(today().to_string(), "2026-11-20");
}

#[test]
fn dates_order_chronologically() {
    assert!(date(2026, 11, 20) > date(2026, 11, 19));
    assert!(date(2026, 12, 1) > date(2026, 11, 30));
    assert!(date(2027, 1, 1) > date(2026, 12, 31));
    assert!(date(1999, 12, 31) < date(2000, 1, 1));
}

#[test]
fn known_dates_have_the_right_unix_day() {
    assert_eq!(date(1970, 1, 1).unix_days(), 0);
    assert_eq!(date(1969, 12, 31).unix_days(), -1);
    assert_eq!(date(1970, 1, 2).unix_days(), 1);
    assert_eq!(date(2000, 1, 1).unix_days(), 10_957); // 946,684,800 s
    assert_eq!(date(2000, 3, 1).unix_days(), 11_017); // after the leap day
    assert_eq!(date(2038, 1, 19).unix_days(), 24_855); // 2^31 s falls on this day
    assert_eq!(date(2026, 11, 20).unix_days(), 20_777); // 1,795,132,800 s, checked with `date`
    assert_eq!(date(1, 1, 1).unix_days(), -719_162);
    assert_eq!(date(0, 1, 1).unix_days(), -719_528);
    assert_eq!(date(9999, 12, 31).unix_days(), 2_932_896);
    assert_eq!(Date::from_unix_days(20_777), date(2026, 11, 20));
    assert_eq!(Date::from_unix_days(0), date(1970, 1, 1));
    assert_eq!(Date::from_unix_days(-1), date(1969, 12, 31));
    assert_eq!(Date::from_unix_days(-719_162), date(1, 1, 1));
    assert_eq!(Date::from_unix_days(2_932_896), date(9999, 12, 31));
}

#[test]
fn every_thousandth_day_from_1970_to_2100_round_trips() {
    let last = date(2100, 12, 31).unix_days();
    let mut day = 0;
    let mut checked = 0;
    while day <= last {
        let converted = Date::from_unix_days(day);
        assert_eq!(converted.unix_days(), day, "{converted}");
        assert_eq!(Date::parse(&converted.to_string()), Ok(converted));
        day += 1_000;
        checked += 1;
    }
    assert!(checked > 47, "{checked}");
    // The same across the years before the epoch and far after 2100.
    for day in (-719_528..=2_932_896).step_by(997) {
        assert_eq!(Date::from_unix_days(day).unix_days(), day);
    }
}

fn month_length(year: i32, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[test]
fn walking_every_day_from_1970_to_2100_agrees_with_a_calendar_kept_the_long_way() {
    let mut current = date(1970, 1, 1);
    let mut day = 0_i64;
    while current.year <= 2100 {
        assert_eq!(current.unix_days(), day, "{current}");
        assert_eq!(Date::from_unix_days(day), current);
        assert_eq!(Date::parse(&current.to_string()), Ok(current));
        day += 1;
        if current.day < month_length(current.year, current.month) {
            current.day += 1;
        } else {
            current.day = 1;
            if current.month == 12 {
                current.month = 1;
                current.year += 1;
            } else {
                current.month += 1;
            }
        }
    }
    assert_eq!(current, date(2101, 1, 1));
    assert_eq!(day, current.unix_days());
}

#[test]
fn days_since_counts_across_year_ends_and_leap_days() {
    assert_eq!(date(2027, 1, 1).days_since(date(2026, 12, 31)), 1);
    assert_eq!(date(2026, 1, 1).days_since(date(2025, 1, 1)), 365);
    assert_eq!(date(2025, 1, 1).days_since(date(2024, 1, 1)), 366);
    assert_eq!(date(2025, 3, 1).days_since(date(2024, 2, 29)), 366);
    assert_eq!(date(2000, 3, 1).days_since(date(1900, 3, 1)), 36_525);
    assert_eq!(date(2026, 11, 20).days_since(date(2026, 11, 20)), 0);
    assert_eq!(date(2026, 11, 20).days_since(date(2026, 1, 1)), 323);
    assert_eq!(date(2026, 11, 10).days_since(date(2026, 11, 20)), -10);
}

#[test]
fn absurd_days_and_unreal_dates_never_panic() {
    let far_future = Date::from_unix_days(i64::MAX);
    let far_past = Date::from_unix_days(i64::MIN);
    assert!(far_future.year > 200_000, "{far_future}");
    assert!(far_past.year < -200_000, "{far_past}");
    assert_eq!(far_future, Date::from_unix_days(100_000_000));
    assert_eq!(far_past, Date::from_unix_days(-100_000_000));
    // Not real dates: any number comes out, none of them panics.
    let _ = date(2026, 13, 99).unix_days();
    let _ = date(i32::MIN, 0, u32::MAX).unix_days();
    let _ = date(i32::MAX, u32::MAX, 0).unix_days();
    let _ = date(2026, 13, 99).to_string();
}

#[test]
fn today_is_a_real_recent_date() {
    let today = Date::today_utc();
    assert!(today >= date(2024, 1, 1), "{today}");
    assert!((1..=12).contains(&today.month));
    assert!((1..=month_length(today.year, today.month)).contains(&today.day));
    assert_eq!(Date::parse(&today.to_string()), Ok(today));
}

// ---- SHA-256 -------------------------------------------------------------------------------

#[test]
fn sha256_matches_the_published_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // Two blocks long.
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    let digest = sha256_hex(&[0xff; 1000]);
    assert_eq!(digest.len(), 64);
    assert!(
        digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
}

// ---- Manifest ------------------------------------------------------------------------------

fn sample_manifest() -> Manifest {
    Manifest {
        format: FORMAT.to_string(),
        entries: 6,
        include_subdomains: 5,
        output_sha256: "cd".repeat(32),
        source: source(Some(date(2026, 11, 10))),
        tool: "hsts-preload 0.1.0".to_string(),
    }
}

#[test]
fn the_manifest_for_a_file_takes_its_counts_and_hash_from_the_data() {
    let (bytes, preload) = file(&good_entries());
    let manifest = Manifest::for_data(&bytes, &preload, source(None));
    assert_eq!(manifest.format, FORMAT);
    assert_eq!(manifest.entries, 11);
    assert_eq!(manifest.include_subdomains, 10);
    assert_eq!(manifest.output_sha256, sha256_hex(&bytes));
    assert_eq!(manifest.source, source(None));
    assert_eq!(
        manifest.tool,
        format!("hsts-preload {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn the_format_text_is_the_one_the_documents_name() {
    assert_eq!(
        FORMAT,
        "fst-map-v3; key = host name (lower-case ASCII); value = 2 * rank + include_subdomains"
    );
}

#[test]
fn manifest_json_has_sorted_keys_two_space_indent_and_a_final_newline() {
    let manifest = sample_manifest();
    let expected = format!(
        r#"{{
  "entries": 6,
  "format": "fst-map-v3; key = host name (lower-case ASCII); value = 2 * rank + include_subdomains",
  "include_subdomains": 5,
  "output_sha256": "{cd}",
  "source": {{
    "force_https": 1200,
    "input_sha256": "{ab}",
    "name": "Chromium net/http/transport_security_state_static.json",
    "objects": 1234,
    "ref": "main",
    "retrieved": "2026-11-10",
    "url": "https://chromium.googlesource.com/chromium/src/+/main/x.json?format=TEXT"
  }},
  "tool": "hsts-preload 0.1.0"
}}
"#,
        cd = "cd".repeat(32),
        ab = "ab".repeat(32),
    );
    assert_eq!(manifest.to_json(), expected);
}

#[test]
fn manifest_json_round_trips() {
    let manifest = sample_manifest();
    assert_eq!(Manifest::parse(&manifest.to_json()), Ok(manifest.clone()));
    // And the text is stable: parse then write gives the same text.
    let text = manifest.to_json();
    assert_eq!(Manifest::parse(&text).unwrap().to_json(), text);
}

#[test]
fn manifest_writes_unknown_fields_as_null_and_reads_them_back() {
    let manifest = Manifest {
        format: FORMAT.to_string(),
        entries: 166_958,
        include_subdomains: 166_708,
        output_sha256: "b9".repeat(32),
        source: Source {
            name: "servo-default-resources 0.8.0".to_string(),
            url: None,
            git_ref: None,
            retrieved: None,
            input_sha256: None,
            objects: None,
            force_https: None,
        },
        tool: "none: copied from servo-default-resources 0.8.0".to_string(),
    };
    let text = manifest.to_json();
    for key in [
        "force_https",
        "input_sha256",
        "objects",
        "ref",
        "retrieved",
        "url",
    ] {
        assert!(
            text.contains(&format!("    \"{key}\": null")),
            "{key}\n{text}"
        );
    }
    assert!(
        text.contains("\"name\": \"servo-default-resources 0.8.0\""),
        "{text}"
    );
    assert_eq!(Manifest::parse(&text), Ok(manifest));
}

#[test]
fn manifest_keys_come_out_sorted_at_both_levels() {
    let text = sample_manifest().to_json();
    let value: Value = serde_json::from_str(&text).expect("valid JSON");
    let keys_in_text = |indent: &str| -> Vec<String> {
        text.lines()
            .filter(|line| line.starts_with(indent) && !line.starts_with(&format!("{indent} ")))
            .filter_map(|line| {
                line.trim()
                    .strip_prefix('"')?
                    .split('"')
                    .next()
                    .map(str::to_string)
            })
            .collect()
    };
    let top = keys_in_text("  ");
    let inner = keys_in_text("    ");
    let mut sorted_top = top.clone();
    sorted_top.sort();
    let mut sorted_inner = inner.clone();
    sorted_inner.sort();
    assert_eq!(top, sorted_top);
    assert_eq!(inner, sorted_inner);
    assert_eq!(top.len(), 6, "{top:?}");
    assert_eq!(inner.len(), 7, "{inner:?}");
    assert_eq!(value["source"]["ref"], json!("main"));
}

#[test]
fn manifest_text_with_quotes_and_unicode_survives() {
    let mut manifest = sample_manifest();
    manifest.source.name =
        "a \"quoted\" name, a back\\slash, a new\nline and \u{e9}\u{4e2d}".to_string();
    manifest.tool = "tool \"x\"".to_string();
    assert_eq!(Manifest::parse(&manifest.to_json()), Ok(manifest));
}

/// The sample manifest as JSON, with the field at `path` replaced (`Some`) or removed (`None`).
fn edited(path: &str, replacement: Option<Value>) -> String {
    let mut value: Value = serde_json::from_str(&sample_manifest().to_json()).expect("json");
    let mut keys: Vec<&str> = path.split('.').collect();
    let last = keys.pop().expect("a path");
    let mut target = &mut value;
    for key in keys {
        target = target.get_mut(key).expect("the parent exists");
    }
    let object = target.as_object_mut().expect("an object");
    match replacement {
        Some(new) => {
            object.insert(last.to_string(), new);
        }
        None => {
            object.remove(last);
        }
    }
    value.to_string()
}

#[test]
fn a_missing_manifest_field_is_named() {
    for path in [
        "entries",
        "format",
        "include_subdomains",
        "output_sha256",
        "source",
        "tool",
        "source.name",
        "source.url",
        "source.ref",
        "source.retrieved",
        "source.input_sha256",
        "source.objects",
        "source.force_https",
    ] {
        let error = Manifest::parse(&edited(path, None)).expect_err(path);
        assert!(error.contains(&format!("`{path}`")), "{path}: {error}");
        assert!(error.contains("no field"), "{path}: {error}");
    }
}

#[test]
fn a_manifest_field_of_the_wrong_type_is_named() {
    let cases: [(&str, Value); 17] = [
        ("entries", json!("6")),
        ("entries", json!(-1)),
        ("entries", json!(1.5)),
        ("entries", json!(null)),
        ("include_subdomains", json!(true)),
        ("output_sha256", json!(5)),
        ("format", json!(["x"])),
        ("tool", json!(null)),
        ("source", json!("x")),
        ("source.name", json!(1)),
        ("source.url", json!(5)),
        ("source.ref", json!({})),
        ("source.retrieved", json!(20_261_110)),
        ("source.input_sha256", json!(false)),
        ("source.objects", json!("12")),
        ("source.objects", json!(1.5)),
        ("source.force_https", json!(-3)),
    ];
    for (path, value) in cases {
        let error = Manifest::parse(&edited(path, Some(value.clone()))).expect_err(path);
        assert!(
            error.contains(&format!("`{path}` must be")),
            "{path} = {value}: {error}"
        );
    }
}

#[test]
fn a_bad_retrieved_date_is_named_with_the_reason() {
    for text in ["2026-02-30", "yesterday", "2026-1-5", ""] {
        let error = Manifest::parse(&edited("source.retrieved", Some(json!(text)))).unwrap_err();
        assert!(error.contains("`source.retrieved`"), "{error}");
        assert!(error.contains(&format!("{text:?}")), "{error}");
    }
}

#[test]
fn text_that_is_not_a_manifest_is_refused() {
    for text in ["", "not json", "[]", "5", "null", "{", "{\"entries\": 1}"] {
        assert!(Manifest::parse(text).is_err(), "{text:?}");
    }
    assert!(
        Manifest::parse("not json")
            .unwrap_err()
            .contains("not valid JSON")
    );
    assert!(
        Manifest::parse("[]")
            .unwrap_err()
            .contains("must be a JSON object")
    );
}

#[test]
fn a_key_the_tool_does_not_know_is_ignored() {
    let text = edited("note", Some(json!("added by hand")));
    assert_eq!(Manifest::parse(&text), Ok(sample_manifest()));
    let text = edited("source.comment", Some(json!(["x"])));
    assert_eq!(Manifest::parse(&text), Ok(sample_manifest()));
}

// ---- check ---------------------------------------------------------------------------------

#[test]
fn a_good_small_file_passes_check_with_notes() {
    let (bytes, _) = file(&good_entries());
    let verdict = check(&bytes, None, today(), &small());
    assert!(verdict.ok(), "{}", problems_of(&verdict));
    assert!(verdict.problems.is_empty());
    assert!(
        has(
            &verdict.notes,
            "11 entries, 10 of them include subdomains (90.9 %)"
        ),
        "{:?}",
        verdict.notes
    );
    assert!(
        has(&verdict.notes, &format!("SHA-256 {}", sha256_hex(&bytes))),
        "{:?}",
        verdict.notes
    );
    assert!(has(&verdict.notes, "age unknown"), "{:?}", verdict.notes);
    assert!(
        !has(&verdict.notes, "byte-identical"),
        "{:?}",
        verdict.notes
    );
}

#[test]
fn a_good_file_with_its_manifest_passes_and_says_how_old_it_is() {
    let (bytes, _) = file(&good_entries());
    let manifest = manifest_for(&bytes, Some(date(2026, 11, 10)));
    let verdict = check(&bytes, Some(&manifest), today(), &small());
    assert!(verdict.ok(), "{}", problems_of(&verdict));
    assert!(
        has(&verdict.notes, "10 days old (retrieved 2026-11-10)"),
        "{:?}",
        verdict.notes
    );
}

#[test]
fn the_default_limits_are_the_shipped_constants() {
    let limits = Limits::default();
    assert_eq!(limits.min_entries, 50_000);
    assert_eq!(limits.min_include_percent, 90);
    assert_eq!(limits.max_age_days, None);
    // A small list fails the default minimum, which is the point of it.
    let (bytes, _) = file(&good_entries());
    let verdict = check(&bytes, None, today(), &limits);
    assert!(!verdict.ok());
    assert!(
        has(&verdict.problems, "11 entries"),
        "{}",
        problems_of(&verdict)
    );
    assert!(
        has(&verdict.problems, "50,000"),
        "{}",
        problems_of(&verdict)
    );
}

#[test]
fn a_file_that_cannot_be_opened_gives_exactly_one_problem() {
    let (good, _) = file(&good_entries());
    let mut flipped = good.clone();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0xff;
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("not a map", b"this is not an fst map at all".to_vec()),
        ("truncated", good[..good.len() / 2].to_vec()),
        ("a flipped byte", flipped),
        ("an upper-case key", raw(&[("Example.com", 1)])),
        ("a key with a space", raw(&[("a b", 1)])),
    ];
    let manifest = manifest_for(&good, Some(date(2026, 11, 10)));
    for (name, bytes) in cases {
        for manifest in [None, Some(&manifest)] {
            let verdict = check(&bytes, manifest, today(), &small());
            assert_eq!(verdict.problems.len(), 1, "{name}: {:?}", verdict.problems);
            assert!(
                verdict.problems[0].contains("cannot be used"),
                "{name}: {:?}",
                verdict.problems
            );
            assert!(verdict.notes.is_empty(), "{name}: {:?}", verdict.notes);
            assert!(!verdict.ok());
        }
    }
}

#[test]
fn a_breach_of_the_id_scheme_is_a_problem_and_the_file_is_not_a_rebuild() {
    // The values `fst` writes from a plain list of 0 and 1 flags: valid for Servo, not ours.
    let bytes = raw(&[
        ("accounts.google.com", 1),
        ("dev", 1),
        ("dropbox.com", 1),
        ("github.com", 1),
    ]);
    let verdict = check(&bytes, None, today(), &small());
    assert_eq!(verdict.problems.len(), 1, "{}", problems_of(&verdict));
    assert!(
        verdict.problems[0].contains("the host \"dev\""),
        "{}",
        problems_of(&verdict)
    );
    assert!(
        has(&verdict.notes, "not byte-identical to a rebuild"),
        "{:?}",
        verdict.notes
    );
}

#[test]
fn too_few_entries_is_a_problem() {
    let (bytes, _) = file(&good_entries());
    let few = Limits {
        min_entries: 12,
        ..small()
    };
    let verdict = check(&bytes, None, today(), &few);
    assert_eq!(verdict.problems.len(), 1, "{}", problems_of(&verdict));
    assert!(
        verdict.problems[0].contains("11 entries"),
        "{}",
        problems_of(&verdict)
    );
    assert!(
        verdict.problems[0].contains("minimum of 12"),
        "{}",
        problems_of(&verdict)
    );
    let exact = Limits {
        min_entries: 11,
        ..small()
    };
    assert!(check(&bytes, None, today(), &exact).ok());
}

#[test]
fn the_include_share_must_reach_the_minimum() {
    let (at_90, _) = file(&shaped(100, 90));
    let (at_89, _) = file(&shaped(100, 89));
    let ok = check(&at_90, None, today(), &small());
    assert!(ok.ok(), "{}", problems_of(&ok));
    let verdict = check(&at_89, None, today(), &small());
    assert_eq!(verdict.problems.len(), 1, "{}", problems_of(&verdict));
    assert!(
        verdict.problems[0].contains("89.0 %"),
        "{}",
        problems_of(&verdict)
    );
    assert!(
        verdict.problems[0].contains("minimum of 90 %"),
        "{}",
        problems_of(&verdict)
    );
    // The limit is a parameter.
    let strict = Limits {
        min_include_percent: 100,
        ..small()
    };
    assert!(!check(&at_90, None, today(), &strict).ok());
    let lax = Limits {
        min_include_percent: 50,
        ..small()
    };
    assert!(check(&at_89, None, today(), &lax).ok());
}

#[test]
fn each_canary_must_be_present() {
    for canary in CANARIES {
        let entries: Vec<Entry> = shaped(100, 100)
            .into_iter()
            .filter(|entry| entry.host != canary)
            .collect();
        let (bytes, _) = file(&entries);
        let verdict = check(&bytes, None, today(), &small());
        assert_eq!(
            verdict.problems.len(),
            1,
            "{canary}: {}",
            problems_of(&verdict)
        );
        assert!(
            verdict.problems[0].contains(&format!("{canary} is missing")),
            "{canary}: {}",
            problems_of(&verdict)
        );
    }
}

#[test]
fn each_canary_must_include_subdomains() {
    for canary in CANARIES {
        let entries: Vec<Entry> = shaped(100, 100)
            .into_iter()
            .map(|entry| {
                if entry.host == canary {
                    Entry {
                        include_subdomains: false,
                        ..entry
                    }
                } else {
                    entry
                }
            })
            .collect();
        let (bytes, _) = file(&entries);
        let verdict = check(&bytes, None, today(), &small());
        assert_eq!(
            verdict.problems.len(),
            1,
            "{canary}: {}",
            problems_of(&verdict)
        );
        assert!(
            verdict.problems[0].contains(&format!("{canary} is listed without include_subdomains")),
            "{canary}: {}",
            problems_of(&verdict)
        );
    }
}

#[test]
fn a_manifest_that_disagrees_with_the_file_is_a_problem_for_each_field() {
    let (bytes, _) = file(&good_entries());
    let right = manifest_for(&bytes, Some(date(2026, 11, 10)));

    let mut wrong_entries = right.clone();
    wrong_entries.entries += 1;
    let verdict = check(&bytes, Some(&wrong_entries), today(), &small());
    assert_eq!(verdict.problems.len(), 1, "{}", problems_of(&verdict));
    assert!(
        verdict.problems[0].contains("manifest says the list has 12 entries, but the file has 11")
    );

    let mut wrong_includes = right.clone();
    wrong_includes.include_subdomains -= 1;
    let verdict = check(&bytes, Some(&wrong_includes), today(), &small());
    assert_eq!(verdict.problems.len(), 1, "{}", problems_of(&verdict));
    assert!(
        verdict.problems[0]
            .contains("manifest says 9 entries include subdomains, but the file has 10")
    );

    let mut wrong_hash = right.clone();
    wrong_hash.output_sha256 = "0".repeat(64);
    let verdict = check(&bytes, Some(&wrong_hash), today(), &small());
    assert_eq!(verdict.problems.len(), 1, "{}", problems_of(&verdict));
    assert!(
        verdict.problems[0].contains("output_sha256"),
        "{}",
        problems_of(&verdict)
    );
    assert!(
        verdict.problems[0].contains(&sha256_hex(&bytes)),
        "{}",
        problems_of(&verdict)
    );

    // A manifest that does not hold a hash at all (an empty string) is a mismatch too.
    let mut no_hash = right.clone();
    no_hash.output_sha256 = String::new();
    assert!(!check(&bytes, Some(&no_hash), today(), &small()).ok());

    // The hash in capital letters is the same hash.
    let mut shouted = right;
    shouted.output_sha256 = shouted.output_sha256.to_uppercase();
    assert!(check(&bytes, Some(&shouted), today(), &small()).ok());
}

#[test]
fn a_manifest_for_another_file_fails_on_the_hash_and_the_counts() {
    let (bytes, _) = file(&good_entries());
    let (other, _) = file(&shaped(20, 20));
    let manifest = manifest_for(&other, None);
    let verdict = check(&bytes, Some(&manifest), today(), &small());
    assert_eq!(verdict.problems.len(), 3, "{}", problems_of(&verdict));
}

#[test]
fn an_empty_list_fails_every_rule_it_can_without_dividing_by_zero() {
    let (bytes, empty) = file(&[]);
    let verdict = check(&bytes, None, today(), &small());
    // Too few entries, and the four canaries; the include share of nothing is not a failure.
    assert_eq!(verdict.problems.len(), 5, "{}", problems_of(&verdict));
    assert!(has(
        &verdict.notes,
        "0 entries, 0 of them include subdomains (0.0 %)"
    ));
    let error = guard(None, &empty, &small()).unwrap_err();
    assert!(error.contains("only 0 entries"), "{error}");
    let lax = Limits {
        min_entries: 0,
        ..small()
    };
    let error = guard(None, &empty, &lax).unwrap_err();
    assert!(error.contains("is missing"), "{error}");
}

#[test]
fn every_problem_is_reported_not_only_the_first() {
    let bytes = raw(&[("a.example", 1), ("b.example", 1)]); // scheme ok for the first, not the second
    let manifest = Manifest {
        entries: 99,
        ..manifest_for(&bytes, None)
    };
    let limits = Limits {
        min_entries: 10,
        min_include_percent: 90,
        max_age_days: Some(5),
    };
    let verdict = check(&bytes, Some(&manifest), today(), &limits);
    // scheme, size, four canaries, the manifest's count, the unknown age.
    assert_eq!(verdict.problems.len(), 8, "{}", problems_of(&verdict));
    assert!(!verdict.ok());
}

#[test]
fn a_list_older_than_the_limit_is_a_problem_and_one_at_the_limit_is_not() {
    let (bytes, _) = file(&good_entries());
    let manifest = manifest_for(&bytes, Some(date(2026, 1, 1)));
    assert_eq!(today().days_since(date(2026, 1, 1)), 323);
    let limit = |days| Limits {
        max_age_days: Some(days),
        ..small()
    };

    let verdict = check(&bytes, Some(&manifest), today(), &limit(322));
    assert_eq!(verdict.problems.len(), 1, "{}", problems_of(&verdict));
    assert!(
        verdict.problems[0].contains("2026-01-01"),
        "{}",
        problems_of(&verdict)
    );
    assert!(
        verdict.problems[0].contains("323 days"),
        "{}",
        problems_of(&verdict)
    );
    assert!(
        verdict.problems[0].contains("limit of 322 days"),
        "{}",
        problems_of(&verdict)
    );
    assert!(check(&bytes, Some(&manifest), today(), &limit(323)).ok());
    assert!(check(&bytes, Some(&manifest), today(), &limit(400)).ok());
    // Without a limit the age is a note and never fails.
    let verdict = check(&bytes, Some(&manifest), today(), &small());
    assert!(verdict.ok());
    assert!(has(&verdict.notes, "323 days old"), "{:?}", verdict.notes);
}

#[test]
fn an_age_that_is_unknown_cannot_meet_a_limit() {
    let (bytes, _) = file(&good_entries());
    let limits = Limits {
        max_age_days: Some(30),
        ..small()
    };
    // No manifest.
    let verdict = check(&bytes, None, today(), &limits);
    assert_eq!(verdict.problems.len(), 1, "{}", problems_of(&verdict));
    assert!(
        verdict.problems[0].contains("age of the list is unknown"),
        "{}",
        problems_of(&verdict)
    );
    assert!(
        verdict.problems[0].contains("limit of 30 days"),
        "{}",
        problems_of(&verdict)
    );
    assert!(
        has(&verdict.notes, "age unknown (no manifest)"),
        "{:?}",
        verdict.notes
    );
    // A manifest without a retrieval date (the file copied from Servo).
    let manifest = manifest_for(&bytes, None);
    let verdict = check(&bytes, Some(&manifest), today(), &limits);
    assert_eq!(verdict.problems.len(), 1, "{}", problems_of(&verdict));
    assert!(
        has(
            &verdict.notes,
            "age unknown (the manifest records no retrieval date)"
        ),
        "{:?}",
        verdict.notes
    );
    // And with no limit it is only a note.
    assert!(check(&bytes, Some(&manifest), today(), &small()).ok());
}

#[test]
fn the_age_note_counts_days_and_says_day_for_one() {
    let (bytes, _) = file(&good_entries());
    for (retrieved, wanted) in [
        (date(2026, 11, 20), "0 days old"),
        (date(2026, 11, 19), "1 day old"),
        (date(2026, 11, 18), "2 days old"),
        (date(2025, 11, 20), "365 days old"),
    ] {
        let manifest = manifest_for(&bytes, Some(retrieved));
        let verdict = check(&bytes, Some(&manifest), today(), &small());
        assert!(has(&verdict.notes, wanted), "{wanted}: {:?}", verdict.notes);
    }
}

#[test]
fn a_retrieved_date_in_the_future_is_a_note_and_never_a_problem() {
    let (bytes, _) = file(&good_entries());
    let manifest = manifest_for(&bytes, Some(date(2026, 12, 1)));
    let limits = Limits {
        max_age_days: Some(0),
        ..small()
    };
    let verdict = check(&bytes, Some(&manifest), today(), &limits);
    assert!(verdict.ok(), "{}", problems_of(&verdict));
    assert!(
        has(&verdict.notes, "retrieved date is in the future"),
        "{:?}",
        verdict.notes
    );
    assert!(has(&verdict.notes, "2026-12-01"), "{:?}", verdict.notes);
    assert!(!has(&verdict.notes, "days old"), "{:?}", verdict.notes);
}

#[test]
fn the_shipped_file_is_checked_at_its_real_size() {
    let bytes = std::fs::read(shipped_path()).expect("read the shipped file");
    let preload = Preload::open(bytes.clone()).expect("open");
    assert_eq!(
        sha256_hex(&bytes),
        "b94818a7e0c3515f9fd6a50af51e32019cbb50ee1015e1e56471fb3b06d72c42"
    );

    // The canaries are chosen so that the list Servo 0.8.0 ships meets every default rule: if
    // `CANARIES` ever names a host the shipped file lists without the flag (`paypal.com` is
    // one), the default guards would refuse the very list they are meant to protect.
    for host in CANARIES {
        assert_eq!(preload.get(host), Some(true), "{host}");
    }
    let no_problems = |verdict: &Verdict| {
        assert!(verdict.ok(), "{}", problems_of(verdict));
    };

    let verdict = check(&bytes, None, today(), &Limits::default());
    no_problems(&verdict);
    assert!(
        has(
            &verdict.notes,
            "166,958 entries, 166,708 of them include subdomains (99.9 %)"
        ),
        "{:?}",
        verdict.notes
    );
    assert!(
        has(&verdict.notes, "age unknown (no manifest)"),
        "{:?}",
        verdict.notes
    );

    // Its manifest, made from the data, agrees; the JSON round trips at this size too.
    let manifest = Manifest::for_data(&bytes, &preload, source(None));
    assert_eq!(
        (manifest.entries, manifest.include_subdomains),
        (166_958, 166_708)
    );
    let parsed = Manifest::parse(&manifest.to_json()).expect("parse");
    assert_eq!(parsed, manifest);
    no_problems(&check(&bytes, Some(&parsed), today(), &Limits::default()));

    // The refresh guards accept it as its own replacement: size, share, canaries and shrink.
    assert_eq!(
        guard_refresh(
            Some(&preload),
            &preload,
            &Limits::default(),
            DEFAULT_MAX_SHRINK_PERCENT,
            false,
        ),
        Ok(())
    );
    let nothing = diff(&preload, &preload);
    assert!(nothing.is_empty());
    assert_eq!(
        render_diff(&nothing, Some(20)),
        "added 0 / removed 0 / changed 0\n"
    );
}

fn shipped_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../third_party/servo-default-resources/resources/hsts_preload.fstmap")
}

// ---- guard_refresh -------------------------------------------------------------------------

fn guard(old: Option<&Preload>, new: &Preload, limits: &Limits) -> Result<(), String> {
    guard_refresh(old, new, limits, DEFAULT_MAX_SHRINK_PERCENT, false)
}

#[test]
fn a_good_new_list_passes_the_guards_with_or_without_an_old_file() {
    let new = open(&good_entries());
    assert_eq!(guard(None, &new, &small()), Ok(()));
    let old = open(&shaped(11, 11));
    assert_eq!(guard(Some(&old), &new, &small()), Ok(()));
}

#[test]
fn a_new_list_below_the_minimum_size_is_refused_with_the_numbers_and_the_way_out() {
    let new = open(&good_entries());
    let error = guard(None, &new, &Limits::default()).unwrap_err();
    assert!(error.contains("only 11 entries"), "{error}");
    assert!(error.contains("minimum of 50,000"), "{error}");
    assert!(error.contains("--min-entries"), "{error}");
    // A parameter, and the minimum itself passes.
    let limits = Limits {
        min_entries: 11,
        ..Limits::default()
    };
    assert_eq!(guard(None, &new, &limits), Ok(()));
    let limits = Limits {
        min_entries: 12,
        ..Limits::default()
    };
    assert!(guard(None, &new, &limits).is_err());
}

#[test]
fn a_new_list_with_too_few_include_subdomains_entries_is_refused() {
    let error = guard(None, &open(&shaped(100, 89)), &small()).unwrap_err();
    assert!(error.contains("89.0 %"), "{error}");
    assert!(error.contains("minimum of 90 %"), "{error}");
    assert!(error.contains("include_subdomains"), "{error}");
    assert_eq!(guard(None, &open(&shaped(100, 90)), &small()), Ok(()));
}

#[test]
fn a_new_list_without_a_canary_or_without_its_flag_is_refused() {
    for canary in CANARIES {
        let without: Vec<Entry> = shaped(100, 100)
            .into_iter()
            .filter(|entry| entry.host != canary)
            .collect();
        let error = guard(None, &open(&without), &small()).unwrap_err();
        assert!(
            error.contains(&format!("{canary} is missing")),
            "{canary}: {error}"
        );

        let unflagged: Vec<Entry> = shaped(100, 100)
            .into_iter()
            .map(|entry| Entry {
                include_subdomains: entry.include_subdomains && entry.host != canary,
                ..entry
            })
            .collect();
        let error = guard(None, &open(&unflagged), &small()).unwrap_err();
        assert!(
            error.contains(&format!("{canary} is listed without include_subdomains")),
            "{canary}: {error}"
        );
    }
}

#[test]
fn shrinking_by_exactly_the_limit_passes_and_one_entry_more_fails() {
    let old = open(&shaped(100, 100));
    let at_limit = open(&shaped(90, 90));
    let beyond = open(&shaped(89, 89));
    assert_eq!(guard(Some(&old), &at_limit, &small()), Ok(()));
    let error = guard(Some(&old), &beyond, &small()).unwrap_err();
    assert!(error.contains("11.0 % fewer entries"), "{error}");
    assert!(error.contains("89 instead of 100"), "{error}");
    assert!(error.contains("limit 10 %"), "{error}");
    assert!(error.contains("--allow-shrink"), "{error}");

    // The same at another size: 900 of 1,000 passes, 899 does not.
    let old = open(&shaped(1_000, 1_000));
    assert_eq!(
        guard(Some(&old), &open(&shaped(900, 900)), &small()),
        Ok(())
    );
    assert!(guard(Some(&old), &open(&shaped(899, 899)), &small()).is_err());
}

#[test]
fn the_shrink_percentage_shown_is_rounded_up_so_it_is_always_over_the_limit() {
    let old = open(&shaped(10_000, 10_000));
    // 1,001 of 10,000 lost is 10.01 %: shown as 10.1 %, never as "10.0 %" against "limit 10 %".
    let error = guard(Some(&old), &open(&shaped(8_999, 8_999)), &small()).unwrap_err();
    assert!(error.contains("10.1 % fewer entries"), "{error}");
    assert_eq!(
        guard(Some(&old), &open(&shaped(9_000, 9_000)), &small()),
        Ok(())
    );
}

#[test]
fn the_shrink_limit_is_a_parameter_and_growth_always_passes() {
    let old = open(&shaped(100, 100));
    let same = open(&shaped(100, 100));
    let bigger = open(&shaped(150, 150));
    let one_fewer = open(&shaped(99, 99));
    assert_eq!(guard_refresh(Some(&old), &same, &small(), 0, false), Ok(()));
    assert_eq!(
        guard_refresh(Some(&old), &bigger, &small(), 0, false),
        Ok(())
    );
    assert!(guard_refresh(Some(&old), &one_fewer, &small(), 0, false).is_err());
    assert_eq!(
        guard_refresh(Some(&old), &one_fewer, &small(), 1, false),
        Ok(())
    );
    assert_eq!(
        guard_refresh(Some(&old), &open(&shaped(5, 5)), &small(), 100, false),
        Ok(())
    );
    assert_eq!(guard(Some(&old), &bigger, &small()), Ok(()));
}

#[test]
fn allow_shrink_waives_the_shrink_rule_and_only_that() {
    let old = open(&shaped(100, 100));
    let shrunk = open(&shaped(50, 50));
    assert!(guard_refresh(Some(&old), &shrunk, &small(), 10, false).is_err());
    assert_eq!(
        guard_refresh(Some(&old), &shrunk, &small(), 10, true),
        Ok(())
    );

    // Size, include share and canaries still apply.
    let too_small = Limits {
        min_entries: 60,
        ..small()
    };
    let error = guard_refresh(Some(&old), &shrunk, &too_small, 10, true).unwrap_err();
    assert!(error.contains("only 50 entries"), "{error}");
    let low_share = open(&shaped(50, 40));
    let error = guard_refresh(Some(&old), &low_share, &small(), 10, true).unwrap_err();
    assert!(error.contains("80.0 %"), "{error}");
    let no_canary: Vec<Entry> = shaped(50, 50)
        .into_iter()
        .filter(|entry| entry.host != "dropbox.com")
        .collect();
    let error = guard_refresh(Some(&old), &open(&no_canary), &small(), 10, true).unwrap_err();
    assert!(error.contains("dropbox.com is missing"), "{error}");
}

#[test]
fn the_first_failing_rule_is_the_one_reported() {
    let old = open(&shaped(200, 200));
    // Breaks all four rules: small, unflagged, no canary, shrunk.
    let mut hollow = canaries(false);
    hollow.truncate(3);
    hollow.extend((0..40).map(|number| entry(&format!("host-{number:05}.test"), false)));
    let hollow = open(&hollow);
    let limits = Limits {
        min_entries: 100,
        ..Limits::default()
    };
    let error = guard(Some(&old), &hollow, &limits).unwrap_err();
    assert!(error.contains("only 43 entries"), "size first: {error}");

    // Big enough: the share comes next.
    let error = guard(Some(&old), &hollow, &small()).unwrap_err();
    assert!(
        error.contains("0.0 % of the new list"),
        "share second: {error}"
    );

    // A share that holds, a canary missing, and a shrink: the canary comes before the shrink.
    let mut missing = shaped(150, 150);
    missing.retain(|entry| entry.host != "github.com");
    let error = guard(Some(&old), &open(&missing), &small()).unwrap_err();
    assert!(
        error.contains("github.com is missing"),
        "canary third: {error}"
    );

    // Only the shrink is wrong.
    let error = guard(Some(&old), &open(&shaped(150, 150)), &small()).unwrap_err();
    assert!(
        error.contains("25.0 % fewer entries"),
        "shrink last: {error}"
    );
}

#[test]
fn the_age_limit_plays_no_part_in_a_refresh() {
    let limits = Limits {
        max_age_days: Some(0),
        ..small()
    };
    assert_eq!(guard(None, &open(&good_entries()), &limits), Ok(()));
}

// ---- diff ----------------------------------------------------------------------------------

fn list(pairs: &[(&str, bool)]) -> Preload {
    let entries: Vec<Entry> = pairs
        .iter()
        .map(|(host, flag)| entry(host, *flag))
        .collect();
    open(&entries)
}

#[test]
fn equal_lists_have_an_empty_diff() {
    let a = list(&[("a.example", true), ("b.example", false)]);
    let b = list(&[("b.example", false), ("a.example", true)]);
    let result = diff(&a, &b);
    assert_eq!(result, Diff::default());
    assert!(result.is_empty());
    assert_eq!(
        render_diff(&result, None),
        "added 0 / removed 0 / changed 0\n"
    );
    assert_eq!(
        render_diff(&result, Some(20)),
        "added 0 / removed 0 / changed 0\n"
    );
    let nothing = list(&[]);
    assert!(diff(&nothing, &nothing).is_empty());
}

#[test]
fn diff_finds_added_removed_and_flag_changed_hosts_in_key_order() {
    let old = list(&[
        ("a.example", true),
        ("b.example", false),
        ("c.example", true),
        ("d.example", true),
    ]);
    let new = list(&[
        ("0.example", true),
        ("a.example", true),
        ("b.example", true),
        ("d.example", false),
        ("e.example", false),
    ]);
    let result = diff(&old, &new);
    assert_eq!(
        result.added,
        vec![entry("0.example", true), entry("e.example", false)]
    );
    assert_eq!(result.removed, vec![entry("c.example", true)]);
    // The entry as it is in the new list.
    assert_eq!(
        result.flag_changed,
        vec![entry("b.example", true), entry("d.example", false)]
    );
    assert!(!result.is_empty());
}

#[test]
fn diff_against_an_empty_list_is_all_added_or_all_removed() {
    let some = list(&[("a.example", true), ("b.example", false)]);
    let none = list(&[]);
    let result = diff(&none, &some);
    assert_eq!(
        result.added,
        vec![entry("a.example", true), entry("b.example", false)]
    );
    assert!(result.removed.is_empty() && result.flag_changed.is_empty());
    let result = diff(&some, &none);
    assert_eq!(
        result.removed,
        vec![entry("a.example", true), entry("b.example", false)]
    );
    assert!(result.added.is_empty() && result.flag_changed.is_empty());
}

#[test]
fn diff_handles_disjoint_and_interleaved_lists() {
    let old = list(&[
        ("b.example", true),
        ("d.example", true),
        ("f.example", true),
    ]);
    let new = list(&[
        ("a.example", true),
        ("c.example", true),
        ("e.example", true),
        ("g.example", true),
    ]);
    let result = diff(&old, &new);
    let hosts = |entries: &[Entry]| {
        entries
            .iter()
            .map(|entry| entry.host.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        hosts(&result.added),
        ["a.example", "c.example", "e.example", "g.example"]
    );
    assert_eq!(
        hosts(&result.removed),
        ["b.example", "d.example", "f.example"]
    );
    assert!(result.flag_changed.is_empty());
    // A prefix is not the same host, and a tail on either side is picked up.
    let result = diff(
        &list(&[("a", true)]),
        &list(&[("a", true), ("a.b", false), ("a.c", true)]),
    );
    assert_eq!(hosts(&result.added), ["a.b", "a.c"]);
}

#[test]
fn diff_is_symmetric_for_additions_and_removals() {
    let one = list(&[("a.example", true), ("b.example", false)]);
    let two = list(&[("b.example", true), ("c.example", true)]);
    let forward = diff(&one, &two);
    let backward = diff(&two, &one);
    assert_eq!(forward.added, backward.removed);
    assert_eq!(forward.removed, backward.added);
    assert_eq!(forward.flag_changed, vec![entry("b.example", true)]);
    assert_eq!(backward.flag_changed, vec![entry("b.example", false)]);
}

fn numbered(count: usize, flag: bool) -> Vec<Entry> {
    (1..=count)
        .map(|number| entry(&format!("host-{number}.example"), flag))
        .collect()
}

#[test]
fn render_diff_shows_up_to_the_limit_per_kind_and_counts_the_rest() {
    let result = Diff {
        added: numbered(5, true),
        removed: vec![entry("gone.example", true)],
        flag_changed: vec![
            entry("one.example", true),
            entry("two.example", false),
            entry("three.example", true),
        ],
    };
    let text = render_diff(&result, Some(2));
    assert_eq!(
        text,
        "added 5 / removed 1 / changed 3\n\
         + host-1.example (includes subdomains)\n\
         + host-2.example (includes subdomains)\n\
         ... and 3 more added\n\
         - gone.example (included subdomains)\n\
         ~ one.example (now includes subdomains)\n\
         ~ two.example (now host only)\n\
         ... and 1 more changed\n"
    );
}

#[test]
fn render_diff_without_a_limit_lists_everything() {
    let result = Diff {
        added: numbered(5, false),
        removed: numbered(2, true),
        flag_changed: Vec::new(),
    };
    let text = render_diff(&result, None);
    assert!(!text.contains("... and"), "{text}");
    assert_eq!(text.lines().count(), 1 + 5 + 2);
    assert_eq!(text.lines().next(), Some("added 5 / removed 2 / changed 0"));
    let plus: Vec<&str> = text.lines().filter(|line| line.starts_with("+ ")).collect();
    assert_eq!(plus.len(), 5);
    assert_eq!(plus[0], "+ host-1.example (host only)");
    assert_eq!(plus[4], "+ host-5.example (host only)");
    assert!(text.ends_with('\n'));
    assert!(!text.contains("~ "), "{text}");
}

#[test]
fn render_diff_limit_boundaries() {
    let result = Diff {
        added: numbered(5, true),
        ..Diff::default()
    };
    // Exactly as many as the limit: no "more" line. One fewer: one.
    let text = render_diff(&result, Some(5));
    assert!(!text.contains("... and"), "{text}");
    let text = render_diff(&result, Some(4));
    assert!(text.contains("... and 1 more added"), "{text}");
    assert!(!text.contains("host-5.example"), "{text}");
    // A limit above the count is fine, and zero shows the header and the counts of the rest.
    assert_eq!(render_diff(&result, Some(100)).lines().count(), 6);
    let text = render_diff(&result, Some(0));
    assert_eq!(
        text,
        "added 5 / removed 0 / changed 0\n... and 5 more added\n"
    );
}

#[test]
fn render_diff_of_a_real_diff_keeps_key_order_and_the_total_in_the_header() {
    let old = list(&[("b.example", false), ("x.example", true)]);
    let new = list(&[
        ("a.example", true),
        ("b.example", true),
        ("c.example", false),
        ("d.example", true),
    ]);
    let text = render_diff(&diff(&old, &new), Some(2));
    assert_eq!(
        text,
        "added 3 / removed 1 / changed 1\n\
         + a.example (includes subdomains)\n\
         + c.example (host only)\n\
         ... and 1 more added\n\
         - x.example (included subdomains)\n\
         ~ b.example (now includes subdomains)\n"
    );
}

// ---------------------------------------------------------------- the flag-loss guard

#[test]
fn losing_the_flag_on_exactly_the_limit_share_of_shared_hosts_passes_and_one_more_fails() {
    let old = open(&shaped(100, 100));
    // One host of a hundred (1 %) passes at a limit of 1 %.
    assert_eq!(guard_flag_loss(&old, &open(&shaped(100, 99)), 1), Ok(()));
    // Two of a hundred do not, and the sentence has the numbers, the first hosts and the way out.
    let error = guard_flag_loss(&old, &open(&shaped(100, 98)), 1).unwrap_err();
    assert!(error.contains("2 of the 100 hosts"), "{error}");
    assert!(error.contains("(2.0 %)"), "{error}");
    assert!(error.contains("limit 1 %"), "{error}");
    assert!(
        error.contains("host-00094.test, host-00095.test"),
        "{error}"
    );
    assert!(error.contains("--allow-flag-loss"), "{error}");
    // The limit is a parameter: 0 % refuses a single host, 2 % lets two pass.
    assert!(guard_flag_loss(&old, &open(&shaped(100, 99)), 0).is_err());
    assert_eq!(guard_flag_loss(&old, &open(&shaped(100, 98)), 2), Ok(()));
}

#[test]
fn only_flags_lost_on_shared_hosts_count_as_flag_loss() {
    // Flags gained do not count.
    let few_flags = open(&shaped(100, 50));
    assert_eq!(
        guard_flag_loss(&few_flags, &open(&shaped(100, 100)), 0),
        Ok(())
    );
    // Neither do hosts that were removed (the new list is the first 60 entries) ...
    let old = open(&shaped(100, 100));
    let mut shrunk = shaped(100, 100);
    shrunk.truncate(60);
    assert_eq!(guard_flag_loss(&old, &open(&shrunk), 0), Ok(()));
    // ... or hosts that are new, whatever their flag.
    let mut grown = shaped(100, 100);
    grown.extend((0..50).map(|number| entry(&format!("new-{number:02}.test"), false)));
    assert_eq!(guard_flag_loss(&old, &open(&grown), 0), Ok(()));
    // Two lists with no host in common have nothing to lose.
    let a = open(&[entry("a.test", true)]);
    let b = open(&[entry("b.test", false)]);
    assert_eq!(guard_flag_loss(&a, &b, 0), Ok(()));
}

#[test]
fn the_flag_loss_share_is_of_the_shared_hosts_not_of_all_of_them() {
    // 100 hosts shared and 100 removed: one lost flag is 1 % of the shared hosts, which passes at
    // a limit of 1 %; two are 2 % of them (and only 1 % of the old list's 200), which fails.
    let old = open(&shaped(200, 200));
    let mut new = shaped(100, 100);
    new.last_mut().unwrap().include_subdomains = false;
    assert_eq!(guard_flag_loss(&old, &open(&new), 1), Ok(()));
    new.iter_mut().rev().nth(1).unwrap().include_subdomains = false;
    let error = guard_flag_loss(&old, &open(&new), 1).unwrap_err();
    assert!(error.contains("2 of the 100 hosts"), "{error}");
}
