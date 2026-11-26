// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The TLS facts of a main-frame connection (NET-2 EC26) and their summary for the padlock details.

use std::fmt;

/// The TLS facts the engine reports for the connection of a main-frame document.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct TlsFacts {
    pub protocol_version: Option<String>,
    pub cipher_suite: Option<String>,
    pub key_exchange_group: Option<String>,
    pub alpn_protocol: Option<String>,
    pub hsts: bool,
    /// The certificate chain as DER, leaf first. Never logged or shown.
    pub certificate_chain_der: Vec<Vec<u8>>,
}

impl fmt::Debug for TlsFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let chain = certificate_count(self.certificate_chain_der.len());
        f.debug_struct("TlsFacts")
            .field("protocol_version", &self.protocol_version)
            .field("cipher_suite", &self.cipher_suite)
            .field("key_exchange_group", &self.key_exchange_group)
            .field("alpn_protocol", &self.alpn_protocol)
            .field("hsts", &self.hsts)
            .field("certificate_chain_der", &chain)
            .finish()
    }
}

fn certificate_count(count: usize) -> String {
    if count == 1 {
        "1 certificate".to_owned()
    } else {
        format!("{count} certificates")
    }
}

/// What the padlock details show: one headline and labelled rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsSummary {
    pub headline: String,
    pub details: Vec<(String, String)>,
}

/// Summarises the facts. Certificates are not parsed: the chain length and the leaf size only.
pub fn summarize_tls(facts: &TlsFacts) -> TlsSummary {
    let headline = match facts.protocol_version.as_deref() {
        Some(protocol) => format!("Encrypted with {protocol}"),
        None => "Connection details unknown".to_owned(),
    };
    let mut details = Vec::new();
    push_detail(&mut details, "Protocol", facts.protocol_version.as_deref());
    push_detail(&mut details, "Cipher suite", facts.cipher_suite.as_deref());
    push_detail(
        &mut details,
        "Key exchange",
        facts.key_exchange_group.as_deref(),
    );
    push_detail(&mut details, "ALPN", facts.alpn_protocol.as_deref());
    let hsts = if facts.hsts { "on" } else { "off" };
    details.push(("HSTS".to_owned(), hsts.to_owned()));
    if let Some(leaf) = facts.certificate_chain_der.first() {
        let chain = facts.certificate_chain_der.len();
        let value = format!("{chain} in chain, leaf {} bytes", leaf.len());
        details.push(("Certificates".to_owned(), value));
    }
    TlsSummary { headline, details }
}

fn push_detail(details: &mut Vec<(String, String)>, label: &str, value: Option<&str>) {
    if let Some(value) = value {
        details.push((label.to_owned(), value.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail<'a>(summary: &'a TlsSummary, label: &str) -> Option<&'a str> {
        summary
            .details
            .iter()
            .find(|(name, _)| name == label)
            .map(|(_, value)| value.as_str())
    }

    fn negotiated() -> TlsFacts {
        TlsFacts {
            protocol_version: Some("TLSv1.3".to_owned()),
            cipher_suite: Some("TLS_AES_128_GCM_SHA256".to_owned()),
            key_exchange_group: Some("X25519".to_owned()),
            alpn_protocol: Some("h2".to_owned()),
            hsts: true,
            certificate_chain_der: vec![vec![0x30, 0x82], vec![0x30, 0x81, 0x9f]],
        }
    }

    #[test]
    fn summarize_tls_names_the_protocol_cipher_and_alpn() {
        let summary = summarize_tls(&negotiated());
        assert!(summary.headline.contains("TLSv1.3"), "{}", summary.headline);
        assert_eq!(detail(&summary, "Protocol"), Some("TLSv1.3"));
        assert_eq!(
            detail(&summary, "Cipher suite"),
            Some("TLS_AES_128_GCM_SHA256")
        );
        assert_eq!(detail(&summary, "Key exchange"), Some("X25519"));
        assert_eq!(detail(&summary, "ALPN"), Some("h2"));
        assert_eq!(detail(&summary, "HSTS"), Some("on"));
        assert_eq!(
            detail(&summary, "Certificates"),
            Some("2 in chain, leaf 2 bytes")
        );
    }

    #[test]
    fn summarize_tls_with_nothing_says_unknown_not_secure() {
        let summary = summarize_tls(&TlsFacts::default());
        let headline = summary.headline.to_lowercase();
        assert!(headline.contains("unknown"), "{}", summary.headline);
        assert!(!headline.contains("secure"), "{}", summary.headline);
    }

    #[test]
    fn debug_of_tls_facts_hides_the_chain_bytes() {
        let facts = TlsFacts {
            certificate_chain_der: vec![vec![231, 242, 253]],
            ..TlsFacts::default()
        };
        let shown = format!("{facts:?}");
        assert!(!shown.contains("231"), "{shown}");
        assert!(!shown.contains("242"), "{shown}");
        assert!(!shown.contains("253"), "{shown}");
        assert!(shown.contains("1 certificate"), "{shown}");
    }
}
