// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The trust anchors for add-on signatures: Mozilla's add-ons production root and the intermediates it
//! signed, bundled as data (decision D4, OD-29).
//!
//! The three certificates are public data copied byte for byte from Firefox's tree
//! (`security/manager/ssl/addons-public*.pem`); see `data/README.md` for the source commit, the
//! fingerprints, the licence note and how to update them. The pinned fingerprints are checked by a test.
//! The stage root (`addons-stage.pem`) is not bundled.
//!
//! Like Firefox's `AppTrustDomain`, the root is the only trust anchor, and the bundled intermediates are
//! offered as candidates when a chain is built, so an XPI whose own copy of an intermediate expired still
//! verifies. Tests replace all of this with [`TrustAnchors::from_der`].

use std::sync::OnceLock;

use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::der::{Decode, pem};

const ROOT_PEM: &[u8] = include_bytes!("../data/addons-public.pem");
const INTERMEDIATE_PEM: &[u8] = include_bytes!("../data/addons-public-intermediate.pem");
const INTERMEDIATE_2018_PEM: &[u8] = include_bytes!("../data/addons-public-2018-intermediate.pem");

/// Why a certificate could not be loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RootsError {
    /// The PEM text is not a single certificate.
    #[error("the PEM text is not a valid certificate")]
    BadPem,
    /// The DER bytes are not an X.509 certificate.
    #[error("the bytes are not a valid X.509 certificate")]
    BadCertificate,
}

/// A parsed certificate together with the DER bytes it was parsed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedCert {
    der: Vec<u8>,
    cert: Certificate,
}

impl TrustedCert {
    /// Parses one DER certificate.
    pub fn from_der(der: &[u8]) -> Result<TrustedCert, RootsError> {
        let cert = Certificate::from_der(der).map_err(|_| RootsError::BadCertificate)?;
        Ok(TrustedCert {
            der: der.to_vec(),
            cert,
        })
    }

    /// Parses one PEM certificate.
    pub fn from_pem(text: &[u8]) -> Result<TrustedCert, RootsError> {
        let (label, der) = pem::decode_vec(text).map_err(|_| RootsError::BadPem)?;
        if label != "CERTIFICATE" {
            return Err(RootsError::BadPem);
        }
        TrustedCert::from_der(&der)
    }

    /// The DER bytes.
    pub fn der(&self) -> &[u8] {
        &self.der
    }

    /// The parsed certificate.
    pub fn certificate(&self) -> &Certificate {
        &self.cert
    }

    /// SHA-256 of the DER bytes: the fingerprint Firefox and `openssl x509 -fingerprint -sha256` show.
    pub fn sha256(&self) -> [u8; 32] {
        Sha256::digest(&self.der).into()
    }
}

/// The certificates a signature chain may end in: trust anchors (roots), and intermediates offered as
/// candidates when a chain is built.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrustAnchors {
    /// Trusted roots. A chain is valid when it ends in one of these (or is signed by one).
    pub roots: Vec<TrustedCert>,
    /// Known intermediates, tried when the signature's own certificates do not complete the chain.
    pub intermediates: Vec<TrustedCert>,
}

impl TrustAnchors {
    /// Anchors from DER bytes. For tests, which bring their own root.
    pub fn from_der(
        roots: &[Vec<u8>],
        intermediates: &[Vec<u8>],
    ) -> Result<TrustAnchors, RootsError> {
        let parse = |list: &[Vec<u8>]| {
            list.iter()
                .map(|der| TrustedCert::from_der(der))
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(TrustAnchors {
            roots: parse(roots)?,
            intermediates: parse(intermediates)?,
        })
    }

    /// Mozilla's add-ons production root with its two bundled intermediates.
    pub fn mozilla() -> Result<TrustAnchors, RootsError> {
        let root = mozilla_addons_root()?.clone();
        let intermediates = mozilla_intermediates()?.to_vec();
        Ok(TrustAnchors {
            roots: vec![root],
            intermediates,
        })
    }
}

struct Bundled {
    root: TrustedCert,
    intermediates: Vec<TrustedCert>,
}

fn bundled() -> Result<&'static Bundled, RootsError> {
    static BUNDLED: OnceLock<Result<Bundled, RootsError>> = OnceLock::new();
    BUNDLED
        .get_or_init(|| {
            Ok(Bundled {
                root: TrustedCert::from_pem(ROOT_PEM)?,
                intermediates: vec![
                    TrustedCert::from_pem(INTERMEDIATE_PEM)?,
                    TrustedCert::from_pem(INTERMEDIATE_2018_PEM)?,
                ],
            })
        })
        .as_ref()
        .map_err(|e| *e)
}

/// `CN=root-ca-production-amo`, valid until 3 December 2200. Parsed once.
pub fn mozilla_addons_root() -> Result<&'static TrustedCert, RootsError> {
    bundled().map(|b| &b.root)
}

/// The two intermediates of Mozilla's add-ons production root (`signingca1.addons.mozilla.org` and
/// `production-signing-ca.addons.mozilla.org`). Parsed once.
pub fn mozilla_intermediates() -> Result<&'static [TrustedCert], RootsError> {
    bundled().map(|b| b.intermediates.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    }

    #[test]
    fn bundled_root_fingerprint_is_pinned() {
        let root = mozilla_addons_root().unwrap();
        assert_eq!(
            hex(&root.sha256()),
            "C8:A8:0E:9A:FA:EF:4E:21:9B:6F:B5:D7:A7:1D:0F:10:12:23:BA:C5:00:1A:C2:8F:9B:0D:43:DC:59:A1:06:DB"
        );
    }

    #[test]
    fn bundled_intermediate_fingerprints_are_pinned() {
        let list = mozilla_intermediates().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(
            hex(&list[0].sha256()),
            "16:76:1B:C9:7F:84:D1:4E:FB:88:8F:36:FC:92:3A:97:18:65:AE:00:1C:92:88:FC:4F:A4:72:12:95:5B:F4:A2"
        );
        assert_eq!(
            hex(&list[1].sha256()),
            "70:C7:FA:72:BC:3C:71:B3:0E:7D:45:76:2B:92:61:B9:D8:39:54:4B:35:C4:3F:27:F7:F3:9D:9C:EA:1E:31:D0"
        );
    }

    #[test]
    fn the_root_is_self_signed_and_the_intermediates_are_issued_by_it() {
        let root = mozilla_addons_root().unwrap();
        let tbs = root.certificate().tbs_certificate();
        assert_eq!(tbs.subject(), tbs.issuer());
        for intermediate in mozilla_intermediates().unwrap() {
            assert_eq!(
                intermediate.certificate().tbs_certificate().issuer(),
                tbs.subject()
            );
        }
    }

    #[test]
    fn trust_anchors_mozilla_has_one_root_and_two_intermediates() {
        let anchors = TrustAnchors::mozilla().unwrap();
        assert_eq!(anchors.roots.len(), 1);
        assert_eq!(anchors.intermediates.len(), 2);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert_eq!(
            TrustedCert::from_der(b"nope"),
            Err(RootsError::BadCertificate)
        );
        assert_eq!(TrustedCert::from_pem(b"nope"), Err(RootsError::BadPem));
        assert_eq!(
            TrustedCert::from_pem(
                b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n"
            ),
            Err(RootsError::BadPem)
        );
        assert!(TrustAnchors::from_der(&[vec![1, 2, 3]], &[]).is_err());
    }
}
