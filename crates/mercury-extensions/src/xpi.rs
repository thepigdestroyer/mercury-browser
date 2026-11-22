// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Firefox's add-on signature check for `.xpi` packages, written fresh from the documented behaviour of
//! Firefox (the file layout of `META-INF/`, the COSE design of Mozilla bug 1792954 and the observable
//! rules of `security/manager/ssl/AppSignatureVerification.cpp` and `AppTrustDomain.cpp`, read for the
//! rules only: no code is copied, so this file stays PolyForm).
//!
//! [`verify_xpi`] returns a [`VerifiedXpi`], the only way to read the files of a signed `.xpi`: it owns
//! the verified bytes and [`VerifiedXpi::open`] opens the archive. Nothing is trusted before it verifies.
//!
//! # What is checked
//!
//! * **COSE** (`META-INF/cose.sig` over `META-INF/cose.manifest`): a COSE_Sign (tagged or not) whose
//!   signatures use ES256, ES384 or ES512 (the algorithms Firefox's verifier accepts; PS256 is not one of
//!   them). Mozilla's `cose-rust` puts the signer's certificate in the signature's protected `kid` (label
//!   4) and the intermediates in the body's protected header (label 4, an array); the standard `x5chain`
//!   (label 33) is read too, in any of the four header maps. The signature covers the COSE
//!   `Sig_structure` with the detached payload `cose.manifest`. Mozilla's signer writes the empty
//!   `external_aad` of that structure as CBOR `null` (not the empty byte string of RFC 9052); the
//!   real add-on test pins it, and the standard encoding is accepted too. The headers do not go
//!   through `coset`'s `Header` type, which cannot hold Mozilla's `kid` array; `coset` supplies the
//!   CBOR codec and nothing else.
//! * **PKCS#7** (`META-INF/*.rsa`, `*.sf`, `manifest.mf`): `SignedData` with an RSA (or ECDSA) signature
//!   over `*.sf` (directly or over its signed attributes), `*.sf` carrying the digest of `manifest.mf`,
//!   and `manifest.mf` carrying a digest of every entry. SHA-256 is preferred; SHA-1 is accepted here and
//!   only here, as Firefox does for old add-ons.
//! * **Coverage**, as Firefox: every entry that is not a directory and not one of the signature files must
//!   be listed in the manifest with a matching SHA-256 digest (PKCS#7: the digest the signature uses); a
//!   listed entry that is missing, a listed signature file, an unlisted entry, a duplicate `Name`, a
//!   `Magic` attribute or a missing digest is an error. MD5 and other digests are never read. The
//!   manifests are parsed with Firefox's rules (72-byte physical lines, continuation lines, CR, LF or CRLF)
//!   and may not contain a NUL byte.
//! * **The chain**: the signer certificate must chain to a trust anchor of [`TrustAnchors`] through the
//!   certificates in the signature and the bundled intermediates: signatures verified with `aws-lc-rs`
//!   (RSA PKCS#1 2048 to 8192 bits and ECDSA P-256, P-384, P-521 with SHA-256, SHA-384 or SHA-512; SHA-1
//!   and RSA-PSS certificate signatures are refused), `basicConstraints` (CA, path length), key usage,
//!   extended key usage (code signing or any), no unknown critical extension, at most
//!   [`Limits::max_certificates`] certificates read from a signature.
//! * **The signer**: the common name of the signer certificate equals the add-on id (from
//!   `browser_specific_settings.gecko.id` or `applications.gecko.id` in `manifest.json`; the SHA-256 hex
//!   digest of the id when the id is longer than 64 characters, as Firefox does). A manifest without an
//!   id takes the certificate's common name if it has the shape of an add-on id, as Firefox does.
//!
//! # Differences from Firefox
//!
//! * **Certificate time validity.** Firefox deliberately ignores expiry and "not yet valid" errors for
//!   add-on signatures (there are no trusted timestamps, and perfectly good add-ons would stop working).
//!   [`ValidityPolicy::Enforce`] (the default of [`XpiPolicy::new`]) checks every certificate at
//!   [`XpiPolicy::now`] and fails closed; [`ValidityPolicy::IgnoreExpiry`] is Firefox's behaviour. The
//!   caller picks: with `Enforce`, a legitimate old add-on whose signing certificate has expired is
//!   refused with [`CertProblem::Expired`].
//! * Every signature present is verified, not only the first (a COSE_Sign with several signatures needs all
//!   of them to be good), and every signature's signer must match the add-on id.
//! * A PKCS#7 signature file (`*.rsa`) that is present must verify even when only COSE is required, and
//!   the same for COSE: a package that has a signature file but fails it is an error, never "unsigned".
//! * Name constraints and certificate policies are not evaluated: a critical `nameConstraints` or
//!   `policyConstraints` extension makes the certificate unusable (fail closed).
//! * Revocation (Firefox's local blocklist) is not consulted; the kill switch is local (decision D7).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use aws_lc_rs::digest::{self, Context as DigestContext};
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_ASN1, ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA384_ASN1,
    ECDSA_P256_SHA512_ASN1, ECDSA_P384_SHA256_ASN1, ECDSA_P384_SHA384_ASN1,
    ECDSA_P384_SHA384_FIXED, ECDSA_P384_SHA512_ASN1, ECDSA_P521_SHA256_ASN1,
    ECDSA_P521_SHA384_ASN1, ECDSA_P521_SHA512_ASN1, ECDSA_P521_SHA512_FIXED, ParsedPublicKey,
    RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY, RSA_PKCS1_2048_8192_SHA256,
    RSA_PKCS1_2048_8192_SHA384, RSA_PKCS1_2048_8192_SHA512, VerificationAlgorithm,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use cms::cert::IssuerAndSerialNumber;
use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier, SignerInfo};
use coset::cbor::Value;
use x509_cert::Certificate;
use x509_cert::der::asn1::{OctetString, PrintableStringRef, TeletexStringRef, Utf8StringRef};
use x509_cert::der::oid::ObjectIdentifier;
use x509_cert::der::{Any, Decode, Encode};
use x509_cert::ext::pkix::{BasicConstraints, ExtendedKeyUsage, KeyUsage};

use crate::archive::{ArchiveError, Limits, ZipPackage};
use crate::roots::{TrustAnchors, TrustedCert};

const COSE_SIG: &str = "META-INF/cose.sig";
const COSE_MANIFEST: &str = "META-INF/cose.manifest";
const MAX_CERT_BYTES: usize = 16 * 1024;
const MAX_CHAIN: usize = 8;
const MAX_COSE_SIGNATURES: usize = 8;
const CBOR_RECURSION_LIMIT: usize = 16;

const OID_BASIC_CONSTRAINTS: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.19");
const OID_KEY_USAGE: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.15");
const OID_EXT_KEY_USAGE: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.37");
const OID_KP_CODE_SIGNING: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.3");
const OID_KP_ANY: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.37.0");
const OID_COMMON_NAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");
const OID_ORG_UNIT: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.11");
const OID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const OID_PKCS7_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const OID_ATTR_CONTENT_TYPE: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const OID_ATTR_MESSAGE_DIGEST: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const OID_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const OID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const OID_RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const OID_SHA1_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.5");
const OID_SHA256_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const OID_SHA384_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
const OID_SHA512_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
const OID_ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const OID_ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const OID_ECDSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.4");

/// Extensions this module understands or may ignore when they are not critical. A critical extension
/// that is not in this list makes the certificate unusable.
const KNOWN_EXTENSIONS: &[&str] = &[
    "2.5.29.19", // basicConstraints
    "2.5.29.15", // keyUsage
    "2.5.29.37", // extKeyUsage
    "2.5.29.14", // subjectKeyIdentifier
    "2.5.29.35", // authorityKeyIdentifier
    "2.5.29.17", // subjectAltName
    "2.5.29.31", // cRLDistributionPoints
    "2.5.29.32", // certificatePolicies
];

/// Which signature the package must carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureRequirement {
    /// COSE is required (a PKCS#7 signature that is also present must still verify).
    Cose,
    /// PKCS#7 is required (a COSE signature that is also present must still verify).
    Pkcs7,
    /// One of them; whichever is present must verify.
    Either,
    /// Manifest version 3 requires COSE; version 2 accepts COSE or PKCS#7. The version is read from the
    /// signed `manifest.json`.
    ByManifestVersion,
}

/// How certificate validity dates are treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidityPolicy {
    /// Every certificate must be valid at [`XpiPolicy::now`].
    Enforce,
    /// Firefox's behaviour: "expired" and "not yet valid" are ignored for add-on signatures.
    IgnoreExpiry,
}

/// What [`verify_xpi`] checks against.
#[derive(Debug, Clone)]
pub struct XpiPolicy {
    /// The trust anchors and known intermediates.
    pub trust: TrustAnchors,
    /// The time certificates are checked at, in seconds since the Unix epoch. Never read from the clock.
    pub now: i64,
    /// The caps.
    pub limits: Limits,
    /// Which signature is required.
    pub require: SignatureRequirement,
    /// Certificate dates: see the module documentation.
    pub validity: ValidityPolicy,
}

impl XpiPolicy {
    /// The strict policy: COSE required, certificate dates enforced, default limits.
    pub fn new(trust: TrustAnchors, now: i64) -> XpiPolicy {
        XpiPolicy {
            trust,
            now,
            limits: Limits::default(),
            require: SignatureRequirement::Cose,
            validity: ValidityPolicy::Enforce,
        }
    }
}

/// The signature kinds a package can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureAlgorithm {
    /// COSE with ECDSA P-256 and SHA-256.
    CoseEs256,
    /// COSE with ECDSA P-384 and SHA-384.
    CoseEs384,
    /// COSE with ECDSA P-521 and SHA-512.
    CoseEs512,
    /// PKCS#7 with a SHA-1 digest (old add-ons only).
    Pkcs7Sha1,
    /// PKCS#7 with a SHA-256 digest.
    Pkcs7Sha256,
}

/// What is wrong with a certificate or a chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CertProblem {
    /// A certificate does not parse, or an extension is malformed or repeated.
    #[error("a certificate is malformed")]
    Malformed,
    /// More certificates than `Limits::max_certificates`.
    #[error("the signature carries too many certificates")]
    TooManyCertificates,
    /// No chain to a trust anchor.
    #[error("the signer certificate does not chain to a trusted root")]
    UnknownIssuer,
    /// A certificate's signature does not verify against its issuer.
    #[error("a certificate signature does not verify")]
    BadCertificateSignature,
    /// A certificate is expired.
    #[error("a certificate in the chain is expired")]
    Expired,
    /// A certificate is not valid yet.
    #[error("a certificate in the chain is not valid yet")]
    NotYetValid,
    /// A certificate is signed with an algorithm that is not accepted (SHA-1, MD5, RSA-PSS, ...).
    #[error("a certificate uses an unsupported or disabled signature algorithm")]
    UnsupportedSignatureAlgorithm,
    /// A public key is too weak or of a type that is not accepted.
    #[error("a certificate has an unacceptable public key")]
    WeakKey,
    /// The signer certificate is a CA certificate.
    #[error("the signer certificate is a CA certificate")]
    NotEndEntity,
    /// An issuer is not a CA certificate.
    #[error("an issuer in the chain is not a CA certificate")]
    NotACa,
    /// The key usage does not allow the certificate's role.
    #[error("a certificate's key usage does not allow its role")]
    KeyUsageNotAllowed,
    /// The extended key usage does not include code signing.
    #[error("a certificate's extended key usage does not allow code signing")]
    ExtendedKeyUsageNotAllowed,
    /// The chain is longer than a path-length constraint or the built-in limit allows.
    #[error("the certificate chain is too long")]
    PathTooLong,
    /// A critical extension that this module does not evaluate.
    #[error("a certificate has a critical extension that is not supported")]
    UnknownCriticalExtension,
}

/// Why an XPI was refused. No variant holds file contents or key bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum XpiError {
    /// The archive itself is not acceptable.
    #[error("the archive is not acceptable: {0}")]
    Archive(#[from] ArchiveError),
    /// The package carries no signature at all.
    #[error("the package is not signed")]
    Unsigned,
    /// Some signature files are present but the package does not carry a complete signature.
    #[error("the package carries an incomplete signature")]
    IncompleteSignature,
    /// The policy requires a COSE signature and there is none.
    #[error("the package has no COSE signature")]
    CoseRequired,
    /// The policy requires a PKCS#7 signature and there is none.
    #[error("the package has no PKCS#7 signature")]
    Pkcs7Required,
    /// A signature file is larger than `Limits::max_signature_file_bytes`.
    #[error("a signature file is over the size limit")]
    SignatureFileTooLarge,
    /// `cose.sig` is not a valid COSE_Sign of the shape Mozilla uses.
    #[error("the COSE signature is malformed")]
    MalformedCose,
    /// The PKCS#7 signature is not valid DER or not a `SignedData` of the expected shape.
    #[error("the PKCS#7 signature is malformed")]
    MalformedPkcs7,
    /// A JAR-style manifest or signature file does not parse.
    #[error("a signature manifest is malformed")]
    MalformedManifest,
    /// The signature uses an algorithm that is not accepted.
    #[error("the signature uses an unsupported algorithm")]
    UnsupportedAlgorithm,
    /// A certificate or the chain is not acceptable.
    #[error("certificate problem: {0}")]
    Certificate(#[from] CertProblem),
    /// A signature does not verify.
    #[error("a signature does not verify")]
    BadSignature,
    /// The digest in a signature file does not match the manifest it covers.
    #[error("a manifest does not match the digest in its signature")]
    ManifestDigestMismatch,
    /// An entry's digest differs from the manifest's.
    #[error("an entry does not match its signed digest")]
    ModifiedEntry,
    /// An entry that is not a directory is missing from the manifest.
    #[error("the package has an entry that is not signed")]
    UnsignedEntry,
    /// The manifest lists an entry that is not in the package.
    #[error("the signed manifest lists an entry that is missing")]
    MissingEntry,
    /// `manifest.json` is missing, too large, too deeply nested, not JSON, or has no usable
    /// `manifest_version`.
    #[error("manifest.json is unusable")]
    ManifestUnusable,
    /// The signer's common name is not the add-on id.
    #[error("the signing certificate is not for this add-on id")]
    SignerMismatch,
    /// There is no add-on id in the manifest and the certificate's name is not a valid id.
    #[error("the add-on id is missing or invalid")]
    InvalidAddonId,
}

/// An XPI whose signature, chain and coverage were verified. It owns the verified bytes.
#[derive(Clone)]
#[non_exhaustive]
pub struct VerifiedXpi {
    /// The add-on id.
    pub addon_id: String,
    /// The signature kind of the primary signature: COSE if the package has one, else PKCS#7.
    pub algorithm: SignatureAlgorithm,
    /// Every signature kind that was verified (COSE first).
    pub algorithms: Vec<SignatureAlgorithm>,
    /// The number of certificates in the primary signer's chain, the signer's and the root included.
    pub signer_chain_len: usize,
    /// The common name of the signer certificate.
    pub signer_common_name: String,
    /// The organizational unit of the signer certificate, if any (Mozilla marks `Mozilla Components`,
    /// `Mozilla Extensions` and preliminary reviews with it).
    pub signer_organizational_unit: Option<String>,
    /// `manifest_version` of the signed `manifest.json`.
    pub manifest_version: u32,
    /// SHA-256 of the whole file.
    pub sha256: [u8; 32],
    bytes: Arc<[u8]>,
}

impl std::fmt::Debug for VerifiedXpi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedXpi")
            .field("addon_id", &self.addon_id)
            .field("algorithm", &self.algorithm)
            .field("len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl VerifiedXpi {
    /// Opens the archive of the verified file and validates its directory against `limits`.
    pub fn open(&self, limits: &Limits) -> Result<ZipPackage, ArchiveError> {
        ZipPackage::open(Arc::clone(&self.bytes), limits)
    }

    /// The verified bytes of the whole file.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Verifies an XPI. The bytes are copied first, so what is verified is what [`VerifiedXpi`] holds.
pub fn verify_xpi(bytes: &[u8], opts: &XpiPolicy) -> Result<VerifiedXpi, XpiError> {
    verify_xpi_shared(Arc::from(bytes), opts)
}

/// Verifies an XPI held in shared bytes.
pub fn verify_xpi_shared(bytes: Arc<[u8]>, opts: &XpiPolicy) -> Result<VerifiedXpi, XpiError> {
    let package = ZipPackage::open(Arc::clone(&bytes), &opts.limits)?;
    let names: HashSet<&str> = package.file_names().collect();
    let has_cose_sig = names.contains(COSE_SIG);
    let rsa = find_one(&package, META_INF_RSA)?;
    let any_signature_file = has_cose_sig
        || names.contains(COSE_MANIFEST)
        || rsa.is_some()
        || package.file_names().any(is_sf_name);
    if !any_signature_file {
        return Err(XpiError::Unsigned);
    }
    if !has_cose_sig && rsa.is_none() {
        return Err(XpiError::IncompleteSignature);
    }
    match opts.require {
        SignatureRequirement::Cose if !has_cose_sig => return Err(XpiError::CoseRequired),
        SignatureRequirement::Pkcs7 if rsa.is_none() => return Err(XpiError::Pkcs7Required),
        _ => {}
    }

    let anchors = Anchors::new(&opts.trust)?;
    let mut ignored: HashSet<String> = HashSet::new();
    let mut signers: Vec<Signer> = Vec::new();

    if let Some(rsa_name) = &rsa {
        signers.push(verify_pkcs7(
            &package,
            rsa_name,
            &anchors,
            opts,
            &mut ignored,
        )?);
    }
    let mut cose_signers = Vec::new();
    if has_cose_sig {
        cose_signers = verify_cose(&package, &anchors, opts, &mut ignored)?;
    }
    // COSE first: it is the primary signature when present.
    cose_signers.append(&mut signers);
    let signers = cose_signers;
    let primary = signers.first().ok_or(XpiError::IncompleteSignature)?;

    let manifest = read_manifest_json(&package, &opts.limits)?;
    if matches!(opts.require, SignatureRequirement::ByManifestVersion)
        && manifest.version == 3
        && !has_cose_sig
    {
        return Err(XpiError::CoseRequired);
    }

    let addon_id = match &manifest.gecko_id {
        Some(id) => {
            let expected = expected_common_name(id);
            for signer in &signers {
                if !names_equal(&signer.common_name, &expected) {
                    return Err(XpiError::SignerMismatch);
                }
            }
            id.clone()
        }
        None => {
            if !looks_like_addon_id(&primary.common_name) {
                return Err(XpiError::InvalidAddonId);
            }
            for signer in &signers {
                if signer.common_name != primary.common_name {
                    return Err(XpiError::SignerMismatch);
                }
            }
            primary.common_name.clone()
        }
    };

    let sha256: [u8; 32] = digest::digest(&digest::SHA256, &bytes)
        .as_ref()
        .try_into()
        .map_err(|_| XpiError::MalformedManifest)?;
    Ok(VerifiedXpi {
        addon_id,
        algorithm: primary.algorithm,
        algorithms: signers.iter().map(|s| s.algorithm).collect(),
        signer_chain_len: primary.chain_len,
        signer_common_name: primary.common_name.clone(),
        signer_organizational_unit: primary.organizational_unit.clone(),
        manifest_version: manifest.version,
        sha256,
        bytes,
    })
}

struct Signer {
    algorithm: SignatureAlgorithm,
    chain_len: usize,
    common_name: String,
    organizational_unit: Option<String>,
}

fn names_equal(common_name: &str, expected: &str) -> bool {
    common_name == expected || (expected.len() == 64 && common_name.eq_ignore_ascii_case(expected))
}

/// The common name Firefox expects in the signing certificate for an add-on id: the id itself, or, for
/// an id longer than 64 characters, the lowercase hex SHA-256 digest of the id.
pub fn expected_common_name(addon_id: &str) -> String {
    if addon_id.chars().count() > 64 {
        let digest = digest::digest(&digest::SHA256, addon_id.as_bytes());
        digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    } else {
        addon_id.to_string()
    }
}

/// Whether `id` has the shape of a Gecko add-on id: `{uuid}` or `name@domain`.
pub fn looks_like_addon_id(id: &str) -> bool {
    if let Some(inner) = id.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        let groups: Vec<&str> = inner.split('-').collect();
        let lengths = [8usize, 4, 4, 4, 12];
        return groups.len() == 5
            && groups
                .iter()
                .zip(lengths)
                .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()));
    }
    let Some((name, domain)) = id.split_once('@') else {
        return false;
    };
    let ok = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_');
    name.bytes().all(ok) && !domain.is_empty() && domain.bytes().all(ok)
}

// ---------------------------------------------------------------------------------------------
// manifest.json: the id and the manifest version
// ---------------------------------------------------------------------------------------------

struct ManifestInfo {
    version: u32,
    gecko_id: Option<String>,
}

fn json_depth_within(bytes: &[u8], max: usize) -> bool {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &b in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth = depth.saturating_add(1);
                if depth > max {
                    return false;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    true
}

fn read_manifest_json(package: &ZipPackage, limits: &Limits) -> Result<ManifestInfo, XpiError> {
    let bytes = package
        .read_file_capped("manifest.json", limits.max_manifest_bytes)
        .map_err(|_| XpiError::ManifestUnusable)?;
    if !json_depth_within(&bytes, limits.max_json_depth) {
        return Err(XpiError::ManifestUnusable);
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| XpiError::ManifestUnusable)?;
    let version = value
        .get("manifest_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| matches!(v, 2 | 3))
        .ok_or(XpiError::ManifestUnusable)?;
    let gecko_id = ["browser_specific_settings", "applications"]
        .iter()
        .filter_map(|key| value.get(key))
        .filter_map(|v| v.get("gecko"))
        .filter_map(|v| v.get("id"))
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .next();
    if let Some(id) = &gecko_id
        && id.is_empty()
    {
        return Err(XpiError::InvalidAddonId);
    }
    Ok(ManifestInfo { version, gecko_id })
}

// ---------------------------------------------------------------------------------------------
// JAR-style manifests
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum ManifestDigest {
    Sha1,
    Sha256,
}

impl ManifestDigest {
    fn algorithm(self) -> &'static digest::Algorithm {
        match self {
            ManifestDigest::Sha1 => &digest::SHA1_FOR_LEGACY_USE_ONLY,
            ManifestDigest::Sha256 => &digest::SHA256,
        }
    }

    fn len(self) -> usize {
        match self {
            ManifestDigest::Sha1 => 20,
            ManifestDigest::Sha256 => 32,
        }
    }

    fn entry_attribute(self) -> &'static str {
        match self {
            ManifestDigest::Sha1 => "sha1-digest",
            ManifestDigest::Sha256 => "sha256-digest",
        }
    }

    fn manifest_attribute(self) -> &'static str {
        match self {
            ManifestDigest::Sha1 => "sha1-digest-manifest",
            ManifestDigest::Sha256 => "sha256-digest-manifest",
        }
    }
}

struct LineReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

const MAX_PHYSICAL_LINE: usize = 72;
const MAX_LOGICAL_LINE: usize = 65_535;

impl<'a> LineReader<'a> {
    fn at_end(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// One logical line (continuation lines joined), with Firefox's limits.
    fn read_line(&mut self, allow_continuations: bool) -> Result<Vec<u8>, XpiError> {
        let mut line: Vec<u8> = Vec::new();
        loop {
            let rest = self.buf.get(self.pos..).unwrap_or(&[]);
            let end = rest
                .iter()
                .position(|b| *b == b'\r' || *b == b'\n')
                .unwrap_or(rest.len());
            let physical = rest.get(..end).unwrap_or(&[]);
            if physical.len() > MAX_PHYSICAL_LINE {
                return Err(XpiError::MalformedManifest);
            }
            line.extend_from_slice(physical);
            if line.len() > MAX_LOGICAL_LINE {
                return Err(XpiError::MalformedManifest);
            }
            let mut next = self.pos.saturating_add(end);
            if self.buf.get(next) == Some(&b'\r') {
                next = next.saturating_add(1);
            }
            if self.buf.get(next) == Some(&b'\n') {
                next = next.saturating_add(1);
            }
            self.pos = next;
            if self.buf.get(next) != Some(&b' ') {
                return Ok(line);
            }
            if !allow_continuations {
                return Err(XpiError::MalformedManifest);
            }
            self.pos = next.saturating_add(1);
        }
    }
}

fn parse_attribute(line: &[u8]) -> Result<(String, Vec<u8>), XpiError> {
    let colon = line
        .iter()
        .position(|b| *b == b':')
        .ok_or(XpiError::MalformedManifest)?;
    let mut name_end = colon;
    loop {
        if name_end == 0 {
            return Err(XpiError::MalformedManifest);
        }
        if line.get(name_end - 1) != Some(&b' ') {
            break;
        }
        name_end -= 1;
    }
    let name = line.get(..name_end).ok_or(XpiError::MalformedManifest)?;
    let mut value_start = colon + 1;
    while line.get(value_start) == Some(&b' ') {
        value_start += 1;
    }
    let value = line.get(value_start..).unwrap_or(&[]);
    let name = String::from_utf8(name.to_vec()).map_err(|_| XpiError::MalformedManifest)?;
    Ok((name, value.to_vec()))
}

fn expect_version_line(reader: &mut LineReader<'_>, expected: &str) -> Result<(), XpiError> {
    let line = reader.read_line(false)?;
    if line == expected.as_bytes() {
        Ok(())
    } else {
        Err(XpiError::MalformedManifest)
    }
}

/// The digest of the manifest from a `*.sf` file (the main section only).
fn parse_signature_file(buf: &[u8], digest: ManifestDigest) -> Result<Vec<u8>, XpiError> {
    if buf.contains(&0) {
        return Err(XpiError::MalformedManifest);
    }
    let mut reader = LineReader { buf, pos: 0 };
    expect_version_line(&mut reader, "Signature-Version: 1.0")?;
    loop {
        let line = reader.read_line(true)?;
        if line.is_empty() {
            return Err(XpiError::MalformedManifest);
        }
        let (name, value) = parse_attribute(&line)?;
        if name.eq_ignore_ascii_case(digest.manifest_attribute()) {
            return STANDARD
                .decode(&value)
                .map_err(|_| XpiError::MalformedManifest);
        }
    }
}

/// The `Name` → digest map of a manifest. Duplicates, a `Magic` attribute and sections without a name
/// or digest are errors.
fn parse_manifest(
    buf: &[u8],
    digest: ManifestDigest,
) -> Result<HashMap<String, Vec<u8>>, XpiError> {
    if buf.contains(&0) {
        return Err(XpiError::MalformedManifest);
    }
    let mut reader = LineReader { buf, pos: 0 };
    expect_version_line(&mut reader, "Manifest-Version: 1.0")?;
    loop {
        if reader.read_line(true)?.is_empty() {
            break;
        }
    }
    let mut items: HashMap<String, Vec<u8>> = HashMap::new();
    if reader.at_end() {
        return Ok(items);
    }
    let mut name: Vec<u8> = Vec::new();
    let mut digest_bytes: Vec<u8> = Vec::new();
    loop {
        let line = reader.read_line(true)?;
        if line.is_empty() {
            if name.is_empty() || digest_bytes.is_empty() {
                return Err(XpiError::MalformedManifest);
            }
            let key = String::from_utf8(std::mem::take(&mut name))
                .map_err(|_| XpiError::MalformedManifest)?;
            if items
                .insert(key, std::mem::take(&mut digest_bytes))
                .is_some()
            {
                return Err(XpiError::MalformedManifest);
            }
            if reader.at_end() {
                break;
            }
            continue;
        }
        let (attribute, value) = parse_attribute(&line)?;
        if attribute.eq_ignore_ascii_case(digest.entry_attribute()) {
            if !digest_bytes.is_empty() {
                return Err(XpiError::MalformedManifest);
            }
            digest_bytes = STANDARD
                .decode(&value)
                .map_err(|_| XpiError::MalformedManifest)?;
            continue;
        }
        if attribute.eq_ignore_ascii_case("name") {
            if !name.is_empty() || value.is_empty() {
                return Err(XpiError::MalformedManifest);
            }
            name = value;
            continue;
        }
        if attribute.eq_ignore_ascii_case("magic") {
            return Err(XpiError::MalformedManifest);
        }
    }
    Ok(items)
}

/// Every non-directory entry outside `ignored` is listed with a matching digest, and every listed entry
/// exists. Structure is checked before any entry is inflated.
fn check_coverage(
    package: &ZipPackage,
    manifest: &HashMap<String, Vec<u8>>,
    ignored: &HashSet<String>,
    digest: ManifestDigest,
    limits: &Limits,
) -> Result<(), XpiError> {
    for entry in package.entries() {
        if entry.is_dir || ignored.contains(&entry.name) {
            continue;
        }
        if !manifest.contains_key(&entry.name) {
            return Err(XpiError::UnsignedEntry);
        }
    }
    for (name, expected) in manifest {
        if ignored.contains(name) {
            return Err(XpiError::MissingEntry);
        }
        match package.entry(name) {
            Some(entry) if !entry.is_dir => {}
            _ => return Err(XpiError::MissingEntry),
        }
        if expected.len() != digest.len() {
            return Err(XpiError::MalformedManifest);
        }
    }
    for (name, expected) in manifest {
        let mut context = DigestContext::new(digest.algorithm());
        package
            .visit_file(name, limits.max_entry_bytes, &mut |chunk| {
                context.update(chunk);
            })
            .map_err(XpiError::Archive)?;
        if context.finish().as_ref() != expected.as_slice() {
            return Err(XpiError::ModifiedEntry);
        }
    }
    Ok(())
}

fn read_signature_file(
    package: &ZipPackage,
    name: &str,
    limits: &Limits,
) -> Result<Vec<u8>, XpiError> {
    match package.read_file_capped(name, limits.max_signature_file_bytes) {
        Ok(bytes) => Ok(bytes),
        Err(ArchiveError::EntryTooLarge { .. }) => Err(XpiError::SignatureFileTooLarge),
        Err(e) => Err(XpiError::Archive(e)),
    }
}

// ---------------------------------------------------------------------------------------------
// Signature scheme helpers
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scheme {
    Rsa(Hash),
    Ecdsa(Hash),
}

fn scheme_from_oid(oid: &ObjectIdentifier) -> Option<Scheme> {
    Some(match *oid {
        OID_SHA1_RSA => Scheme::Rsa(Hash::Sha1),
        OID_SHA256_RSA => Scheme::Rsa(Hash::Sha256),
        OID_SHA384_RSA => Scheme::Rsa(Hash::Sha384),
        OID_SHA512_RSA => Scheme::Rsa(Hash::Sha512),
        OID_ECDSA_SHA256 => Scheme::Ecdsa(Hash::Sha256),
        OID_ECDSA_SHA384 => Scheme::Ecdsa(Hash::Sha384),
        OID_ECDSA_SHA512 => Scheme::Ecdsa(Hash::Sha512),
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SigFailure {
    KeyRejected,
    Invalid,
}

/// Verifies `signature` over `message` with the key of `spki` (DER SubjectPublicKeyInfo).
fn verify_with_spki(
    spki: &[u8],
    scheme: Scheme,
    message: &[u8],
    signature: &[u8],
) -> Result<(), SigFailure> {
    let candidates: [&'static dyn VerificationAlgorithm; 3] = match scheme {
        Scheme::Rsa(hash) => {
            let one: &'static dyn VerificationAlgorithm = match hash {
                Hash::Sha1 => &RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY,
                Hash::Sha256 => &RSA_PKCS1_2048_8192_SHA256,
                Hash::Sha384 => &RSA_PKCS1_2048_8192_SHA384,
                Hash::Sha512 => &RSA_PKCS1_2048_8192_SHA512,
            };
            [one, one, one]
        }
        Scheme::Ecdsa(Hash::Sha256) => [
            &ECDSA_P256_SHA256_ASN1,
            &ECDSA_P384_SHA256_ASN1,
            &ECDSA_P521_SHA256_ASN1,
        ],
        Scheme::Ecdsa(Hash::Sha384) => [
            &ECDSA_P256_SHA384_ASN1,
            &ECDSA_P384_SHA384_ASN1,
            &ECDSA_P521_SHA384_ASN1,
        ],
        Scheme::Ecdsa(Hash::Sha512) => [
            &ECDSA_P256_SHA512_ASN1,
            &ECDSA_P384_SHA512_ASN1,
            &ECDSA_P521_SHA512_ASN1,
        ],
        Scheme::Ecdsa(Hash::Sha1) => return Err(SigFailure::KeyRejected),
    };
    for algorithm in candidates {
        if let Ok(key) = ParsedPublicKey::new(algorithm, spki) {
            return key
                .verify_sig(message, signature)
                .map_err(|_| SigFailure::Invalid);
        }
    }
    Err(SigFailure::KeyRejected)
}

// ---------------------------------------------------------------------------------------------
// Certificates and chains
// ---------------------------------------------------------------------------------------------

struct Cert {
    der: Vec<u8>,
    parsed: Certificate,
    tbs: Vec<u8>,
    issuer: Vec<u8>,
    subject: Vec<u8>,
    spki: Vec<u8>,
}

impl Cert {
    fn parse(der: &[u8]) -> Result<Cert, CertProblem> {
        if der.len() > MAX_CERT_BYTES {
            return Err(CertProblem::Malformed);
        }
        let parsed = Certificate::from_der(der).map_err(|_| CertProblem::Malformed)?;
        // The signed bytes are the TBSCertificate exactly as they appear in the file, not a
        // re-encoding of the parsed structure.
        let tbs = raw_tbs(der).ok_or(CertProblem::Malformed)?.to_vec();
        let issuer = parsed
            .tbs_certificate()
            .issuer()
            .to_der()
            .map_err(|_| CertProblem::Malformed)?;
        let subject = parsed
            .tbs_certificate()
            .subject()
            .to_der()
            .map_err(|_| CertProblem::Malformed)?;
        let spki = parsed
            .tbs_certificate()
            .subject_public_key_info()
            .to_der()
            .map_err(|_| CertProblem::Malformed)?;
        Ok(Cert {
            der: der.to_vec(),
            parsed,
            tbs,
            issuer,
            subject,
            spki,
        })
    }

    fn from_trusted(trusted: &TrustedCert) -> Result<Cert, CertProblem> {
        Cert::parse(trusted.der())
    }

    fn name_attribute(&self, oid: ObjectIdentifier) -> Result<Vec<String>, CertProblem> {
        let mut found = Vec::new();
        for attribute in self.parsed.tbs_certificate().subject().iter() {
            if attribute.oid == oid {
                found.push(any_to_string(&attribute.value).ok_or(CertProblem::Malformed)?);
            }
        }
        Ok(found)
    }
}

/// One DER element with a single-byte tag: `(tag, content, bytes after the element)`.
fn read_tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    if tag & 0x1f == 0x1f {
        return None;
    }
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 {
            return None;
        }
        let (digits, rest) = rest.split_at_checked(count)?;
        let length = digits.iter().fold(0usize, |acc, d| {
            acc.saturating_mul(256).saturating_add(usize::from(*d))
        });
        (length, rest)
    };
    let (content, after) = rest.split_at_checked(length)?;
    Some((tag, content, after))
}

/// The TBSCertificate of a certificate, as the bytes of the file.
fn raw_tbs(der: &[u8]) -> Option<&[u8]> {
    let (tag, content, _) = read_tlv(der)?;
    if tag != 0x30 {
        return None;
    }
    let (inner_tag, _, after) = read_tlv(content)?;
    if inner_tag != 0x30 {
        return None;
    }
    content.get(..content.len().checked_sub(after.len())?)
}

/// The certificates of a PKCS#7 `SignedData`, as the bytes of the file.
fn pkcs7_certificates(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    let (tag, content, _) = read_tlv(bytes)?;
    if tag != 0x30 {
        return None;
    }
    let (_, _, rest) = read_tlv(content)?;
    let (tag, explicit, _) = read_tlv(rest)?;
    if tag != 0xA0 {
        return None;
    }
    let (tag, signed_data, _) = read_tlv(explicit)?;
    if tag != 0x30 {
        return None;
    }
    let (_, _, rest) = read_tlv(signed_data)?;
    let (_, _, rest) = read_tlv(rest)?;
    let (_, _, rest) = read_tlv(rest)?;
    let mut out = Vec::new();
    let (tag, mut certs, _) = match read_tlv(rest) {
        Some(found) => found,
        None => return Some(out),
    };
    if tag != 0xA0 {
        return Some(out);
    }
    while !certs.is_empty() {
        let (tag, _, after) = read_tlv(certs)?;
        if tag == 0x30 {
            out.push(certs.get(..certs.len().checked_sub(after.len())?)?);
        }
        certs = after;
        if out.len() > 4096 {
            return None;
        }
    }
    Some(out)
}

fn any_to_string(value: &Any) -> Option<String> {
    if let Ok(s) = value.decode_as::<Utf8StringRef<'_>>() {
        return Some(s.as_str().to_string());
    }
    if let Ok(s) = value.decode_as::<PrintableStringRef<'_>>() {
        return Some(s.as_str().to_string());
    }
    if let Ok(s) = value.decode_as::<TeletexStringRef<'_>>() {
        return Some(s.as_str().to_string());
    }
    None
}

struct Anchors {
    roots: Vec<Cert>,
    intermediates: Vec<Cert>,
}

impl Anchors {
    fn new(trust: &TrustAnchors) -> Result<Anchors, XpiError> {
        let roots = trust
            .roots
            .iter()
            .map(Cert::from_trusted)
            .collect::<Result<Vec<_>, _>>()?;
        let intermediates = trust
            .intermediates
            .iter()
            .map(Cert::from_trusted)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Anchors {
            roots,
            intermediates,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    EndEntity,
    Ca,
}

struct ChainContext<'a> {
    anchors: &'a Anchors,
    pool: &'a [Cert],
    now: i64,
    validity: ValidityPolicy,
}

fn check_validity(cert: &Cert, ctx: &ChainContext<'_>) -> Result<(), CertProblem> {
    if ctx.validity == ValidityPolicy::IgnoreExpiry {
        return Ok(());
    }
    let validity = &cert.parsed.tbs_certificate().validity();
    let not_before =
        i64::try_from(validity.not_before.to_unix_duration().as_secs()).unwrap_or(i64::MAX);
    let not_after =
        i64::try_from(validity.not_after.to_unix_duration().as_secs()).unwrap_or(i64::MAX);
    if ctx.now < not_before {
        return Err(CertProblem::NotYetValid);
    }
    if ctx.now > not_after {
        return Err(CertProblem::Expired);
    }
    Ok(())
}

struct CertExtensions {
    basic: Option<BasicConstraints>,
    key_usage: Option<KeyUsage>,
    ext_key_usage: Option<ExtendedKeyUsage>,
}

fn read_extensions(cert: &Cert) -> Result<CertExtensions, CertProblem> {
    let mut out = CertExtensions {
        basic: None,
        key_usage: None,
        ext_key_usage: None,
    };
    let mut seen: HashSet<ObjectIdentifier> = HashSet::new();
    let Some(extensions) = cert.parsed.tbs_certificate().extensions() else {
        return Ok(out);
    };
    for extension in extensions {
        if !seen.insert(extension.extn_id) {
            return Err(CertProblem::Malformed);
        }
        let value = extension.extn_value.as_bytes();
        match extension.extn_id {
            OID_BASIC_CONSTRAINTS => {
                out.basic =
                    Some(BasicConstraints::from_der(value).map_err(|_| CertProblem::Malformed)?);
            }
            OID_KEY_USAGE => {
                out.key_usage =
                    Some(KeyUsage::from_der(value).map_err(|_| CertProblem::Malformed)?);
            }
            OID_EXT_KEY_USAGE => {
                out.ext_key_usage =
                    Some(ExtendedKeyUsage::from_der(value).map_err(|_| CertProblem::Malformed)?);
            }
            other => {
                let known = KNOWN_EXTENSIONS.iter().any(|k| other.to_string() == *k);
                if extension.critical && !known {
                    return Err(CertProblem::UnknownCriticalExtension);
                }
            }
        }
    }
    Ok(out)
}

fn check_role(cert: &Cert, role: Role, ctx: &ChainContext<'_>) -> Result<Option<u8>, CertProblem> {
    check_validity(cert, ctx)?;
    let extensions = read_extensions(cert)?;
    let allows_code_signing = |eku: &ExtendedKeyUsage| {
        eku.0
            .iter()
            .any(|oid| *oid == OID_KP_CODE_SIGNING || *oid == OID_KP_ANY)
    };
    match role {
        Role::EndEntity => {
            if extensions.basic.as_ref().is_some_and(|b| b.ca) {
                return Err(CertProblem::NotEndEntity);
            }
            if let Some(usage) = &extensions.key_usage
                && !usage.digital_signature()
            {
                return Err(CertProblem::KeyUsageNotAllowed);
            }
            if let Some(eku) = &extensions.ext_key_usage
                && !allows_code_signing(eku)
            {
                return Err(CertProblem::ExtendedKeyUsageNotAllowed);
            }
            Ok(None)
        }
        Role::Ca => {
            let Some(basic) = &extensions.basic else {
                return Err(CertProblem::NotACa);
            };
            if !basic.ca {
                return Err(CertProblem::NotACa);
            }
            if let Some(usage) = &extensions.key_usage
                && !usage.key_cert_sign()
            {
                return Err(CertProblem::KeyUsageNotAllowed);
            }
            if let Some(eku) = &extensions.ext_key_usage
                && !allows_code_signing(eku)
            {
                return Err(CertProblem::ExtendedKeyUsageNotAllowed);
            }
            Ok(basic.path_len_constraint)
        }
    }
}

fn verify_cert_signature(child: &Cert, issuer: &Cert) -> Result<(), CertProblem> {
    let algorithm = child.parsed.signature_algorithm();
    let scheme = match scheme_from_oid(&algorithm.oid) {
        Some(Scheme::Rsa(Hash::Sha1)) | Some(Scheme::Ecdsa(Hash::Sha1)) => {
            return Err(CertProblem::UnsupportedSignatureAlgorithm);
        }
        Some(scheme) => scheme,
        None => return Err(CertProblem::UnsupportedSignatureAlgorithm),
    };
    let signature = child.parsed.signature().raw_bytes();
    match verify_with_spki(&issuer.spki, scheme, &child.tbs, signature) {
        Ok(()) => Ok(()),
        Err(SigFailure::KeyRejected) => Err(CertProblem::WeakKey),
        Err(SigFailure::Invalid) => Err(CertProblem::BadCertificateSignature),
    }
}

fn note(best: &mut Option<CertProblem>, problem: CertProblem) {
    if best.is_none() {
        *best = Some(problem);
    }
}

/// Builds a path from `cert` to a root, depth first. Returns the number of certificates on the path
/// (the signer's and the root included).
fn find_path(
    ctx: &ChainContext<'_>,
    cert: &Cert,
    visited: &mut Vec<Vec<u8>>,
    cas_below: usize,
    best: &mut Option<CertProblem>,
) -> Option<usize> {
    if ctx.anchors.roots.iter().any(|r| r.der == cert.der) {
        return Some(visited.len());
    }
    for root in &ctx.anchors.roots {
        if root.subject != cert.issuer {
            continue;
        }
        if let Err(problem) = verify_cert_signature(cert, root) {
            note(best, problem);
            continue;
        }
        if let Err(problem) = check_validity(root, ctx) {
            note(best, problem);
            continue;
        }
        if let Ok(extensions) = read_extensions(root)
            && let Some(limit) = extensions.basic.and_then(|b| b.path_len_constraint)
            && usize::from(limit) < cas_below
        {
            note(best, CertProblem::PathTooLong);
            continue;
        }
        return Some(visited.len().saturating_add(1));
    }
    let candidates = ctx.anchors.intermediates.iter().chain(ctx.pool.iter());
    for candidate in candidates {
        if candidate.subject != cert.issuer
            || visited.contains(&candidate.der)
            || ctx.anchors.roots.iter().any(|r| r.der == candidate.der)
        {
            continue;
        }
        if let Err(problem) = verify_cert_signature(cert, candidate) {
            note(best, problem);
            continue;
        }
        let path_len = match check_role(candidate, Role::Ca, ctx) {
            Ok(limit) => limit,
            Err(problem) => {
                note(best, problem);
                continue;
            }
        };
        if let Some(limit) = path_len
            && usize::from(limit) < cas_below
        {
            note(best, CertProblem::PathTooLong);
            continue;
        }
        if visited.len() >= MAX_CHAIN {
            note(best, CertProblem::PathTooLong);
            continue;
        }
        visited.push(candidate.der.clone());
        let found = find_path(ctx, candidate, visited, cas_below.saturating_add(1), best);
        visited.pop();
        if found.is_some() {
            return found;
        }
    }
    None
}

struct VerifiedSigner {
    chain_len: usize,
    common_name: String,
    organizational_unit: Option<String>,
}

fn verify_signer_chain(
    signer: &Cert,
    pool: &[Cert],
    anchors: &Anchors,
    opts: &XpiPolicy,
) -> Result<VerifiedSigner, CertProblem> {
    let ctx = ChainContext {
        anchors,
        pool,
        now: opts.now,
        validity: opts.validity,
    };
    check_role(signer, Role::EndEntity, &ctx)?;
    let mut visited = vec![signer.der.clone()];
    let mut best = None;
    let chain_len = find_path(&ctx, signer, &mut visited, 0, &mut best)
        .ok_or_else(|| best.unwrap_or(CertProblem::UnknownIssuer))?;
    let mut names = signer.name_attribute(OID_COMMON_NAME)?;
    if names.len() != 1 {
        return Err(CertProblem::Malformed);
    }
    let common_name = names.remove(0);
    let organizational_unit = signer.name_attribute(OID_ORG_UNIT)?.into_iter().next();
    Ok(VerifiedSigner {
        chain_len,
        common_name,
        organizational_unit,
    })
}

// ---------------------------------------------------------------------------------------------
// COSE
// ---------------------------------------------------------------------------------------------

type CborMap = Vec<(Value, Value)>;

struct CoseHeaders {
    raw: Vec<u8>,
    map: CborMap,
}

struct CoseSignature {
    protected: CoseHeaders,
    unprotected: CborMap,
    signature: Vec<u8>,
}

struct CoseSign {
    body_protected: CoseHeaders,
    body_unprotected: CborMap,
    signatures: Vec<CoseSignature>,
}

fn decode_cbor(bytes: &[u8]) -> Result<Value, XpiError> {
    let mut slice = bytes;
    let value: Value =
        coset::cbor::de::from_reader_with_recursion_limit(&mut slice, CBOR_RECURSION_LIMIT)
            .map_err(|_| XpiError::MalformedCose)?;
    if !slice.is_empty() {
        return Err(XpiError::MalformedCose);
    }
    Ok(value)
}

fn into_map(value: Value) -> Result<CborMap, XpiError> {
    match value {
        Value::Map(map) => Ok(map),
        _ => Err(XpiError::MalformedCose),
    }
}

fn parse_protected(value: Value) -> Result<CoseHeaders, XpiError> {
    let Value::Bytes(raw) = value else {
        return Err(XpiError::MalformedCose);
    };
    if raw.is_empty() {
        return Ok(CoseHeaders {
            raw,
            map: Vec::new(),
        });
    }
    let map = into_map(decode_cbor(&raw)?)?;
    Ok(CoseHeaders { raw, map })
}

fn parse_cose_sign(bytes: &[u8]) -> Result<CoseSign, XpiError> {
    let mut value = decode_cbor(bytes)?;
    if let Value::Tag(tag, inner) = value {
        if tag != 98 {
            return Err(XpiError::MalformedCose);
        }
        value = *inner;
    }
    let Value::Array(items) = value else {
        return Err(XpiError::MalformedCose);
    };
    let [body_protected, body_unprotected, payload, signatures] =
        <[Value; 4]>::try_from(items).map_err(|_| XpiError::MalformedCose)?;
    if !matches!(payload, Value::Null) {
        return Err(XpiError::MalformedCose);
    }
    let Value::Array(signatures) = signatures else {
        return Err(XpiError::MalformedCose);
    };
    if signatures.is_empty() || signatures.len() > MAX_COSE_SIGNATURES {
        return Err(XpiError::MalformedCose);
    }
    let mut parsed = Vec::with_capacity(signatures.len());
    for signature in signatures {
        let Value::Array(parts) = signature else {
            return Err(XpiError::MalformedCose);
        };
        let [protected, unprotected, raw_signature] =
            <[Value; 3]>::try_from(parts).map_err(|_| XpiError::MalformedCose)?;
        let Value::Bytes(raw_signature) = raw_signature else {
            return Err(XpiError::MalformedCose);
        };
        parsed.push(CoseSignature {
            protected: parse_protected(protected)?,
            unprotected: into_map(unprotected)?,
            signature: raw_signature,
        });
    }
    Ok(CoseSign {
        body_protected: parse_protected(body_protected)?,
        body_unprotected: into_map(body_unprotected)?,
        signatures: parsed,
    })
}

fn label_is(label: &Value, wanted: i64) -> bool {
    matches!(label, Value::Integer(i) if i128::from(*i) == i128::from(wanted))
}

/// The value of header `label`; a label that appears twice is an error.
fn header_value(map: &CborMap, label: i64) -> Result<Option<&Value>, XpiError> {
    let mut found = None;
    for (key, value) in map {
        if label_is(key, label) {
            if found.is_some() {
                return Err(XpiError::MalformedCose);
            }
            found = Some(value);
        }
    }
    Ok(found)
}

/// Certificates in a header value: a byte string or an array of byte strings.
fn certificate_blobs(value: &Value) -> Result<Vec<&[u8]>, XpiError> {
    match value {
        Value::Bytes(bytes) => Ok(vec![bytes.as_slice()]),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Bytes(bytes) => Ok(bytes.as_slice()),
                _ => Err(XpiError::MalformedCose),
            })
            .collect(),
        _ => Err(XpiError::MalformedCose),
    }
}

/// Certificates carried by the headers `kid` (4, Mozilla's placement) and `x5chain` (33).
fn header_certificates<'a>(map: &'a CborMap, out: &mut Vec<&'a [u8]>) -> Result<(), XpiError> {
    if let Some(value) = header_value(map, 4)? {
        // `kid` in a standard COSE file is a short identifier, not a certificate: only byte strings
        // that parse as certificates are used, and Mozilla's array form is always certificates.
        match value {
            Value::Bytes(bytes) => {
                if Certificate::from_der(bytes).is_ok() {
                    out.push(bytes.as_slice());
                }
            }
            Value::Array(_) => out.extend(certificate_blobs(value)?),
            _ => return Err(XpiError::MalformedCose),
        }
    }
    if let Some(value) = header_value(map, 33)? {
        out.extend(certificate_blobs(value)?);
    }
    Ok(())
}

fn cose_algorithm(headers: &CoseHeaders) -> Result<SignatureAlgorithm, XpiError> {
    let value = header_value(&headers.map, 1)?.ok_or(XpiError::UnsupportedAlgorithm)?;
    let Value::Integer(i) = value else {
        return Err(XpiError::MalformedCose);
    };
    match i128::from(*i) {
        -7 => Ok(SignatureAlgorithm::CoseEs256),
        -35 => Ok(SignatureAlgorithm::CoseEs384),
        -36 => Ok(SignatureAlgorithm::CoseEs512),
        _ => Err(XpiError::UnsupportedAlgorithm),
    }
}

/// The COSE `Sig_structure`. Mozilla's `cose-rust` writes the (empty) `external_aad` as CBOR `null`
/// instead of the empty byte string RFC 9052 asks for, and that is what real add-on signatures are made
/// over (`mozilla_aad`); the standard encoding is accepted as well.
fn sig_structure(
    body_protected: &[u8],
    sign_protected: &[u8],
    payload: &[u8],
    mozilla_aad: bool,
) -> Result<Vec<u8>, XpiError> {
    let structure = Value::Array(vec![
        Value::Text("Signature".to_string()),
        Value::Bytes(body_protected.to_vec()),
        Value::Bytes(sign_protected.to_vec()),
        if mozilla_aad {
            Value::Null
        } else {
            Value::Bytes(Vec::new())
        },
        Value::Bytes(payload.to_vec()),
    ]);
    let mut out = Vec::new();
    coset::cbor::ser::into_writer(&structure, &mut out).map_err(|_| XpiError::MalformedCose)?;
    Ok(out)
}

fn verify_cose(
    package: &ZipPackage,
    anchors: &Anchors,
    opts: &XpiPolicy,
    ignored: &mut HashSet<String>,
) -> Result<Vec<Signer>, XpiError> {
    let sig_bytes = read_signature_file(package, COSE_SIG, &opts.limits)?;
    if !package.contains(COSE_MANIFEST) {
        return Err(XpiError::IncompleteSignature);
    }
    let manifest_bytes = read_signature_file(package, COSE_MANIFEST, &opts.limits)?;
    let cose = parse_cose_sign(&sig_bytes)?;

    let mut body_certs: Vec<&[u8]> = Vec::new();
    header_certificates(&cose.body_protected.map, &mut body_certs)?;
    header_certificates(&cose.body_unprotected, &mut body_certs)?;

    let mut signers = Vec::new();
    for signature in &cose.signatures {
        let algorithm = cose_algorithm(&signature.protected)?;
        let mut signer_certs: Vec<&[u8]> = Vec::new();
        header_certificates(&signature.protected.map, &mut signer_certs)?;
        header_certificates(&signature.unprotected, &mut signer_certs)?;

        let total = signer_certs.len().saturating_add(body_certs.len());
        if total > opts.limits.max_certificates {
            return Err(CertProblem::TooManyCertificates.into());
        }
        let mut all = signer_certs.iter().chain(body_certs.iter());
        let signer_der = signer_certs
            .first()
            .or_else(|| body_certs.first())
            .copied()
            .ok_or(XpiError::MalformedCose)?;
        let signer_cert = Cert::parse(signer_der)?;
        let pool = all
            .by_ref()
            .map(|der| Cert::parse(der))
            .collect::<Result<Vec<_>, _>>()?;
        let verified = verify_signer_chain(&signer_cert, &pool, anchors, opts)?;

        let fixed: &'static dyn VerificationAlgorithm = match algorithm {
            SignatureAlgorithm::CoseEs256 => &ECDSA_P256_SHA256_FIXED,
            SignatureAlgorithm::CoseEs384 => &ECDSA_P384_SHA384_FIXED,
            _ => &ECDSA_P521_SHA512_FIXED,
        };
        let key = ParsedPublicKey::new(fixed, &signer_cert.spki)
            .map_err(|_| XpiError::Certificate(CertProblem::WeakKey))?;
        let mut good = false;
        for mozilla_aad in [true, false] {
            let message = sig_structure(
                &cose.body_protected.raw,
                &signature.protected.raw,
                &manifest_bytes,
                mozilla_aad,
            )?;
            if key.verify_sig(&message, &signature.signature).is_ok() {
                good = true;
                break;
            }
        }
        if !good {
            return Err(XpiError::BadSignature);
        }
        signers.push(Signer {
            algorithm,
            chain_len: verified.chain_len,
            common_name: verified.common_name,
            organizational_unit: verified.organizational_unit,
        });
    }

    ignored.insert(COSE_SIG.to_string());
    ignored.insert(COSE_MANIFEST.to_string());
    let manifest = parse_manifest(&manifest_bytes, ManifestDigest::Sha256)?;
    check_coverage(
        package,
        &manifest,
        ignored,
        ManifestDigest::Sha256,
        &opts.limits,
    )?;
    Ok(signers)
}

// ---------------------------------------------------------------------------------------------
// PKCS#7
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum JarFile {
    Rsa,
    Sf,
    Manifest,
}

const META_INF_RSA: JarFile = JarFile::Rsa;
const META_INF_SF: JarFile = JarFile::Sf;
const META_INF_MF: JarFile = JarFile::Manifest;

fn matches_jar_file(name: &str, kind: JarFile) -> bool {
    let Some(rest) = name.strip_prefix("META-INF/") else {
        return false;
    };
    if rest.is_empty() || rest.contains('/') {
        return false;
    }
    match kind {
        JarFile::Rsa => rest.len() > 4 && (rest.ends_with(".rsa") || rest.ends_with(".RSA")),
        JarFile::Sf => rest.len() > 3 && (rest.ends_with(".sf") || rest.ends_with(".SF")),
        JarFile::Manifest => matches!(
            rest,
            "MANIFEST.MF"
                | "MANIFEST.mf"
                | "manifest.MF"
                | "manifest.mf"
                | "Manifest.MF"
                | "Manifest.mf"
        ),
    }
}

fn is_sf_name(name: &str) -> bool {
    matches_jar_file(name, JarFile::Sf)
}

/// The one entry of that kind. Two are an error ("exactly one", as Firefox).
fn find_one(package: &ZipPackage, kind: JarFile) -> Result<Option<String>, XpiError> {
    let mut found = None;
    for name in package.file_names() {
        if matches_jar_file(name, kind) {
            if found.is_some() {
                return Err(XpiError::MalformedPkcs7);
            }
            found = Some(name.to_string());
        }
    }
    Ok(found)
}

fn hash_digest(hash: Hash, data: &[u8]) -> Vec<u8> {
    let algorithm = match hash {
        Hash::Sha1 => &digest::SHA1_FOR_LEGACY_USE_ONLY,
        Hash::Sha256 => &digest::SHA256,
        Hash::Sha384 => &digest::SHA384,
        Hash::Sha512 => &digest::SHA512,
    };
    digest::digest(algorithm, data).as_ref().to_vec()
}

fn serial_bytes(serial: &x509_cert::serial_number::SerialNumber) -> &[u8] {
    serial.as_bytes()
}

fn signer_info_digest(info: &SignerInfo) -> Option<ManifestDigest> {
    if info.digest_alg.oid == OID_SHA256 {
        Some(ManifestDigest::Sha256)
    } else if info.digest_alg.oid == OID_SHA1 {
        Some(ManifestDigest::Sha1)
    } else {
        None
    }
}

fn verify_pkcs7(
    package: &ZipPackage,
    rsa_name: &str,
    anchors: &Anchors,
    opts: &XpiPolicy,
    ignored: &mut HashSet<String>,
) -> Result<Signer, XpiError> {
    let sf_name = find_one(package, META_INF_SF)?.ok_or(XpiError::IncompleteSignature)?;
    let mf_name = find_one(package, META_INF_MF)?.ok_or(XpiError::IncompleteSignature)?;
    let rsa_bytes = read_signature_file(package, rsa_name, &opts.limits)?;
    let sf_bytes = read_signature_file(package, &sf_name, &opts.limits)?;
    let mf_bytes = read_signature_file(package, &mf_name, &opts.limits)?;

    let content_info = ContentInfo::from_der(&rsa_bytes).map_err(|_| XpiError::MalformedPkcs7)?;
    if content_info.content_type != OID_SIGNED_DATA {
        return Err(XpiError::MalformedPkcs7);
    }
    let signed: SignedData = content_info
        .content
        .decode_as()
        .map_err(|_| XpiError::MalformedPkcs7)?;

    // The certificates are taken from the file's own bytes, not re-encoded from the parsed structure.
    let raw_certificates = pkcs7_certificates(&rsa_bytes).ok_or(XpiError::MalformedPkcs7)?;
    if raw_certificates.len() > opts.limits.max_certificates {
        return Err(CertProblem::TooManyCertificates.into());
    }
    let certs: Vec<Cert> = raw_certificates
        .iter()
        .map(|der| Cert::parse(der))
        .collect::<Result<_, _>>()?;
    if certs.is_empty() {
        return Err(XpiError::MalformedPkcs7);
    }

    // Firefox prefers a SHA-256 signer over a SHA-1 one.
    let infos: Vec<&SignerInfo> = signed.signer_infos.0.iter().collect();
    let chosen = infos
        .iter()
        .find(|i| signer_info_digest(i) == Some(ManifestDigest::Sha256))
        .or_else(|| {
            infos
                .iter()
                .find(|i| signer_info_digest(i) == Some(ManifestDigest::Sha1))
        })
        .copied()
        .ok_or(XpiError::UnsupportedAlgorithm)?;
    let digest_kind = signer_info_digest(chosen).ok_or(XpiError::UnsupportedAlgorithm)?;
    let (digest_hash, algorithm) = match digest_kind {
        ManifestDigest::Sha256 => (Hash::Sha256, SignatureAlgorithm::Pkcs7Sha256),
        ManifestDigest::Sha1 => (Hash::Sha1, SignatureAlgorithm::Pkcs7Sha1),
    };

    let SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
        issuer,
        serial_number,
    }) = &chosen.sid
    else {
        return Err(XpiError::MalformedPkcs7);
    };
    let issuer_der = issuer.to_der().map_err(|_| XpiError::MalformedPkcs7)?;
    let signer_index = certs
        .iter()
        .position(|c| {
            c.issuer == issuer_der
                && serial_bytes(c.parsed.tbs_certificate().serial_number())
                    == serial_bytes(serial_number)
        })
        .ok_or(XpiError::MalformedPkcs7)?;
    let signer_cert = certs.get(signer_index).ok_or(XpiError::MalformedPkcs7)?;
    let pool: Vec<Cert> = certs
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != signer_index)
        .map(|(_, c)| Cert::parse(&c.der))
        .collect::<Result<_, _>>()?;
    let verified = verify_signer_chain(signer_cert, &pool, anchors, opts)?;

    // The signature: over the DER of the signed attributes (after checking the content type and the
    // message digest in them), or directly over the signature file.
    let sf_digest = hash_digest(digest_hash, &sf_bytes);
    let scheme = match scheme_from_oid(&chosen.signature_algorithm.oid) {
        Some(scheme) => scheme,
        None if chosen.signature_algorithm.oid == OID_RSA_ENCRYPTION => Scheme::Rsa(digest_hash),
        None => return Err(XpiError::UnsupportedAlgorithm),
    };
    if matches!(scheme, Scheme::Ecdsa(Hash::Sha1)) {
        return Err(XpiError::UnsupportedAlgorithm);
    }
    let signed_message: Vec<u8> = match &chosen.signed_attrs {
        Some(attributes) => {
            let mut content_type_ok = false;
            let mut digest_ok = false;
            for attribute in attributes.iter() {
                if attribute.oid == OID_ATTR_CONTENT_TYPE {
                    let mut values = attribute.values.iter();
                    if let (Some(value), None) = (values.next(), values.next()) {
                        content_type_ok = value
                            .decode_as::<ObjectIdentifier>()
                            .is_ok_and(|oid| oid == OID_PKCS7_DATA);
                    }
                } else if attribute.oid == OID_ATTR_MESSAGE_DIGEST {
                    let mut values = attribute.values.iter();
                    if let (Some(value), None) = (values.next(), values.next()) {
                        digest_ok = value
                            .decode_as::<OctetString>()
                            .is_ok_and(|d| d.as_bytes() == sf_digest.as_slice());
                    }
                }
            }
            if !content_type_ok || !digest_ok {
                return Err(XpiError::BadSignature);
            }
            attributes.to_der().map_err(|_| XpiError::MalformedPkcs7)?
        }
        None => sf_bytes.clone(),
    };
    match verify_with_spki(
        &signer_cert.spki,
        scheme,
        &signed_message,
        chosen.signature.as_bytes(),
    ) {
        Ok(()) => {}
        Err(SigFailure::KeyRejected) => return Err(CertProblem::WeakKey.into()),
        Err(SigFailure::Invalid) => return Err(XpiError::BadSignature),
    }

    let expected_manifest_digest = parse_signature_file(&sf_bytes, digest_kind)?;
    let actual = hash_digest(digest_hash, &mf_bytes);
    if actual != expected_manifest_digest {
        return Err(XpiError::ManifestDigestMismatch);
    }
    ignored.insert(sf_name);
    ignored.insert(rsa_name.to_string());
    ignored.insert(mf_name);
    let manifest = parse_manifest(&mf_bytes, digest_kind)?;
    let result = check_coverage(package, &manifest, ignored, digest_kind, &opts.limits);
    if result.is_err() {
        ignored.clear();
    }
    result?;
    Ok(Signer {
        algorithm,
        chain_len: verified.chain_len,
        common_name: verified.common_name,
        organizational_unit: verified.organizational_unit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_common_name_is_the_id_up_to_64_characters() {
        assert_eq!(expected_common_name("a@b.c"), "a@b.c");
        let at_limit = format!("{}@x.y", "a".repeat(60));
        assert_eq!(at_limit.chars().count(), 64);
        assert_eq!(expected_common_name(&at_limit), at_limit);
    }

    #[test]
    fn expected_common_name_is_the_sha256_hex_over_64_characters() {
        let long = format!("{}@x.y", "a".repeat(61));
        assert_eq!(long.chars().count(), 65);
        let name = expected_common_name(&long);
        assert_eq!(name.len(), 64);
        assert!(
            name.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }

    #[test]
    fn addon_id_shapes() {
        assert!(looks_like_addon_id("video-bg-play@timdream.org"));
        assert!(looks_like_addon_id(
            "{12345678-1234-1234-1234-123456789abc}"
        ));
        assert!(!looks_like_addon_id(
            "{12345678-1234-1234-1234-123456789ab}"
        ));
        assert!(!looks_like_addon_id("no-at-sign"));
        assert!(!looks_like_addon_id("a@"));
        assert!(!looks_like_addon_id("a b@c.d"));
        assert!(!looks_like_addon_id(""));
    }

    #[test]
    fn json_depth_edge() {
        let at = format!("{}{}", "[".repeat(64), "]".repeat(64));
        assert!(json_depth_within(at.as_bytes(), 64));
        let over = format!("{}{}", "[".repeat(65), "]".repeat(65));
        assert!(!json_depth_within(over.as_bytes(), 64));
        assert!(json_depth_within(br#"{"a":"[[[[[[[[","b":"\"[[["}"#, 2));
    }

    fn manifest(text: &str) -> Result<HashMap<String, Vec<u8>>, XpiError> {
        parse_manifest(text.as_bytes(), ManifestDigest::Sha256)
    }

    const D: &str = "SHA256-Digest: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    #[test]
    fn manifest_with_two_entries_parses() {
        let text = format!("Manifest-Version: 1.0\n\nName: a.js\n{D}\n\nName: b/c.js\n{D}\n");
        let items = manifest(&text).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.contains_key("b/c.js"));
    }

    #[test]
    fn manifest_accepts_crlf_cr_and_no_trailing_newline() {
        let crlf = format!("Manifest-Version: 1.0\r\n\r\nName: a\r\n{D}\r\n");
        assert_eq!(manifest(&crlf).unwrap().len(), 1);
        let cr = format!("Manifest-Version: 1.0\r\rName: a\r{D}\r");
        assert_eq!(manifest(&cr).unwrap().len(), 1);
        let bare = format!("Manifest-Version: 1.0\n\nName: a\n{D}");
        assert_eq!(manifest(&bare).unwrap().len(), 1);
    }

    #[test]
    fn manifest_with_only_a_header_is_empty() {
        assert!(manifest("Manifest-Version: 1.0\n\n").unwrap().is_empty());
    }

    #[test]
    fn manifest_continuation_lines_join_a_long_name() {
        let text = format!("Manifest-Version: 1.0\n\nName: aaaaaaaaaa\n bbbbbbbbbb\n{D}\n");
        assert!(
            manifest(&text)
                .unwrap()
                .contains_key("aaaaaaaaaabbbbbbbbbb")
        );
    }

    #[test]
    fn manifest_refuses_a_wrong_or_missing_version_line() {
        assert!(manifest("Manifest-Version: 2.0\n\n").is_err());
        assert!(manifest("manifest-version: 1.0\n\n").is_err());
        assert!(manifest("").is_err());
        assert!(manifest("Manifest-Version: 1.0\n continued\n\n").is_err());
    }

    #[test]
    fn manifest_refuses_duplicates_magic_and_incomplete_sections() {
        let dup = format!("Manifest-Version: 1.0\n\nName: a\n{D}\n\nName: a\n{D}\n");
        assert!(manifest(&dup).is_err());
        let twice = format!("Manifest-Version: 1.0\n\nName: a\n{D}\n{D}\n");
        assert!(manifest(&twice).is_err());
        let magic = format!("Manifest-Version: 1.0\n\nName: a\n{D}\nMagic: x\n");
        assert!(manifest(&magic).is_err());
        let nameless = format!("Manifest-Version: 1.0\n\n{D}\n");
        assert!(manifest(&nameless).is_err());
        let digestless = "Manifest-Version: 1.0\n\nName: a\n";
        assert!(manifest(digestless).is_err());
        let two_names = format!("Manifest-Version: 1.0\n\nName: a\nName: b\n{D}\n");
        assert!(manifest(&two_names).is_err());
        let empty_name = format!("Manifest-Version: 1.0\n\nName:\n{D}\n");
        assert!(manifest(&empty_name).is_err());
    }

    #[test]
    fn manifest_never_reads_md5_or_sha1_when_sha256_is_wanted() {
        let md5 = "Manifest-Version: 1.0\n\nName: a\nMD5-Digest: AAAAAAAAAAAAAAAAAAAAAA==\n";
        assert!(manifest(md5).is_err());
        let sha1 = "Manifest-Version: 1.0\n\nName: a\nSHA1-Digest: AAAAAAAAAAAAAAAAAAAAAAAAAAA=\n";
        assert!(manifest(sha1).is_err());
    }

    #[test]
    fn manifest_line_limit_is_72_bytes_per_physical_line() {
        let ok = format!("Manifest-Version: 1.0\n\nName: {}\n{D}\n", "a".repeat(66));
        assert!(manifest(&ok).is_ok());
        let over = format!("Manifest-Version: 1.0\n\nName: {}\n{D}\n", "a".repeat(67));
        assert!(manifest(&over).is_err());
    }

    #[test]
    fn manifest_refuses_a_nul_byte_and_trailing_blank_lines() {
        let nul = format!("Manifest-Version: 1.0\n\nName: a\0b\n{D}\n");
        assert!(manifest(&nul).is_err());
        let blanks = format!("Manifest-Version: 1.0\n\nName: a\n{D}\n\n\n");
        assert!(manifest(&blanks).is_err());
    }

    #[test]
    fn signature_file_gives_the_manifest_digest() {
        let sf = "Signature-Version: 1.0\nSHA1-Digest-Manifest: AAAAAAAAAAAAAAAAAAAAAAAAAAA=\nSHA256-Digest-Manifest: AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=\n\n";
        let sha256 = parse_signature_file(sf.as_bytes(), ManifestDigest::Sha256).unwrap();
        assert_eq!(sha256, vec![1u8; 32]);
        let sha1 = parse_signature_file(sf.as_bytes(), ManifestDigest::Sha1).unwrap();
        assert_eq!(sha1, vec![0u8; 20]);
        assert!(
            parse_signature_file(b"Signature-Version: 1.0\n\n", ManifestDigest::Sha256).is_err()
        );
        assert!(
            parse_signature_file(b"Manifest-Version: 1.0\n\n", ManifestDigest::Sha256).is_err()
        );
    }

    #[test]
    fn read_tlv_handles_short_and_long_lengths_and_refuses_the_rest() {
        let (tag, content, rest) = read_tlv(&[0x30, 0x02, 0xAA, 0xBB, 0xCC]).unwrap();
        assert_eq!((tag, content, rest), (0x30, &[0xAA, 0xBB][..], &[0xCC][..]));
        let mut long = vec![0x04, 0x81, 0x80];
        long.extend(vec![7u8; 0x80]);
        let (_, content, rest) = read_tlv(&long).unwrap();
        assert_eq!(content.len(), 0x80);
        assert!(rest.is_empty());
        assert!(read_tlv(&[]).is_none());
        assert!(read_tlv(&[0x30]).is_none());
        assert!(
            read_tlv(&[0x30, 0x05, 1, 2]).is_none(),
            "length past the end"
        );
        assert!(read_tlv(&[0x30, 0x80, 0, 0]).is_none(), "indefinite length");
        assert!(
            read_tlv(&[0x30, 0x85, 0, 0, 0, 0, 1, 0]).is_none(),
            "five length bytes"
        );
        assert!(read_tlv(&[0x1f, 0x01, 0]).is_none(), "multi-byte tag");
        assert!(read_tlv(&[0x30, 0x84, 0xFF, 0xFF, 0xFF, 0xFF]).is_none());
    }

    #[test]
    fn raw_tbs_is_the_first_element_of_the_certificate() {
        let cert = [0x30, 0x07, 0x30, 0x02, 0x01, 0x02, 0x05, 0x00, 0x00];
        assert_eq!(raw_tbs(&cert), Some(&[0x30, 0x02, 0x01, 0x02][..]));
        assert_eq!(raw_tbs(&[0x31, 0x00]), None);
        assert_eq!(raw_tbs(&[0x30, 0x02, 0x04, 0x00]), None);
    }

    #[test]
    fn pkcs7_without_a_certificates_field_has_none() {
        assert_eq!(pkcs7_certificates(&[0x30, 0x00]), None);
        assert_eq!(pkcs7_certificates(b"garbage"), None);
    }

    #[test]
    fn jar_file_name_patterns() {
        assert!(matches_jar_file("META-INF/mozilla.rsa", JarFile::Rsa));
        assert!(matches_jar_file("META-INF/ANY.RSA", JarFile::Rsa));
        assert!(!matches_jar_file("META-INF/a/b.rsa", JarFile::Rsa));
        assert!(!matches_jar_file("other/mozilla.rsa", JarFile::Rsa));
        assert!(!matches_jar_file("META-INF/.rsa", JarFile::Rsa));
        assert!(matches_jar_file("META-INF/mozilla.sf", JarFile::Sf));
        assert!(matches_jar_file("META-INF/manifest.mf", JarFile::Manifest));
        assert!(matches_jar_file("META-INF/MANIFEST.MF", JarFile::Manifest));
        assert!(!matches_jar_file("META-INF/mozilla.mf", JarFile::Manifest));
        assert!(!matches_jar_file(
            "META-INF/cose.manifest",
            JarFile::Manifest
        ));
    }
}
