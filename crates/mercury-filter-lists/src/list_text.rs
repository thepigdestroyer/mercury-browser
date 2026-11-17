//! What a list's text says about itself, and whether a downloaded copy may replace the copy in
//! use.

use adblock::lists::{ParseOptions, ParsedLine, RuleTypes, parse_filter};

/// The largest list accepted, in bytes. The largest bundled list, EasyList, is about 2 MB.
pub const MAX_LIST_BYTES: usize = 16 * 1024 * 1024;

/// How many lines at the top of a list are searched for its version header.
const HEADER_LINES: usize = 50;
const MAX_VERSION_BYTES: usize = 64;

/// What Mercury uses of a list: how many network rules it holds (cosmetic rules are not used)
/// and its version header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListSummary {
    pub network_rules: usize,
    pub version: Option<String>,
}

/// Why a downloaded list was not used. The copy in use stays.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    #[error("it is larger than {MAX_LIST_BYTES} bytes")]
    TooLarge,
    #[error("it is not UTF-8 text")]
    NotText,
    #[error("it looks like a web page, not a filter list")]
    LooksLikeHtml,
    #[error("it has {got} network rules, fewer than half of the {current} in the copy in use")]
    TooFewRules { got: usize, current: usize },
}

/// Counts the network rules Mercury would load from `text` and reads its version header.
pub fn summarize(text: &str) -> ListSummary {
    ListSummary {
        network_rules: count_network_rules(text),
        version: header_version(text),
    }
}

/// The network rules Mercury would load from `text`.
pub fn count_network_rules(text: &str) -> usize {
    text.trim_start_matches('\u{feff}')
        .lines().filter(|line| is_network_rule(line)).count()
}

/// `! Version: …`, or else a `! Last modified: …` that is not a template placeholder, in the
/// first lines of the list.
pub fn header_version(text: &str) -> Option<String> {
    let header = |prefix: &str| {
        text.lines()
            .take(HEADER_LINES)
            .filter_map(|line| line.trim_start_matches('\u{feff}').strip_prefix('!'))
            .filter_map(|line| line.trim().strip_prefix(prefix))
            .map(str::trim)
            .find(|value| {
                !value.is_empty()
                    && value.len() <= MAX_VERSION_BYTES
                    && !value.contains('%')
                    && !value.chars().any(char::is_control)
            })
            .map(str::to_owned)
    };
    header("Version:").or_else(|| header("Last modified:"))
}

/// Checks a downloaded list against the copy in use, which has `current_rules` network rules.
///
/// Accepted only if it is at most [`MAX_LIST_BYTES`], UTF-8, does not start like an HTML page
/// (a captive portal or an error page served with status 200), and has at least half as many
/// network rules as the copy in use. So a truncated or emptied download never replaces a
/// working list and silently turns blocking off.
pub fn check_download(
    bytes: &[u8],
    current_rules: usize,
) -> Result<(String, ListSummary), Rejection> {
    let text = check_text(bytes)?;
    let summary = summarize(&text);
    check_shrink(summary.network_rules, current_rules)?;
    Ok((text, summary))
}

/// The checks every downloaded list gets whatever its kind: size, UTF-8, not a web page.
pub fn check_text(bytes: &[u8]) -> Result<String, Rejection> {
    if bytes.len() > MAX_LIST_BYTES {
        return Err(Rejection::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Rejection::NotText)?;
    let start = text.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    let lowered = start
        .get(..start.len().min(16))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if lowered.starts_with("<!doctype") || lowered.starts_with("<html") || lowered.starts_with("<?xml")
    {
        return Err(Rejection::LooksLikeHtml);
    }
    Ok(text.to_owned())
}

/// The shrink rule: a list with fewer than half the usable rules of the copy in use is refused.
pub fn check_shrink(got: usize, current: usize) -> Result<(), Rejection> {
    if got.saturating_mul(2) < current {
        Err(Rejection::TooFewRules { got, current })
    } else {
        Ok(())
    }
}

/// The options lists are parsed with: network rules only, as the engine is built.
pub(crate) fn network_options() -> ParseOptions {
    ParseOptions {
        rule_types: RuleTypes::NetworkOnly,
        ..ParseOptions::default()
    }
}

fn is_network_rule(line: &str) -> bool {
    matches!(
        parse_filter(line, false, network_options()),
        Ok(ParsedLine::Network(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const EASYLIST_HEAD: &str = "[Adblock Plus 2.0]\n! Version: 202611152034\n! Title: EasyList\n\
        ! Last modified: 15 Nov 2026 20:34 UTC\n";

    fn rules(count: usize) -> String {
        (0..count)
            .map(|index| format!("||ads{index}.example^\n"))
            .collect()
    }

    #[test]
    fn network_rules_are_counted_and_comments_and_cosmetic_rules_are_not() {
        let text = format!(
            "{EASYLIST_HEAD}! a comment\n\n||tracker.example^$third-party\n/banner_ad.\n\
             example.com##.ad\n##.sponsored\n@@||example.com/ok.js\n"
        );

        let summary = summarize(&text);

        assert_eq!(summary.network_rules, 3);
        assert_eq!(summary.version.as_deref(), Some("202611152034"));
    }

    #[test]
    fn the_version_falls_back_to_a_real_last_modified_date() {
        assert_eq!(
            summarize("! Title: X\n! Last modified: 15 Nov 2026 20:34 UTC\n").version.as_deref(),
            Some("15 Nov 2026 20:34 UTC")
        );
        // uBlock's unexpanded placeholder is not a date.
        assert_eq!(summarize("! Title: X\n! Last modified: %timestamp%\n").version, None);
        assert_eq!(summarize("! Title: Brave Specific\n").version, None);
    }

    #[test]
    fn a_version_far_down_or_too_long_is_ignored() {
        let late = format!("{}! Version: 1\n", "! filler\n".repeat(HEADER_LINES));
        assert_eq!(summarize(&late).version, None);
        let long = format!("! Version: {}\n", "9".repeat(MAX_VERSION_BYTES + 1));
        assert_eq!(summarize(&long).version, None);
    }

    #[test]
    fn a_download_with_at_least_half_the_rules_is_accepted() {
        let text = format!("{EASYLIST_HEAD}{}", rules(50));

        let (accepted, summary) = check_download(text.as_bytes(), 100).expect("accepted");

        assert_eq!(accepted, text);
        assert_eq!(summary.network_rules, 50);
    }

    #[test]
    fn a_truncated_download_is_refused() {
        let text = format!("{EASYLIST_HEAD}{}", rules(49));
        assert_eq!(
            check_download(text.as_bytes(), 100),
            Err(Rejection::TooFewRules {
                got: 49,
                current: 100
            })
        );
        assert_eq!(
            check_download(EASYLIST_HEAD.as_bytes(), 1),
            Err(Rejection::TooFewRules { got: 0, current: 1 })
        );
    }

    #[test]
    fn a_list_without_network_rules_may_replace_one_without_them() {
        assert!(check_download(b"! Brave desktop filters\n", 0).is_ok());
    }

    #[test]
    fn web_pages_binary_data_and_huge_files_are_refused() {
        assert_eq!(
            check_download(b"<!DOCTYPE html><html><body>Sign in to Wi-Fi</body></html>", 0),
            Err(Rejection::LooksLikeHtml)
        );
        assert_eq!(
            check_download(b"\n\n  <html>\n", 0),
            Err(Rejection::LooksLikeHtml)
        );
        assert_eq!(check_download(&[0xff, 0xfe, 0x00], 0), Err(Rejection::NotText));
        let huge = vec![b'!'; MAX_LIST_BYTES + 1];
        assert_eq!(check_download(&huge, 0), Err(Rejection::TooLarge));
    }

    #[test]
    fn a_byte_order_mark_is_accepted() {
        let text = format!("\u{feff}{EASYLIST_HEAD}{}", rules(2));
        assert_eq!(
            check_download(text.as_bytes(), 2).map(|(_, summary)| summary.network_rules),
            Ok(2)
        );
    }
}
