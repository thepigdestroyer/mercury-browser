// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The update2 (Omaha) side of the update model: building requests, `store_update_url`, and
//! parsing `gupdate` responses, including XML that tries to hurt the parser.

use mercury_extensions::id::ExtensionId;
use mercury_extensions::update::{
    BrowserInfo, InstalledExtension, UpdateCheckResult, UpdateError, UpdateErrorKind, UpdateLimits,
    build_update2_request, build_update2_request_with, parse_update2_response, store_update_url,
};
use omaha::protocol::update2::{ResponseMode, decode_query, encode_xml};
use omaha::protocol::{
    AppId, AppResponse, AppStatus, Limits, Operation, Outcome, Pipeline, Response, Sha256,
    UpdateCheckResponse,
};
use url::Url;

const ID_A: &str = "jpignaibiiemhngfjkcpokkamffknabf";
const ID_B: &str = "abcdefghijklmnopabcdefghijklmnop";
const ID_C: &str = "pponmlkjihgfedcbaponmlkjihgfedcb";
const HASH: &str = "b290f77cde930116b290f77cde930116b290f77cde930116b290f77cde930116";

fn id(s: &str) -> ExtensionId {
    ExtensionId::parse(s).unwrap()
}

fn installed(id_text: &str, version: &str, update_url: Option<&str>) -> InstalledExtension {
    InstalledExtension {
        id: id(id_text),
        version: version.to_owned(),
        update_url: update_url.map(|u| Url::parse(u).unwrap()),
    }
}

fn browser() -> BrowserInfo {
    BrowserInfo {
        prod: "mercury".to_owned(),
        prodversion: "0.3.0".to_owned(),
        os: "mac".to_owned(),
        arch: "x64".to_owned(),
        lang: Some("en-US".to_owned()),
    }
}

fn parse(xml: &str) -> Result<Vec<UpdateCheckResult>, UpdateError> {
    parse_update2_response(xml.as_bytes(), &UpdateLimits::default())
}

fn gupdate(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><gupdate xmlns="http://www.google.com/update2/response" protocol="2.0">{body}</gupdate>"#
    )
}

// ---- building requests -------------------------------------------------------------------

#[test]
fn a_request_decodes_with_the_omaha_decoder() {
    let reqs = build_update2_request(
        &[
            installed(ID_A, "1.0", Some("https://updates.example/crx")),
            installed(ID_B, "2.3.4", Some("https://updates.example/crx")),
        ],
        &browser(),
    );
    assert_eq!(reqs.len(), 1);
    let url = &reqs[0].url;
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.path(), "/crx");
    assert_eq!(reqs[0].ids, vec![id(ID_A), id(ID_B)]);
    let decoded = decode_query(url.query().unwrap(), &Limits::default()).unwrap();
    assert_eq!(decoded.mode, ResponseMode::Xml);
    assert_eq!(decoded.request.client.os.as_deref(), Some("mac"));
    assert_eq!(decoded.request.client.arch.as_deref(), Some("x64"));
    assert_eq!(decoded.request.client.updater.as_deref(), Some("mercury"));
    assert_eq!(
        decoded
            .request
            .client
            .prodversion
            .as_ref()
            .unwrap()
            .to_string(),
        "0.3.0"
    );
    assert_eq!(decoded.request.apps.len(), 2);
    assert_eq!(decoded.request.apps[0].appid.as_str(), ID_A);
    assert_eq!(
        decoded.request.apps[0]
            .version
            .as_ref()
            .unwrap()
            .to_string(),
        "1.0"
    );
    assert!(decoded.request.apps[0].updatecheck.is_some());
    assert_eq!(
        decoded.request.apps[1]
            .version
            .as_ref()
            .unwrap()
            .to_string(),
        "2.3.4"
    );
    assert_eq!(decoded.request.apps[0].lang.as_deref(), Some("en-US"));
}

#[test]
fn the_request_asks_for_crx3() {
    let reqs = build_update2_request(
        &[installed(ID_A, "1", Some("https://u.example/crx"))],
        &browser(),
    );
    let decoded = decode_query(reqs[0].url.query().unwrap(), &Limits::default()).unwrap();
    assert!(decoded.request.client.accepts("crx3"));
}

#[test]
fn requests_are_grouped_per_update_url_in_url_order() {
    let reqs = build_update2_request(
        &[
            installed(ID_A, "1", Some("https://b.example/crx")),
            installed(ID_B, "1", Some("https://a.example/crx")),
            installed(ID_C, "1", Some("https://b.example/crx")),
        ],
        &browser(),
    );
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].url.host_str(), Some("a.example"));
    assert_eq!(reqs[0].ids, vec![id(ID_B)]);
    assert_eq!(reqs[1].url.host_str(), Some("b.example"));
    assert_eq!(reqs[1].ids, vec![id(ID_A), id(ID_C)]);
}

#[test]
fn nothing_is_requested_for_extensions_that_cannot_be_checked() {
    let reqs = build_update2_request(
        &[
            installed(ID_A, "1", None),
            installed("tool@example.org", "1.0", Some("https://u.example/x")),
            installed(ID_B, "1", Some("ftp://u.example/x")),
            installed(ID_C, "1", Some("file:///etc/passwd")),
            installed(
                "cccccccccccccccccccccccccccccccc",
                "not.a.version",
                Some("https://u.example/x"),
            ),
            installed(
                "dddddddddddddddddddddddddddddddd",
                "1.2.3.4.5",
                Some("https://u.example/x"),
            ),
        ],
        &browser(),
    );
    assert!(reqs.is_empty(), "{reqs:?}");
}

#[test]
fn an_existing_query_on_the_update_url_is_kept() {
    let reqs = build_update2_request(
        &[installed(
            ID_A,
            "1",
            Some("https://u.example/crx?channel=beta"),
        )],
        &browser(),
    );
    let q = reqs[0].url.query().unwrap();
    assert!(q.starts_with("channel=beta&"), "{q}");
    assert!(decode_query(q, &Limits::default()).is_ok());
}

#[test]
fn long_requests_are_split_under_the_url_cap() {
    let limits = UpdateLimits::default();
    let many: Vec<InstalledExtension> = (0..200)
        .map(|i: u8| {
            // Distinct valid ids: two letters a-p from the index, padded.
            let hi = char::from(b'a' + i / 16);
            let lo = char::from(b'a' + i % 16);
            let s = format!("{hi}{lo}{}", "k".repeat(30));
            installed(&s, "10.20.30.40", Some("https://u.example/crx"))
        })
        .collect();
    let reqs = build_update2_request_with(&many, &browser(), &limits);
    assert!(reqs.len() > 1);
    assert!(
        reqs.iter()
            .all(|r| r.url.as_str().len() <= limits.max_request_url_len)
    );
    let mut ids: Vec<&ExtensionId> = reqs.iter().flat_map(|r| r.ids.iter()).collect();
    let before = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(before, ids.len(), "no id twice");
    let distinct_in: std::collections::BTreeSet<&ExtensionId> =
        many.iter().map(|e| &e.id).collect();
    assert_eq!(ids.len(), distinct_in.len(), "no id lost");
}

#[test]
fn an_extension_that_cannot_fit_a_request_alone_is_left_out() {
    let limits = UpdateLimits {
        max_request_url_len: 60,
        ..UpdateLimits::default()
    };
    let reqs = build_update2_request_with(
        &[installed(ID_A, "1", Some("https://u.example/crx"))],
        &browser(),
        &limits,
    );
    assert!(reqs.is_empty());
}

// ---- store_update_url --------------------------------------------------------------------

#[test]
fn the_store_url_has_chromes_form() {
    let url = store_update_url(&id(ID_A), "120.0.6099.0").unwrap();
    assert_eq!(
        url.as_str(),
        format!(
            "https://clients2.google.com/service/update2/crx?response=redirect&prodversion=120.0.6099.0&acceptformat=crx2,crx3&x=id%3D{ID_A}%26uc"
        )
    );
    let decoded = decode_query(url.query().unwrap(), &Limits::default()).unwrap();
    assert_eq!(decoded.mode, ResponseMode::Redirect);
    assert_eq!(decoded.request.apps[0].appid.as_str(), ID_A);
    assert!(decoded.request.apps[0].updatecheck.is_some());
}

#[test]
fn the_store_url_refuses_bad_input() {
    assert_eq!(
        store_update_url(&id("tool@example.org"), "1.0"),
        Err(UpdateError::InvalidInput)
    );
    let long = "1".repeat(33);
    for bad in ["", "1.0&evil=1", "1.0 ", "abc", long.as_str()] {
        assert_eq!(
            store_update_url(&id(ID_A), bad),
            Err(UpdateError::InvalidInput),
            "{bad:?}"
        );
    }
}

// ---- parsing responses -------------------------------------------------------------------

#[test]
fn a_real_encoded_response_round_trips() {
    let appid = AppId::new(ID_A).unwrap();
    let hash: Sha256 = HASH.parse().unwrap();
    let mut app = AppResponse::new(appid, AppStatus::Ok);
    app.updatecheck = Some(UpdateCheckResponse::new(Outcome::Update {
        nextversion: "2.0.1".parse().unwrap(),
        pipelines: vec![Pipeline {
            id: "full".to_owned(),
            operations: vec![
                Operation::Download {
                    size: 4096,
                    out: hash,
                    urls: vec!["https://dl.example/pkg.crx".to_owned()],
                },
                Operation::Crx3 {
                    input: hash,
                    size: None,
                    path: None,
                    arguments: None,
                },
            ],
        }],
    }));
    let mut quiet = AppResponse::new(AppId::new(ID_B).unwrap(), AppStatus::Ok);
    quiet.updatecheck = Some(UpdateCheckResponse::new(Outcome::NoUpdate));
    let xml = encode_xml(&Response {
        apps: vec![
            app,
            quiet,
            AppResponse::new(AppId::new(ID_C).unwrap(), AppStatus::UnknownApplication),
        ],
        daystart: None,
        server: Some("prod".to_owned()),
    })
    .unwrap();
    let results = parse(&xml).unwrap();
    assert_eq!(results.len(), 3);
    match &results[0] {
        UpdateCheckResult::Available {
            id,
            version,
            codebase,
            sha256,
            size,
        } => {
            assert_eq!(id, ID_A);
            assert_eq!(version, "2.0.1");
            assert_eq!(codebase.as_str(), "https://dl.example/pkg.crx");
            assert_eq!(sha256, hash.as_bytes());
            assert_eq!(*size, Some(4096));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        results[1],
        UpdateCheckResult::UpToDate {
            id: ID_B.to_owned()
        }
    );
    assert!(
        matches!(&results[2], UpdateCheckResult::Error { kind: UpdateErrorKind::Status(s), .. } if s == "error-unknownApplication")
    );
}

#[test]
fn an_update_is_available() {
    let xml = gupdate(&format!(
        r#"<app appid="{ID_A}" status="ok"><updatecheck codebase="https://dl.example/a.crx" version="1.5" hash_sha256="{HASH}" size="123" status="ok"/></app>"#
    ));
    let r = parse(&xml).unwrap();
    assert_eq!(
        r,
        vec![UpdateCheckResult::Available {
            id: ID_A.to_owned(),
            version: "1.5".to_owned(),
            codebase: Url::parse("https://dl.example/a.crx").unwrap(),
            sha256: *HASH.parse::<Sha256>().unwrap().as_bytes(),
            size: Some(123),
        }]
    );
}

#[test]
fn no_update_is_up_to_date() {
    let xml = gupdate(&format!(
        r#"<app appid="{ID_A}" status="ok"><updatecheck status="noupdate"/></app>"#
    ));
    assert_eq!(
        parse(&xml).unwrap(),
        vec![UpdateCheckResult::UpToDate {
            id: ID_A.to_owned()
        }]
    );
}

#[test]
fn server_errors_become_error_results_with_a_clean_status() {
    let xml = gupdate(&format!(
        r#"<app appid="{ID_A}" status="error-unknownApplication"/><app appid="{ID_B}" status="ok"><updatecheck status="error-internal&lt;script&gt;"/></app>"#
    ));
    let r = parse(&xml).unwrap();
    assert!(
        matches!(&r[0], UpdateCheckResult::Error { kind: UpdateErrorKind::Status(s), .. } if s == "error-unknownApplication")
    );
    // `<`, `>` and spaces are dropped from a status.
    assert!(
        matches!(&r[1], UpdateCheckResult::Error { kind: UpdateErrorKind::Status(s), .. } if s == "error-internalscript")
    );
}

#[test]
fn unusable_offers_are_errors_not_updates() {
    let cases: &[(&str, UpdateErrorKind)] = &[
        (
            r#"<updatecheck codebase="https://d.example/a.crx" version="1.0"/>"#,
            UpdateErrorKind::MissingHash,
        ),
        (
            r#"<updatecheck codebase="https://d.example/a.crx" version="1.0" hash_sha256="abc"/>"#,
            UpdateErrorKind::InvalidHash,
        ),
        (
            r#"<updatecheck codebase="javascript:alert(1)" version="1.0" hash_sha256="HASH"/>"#,
            UpdateErrorKind::InvalidCodebase,
        ),
        (
            r#"<updatecheck codebase="file:///etc/passwd" version="1.0" hash_sha256="HASH"/>"#,
            UpdateErrorKind::InvalidCodebase,
        ),
        (
            r#"<updatecheck version="1.0" hash_sha256="HASH"/>"#,
            UpdateErrorKind::InvalidCodebase,
        ),
        (
            r#"<updatecheck codebase="https://d.example/a.crx" hash_sha256="HASH"/>"#,
            UpdateErrorKind::InvalidVersion,
        ),
        (
            r#"<updatecheck codebase="https://d.example/a.crx" version="1.2.3.4.5" hash_sha256="HASH"/>"#,
            UpdateErrorKind::InvalidVersion,
        ),
        (
            r#"<updatecheck codebase="https://d.example/a.crx" version="x" hash_sha256="HASH"/>"#,
            UpdateErrorKind::InvalidVersion,
        ),
    ];
    for (check, expected) in cases {
        let xml = gupdate(&format!(
            r#"<app appid="{ID_A}" status="ok">{}</app>"#,
            check.replace("HASH", HASH)
        ));
        let r = parse(&xml).unwrap();
        assert!(
            matches!(&r[0], UpdateCheckResult::Error { kind, .. } if kind == expected),
            "{check}: {r:?}"
        );
    }
}

#[test]
fn an_app_without_an_updatecheck_is_an_error() {
    let xml = gupdate(&format!(
        r#"<app appid="{ID_A}" status="ok"></app><app appid="{ID_B}"/>"#
    ));
    let r = parse(&xml).unwrap();
    assert!(r.iter().all(|x| matches!(
        x,
        UpdateCheckResult::Error {
            kind: UpdateErrorKind::MissingUpdateCheck,
            ..
        }
    )));
    assert_eq!(r.len(), 2);
}

#[test]
fn an_invalid_app_id_is_an_error_with_no_id_echoed() {
    let xml = gupdate(
        r#"<app appid="not-a-chrome-id" status="ok"><updatecheck status="noupdate"/></app>"#,
    );
    assert_eq!(
        parse(&xml).unwrap(),
        vec![UpdateCheckResult::Error {
            id: String::new(),
            kind: UpdateErrorKind::InvalidAppId
        }]
    );
}

#[test]
fn entities_in_attributes_are_decoded_but_only_the_five_predefined() {
    let xml = gupdate(&format!(
        r#"<app appid="{ID_A}"><updatecheck codebase="https://d.example/a.crx?x=1&amp;y=2" version="1.0" hash_sha256="{HASH}"/></app>"#
    ));
    match &parse(&xml).unwrap()[0] {
        UpdateCheckResult::Available { codebase, .. } => {
            assert_eq!(codebase.as_str(), "https://d.example/a.crx?x=1&y=2")
        }
        other => panic!("{other:?}"),
    }
    let custom = gupdate(&format!(
        r#"<app appid="{ID_A}"><updatecheck codebase="https://d.example/&custom;" version="1.0" hash_sha256="{HASH}"/></app>"#
    ));
    assert_eq!(parse(&custom), Err(UpdateError::NotXml));
}

#[test]
fn a_prefixed_namespace_and_extra_elements_are_tolerated() {
    let xml = format!(
        r#"<g:gupdate xmlns:g="http://www.google.com/update2/response" protocol="2.0"><g:daystart elapsed_days="1"/><g:app appid="{ID_A}" status="ok"><g:updatecheck status="noupdate"/><g:other/></g:app><!-- c --><?pi x?></g:gupdate>"#
    );
    assert_eq!(
        parse(&xml).unwrap(),
        vec![UpdateCheckResult::UpToDate {
            id: ID_A.to_owned()
        }]
    );
}

#[test]
fn the_first_updatecheck_of_an_app_wins() {
    let xml = gupdate(&format!(
        r#"<app appid="{ID_A}"><updatecheck status="noupdate"/><updatecheck codebase="https://d.example/a" version="9.9" hash_sha256="{HASH}"/></app>"#
    ));
    assert_eq!(
        parse(&xml).unwrap(),
        vec![UpdateCheckResult::UpToDate {
            id: ID_A.to_owned()
        }]
    );
}

#[test]
fn an_invalid_size_is_ignored() {
    let xml = gupdate(&format!(
        r#"<app appid="{ID_A}"><updatecheck codebase="https://d.example/a" version="1.1" hash_sha256="{HASH}" size="-5"/></app>"#
    ));
    assert!(matches!(
        &parse(&xml).unwrap()[0],
        UpdateCheckResult::Available { size: None, .. }
    ));
}

#[test]
fn an_empty_gupdate_has_no_results() {
    assert_eq!(parse(&gupdate("")).unwrap(), vec![]);
}

// ---- hostile XML -------------------------------------------------------------------------

#[test]
fn a_billion_laughs_document_is_refused_before_anything_expands() {
    let xml = r#"<?xml version="1.0"?>
<!DOCTYPE lolz [
 <!ENTITY lol "lol">
 <!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">
 <!ENTITY lol3 "&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;">
 <!ENTITY lol4 "&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;">
 <!ENTITY lol5 "&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;">
 <!ENTITY lol6 "&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;">
 <!ENTITY lol7 "&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;">
 <!ENTITY lol8 "&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;">
 <!ENTITY lol9 "&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;">
]>
<gupdate xmlns="http://www.google.com/update2/response"><app appid="jpignaibiiemhngfjkcpokkamffknabf">&lol9;</app></gupdate>"#;
    assert_eq!(parse(xml), Err(UpdateError::DoctypeNotAllowed));
}

#[test]
fn any_doctype_is_refused() {
    for doctype in [
        "<!DOCTYPE gupdate>",
        "<!DOCTYPE gupdate SYSTEM \"http://evil.example/x.dtd\">",
        "<!doctype x [<!ENTITY a \"b\">]>",
    ] {
        let xml = format!("{doctype}{}", gupdate(""));
        assert_eq!(
            parse(&xml),
            Err(UpdateError::DoctypeNotAllowed),
            "{doctype}"
        );
    }
}

#[test]
fn an_external_entity_reference_in_an_attribute_is_refused() {
    let xml = gupdate(&format!(r#"<app appid="{ID_A}" status="&xxe;"/>"#));
    assert_eq!(parse(&xml), Err(UpdateError::NotXml));
}

#[test]
fn a_response_over_the_cap_is_refused_at_the_edge() {
    let limits = UpdateLimits::default();
    let base = gupdate("");
    let at = format!(
        "{base}{}",
        " ".repeat(limits.max_response_bytes - base.len())
    );
    assert_eq!(at.len(), limits.max_response_bytes);
    assert_eq!(parse_update2_response(at.as_bytes(), &limits), Ok(vec![]));
    let over = format!("{at} ");
    assert_eq!(
        parse_update2_response(over.as_bytes(), &limits),
        Err(UpdateError::TooLarge)
    );
}

#[test]
fn nesting_over_the_cap_is_refused() {
    let limits = UpdateLimits::default();
    let deep = |n: usize| gupdate(&format!("{}{}", "<x>".repeat(n), "</x>".repeat(n)));
    // gupdate itself is level 1.
    assert!(parse_update2_response(deep(limits.max_xml_depth - 1).as_bytes(), &limits).is_ok());
    assert_eq!(
        parse_update2_response(deep(limits.max_xml_depth).as_bytes(), &limits),
        Err(UpdateError::TooDeep)
    );
    let bomb = format!("<gupdate>{}", "<x>".repeat(100_000));
    assert_eq!(
        parse_update2_response(bomb.as_bytes(), &limits),
        Err(UpdateError::TooDeep)
    );
}

#[test]
fn the_app_count_is_capped_at_the_edge() {
    let limits = UpdateLimits::default();
    let app = |i: usize| format!(r#"<app appid="{ID_A}" status="error-x{i}"/>"#);
    let at: String = (0..limits.max_apps).map(app).collect();
    assert_eq!(
        parse_update2_response(gupdate(&at).as_bytes(), &limits)
            .unwrap()
            .len(),
        limits.max_apps
    );
    let over = format!("{at}{}", app(0));
    assert_eq!(
        parse_update2_response(gupdate(&over).as_bytes(), &limits),
        Err(UpdateError::TooManyApps)
    );
}

#[test]
fn an_attribute_over_the_cap_is_refused() {
    let limits = UpdateLimits::default();
    let long = "a".repeat(limits.max_attr_len + 1);
    let xml = gupdate(&format!(r#"<app appid="{ID_A}" status="{long}"/>"#));
    assert_eq!(
        parse_update2_response(xml.as_bytes(), &limits),
        Err(UpdateError::TooLarge)
    );
}

#[test]
fn malformed_xml_is_an_error() {
    for bad in [
        "",
        "plain text",
        "<gupdate>",
        "<gupdate></wrong>",
        "<gupdate><app></gupdate>",
        "<a><b></a></b>",
        "<other/>",
        r#"<gupdate><app appid="x" appid="y"/></gupdate>"#,
        r#"<gupdate><app appid=x/></gupdate>"#,
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
    assert_eq!(
        parse_update2_response(&[0xFF, 0xFE, 0x00], &UpdateLimits::default()),
        Err(UpdateError::NotXml)
    );
}

#[test]
fn error_messages_carry_no_response_content() {
    let secret = "s3cr3t-token";
    let docs = [
        format!("<!DOCTYPE {secret}>"),
        format!("<{secret}/>"),
        format!("<gupdate><app appid=\"{secret}\" status=\"&{secret};\"/></gupdate>"),
    ];
    for doc in docs {
        if let Err(e) = parse(&doc) {
            assert!(!format!("{e} {e:?}").contains(secret), "{e:?}");
        }
    }
    // The error result for an unusable app id does not echo the id either.
    let r = parse(&gupdate(&format!(r#"<app appid="{secret}"/>"#))).unwrap();
    assert!(!format!("{r:?}").contains(secret));
}

#[test]
fn mutated_responses_never_panic() {
    let seed = gupdate(&format!(
        r#"<app appid="{ID_A}" status="ok"><updatecheck codebase="https://d.example/a.crx" version="1.5" hash_sha256="{HASH}" size="12" status="ok"/></app><app appid="{ID_B}" status="ok"><updatecheck status="noupdate"/></app>"#
    ));
    let mut state = 0xA5A5_5A5A_1234_5678u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let junk = [
        "<",
        ">",
        "&",
        "\"",
        "'",
        "/",
        "<!DOCTYPE a>",
        "&amp;",
        "&#0;",
        "&#x110000;",
        "\u{0}",
        "]]>",
        "<![CDATA[",
        "<?",
        "=",
        " ",
    ];
    for _ in 0..5000 {
        let mut s = seed.clone();
        for _ in 0..(1 + next() % 4) {
            let chars: Vec<char> = s.chars().collect();
            let at = (next() as usize) % chars.len();
            let mut out: String = chars[..at].iter().collect();
            match next() % 3 {
                0 => out.push_str(junk[(next() as usize) % junk.len()]),
                1 => {}
                _ => {
                    out.push_str(junk[(next() as usize) % junk.len()]);
                    out.extend(&chars[at..]);
                    s = out;
                    continue;
                }
            }
            out.extend(&chars[(at + 1).min(chars.len())..]);
            s = out;
        }
        let _ = parse(&s);
    }
}
