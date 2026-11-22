// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Firefox update manifest (MDN "Updates"): parsing, filtering and limits.

use mercury_extensions::id::ExtensionId;
use mercury_extensions::update::{
    AvailableUpdate, UpdateError, UpdateHash, UpdateLimits, parse_firefox_update_manifest,
};

const SHA256: &str = "sha256:b290f77cde930116b290f77cde930116b290f77cde930116b290f77cde930116";
const SHA512: &str = "sha512:b290f77cde930116b290f77cde930116b290f77cde930116b290f77cde930116b290f77cde930116b290f77cde930116b290f77cde930116b290f77cde930116";

fn addon() -> ExtensionId {
    ExtensionId::parse("tool@example.org").unwrap()
}

fn parse(json: &str) -> Result<Vec<AvailableUpdate>, UpdateError> {
    parse_firefox_update_manifest(json.as_bytes(), &addon(), &UpdateLimits::default())
}

fn doc(updates: &str) -> String {
    format!(r#"{{"addons":{{"tool@example.org":{{"updates":[{updates}]}}}}}}"#)
}

#[test]
fn the_documented_shape_parses() {
    let json = doc(&format!(
        r#"{{"version":"2.0","update_link":"https://example.org/tool-2.0.xpi","update_hash":"{SHA256}",
             "update_info_url":"https://example.org/notes","applications":{{"gecko":{{"strict_min_version":"68.0","strict_max_version":"120.*"}}}}}},
           {{"version":"1.5","update_link":"https://example.org/tool-1.5.xpi"}}"#
    ));
    let updates = parse(&json).unwrap();
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].version, "2.0");
    assert_eq!(
        updates[0].update_link.as_str(),
        "https://example.org/tool-2.0.xpi"
    );
    assert!(matches!(
        updates[0].update_hash,
        Some(UpdateHash::Sha256(_))
    ));
    assert_eq!(updates[0].strict_min_version.as_deref(), Some("68.0"));
    assert_eq!(updates[0].strict_max_version.as_deref(), Some("120.*"));
    assert_eq!(updates[1].update_hash, None);
    assert_eq!(updates[1].strict_min_version, None);
}

#[test]
fn sha512_is_accepted() {
    let updates = parse(&doc(&format!(
        r#"{{"version":"2.0","update_link":"https://e.org/x.xpi","update_hash":"{SHA512}"}}"#
    )))
    .unwrap();
    assert!(matches!(
        updates[0].update_hash,
        Some(UpdateHash::Sha512(_))
    ));
}

#[test]
fn another_addons_entry_is_not_ours() {
    let json = r#"{"addons":{"other@example.org":{"updates":[{"version":"9","update_link":"https://e.org/x.xpi"}]}}}"#;
    assert_eq!(parse(json).unwrap(), vec![]);
}

#[test]
fn unusable_entries_are_left_out() {
    let weak = [
        r#"{"version":"2.0","update_link":"https://e.org/a.xpi","update_hash":"sha1:da39a3ee5e6b4b0d3255bfef95601890afd80709"}"#,
        r#"{"version":"2.0","update_link":"https://e.org/a.xpi","update_hash":"md5:d41d8cd98f00b204e9800998ecf8427e"}"#,
        r#"{"version":"2.0","update_link":"https://e.org/a.xpi","update_hash":"sha256:abc"}"#,
        r#"{"version":"2.0","update_link":"https://e.org/a.xpi","update_hash":"nocolon"}"#,
        r#"{"version":"2.0","update_link":"https://e.org/a.xpi","update_hash":5}"#,
        r#"{"version":"bad version","update_link":"https://e.org/a.xpi"}"#,
        r#"{"version":5,"update_link":"https://e.org/a.xpi"}"#,
        r#"{"update_link":"https://e.org/a.xpi"}"#,
        r#"{"version":"2.0"}"#,
        r#"{"version":"2.0","update_link":"javascript:alert(1)"}"#,
        r#"{"version":"2.0","update_link":"ftp://e.org/a.xpi"}"#,
        r#"{"version":"2.0","update_link":"file:///etc/passwd"}"#,
        r#"{"version":"2.0","update_link":"not a url"}"#,
        r#"5"#,
        r#"null"#,
    ];
    for entry in weak {
        assert_eq!(parse(&doc(entry)).unwrap(), vec![], "{entry}");
    }
    // One good entry among bad ones survives.
    let mixed = doc(&format!(
        "{},{{\"version\":\"3.0\",\"update_link\":\"https://e.org/ok.xpi\"}}",
        weak[0]
    ));
    assert_eq!(parse(&mixed).unwrap().len(), 1);
}

#[test]
fn bad_gecko_versions_are_dropped_but_the_entry_stays() {
    let updates = parse(&doc(
        r#"{"version":"2.0","update_link":"https://e.org/a.xpi","applications":{"gecko":{"strict_min_version":"not valid!","strict_max_version":5}}}"#,
    ))
    .unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].strict_min_version, None);
    assert_eq!(updates[0].strict_max_version, None);
}

#[test]
fn the_wrong_shapes_are_errors() {
    for (json, expected) in [
        ("", UpdateError::NotJson),
        ("{", UpdateError::NotJson),
        ("[]", UpdateError::BadShape),
        ("5", UpdateError::BadShape),
        (r#"{}"#, UpdateError::BadShape),
        (r#"{"addons":[]}"#, UpdateError::BadShape),
        (
            r#"{"addons":{"tool@example.org":{}}}"#,
            UpdateError::BadShape,
        ),
        (
            r#"{"addons":{"tool@example.org":{"updates":{}}}}"#,
            UpdateError::BadShape,
        ),
        (
            r#"{"addons":{"tool@example.org":5}}"#,
            UpdateError::BadShape,
        ),
    ] {
        assert_eq!(parse(json), Err(expected), "{json}");
    }
}

#[test]
fn the_update_count_is_capped_at_the_edge() {
    let limits = UpdateLimits::default();
    let entry = r#"{"version":"2.0","update_link":"https://e.org/a.xpi"}"#;
    let at = vec![entry; limits.max_updates].join(",");
    assert_eq!(
        parse_firefox_update_manifest(doc(&at).as_bytes(), &addon(), &limits)
            .unwrap()
            .len(),
        limits.max_updates
    );
    let over = format!("{at},{entry}");
    assert_eq!(
        parse_firefox_update_manifest(doc(&over).as_bytes(), &addon(), &limits),
        Err(UpdateError::TooManyUpdates)
    );
}

#[test]
fn the_size_cap_is_enforced_at_the_edge() {
    let limits = UpdateLimits::default();
    let base = doc("");
    let at = format!(
        "{base}{}",
        " ".repeat(limits.max_response_bytes - base.len())
    );
    assert!(parse_firefox_update_manifest(at.as_bytes(), &addon(), &limits).is_ok());
    let over = format!("{at} ");
    assert_eq!(
        parse_firefox_update_manifest(over.as_bytes(), &addon(), &limits),
        Err(UpdateError::TooLarge)
    );
}

#[test]
fn deep_nesting_is_refused_before_parsing() {
    let limits = UpdateLimits::default();
    let deep = format!(
        "{}{}",
        "[".repeat(limits.max_json_depth + 1),
        "]".repeat(limits.max_json_depth + 1)
    );
    assert_eq!(
        parse_firefox_update_manifest(deep.as_bytes(), &addon(), &limits),
        Err(UpdateError::TooDeep)
    );
    let bomb = "[".repeat(500_000);
    assert_eq!(
        parse_firefox_update_manifest(bomb.as_bytes(), &addon(), &limits),
        Err(UpdateError::TooDeep)
    );
}

#[test]
fn hash_parsing_and_verification() {
    assert!(UpdateHash::parse_prefixed(SHA256).is_some());
    assert!(
        UpdateHash::parse_prefixed(&SHA256.to_uppercase().replace("SHA256", "sha256")).is_some()
    );
    assert!(
        UpdateHash::parse_prefixed(
            "SHA256:b290f77cde930116b290f77cde930116b290f77cde930116b290f77cde930116"
        )
        .is_some()
    );
    for bad in [
        "",
        "sha256:",
        "sha256",
        "sha1:da39a3ee5e6b4b0d3255bfef95601890afd80709",
        "md5:d41d8cd98f00b204e9800998ecf8427e",
        "sha256:zz",
        "sha512:abcd",
    ] {
        assert!(UpdateHash::parse_prefixed(bad).is_none(), "{bad}");
    }
    // sha256("abc")
    let abc = UpdateHash::parse_prefixed(
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    )
    .unwrap();
    assert!(abc.verify(b"abc"));
    assert!(!abc.verify(b"abd"));
    assert!(!abc.verify(b""));
    // sha512("abc")
    let abc512 = UpdateHash::parse_prefixed(
        "sha512:ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
    )
    .unwrap();
    assert!(abc512.verify(b"abc"));
    assert!(!abc512.verify(b"abcd"));
}

#[test]
fn mutated_manifests_never_panic() {
    let seed = doc(&format!(
        r#"{{"version":"2.0","update_link":"https://example.org/tool-2.0.xpi","update_hash":"{SHA256}","applications":{{"gecko":{{"strict_min_version":"68.0"}}}}}}"#
    ));
    let mut state = 0x0F0F_F0F0_1357_9BDFu64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let junk: [&[u8]; 10] = [
        b"\"", b"{", b"}", b"[", b"]", b",", b":", b"\\u0000", b"\xff", b"null",
    ];
    for _ in 0..5000 {
        let mut bytes = seed.clone().into_bytes();
        for _ in 0..(1 + next() % 4) {
            let at = (next() as usize) % bytes.len();
            match next() % 4 {
                0 => {
                    bytes.remove(at);
                }
                1 => {
                    let j = junk[(next() as usize) % junk.len()];
                    bytes.splice(at..at, j.iter().copied());
                }
                2 => bytes[at] = (next() & 0xff) as u8,
                _ => bytes.truncate(at.max(1)),
            }
            if bytes.is_empty() {
                bytes.push(b'{');
            }
        }
        let _ = parse_firefox_update_manifest(&bytes, &addon(), &UpdateLimits::default());
    }
}
