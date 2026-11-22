// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A deterministic mutation loop over `Manifest::parse` (xorshift, no `rand`). The only
//! assertion is that nothing panics and that an accepted manifest stays internally consistent.

use mercury_extensions::manifest::{Manifest, ManifestLimits};

const SEEDS: &[&[u8]] = &[
    include_bytes!("fixtures/manifest/chrome_mv3.json"),
    include_bytes!("fixtures/manifest/chrome_mv2.json"),
    include_bytes!("fixtures/manifest/firefox_mv2.json"),
    include_bytes!("fixtures/manifest/firefox_mv3.json"),
    include_bytes!("fixtures/manifest/minimal.json"),
];

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn mutate(seed: &[u8], state: &mut u64) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    let junk: &[&[u8]] = &[
        b"\"",
        b"{",
        b"}",
        b"[",
        b"]",
        b",",
        b":",
        b"\\",
        b"\\u0000",
        b"\\ud800",
        b"null",
        b"-1",
        b"1e999",
        b"\"../\"",
        b"\xff",
        b"\xef\xbb\xbf",
        b"*",
        b"<all_urls>",
    ];
    for _ in 0..(1 + xorshift(state) % 5) {
        if bytes.is_empty() {
            bytes.push(b'{');
            continue;
        }
        let at = (xorshift(state) as usize) % bytes.len();
        match xorshift(state) % 5 {
            0 => {
                bytes.remove(at);
            }
            1 => {
                let j = junk[(xorshift(state) as usize) % junk.len()];
                bytes.splice(at..at, j.iter().copied());
            }
            2 => bytes[at] = (xorshift(state) & 0xff) as u8,
            3 => bytes.truncate(at),
            _ => {
                let end = (at + (xorshift(state) as usize) % 16).min(bytes.len());
                bytes.drain(at..end);
            }
        }
    }
    bytes
}

#[test]
fn mutated_manifests_never_panic() {
    let limits = ManifestLimits::default();
    let mut state = 0xDEAD_BEEF_CAFE_F00Du64;
    let mut accepted = 0usize;
    for round in 0..6000 {
        let seed = SEEDS[round % SEEDS.len()];
        let bytes = mutate(seed, &mut state);
        if let Ok(m) = Manifest::parse(&bytes, &limits) {
            accepted += 1;
            // Every reference an accepted manifest reports is a clean relative path.
            for r in m.all_file_references() {
                assert!(
                    !r.starts_with('/')
                        && !r.contains("..")
                        && !r.contains('\\')
                        && !r.contains('\0'),
                    "{r:?}"
                );
            }
            let _ = m.declared_id();
        }
    }
    assert!(
        accepted > 0,
        "the loop should accept at least some mutants (whitespace and unknown-key changes)"
    );
}

#[test]
fn every_truncation_of_a_fixture_is_handled() {
    let limits = ManifestLimits::default();
    for seed in SEEDS {
        for len in 0..seed.len() {
            let _ = Manifest::parse(&seed[..len], &limits);
        }
    }
}

#[test]
fn every_single_byte_flip_of_a_small_fixture_is_handled() {
    let limits = ManifestLimits::default();
    let seed = SEEDS[4];
    for at in 0..seed.len() {
        for value in [0u8, b'"', b'\\', b'{', b'}', 0xff] {
            let mut bytes = seed.to_vec();
            bytes[at] = value;
            let _ = Manifest::parse(&bytes, &limits);
        }
    }
}

#[test]
fn hostile_shapes_are_errors_not_panics() {
    let limits = ManifestLimits::default();
    let shapes: &[&str] = &[
        r#"{"manifest_version":3,"name":"T","version":"1","content_scripts":{"matches":[]}}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","content_scripts":[null]}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","background":[]}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","icons":{"-1":"a"}}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","icons":{"99999999999999999999":"a"}}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","permissions":[null,{}]}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","web_accessible_resources":"x"}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","web_accessible_resources":[[]]}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","commands":{"a":{"suggested_key":5}}}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","browser_specific_settings":[]}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","browser_specific_settings":{"gecko":5}}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","declarative_net_request":{"rule_resources":null}}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","content_security_policy":null}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","options_ui":{}}"#,
    ];
    for json in shapes {
        assert!(Manifest::parse(json.as_bytes(), &limits).is_err(), "{json}");
    }
}
