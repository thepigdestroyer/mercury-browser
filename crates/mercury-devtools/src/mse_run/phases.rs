// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Which tests each phase runs, in which order, and the URL of each.

use std::fs;
use std::io;
use std::path::Path;

use super::model::Phase;
use crate::serve::wpt;

/// The first pages, in the order the brief names them: `mse-basic` first (if it fails, nothing
/// after it is worth reading), the rest of round 1, then the pages of round 2. Pages that are
/// not named here follow in name order.
pub const PAGE_ORDER: [&str; 11] = [
    "mse-basic.html",
    "mse-split-av.html",
    "mse-switch.html",
    "mse-seek.html",
    "mse-remove.html",
    "mse-quota.html",
    "mse-object-url.html",
    "mse-abort.html",
    "mse-tracks.html",
    "mse-underrun.html",
    "mse-quality.html",
];

/// The player pages, in order.
pub const PLAYER_ORDER: [&str; 3] = ["dashjs.html", "shaka.html", "hlsjs.html"];

/// One test of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub phase: Phase,
    /// The key in the journal: a page file name, or a WPT path.
    pub id: String,
    /// The URL path without the query, starting with `/`.
    pub url_path: String,
    /// The watchdog multiplier: 2 for a WPT file that asks for the long timeout.
    pub timeout_factor: u32,
}

fn sorted_by(order: &[&str], mut names: Vec<String>) -> Vec<String> {
    names.sort_by(|a, b| {
        let rank = |name: &String| {
            order
                .iter()
                .position(|first| first == name)
                .unwrap_or(order.len())
        };
        rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
    });
    names
}

fn html_files(dir: &Path, prefix: &str, skip: &[&str]) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_file()
            && name.starts_with(prefix)
            && name.ends_with(".html")
            && !skip.contains(&name.as_str())
        {
            names.push(name);
        }
    }
    Ok(names)
}

/// `tests/pages/mse-*.html`.
pub fn plan_pages(tests_root: &Path) -> io::Result<Vec<Planned>> {
    let names = html_files(&tests_root.join("pages"), "mse-", &[])?;
    Ok(sorted_by(&PAGE_ORDER, names)
        .into_iter()
        .map(|name| Planned {
            phase: Phase::Pages,
            url_path: format!("/pages/{name}"),
            id: name,
            timeout_factor: 1,
        })
        .collect())
}

/// `tests/pages/mse-players/*.html` except the index.
pub fn plan_players(tests_root: &Path) -> io::Result<Vec<Planned>> {
    let names = html_files(&tests_root.join("pages/mse-players"), "", &["index.html"])?;
    Ok(sorted_by(&PLAYER_ORDER, names)
        .into_iter()
        .map(|name| Planned {
            phase: Phase::Players,
            url_path: format!("/pages/mse-players/{name}"),
            id: name,
            timeout_factor: 1,
        })
        .collect())
}

/// The WPT files of `media-source/`, and how many reftest pages were left out.
pub fn plan_wpt(wpt_root: &Path) -> io::Result<(Vec<Planned>, usize)> {
    let mut reftests = 0;
    let mut planned = Vec::new();
    for test in wpt::discover(wpt_root, "media-source")? {
        if test.kind == wpt::Kind::Reftest {
            reftests += 1;
            continue;
        }
        planned.push(Planned {
            phase: Phase::Wpt,
            url_path: format!("/{}", test.path),
            id: test.path,
            timeout_factor: if test.long_timeout { 2 } else { 1 },
        });
    }
    Ok((planned, reftests))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "<!doctype html>").unwrap();
    }

    #[test]
    fn pages_come_in_the_briefs_order_with_the_others_after() {
        let dir = tempfile::tempdir().unwrap();
        let pages = dir.path().join("pages");
        for name in [
            "mse-quality.html",
            "mse-zeta.html",
            "mse-basic.html",
            "mse-alpha.html",
            "mse-abort.html",
            "media.html",
            "colors.html",
        ] {
            touch(&pages.join(name));
        }
        touch(&pages.join("mse-players/dashjs.html"));
        let ids: Vec<String> = plan_pages(dir.path())
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(
            ids,
            [
                "mse-basic.html",
                "mse-abort.html",
                "mse-quality.html",
                "mse-alpha.html",
                "mse-zeta.html"
            ]
        );
    }

    #[test]
    fn players_are_dash_shaka_hls_and_the_index_is_not_a_test() {
        let dir = tempfile::tempdir().unwrap();
        let players = dir.path().join("pages/mse-players");
        for name in ["index.html", "hlsjs.html", "shaka.html", "dashjs.html"] {
            touch(&players.join(name));
        }
        let planned = plan_players(dir.path()).unwrap();
        let ids: Vec<&str> = planned.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["dashjs.html", "shaka.html", "hlsjs.html"]);
        assert_eq!(planned[0].url_path, "/pages/mse-players/dashjs.html");
    }

    #[test]
    fn wpt_plans_skip_reftests_and_double_long_timeouts() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("media-source");
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join("a.html"), "x").unwrap();
        fs::write(tree.join("long.html"), "<meta name=timeout content=long>").unwrap();
        fs::write(tree.join("r.html"), "<link rel=match href=r-ref.html>").unwrap();
        let (planned, reftests) = plan_wpt(dir.path()).unwrap();
        assert_eq!(reftests, 1);
        let summary: Vec<(&str, u32)> = planned
            .iter()
            .map(|p| (p.url_path.as_str(), p.timeout_factor))
            .collect();
        assert_eq!(
            summary,
            [("/media-source/a.html", 1), ("/media-source/long.html", 2)]
        );
    }
}
