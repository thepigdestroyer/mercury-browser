// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The page: structure, escaping of hostile input, the MPL source table, determinism.
mod common;

use common::{APACHE, Fake, MIT};
use mercury_credits::page::{self, escape, is_web_url};
use sha2::{Digest, Sha256};

fn page_of(fake: &Fake) -> String {
    let outcome = fake.outcome();
    assert!(outcome.ok(), "problems: {:#?}", outcome.report.problems);
    outcome.page.expect("a page")
}

/// A fake repository with a few ordinary crates.
fn ordinary() -> Fake {
    let mut fake = Fake::new();
    fake.add("alpha", "1.0.0", "MIT OR Apache-2.0");
    fake.add("beta", "0.3.0+spec-1.1.0", "MIT");
    fake.crate_file("alpha", "LICENSE-MIT", MIT);
    fake.crate_file("alpha", "LICENSE-APACHE", APACHE);
    fake.crate_file("beta", "LICENSE", MIT);
    fake.package("alpha").repository = Some("https://github.com/example/alpha".into());
    fake
}

#[test]
fn the_page_has_the_fixed_structure() {
    let page = page_of(&ordinary());
    assert!(page.starts_with("<!doctype html>\n<html lang=\"en\">"));
    assert!(page.contains("<meta charset=\"utf-8\">"));
    assert!(page.contains(&format!(
        "<meta name=\"generator\" content=\"mercury-credits {}\">",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(
        page.contains("<meta name=\"credits-crates\" content=\"3\">"),
        "alpha, beta and the app"
    );
    assert!(page.contains("<title>Mercury Browser: third-party notices</title>"));
    let position = |needle: &str| {
        page.find(needle)
            .unwrap_or_else(|| panic!("{needle} missing"))
    };
    let notice = position("<section id=\"notice\">");
    let source = position("<section id=\"source\">");
    let bundled = position("<section id=\"bundled\">");
    let crates = position("<section id=\"crates\">");
    assert!(notice < source && source < bundled && bundled < crates);
    assert!(
        page.contains("Mercury Browser notice\nsecond line"),
        "NOTICE is shown as it is"
    );
    assert!(
        page.contains("# Third-party notices"),
        "THIRD-PARTY-NOTICES.md is shown as it is"
    );
    assert!(page.contains("<h3>Index</h3>"));
    assert!(page.contains("<article id=\"t-1\">"));
    assert!(page.ends_with("</body>\n</html>\n"));
}

#[test]
fn the_page_shows_the_version_the_target_and_the_lock_digest_but_no_date() {
    let fake = ordinary();
    let page = page_of(&fake);
    let digest = Sha256::digest(b"# a lock file\n");
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    assert!(
        page.contains(&format!("Cargo.lock sha256 {}", &hex[..12])),
        "{}",
        &hex[..12]
    );
    assert!(page.contains("Version 9.9.9"));
    assert!(page.contains("target aarch64-apple-darwin"));
    assert!(!page.contains("runtime crates only"));
    assert!(
        !page.to_lowercase().contains("generated on"),
        "no timestamp"
    );
}

#[test]
fn the_page_has_no_script_and_no_external_resource() {
    let page = page_of(&ordinary()).to_lowercase();
    for forbidden in [
        "<script",
        "<link",
        "<iframe",
        "<img",
        "<object",
        "<embed",
        " src=",
        "@import",
        "url(",
        "javascript:",
        "onerror=",
        "onclick=",
    ] {
        assert!(!page.contains(forbidden), "{forbidden}");
    }
    assert!(page.contains("prefers-color-scheme:dark"), "dark colours");
    assert!(page.contains("color-scheme:light dark"));
}

#[test]
fn every_text_is_escaped_including_a_licence_that_tries_to_close_the_pre() {
    let mut fake = Fake::new();
    fake.add("evil", "1.0.0", "MIT");
    fake.crate_file(
        "evil",
        "LICENSE",
        &format!(
            "{MIT}\n</pre><script>alert(1)</script><img src=x onerror=alert(2)> & \"q\" 'a'\n"
        ),
    );
    let page = page_of(&fake);
    assert!(!page.contains("<script"), "no script tag survives");
    assert!(!page.contains("</pre><script>"));
    assert!(page.contains("&lt;/pre&gt;&lt;script&gt;alert(1)&lt;/script&gt;&lt;img src=x onerror=alert(2)&gt; &amp; &quot;q&quot; &#39;a&#39;"));
}

#[test]
fn crate_names_authors_repositories_and_notes_are_escaped_and_javascript_urls_are_not_links() {
    let mut fake = Fake::new();
    fake.add("<b>name</b>", "1.0.0-<i>", "MIT");
    fake.add("js-link", "1.0.0", "MIT");
    fake.add("quote-link", "1.0.0", "MIT");
    fake.add("bare", "1.0.0", "MIT");
    fake.crate_file(
        "<b>name</b>",
        "LICENSE",
        &MIT.replace("Jane Roe", "<u>Jane</u>"),
    );
    fake.crate_file("js-link", "LICENSE", MIT);
    fake.crate_file("quote-link", "LICENSE", &MIT.replace("2019", "2018"));
    fake.package("<b>name</b>").repository =
        Some("https://example.org/\"><script>x</script>".into());
    fake.package("js-link").repository = Some("javascript:alert(1)".into());
    fake.package("quote-link").repository =
        Some("https://example.org/a\"onmouseover=\"alert(1)".into());
    // The fallback copyright line is built from authors and the repository: both untrusted.
    fake.package("bare").authors = vec!["Eve & Co \"quoted\" <eve@example.org>".into()];
    fake.package("bare").repository = Some("data:text/html,<script>alert(1)</script>".into());
    fake.write(
        "crates/mercury-credits/credits.toml",
        "[[extra]]\ncrate = \"js-link\"\nfiles = []\nnote = \"<script>note()</script>\"\n[[ignore]]\ncrate = \"ghost\"\nreason = \"<b>x</b>\"\n",
    );
    fake.crate_file("js-link", "deps/x/LICENSE", MIT);
    let page = page_of(&fake);
    assert!(!page.contains("<script"), "no script element");
    assert!(!page.contains("<b>name"), "the crate name is escaped");
    assert!(page.contains("&lt;b&gt;name&lt;/b&gt; 1.0.0-&lt;i&gt;"));
    assert!(!page.contains("href=\"javascript:"));
    assert!(!page.contains("href=\"data:"));
    assert!(
        !page.contains("onmouseover=\"alert"),
        "a quote in a URL cannot open an attribute"
    );
    assert!(
        page.contains("href=\"https://example.org/a&quot;onmouseover=&quot;alert(1)\""),
        "{}",
        page.lines()
            .filter(|line| line.contains("quote-link"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        page.contains("js-link 1.0.0"),
        "the crate is still named, only without a link"
    );
    assert!(
        page.contains("Copyright (c) Eve &amp; Co &quot;quoted&quot;\n"),
        "an email address is dropped from the authors, the rest escaped"
    );
    assert!(!page.contains("eve@example.org"));
    assert!(page.contains("&lt;u&gt;Jane&lt;/u&gt;"));
}

#[test]
fn only_http_and_https_urls_are_web_urls() {
    for yes in [
        "https://example.org/x",
        "http://example.org",
        "HTTPS://EXAMPLE.ORG",
    ] {
        assert!(is_web_url(yes), "{yes}");
    }
    for no in [
        "javascript:alert(1)",
        "data:text/html,x",
        "file:///etc/passwd",
        "ftp://example.org",
        "//example.org",
        "https://exa mple.org",
        "https://example.org/\nx",
        " https://example.org",
        "",
        "mailto:a@b",
    ] {
        assert!(!is_web_url(no), "{no:?}");
    }
}

#[test]
fn escape_covers_the_five_characters_and_nul() {
    assert_eq!(
        escape("a & b < c > d \" e ' f"),
        "a &amp; b &lt; c &gt; d &quot; e &#39; f"
    );
    assert_eq!(escape("plain text é ©"), "plain text é ©");
    assert_eq!(escape("a\0b"), "a\u{fffd}b");
    assert_eq!(
        escape("&amp;"),
        "&amp;amp;",
        "an entity in the input is shown as typed"
    );
}

#[test]
fn the_mpl_source_table_lists_crates_that_need_mpl_with_their_source_urls() {
    let mut fake = Fake::new();
    fake.add("stylo", "0.17.0", "MPL-2.0");
    fake.add("plus", "0.9.12+spec-1.1.0", "MPL-2.0 AND MIT");
    fake.add("optional", "1.0.0", "MIT OR Apache-2.0 OR MPL-2.0");
    fake.add("plain", "1.0.0", "MIT");
    fake.add_vendored("servo-net", "0.8.0", "MPL-2.0");
    fake.crate_file("plain", "LICENSE", MIT);
    fake.crate_file("optional", "LICENSE", MIT);
    fake.write(
        "third_party/vendor.toml",
        "[[crate]]\nname = \"servo-net\"\nversion = \"0.8.0\"\nrepository = \"https://github.com/servo/servo\"\ncommit = \"4e136d8ef31b7c4748fbfac606ca463f3efd7537\"\npath = \"components/net\"\nlicense = \"MPL-2.0\"\n",
    );
    let page = page_of(&fake);
    let start = page.find("<section id=\"source\">").unwrap();
    let end = page.find("<section id=\"bundled\">").unwrap();
    let source = &page[start..end];
    assert!(source.contains("The 3 crates below"), "{source}");
    assert!(source.contains("https://crates.io/crates/stylo/0.17.0"));
    assert!(source.contains("https://static.crates.io/crates/stylo/stylo-0.17.0.crate"));
    assert!(source.contains("https://crates.io/crates/plus/0.9.12+spec-1.1.0"));
    assert!(source.contains("https://static.crates.io/crates/plus/plus-0.9.12+spec-1.1.0.crate"));
    assert!(
        !source.contains("crates/optional/"),
        "MIT is an alternative to MPL there"
    );
    assert!(!source.contains("crates/plain/"));
    // The vendored crate: the Mercury tag, the upstream repository and the commit.
    assert!(source.contains(
        "https://github.com/thepigdestroyer/mercury-browser/tree/v9.9.9/third_party/servo-net"
    ));
    assert!(
        source.contains(
            "https://github.com/servo/servo/tree/4e136d8ef31b7c4748fbfac606ca463f3efd7537"
        )
    );
    assert!(source.contains("<code>4e136d8ef31b7c4748fbfac606ca463f3efd7537</code>"));
    // The sentence about Mercury's own MPL files and the LGPL sources.
    assert!(source.contains("The files marked MPL-2.0 in the Mercury repository"));
    assert!(source.contains("https://github.com/thepigdestroyer/mercury-browser/tree/v9.9.9"));
    assert!(source.contains("Mercury-Browser-9.9.9-lgpl-sources.tar"));
}

#[test]
fn without_mpl_crates_the_source_section_has_no_table() {
    let page = page_of(&ordinary());
    let start = page.find("<section id=\"source\">").unwrap();
    let end = page.find("<section id=\"bundled\">").unwrap();
    assert!(!page[start..end].contains("<table>"));
    assert!(page[start..end].contains("The files marked MPL-2.0"));
}

#[test]
fn a_vendored_crates_licence_note_is_shown_under_its_text() {
    let mut fake = Fake::new();
    fake.add_vendored("i-slint-backend-winit", "1.17.1", "MIT");
    fake.crate_file("i-slint-backend-winit", "LICENSE", MIT);
    fake.write(
        "third_party/vendor.toml",
        "[[crate]]\nname = \"i-slint-backend-winit\"\nversion = \"1.17.1\"\nrepository = \"https://github.com/slint-ui/slint\"\ncommit = \"cf62c9\"\nlicense_note = \"Used under the Royalty-free terms & <conditions>.\"\n",
    );
    let page = page_of(&fake);
    assert!(page.contains("<strong>i-slint-backend-winit 1.17.1:</strong> Used under the Royalty-free terms &amp; &lt;conditions&gt;."));
}

#[test]
fn the_page_is_the_same_bytes_every_time_and_whatever_the_order_of_the_packages() {
    let fake = ordinary();
    let first = page_of(&fake);
    let second = page_of(&fake);
    assert_eq!(first, second);

    let mut shuffled = ordinary();
    shuffled.packages.reverse();
    let metadata = shuffled.metadata();
    let tree = {
        let text = shuffled.tree_text();
        let mut lines: Vec<&str> = text.lines().collect();
        lines.reverse();
        lines.join("\n") + "\n"
    };
    let outcome = mercury_credits::app::analyse_inputs(&mercury_credits::collect::Inputs {
        root: shuffled.root(),
        tree: &tree,
        metadata: &metadata,
        target: "aarch64-apple-darwin",
        runtime_only: false,
    });
    assert_eq!(
        outcome.page.unwrap(),
        first,
        "the output does not depend on input order"
    );
}

#[test]
fn runtime_only_is_said_on_the_page() {
    let fake = ordinary();
    let metadata = fake.metadata();
    let tree = fake.tree_text();
    let outcome = mercury_credits::app::analyse_inputs(&mercury_credits::collect::Inputs {
        root: fake.root(),
        tree: &tree,
        metadata: &metadata,
        target: "x86_64-unknown-linux-gnu",
        runtime_only: true,
    });
    let page = outcome.page.unwrap();
    assert!(page.contains("target x86_64-unknown-linux-gnu (runtime crates only)"));
}

#[test]
fn standard_text_crates_are_flagged_on_the_page_and_in_the_index() {
    let mut fake = Fake::new();
    fake.add("bare", "1.0.0", "MIT");
    let page = page_of(&fake);
    assert!(page.contains("MIT (standard text)"));
    assert!(page.contains("<span class=\"flag\">(standard text)</span>"));
    assert!(page.contains("the package ships no text for this licence"));
}

#[test]
fn unclassified_and_statement_texts_have_their_own_headings() {
    let mut fake = Fake::new();
    fake.add("weird", "1.0.0", "MIT");
    fake.add("pointer", "1.0.0", "MIT");
    fake.crate_file(
        "weird",
        "LICENSE",
        "Weird licence\n\nYou may use this software provided you keep this line.\n",
    );
    fake.crate_file("pointer", "LICENSE", "MIT OR Apache-2.0\n");
    fake.crate_file("pointer", "NOTICE", "A notice\n");
    let page = page_of(&fake);
    assert!(page.contains("<h3>Licence text of unknown wording: weird 1.0.0</h3>"));
    assert!(page.contains("<h3>Licence statement: pointer 1.0.0</h3>"));
    assert!(page.contains("<h3>NOTICE file: pointer 1.0.0</h3>"));
    assert!(page.contains("Declared licences: MIT."));
}

#[test]
fn the_tool_constant_names_the_package_version() {
    assert_eq!(
        page::TOOL,
        format!("mercury-credits {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        page::mercury_tree_url("1.2.3+x", "third_party/servo"),
        "https://github.com/thepigdestroyer/mercury-browser/tree/v1.2.3+x/third_party/servo"
    );
}
