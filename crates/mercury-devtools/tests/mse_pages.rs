// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The player pages and the server agree: every URL a player page loads is one the server answers
//! (with the pinned player file names of the fetch script), and each page has the elements the
//! shared script and the reporter need. No browser: this reads the pages as text.

use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use mercury_devtools::mse_run::phases::plan_players;
use mercury_devtools::serve::Server;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn status(port: u16, path: &str) -> u16 {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    String::from_utf8_lossy(&raw)
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

#[test]
fn there_is_a_page_for_each_of_the_three_players() {
    let tests = repo().join("tests");
    let ids: Vec<String> = plan_players(&tests)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, ["dashjs.html", "shaka.html", "hlsjs.html"]);
    let index = fs::read_to_string(tests.join("pages/mse-players/index.html")).unwrap();
    for id in &ids {
        assert!(
            index.contains(&format!("href=\"{id}\"")),
            "the index links {id}"
        );
    }
}

#[test]
fn every_player_page_has_what_the_shared_script_needs() {
    let dir = repo().join("tests/pages/mse-players");
    for id in ["dashjs.html", "shaka.html", "hlsjs.html"] {
        let page = fs::read_to_string(dir.join(id)).unwrap();
        for needle in [
            "<script src=\"common.js\"></script>",
            "id=\"out\"",
            "id=\"sum\"",
            "id=\"v\"",
            "kit.run(",
            "kit.standardRun(",
            "SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0",
        ] {
            assert!(page.contains(needle), "{id}: {needle}");
        }
        assert!(page.contains("<title>MSE players:"), "{id}");
    }
    let common = fs::read_to_string(dir.join("common.js")).unwrap();
    for needle in ["\"line \" + cls", "(done)", "document.title", "window.kit"] {
        assert!(common.contains(needle), "common.js: {needle}");
    }
}

#[test]
fn the_files_and_streams_the_pages_load_are_served() {
    let repo = repo();
    let script = fs::read_to_string(repo.join("scripts/dev/fetch-mse-tools.sh")).unwrap();
    let tools = tempfile::tempdir().unwrap();
    fs::create_dir_all(tools.path().join("players")).unwrap();
    let mut loaded = Vec::new();
    for id in ["dashjs.html", "shaka.html", "hlsjs.html"] {
        let page = fs::read_to_string(repo.join("tests/pages/mse-players").join(id)).unwrap();
        let mut rest = page.as_str();
        while let Some(at) = rest.find("\"/mse-") {
            let tail = &rest[at + 1..];
            let end = tail.find('"').unwrap();
            loaded.push(tail[..end].to_owned());
            rest = &tail[end..];
        }
    }
    assert!(loaded.len() >= 5, "{loaded:?}");
    let server = Server::bind(&repo.join("tests"), 0)
        .unwrap()
        .with_mse_tools_dir(tools.path())
        .spawn();
    for url in &loaded {
        if let Some(file) = url.strip_prefix("/mse-players/") {
            assert!(
                script.contains(file),
                "{url}: the fetch script does not name {file}"
            );
            fs::write(tools.path().join("players").join(file), "//").unwrap();
        }
        assert_eq!(status(server, url), 200, "{url}");
    }
    assert!(loaded.contains(&"/mse-dash/h264/manifest.mpd".to_owned()));
    assert!(loaded.contains(&"/mse-dash/vp9/manifest.mpd".to_owned()));
    assert!(loaded.contains(&"/mse-hls/h264/master.m3u8".to_owned()));
}
