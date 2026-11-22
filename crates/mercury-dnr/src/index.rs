// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A token index over the URL filters of a ruleset, so a request looks at a few rules instead of all.
//!
//! The idea is the one of Chromium's `url_pattern_index` (behaviour per its documentation; no code is
//! copied): every filter is stored under its most selective literal token, and a request is tokenised
//! once. A *token* is a maximal run of ASCII letters and digits of at least [`MIN_TOKEN_LEN`] characters.
//! A filter may be stored under a token only if every URL the filter matches must contain that token as
//! a **whole** maximal run: the run is bounded on the left by a non-alphanumeric character inside the
//! literal, or by the start of the literal when the filter has a left anchor (`|` or `||`) or a `^`
//! before it; and on the right by a non-alphanumeric character inside the literal, or by the end of the
//! literal when the filter has a right anchor (`|`) or a `^` after it. A literal next to a `*`, or at an
//! unanchored end, gives no usable token there (`ads` in `*ads*` may be part of `uploads`).
//!
//! Filters with no usable token, regex filters and rules without a URL condition go to a fallback list
//! that every lookup returns. The index is a performance structure only: [`RuleIndex::candidates`] returns
//! a **superset** of the filters that match, never fewer, and the differential test in this module checks
//! that on a generated corpus.

use std::collections::{HashMap, HashSet};

use crate::url_filter::{Piece, UrlFilter, UrlPattern};

/// The shortest token that is indexed.
pub const MIN_TOKEN_LEN: usize = 3;

/// A token index. Entries are positions (`u32`) in the caller's list of rules.
#[derive(Debug, Clone, Default)]
pub struct RuleIndex {
    by_token: HashMap<Vec<u8>, Vec<u32>>,
    fallback: Vec<u32>,
}

impl RuleIndex {
    /// Builds an index. `patterns` gives, for each rule in order, its URL condition (`None` when it has
    /// none). The position of a rule in the iterator is the id [`candidates`](Self::candidates) returns.
    pub fn build<'a, I>(patterns: I) -> RuleIndex
    where
        I: IntoIterator<Item = Option<&'a UrlPattern>>,
    {
        let mut index = RuleIndex::default();
        for (position, pattern) in patterns.into_iter().enumerate() {
            let Ok(position) = u32::try_from(position) else {
                break;
            };
            let token = match pattern {
                Some(UrlPattern::Filter(filter)) => select_token(filter),
                _ => None,
            };
            match token {
                Some(token) => index.by_token.entry(token).or_default().push(position),
                None => index.fallback.push(position),
            }
        }
        index
    }

    /// Positions of the rules that may match a URL, sorted, without duplicates. `text` is the URL
    /// without its fragment.
    pub fn candidates(&self, text: &str) -> Vec<u32> {
        let mut out = self.fallback.clone();
        if !self.by_token.is_empty() {
            let lower = text.to_ascii_lowercase();
            let mut seen: HashSet<&[u8]> = HashSet::new();
            for run in alnum_runs(lower.as_bytes()) {
                if run.len() < MIN_TOKEN_LEN || !seen.insert(run) {
                    continue;
                }
                if let Some(bucket) = self.by_token.get(run) {
                    out.extend_from_slice(bucket);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// How many rules are stored under a token.
    pub fn indexed_len(&self) -> usize {
        self.by_token.values().map(Vec::len).sum()
    }

    /// How many rules are in the fallback list.
    pub fn fallback_len(&self) -> usize {
        self.fallback.len()
    }
}

/// The maximal runs of ASCII letters and digits in `bytes`.
fn alnum_runs(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(|b| !b.is_ascii_alphanumeric())
        .filter(|run| !run.is_empty())
}

/// The most selective token a filter may be stored under (the longest usable one; the leftmost on a
/// tie), lowercased; `None` when the filter has no usable token.
pub(crate) fn select_token(filter: &UrlFilter) -> Option<Vec<u8>> {
    let pieces = filter.pieces();
    let mut best: Option<&[u8]> = None;
    for (i, piece) in pieces.iter().enumerate() {
        let Piece::Literal(lit) = piece else {
            continue;
        };
        let left_closed = match i.checked_sub(1).and_then(|p| pieces.get(p)) {
            None => filter.has_left_anchor(),
            Some(Piece::Separator) => true,
            Some(_) => false,
        };
        let right_closed = match pieces.get(i + 1) {
            None => filter.has_end_anchor(),
            Some(Piece::Separator) => true,
            Some(_) => false,
        };
        let mut start = 0usize;
        while start < lit.len() {
            let rest = lit.get(start..).unwrap_or_default();
            let Some(offset) = rest.iter().position(|b| b.is_ascii_alphanumeric()) else {
                break;
            };
            let run_start = start + offset;
            let run_len = lit
                .get(run_start..)
                .unwrap_or_default()
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric())
                .count();
            let run_end = run_start + run_len;
            let bounded_left = run_start > 0 || left_closed;
            let bounded_right = run_end < lit.len() || right_closed;
            if bounded_left && bounded_right && run_len >= MIN_TOKEN_LEN {
                let candidate = lit.get(run_start..run_end).unwrap_or_default();
                if best.is_none_or(|b| candidate.len() > b.len()) {
                    best = Some(candidate);
                }
            }
            start = run_end;
        }
    }
    best.map(|b| b.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::url_filter::UrlTarget;
    use url::Url;

    fn parse(pattern: &str, case_sensitive: bool) -> Option<UrlFilter> {
        UrlFilter::parse(pattern, case_sensitive, 2048).ok()
    }

    fn token(pattern: &str) -> Option<String> {
        select_token(&parse(pattern, false).expect("valid"))
            .map(|t| String::from_utf8(t).expect("ascii"))
    }

    #[test]
    fn domain_anchored_filter_uses_the_longest_bounded_token() {
        assert_eq!(token("||example.com^"), Some("example".into()));
    }

    #[test]
    fn path_segment_between_wildcards_is_a_token() {
        assert_eq!(token("*/ads/*"), Some("ads".into()));
    }

    #[test]
    fn unanchored_plain_text_has_no_token() {
        assert_eq!(token("banner"), None);
        assert_eq!(token("/banner"), None);
        assert_eq!(token("banner/"), None);
    }

    #[test]
    fn right_anchor_closes_the_last_token() {
        assert_eq!(token("/banner|"), Some("banner".into()));
        assert_eq!(token("/banner^"), Some("banner".into()));
    }

    #[test]
    fn left_anchor_closes_the_first_token() {
        assert_eq!(token("|https://x"), Some("https".into()));
        assert_eq!(token("^banner/"), Some("banner".into()));
    }

    #[test]
    fn wildcard_neighbours_do_not_close_a_token() {
        assert_eq!(token("*ads*"), None);
        assert_eq!(token("a*banner/"), None);
        assert_eq!(token("*banner/x"), None);
    }

    #[test]
    fn short_tokens_are_not_indexed() {
        assert_eq!(token("||ab.cd^"), None);
        assert_eq!(token("/ab/"), None);
    }

    #[test]
    fn tokens_are_lowercased_even_for_case_sensitive_filters() {
        let f = parse("||EXAMPLE.com^", true).expect("valid");
        assert_eq!(select_token(&f), Some(b"example".to_vec()));
    }

    #[test]
    fn wildcard_alone_and_empty_patterns_have_no_token() {
        assert_eq!(token("*"), None);
        assert_eq!(token("|"), None);
    }

    #[test]
    fn candidates_include_fallback_and_token_hits_sorted_and_unique() {
        let a = UrlPattern::Filter(parse("||example.com^", false).expect("valid"));
        let b = UrlPattern::Filter(parse("banner", false).expect("valid"));
        let c = UrlPattern::Filter(parse("|zzz|", false).expect("valid"));
        let index = RuleIndex::build([Some(&a), Some(&b), None, Some(&c)]);
        assert_eq!(index.indexed_len(), 2);
        assert_eq!(index.fallback_len(), 2);
        assert_eq!(
            index.candidates("https://www.example.com/example/"),
            vec![0, 1, 2]
        );
        assert_eq!(index.candidates("https://x.test/zzz"), vec![1, 2, 3]);
        assert_eq!(index.candidates("https://other.test/"), vec![1, 2]);
    }

    #[test]
    fn index_is_case_blind_on_lookup() {
        let a = UrlPattern::Filter(parse("||example.com^", false).expect("valid"));
        let index = RuleIndex::build([Some(&a)]);
        assert_eq!(index.candidates("https://WWW.EXAMPLE.COM/"), vec![0]);
    }

    #[test]
    fn empty_index_returns_nothing() {
        let index = RuleIndex::build(std::iter::empty());
        assert!(index.candidates("https://a.test/").is_empty());
    }

    // --- differential test --------------------------------------------------------------------------

    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next() % u64::try_from(n).unwrap_or(1)).unwrap_or(0)
        }

        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[self.below(items.len())]
        }
    }

    const WORDS: &[&str] = &[
        "ads", "track", "example", "cdn", "img", "api", "v1", "x", "banner", "pixel", "metrics",
        "static", "sync", "login", "assets", "ab", "Ad", "TRACK",
    ];
    const SEPS: &[&str] = &["/", "?", "&", "=", ".", "-", "_", "%20", ":", "/"];
    const TLDS: &[&str] = &["com", "net", "org", "io", "test"];

    fn gen_url(rng: &mut XorShift) -> String {
        let scheme = rng.pick(&["http", "https", "wss"]);
        let mut host = String::new();
        for _ in 0..rng.below(3) {
            host.push_str(&rng.pick(WORDS).to_ascii_lowercase());
            host.push('.');
        }
        host.push_str(&rng.pick(WORDS).to_ascii_lowercase());
        host.push('.');
        host.push_str(rng.pick(TLDS));
        let mut url = format!("{scheme}://{host}");
        if rng.below(4) == 0 {
            url.push_str(":8080");
        }
        for _ in 0..rng.below(5) {
            url.push('/');
            url.push_str(rng.pick(WORDS));
            if rng.below(3) == 0 {
                url.push_str(rng.pick(&["", ".js", ".png", "1", "s"]));
            }
        }
        if rng.below(2) == 0 {
            url.push('?');
            for _ in 0..rng.below(3) {
                url.push_str(rng.pick(WORDS));
                url.push('=');
                url.push_str(rng.pick(WORDS));
                url.push('&');
            }
        }
        url
    }

    fn gen_filter(rng: &mut XorShift, corpus: &[Url]) -> Option<UrlFilter> {
        let mut pattern = String::new();
        if rng.below(2) == 0 {
            // derived from a corpus URL, so that matches exist
            let base = corpus[rng.below(corpus.len())].as_str().to_owned();
            let len = base.len();
            let a = rng.below(len);
            let b = a + 1 + rng.below(len - a);
            pattern = base.get(a..b)?.to_owned();
            if rng.below(2) == 0 {
                pattern = pattern
                    .chars()
                    .map(|c| match rng.below(8) {
                        0 => '*',
                        1 if !c.is_ascii_alphanumeric() => '^',
                        _ => c,
                    })
                    .collect();
            }
        } else {
            for _ in 0..1 + rng.below(4) {
                match rng.below(6) {
                    0 => pattern.push('*'),
                    1 => pattern.push('^'),
                    2 => pattern.push_str(rng.pick(SEPS)),
                    _ => pattern.push_str(rng.pick(WORDS)),
                }
            }
        }
        match rng.below(6) {
            0 => pattern.insert_str(0, "||"),
            1 => pattern.insert(0, '|'),
            _ => {}
        }
        if rng.below(5) == 0 {
            pattern.push('|');
        }
        let case_sensitive = rng.below(4) == 0;
        UrlFilter::parse(&pattern, case_sensitive, 2048).ok()
    }

    fn differential(seed: u64, filters: usize, urls: usize) -> (usize, usize, usize) {
        let mut rng = XorShift(seed);
        let corpus: Vec<Url> = (0..urls)
            .filter_map(|_| Url::parse(&gen_url(&mut rng)).ok())
            .collect();
        let mut compiled: Vec<UrlPattern> = Vec::new();
        while compiled.len() < filters {
            if let Some(f) = gen_filter(&mut rng, &corpus) {
                compiled.push(UrlPattern::Filter(f));
            }
        }
        let index = RuleIndex::build(compiled.iter().map(Some));
        let mut pairs = 0usize;
        let mut matched = 0usize;
        for url in &corpus {
            let target = UrlTarget::new(url);
            let linear: Vec<u32> = compiled
                .iter()
                .enumerate()
                .filter(|(_, p)| p.matches(&target))
                .map(|(i, _)| u32::try_from(i).expect("small"))
                .collect();
            let indexed: Vec<u32> = index
                .candidates(target.text())
                .into_iter()
                .filter(|i| compiled[usize::try_from(*i).expect("small")].matches(&target))
                .collect();
            assert_eq!(linear, indexed, "index lost or invented a match for {url}");
            pairs += compiled.len();
            matched += linear.len();
        }
        (pairs, matched, index.indexed_len())
    }

    #[test]
    fn differential_index_matches_linear_scan_over_a_generated_corpus() {
        let (pairs, matched, indexed) = differential(0x9E37_79B9_7F4A_7C15, 300, 200);
        assert!(pairs >= 5_000, "only {pairs} pairs");
        assert!(matched >= 200, "only {matched} matching pairs");
        assert!(indexed >= 30, "only {indexed} filters were indexed");
    }

    #[test]
    fn differential_holds_for_other_seeds() {
        for seed in [1u64, 2, 3, 0xDEAD_BEEF, 0x1234_5678_9ABC_DEF1] {
            let (pairs, _, _) =
                differential(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1, 150, 120);
            assert!(pairs >= 5_000);
        }
    }

    #[test]
    fn timing_report_shows_the_speed_up_without_asserting_on_it() {
        let mut rng = XorShift(0xA5A5_A5A5_1234_5678);
        let corpus: Vec<Url> = (0..300)
            .filter_map(|_| Url::parse(&gen_url(&mut rng)).ok())
            .collect();
        let mut compiled: Vec<UrlPattern> = Vec::new();
        while compiled.len() < 1500 {
            if let Some(f) = gen_filter(&mut rng, &corpus) {
                compiled.push(UrlPattern::Filter(f));
            }
        }
        let index = RuleIndex::build(compiled.iter().map(Some));
        let start = std::time::Instant::now();
        let mut linear_hits = 0usize;
        for url in &corpus {
            let target = UrlTarget::new(url);
            linear_hits += compiled.iter().filter(|p| p.matches(&target)).count();
        }
        let linear = start.elapsed();
        let start = std::time::Instant::now();
        let mut indexed_hits = 0usize;
        let mut candidates = 0usize;
        for url in &corpus {
            let target = UrlTarget::new(url);
            let c = index.candidates(target.text());
            candidates += c.len();
            indexed_hits += c
                .iter()
                .filter(|i| compiled[usize::try_from(**i).expect("small")].matches(&target))
                .count();
        }
        let indexed = start.elapsed();
        assert_eq!(linear_hits, indexed_hits);

        // a blocklist-shaped ruleset: 5,000 domain-anchored filters, every one indexable
        let blocklist: Vec<UrlPattern> = (0..5000)
            .filter_map(|i| parse(&format!("||tracker{i}.ads{}.example^", i % 97), false))
            .map(UrlPattern::Filter)
            .collect();
        let block_index = RuleIndex::build(blocklist.iter().map(Some));
        let mut urls = corpus.clone();
        urls.extend((0..30).filter_map(|n| {
            Url::parse(&format!(
                "https://x.tracker{}.ads{}.example/p",
                n * 7,
                (n * 7) % 97
            ))
            .ok()
        }));
        let start = std::time::Instant::now();
        let mut linear_blocked = 0usize;
        for url in &urls {
            let target = UrlTarget::new(url);
            linear_blocked += blocklist.iter().filter(|p| p.matches(&target)).count();
        }
        let linear_block_time = start.elapsed();
        let start = std::time::Instant::now();
        let mut indexed_blocked = 0usize;
        for url in &urls {
            let target = UrlTarget::new(url);
            indexed_blocked += block_index
                .candidates(target.text())
                .iter()
                .filter(|i| blocklist[usize::try_from(**i).expect("small")].matches(&target))
                .count();
        }
        let indexed_block_time = start.elapsed();
        assert_eq!(linear_blocked, indexed_blocked);
        assert!(linear_blocked >= 30);
        eprintln!(
            "index timing (blocklist): {} filters x {} urls: linear {linear_block_time:?}, \
             indexed {indexed_block_time:?} ({} indexed, {} fallback)",
            blocklist.len(),
            urls.len(),
            block_index.indexed_len(),
            block_index.fallback_len(),
        );
        eprintln!(
            "index timing: {} filters x {} urls: linear {linear:?}, indexed {indexed:?}, \
             {candidates} candidates instead of {} ({} indexed, {} fallback)",
            compiled.len(),
            corpus.len(),
            compiled.len() * corpus.len(),
            index.indexed_len(),
            index.fallback_len(),
        );
    }
}
