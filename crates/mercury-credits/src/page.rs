// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `credits.html`: one self-contained page with no script and no external resource.
//!
//! Every string that comes from outside the tool (licence texts, crate names, authors,
//! repository URLs) is untrusted and goes through [`escape`]; only `http://` and `https://`
//! URLs become links. The page is a pure function of its inputs: sorted, no timestamps.
use crate::collect::{Block, BlockKind, Collected, CrateRef, CreditedCrate, Kind, Member};
use std::fmt::Write as _;

/// The repository the source pointers use.
pub const REPOSITORY: &str = "https://github.com/thepigdestroyer/mercury-browser";

/// The tool's own version (shown in the page).
pub const TOOL: &str = concat!("mercury-credits ", env!("CARGO_PKG_VERSION"));

const STYLE: &str = "\
:root{color-scheme:light dark;--bg:#ffffff;--fg:#1b1b1f;--muted:#5b5b66;--line:#d5d5dd;--code:#f4f4f8;--link:#0b57d0;--flag:#8a4b00}\
@media (prefers-color-scheme:dark){:root{--bg:#16161a;--fg:#e6e6ea;--muted:#a0a0ac;--line:#33333b;--code:#1f1f25;--link:#8ab4f8;--flag:#f0b46a}}\
body{margin:0 auto;max-width:62rem;padding:1.5rem 1rem 4rem;background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,-apple-system,\"Segoe UI\",sans-serif}\
h1{font-size:1.6rem;margin:0 0 .25rem}h2{font-size:1.25rem;margin:2.5rem 0 .75rem;border-bottom:1px solid var(--line);padding-bottom:.25rem}h3{font-size:1.05rem;margin:0 0 .5rem}\
a{color:var(--link)}p{margin:.5rem 0}.muted{color:var(--muted)}.flag{color:var(--flag)}\
pre{background:var(--code);border:1px solid var(--line);border-radius:6px;padding:.75rem;overflow-x:auto;white-space:pre-wrap;overflow-wrap:anywhere;font:12.5px/1.45 ui-monospace,Menlo,Consolas,monospace}\
code{font:13px ui-monospace,Menlo,Consolas,monospace}nav a{margin-right:1rem}\
table{border-collapse:collapse;width:100%;font-size:.9rem}th,td{border-bottom:1px solid var(--line);padding:.3rem .5rem;text-align:left;vertical-align:top}\
article{margin:1.5rem 0}ul.index{columns:2;column-gap:2rem;padding-left:1.2rem}.crates{overflow-wrap:anywhere}";

/// `& < > " '` as entities: safe in text and in quoted attributes.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 16);
    escape_into(&mut out, text);
    out
}

pub fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\0' => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
}

/// Whether `url` may become a link: it starts with `https://` or `http://`.
pub fn is_web_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// `<a href="url">text</a>`, or the escaped text when `url` is not a web URL.
fn link(out: &mut String, url: Option<&str>, text: &str) {
    match url.filter(|url| is_web_url(url)) {
        Some(url) => {
            out.push_str("<a href=\"");
            escape_into(out, url);
            out.push_str("\" rel=\"noopener noreferrer\">");
            escape_into(out, text);
            out.push_str("</a>");
        }
        None => escape_into(out, text),
    }
}

/// A path segment of a URL: everything but letters, digits and `-_.~+` is percent-encoded.
fn url_segment(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'+') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// The tag of this version in the Mercury repository.
pub fn mercury_tree_url(version: &str, path: &str) -> String {
    format!("{REPOSITORY}/tree/v{}/{path}", url_segment(version))
}

/// What the texts of a group have in common: the licences, or the kind of file.
fn group_title(block: &Block) -> String {
    match block.kind {
        BlockKind::License => {
            let ids = block.ids.join(", ");
            if block.standard_only() {
                format!("{ids} (standard text)")
            } else {
                ids
            }
        }
        BlockKind::Statement => "Licence statements".to_owned(),
        BlockKind::Notice => "NOTICE files".to_owned(),
        BlockKind::Patents => "PATENTS files".to_owned(),
        BlockKind::Unclassified => "Licence texts of unknown wording".to_owned(),
    }
}

/// The heading of one text.
fn block_title(block: &Block) -> String {
    match block.kind {
        BlockKind::License => group_title(block),
        BlockKind::Statement => with_first_member("Licence statement", block),
        BlockKind::Notice => with_first_member("NOTICE file", block),
        BlockKind::Patents => with_first_member("PATENTS file", block),
        BlockKind::Unclassified => with_first_member("Licence text of unknown wording", block),
    }
}

/// `Title: crate version[ and N more]`, so that the many small texts of one kind can be told
/// apart in the index.
fn with_first_member(title: &str, block: &Block) -> String {
    let Some(first) = block.members.first() else {
        return title.to_owned();
    };
    match block.members.len() - 1 {
        0 => format!("{title}: {}", first.krate.label()),
        more => format!("{title}: {} and {more} more", first.krate.label()),
    }
}

fn member_line(out: &mut String, member: &Member, crates: &[CreditedCrate]) {
    let repository = crates
        .binary_search_by(|krate| krate.krate.cmp(&member.krate))
        .ok()
        .and_then(|index| crates[index].repository.as_deref());
    link(out, repository, &member.krate.label());
    if member.standard {
        out.push_str(" <span class=\"flag\">(standard text)</span>");
    }
    for via in &member.via {
        out.push_str(" <code>");
        escape_into(out, via);
        out.push_str("</code>");
    }
}

fn write_head(out: &mut String, collected: &Collected) {
    let _ = write!(
        out,
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<meta name=\"generator\" content=\"{}\">\n\
<meta name=\"credits-crates\" content=\"{}\">\n\
<title>Mercury Browser: third-party notices</title>\n<style>{STYLE}</style>\n</head>\n<body>\n",
        escape(TOOL),
        collected.credited_count()
    );
}

fn write_intro(out: &mut String, collected: &Collected) {
    out.push_str("<header>\n<h1>Mercury Browser: third-party notices</h1>\n<p class=\"muted\">");
    let _ = write!(
        out,
        "Version {} &middot; target {}{} &middot; {} crates &middot; Cargo.lock sha256 {} &middot; {}",
        escape(&collected.workspace_version),
        escape(&collected.target),
        if collected.runtime_only {
            " (runtime crates only)"
        } else {
            ""
        },
        collected.credited_count(),
        escape(&collected.lock_digest),
        escape(TOOL),
    );
    out.push_str("</p>\n<p>Mercury Browser contains the software listed here. Each part keeps its own licence and notices; they are reproduced below as the packages ship them. Where a package ships no text for a licence that Mercury uses it under, the standard text of that licence is shown with a copyright line taken from the package's Cargo.toml, and the entry says so.</p>\n");
    out.push_str("<nav><a href=\"#notice\">Notice</a><a href=\"#source\">Source code</a><a href=\"#bundled\">Bundled libraries and data</a><a href=\"#crates\">Rust crates</a></nav>\n</header>\n");
}

fn write_source(out: &mut String, collected: &Collected) {
    let version = &collected.workspace_version;
    out.push_str("<section id=\"source\">\n<h2>Source code</h2>\n<p>The files marked MPL-2.0 in the Mercury repository at the tag of this version are the source code of Mercury's Mozilla Public License parts: ");
    link(
        out,
        Some(&format!("{REPOSITORY}/tree/v{}", url_segment(version))),
        &format!("{REPOSITORY}/tree/v{version}"),
    );
    out.push_str(".</p>\n<p>Libraries under the GNU Lesser General Public License that a release bundles (the macOS app bundles GStreamer, FFmpeg, GLib, proxy-libintl and libnice) are separate dynamic libraries that can be replaced. Their corresponding source is published with every release as <code>Mercury-Browser-");
    escape_into(out, version);
    out.push_str("-lgpl-sources.tar</code> on the ");
    link(
        out,
        Some(&format!(
            "{REPOSITORY}/releases/tag/v{}",
            url_segment(version)
        )),
        "release page",
    );
    out.push_str(".</p>\n");
    let mpl: Vec<&CreditedCrate> = collected
        .crates
        .iter()
        .filter(|krate| krate.requires_mpl)
        .collect();
    if mpl.is_empty() {
        out.push_str("</section>\n");
        return;
    }
    let _ = writeln!(
        out,
        "<p>The {} crates below are used under the Mozilla Public License 2.0. Their source is available here.</p>",
        mpl.len()
    );
    out.push_str(
        "<table>\n<thead><tr><th>Crate</th><th>Version</th><th>Source</th></tr></thead>\n<tbody>\n",
    );
    for krate in mpl {
        out.push_str("<tr><td>");
        escape_into(out, &krate.krate.name);
        out.push_str("</td><td>");
        escape_into(out, &krate.krate.version);
        out.push_str("</td><td>");
        match (krate.kind, &krate.vendor) {
            (Kind::Vendored, Some(vendor)) => {
                let path = format!("third_party/{}", krate.krate.name);
                link(
                    out,
                    Some(&mercury_tree_url(version, &path)),
                    &format!("Mercury {version}: {path}"),
                );
                if let Some(repository) = &vendor.repository {
                    out.push_str("; upstream ");
                    let url = vendor.commit.as_deref().map_or_else(
                        || repository.clone(),
                        |commit| {
                            format!(
                                "{}/tree/{}",
                                repository.trim_end_matches('/'),
                                url_segment(commit)
                            )
                        },
                    );
                    link(out, Some(&url), repository);
                    if let Some(commit) = &vendor.commit {
                        out.push_str(" at <code>");
                        escape_into(out, commit);
                        out.push_str("</code>");
                    }
                }
            }
            _ => {
                let name = url_segment(&krate.krate.name);
                let crate_version = url_segment(&krate.krate.version);
                link(
                    out,
                    Some(&format!("https://crates.io/crates/{name}/{crate_version}")),
                    "crates.io",
                );
                out.push_str(", ");
                link(
                    out,
                    Some(&format!(
                        "https://static.crates.io/crates/{name}/{name}-{crate_version}.crate"
                    )),
                    ".crate",
                );
            }
        }
        out.push_str("</td></tr>\n");
    }
    out.push_str("</tbody>\n</table>\n</section>\n");
}

fn write_block(out: &mut String, index: usize, block: &Block, crates: &[CreditedCrate]) {
    let _ = write!(out, "<article id=\"t-{index}\">\n<h3>");
    escape_into(out, &block_title(block));
    out.push_str("</h3>\n");
    let _ = write!(
        out,
        "<p class=\"crates\">{} {}: ",
        block.members.len(),
        if block.members.len() == 1 {
            "crate"
        } else {
            "crates"
        }
    );
    for (position, member) in block.members.iter().enumerate() {
        if position > 0 {
            out.push_str(", ");
        }
        member_line(out, member, crates);
    }
    out.push_str("</p>\n");
    if block.kind == BlockKind::Unclassified {
        out.push_str("<p class=\"flag\">The wording of this text is not one the tool knows; it is reproduced as shipped. Declared licences: ");
        let mut declared: Vec<&str> = block
            .members
            .iter()
            .map(|member| member.declared.as_str())
            .collect();
        declared.sort_unstable();
        declared.dedup();
        escape_into(out, &declared.join("; "));
        out.push_str(".</p>\n");
    }
    if block.members.iter().any(|member| member.standard) {
        out.push_str("<p class=\"flag\">Standard text: the package ships no text for this licence, so the standard text is shown with a copyright line built from the package's metadata.</p>\n");
    }
    let mut notes: Vec<(String, &str)> = Vec::new();
    for member in &block.members {
        for note in &member.notes {
            notes.push((member.krate.label(), note));
        }
    }
    for (label, note) in notes {
        out.push_str("<p class=\"muted\"><strong>");
        escape_into(out, &label);
        out.push_str(":</strong> ");
        escape_into(out, note);
        out.push_str("</p>\n");
    }
    out.push_str("<pre>");
    escape_into(out, &block.body);
    out.push_str("</pre>\n</article>\n");
}

fn write_crates(out: &mut String, collected: &Collected) {
    out.push_str("<section id=\"crates\">\n<h2>Rust crates</h2>\n");
    let _ = writeln!(
        out,
        "<p>{} Rust crates are built into this version: {} from crates.io, {} vendored in the Mercury repository under third_party/, and {} of Mercury's own.</p>",
        collected.credited_count(),
        collected.registry_count(),
        collected.vendored_count(),
        collected.own.len()
    );
    out.push_str("<p>Mercury's own crates are covered by LICENSE and the notice above: ");
    let own: Vec<String> = collected
        .own
        .iter()
        .map(|krate| escape(&krate.name))
        .collect();
    out.push_str(&own.join(", "));
    out.push_str(".</p>\n");
    if !collected.ignored.is_empty() {
        out.push_str("<p class=\"muted\">Not credited (their output holds none of their code): ");
        let ignored: Vec<String> = collected
            .ignored
            .iter()
            .map(|(krate, reason)| format!("{} ({})", escape(&krate.label()), escape(reason)))
            .collect();
        out.push_str(&ignored.join("; "));
        out.push_str(".</p>\n");
    }
    // The index: one line for each run of texts with the same licences (the blocks are sorted,
    // so a run is contiguous), linking to its first text.
    out.push_str("<h3>Index</h3>\n<ul class=\"index\">\n");
    let mut position = 0;
    while position < collected.blocks.len() {
        let title = group_title(&collected.blocks[position]);
        let end = collected.blocks[position..]
            .iter()
            .position(|block| group_title(block) != title)
            .map_or(collected.blocks.len(), |offset| position + offset);
        let group = &collected.blocks[position..end];
        let crates: std::collections::BTreeSet<&CrateRef> = group
            .iter()
            .flat_map(|block| block.members.iter().map(|member| &member.krate))
            .collect();
        let _ = write!(out, "<li><a href=\"#t-{}\">", position + 1);
        escape_into(out, &title);
        let _ = writeln!(
            out,
            "</a> <span class=\"muted\">({} {}, {} {})</span></li>",
            group.len(),
            if group.len() == 1 { "text" } else { "texts" },
            crates.len(),
            if crates.len() == 1 { "crate" } else { "crates" },
        );
        position = end;
    }
    out.push_str("</ul>\n");
    for (position, block) in collected.blocks.iter().enumerate() {
        write_block(out, position + 1, block, &collected.crates);
    }
    out.push_str("</section>\n");
}

/// The whole page.
pub fn render(collected: &Collected) -> String {
    let mut out = String::with_capacity(
        collected
            .blocks
            .iter()
            .map(|block| block.body.len() + 600)
            .sum::<usize>()
            + 64 * 1024,
    );
    write_head(&mut out, collected);
    write_intro(&mut out, collected);
    out.push_str("<section id=\"notice\">\n<h2>Notice</h2>\n<pre>");
    escape_into(&mut out, &collected.notice);
    out.push_str("</pre>\n</section>\n");
    write_source(&mut out, collected);
    out.push_str("<section id=\"bundled\">\n<h2>Bundled libraries and data</h2>\n<pre>");
    escape_into(&mut out, &collected.bundled);
    out.push_str("</pre>\n</section>\n");
    write_crates(&mut out, collected);
    out.push_str("</body>\n</html>\n");
    out
}
