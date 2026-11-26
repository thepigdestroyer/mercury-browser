// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The search terms shown in place of a results page's URL (draft for src/omnibox/results_page.rs).

use url::Url;

use crate::omnibox::modes::SearchModes;

const MAX_TERMS_CHARS: usize = 256;

impl SearchModes {
    /// The search terms to show in the field instead of `url`, when `url` is a results page of the
    /// default engine; `None` shows the URL.
    pub fn search_terms_for_url(&self, url: &Url) -> Option<String> {
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return None;
        }

        let engine = self
            .engines
            .iter()
            .find(|engine| engine.id == self.default_engine)
            .or_else(|| self.engines.first())?;
        let template = Url::parse(&engine.url_template.replace("{searchTerms}", "")).ok()?;

        if url.scheme() != template.scheme()
            || url.host_str() != template.host_str()
            || url.port_or_known_default() != template.port_or_known_default()
        {
            return None;
        }

        let raw = if engine.url_template.contains("?") {
            query_terms(url, &template, &engine.url_template)?
        } else {
            path_terms(url, &template, &engine.url_template)?
        };

        valid_terms(&raw)
    }
}

/// A query template: the path must match, every template parameter other than the placeholder
/// must equal the URL's, and the placeholder's value is the terms.
fn query_terms(url: &Url, template: &Url, raw_template: &str) -> Option<String> {
    if url.path() != template.path() {
        return None;
    }

    let placeholder_key = raw_template
        .split(['?', '&'])
        .find(|pair| pair.contains("{searchTerms}"))
        .and_then(|pair| pair.split('=').next())?;

    let mut terms = None;
    for (key, value) in url.query_pairs() {
        if key == placeholder_key {
            if terms.is_some() {
                return None;
            }
            terms = Some(value.into_owned());
        }
    }

    for (key, value) in template.query_pairs() {
        if key == placeholder_key {
            continue;
        }
        let matched = url.query_pairs().any(|(k, v)| k == key && v == value);
        if !matched {
            return None;
        }
    }

    terms
}

/// A path template such as `/search/{searchTerms}`: the path must have the template's prefix and
/// exactly one more segment, which is the terms.
fn path_terms(url: &Url, template: &Url, raw_template: &str) -> Option<String> {
    let prefix = raw_template.split("{searchTerms}").next()?;
    let prefix_path = Url::parse(prefix).ok().map(|url| url.path().to_owned())?;

    let rest = url.path().strip_prefix(prefix_path.as_str())?;
    if rest.is_empty() || rest.contains('/') || template.path().is_empty() {
        return None;
    }

    percent_decode(rest)
}

/// The decoded value must be 1..=256 characters with no control, bidirectional-format,
/// zero-width or replacement characters.
fn valid_terms(raw: &str) -> Option<String> {
    let terms = raw.trim();
    if terms.is_empty() || terms.chars().count() > MAX_TERMS_CHARS {
        return None;
    }

    let disallowed = |character: char| {
        character.is_control()
            || matches!(
                character,
                '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{FEFF}'
                    | '\u{FFFD}'
            )
    };
    if terms.chars().any(disallowed) {
        return None;
    }

    Some(terms.to_owned())
}

fn percent_decode(segment: &str) -> Option<String> {
    let decoded = Url::parse(&format!("https://x.test/{segment}"))
        .ok()?
        .path_segments()?
        .next()?
        .to_owned();
    let bytes = percent_decode_bytes(&decoded)?;
    String::from_utf8(bytes).ok()
}

fn percent_decode_bytes(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)?;
            let low = char::from(bytes.next()?).to_digit(16)?;
            out.push(u8::try_from(high * 16 + low).ok()?);
        } else {
            out.push(byte);
        }
    }
    Some(out)
}
