// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Release tooling for Mercury Browser.
//!
//! Each GitHub Release carries `update.json`, a small manifest naming the version and the
//! digest of every download, and `update.json.minisig`, its minisign signature. They are
//! for Mercury's update check, which is not built yet. That check is to fetch both from the
//! latest release and call [`verify_update`] with the public key compiled into the browser
//! and its own version. The call fails closed: on any error (a bad signature, another key, a
//! manifest of another shape, a version that is not newer than the installed one) the
//! manifest is ignored and nothing is offered. Only a manifest that passes may be read.
//!
//! The signature's trusted comment is `mercury-browser update <version>`: it is covered by
//! the signature, so a verifier learns the signed version from it, and a replayed older
//! manifest is recognised by its older version. [`verify_signed_manifest`] returns that
//! signed version; a caller that uses it must itself refuse a version that is equal to or
//! older than its own ([`is_newer`] compares them numerically), which [`verify_update`] does.
//!
//! The signer is stricter than the verifier: [`sign_manifest`] signs only a manifest that is
//! byte for byte what [`manifest_json`] writes, so a signed manifest has one `version` key,
//! written once, and nothing after it that a JSON parser could read differently. The verifier
//! checks only the head (product and version), so that a later manifest format that adds
//! fields can still be verified by an older browser.
//!
//! No error from this crate carries text taken from a secret key or a password.

use std::fmt;
use std::io::Cursor;

/// The product name written into every manifest.
pub const PRODUCT: &str = "mercury-browser";

/// The largest manifest that is signed or verified. A real one is well under 2 KiB; a caller
/// that downloads one should stop reading at this size.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// The largest signature or public key text that is verified (a real one is about 300 bytes).
const MAX_TEXT_BYTES: usize = 4 * 1024;

/// The longest asset name (the usual file name limit).
const MAX_NAME_LEN: usize = 255;

/// The longest platform key.
const MAX_PLATFORM_LEN: usize = 32;

/// The largest size written to the manifest: the largest integer every JSON reader keeps
/// exactly (2^53 - 1).
const MAX_SIZE: u64 = (1 << 53) - 1;

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
            && self.name.len() <= MAX_NAME_LEN
            && !self.name.starts_with('.')
            && self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        if !name_ok {
            return Err(Error::Invalid(format!(
                "asset name '{}' may only hold letters, digits, '.', '-' and '_' (at most {MAX_NAME_LEN}), and not start with '.'",
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
        if self.size > MAX_SIZE {
            return Err(Error::Invalid(format!(
                "asset '{}' is larger than {MAX_SIZE} bytes",
                self.name
            )));
        }
        Ok(())
    }
}

/// What the manifest describes: one release and its downloads per platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// `MAJOR.MINOR.PATCH`; pre-release versions are refused, since they are never offered.
    pub version: String,
    /// Publication date, `YYYY-MM-DD` (UTC), a real calendar date.
    pub published: String,
    /// `(platform, asset)` pairs, for example `("macos-arm64", …)`, written in this order.
    pub platforms: Vec<(String, Asset)>,
}

/// Errors from building, signing or verifying a manifest.
///
/// None of them holds text taken from a secret key or a password.
#[derive(Debug)]
pub enum Error {
    /// An input did not have the expected form.
    Invalid(String),
    /// The minisign library refused a key or a signature, or a signature did not match.
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

/// minisign's messages about signatures and public keys (they name key ids, which are
/// public). Secret keys go through [`key_error`] instead.
impl From<minisign::PError> for Error {
    fn from(error: minisign::PError) -> Self {
        Self::Signature(error.to_string())
    }
}

/// A problem with the secret key or its password, described without any text taken from the
/// key, the password or the library's own message, so that nothing secret can reach a log.
fn key_error(error: &minisign::PError) -> Error {
    let reason = match error.kind() {
        minisign::ErrorKind::Verify => "the password is wrong, or the key is corrupt",
        minisign::ErrorKind::KDF => "its password derivation parameters are not usable",
        _ => "it is malformed, or its password protection does not match the password given",
    };
    Error::Signature(format!("the secret key cannot be used: {reason}"))
}

/// Whether `version` is a plain `MAJOR.MINOR.PATCH` release version: three numbers of at
/// most nine digits, without leading zeros.
pub fn is_release_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 9
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
        })
}

fn release_numbers(version: &str) -> Result<[u32; 3], Error> {
    let not_a_version = || {
        Error::Invalid(format!(
            "version '{version}' is not MAJOR.MINOR.PATCH (pre-releases are never offered as updates)"
        ))
    };
    if !is_release_version(version) {
        return Err(not_a_version());
    }
    let mut numbers = [0; 3];
    for (number, part) in numbers.iter_mut().zip(version.split('.')) {
        *number = part.parse().map_err(|_| not_a_version())?;
    }
    Ok(numbers)
}

/// Whether release version `candidate` is strictly newer than `current`, comparing
/// `MAJOR.MINOR.PATCH` numerically (`0.10.0` is newer than `0.9.0`; a text comparison says
/// the opposite). Equal and older versions are not newer. Fails closed: if either is not a
/// plain release version the answer is an error, never "newer".
pub fn is_newer(candidate: &str, current: &str) -> Result<bool, Error> {
    Ok(release_numbers(candidate)? > release_numbers(current)?)
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
    let year: u32 = date[0..4].parse().unwrap_or(0);
    let month: u32 = date[5..7].parse().unwrap_or(0);
    let day: u32 = date[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day)
}

/// The number of days in `month` (1 to 12) of `year`, in the Gregorian calendar.
fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        _ => 28,
    }
}

fn validate_platform(platform: &str) -> Result<(), Error> {
    let platform_ok = !platform.is_empty()
        && platform.len() <= MAX_PLATFORM_LEN
        && platform
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'));
    if platform_ok {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "platform '{platform}' may only hold lower-case letters, digits, '-' and '_' (at most {MAX_PLATFORM_LEN})"
        )))
    }
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
            "publication date '{}' is not a calendar date YYYY-MM-DD",
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
        validate_platform(platform)?;
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

/// Reads back a manifest written by [`manifest_json`], and refuses anything that is not
/// byte for byte what [`manifest_json`] writes for the values read: another layout, a
/// second `version` key, text after the closing brace, CRLF line ends, an oversized file.
fn parse_canonical(manifest: &[u8]) -> Result<Release, Error> {
    let bad = || {
        Error::Invalid(format!(
            "the manifest is not exactly what mercury-release update-manifest writes ({PRODUCT})"
        ))
    };
    if manifest.len() > MAX_MANIFEST_BYTES {
        return Err(Error::Invalid(format!(
            "the manifest is larger than {MAX_MANIFEST_BYTES} bytes"
        )));
    }
    let text = std::str::from_utf8(manifest).map_err(|_| bad())?;
    let lines: Vec<&str> = text.split('\n').collect();
    // `{`, product, version, published, `"platforms": {`, five lines per platform, `}` of the
    // platforms, `}`, and the empty piece after the final newline.
    if lines.len() < 13 || !(lines.len() - 8).is_multiple_of(5) {
        return Err(bad());
    }
    let field = |line: &str, prefix: &str, suffix: &str| {
        line.strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(suffix))
            .map(str::to_owned)
    };
    let version = field(lines[2], "  \"version\": \"", "\",").ok_or_else(bad)?;
    let published = field(lines[3], "  \"published\": \"", "\",").ok_or_else(bad)?;
    let mut platforms = Vec::new();
    for block in lines[5..lines.len() - 3].chunks(5) {
        let platform = field(block[0], "    \"", "\": {").ok_or_else(bad)?;
        let name = field(block[1], "      \"asset\": \"", "\",").ok_or_else(bad)?;
        let sha256 = field(block[2], "      \"sha256\": \"", "\",").ok_or_else(bad)?;
        let size = block[3]
            .strip_prefix("      \"size\": ")
            .and_then(|size| size.parse::<u64>().ok())
            .ok_or_else(bad)?;
        platforms.push((platform, Asset { name, sha256, size }));
    }
    let release = Release {
        version,
        published,
        platforms,
    };
    // The values were only cut out of the text above; this is what makes it strict.
    if manifest_json(&release)?.as_bytes() != manifest {
        return Err(bad());
    }
    Ok(release)
}

/// Signs `manifest` for `version` with a minisign secret key (the text of a `.key` file) and
/// its password (`None` for a key stored without one, as the tests use; the release key is
/// always password-protected, and the command-line tool refuses a missing password). Returns
/// the text of the `.minisig` file.
///
/// The manifest must be byte for byte one [`manifest_json`] wrote for `version`, so the signed
/// version and the manifest's own never disagree and nothing else hides in the file. The
/// signature is checked against the key's own public half before it is returned. This never
/// prompts: a password-protected key with `None`, an unprotected key with a password, and an
/// empty password are all errors, and no error holds text from the key or the password.
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
    let release = parse_canonical(manifest)?;
    if release.version != version {
        return Err(Error::Invalid(format!(
            "the manifest is for version {}, not {version}",
            release.version
        )));
    }
    if password.as_deref() == Some("") {
        return Err(Error::Invalid("the secret key's password is empty".into()));
    }
    let key_box =
        minisign::SecretKeyBox::from_string(secret_key.trim()).map_err(|e| key_error(&e))?;
    let secret = match password {
        Some(password) => key_box.into_secret_key(Some(password)),
        None => key_box.into_unencrypted_secret_key(),
    }
    .map_err(|e| key_error(&e))?;
    let public = minisign::PublicKey::from_secret_key(&secret).map_err(|e| key_error(&e))?;
    let signature = minisign::sign(
        Some(&public),
        &secret,
        Cursor::new(manifest),
        Some(&trusted_comment(version)),
        Some(&format!("{PRODUCT} update manifest")),
    )?;
    Ok(signature.into_string())
}

/// Whether `manifest` starts the way [`manifest_json`] writes one for `version`.
fn manifest_names_version(manifest: &[u8], version: &str) -> bool {
    let head = format!("{{\n  \"product\": \"{PRODUCT}\",\n  \"version\": \"{version}\",\n");
    manifest.starts_with(head.as_bytes())
}

/// Checks `signature` (the text of a `.minisig` file) over `manifest` with `public_key` (the
/// text of a `.pub` file, or its base64 line) and returns the version it was signed for.
///
/// This is the check an update client makes before it believes anything in the manifest, and
/// it fails closed: any error means the manifest must be ignored. It succeeds only when the
/// signature is a current (prehashed) minisign signature made by that key, its trusted comment
/// (which the signature covers) is exactly `mercury-browser update <MAJOR.MINOR.PATCH>`, and
/// the manifest names the same product and version.
///
/// It does not know the installed version, so a replayed older manifest passes it: the caller
/// must compare the returned version with its own and refuse one that is equal or older.
/// [`verify_update`] does that, and is what an update check should call.
pub fn verify_signed_manifest(
    manifest: &[u8],
    signature: &str,
    public_key: &str,
) -> Result<String, Error> {
    if manifest.len() > MAX_MANIFEST_BYTES {
        return Err(Error::Invalid(format!(
            "the manifest is larger than {MAX_MANIFEST_BYTES} bytes"
        )));
    }
    if signature.len() > MAX_TEXT_BYTES || public_key.len() > MAX_TEXT_BYTES {
        return Err(Error::Invalid(
            "the signature or the public key is far longer than a real one".into(),
        ));
    }
    let public = parse_public_key(public_key)?;
    let signature = minisign::SignatureBox::from_string(signature)?;
    // quiet, no output, legacy (non-prehashed) signatures refused.
    minisign::verify(
        &public,
        &signature,
        Cursor::new(manifest),
        true,
        false,
        false,
    )?;
    let comment = signature.trusted_comment()?;
    let version = comment
        .strip_prefix(&trusted_comment(""))
        .filter(|version| is_release_version(version))
        .ok_or_else(|| {
            Error::Signature(format!(
                "trusted comment '{comment}' is not '{}<version>'",
                trusted_comment("")
            ))
        })?;
    if !manifest_names_version(manifest, version) {
        return Err(Error::Signature(format!(
            "the signed manifest does not name {PRODUCT} {version}, as its trusted comment does"
        )));
    }
    Ok(version.to_owned())
}

/// [`verify_signed_manifest`], and the signed version must be `version`.
pub fn verify_manifest(
    manifest: &[u8],
    signature: &str,
    public_key: &str,
    version: &str,
) -> Result<(), Error> {
    let signed = verify_signed_manifest(manifest, signature, public_key)?;
    if signed != version {
        return Err(Error::Signature(format!(
            "the manifest is signed for version {signed}, expected {version}"
        )));
    }
    Ok(())
}

/// What an update check calls: [`verify_signed_manifest`], and the signed version must be
/// strictly newer than `current`, the version of the running browser ([`is_newer`]). Returns
/// the signed version. An equal or older version (a replayed old manifest, or no update) is an
/// error like any other, and means the manifest is to be ignored.
pub fn verify_update(
    manifest: &[u8],
    signature: &str,
    public_key: &str,
    current: &str,
) -> Result<String, Error> {
    let signed = verify_signed_manifest(manifest, signature, public_key)?;
    if !is_newer(&signed, current)? {
        return Err(Error::Invalid(format!(
            "version {signed} is not newer than the installed {current}"
        )));
    }
    Ok(signed)
}

fn parse_public_key(text: &str) -> Result<minisign::PublicKey, Error> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    match lines.as_slice() {
        [base64] => Ok(minisign::PublicKey::from_base64(base64)?),
        [comment, base64] if comment.starts_with("untrusted comment:") => {
            Ok(minisign::PublicKey::from_base64(base64)?)
        }
        _ => Err(Error::Invalid(
            "the public key is neither a minisign .pub file nor its base64 line".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    const DIGEST_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const DIGEST_B: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
    const PASSWORD: &str = "correct horse battery staple";

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

    /// A fresh key pair for each test, never the release key, stored without a password.
    fn key_pair() -> (String, String) {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let public = pair.pk.to_box().unwrap().into_string();
        let secret = pair.sk.to_box(None).unwrap().into_string();
        (public, secret)
    }

    /// One password-protected key pair (public key text, secret key text) shared by the tests
    /// that need one. minisign derives its key with scrypt at 32 MiB, about 0.1 s.
    fn protected_key_pair() -> &'static (String, String) {
        static PAIR: OnceLock<(String, String)> = OnceLock::new();
        PAIR.get_or_init(|| {
            let pair =
                minisign::KeyPair::generate_encrypted_keypair(Some(PASSWORD.to_owned())).unwrap();
            (
                pair.pk.to_box().unwrap().into_string(),
                pair.sk.to_box(None).unwrap().into_string(),
            )
        })
    }

    /// Signs with minisign directly, so the trusted comment can be anything.
    fn sign_raw(manifest: &str, secret: &str, comment: &str) -> String {
        let secret = minisign::SecretKeyBox::from_string(secret)
            .unwrap()
            .into_unencrypted_secret_key()
            .unwrap();
        minisign::sign(None, &secret, Cursor::new(manifest), Some(comment), None)
            .unwrap()
            .into_string()
    }

    /// A manifest for `version` and its signature by a fresh key: `(manifest, signature, public)`.
    fn signed(version: &str) -> (String, String, String) {
        let (public, secret) = key_pair();
        let mut release = release();
        release.version = version.into();
        let manifest = manifest_json(&release).unwrap();
        let signature = sign_manifest(manifest.as_bytes(), version, &secret, None).unwrap();
        (manifest, signature, public)
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
        for version in [
            "0.3.0-rc.1",
            "0.3",
            "0.3.0.1",
            "v0.3.0",
            "0.03.0",
            "0.3.0 ",
            "",
            "1234567890.0.0",
            "0.3.\"0",
        ] {
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
        bad.platforms[0].1.sha256 = DIGEST_A[1..].into();
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
    fn manifest_values_never_need_json_escaping() {
        // Every string that reaches the JSON is refused if it holds a quote, a backslash, a
        // control character, a newline or any non-ASCII text.
        for hostile in [
            "a\"b", "a\\b", "a\nb", "a\tb", "a b", "a/b", "ä", "a\u{0}b", "a,b",
        ] {
            let mut bad = release();
            bad.platforms[0].1.name = format!("{hostile}.dmg");
            assert!(manifest_json(&bad).is_err(), "asset name {hostile:?}");

            let mut bad = release();
            bad.platforms[0].0 = hostile.into();
            assert!(manifest_json(&bad).is_err(), "platform {hostile:?}");

            let mut bad = release();
            bad.platforms[0].1.sha256 = format!("{hostile}{}", &DIGEST_A[hostile.len()..]);
            assert!(manifest_json(&bad).is_err(), "digest {hostile:?}");

            let mut bad = release();
            bad.published = format!("2026-12-{hostile}");
            assert!(manifest_json(&bad).is_err(), "date {hostile:?}");
        }
        for bad_name in ["", ".dmg-hidden", "..", ".", &"a".repeat(MAX_NAME_LEN + 1)] {
            let mut bad = release();
            bad.platforms[0].1.name = bad_name.into();
            assert!(manifest_json(&bad).is_err(), "asset name {bad_name:?}");
        }
        for bad_platform in ["", "Macos", "mac os", &"a".repeat(MAX_PLATFORM_LEN + 1)] {
            let mut bad = release();
            bad.platforms[0].0 = bad_platform.into();
            assert!(manifest_json(&bad).is_err(), "platform {bad_platform:?}");
        }
        let mut bad = release();
        bad.platforms[0].1.size = 0;
        assert!(manifest_json(&bad).is_err());
        let mut bad = release();
        bad.platforms[0].1.size = MAX_SIZE + 1;
        assert!(manifest_json(&bad).is_err());
        let mut fine = release();
        fine.platforms[0].1.size = MAX_SIZE;
        assert!(manifest_json(&fine).is_ok());
    }

    #[test]
    fn dates_are_real_calendar_dates() {
        for good in [
            "2024-02-29",
            "2000-02-29",
            "2026-02-28",
            "2026-04-30",
            "2026-12-31",
            "2026-01-01",
        ] {
            assert!(is_date(good), "refused {good}");
        }
        for bad in [
            "2026-02-29",
            "2026-02-30",
            "2026-02-31",
            "2026-04-31",
            "2026-06-31",
            "2026-09-31",
            "2026-11-31",
            "2025-02-29",
            "2100-02-29",
            "1900-02-29",
            "2026-00-10",
            "2026-01-00",
            "2026-13-01",
            "2026-1-01",
            "2026/01/01",
            "20260101",
            "2026-01-011",
            " 2026-01-01",
            "2026-01-01 ",
            "2026-0a-01",
            "",
        ] {
            assert!(!is_date(bad), "accepted {bad:?}");
        }
    }

    #[test]
    fn the_manifest_refuses_impossible_dates() {
        for impossible in ["2026-02-29", "2026-02-31", "2026-04-31"] {
            let mut bad = release();
            bad.published = impossible.into();
            assert!(manifest_json(&bad).is_err(), "accepted {impossible}");
        }
        let mut leap = release();
        leap.published = "2024-02-29".into();
        assert!(manifest_json(&leap).is_ok());
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
        assert!(Asset::parse(&format!("a.dmg:{DIGEST_A}:-1")).is_err());
        assert!(Asset::parse(&format!("../a.dmg:{DIGEST_A}:7")).is_err());
        assert!(Asset::parse("").is_err());
    }

    #[test]
    fn versions_compare_numerically() {
        for (candidate, current) in [
            ("0.10.0", "0.9.0"),
            ("0.3.1", "0.3.0"),
            ("0.4.0", "0.3.9"),
            ("1.0.0", "0.99.99"),
            ("0.0.10", "0.0.9"),
            ("999999999.0.0", "0.0.0"),
        ] {
            assert!(
                is_newer(candidate, current).unwrap(),
                "{candidate} > {current}"
            );
            assert!(
                !is_newer(current, candidate).unwrap(),
                "{current} < {candidate}"
            );
        }
        assert!(!is_newer("0.3.0", "0.3.0").unwrap());
        // As text, "0.9.0" sorts after "0.10.0"; the numeric comparison is what matters.
        assert!("0.9.0" > "0.10.0");
        assert!(is_newer("0.10.0", "0.9.0").unwrap());
        for (candidate, current) in [
            ("0.3.0-rc.1", "0.2.0"),
            ("0.3.0", "0.2.0-rc.1"),
            ("0.3", "0.2.0"),
            ("v0.3.0", "0.2.0"),
            ("0.03.0", "0.2.0"),
            ("", "0.2.0"),
            ("0.3.0", ""),
            ("1000000000.0.0", "0.2.0"),
            ("99999999999999999999.0.0", "0.0.0"),
        ] {
            assert!(
                is_newer(candidate, current).is_err(),
                "{candidate:?} against {current:?} was not refused"
            );
        }
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

        // One flipped bit anywhere in the manifest is enough.
        for index in [0, manifest.len() / 2, manifest.len() - 1] {
            let mut bytes = manifest.clone().into_bytes();
            bytes[index] ^= 1;
            assert!(verify_signed_manifest(&bytes, &signature, &public).is_err());
        }
        // So is a missing or an added byte.
        assert!(verify_signed_manifest(&manifest.as_bytes()[1..], &signature, &public).is_err());
        let longer = format!("{manifest} ");
        assert!(verify_signed_manifest(longer.as_bytes(), &signature, &public).is_err());
    }

    #[test]
    fn the_global_signature_covers_the_trusted_comment() {
        let (public, secret) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        let signature = sign_manifest(manifest.as_bytes(), "0.3.0", &secret, None).unwrap();
        // Another comment that would otherwise be acceptable, with the old global signature.
        let swapped = signature.replace("update 0.3.0", "update 0.9.9");
        assert_ne!(swapped, signature);
        let error = verify_signed_manifest(manifest.as_bytes(), &swapped, &public)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Comment signature verification failed"),
            "{error}"
        );
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
        let error = verify_manifest(manifest.as_bytes(), &signature, &other_public, "0.3.0")
            .unwrap_err()
            .to_string();
        assert!(error.contains("is different from public key"), "{error}");
    }

    #[test]
    fn legacy_signatures_are_refused() {
        let (manifest, signature, public) = signed("0.3.0");
        // A legacy (non-prehashed, algorithm "Ed") signature starts "RW"; the current one
        // (algorithm "ED") starts "RU". Only that one is accepted.
        assert!(signature.contains("\nRU"));
        let legacy = signature.replacen("\nRU", "\nRW", 1);
        let error = verify_signed_manifest(manifest.as_bytes(), &legacy, &public)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Legacy signatures are not accepted"),
            "{error}"
        );
    }

    #[test]
    fn signing_refuses_a_malformed_key_or_version() {
        let (_, secret) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        assert!(sign_manifest(manifest.as_bytes(), "0.3.0", "not a key", None).is_err());
        assert!(sign_manifest(manifest.as_bytes(), "0.3.0-rc.1", &secret, None).is_err());
        assert!(sign_manifest(b"{}", "0.3.0", &secret, None).is_err());
    }

    #[test]
    fn signing_refuses_a_manifest_of_another_version() {
        let (_, secret) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        assert!(sign_manifest(manifest.as_bytes(), "0.4.0", &secret, None).is_err());
    }

    #[test]
    fn parsing_reads_back_what_the_writer_wrote() {
        let both = release();
        let json = manifest_json(&both).unwrap();
        assert_eq!(parse_canonical(json.as_bytes()).unwrap(), both);

        let mut one = release();
        one.platforms.truncate(1);
        let json = manifest_json(&one).unwrap();
        assert_eq!(parse_canonical(json.as_bytes()).unwrap(), one);
    }

    #[test]
    fn signing_refuses_a_manifest_that_is_not_exactly_the_written_one() {
        let (_, secret) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        // The head names 0.3.0, but a second "version" key follows, which a JSON parser that
        // keeps the last duplicate would read as 0.1.0.
        let second_version = manifest.replace(
            "  \"platforms\"",
            "  \"version\": \"0.1.0\",\n  \"platforms\"",
        );
        let extra_key = manifest.replace("  \"published\"", "  \"notes\": \"x\",\n  \"published\"");
        let trailing = format!("{manifest}{{\"version\": \"0.1.0\"}}\n");
        let crlf = manifest.replace('\n', "\r\n");
        let no_final_newline = manifest.trim_end().to_owned();
        let other_date = manifest.replace("2026-12-01", "2026-02-31");
        let other_digest = manifest.replace(DIGEST_A, &DIGEST_A.to_uppercase());
        let other_product = manifest.replacen(PRODUCT, "other-product", 1);
        let indented = manifest.replace("\n  \"published\"", "\n   \"published\"");
        let oversized = format!("{manifest}{}", " ".repeat(MAX_MANIFEST_BYTES));
        for bad in [
            &second_version,
            &extra_key,
            &trailing,
            &crlf,
            &no_final_newline,
            &other_date,
            &other_digest,
            &other_product,
            &indented,
            &oversized,
            &String::new(),
            &"{}\n".to_owned(),
        ] {
            assert!(
                sign_manifest(bad.as_bytes(), "0.3.0", &secret, None).is_err(),
                "signed {bad:?}"
            );
        }
        assert!(sign_manifest(&[0xff, 0xfe, 0x00], "0.3.0", &secret, None).is_err());
        sign_manifest(manifest.as_bytes(), "0.3.0", &secret, None).unwrap();
    }

    #[test]
    fn verification_returns_the_signed_version() {
        let (manifest, signature, public) = signed("0.3.0");
        assert_eq!(
            verify_signed_manifest(manifest.as_bytes(), &signature, &public).unwrap(),
            "0.3.0"
        );
    }

    #[test]
    fn an_update_must_be_newer_than_the_installed_version() {
        let (manifest, signature, public) = signed("0.10.0");
        let verify =
            |current: &str| verify_update(manifest.as_bytes(), &signature, &public, current);
        assert_eq!(verify("0.9.0").unwrap(), "0.10.0");
        assert_eq!(verify("0.2.0").unwrap(), "0.10.0");
        assert_eq!(verify("0.9.99").unwrap(), "0.10.0");
        for current in ["0.10.0", "0.10.1", "0.11.0", "1.0.0"] {
            assert!(verify(current).is_err(), "offered 0.10.0 over {current}");
        }
        // An installed version that is not a plain release version is an error too.
        for current in ["", "0.9", "0.9.0-rc.1", "latest"] {
            assert!(verify(current).is_err(), "offered over {current:?}");
        }
        // A replayed older manifest, correctly signed, is not an update.
        let (old_manifest, old_signature, old_public) = signed("0.2.0");
        assert!(
            verify_update(
                old_manifest.as_bytes(),
                &old_signature,
                &old_public,
                "0.3.0"
            )
            .is_err()
        );
        // And a bad signature never gets as far as the version.
        let other = key_pair().0;
        assert!(verify_update(manifest.as_bytes(), &signature, &other, "0.2.0").is_err());
    }

    #[test]
    fn a_valid_signature_with_a_foreign_trusted_comment_is_refused() {
        let (public, secret) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        for comment in [
            "timestamp:1234\tfile:update.json",
            "mercury-browser update ",
            "mercury-browser update 0.3.0-rc.1",
            "mercury-browser update 0.3.0 and more",
            "mercury-browser update  0.3.0",
            "mercury-browser update 0.03.0",
            "other-product update 0.3.0",
            "",
        ] {
            let signature = sign_raw(&manifest, &secret, comment);
            assert!(
                verify_signed_manifest(manifest.as_bytes(), &signature, &public).is_err(),
                "accepted the trusted comment {comment:?}"
            );
        }
        // The exact comment, signed by the same route, is accepted (so the refusals above are
        // about the comment).
        let signature = sign_raw(&manifest, &secret, "mercury-browser update 0.3.0");
        assert_eq!(
            verify_signed_manifest(manifest.as_bytes(), &signature, &public).unwrap(),
            "0.3.0"
        );
    }

    #[test]
    fn a_comment_that_disagrees_with_the_manifest_is_refused() {
        let (public, secret) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        let signature = sign_raw(&manifest, &secret, "mercury-browser update 0.4.0");
        assert!(verify_signed_manifest(manifest.as_bytes(), &signature, &public).is_err());
        assert!(verify_manifest(manifest.as_bytes(), &signature, &public, "0.4.0").is_err());
    }

    #[test]
    fn malformed_signatures_and_keys_fail_closed() {
        let (public, secret) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        let signature = sign_manifest(manifest.as_bytes(), "0.3.0", &secret, None).unwrap();
        let lines: Vec<&str> = signature.lines().collect();

        let without_trusted_comment = lines[..2].join("\n");
        let without_global = lines[..3].join("\n");
        let truncated = &signature[..signature.len() / 2];
        let mut bad_global = lines.clone();
        bad_global[3] = lines[1];
        let bad_global = bad_global.join("\n");
        for bad in [
            "",
            "garbage",
            truncated,
            &without_trusted_comment,
            &without_global,
            &bad_global,
        ] {
            assert!(
                verify_signed_manifest(manifest.as_bytes(), bad, &public).is_err(),
                "accepted the signature {bad:?}"
            );
        }
        for bad_key in ["", "garbage", "untrusted comment: x\nAAAA", &secret] {
            assert!(
                verify_signed_manifest(manifest.as_bytes(), &signature, bad_key).is_err(),
                "accepted the public key {bad_key:?}"
            );
        }
        assert!(verify_signed_manifest(b"", &signature, &public).is_err());
    }

    #[test]
    fn degenerate_keys_and_signatures_are_refused_without_panicking() {
        let (public, secret) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        let signature = sign_manifest(manifest.as_bytes(), "0.3.0", &secret, None).unwrap();
        // 56 characters of valid base64 that decode to 42 bytes of nothing useful.
        for filler in ["A", "/", "+", "B"] {
            let key = filler.repeat(56);
            assert!(
                verify_signed_manifest(manifest.as_bytes(), &signature, &key).is_err(),
                "accepted the key {key}"
            );
            let boxed = format!("untrusted comment: x\n{key}\n");
            assert!(verify_signed_manifest(manifest.as_bytes(), &signature, &boxed).is_err());
        }
        let zeros = format!(
            "untrusted comment: x\n{}=\ntrusted comment: mercury-browser update 0.3.0\n{}\n",
            "A".repeat(99),
            "A".repeat(86) + "=="
        );
        assert!(verify_signed_manifest(manifest.as_bytes(), &zeros, &public).is_err());
    }

    #[test]
    fn oversized_inputs_are_refused_before_any_work() {
        let (manifest, signature, public) = signed("0.3.0");
        let huge = vec![b' '; MAX_MANIFEST_BYTES + 1];
        assert!(verify_signed_manifest(&huge, &signature, &public).is_err());
        let long_signature = format!("{signature}{}", "x".repeat(MAX_TEXT_BYTES));
        assert!(verify_signed_manifest(manifest.as_bytes(), &long_signature, &public).is_err());
        let long_key = format!("{public}{}", "x".repeat(MAX_TEXT_BYTES));
        assert!(verify_signed_manifest(manifest.as_bytes(), &signature, &long_key).is_err());
    }

    #[test]
    fn a_password_protected_key_signs_and_verifies() {
        let (public, secret) = protected_key_pair();
        let manifest = manifest_json(&release()).unwrap();
        let signature =
            sign_manifest(manifest.as_bytes(), "0.3.0", secret, Some(PASSWORD.into())).unwrap();
        assert_eq!(
            verify_signed_manifest(manifest.as_bytes(), &signature, public).unwrap(),
            "0.3.0"
        );
        // Surrounding whitespace, as a pasted secret has, makes no difference.
        let pasted = format!("\n  {secret}\n\n");
        sign_manifest(manifest.as_bytes(), "0.3.0", &pasted, Some(PASSWORD.into())).unwrap();
    }

    #[test]
    fn signing_never_prompts_and_refuses_a_mismatched_password() {
        let (_, protected) = protected_key_pair();
        let (_, unprotected) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        let sign = |key: &str, password: Option<&str>| {
            sign_manifest(
                manifest.as_bytes(),
                "0.3.0",
                key,
                password.map(str::to_owned),
            )
        };
        // A protected key without a password would make minisign prompt on the terminal.
        assert!(sign(protected, None).is_err());
        assert!(sign(protected, Some("wrong")).is_err());
        assert!(sign(protected, Some("")).is_err());
        assert!(sign(&unprotected, Some(PASSWORD)).is_err());
        assert!(sign(&unprotected, Some("")).is_err());
        assert!(sign(&unprotected, None).is_ok());
    }

    #[test]
    fn no_error_holds_text_from_a_key_or_a_password() {
        const WORD: &str = "SENTINEL-8c1f3a-secret";
        let (_, protected) = protected_key_pair();
        let (_, unprotected) = key_pair();
        let manifest = manifest_json(&release()).unwrap();
        let secrets = [
            WORD,
            protected.lines().nth(1).unwrap(),
            unprotected.lines().nth(1).unwrap(),
            PASSWORD,
        ];
        let cases: Vec<(String, Option<String>)> = vec![
            (WORD.into(), Some(WORD.into())),
            (format!("untrusted comment: x\n{WORD}\n"), Some(WORD.into())),
            (format!("untrusted comment: {WORD}\n"), Some(WORD.into())),
            (unprotected.clone(), Some(WORD.into())),
            (unprotected.clone(), Some(String::new())),
            (protected.clone(), Some(WORD.into())),
            (protected.clone(), None),
            (
                protected.replacen('\n', &format!("\n{WORD}"), 1),
                Some(PASSWORD.into()),
            ),
        ];
        for (key, password) in cases {
            let error = sign_manifest(manifest.as_bytes(), "0.3.0", &key, password).unwrap_err();
            let shown = format!("{error} / {error:?}");
            for secret in secrets {
                assert!(!shown.contains(secret), "{shown:?} holds {secret:?}");
            }
        }
        let wrong = sign_manifest(manifest.as_bytes(), "0.3.0", protected, Some(WORD.into()))
            .unwrap_err()
            .to_string();
        assert!(wrong.contains("the password is wrong"), "{wrong}");
    }
}
