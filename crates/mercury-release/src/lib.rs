// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Release tooling for Mercury Browser.
//!
//! Each GitHub Release carries `update.json`, a small manifest naming the version and the
//! digest of every download, and `update.json.minisig`, its minisign signature. Mercury's
//! update check fetches both from the latest release, verifies the signature against the
//! public key compiled into the browser and compares the versions locally.
//!
//! The signature's trusted comment is `mercury-browser update <version>`: it is covered by
//! the signature, so a verifier can refuse a manifest whose comment does not name the version
//! it expects (a replayed older manifest, for example).

use std::fmt;
use std::io::Cursor;

/// The product name written into every manifest.
pub const PRODUCT: &str = "mercury-browser";

/// One downloadable file of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// The file name as attached to the release.
    pub name: String,
    /// Lower-case hexadecimal SHA-256 of the file.
    pub sha256: String,
    /// Size in bytes.
    pub size: u64,
}

impl Asset {
    /// Parses `NAME:SHA256:SIZE`, the form the release workflow passes on the command line.
    pub fn parse(spec: &str) -> Result<Self, Error> {
        let mut parts = spec.split(':');
        let (Some(name), Some(sha256), Some(size), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(Error::Invalid(format!(
                "asset '{spec}' is not NAME:SHA256:SIZE"
            )));
        };
        let size = size
            .parse::<u64>()
            .map_err(|_| Error::Invalid(format!("asset size '{size}' is not a number")))?;
        let asset = Self {
            name: name.to_owned(),
            sha256: sha256.to_owned(),
            size,
        };
        asset.validate()?;
        Ok(asset)
    }

    fn validate(&self) -> Result<(), Error> {
        let name_ok = !self.name.is_empty()
            && !self.name.starts_with('.')
            && self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        if !name_ok {
            return Err(Error::Invalid(format!(
                "asset name '{}' may only hold letters, digits, '.', '-' and '_'",
                self.name
            )));
        }
        let digest_ok = self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !digest_ok {
            return Err(Error::Invalid(format!(
                "'{}' is not a lower-case hexadecimal SHA-256",
                self.sha256
            )));
        }
        if self.size == 0 {
            return Err(Error::Invalid(format!("asset '{}' is empty", self.name)));
        }
        Ok(())
    }
}

/// What the manifest describes: one release and its downloads per platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// `MAJOR.MINOR.PATCH`; pre-release versions are refused, since they are never offered.
    pub version: String,
    /// Publication date, `YYYY-MM-DD` (UTC).
    pub published: String,
    /// `(platform, asset)` pairs, for example `("macos-arm64", …)`, written in this order.
    pub platforms: Vec<(String, Asset)>,
}

/// Errors from building, signing or verifying a manifest.
#[derive(Debug)]
pub enum Error {
    /// An input did not have the expected form.
    Invalid(String),
    /// The minisign library refused a key or a signature.
    Signature(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "{message}"),
            Self::Signature(message) => write!(f, "signature: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<minisign::PError> for Error {
    fn from(error: minisign::PError) -> Self {
        Self::Signature(error.to_string())
    }
}

/// Whether `version` is a plain `MAJOR.MINOR.PATCH` release version.
pub fn is_release_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
        })
}

fn is_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let digits = |range: std::ops::Range<usize>| bytes[range].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let month: u32 = date[5..7].parse().unwrap_or(0);
    let day: u32 = date[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

/// The trusted comment signed together with the manifest of `version`.
pub fn trusted_comment(version: &str) -> String {
    format!("{PRODUCT} update {version}")
}

/// Writes the manifest as JSON with a fixed key order and a trailing newline.
///
/// Every value is validated first, and the values that reach the JSON are restricted to
/// characters that need no escaping.
pub fn manifest_json(release: &Release) -> Result<String, Error> {
    if !is_release_version(&release.version) {
        return Err(Error::Invalid(format!(
            "version '{}' is not MAJOR.MINOR.PATCH (pre-releases are never offered as updates)",
            release.version
        )));
    }
    if !is_date(&release.published) {
        return Err(Error::Invalid(format!(
            "publication date '{}' is not YYYY-MM-DD",
            release.published
        )));
    }
    if release.platforms.is_empty() {
        return Err(Error::Invalid(
            "a manifest needs at least one platform".into(),
        ));
    }
    let mut platforms = Vec::new();
    for (index, (platform, asset)) in release.platforms.iter().enumerate() {
        let platform_ok = !platform.is_empty()
            && platform
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'));
        if !platform_ok {
            return Err(Error::Invalid(format!(
                "platform '{platform}' may only hold lower-case letters, digits, '-' and '_'"
            )));
        }
        if release.platforms[..index]
            .iter()
            .any(|(seen, _)| seen == platform)
        {
            return Err(Error::Invalid(format!(
                "platform '{platform}' is listed twice"
            )));
        }
        asset.validate()?;
        platforms.push(format!(
            "    \"{platform}\": {{\n      \"asset\": \"{}\",\n      \"sha256\": \"{}\",\n      \"size\": {}\n    }}",
            asset.name, asset.sha256, asset.size
        ));
    }
    Ok(format!(
        "{{\n  \"product\": \"{PRODUCT}\",\n  \"version\": \"{}\",\n  \"published\": \"{}\",\n  \"platforms\": {{\n{}\n  }}\n}}\n",
        release.version,
        release.published,
        platforms.join(",\n")
    ))
}

/// Signs `manifest` for `version` with a minisign secret key (the text of a `.key` file) and
/// its password (`None` for a key stored without one, as the tests use). Returns the text of
/// the `.minisig` file. The signature is checked against the key's own public half before it
/// is returned.
pub fn sign_manifest(
    manifest: &[u8],
    version: &str,
    secret_key: &str,
    password: Option<String>,
) -> Result<String, Error> {
    if !is_release_version(version) {
        return Err(Error::Invalid(format!(
            "version '{version}' is not MAJOR.MINOR.PATCH"
        )));
    }
    let key_box = minisign::SecretKeyBox::from_string(secret_key.trim_end())?;
    let secret = match password {
        Some(password) => key_box.into_secret_key(Some(password))?,
        None => key_box.into_unencrypted_secret_key()?,
    };
    let public = minisign::PublicKey::from_secret_key(&secret)?;
    let signature = minisign::sign(
        Some(&public),
        &secret,
        Cursor::new(manifest),
        Some(&trusted_comment(version)),
        Some(&format!("{PRODUCT} update manifest")),
    )?;
    Ok(signature.into_string())
}

/// Checks `signature` (the text of a `.minisig` file) over `manifest` with `public_key` (the
/// text of a `.pub` file, or its base64 line), and that the trusted comment names `version`.
pub fn verify_manifest(
    manifest: &[u8],
    signature: &str,
    public_key: &str,
    version: &str,
) -> Result<(), Error> {
    let public = parse_public_key(public_key)?;
    let signature = minisign::SignatureBox::from_string(signature)?;
    minisign::verify(
        &public,
        &signature,
        Cursor::new(manifest),
        true,
        false,
        false,
    )?;
    let comment = signature.trusted_comment()?;
    let expected = trusted_comment(version);
    if comment != expected {
        return Err(Error::Signature(format!(
            "trusted comment is '{comment}', expected '{expected}'"
        )));
    }
    Ok(())
}

fn parse_public_key(text: &str) -> Result<minisign::PublicKey, Error> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    match lines.as_slice() {
        [base64] => Ok(minisign::PublicKey::from_base64(base64)?),
        [comment, _] if comment.starts_with("untrusted comment:") => {
            Ok(minisign::PublicKeyBox::from_string(text)?.into_public_key()?)
        }
        _ => Err(Error::Invalid(
            "the public key is neither a minisign .pub file nor its base64 line".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const DIGEST_B: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    fn release() -> Release {
        Release {
            version: "0.3.0".into(),
            published: "2026-12-01".into(),
            platforms: vec![
                (
                    "macos-arm64".into(),
                    Asset {
                        name: "Mercury-Browser-0.3.0-macos-arm64.dmg".into(),
                        sha256: DIGEST_A.into(),
                        size: 123_456_789,
                    },
                ),
                (
                    "linux-x86_64".into(),
                    Asset {
                        name: "mercury-browser-linux-x86_64.tar.gz".into(),
                        sha256: DIGEST_B.into(),
                        size: 42,
                    },
                ),
            ],
        }
    }

    /// A fresh key pair for each test, never the release key. Unencrypted, because
    /// minisign's password derivation (scrypt with 1 GiB) is too heavy for a unit test.
    fn key_pair() -> (String, String) {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let public = pair.pk.to_box().unwrap().into_string();
        let secret = pair.sk.to_box(None).unwrap().into_string();
        (public, secret)
    }

    #[test]
    fn manifest_has_a_fixed_shape() {
        let json = manifest_json(&release()).unwrap();
        assert_eq!(
            json,
            format!(
                "{{\n  \"product\": \"mercury-browser\",\n  \"version\": \"0.3.0\",\n  \"published\": \"2026-12-01\",\n  \"platforms\": {{\n    \"macos-arm64\": {{\n      \"asset\": \"Mercury-Browser-0.3.0-macos-arm64.dmg\",\n      \"sha256\": \"{DIGEST_A}\",\n      \"size\": 123456789\n    }},\n    \"linux-x86_64\": {{\n      \"asset\": \"mercury-browser-linux-x86_64.tar.gz\",\n      \"sha256\": \"{DIGEST_B}\",\n      \"size\": 42\n    }}\n  }}\n}}\n"
            )
        );
    }

    #[test]
    fn manifest_refuses_pre_releases_and_bad_versions() {
        for version in ["0.3.0-rc.1", "0.3", "v0.3.0", "0.03.0", "0.3.0 ", ""] {
            let mut bad = release();
            bad.version = version.into();
            assert!(manifest_json(&bad).is_err(), "accepted {version:?}");
        }
    }

    #[test]
    fn manifest_refuses_bad_dates_assets_and_platforms() {
        let mut bad = release();
        bad.published = "2026-13-01".into();
        assert!(manifest_json(&bad).is_err());

        let mut bad = release();
        bad.platforms[0].1.sha256 = DIGEST_A.to_uppercase();
        assert!(manifest_json(&bad).is_err());

        let mut bad = release();
        bad.platforms[0].1.name = "a\"b.dmg".into();
        assert!(manifest_json(&bad).is_err());

        let mut bad = release();
        bad.platforms[1].0 = "macos-arm64".into();
        assert!(manifest_json(&bad).is_err());

        let mut bad = release();
        bad.platforms.clear();
        assert!(manifest_json(&bad).is_err());
    }

    #[test]
    fn asset_specs_parse_and_validate() {
        let asset = Asset::parse(&format!("a.dmg:{DIGEST_A}:7")).unwrap();
        assert_eq!(asset.name, "a.dmg");
        assert_eq!(asset.size, 7);
        assert!(Asset::parse(&format!("a.dmg:{DIGEST_A}")).is_err());
        assert!(Asset::parse(&format!("a.dmg:{DIGEST_A}:7:8")).is_err());
        assert!(Asset::parse(&format!("a.dmg:{DIGEST_A}:x")).is_err());
        assert!(Asset::parse(&format!("a.dmg:{DIGEST_A}:0")).is_err());
        assert!(Asset::parse(&format!("../a.dmg:{DIGEST_A}:7")).is_err());
    }

    #[test]
    fn a_signed_manifest_verifies() {
        let (public, secret) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        let signature = sign_manifest(manifest.as_bytes(), "0.3.0", &secret, None).unwrap();
        assert!(signature.contains("trusted comment: mercury-browser update 0.3.0"));
        verify_manifest(manifest.as_bytes(), &signature, &public, "0.3.0").unwrap();

        let base64_line = public.lines().nth(1).unwrap();
        verify_manifest(manifest.as_bytes(), &signature, base64_line, "0.3.0").unwrap();
    }

    #[test]
    fn tampering_is_detected() {
        let (public, secret) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        let signature = sign_manifest(manifest.as_bytes(), "0.3.0", &secret, None).unwrap();

        let altered = manifest.replace("123456789", "123456788");
        assert!(verify_manifest(altered.as_bytes(), &signature, &public, "0.3.0").is_err());

        let comment = signature.replace("update 0.3.0", "update 0.4.0");
        assert!(verify_manifest(manifest.as_bytes(), &comment, &public, "0.4.0").is_err());
    }

    #[test]
    fn the_trusted_comment_must_name_the_expected_version() {
        let (public, secret) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        let signature = sign_manifest(manifest.as_bytes(), "0.3.0", &secret, None).unwrap();
        assert!(verify_manifest(manifest.as_bytes(), &signature, &public, "0.4.0").is_err());
    }

    #[test]
    fn another_key_does_not_verify() {
        let (_, secret) = key_pair();
        let (other_public, _) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        let signature = sign_manifest(manifest.as_bytes(), "0.3.0", &secret, None).unwrap();
        assert!(verify_manifest(manifest.as_bytes(), &signature, &other_public, "0.3.0").is_err());
    }

    #[test]
    fn signing_refuses_a_malformed_key_or_version() {
        let (_, secret) = key_pair();
        assert!(sign_manifest(b"{}", "0.3.0", "not a key", None).is_err());
        assert!(sign_manifest(b"{}", "0.3.0-rc.1", &secret, None).is_err());
    }
}
