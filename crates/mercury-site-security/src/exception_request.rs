// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The request Mercury's certificate page makes when the user continues to a site whose
//! certificate is not valid (see `error_pages`).

use url::Url;

/// A certificate the user accepted for a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateException {
    /// The host the page was for, in ASCII.
    pub host: String,
    /// The certificate, DER-encoded.
    pub certificate: Vec<u8>,
}

/// Why a request for an exception was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExceptionRefusal {
    /// Not `<scheme>:allow?…` with this run's secret: it did not come from Mercury's page.
    NotFromMercury,
    /// The host or the certificate is missing or does not decode.
    Malformed,
    /// The host is on the public internet. Servo 0.8 accepts an exception for the certificate
    /// on every host, so one accepted for a public site would let whoever holds its key pass
    /// for any site; until the engine binds exceptions to a host, only names on this computer
    /// or the local network (`talaria_core::is_local_host`) get one.
    PublicHost(String),
}

/// The exception `url` asks for: `<scheme>:allow?…` carrying `secret`, a local host and a
/// certificate that decodes.
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
    let host = host
        .filter(|host| !host.is_empty())
        .ok_or(ExceptionRefusal::Malformed)?
        .to_ascii_lowercase();
    let certificate = certificate
        .and_then(|certificate| decode_base64(&certificate))
        .filter(|bytes| !bytes.is_empty())
        .ok_or(ExceptionRefusal::Malformed)?;
    if !talaria_core::is_local_host(&host) {
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
