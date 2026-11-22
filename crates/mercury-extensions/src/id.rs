// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Extension ids: the 32-letter Chrome id, the Firefox `gecko.id`, and what a manifest may claim.
//!
//! Behaviour per Chrome's "Manifest key" page (the id is the first 128 bits of the SHA-256 of the
//! DER public key, each nibble written as a letter `a`-`p`) and MDN's `browser_specific_settings`
//! page (the Firefox id is `name@domain` or a braced GUID).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

/// Longest id accepted, in bytes. Firefox recommends 80 characters; the cap is a safety limit.
pub const MAX_ID_LEN: usize = 255;
/// Length of a Chrome extension id.
pub const CHROME_ID_LEN: usize = 32;
/// Longest `key` string accepted, in bytes (a 4096-bit RSA public key is about 800 characters).
pub const MAX_KEY_LEN: usize = 8192;

/// Which browser family an id belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExtensionKind {
    /// A Chrome extension: the id is 32 letters `a`-`p`.
    Chrome,
    /// A Firefox add-on: the id is `name@domain` or `{uuid}`.
    Firefox,
}

/// Why an id or a manifest `key` was refused. The messages carry no input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// The id is empty.
    #[error("the extension id is empty")]
    Empty,
    /// The id is longer than [`MAX_ID_LEN`].
    #[error("the extension id is too long")]
    TooLong,
    /// The id is neither a Chrome id nor a Firefox id.
    #[error("the extension id has an invalid form")]
    InvalidForm,
    /// The manifest `key` is empty or longer than [`MAX_KEY_LEN`].
    #[error("the manifest key has an invalid length")]
    KeyLength,
    /// The manifest `key` is not base64.
    #[error("the manifest key is not valid base64")]
    KeyBase64,
    /// The decoded manifest `key` is not a DER-encoded public key.
    #[error("the manifest key is not a DER public key")]
    KeyDer,
}

/// A validated extension id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExtensionId(String);

impl ExtensionId {
    /// Validates `s` as a Chrome or Firefox id.
    pub fn parse(s: &str) -> Result<Self, IdError> {
        if s.is_empty() {
            return Err(IdError::Empty);
        }
        if s.len() > MAX_ID_LEN {
            return Err(IdError::TooLong);
        }
        if is_chrome_id(s) || is_firefox_guid(s) || is_firefox_email_form(s) {
            Ok(ExtensionId(s.to_owned()))
        } else {
            Err(IdError::InvalidForm)
        }
    }

    /// The id derived from a DER `SubjectPublicKeyInfo`: the first 128 bits of its SHA-256, each
    /// nibble mapped to `a`-`p`.
    pub fn from_public_key_der(der: &[u8]) -> Self {
        ExtensionId(map_digest_to_letters(der))
    }

    /// The id for a manifest `key` (base64 of the DER public key; PEM armour and whitespace are
    /// tolerated, as in Chrome).
    pub fn from_manifest_key(key: &str) -> Result<Self, IdError> {
        let der = decode_manifest_key(key)?;
        Ok(Self::from_public_key_der(&der))
    }

    /// The id as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Chrome or Firefox form.
    pub fn kind(&self) -> ExtensionKind {
        if is_chrome_id(&self.0) {
            ExtensionKind::Chrome
        } else {
            ExtensionKind::Firefox
        }
    }

    /// The host label used in the extension's own URLs (`mercury-extension://<label>/...`): the id
    /// itself for a Chrome id, and, because `@`, `{` and `}` cannot be in a host, a 32-letter
    /// label derived from the id for a Firefox id. The extension-scheme lane (EE23) may replace
    /// this mapping; the permission checks only compare against it.
    pub fn host_label(&self) -> String {
        match self.kind() {
            ExtensionKind::Chrome => self.0.clone(),
            ExtensionKind::Firefox => map_digest_to_letters(self.0.as_bytes()),
        }
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ExtensionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for ExtensionId {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, IdError> {
        ExtensionId::parse(s)
    }
}

impl Serialize for ExtensionId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ExtensionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        ExtensionId::parse(&s).map_err(serde::de::Error::custom)
    }
}

fn map_digest_to_letters(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    let mut out = String::with_capacity(CHROME_ID_LEN);
    for byte in digest.iter().take(CHROME_ID_LEN / 2) {
        out.push(nibble_letter(byte >> 4));
        out.push(nibble_letter(byte & 0x0f));
    }
    out
}

fn nibble_letter(nibble: u8) -> char {
    char::from(b'a' + (nibble & 0x0f))
}

fn is_chrome_id(s: &str) -> bool {
    s.len() == CHROME_ID_LEN && s.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

fn is_firefox_guid(s: &str) -> bool {
    // `{8-4-4-4-12}` of hex digits.
    let Some(inner) = s.strip_prefix('{').and_then(|r| r.strip_suffix('}')) else {
        return false;
    };
    let groups: Vec<&str> = inner.split('-').collect();
    let want = [8usize, 4, 4, 4, 12];
    groups.len() == want.len()
        && groups
            .iter()
            .zip(want)
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn is_firefox_email_form(s: &str) -> bool {
    // `^[a-zA-Z0-9-._]*@[a-zA-Z0-9-._]+$`
    let ok = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_');
    match s.split_once('@') {
        Some((local, domain)) => {
            !domain.is_empty() && local.bytes().all(ok) && domain.bytes().all(ok)
        }
        None => false,
    }
}

/// Decodes a manifest `key` to its DER bytes and checks that it looks like a DER `SEQUENCE`
/// (the outer length must match). The key itself is not parsed further.
pub fn decode_manifest_key(key: &str) -> Result<Vec<u8>, IdError> {
    if key.is_empty() || key.len() > MAX_KEY_LEN {
        return Err(IdError::KeyLength);
    }
    let mut compact = String::with_capacity(key.len());
    for line in key.lines() {
        let line = line.trim();
        if line.starts_with("-----") {
            continue;
        }
        compact.extend(line.chars().filter(|c| !c.is_ascii_whitespace()));
    }
    let der = base64_decode(&compact).ok_or(IdError::KeyBase64)?;
    if !der_sequence_is_complete(&der) {
        return Err(IdError::KeyDer);
    }
    Ok(der)
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let trimmed = s.trim_end_matches('=');
    let padding = s.len() - trimmed.len();
    if padding > 2 || (padding > 0 && !s.len().is_multiple_of(4)) || trimmed.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() / 4 * 3 + 2);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for b in trimmed.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn der_sequence_is_complete(der: &[u8]) -> bool {
    let (Some(&tag), Some(&first)) = (der.first(), der.get(1)) else {
        return false;
    };
    if tag != 0x30 {
        return false;
    }
    let (len, header) = if first < 0x80 {
        (usize::from(first), 2usize)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 {
            return false;
        }
        let Some(bytes) = der.get(2..2 + n) else {
            return false;
        };
        let len = bytes
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
        (len, 2 + n)
    };
    header.checked_add(len) == Some(der.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "jpignaibiiemhngfjkcpokkamffknabf";

    #[test]
    fn chrome_id_is_accepted() {
        let id = ExtensionId::parse(GOOD).unwrap();
        assert_eq!(id.kind(), ExtensionKind::Chrome);
        assert_eq!(id.to_string(), GOOD);
        assert_eq!(id.host_label(), GOOD);
    }

    #[test]
    fn near_misses_of_the_chrome_form_are_refused() {
        for bad in [
            "",
            "jpignaibiiemhngfjkcpokkamffknab",
            "jpignaibiiemhngfjkcpokkamffknabfa",
            "qpignaibiiemhngfjkcpokkamffknabf",
            "JPIGNAIBIIEMHNGFJKCPOKKAMFFKNABF",
        ] {
            assert!(ExtensionId::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn firefox_forms_are_accepted() {
        for good in [
            "uBlock0@raymondhill.net",
            "@addon-example",
            "great_app@developers.company",
            "{daf44bf7-a45e-4450-979c-91cf07434c3d}",
        ] {
            let id = ExtensionId::parse(good).unwrap();
            assert_eq!(id.kind(), ExtensionKind::Firefox, "{good}");
            assert_eq!(id.host_label().len(), CHROME_ID_LEN);
        }
    }

    #[test]
    fn firefox_near_misses_are_refused() {
        for bad in [
            "name@",
            "a@b@c",
            "a b@c.d",
            "a@b/c",
            "{daf44bf7-a45e-4450-979c-91cf07434c3}",
            "{daf44bf7-a45e-4450-979c-91cf07434c3d",
            "daf44bf7-a45e-4450-979c-91cf07434c3d",
            "{zzzzzzzz-a45e-4450-979c-91cf07434c3d}",
            "plain",
        ] {
            assert!(ExtensionId::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn overlong_ids_are_refused() {
        let long = format!("{}@x.org", "a".repeat(MAX_ID_LEN));
        assert_eq!(ExtensionId::parse(&long), Err(IdError::TooLong));
    }

    #[test]
    fn serde_round_trips_as_a_string() {
        let id = ExtensionId::parse(GOOD).unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{GOOD}\""));
        let back: ExtensionId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
        assert!(serde_json::from_str::<ExtensionId>("\"nope\"").is_err());
        assert!(serde_json::from_str::<ExtensionId>("5").is_err());
    }

    #[test]
    fn key_id_matches_crx3() {
        // A DER SEQUENCE of any content hashes the same way crx3 does it.
        let der = [0x30, 0x03, 0x02, 0x01, 0x05];
        let id = ExtensionId::from_public_key_der(&der);
        assert_eq!(id.as_str(), crx3::id_from_public_key(&der));
        assert_eq!(id.kind(), ExtensionKind::Chrome);
    }

    #[test]
    fn manifest_key_is_base64_der() {
        // base64 of 30 03 02 01 05
        let id = ExtensionId::from_manifest_key("MAMCAQU=").unwrap();
        assert_eq!(
            id,
            ExtensionId::from_public_key_der(&[0x30, 0x03, 0x02, 0x01, 0x05])
        );
        // The same key with PEM armour and line breaks.
        let pem = "-----BEGIN PUBLIC KEY-----\nMAMC\nAQU=\n-----END PUBLIC KEY-----\n";
        assert_eq!(ExtensionId::from_manifest_key(pem).unwrap(), id);
    }

    #[test]
    fn bad_manifest_keys_are_refused() {
        assert_eq!(ExtensionId::from_manifest_key(""), Err(IdError::KeyLength));
        assert_eq!(
            ExtensionId::from_manifest_key(&"A".repeat(MAX_KEY_LEN + 1)),
            Err(IdError::KeyLength)
        );
        assert_eq!(
            ExtensionId::from_manifest_key("!!!!"),
            Err(IdError::KeyBase64)
        );
        assert_eq!(
            ExtensionId::from_manifest_key("AAAAA"),
            Err(IdError::KeyBase64)
        );
        // Valid base64 that is not a DER sequence, and a sequence with a wrong length.
        assert_eq!(ExtensionId::from_manifest_key("AAAA"), Err(IdError::KeyDer));
        assert_eq!(
            ExtensionId::from_manifest_key("MAQCAQU="),
            Err(IdError::KeyDer)
        );
    }
}
