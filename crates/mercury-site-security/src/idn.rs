// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! How a host name is shown to the user.
//!
//! The engine reports every host in its ASCII form, where an internationalised label is written
//! as punycode (`xn--…`). Mercury shows such a label in its own script only when it cannot pass
//! for another name; otherwise the whole host stays in punycode, so a look-alike of a known name
//! is visibly different. A label is shown in Unicode when all of these hold:
//!
//! 1. It decodes cleanly from punycode.
//! 2. Every character is an ASCII digit, a hyphen, a combining mark, or a letter of a script
//!    that Unicode recommends for identifiers (UAX #31, table 5). Symbols, punctuation and
//!    other "Common" characters (a middle dot, a fraction slash, a heart) are not allowed.
//! 3. Its scripts form one of the "moderately restrictive" combinations of UTS #39 §5.2 (the
//!    rule Firefox applies): a single script; Latin with Han, Hiragana and Katakana (Japanese);
//!    Latin with Han and Bopomofo (Chinese); Latin with Han and Hangul (Korean); or Latin with
//!    one other script that is neither Cyrillic nor Greek. Characters used by several scripts
//!    (the Japanese prolonged sound mark, say) count as whichever of them the label already
//!    uses.
//! 4. Combining marks do not follow Latin, Greek or Cyrillic letters: names are normalised
//!    before they reach Mercury, so a mark left there has no precomposed letter and is a common
//!    way to fake one (`q̇`).
//! 5. It is not a whole-script look-alike: a label written only in Cyrillic (or only in Greek)
//!    letters that look like Latin ones (`аррӏе`, Cyrillic) is shown in punycode unless the
//!    top-level domain is in the same script (`сосо.рф`). This is the check Chrome makes for
//!    Cyrillic; Mercury extends it to Greek.
//!
//! One label that fails shows the whole host in punycode. What this does not catch: a name
//! that differs from a well-known one only by an accented Latin letter (`ãpple.com`). Chrome
//! compares names against a list of popular domains for that; Mercury has no such list.

use unicode_script::{Script, UnicodeScript};
use url::{Position, Url};

/// Cyrillic letters that look like lower-case Latin ones.
const CYRILLIC_LATIN_LOOK_ALIKES: &str = "асԁеһіјӏорԛѕԝхуъьҽпгѵѡ";

/// Greek letters that look like lower-case Latin ones.
const GREEK_LATIN_LOOK_ALIKES: &str = "αεικνορτυχ";

const JAPANESE: &[Script] = &[
    Script::Latin,
    Script::Han,
    Script::Hiragana,
    Script::Katakana,
];
const CHINESE: &[Script] = &[Script::Latin, Script::Han, Script::Bopomofo];
const KOREAN: &[Script] = &[Script::Latin, Script::Han, Script::Hangul];

/// The host as the user should read it: `ascii_host` (as the engine reports it) with its
/// punycode labels decoded, or unchanged when any label could be mistaken for another name.
pub fn display_host(ascii_host: &str) -> String {
    if !ascii_host.contains("xn--") {
        return ascii_host.to_owned();
    }
    let Some(labels) = ascii_host
        .split('.')
        .map(decode_label)
        .collect::<Option<Vec<_>>>()
    else {
        return ascii_host.to_owned();
    };
    let top_level = labels.last().map(String::as_str).unwrap_or_default();
    if labels.iter().all(|label| label_is_safe(label, top_level)) {
        labels.join(".")
    } else {
        ascii_host.to_owned()
    }
}

/// `url` with its host shown as `display_host` shows it; anything else unchanged. Text that is
/// not a URL is returned as it is.
pub fn display_url(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return url.to_owned();
    };
    let Some(host) = parsed.host_str() else {
        return url.to_owned();
    };
    let shown = display_host(host);
    if shown == host {
        return url.to_owned();
    }
    format!(
        "{}{}{}",
        &parsed[..Position::BeforeHost],
        shown,
        &parsed[Position::AfterHost..]
    )
}

/// A label in Unicode, or None if it is punycode that does not decode.
fn decode_label(label: &str) -> Option<String> {
    let Some(encoded) = label.strip_prefix("xn--") else {
        return Some(label.to_owned());
    };
    if encoded.is_empty() {
        return None;
    }
    let decoded = url::quirks::domain_to_unicode(label);
    let failed = decoded.is_empty()
        || decoded.starts_with("xn--")
        || decoded.contains('.')
        || decoded.contains('\u{fffd}');
    (!failed).then_some(decoded)
}

fn label_is_safe(label: &str, top_level: &str) -> bool {
    let mut scripts: Vec<Script> = Vec::new();
    // Characters that belong to several scripts, resolved once the single-script ones are known.
    let mut shared: Vec<Vec<Script>> = Vec::new();
    let mut has_combining_mark = false;
    for character in label.chars() {
        if character.is_ascii_digit() || character == '-' {
            continue;
        }
        if character.script() == Script::Inherited {
            has_combining_mark = true;
            continue;
        }
        let extension = character.script_extension();
        // A "Common" character is allowed only as a letter of particular scripts (the Japanese
        // prolonged sound mark); a middle dot also lists scripts, but it is punctuation.
        let common_but_not_a_letter =
            character.script() == Script::Common && !character.is_alphabetic();
        if common_but_not_a_letter || extension.is_common() || extension.is_empty() {
            return false;
        }
        let candidates = extension
            .iter()
            .filter(|script| script.is_recommended())
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [] => return false,
            [script] => add(&mut scripts, *script),
            _ => shared.push(candidates),
        }
    }
    for candidates in shared {
        if !candidates.iter().any(|script| scripts.contains(script)) {
            add(&mut scripts, candidates[0]);
        }
    }
    if !allowed_combination(&scripts) {
        return false;
    }
    let latin_like = [Script::Latin, Script::Greek, Script::Cyrillic];
    if has_combining_mark && scripts.iter().any(|script| latin_like.contains(script)) {
        return false;
    }
    !is_whole_script_look_alike(label, &scripts, top_level)
}

fn add(scripts: &mut Vec<Script>, script: Script) {
    if !scripts.contains(&script) {
        scripts.push(script);
    }
}

fn allowed_combination(scripts: &[Script]) -> bool {
    let within = |allowed: &[Script]| scripts.iter().all(|script| allowed.contains(script));
    match scripts {
        [] | [_] => true,
        _ if within(JAPANESE) || within(CHINESE) || within(KOREAN) => true,
        [first, second] => {
            let other = if *first == Script::Latin {
                *second
            } else if *second == Script::Latin {
                *first
            } else {
                return false;
            };
            !matches!(other, Script::Cyrillic | Script::Greek)
        }
        _ => false,
    }
}

fn is_whole_script_look_alike(label: &str, scripts: &[Script], top_level: &str) -> bool {
    let (script, look_alikes) = match scripts {
        [Script::Cyrillic] => (Script::Cyrillic, CYRILLIC_LATIN_LOOK_ALIKES),
        [Script::Greek] => (Script::Greek, GREEK_LATIN_LOOK_ALIKES),
        _ => return false,
    };
    if top_level
        .chars()
        .any(|character| character.script() == script)
    {
        return false;
    }
    label
        .chars()
        .filter(|character| !character.is_ascii_digit() && *character != '-')
        .all(|character| look_alikes.contains(character))
}
