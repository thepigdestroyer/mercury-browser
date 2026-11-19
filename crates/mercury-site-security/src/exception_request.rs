// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The request Mercury's certificate page makes when the user continues to a site whose
//! certificate is not valid (see `error_pages`), and the answer the browser gives it.
//!
//! The request is `<scheme>:allow?secret=…&host=…&certificate=…`. It is answered with status 200
//! and the body `accepted` (`CERTIFICATE_EXCEPTION_ACCEPTED`), or, when refused, with
//! `ExceptionRefusal::status` and `ExceptionRefusal::code` as the body (`text/plain`). The page
//! looks the code up in a fixed table of its own lines and never shows the text of the answer;
//! a request without the secret gets a network error and nothing else.

use url::Url;

/// The body of the answer to a request the browser accepted.
pub const CERTIFICATE_EXCEPTION_ACCEPTED: &str = "accepted";

/// A certificate the user accepted for a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateException {
    /// The host the page was for, in ASCII, in the form certificate exceptions use
    /// (`talaria_core::certificate_exception_host`: lower-case, no trailing dot, IPv6 literals
    /// in brackets).
    pub host: String,
    /// The certificate, DER-encoded.
    pub certificate: Vec<u8>,
}

/// Why the engine refused to accept a certificate. The engine says it with a code (the `code`
/// of `servo-net`'s `CertificateExceptionRefusal`), which the handler passes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineRefusal {
    /// The engine cannot name the host (`invalid-host`).
    InvalidHost,
    /// The host asked to be reached only with a valid certificate, by HSTS or the preload list
    /// (`hsts-host`).
    Hsts,
    /// The engine has no record that this host failed with a certificate (`no-failure-recorded`).
    NoFailureRecorded,
    /// The host's failing certificate is not the one the user saw (`certificate-changed`).
    CertificateChanged,
    /// A code this version does not know.
    Other,
}

impl EngineRefusal {
    /// The refusal for an engine code; any code that is not one of the four is `Other`.
    pub fn from_code(code: &str) -> Self {
        match code {
            "invalid-host" => Self::InvalidHost,
            "hsts-host" => Self::Hsts,
            "no-failure-recorded" => Self::NoFailureRecorded,
            "certificate-changed" => Self::CertificateChanged,
            _ => Self::Other,
        }
    }

    /// The code the answer carries: the engine's own for the four it has, `refused` for `Other`.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidHost => "invalid-host",
            Self::Hsts => "hsts-host",
            Self::NoFailureRecorded => "no-failure-recorded",
            Self::CertificateChanged => "certificate-changed",
            Self::Other => "refused",
        }
    }
}

/// Why a request for an exception was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExceptionRefusal {
    /// Not `<scheme>:allow?…` with this run's secret: it did not come from Mercury's page. The
    /// browser answers it with a network error, which tells a caller that is not Mercury's page
    /// nothing.
    NotFromMercury,
    /// The host or the certificate is missing or does not decode, or the host is not one an
    /// exception can name.
    Malformed,
    /// The host is not on this computer or the local network, the only hosts that may get an
    /// exception (`talaria_core::certificate_exception_allowed`; the reason is there). Carries
    /// the host in its exception form.
    PublicHost(String),
    /// The engine refused to accept the certificate.
    Engine(EngineRefusal),
}

impl ExceptionRefusal {
    /// Every code `code` can give, for the certificate page's table of lines.
    pub const CODES: &'static [&'static str] = &[
        "not-from-mercury",
        "malformed",
        "public-host",
        "invalid-host",
        "hsts-host",
        "no-failure-recorded",
        "certificate-changed",
        "refused",
    ];

    /// The status of the answer: 400 for a request that is not well formed, 403 for every
    /// refusal of the browser's policy or the engine (and for a request that did not come from
    /// Mercury, which is answered with a network error and never with this status).
    pub fn status(&self) -> u16 {
        match self {
            Self::Malformed => 400,
            Self::NotFromMercury | Self::PublicHost(_) | Self::Engine(_) => 403,
        }
    }

    /// The body of the answer: one word out of `CODES`, fixed per refusal. Nothing from the
    /// request or the engine is ever put in it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFromMercury => "not-from-mercury",
            Self::Malformed => "malformed",
            Self::PublicHost(_) => "public-host",
            Self::Engine(refusal) => refusal.code(),
        }
    }
}

/// The exception `url` asks for: `<scheme>:allow?…` carrying `secret`, a host that
/// `talaria_core::certificate_exception_allowed` accepts (the host is returned in its exception
/// form) and a certificate that decodes. Whether the engine accepts the certificate for the host
/// is its decision, not this function's.
pub fn parse_certificate_exception_request(
    url: &Url,
    secret: &str,
) -> Result<CertificateException, ExceptionRefusal> {
    if url.scheme() != crate::CERTIFICATE_EXCEPTION_SCHEME || url.path() != "allow" {
        return Err(ExceptionRefusal::NotFromMercury);
    }
    let mut request_secret = None;
    let mut host = None;
    let mut certificate = None;
    for (name, value) in url.query_pairs() {
        match name.as_ref() {
            "secret" => request_secret = Some(value.into_owned()),
            "host" => host = Some(value.into_owned()),
            "certificate" => certificate = Some(value.into_owned()),
            _ => {}
        }
    }
    let request_secret = request_secret.ok_or(ExceptionRefusal::NotFromMercury)?;
    if secret.is_empty()
        || request_secret.len() != secret.len()
        || request_secret
            .bytes()
            .zip(secret.bytes())
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            != 0
    {
        return Err(ExceptionRefusal::NotFromMercury);
    }
    // The one form of the host every part of the browser uses; text that is not a host cannot
    // be named by an exception.
    let host = host
        .filter(|host| !host.is_empty())
        .and_then(|host| talaria_core::certificate_exception_host(&host))
        .ok_or(ExceptionRefusal::Malformed)?;
    let certificate = certificate
        .and_then(|certificate| decode_base64(&certificate))
        .filter(|bytes| !bytes.is_empty())
        .ok_or(ExceptionRefusal::Malformed)?;
    if !talaria_core::certificate_exception_allowed(&host) {
        return Err(ExceptionRefusal::PublicHost(host));
    }
    Ok(CertificateException { host, certificate })
}

/// Standard base64, with or without padding; None for anything else.
pub(crate) fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut bytes = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0_u32;
    let mut bits = 0;
    for character in text.bytes() {
        let value = match character {
            b'A'..=b'Z' => character - b'A',
            b'a'..=b'z' => character - b'a' + 26,
            b'0'..=b'9' => character - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    // A single leftover character cannot encode a byte.
    (bits < 6).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::decode_base64;

    #[test]
    fn decodes_standard_base64_with_and_without_padding() {
        assert_eq!(decode_base64("").as_deref(), Some(&b""[..]));
        assert_eq!(decode_base64("TQ").as_deref(), Some(&b"M"[..]));
        assert_eq!(decode_base64("TQ==").as_deref(), Some(&b"M"[..]));
        assert_eq!(decode_base64("TWE").as_deref(), Some(&b"Ma"[..]));
        assert_eq!(decode_base64("TWFu").as_deref(), Some(&b"Man"[..]));
        assert_eq!(decode_base64("+/8").as_deref(), Some(&[0xfb, 0xff][..]));
        assert_eq!(decode_base64("T"), None);
        assert_eq!(decode_base64("TW-u"), None);
    }
}
