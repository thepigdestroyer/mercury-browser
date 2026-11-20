// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The Chromium JSON reader of `hsts-preload`: comments, trailing commas, base64, the selection
//! rules (D3 to D5) and the host rule (D4). The fixtures are small synthetic documents shaped like
//! `net/http/transport_security_state_static.json`; nothing here reads a file or the network.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use mercury_devtools::hsts::Entry;
use mercury_devtools::hsts::source::{
    Parsed, decode_base64, parse_chromium, strip_comments, valid_host,
};
use serde_json::{Value, json};

fn parse(json: &str) -> Parsed {
    match parse_chromium(json.as_bytes()) {
        Ok(parsed) => parsed,
        Err(error) => panic!("expected the list to parse, got: {error}"),
    }
}

fn error(json: &str) -> String {
    match parse_chromium(json.as_bytes()) {
        Ok(parsed) => panic!("expected an error, got {parsed:?}"),
        Err(error) => error,
    }
}

/// `{"entries": [<objects>]}`.
fn list(objects: &str) -> String {
    format!(r#"{{"entries": [{objects}]}}"#)
}

/// The same document with the given names, all `force-https`.
fn force_https_list(names: &[&str]) -> String {
    let objects: Vec<Value> = names
        .iter()
        .map(|name| json!({"name": name, "mode": "force-https"}))
        .collect();
    json!({ "entries": objects }).to_string()
}

fn pairs(parsed: &Parsed) -> Vec<(&str, bool)> {
    parsed
        .entries
        .iter()
        .map(|entry| (entry.host.as_str(), entry.include_subdomains))
        .collect()
}

/// A document shaped like the real file: licence comments, pin sets, trailing commas and comments
/// between entries.
const CHROMIUM_LIKE: &str = r#"// Copyright 2015 The Chromium Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

// This file is the HSTS preload list and the pin sets.
{
  "pinsets": [
    {
      "name": "google",
      "static_spki_hashes": [
        "GoogleBackup2048",
        "GoogleG2"
      ],
      "bad_static_spki_hashes": [
        "Bad1",
      ],
      "report_uri": "http://clients3.google.com/cert_upload_json"
    },
  ],

  "entries": [
    // Google.
    { "name": "google.com", "policy": "google", "mode": "force-https", "include_subdomains": true, "pins": "google" },
    { "name": "pin-only.example", "policy": "custom", "pins": "tor" }, // trailing comment
    { "name": "pinningtest.appspot.com", "policy": "test", "mode": "force-https", "include_subdomains": true, },
    /* a block
       comment */
    { "name": "github.com", "mode": "force-https", "include_subdomains": true },
    { "name": "dev", "mode": "force-https" },
  ]
}
"#;

// ---------------------------------------------------------------------------------------------
// comments and commas

#[test]
fn line_and_trailing_comments_are_ignored() {
    let parsed = parse(
        "// header\n{\"entries\": [ // after the bracket\n  {\"name\": \"a.com\", \"mode\": \"force-https\"} // trailing\n]} // end",
    );
    assert_eq!(pairs(&parsed), [("a.com", false)]);
}

#[test]
fn block_comments_are_ignored_even_across_lines() {
    let parsed = parse(
        "/* licence\n   text */ {\"entries\": [/* x */ {\"name\": /* y */ \"a.com\", \"mode\": \"force-https\" /* z\n */}]}",
    );
    assert_eq!(pairs(&parsed), [("a.com", false)]);
}

#[test]
fn comment_markers_inside_strings_stay() {
    let text =
        r#"{"a": "http://x", "b": "/* not a comment */", "c": "q\"//z", "d": ["a,]", "b,}"]}"#;
    assert_eq!(strip_comments(text), text);
    let value: Value = serde_json::from_str(&strip_comments(text)).expect("still JSON");
    assert_eq!(value["a"], "http://x");
    assert_eq!(value["c"], "q\"//z");

    // The same inside an entry: a `pins` value with a URL is one string, and the key is known.
    let parsed = parse(&list(
        r#"{"name": "a.com", "mode": "force-https", "pins": "http://x // y"}"#,
    ));
    assert_eq!(pairs(&parsed), [("a.com", false)]);
    assert!(parsed.report.unknown_keys.is_empty());
}

#[test]
fn an_escaped_backslash_ends_the_string_before_a_comment() {
    // `"a\\"` is the string `a\`; the `//` after it is a comment, not string content.
    let stripped = strip_comments(r#"["a\\"] // c"#);
    assert_eq!(stripped, format!(r#"["a\\"] {}"#, " ".repeat(4)));
}

#[test]
fn trailing_commas_in_lists_and_objects_are_dropped() {
    let parsed = parse(
        r#"{"entries": [
            {"name": "a.com", "mode": "force-https", "include_subdomains": true,},
            {"name": "b.com", "mode": "force-https",},
        ],}"#,
    );
    assert_eq!(pairs(&parsed), [("a.com", true), ("b.com", false)]);
}

#[test]
fn a_trailing_comma_before_a_comment_and_a_bracket_is_dropped() {
    let text = "{\"entries\": [{\"name\": \"a.com\", \"mode\": \"force-https\", // last key\n}, /* last entry */ ]}";
    assert_eq!(pairs(&parse(text)), [("a.com", false)]);
    // A comma that is followed by a value is not trailing.
    assert_eq!(strip_comments(r#"[1, "x"]"#), r#"[1, "x"]"#);
    assert_eq!(strip_comments("[1, 2, ]"), "[1, 2  ]");
}

#[test]
fn a_doubled_comma_is_still_an_error() {
    // Only a comma that follows a value is dropped.
    for text in ["[,]", "[1,,]", r#"{"a":,}"#, "{,}"] {
        assert_eq!(strip_comments(text), text);
    }
    for text in [
        r#"{"entries": [,]}"#,
        r#"{"entries": [{"name": "a.com", "mode": "force-https"},,]}"#,
    ] {
        let message = error(text);
        assert!(message.contains("not valid JSON"), "{message}");
        assert!(message.contains("line 1 column"), "{message}");
    }
}

#[test]
fn crlf_input_parses() {
    let crlf = CHROMIUM_LIKE.replace('\n', "\r\n");
    let parsed = parse(&crlf);
    assert_eq!(parsed.report.objects, 5);
    assert_eq!(parsed.entries.len(), 4);
    // A line comment at the very end, without a line end, is fine too.
    let parsed = parse("{\"entries\": []} // done");
    assert_eq!(parsed.report.objects, 0);
}

#[test]
fn stripping_keeps_every_line_and_column() {
    let text = "// é ü 日本\n/* é\n日 */ [1, // c\n\"x // é\" , ]\r\n/**/ // end";
    let stripped = strip_comments(text);
    assert_eq!(stripped.len(), text.len(), "one blank per byte");
    let line_ends = |s: &str| -> Vec<usize> {
        s.bytes()
            .enumerate()
            .filter(|&(_, byte)| byte == b'\n' || byte == b'\r')
            .map(|(at, _)| at)
            .collect()
    };
    assert_eq!(line_ends(&stripped), line_ends(text));
    assert!(stripped.contains("\"x // é\""), "{stripped:?}");
    assert!(!stripped.contains("日本"), "{stripped:?}");
}

#[test]
fn a_json_error_points_into_the_original_text() {
    // Truncated after the comma on line 5: serde says where the text ends, and the comment lines
    // above it count.
    let text = "// one\n// two\n{\n  \"entries\": [\n    {\"name\": \"b.com\",";
    let message = error(text);
    assert!(message.contains("not valid JSON"), "{message}");
    assert!(message.contains("line 5 column 21"), "{message}");

    // A block comment in front of the error does not move its column: the message is the one
    // the same text gives when the comment is spaces.
    let with_comment = error(r#"{"entries": /* long comment */ oops}"#);
    let with_spaces = error(&format!(r#"{{"entries": {} oops}}"#, " ".repeat(18)));
    assert_eq!(with_comment, with_spaces);
    assert!(with_comment.contains("line 1 column"), "{with_comment}");
}

#[test]
fn unterminated_comments_and_strings_are_errors_with_a_position() {
    let message = error(r#"{"entries": [] /* never closed"#);
    assert!(message.contains("block comment"), "{message}");
    assert!(message.contains("line 1 column 16"), "{message}");

    let message = error(r#"{"entries": [{"name": "a.com"#);
    assert!(message.contains("string"), "{message}");
    assert!(message.contains("line 1 column 23"), "{message}");

    let message = error("{\n\"entries\": [],\n/* open\n\n");
    assert!(message.contains("line 3 column 1"), "{message}");

    // `/*/` does not close itself.
    let message = error(r#"{"entries": [] /*/ x"#);
    assert!(message.contains("block comment"), "{message}");
}

// ---------------------------------------------------------------------------------------------
// base64

#[test]
fn base64_round_trips_with_every_padding() {
    for length in 0..=9usize {
        let data: Vec<u8> = (0..length as u8)
            .map(|byte| byte.wrapping_mul(37))
            .collect();
        let encoded = STANDARD.encode(&data);
        assert_eq!(decode_base64(encoded.as_bytes()).as_deref(), Ok(&data[..]));
    }
    // Padding of one and two characters really occurs in that range.
    assert!(STANDARD.encode([1u8; 4]).ends_with("=="));
    assert!(STANDARD.encode([1u8; 5]).ends_with('='));
}

#[test]
fn base64_ignores_line_breaks_and_trailing_newlines() {
    let data = CHROMIUM_LIKE.as_bytes();
    let encoded = STANDARD.encode(data);
    for separator in ["\n", "\r\n"] {
        let wrapped: Vec<&str> = encoded
            .as_bytes()
            .chunks(64)
            .map(|chunk| std::str::from_utf8(chunk).expect("ASCII"))
            .collect();
        let text = format!("{}{separator}", wrapped.join(separator));
        assert_eq!(decode_base64(text.as_bytes()).as_deref(), Ok(data));
    }
    assert_eq!(
        decode_base64(b"  QU\tJD\n").as_deref(),
        Ok(&b"ABC"[..]),
        "white space anywhere"
    );
}

#[test]
fn bad_base64_says_so() {
    for bad in [
        &b"!!!!"[..],
        b"QUJD*",
        b"Pz8_",     // URL-safe alphabet is not the standard one
        b"QQ==QQ==", // padding in the middle
        b"QQ",       // padding is required: a cut-off download must not decode
        b"QUJDR",    // length that is not a multiple of four
    ] {
        let message = decode_base64(bad).expect_err("not base64");
        assert!(message.contains("not base64"), "{message}");
    }
}

#[test]
fn a_base64_download_decodes_and_parses() {
    let wrapped = STANDARD
        .encode(CHROMIUM_LIKE)
        .as_bytes()
        .chunks(76)
        .map(|chunk| std::str::from_utf8(chunk).expect("ASCII"))
        .collect::<Vec<_>>()
        .join("\n");
    let decoded = decode_base64(wrapped.as_bytes()).expect("decode");
    let parsed = parse_chromium(&decoded).expect("parse");
    assert_eq!(
        pairs(&parsed),
        [
            ("dev", false),
            ("github.com", true),
            ("google.com", true),
            ("pinningtest.appspot.com", true),
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// selection (D3 to D5)

#[test]
fn only_force_https_entries_are_kept() {
    let parsed = parse(&list(
        r#"{"name": "a.com", "mode": "force-https"},
           {"name": "b.com"},
           {"name": "c.com", "mode": "other"},
           {"name": "d.com", "mode": ""}"#,
    ));
    assert_eq!(pairs(&parsed), [("a.com", false)]);
    assert_eq!(parsed.report.objects, 4);
    assert_eq!(parsed.report.force_https, 1);
    assert_eq!(parsed.report.skipped, 3);
}

#[test]
fn a_mode_that_is_not_a_string_is_an_error_naming_the_object() {
    for mode in ["true", "1", "null", "[]", r#"{"x": 1}"#] {
        let message = error(&list(&format!(
            r#"{{"name": "a.com", "mode": "force-https"}}, {{"name": "b.com", "mode": "force-https"}}, {{"name": "c.com", "mode": {mode}}}"#
        )));
        assert!(message.contains("entries[2]"), "{message}");
        assert!(message.contains("\"c.com\""), "{message}");
        assert!(message.contains("mode"), "{message}");
    }
}

#[test]
fn include_subdomains_absent_true_and_false() {
    let parsed = parse(&list(
        r#"{"name": "absent.com", "mode": "force-https"},
           {"name": "yes.com", "mode": "force-https", "include_subdomains": true},
           {"name": "no.com", "mode": "force-https", "include_subdomains": false}"#,
    ));
    assert_eq!(
        pairs(&parsed),
        [("absent.com", false), ("no.com", false), ("yes.com", true)]
    );
}

#[test]
fn include_subdomains_that_is_not_a_boolean_is_an_error_naming_the_object() {
    for value in [r#""true""#, "1", "0", "null", "[true]"] {
        let message = error(&list(&format!(
            r#"{{"name": "ok.com", "mode": "force-https"}}, {{"name": "odd.com", "mode": "force-https", "include_subdomains": {value}}}"#
        )));
        assert!(message.contains("entries[1]"), "{message}");
        assert!(message.contains("\"odd.com\""), "{message}");
        assert!(message.contains("include_subdomains"), "{message}");
    }
}

#[test]
fn the_policy_is_ignored_and_the_test_policy_is_kept() {
    let parsed = parse(&list(
        r#"{"name": "pinningtest.appspot.com", "policy": "test", "mode": "force-https", "include_subdomains": true},
           {"name": "g.com", "policy": "google", "mode": "force-https"},
           {"name": "c.com", "policy": "custom", "mode": "force-https"},
           {"name": "n.com", "mode": "force-https"}"#,
    ));
    assert_eq!(
        pairs(&parsed),
        [
            ("c.com", false),
            ("g.com", false),
            ("n.com", false),
            ("pinningtest.appspot.com", true),
        ]
    );
    assert!(parsed.report.unknown_keys.is_empty());
}

#[test]
fn pin_only_objects_are_skipped_before_the_host_rules() {
    let parsed = parse(&list(
        r#"{"name": "bad_name*", "pins": "x"},
           {"name": "under_score.example", "policy": "custom", "include_subdomains": "not a flag"},
           {"name": "wild*.example", "mode": "other"},
           {"name": "ok.com", "mode": "force-https"}"#,
    ));
    assert_eq!(pairs(&parsed), [("ok.com", false)]);
    assert_eq!((parsed.report.objects, parsed.report.skipped), (4, 3));
    // The same name on a force-https entry is refused.
    let message = error(&force_https_list(&["ok.com", "bad_name*"]));
    assert!(message.contains("entries[1]"), "{message}");
}

#[test]
fn duplicates_with_equal_flags_are_merged_and_counted() {
    let parsed = parse(&list(
        r#"{"name": "a.com", "mode": "force-https", "include_subdomains": true},
           {"name": "a.com", "mode": "force-https", "include_subdomains": true},
           {"name": "b.com", "mode": "force-https"},
           {"name": "b.com", "mode": "force-https"}"#,
    ));
    assert_eq!(pairs(&parsed), [("a.com", true), ("b.com", false)]);
    assert_eq!(parsed.report.merged_duplicates, 2);
    assert_eq!(parsed.report.force_https, 4);
}

#[test]
fn duplicates_with_different_flags_keep_include_subdomains() {
    for flags in [[false, true], [true, false]] {
        let objects: Vec<String> = flags
            .iter()
            .map(|flag| {
                format!(
                    r#"{{"name": "a.com", "mode": "force-https", "include_subdomains": {flag}}}"#
                )
            })
            .collect();
        let parsed = parse(&list(&objects.join(",")));
        assert_eq!(pairs(&parsed), [("a.com", true)], "order {flags:?}");
        assert_eq!(parsed.report.merged_duplicates, 1);
    }
    // Three listings, the middle one absent: still one entry, two merges.
    let parsed = parse(&list(
        r#"{"name": "a.com", "mode": "force-https"},
           {"name": "a.com", "mode": "force-https", "include_subdomains": true},
           {"name": "a.com", "mode": "force-https", "include_subdomains": false}"#,
    ));
    assert_eq!(pairs(&parsed), [("a.com", true)]);
    assert_eq!(parsed.report.merged_duplicates, 2);
}

#[test]
fn upper_case_names_are_folded_and_merge_with_their_lower_case_twin() {
    let parsed = parse(&force_https_list(&[
        "Example.COM",
        "GitHub.com",
        "example.com",
    ]));
    assert_eq!(
        pairs(&parsed),
        [("example.com", false), ("github.com", false)]
    );
    assert_eq!(parsed.report.merged_duplicates, 1);
}

#[test]
fn unknown_keys_are_counted_not_refused() {
    let parsed = parse(&list(
        r#"{"name": "a.com", "mode": "force-https", "foo": 1, "bar": [1, 2]},
           {"name": "b.com", "foo": "x", "expect_staple_report_uri": "u", "include_subdomains_for_pinning": true},
           {"name": "c.com", "mode": "force-https", "expect_ct": true, "expect_ct_report_uri": "u", "expect_staple": true, "pins": "p", "policy": "q"}"#,
    ));
    let counted: Vec<(&str, usize)> = parsed
        .report
        .unknown_keys
        .iter()
        .map(|(key, count)| (key.as_str(), *count))
        .collect();
    assert_eq!(counted, [("bar", 1), ("foo", 2)]);
    assert_eq!(pairs(&parsed), [("a.com", false), ("c.com", false)]);
}

#[test]
fn output_is_sorted_and_unique_and_the_report_adds_up() {
    let parsed = parse(&list(
        r#"{"name": "b.com", "mode": "force-https"},
           {"name": "dev", "mode": "force-https", "include_subdomains": true},
           {"name": "a.com", "mode": "force-https"},
           {"name": "pin.example", "pins": "x"},
           {"name": "a-b.com", "mode": "force-https"},
           {"name": "0.com", "mode": "force-https"},
           {"name": "A.com", "mode": "force-https", "include_subdomains": true},
           {"name": "other.example", "mode": "other"}"#,
    ));
    // Byte order: '-' sorts before '.', digits before letters.
    let hosts: Vec<&str> = parsed.entries.iter().map(|e| e.host.as_str()).collect();
    assert_eq!(hosts, ["0.com", "a-b.com", "a.com", "b.com", "dev"]);
    assert!(parsed.entries.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        parsed.entries[2],
        Entry {
            host: "a.com".into(),
            include_subdomains: true
        }
    );

    let report = &parsed.report;
    assert_eq!(report.objects, 8);
    assert_eq!(report.force_https, 6);
    assert_eq!(report.skipped, 2);
    assert_eq!(report.merged_duplicates, 1);
    assert_eq!(report.objects, report.force_https + report.skipped);
    assert_eq!(
        parsed.entries.len(),
        report.force_https - report.merged_duplicates
    );
}

#[test]
fn a_chromium_shaped_file_reads_as_expected() {
    let parsed = parse(CHROMIUM_LIKE);
    assert_eq!(
        pairs(&parsed),
        [
            ("dev", false),
            ("github.com", true),
            ("google.com", true),
            ("pinningtest.appspot.com", true),
        ]
    );
    let report = &parsed.report;
    assert_eq!(
        (report.objects, report.force_https, report.skipped),
        (5, 4, 1)
    );
    assert_eq!(report.merged_duplicates, 0);
    assert!(report.unknown_keys.is_empty(), "{:?}", report.unknown_keys);
}

#[test]
fn a_byte_order_mark_is_ignored() {
    let parsed = parse("\u{feff}{\"entries\": []}");
    assert_eq!(parsed.report.objects, 0);
    assert!(parsed.entries.is_empty());
}

// ---------------------------------------------------------------------------------------------
// the host rule (D4)

#[test]
fn valid_hosts_are_lower_cased_and_returned() {
    assert_eq!(valid_host("0--1.de").as_deref(), Ok("0--1.de"));
    assert_eq!(valid_host("dev").as_deref(), Ok("dev"));
    assert_eq!(valid_host("Example.COM").as_deref(), Ok("example.com"));
    assert_eq!(
        valid_host("xn--80ak6aa92e.com").as_deref(),
        Ok("xn--80ak6aa92e.com")
    );
    assert_eq!(
        valid_host("-a-.b-").as_deref(),
        Ok("-a-.b-"),
        "hyphens are not restricted"
    );
    let label_63 = format!("{}.com", "a".repeat(63));
    assert_eq!(valid_host(&label_63), Ok(label_63.clone()));
    // 63 + 63 + 63 + 61 bytes of label and three dots: exactly 253.
    let name_253 = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    assert_eq!(name_253.len(), 253);
    assert_eq!(valid_host(&name_253), Ok(name_253.clone()));
}

#[test]
fn each_host_rule_refusal_names_the_entry() {
    let label_64 = format!("{}.com", "a".repeat(64));
    let name_254 = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(62)
    );
    assert_eq!(name_254.len(), 254);
    let cases: [(&str, &str); 11] = [
        ("exämple.com", "non-ASCII"),
        ("日本.jp", "non-ASCII"),
        ("my_host.com", "'_'"),
        ("*.example.com", "'*'"),
        ("a..b.com", "empty label"),
        (".example.com", "starts with a dot"),
        ("example.com.", "ends with a dot"),
        ("", "is empty"),
        ("exa mple.com", "' '"),
        (&label_64, "label of 64 bytes"),
        (&name_254, "254 bytes"),
    ];
    for (name, reason) in cases {
        let direct = valid_host(name).expect_err(name);
        assert!(direct.contains(reason), "{name:?}: {direct}");
        if name.len() < 100 {
            assert!(direct.contains(name), "the entry is named: {direct}");
        }
        // Through the list: the object's index comes first, and nothing is written.
        let message = error(&force_https_list(&["ok.com", name]));
        assert!(message.starts_with("entries[1]:"), "{message}");
        assert!(message.contains(reason), "{name:?}: {message}");
    }
}

#[test]
fn a_very_long_name_is_cut_in_the_message() {
    let name = format!("{}.com", "x".repeat(300));
    let message = valid_host(&name).expect_err("too long");
    assert!(message.len() < 300, "{message}");
    assert!(message.contains("..."), "{message}");
}

// ---------------------------------------------------------------------------------------------
// shapes that are not the list

#[test]
fn a_list_without_entries_is_an_error_that_names_what_it_has() {
    let message = error(r#"{"pinsets": [], "other": 1}"#);
    assert!(message.contains("no \"entries\""), "{message}");
    assert!(message.contains("pinsets"), "{message}");
    assert!(error("{}").contains("none"));
}

#[test]
fn entries_that_is_not_a_list_is_an_error() {
    for (text, kind) in [
        (r#"{"entries": {}}"#, "an object"),
        (r#"{"entries": "a.com"}"#, "a string"),
        (r#"{"entries": null}"#, "null"),
    ] {
        let message = error(text);
        assert!(message.contains("not a list"), "{message}");
        assert!(message.contains(kind), "{message}");
    }
}

#[test]
fn the_top_level_must_be_an_object() {
    let message = error("[]");
    assert!(message.contains("a list, not an object"), "{message}");
    assert!(error("42").contains("a number"));
}

#[test]
fn an_entry_that_is_not_an_object_or_has_no_string_name_is_an_error() {
    let message = error(&list(
        r#"{"name": "a.com", "mode": "force-https"}, "b.com""#,
    ));
    assert!(message.contains("entries[1] is a string"), "{message}");

    let message = error(&list(r#"{"name": 5, "mode": "force-https"}"#));
    assert!(message.contains("entries[0]"), "{message}");
    assert!(message.contains("a number"), "{message}");

    let message = error(&list(r#"{"name": null}"#));
    assert!(message.contains("null"), "{message}");

    let message = error(&list(r#"{"mode": "force-https"}"#));
    assert!(message.contains("entries[0] has no \"name\""), "{message}");
}

#[test]
fn invalid_utf8_is_an_error() {
    let mut bytes = br#"{"entries": ["#.to_vec();
    bytes.extend_from_slice(&[0xff, 0xfe]);
    bytes.extend_from_slice(b"]}");
    let message = parse_chromium(&bytes).expect_err("not UTF-8");
    assert!(message.contains("UTF-8"), "{message}");
    assert!(message.contains("offset 13"), "{message}");
}

#[test]
fn truncated_json_reports_line_and_column() {
    for text in [
        "{",
        "{\n  \"entries\": [\n    {\"name\": \"a.com\", \"mode\": \"force-https\"},\n    {\"name\"",
        r#"{"entries": [{"name": "a.com", "mode": "force-https"}"#,
        "",
    ] {
        let message = error(text);
        assert!(message.contains("not valid JSON"), "{message}");
        assert!(message.contains("line "), "{message}");
        assert!(message.contains("column "), "{message}");
    }
}
