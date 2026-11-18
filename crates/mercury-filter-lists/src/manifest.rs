// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The manifest that ships beside the bundled lists: which lists there are, where each one is
//! published, its licence, its version or date, and the SHA-256 of the bundled file.

use serde::Deserialize;
use url::Url;

use crate::timestamp::parse_utc;

/// The manifest's file name in the bundled directory.
pub const MANIFEST_FILE: &str = "manifest.json";
/// The only manifest format this version reads.
pub const MANIFEST_FORMAT_VERSION: u32 = 1;
/// The largest manifest read, in bytes.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// The most lists a manifest may name.
pub const MAX_LISTS: usize = 32;

const MAX_ID_BYTES: usize = 64;
const MAX_NAME_BYTES: usize = 100;
const MAX_LICENCE_BYTES: usize = 200;
const MAX_VERSION_BYTES: usize = 64;
const MAX_URL_BYTES: usize = 2048;

/// The bundled lists, in the order they are given to the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub lists: Vec<ListEntry>,
}

/// One bundled list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListEntry {
    /// A short stable name (`easylist`): lower-case ASCII letters, digits and `-`.
    pub id: String,
    /// The list's title, as the user sees it (`EasyList`).
    pub name: String,
    /// The bundled file, in the manifest's directory: `<id-like name>.txt`, no directories.
    pub file: String,
    /// Where the publisher serves the current version. Always `https`.
    pub source_url: Url,
    /// The licence the list is used under: one SPDX identifier, whose text ships as
    /// `LICENSES/<licence>.txt` beside the lists.
    pub licence: String,
    /// Whom the notice credits for the list.
    pub credit: String,
    /// The list's own version header, if it has one.
    pub version: Option<String>,
    /// When the bundled copy was taken, in seconds since the Unix epoch.
    pub date: u64,
    /// SHA-256 of the bundled file, lower-case hexadecimal.
    pub sha256: String,
}

/// Why a manifest cannot be used.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("the manifest is not valid JSON of the expected shape: {0}")]
    Syntax(String),
    #[error(
        "the manifest has format version {0}; this version of Mercury Browser reads {MANIFEST_FORMAT_VERSION}"
    )]
    FormatVersion(u32),
    #[error("the manifest names no lists")]
    NoLists,
    #[error("the manifest names more than {MAX_LISTS} lists")]
    TooManyLists,
    #[error("list {index}: {problem}")]
    List { index: usize, problem: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    format_version: u32,
    lists: Vec<RawEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    id: String,
    name: String,
    file: String,
    source_url: String,
    licence: String,
    credit: String,
    version: Option<String>,
    date: String,
    sha256: String,
}

/// Reads and checks a manifest.
pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    let raw: RawManifest =
        serde_json::from_str(text).map_err(|error| ManifestError::Syntax(error.to_string()))?;
    if raw.format_version != MANIFEST_FORMAT_VERSION {
        return Err(ManifestError::FormatVersion(raw.format_version));
    }
    if raw.lists.is_empty() {
        return Err(ManifestError::NoLists);
    }
    if raw.lists.len() > MAX_LISTS {
        return Err(ManifestError::TooManyLists);
    }
    let mut lists: Vec<ListEntry> = Vec::with_capacity(raw.lists.len());
    for (index, raw) in raw.lists.into_iter().enumerate() {
        let entry = checked_entry(raw).map_err(|problem| ManifestError::List { index, problem })?;
        let problem = if lists.iter().any(|list| list.id == entry.id) {
            Some(format!("the id {:?} is used twice", entry.id))
        } else if lists.iter().any(|list| list.file == entry.file) {
            Some(format!("the file {:?} is used twice", entry.file))
        } else {
            None
        };
        if let Some(problem) = problem {
            return Err(ManifestError::List { index, problem });
        }
        lists.push(entry);
    }
    Ok(Manifest { lists })
}

/// A short safe name: lower-case ASCII letters, digits and `-`, not starting with `-`.
pub(crate) fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn checked_entry(raw: RawEntry) -> Result<ListEntry, String> {
    if !is_safe_id(&raw.id) {
        return Err(format!(
            "the id {:?} is not lower-case letters, digits and '-'",
            raw.id
        ));
    }
    let stem = raw.file.strip_suffix(".txt").unwrap_or_default();
    if !is_safe_id(stem) {
        return Err(format!(
            "the file {:?} is not a plain name ending in .txt",
            raw.file
        ));
    }
    let name = raw.name.trim();
    if name.is_empty() || name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        return Err("the name is empty, too long or has control characters".to_owned());
    }
    let licence = raw.licence.trim();
    if licence.is_empty()
        || licence.len() > MAX_LICENCE_BYTES
        || licence.starts_with(['.', '-'])
        || !licence
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return Err("the licence is not one SPDX licence identifier".to_owned());
    }
    let credit = raw.credit.trim();
    if credit.is_empty() || credit.len() > MAX_LICENCE_BYTES || credit.chars().any(char::is_control)
    {
        return Err("the credit is empty, too long or has control characters".to_owned());
    }
    if raw.source_url.len() > MAX_URL_BYTES {
        return Err("the source URL is too long".to_owned());
    }
    let source_url = Url::parse(&raw.source_url)
        .map_err(|error| format!("the source URL does not parse: {error}"))?;
    if source_url.scheme() != "https"
        || source_url.host_str().is_none()
        || !source_url.username().is_empty()
        || source_url.password().is_some()
    {
        return Err("the source URL is not an https URL without credentials".to_owned());
    }
    let version = match raw.version {
        Some(version)
            if version.trim().is_empty()
                || version.len() > MAX_VERSION_BYTES
                || version.chars().any(char::is_control) =>
        {
            return Err("the version is empty, too long or has control characters".to_owned());
        }
        version => version,
    };
    let date = parse_utc(&raw.date)
        .ok_or_else(|| format!("the date {:?} is not YYYY-MM-DDTHH:MM:SSZ", raw.date))?;
    if !is_sha256_hex(&raw.sha256) {
        return Err("the sha256 is not 64 lower-case hexadecimal digits".to_owned());
    }
    Ok(ListEntry {
        id: raw.id,
        name: name.to_owned(),
        file: raw.file,
        source_url,
        licence: licence.to_owned(),
        credit: credit.to_owned(),
        version,
        date,
        sha256: raw.sha256,
    })
}

pub(crate) fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "ca554da776cd140d0a4803bc6a298c5d2251595e13ddbdcad888c347ee7ad1f8";

    fn entry(id: &str, file: &str, url: &str) -> String {
        format!(
            r#"{{"id": "{id}", "name": "Name of {id}", "file": "{file}", "source_url": "{url}",
                "licence": "MPL-2.0", "credit": "Credit of {id}", "version": null,
                "date": "2026-11-15T20:46:51Z",
                "sha256": "{HASH}"}}"#
        )
    }

    fn manifest(entries: &[String]) -> String {
        format!(
            r#"{{"format_version": 1, "lists": [{}]}}"#,
            entries.join(",")
        )
    }

    fn problem(text: &str) -> String {
        match parse_manifest(text) {
            Err(ManifestError::List { problem, .. }) => problem,
            other => panic!("expected a list problem, got {other:?}"),
        }
    }

    #[test]
    fn a_valid_manifest_is_read_in_order() {
        let text = format!(
            r#"{{"format_version": 1, "lists": [
                {{"id": "easylist", "name": "EasyList", "file": "easylist.txt",
                  "source_url": "https://easylist.to/easylist/easylist.txt",
                  "licence": "CC-BY-SA-3.0",
                  "credit": "The EasyList authors (https://easylist.to/)",
                  "version": "202611152034",
                  "date": "2026-11-15T20:46:51Z", "sha256": "{HASH}"}},
                {}
            ]}}"#,
            entry(
                "brave-unbreak",
                "brave-unbreak.txt",
                "https://example.org/u.txt"
            )
        );

        let manifest = parse_manifest(&text).expect("a valid manifest");

        assert_eq!(manifest.lists.len(), 2);
        let first = &manifest.lists[0];
        assert_eq!(first.id, "easylist");
        assert_eq!(first.name, "EasyList");
        assert_eq!(first.file, "easylist.txt");
        assert_eq!(
            first.source_url.as_str(),
            "https://easylist.to/easylist/easylist.txt"
        );
        assert_eq!(first.licence, "CC-BY-SA-3.0");
        assert_eq!(first.credit, "The EasyList authors (https://easylist.to/)");
        assert_eq!(first.version.as_deref(), Some("202611152034"));
        assert_eq!(first.date, 1_794_775_611);
        assert_eq!(first.sha256, HASH);
        assert_eq!(manifest.lists[1].id, "brave-unbreak");
        assert_eq!(manifest.lists[1].version, None);
    }

    #[test]
    fn the_bundled_manifest_is_valid_and_matches_its_files() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../talaria-servo/resources/adblock");
        let text = std::fs::read_to_string(directory.join(MANIFEST_FILE)).expect("the manifest");
        let manifest = parse_manifest(&text).expect("a valid manifest");

        assert_eq!(manifest.lists.len(), 5);
        for list in &manifest.lists {
            let bytes = std::fs::read(directory.join(&list.file)).expect("the list");
            assert_eq!(crate::sha256_hex(&bytes), list.sha256, "{}", list.file);
            assert_eq!(list.source_url.scheme(), "https");
        }
    }

    #[test]
    fn wrong_shapes_and_versions_are_refused() {
        assert!(matches!(
            parse_manifest("not json"),
            Err(ManifestError::Syntax(_))
        ));
        assert!(matches!(
            parse_manifest(r#"{"format_version": 1}"#),
            Err(ManifestError::Syntax(_))
        ));
        assert!(matches!(
            parse_manifest(r#"{"format_version": 1, "lists": [], "extra": 1}"#),
            Err(ManifestError::Syntax(_))
        ));
        assert_eq!(
            parse_manifest(r#"{"format_version": 2, "lists": []}"#),
            Err(ManifestError::FormatVersion(2))
        );
        assert_eq!(
            parse_manifest(r#"{"format_version": 1, "lists": []}"#),
            Err(ManifestError::NoLists)
        );
        let many = (0..=MAX_LISTS)
            .map(|index| {
                entry(
                    &format!("l{index}"),
                    &format!("l{index}.txt"),
                    "https://a.test/",
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            parse_manifest(&manifest(&many)),
            Err(ManifestError::TooManyLists)
        );
    }

    #[test]
    fn unsafe_or_odd_fields_are_refused() {
        let ok = "https://example.org/list.txt";
        for (id, file, url) in [
            ("Easy", "easy.txt", ok),
            ("", "easy.txt", ok),
            ("easy list", "easy.txt", ok),
            ("easy", "../easy.txt", ok),
            ("easy", "sub/easy.txt", ok),
            ("easy", ".easy.txt", ok),
            ("easy", "easy.dat", ok),
            ("easy", "easy.txt", "http://example.org/list.txt"),
            ("easy", "easy.txt", "file:///etc/passwd"),
            ("easy", "easy.txt", "https://user:pw@example.org/list.txt"),
            ("easy", "easy.txt", "not a url"),
        ] {
            assert!(
                !problem(&manifest(&[entry(id, file, url)])).is_empty(),
                "{id} {file} {url}"
            );
        }
        let bad_hash = entry("easy", "easy.txt", ok).replace(HASH, &HASH.to_uppercase());
        assert!(!problem(&manifest(&[bad_hash])).is_empty());
        let short_hash = entry("easy", "easy.txt", ok).replace(HASH, &HASH[1..]);
        assert!(!problem(&manifest(&[short_hash])).is_empty());
        let bad_date = entry("easy", "easy.txt", ok).replace("2026-11-15T20:46:51Z", "yesterday");
        assert!(!problem(&manifest(&[bad_date])).is_empty());
        let no_name = entry("easy", "easy.txt", ok).replace("Name of easy", "");
        assert!(!problem(&manifest(&[no_name])).is_empty());
        let no_credit = entry("easy", "easy.txt", ok).replace("Credit of easy", " ");
        assert!(problem(&manifest(&[no_credit])).contains("credit"));
        for licence in [
            "GPL-3.0-or-later OR CC-BY-SA-3.0",
            "../MPL-2.0",
            "MPL 2.0",
            "",
        ] {
            let odd = entry("easy", "easy.txt", ok).replace("\"MPL-2.0\"", &format!("{licence:?}"));
            assert!(problem(&manifest(&[odd])).contains("licence"), "{licence}");
        }
    }

    #[test]
    fn ids_and_files_must_be_unique() {
        let ok = "https://example.org/list.txt";
        assert!(
            problem(&manifest(&[
                entry("a", "a.txt", ok),
                entry("a", "b.txt", ok)
            ]))
            .contains("id")
        );
        assert!(
            problem(&manifest(&[
                entry("a", "a.txt", ok),
                entry("b", "a.txt", ok)
            ]))
            .contains("file")
        );
    }
}
