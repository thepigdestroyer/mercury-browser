// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The built-in sign-in exception pairs (NET-2 N2-D4).

/// One sign-in exception: a request to `embedded` made while the top-level site is `top_level`
/// keeps full cookie access, although `embedded` is a third party there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SigninPair {
    /// The embedded site, as a lower-case registrable domain.
    pub embedded: &'static str,
    /// The top-level site the embed sits under, as a lower-case registrable domain.
    pub top_level: &'static str,
}

/// The seed list. Each company's own sites are paired with each other in both directions; there
/// are no social network pairs. It is data: a new pair is one line plus a test.
const SIGNIN_EXCEPTIONS: &[SigninPair] = &[
    SigninPair {
        embedded: "google.com",
        top_level: "youtube.com",
    },
    SigninPair {
        embedded: "youtube.com",
        top_level: "google.com",
    },
    SigninPair {
        embedded: "apple.com",
        top_level: "icloud.com",
    },
    SigninPair {
        embedded: "icloud.com",
        top_level: "apple.com",
    },
    SigninPair {
        embedded: "microsoft.com",
        top_level: "live.com",
    },
    SigninPair {
        embedded: "microsoft.com",
        top_level: "office.com",
    },
    SigninPair {
        embedded: "microsoft.com",
        top_level: "microsoftonline.com",
    },
    SigninPair {
        embedded: "microsoft.com",
        top_level: "microsoft365.com",
    },
    SigninPair {
        embedded: "microsoft.com",
        top_level: "msn.com",
    },
    SigninPair {
        embedded: "live.com",
        top_level: "microsoft.com",
    },
    SigninPair {
        embedded: "live.com",
        top_level: "office.com",
    },
    SigninPair {
        embedded: "live.com",
        top_level: "microsoftonline.com",
    },
    SigninPair {
        embedded: "live.com",
        top_level: "microsoft365.com",
    },
    SigninPair {
        embedded: "live.com",
        top_level: "msn.com",
    },
    SigninPair {
        embedded: "office.com",
        top_level: "microsoft.com",
    },
    SigninPair {
        embedded: "office.com",
        top_level: "live.com",
    },
    SigninPair {
        embedded: "office.com",
        top_level: "microsoftonline.com",
    },
    SigninPair {
        embedded: "office.com",
        top_level: "microsoft365.com",
    },
    SigninPair {
        embedded: "office.com",
        top_level: "msn.com",
    },
    SigninPair {
        embedded: "microsoftonline.com",
        top_level: "microsoft.com",
    },
    SigninPair {
        embedded: "microsoftonline.com",
        top_level: "live.com",
    },
    SigninPair {
        embedded: "microsoftonline.com",
        top_level: "office.com",
    },
    SigninPair {
        embedded: "microsoftonline.com",
        top_level: "microsoft365.com",
    },
    SigninPair {
        embedded: "microsoftonline.com",
        top_level: "msn.com",
    },
    SigninPair {
        embedded: "microsoft365.com",
        top_level: "microsoft.com",
    },
    SigninPair {
        embedded: "microsoft365.com",
        top_level: "live.com",
    },
    SigninPair {
        embedded: "microsoft365.com",
        top_level: "office.com",
    },
    SigninPair {
        embedded: "microsoft365.com",
        top_level: "microsoftonline.com",
    },
    SigninPair {
        embedded: "microsoft365.com",
        top_level: "msn.com",
    },
    SigninPair {
        embedded: "msn.com",
        top_level: "microsoft.com",
    },
    SigninPair {
        embedded: "msn.com",
        top_level: "live.com",
    },
    SigninPair {
        embedded: "msn.com",
        top_level: "office.com",
    },
    SigninPair {
        embedded: "msn.com",
        top_level: "microsoftonline.com",
    },
    SigninPair {
        embedded: "msn.com",
        top_level: "microsoft365.com",
    },
];

/// The built-in sign-in exception pairs.
pub fn signin_exceptions() -> &'static [SigninPair] {
    SIGNIN_EXCEPTIONS
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUFFIX_LIKE: &[&str] = &["com", "co.uk", "github.io"];

    fn has(embedded: &str, top_level: &str) -> bool {
        signin_exceptions()
            .iter()
            .any(|pair| pair.embedded == embedded && pair.top_level == top_level)
    }

    fn is_clean_site(site: &str) -> bool {
        !site.is_empty()
            && site == site.to_ascii_lowercase()
            && !site.contains("://")
            && !site.contains('/')
            && site.contains('.')
            && !site.starts_with('.')
            && !site.ends_with('.')
    }

    #[test]
    fn the_seed_list_has_no_duplicates_and_every_entry_is_lower_case_without_scheme_or_dot_edges() {
        let list = signin_exceptions();
        assert!(!list.is_empty());
        for (index, pair) in list.iter().enumerate() {
            assert!(
                is_clean_site(pair.embedded),
                "bad embedded site in {pair:?}"
            );
            assert!(
                is_clean_site(pair.top_level),
                "bad top-level site in {pair:?}"
            );
            assert!(!list[..index].contains(pair), "duplicate pair {pair:?}");
        }
    }

    #[test]
    fn google_and_youtube_are_paired_in_both_directions() {
        assert!(has("google.com", "youtube.com"));
        assert!(has("youtube.com", "google.com"));
    }

    #[test]
    fn no_pair_has_the_same_site_on_both_sides() {
        for pair in signin_exceptions() {
            assert_ne!(pair.embedded, pair.top_level, "{pair:?}");
        }
    }

    #[test]
    fn an_embedded_site_is_never_a_public_suffix_like_com() {
        for pair in signin_exceptions() {
            assert!(!SUFFIX_LIKE.contains(&pair.embedded), "{pair:?}");
            assert!(!SUFFIX_LIKE.contains(&pair.top_level), "{pair:?}");
        }
    }
}
