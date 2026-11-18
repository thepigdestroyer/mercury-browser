// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

/// Whether the engine's site `site` (a registrable domain such as `example.com`, or an address)
/// holds the data of `host`: the same name, or a subdomain of it.
pub fn site_covers_host(site: &str, host: &str) -> bool {
    let site = site.trim_end_matches('.').to_ascii_lowercase();
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if site.is_empty() {
        return false;
    }
    host == site
        || host
            .strip_suffix(&site)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use super::site_covers_host;

    #[test]
    fn a_site_covers_its_own_name_and_its_subdomains_only() {
        assert!(site_covers_host("example.com", "example.com"));
        assert!(site_covers_host("example.com", "www.Example.com"));
        assert!(site_covers_host("example.com.", "a.b.example.com"));
        assert!(site_covers_host("127.0.0.1", "127.0.0.1"));
        assert!(!site_covers_host("example.com", "badexample.com"));
        assert!(!site_covers_host("example.com", "example.org"));
        assert!(!site_covers_host("www.example.com", "example.com"));
        assert!(!site_covers_host("", "example.com"));
        assert!(!site_covers_host("1.0.0.1", "127.0.0.1"));
    }
}
