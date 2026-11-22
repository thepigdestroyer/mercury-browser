// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A test PKI (root, intermediate, end entity with CN = add-on id) and signers that produce the
//! COSE and PKCS#7 signature files Firefox expects. There is no Mozilla signing key, so every test
//! that is not about the real add-on uses this.
#![allow(dead_code)]

use std::sync::OnceLock;

use aws_lc_rs::digest::{SHA1_FOR_LEGACY_USE_ONLY, SHA256, digest};
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_ASN1_SIGNING, ECDSA_P256_SHA256_FIXED_SIGNING,
    ECDSA_P384_SHA384_FIXED_SIGNING, EcdsaKeyPair, RSA_PKCS1_SHA256, RsaKeyPair,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedAttributes, SignedData, SignerIdentifier,
    SignerInfo, SignerInfos,
};
use coset::cbor::Value;
use mercury_extensions::roots::TrustAnchors;
use rcgen::{
    BasicConstraints, CertificateParams, CustomExtension, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
    PKCS_ECDSA_P384_SHA384, date_time_ymd,
};
use x509_cert::Certificate;
use x509_cert::attr::Attribute;
use x509_cert::der::asn1::{ObjectIdentifier, OctetString, SetOfVec};
use x509_cert::der::{Any, Decode, Encode};
use x509_cert::spki::AlgorithmIdentifierOwned;

pub type Files = Vec<(String, Vec<u8>)>;

/// 2026-06-01T00:00:00Z: the `now` of the tests that use the test PKI.
pub fn now() -> i64 {
    date_time_ymd(2026, 6, 1).unix_timestamp()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    Ec256,
    Ec384,
    Rsa,
}

fn rsa_pkcs8() -> &'static Vec<u8> {
    static KEY: OnceLock<Vec<u8>> = OnceLock::new();
    KEY.get_or_init(|| {
        crx3::aws_lc::RsaSigner::generate()
            .unwrap()
            .to_pkcs8()
            .unwrap()
    })
}

fn generate_key(kind: KeyKind) -> (KeyPair, Vec<u8>) {
    match kind {
        KeyKind::Ec256 => {
            let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
            let der = key.serialize_der();
            (key, der)
        }
        KeyKind::Ec384 => {
            let key = KeyPair::generate_for(&PKCS_ECDSA_P384_SHA384).unwrap();
            let der = key.serialize_der();
            (key, der)
        }
        KeyKind::Rsa => {
            let der = rsa_pkcs8().clone();
            let key = KeyPair::try_from(der.as_slice()).unwrap();
            (key, der)
        }
    }
}

pub type Date = (i32, u8, u8);

#[derive(Clone)]
pub struct PkiSpec {
    pub cn: String,
    pub ee_kind: KeyKind,
    pub root_validity: (Date, Date),
    pub inter_validity: (Date, Date),
    pub ee_validity: (Date, Date),
    pub ee_ekus: Vec<ExtendedKeyUsagePurpose>,
    pub ee_key_usages: Vec<KeyUsagePurpose>,
    pub ee_is_ca: bool,
    pub inter_is_ca: bool,
    pub inter_path_len: Option<u8>,
    pub inter_ekus: Vec<ExtendedKeyUsagePurpose>,
    pub inter_key_usages: Vec<KeyUsagePurpose>,
    pub ee_critical_unknown_extension: bool,
}

impl PkiSpec {
    pub fn new(cn: &str) -> PkiSpec {
        PkiSpec {
            cn: cn.to_string(),
            ee_kind: KeyKind::Ec256,
            root_validity: ((2020, 1, 1), (2090, 1, 1)),
            inter_validity: ((2020, 1, 1), (2080, 1, 1)),
            ee_validity: ((2025, 1, 1), (2030, 1, 1)),
            ee_ekus: vec![ExtendedKeyUsagePurpose::CodeSigning],
            ee_key_usages: vec![KeyUsagePurpose::DigitalSignature],
            ee_is_ca: false,
            inter_is_ca: true,
            inter_path_len: None,
            inter_ekus: Vec::new(),
            inter_key_usages: vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign],
            ee_critical_unknown_extension: false,
        }
    }

    pub fn kind(mut self, kind: KeyKind) -> PkiSpec {
        self.ee_kind = kind;
        self
    }
}

pub struct Pki {
    pub cn: String,
    pub root_der: Vec<u8>,
    pub inter_der: Vec<u8>,
    pub ee_der: Vec<u8>,
    pub ee_pkcs8: Vec<u8>,
    pub ee_kind: KeyKind,
}

fn params(cn: &str, validity: (Date, Date)) -> CertificateParams {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, cn);
    p.distinguished_name = dn;
    p.not_before = date_time_ymd(validity.0.0, validity.0.1, validity.0.2);
    p.not_after = date_time_ymd(validity.1.0, validity.1.1, validity.1.2);
    p
}

pub fn build_pki(spec: &PkiSpec) -> Pki {
    let (root_key, _) = generate_key(KeyKind::Ec256);
    let mut root_params = params("Test Add-ons Root", spec.root_validity);
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let root = root_params.self_signed(&root_key).unwrap();
    let root_issuer = Issuer::new(root_params, root_key);

    let (inter_key, _) = generate_key(KeyKind::Ec256);
    let mut inter_params = params("Test Add-ons Intermediate", spec.inter_validity);
    inter_params.is_ca = if spec.inter_is_ca {
        IsCa::Ca(match spec.inter_path_len {
            Some(n) => BasicConstraints::Constrained(n),
            None => BasicConstraints::Unconstrained,
        })
    } else {
        IsCa::NoCa
    };
    inter_params.key_usages = spec.inter_key_usages.clone();
    inter_params.extended_key_usages = spec.inter_ekus.clone();
    let inter = inter_params.signed_by(&inter_key, &root_issuer).unwrap();
    let inter_issuer = Issuer::new(inter_params, inter_key);

    let (ee_key, ee_pkcs8) = generate_key(spec.ee_kind);
    let mut ee_params = params(&spec.cn, spec.ee_validity);
    ee_params.is_ca = if spec.ee_is_ca {
        IsCa::Ca(BasicConstraints::Unconstrained)
    } else {
        IsCa::NoCa
    };
    ee_params.key_usages = spec.ee_key_usages.clone();
    ee_params.extended_key_usages = spec.ee_ekus.clone();
    if spec.ee_critical_unknown_extension {
        let mut ext = CustomExtension::from_oid_content(&[1, 2, 3, 4, 5], vec![0x05, 0x00]);
        ext.set_criticality(true);
        ee_params.custom_extensions.push(ext);
    }
    let ee = ee_params.signed_by(&ee_key, &inter_issuer).unwrap();

    Pki {
        cn: spec.cn.clone(),
        root_der: root.der().to_vec(),
        inter_der: inter.der().to_vec(),
        ee_der: ee.der().to_vec(),
        ee_pkcs8,
        ee_kind: spec.ee_kind,
    }
}

impl Pki {
    pub fn new(cn: &str) -> Pki {
        build_pki(&PkiSpec::new(cn))
    }

    /// The root is the only anchor; the intermediate must come with the signature.
    pub fn anchors(&self) -> TrustAnchors {
        TrustAnchors::from_der(std::slice::from_ref(&self.root_der), &[]).unwrap()
    }

    /// The root is the anchor and the intermediate is a known candidate (as Mozilla's are).
    pub fn anchors_with_intermediate(&self) -> TrustAnchors {
        TrustAnchors::from_der(
            std::slice::from_ref(&self.root_der),
            std::slice::from_ref(&self.inter_der),
        )
        .unwrap()
    }
}

// ---------------------------------------------------------------------------------------------
// Files, manifests
// ---------------------------------------------------------------------------------------------

pub fn b64(data: &[u8]) -> String {
    STANDARD.encode(data)
}

pub fn sha256(data: &[u8]) -> Vec<u8> {
    digest(&SHA256, data).as_ref().to_vec()
}

pub fn sha1(data: &[u8]) -> Vec<u8> {
    digest(&SHA1_FOR_LEGACY_USE_ONLY, data).as_ref().to_vec()
}

/// One manifest line, wrapped at 72 bytes with continuation lines.
pub fn jar_line(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut first = true;
    let mut rest = bytes;
    while !rest.is_empty() {
        let take = rest.len().min(72);
        let (chunk, tail) = rest.split_at(take);
        if !first {
            out.push(' ');
        }
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push('\n');
        rest = tail;
        first = false;
    }
    if first {
        out.push('\n');
    }
    out
}

/// A JAR manifest in the form AMO writes: both SHA-1 and SHA-256 digests per entry.
pub fn jar_manifest(files: &[(String, Vec<u8>)]) -> String {
    let mut out = String::from("Manifest-Version: 1.0\n\n");
    for (name, data) in files {
        out.push_str(&jar_line(&format!("Name: {name}")));
        out.push_str("Digest-Algorithms: SHA1 SHA256\n");
        out.push_str(&format!("SHA1-Digest: {}\n", b64(&sha1(data))));
        out.push_str(&format!("SHA256-Digest: {}\n", b64(&sha256(data))));
        out.push('\n');
    }
    out
}

pub fn manifest_json(id: &str, manifest_version: u32) -> Vec<u8> {
    let gecko = format!(r#""browser_specific_settings": {{ "gecko": {{ "id": "{id}" }} }}"#);
    format!(
        r#"{{ "manifest_version": {manifest_version}, "name": "Test", "version": "1.0", {gecko} }}"#
    )
    .into_bytes()
}

/// The files of a small add-on, unsigned.
pub fn base_files(id: &str, manifest_version: u32) -> Files {
    vec![
        (
            "manifest.json".to_string(),
            manifest_json(id, manifest_version),
        ),
        (
            "background.js".to_string(),
            b"console.log('hi');\n".to_vec(),
        ),
        (
            "_locales/en/messages.json".to_string(),
            br#"{"name":{"message":"Test"}}"#.to_vec(),
        ),
    ]
}

// ---------------------------------------------------------------------------------------------
// COSE
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoseLayout {
    /// What Mozilla's `cose-rust` writes: the signer in the signature's protected `kid` (4), the
    /// intermediates in the body's protected header (4, an array).
    Mozilla,
    /// A standard `x5chain` (33) in the signature's unprotected header: [signer, intermediate].
    X5ChainUnprotected,
    /// A standard `x5chain` (33) in the body's protected header: [signer, intermediate].
    X5ChainBodyProtected,
}

#[derive(Clone)]
pub struct CoseOptions {
    pub layout: CoseLayout,
    pub tagged: bool,
    pub alg_label: i64,
    pub include_intermediate: bool,
    pub extra_certs: Vec<Vec<u8>>,
    /// Sign this payload instead of `cose.manifest`.
    pub payload_override: Option<Vec<u8>>,
    pub signature_count: usize,
    pub omit_alg: bool,
    pub embed_payload: bool,
}

impl Default for CoseOptions {
    fn default() -> Self {
        CoseOptions {
            layout: CoseLayout::Mozilla,
            tagged: true,
            alg_label: -7,
            include_intermediate: true,
            extra_certs: Vec::new(),
            payload_override: None,
            signature_count: 1,
            omit_alg: false,
            embed_payload: false,
        }
    }
}

fn cbor(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    coset::cbor::ser::into_writer(value, &mut out).unwrap();
    out
}

fn int(i: i64) -> Value {
    Value::from(i)
}

fn sign_fixed(pki: &Pki, message: &[u8]) -> Vec<u8> {
    let rng = SystemRandom::new();
    match pki.ee_kind {
        KeyKind::Ec256 => EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pki.ee_pkcs8)
            .unwrap()
            .sign(&rng, message)
            .unwrap()
            .as_ref()
            .to_vec(),
        KeyKind::Ec384 => EcdsaKeyPair::from_pkcs8(&ECDSA_P384_SHA384_FIXED_SIGNING, &pki.ee_pkcs8)
            .unwrap()
            .sign(&rng, message)
            .unwrap()
            .as_ref()
            .to_vec(),
        KeyKind::Rsa => panic!("COSE needs an ECDSA signer"),
    }
}

/// `META-INF/cose.sig` for `manifest` (the bytes of `META-INF/cose.manifest`).
pub fn cose_sig(manifest: &[u8], pki: &Pki, opts: &CoseOptions) -> Vec<u8> {
    let mut chain: Vec<Vec<u8>> = Vec::new();
    if opts.include_intermediate {
        chain.push(pki.inter_der.clone());
    }
    chain.extend(opts.extra_certs.iter().cloned());
    let bytes = |der: &[u8]| Value::Bytes(der.to_vec());
    let array = |items: &[Vec<u8>]| Value::Array(items.iter().map(|d| bytes(d)).collect());

    let (body_protected, signature_protected_entries, signature_unprotected) = match opts.layout {
        CoseLayout::Mozilla => {
            let body = if chain.is_empty() {
                Vec::new()
            } else {
                cbor(&Value::Map(vec![(int(4), array(&chain))]))
            };
            (
                body,
                vec![(int(4), bytes(&pki.ee_der))],
                Value::Map(Vec::new()),
            )
        }
        CoseLayout::X5ChainUnprotected => {
            let mut all = vec![pki.ee_der.clone()];
            all.extend(chain.clone());
            (
                Vec::new(),
                Vec::new(),
                Value::Map(vec![(int(33), array(&all))]),
            )
        }
        CoseLayout::X5ChainBodyProtected => {
            let mut all = vec![pki.ee_der.clone()];
            all.extend(chain.clone());
            (
                cbor(&Value::Map(vec![(int(33), array(&all))])),
                Vec::new(),
                Value::Map(Vec::new()),
            )
        }
    };
    let mut protected_map = Vec::new();
    if !opts.omit_alg {
        protected_map.push((int(1), int(opts.alg_label)));
    }
    protected_map.extend(signature_protected_entries);
    let signature_protected = cbor(&Value::Map(protected_map));

    let payload = opts
        .payload_override
        .clone()
        .unwrap_or_else(|| manifest.to_vec());
    let structure = cbor(&Value::Array(vec![
        Value::Text("Signature".to_string()),
        Value::Bytes(body_protected.clone()),
        Value::Bytes(signature_protected.clone()),
        // Mozilla's `cose-rust` writes the empty external_aad as null; the standard layouts use a bstr.
        if opts.layout == CoseLayout::Mozilla {
            Value::Null
        } else {
            Value::Bytes(Vec::new())
        },
        Value::Bytes(payload),
    ]));
    let signature = sign_fixed(pki, &structure);
    let one = Value::Array(vec![
        Value::Bytes(signature_protected),
        signature_unprotected,
        Value::Bytes(signature),
    ]);
    let signatures = Value::Array(vec![one; opts.signature_count]);
    let payload_field = if opts.embed_payload {
        Value::Bytes(manifest.to_vec())
    } else {
        Value::Null
    };
    let sign = Value::Array(vec![
        Value::Bytes(body_protected),
        Value::Map(Vec::new()),
        payload_field,
        signatures,
    ]);
    cbor(&if opts.tagged {
        Value::Tag(98, Box::new(sign))
    } else {
        sign
    })
}

// ---------------------------------------------------------------------------------------------
// PKCS#7
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pkcs7Digest {
    Sha1,
    Sha256,
}

#[derive(Clone)]
pub struct Pkcs7Options {
    pub digest: Pkcs7Digest,
    pub signed_attributes: bool,
    pub include_intermediate: bool,
    pub tamper_message_digest: bool,
    pub wrong_content_type: bool,
    /// Sign something other than the signature file.
    pub sign_other_content: bool,
}

impl Default for Pkcs7Options {
    fn default() -> Self {
        Pkcs7Options {
            digest: Pkcs7Digest::Sha256,
            signed_attributes: true,
            include_intermediate: true,
            tamper_message_digest: false,
            wrong_content_type: false,
            sign_other_content: false,
        }
    }
}

pub struct Pkcs7Parts {
    pub rsa: Vec<u8>,
    pub sf: Vec<u8>,
    pub mf: Vec<u8>,
}

fn alg(oid: &str) -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned {
        oid: oid.parse().unwrap(),
        parameters: Some(Any::null()),
    }
}

fn attribute(oid: &str, value: Any) -> Attribute {
    Attribute {
        oid: oid.parse().unwrap(),
        values: SetOfVec::try_from(vec![value]).unwrap(),
    }
}

fn sign_der(pki: &Pki, digest: Pkcs7Digest, message: &[u8]) -> (Vec<u8>, &'static str) {
    let rng = SystemRandom::new();
    match pki.ee_kind {
        KeyKind::Rsa => {
            let key = RsaKeyPair::from_pkcs8(&pki.ee_pkcs8).unwrap();
            let mut signature = vec![0u8; key.public_modulus_len()];
            // aws-lc cannot sign with SHA-1: a SHA-1 digest algorithm is paired with a SHA-256
            // signature algorithm, which is what the digest and the signature fields allow.
            let _ = digest;
            key.sign(&RSA_PKCS1_SHA256, &rng, message, &mut signature)
                .unwrap();
            (signature, "1.2.840.113549.1.1.11")
        }
        KeyKind::Ec256 => {
            let key =
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &pki.ee_pkcs8).unwrap();
            (
                key.sign(&rng, message).unwrap().as_ref().to_vec(),
                "1.2.840.10045.4.3.2",
            )
        }
        KeyKind::Ec384 => panic!("PKCS#7 tests use RSA or P-256"),
    }
}

/// `mozilla.mf`, `mozilla.sf` and `mozilla.rsa` for `files` (the entries the manifest covers).
pub fn pkcs7_parts(files: &[(String, Vec<u8>)], pki: &Pki, opts: &Pkcs7Options) -> Pkcs7Parts {
    let mf = jar_manifest(files).into_bytes();
    let sf = format!(
        "Signature-Version: 1.0\nSHA1-Digest-Manifest: {}\nSHA256-Digest-Manifest: {}\n\n",
        b64(&sha1(&mf)),
        b64(&sha256(&mf)),
    )
    .into_bytes();

    let (digest_oid, sf_digest) = match opts.digest {
        Pkcs7Digest::Sha256 => ("2.16.840.1.101.3.4.2.1", sha256(&sf)),
        Pkcs7Digest::Sha1 => ("1.3.14.3.2.26", sha1(&sf)),
    };
    let ee = Certificate::from_der(&pki.ee_der).unwrap();
    let inter = Certificate::from_der(&pki.inter_der).unwrap();

    let message_digest = if opts.tamper_message_digest {
        vec![0u8; sf_digest.len()]
    } else {
        sf_digest
    };
    let content_type_oid: ObjectIdentifier = if opts.wrong_content_type {
        "1.2.840.113549.1.7.2".parse().unwrap()
    } else {
        "1.2.840.113549.1.7.1".parse().unwrap()
    };
    let attributes: SignedAttributes = SetOfVec::try_from(vec![
        attribute(
            "1.2.840.113549.1.9.3",
            Any::encode_from(&content_type_oid).unwrap(),
        ),
        attribute(
            "1.2.840.113549.1.9.4",
            Any::encode_from(&OctetString::new(message_digest).unwrap()).unwrap(),
        ),
    ])
    .unwrap();
    let to_sign: Vec<u8> = if opts.sign_other_content {
        b"something else".to_vec()
    } else if opts.signed_attributes {
        attributes.to_der().unwrap()
    } else {
        sf.clone()
    };
    let (signature, signature_oid) = sign_der(pki, opts.digest, &to_sign);

    let signer_info = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: ee.tbs_certificate().issuer().clone(),
            serial_number: ee.tbs_certificate().serial_number().clone(),
        }),
        digest_alg: alg(digest_oid),
        signed_attrs: if opts.signed_attributes {
            Some(attributes)
        } else {
            None
        },
        signature_algorithm: alg(signature_oid),
        signature: OctetString::new(signature).unwrap(),
        unsigned_attrs: None,
    };
    let mut certificates = vec![CertificateChoices::Certificate(ee)];
    if opts.include_intermediate {
        certificates.push(CertificateChoices::Certificate(inter));
    }
    let signed = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![alg(digest_oid)]).unwrap(),
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: "1.2.840.113549.1.7.1".parse().unwrap(),
            econtent: None,
        },
        certificates: Some(CertificateSet(SetOfVec::try_from(certificates).unwrap())),
        crls: None,
        signer_infos: SignerInfos(SetOfVec::try_from(vec![signer_info]).unwrap()),
    };
    let content_info = ContentInfo {
        content_type: "1.2.840.113549.1.7.2".parse().unwrap(),
        content: Any::encode_from(&signed).unwrap(),
    };
    Pkcs7Parts {
        rsa: content_info.to_der().unwrap(),
        sf,
        mf,
    }
}

// ---------------------------------------------------------------------------------------------
// Whole packages
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct SignSpec {
    pub cose: Option<CoseOptions>,
    pub pkcs7: Option<Pkcs7Options>,
}

impl SignSpec {
    pub fn cose() -> SignSpec {
        SignSpec {
            cose: Some(CoseOptions::default()),
            pkcs7: None,
        }
    }

    pub fn pkcs7() -> SignSpec {
        SignSpec {
            cose: None,
            pkcs7: Some(Pkcs7Options::default()),
        }
    }

    pub fn both() -> SignSpec {
        SignSpec {
            cose: Some(CoseOptions::default()),
            pkcs7: Some(Pkcs7Options::default()),
        }
    }
}

/// Signs `files`: the COSE files first (covering the add-on), then the PKCS#7 files (covering the
/// add-on and the COSE files), as AMO does. Returns the files with `META-INF/` added.
pub fn sign_package(mut files: Files, pki: &Pki, spec: &SignSpec) -> Files {
    if let Some(options) = &spec.cose {
        let manifest = jar_manifest(&files).into_bytes();
        let sig = cose_sig(&manifest, pki, options);
        files.push(("META-INF/cose.manifest".to_string(), manifest));
        files.push(("META-INF/cose.sig".to_string(), sig));
    }
    if let Some(options) = &spec.pkcs7 {
        let parts = pkcs7_parts(&files, pki, options);
        files.push(("META-INF/manifest.mf".to_string(), parts.mf));
        files.push(("META-INF/mozilla.sf".to_string(), parts.sf));
        files.push(("META-INF/mozilla.rsa".to_string(), parts.rsa));
    }
    files
}

/// A signed add-on ready to be zipped.
pub fn signed_files(pki: &Pki, manifest_version: u32, spec: &SignSpec) -> Files {
    sign_package(base_files(&pki.cn, manifest_version), pki, spec)
}

pub fn replace_file(files: &mut Files, name: &str, data: &[u8]) {
    for (n, d) in files.iter_mut() {
        if n == name {
            *d = data.to_vec();
            return;
        }
    }
    panic!("no file {name}");
}

pub fn remove_file(files: &mut Files, name: &str) {
    files.retain(|(n, _)| n != name);
}

pub fn file<'a>(files: &'a Files, name: &str) -> &'a [u8] {
    &files.iter().find(|(n, _)| n == name).unwrap().1
}
