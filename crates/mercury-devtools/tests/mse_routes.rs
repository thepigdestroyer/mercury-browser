// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The routes of the MSE window run over loopback: the generated DASH and HLS streams, the fetched
//! players and the WPT tree, the report endpoint and the reporter script.

use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mercury_devtools::serve::Server;

struct Response {
    status_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn status(&self) -> u16 {
        self.status_line
            .split(' ')
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0)
    }
}

fn exchange(port: u16, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    stream.write_all(request).expect("send");
    stream.shutdown(Shutdown::Write).expect("shutdown");
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .expect("the server closes the connection");
    raw
}

fn parse(raw: Vec<u8>) -> Response {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a header block");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default().to_owned();
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(": ").expect("a header");
            (name.to_owned(), value.to_owned())
        })
        .collect();
    Response {
        status_line,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

fn request(port: u16, method: &str, path: &str, headers: &[&str], body: &[u8]) -> Response {
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for header in headers {
        head.push_str(header);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    parse(exchange(port, &bytes))
}

fn get(port: u16, path: &str) -> Response {
    request(port, "GET", path, &[], b"")
}

struct Fixture {
    _dir: tempfile::TempDir,
    port: u16,
    reports: Arc<Mutex<Vec<Vec<u8>>>>,
}

/// A server over a small `tests/` tree and a tools directory with a fake WPT checkout and players.
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("temporary directory");
    let tests = dir.path().join("tests");
    fs::create_dir_all(tests.join("pages")).unwrap();
    fs::write(
        tests.join("pages/p.html"),
        "<!DOCTYPE html>\n<html>\n<head>\n<title>p</title>\n</head>\n<body>x</body>\n</html>\n",
    )
    .unwrap();
    fs::write(dir.path().join("secret.txt"), "secret").unwrap();
    let tools = dir.path().join("tools");
    fs::create_dir_all(tools.join("players")).unwrap();
    fs::write(tools.join("players/dash.all.min.js"), "0123456789").unwrap();
    for sub in ["media-source", "resources", "common", "interfaces"] {
        fs::create_dir_all(tools.join("wpt").join(sub)).unwrap();
    }
    fs::write(tools.join("wpt/resources/testharness.js"), "// testharness").unwrap();
    fs::write(
        tools.join("wpt/resources/testharnessreport.js"),
        "// the original report",
    )
    .unwrap();
    fs::write(tools.join("wpt/media-source/a.html"), "<title>a</title>").unwrap();
    fs::write(
        tools.join("wpt/media-source/idl.any.js"),
        "// META: script=/resources/WebIDLParser.js\n// META: script=helper.js\n// META: timeout=long\n'use strict';\n",
    )
    .unwrap();
    fs::write(
        tools.join("wpt/media-source/w.window.js"),
        "// window test\n",
    )
    .unwrap();
    fs::write(dir.path().join("tools/outside.txt"), "outside").unwrap();
    let reports = Arc::new(Mutex::new(Vec::new()));
    let sink_reports = Arc::clone(&reports);
    let port = Server::bind(&tests, 0)
        .expect("bind")
        .with_mse_tools_dir(&tools)
        .with_report_sink(Arc::new(move |body: &[u8]| {
            sink_reports.lock().unwrap().push(body.to_vec());
        }))
        .spawn();
    Fixture {
        _dir: dir,
        port,
        reports,
    }
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

struct MpdRepresentation {
    id: String,
    init: String,
    media: String,
    segments: Vec<(u64, u64)>,
}

/// A structural read of the generated MPD: each representation's templates and expanded timeline.
fn representations(mpd: &str) -> Vec<MpdRepresentation> {
    let mut found = Vec::new();
    for block in mpd.split("<Representation ").skip(1) {
        let block = block.split("</Representation>").next().unwrap();
        let id = attribute(&format!(" {block}"), "id").expect("a Representation id");
        let template_start = block.find("<SegmentTemplate ").expect("a SegmentTemplate");
        let template =
            &block[template_start..block[template_start..].find('>').unwrap() + template_start];
        let mut segments = Vec::new();
        let mut next_start = 0;
        for tag in block.split("<S ").skip(1) {
            let tag = format!(" {}", tag.split("/>").next().unwrap());
            let start = attribute(&tag, "t").map_or(next_start, |t| t.parse().unwrap());
            let duration: u64 = attribute(&tag, "d").unwrap().parse().unwrap();
            let repeats: u64 = attribute(&tag, "r").map_or(0, |r| r.parse().unwrap());
            for index in 0..=repeats {
                segments.push((start + index * duration, duration));
            }
            next_start = start + (repeats + 1) * duration;
        }
        found.push(MpdRepresentation {
            id,
            init: attribute(template, "initialization").unwrap(),
            media: attribute(template, "media").unwrap(),
            segments,
        });
    }
    found
}

#[test]
fn the_dash_manifests_describe_segments_that_rebuild_each_fixture() {
    let fixture = fixture();
    for (set, renditions) in [
        ("h264", vec!["h264-low", "h264-high", "aac"]),
        ("vp9", vec!["vp9-low", "vp9-high", "opus"]),
    ] {
        let manifest = get(fixture.port, &format!("/mse-dash/{set}/manifest.mpd"));
        assert_eq!(manifest.status(), 200, "{set}");
        assert_eq!(
            manifest.header("Content-Type"),
            Some("application/dash+xml")
        );
        let mpd = manifest.text();
        assert!(mpd.starts_with("<?xml"), "{mpd}");
        assert!(mpd.contains("type=\"static\""));
        assert!(mpd.contains("mediaPresentationDuration=\"PT8S\""), "{mpd}");
        let found = representations(&mpd);
        let ids: Vec<&str> = found.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, renditions, "{set}");
        for representation in found {
            let total: u64 = representation.segments.iter().map(|(_, d)| d).sum();
            assert_eq!(total, 8000, "{} durations add up to 8 s", representation.id);
            assert!(representation.segments.len() >= 2, "{}", representation.id);
            let mut expected_start = 0;
            for (start, duration) in &representation.segments {
                assert_eq!(*start, expected_start, "{}: contiguous", representation.id);
                expected_start += duration;
            }
            let url = |template: &str, number: usize| {
                format!(
                    "/mse-dash/{set}/{}",
                    template
                        .replace("$RepresentationID$", &representation.id)
                        .replace("$Number$", &number.to_string())
                )
            };
            let init = get(fixture.port, &url(&representation.init, 0));
            assert_eq!(init.status(), 200, "{}", url(&representation.init, 0));
            let mut rebuilt = init.body.clone();
            for number in 1..=representation.segments.len() {
                let segment = get(fixture.port, &url(&representation.media, number));
                assert_eq!(
                    segment.status(),
                    200,
                    "{}",
                    url(&representation.media, number)
                );
                rebuilt.extend_from_slice(&segment.body);
            }
            assert_eq!(
                get(
                    fixture.port,
                    &url(&representation.media, representation.segments.len() + 1)
                )
                .status(),
                404,
                "{}: one segment past the end",
                representation.id
            );
            let original = mercury_mse_testkit::read_fixture(&representation.id).unwrap();
            assert!(
                rebuilt == original,
                "{}: init and segments, concatenated, are the fixture's bytes ({} against {})",
                representation.id,
                rebuilt.len(),
                original.len()
            );
        }
    }
}

#[test]
fn the_hls_playlists_list_segments_that_rebuild_each_fixture() {
    let fixture = fixture();
    let master = get(fixture.port, "/mse-hls/h264/master.m3u8");
    assert_eq!(master.status(), 200);
    assert_eq!(
        master.header("Content-Type"),
        Some("application/vnd.apple.mpegurl")
    );
    let master = master.text();
    assert!(master.starts_with("#EXTM3U\n"));
    let variants: Vec<&str> = master
        .lines()
        .filter(|l| l.starts_with("#EXT-X-STREAM-INF:"))
        .collect();
    assert_eq!(variants.len(), 2, "{master}");
    assert!(
        variants[0].contains("RESOLUTION=320x180") && variants[1].contains("RESOLUTION=640x360"),
        "{master}"
    );
    assert!(
        variants.iter().all(|v| v.contains("CODECS=\"avc1.")
            && v.contains("mp4a.40.2")
            && v.contains("AUDIO=\"aud\"")),
        "{master}"
    );
    assert!(
        master.contains("#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\""),
        "{master}"
    );
    let playlists: Vec<String> = master
        .lines()
        .filter(|l| l.ends_with(".m3u8\"") || (l.ends_with(".m3u8") && !l.starts_with('#')))
        .map(|l| {
            l.rsplit('"')
                .nth(if l.ends_with('"') { 1 } else { 0 })
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(playlists.len(), 3, "{playlists:?}");
    for playlist in playlists {
        let id = playlist.trim_end_matches(".m3u8");
        let media = get(fixture.port, &format!("/mse-hls/h264/{playlist}"));
        assert_eq!(media.status(), 200, "{playlist}");
        let text = media.text();
        assert!(
            text.contains("#EXT-X-PLAYLIST-TYPE:VOD")
                && text.trim_end().ends_with("#EXT-X-ENDLIST"),
            "{text}"
        );
        let map = text
            .lines()
            .find_map(|l| l.strip_prefix("#EXT-X-MAP:URI=\""))
            .and_then(|l| l.strip_suffix('"'))
            .expect("an init segment")
            .to_owned();
        let mut total = 0.0f64;
        let mut files = Vec::new();
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            if let Some(duration) = line.strip_prefix("#EXTINF:") {
                total += duration.trim_end_matches(',').parse::<f64>().unwrap();
                files.push(lines.next().expect("a segment name").to_owned());
            }
        }
        assert!((total - 8.0).abs() < 1e-6, "{playlist}: {total} s");
        let mut rebuilt = get(fixture.port, &format!("/mse-hls/h264/{map}")).body;
        for file in &files {
            let segment = get(fixture.port, &format!("/mse-hls/h264/{file}"));
            assert_eq!(segment.status(), 200, "{file}");
            rebuilt.extend_from_slice(&segment.body);
        }
        let original = mercury_mse_testkit::read_fixture(id).unwrap();
        assert!(
            rebuilt == original,
            "{id}: rebuilt {} bytes, fixture {}",
            rebuilt.len(),
            original.len()
        );
    }
}

#[test]
fn generated_segments_answer_ranges_and_head() {
    let fixture = fixture();
    let whole = get(fixture.port, "/mse-dash/h264/h264-low-init.mp4");
    assert_eq!(whole.status(), 200);
    assert_eq!(whole.header("Content-Type"), Some("video/mp4"));
    assert_eq!(whole.header("Accept-Ranges"), Some("bytes"));
    let size = whole.body.len();
    assert!(size > 100);
    let part = request(
        fixture.port,
        "GET",
        "/mse-dash/h264/h264-low-init.mp4",
        &["Range: bytes=4-11"],
        b"",
    );
    assert_eq!(part.status(), 206);
    assert_eq!(
        part.header("Content-Range"),
        Some(format!("bytes 4-11/{size}").as_str())
    );
    assert_eq!(part.body, whole.body[4..12]);
    let tail = request(
        fixture.port,
        "GET",
        "/mse-dash/h264/h264-low-init.mp4",
        &["Range: bytes=-10"],
        b"",
    );
    assert_eq!(tail.body, whole.body[size - 10..]);
    let past = request(
        fixture.port,
        "GET",
        "/mse-dash/h264/h264-low-init.mp4",
        &[&format!("Range: bytes={size}-")],
        b"",
    );
    assert_eq!(past.status(), 416);
    let head = request(
        fixture.port,
        "HEAD",
        "/mse-dash/h264/h264-low-init.mp4",
        &[],
        b"",
    );
    assert_eq!(head.status(), 200);
    assert_eq!(
        head.header("Content-Length"),
        Some(size.to_string().as_str())
    );
    assert!(head.body.is_empty());
    let webm = get(fixture.port, "/mse-dash/vp9/vp9-low-init.webm");
    assert_eq!(webm.header("Content-Type"), Some("video/webm"));
    let audio = get(fixture.port, "/mse-dash/h264/aac-init.mp4");
    assert_eq!(audio.header("Content-Type"), Some("audio/mp4"));
}

#[test]
fn dot_dot_and_encoded_dot_dot_reach_nothing_outside_a_route() {
    let fixture = fixture();
    for path in [
        "/mse-dash/h264/../../../Cargo.toml",
        "/mse-dash/%2e%2e/%2e%2e/Cargo.toml",
        "/mse-dash/h264/%2e%2e%2f%2e%2e%2fCargo.toml",
        "/mse-dash/h264/..%2f..%2fCargo.toml",
        "/mse-hls/h264/%2E%2E/%2E%2E/Cargo.toml",
        "/mse-hls/../h264/master.m3u8",
        "/mse-players/../outside.txt",
        "/mse-players/%2e%2e/outside.txt",
        "/mse-players/..%2foutside.txt",
        "/mse-players/%2e%2e%5coutside.txt",
        "/wpt/../../outside.txt",
        "/wpt/%2e%2e/%2e%2e/secret.txt",
        "/wpt/media-source/../../../secret.txt",
        "/resources/../../outside.txt",
        "/resources/%2e%2e/%2e%2e/outside.txt",
        "/media-source/..%2f..%2foutside.txt",
    ] {
        let response = get(fixture.port, path);
        assert_eq!(response.status(), 404, "{path}");
        let text = response.text();
        assert!(
            !text.contains("outside") && !text.contains("secret"),
            "{path}: {text}"
        );
    }
}

#[test]
fn the_players_are_served_from_the_tools_directory_with_ranges() {
    let fixture = fixture();
    let whole = get(fixture.port, "/mse-players/dash.all.min.js");
    assert_eq!(whole.status(), 200);
    assert_eq!(whole.header("Content-type"), Some("text/javascript"));
    assert_eq!(whole.text(), "0123456789");
    let part = request(
        fixture.port,
        "GET",
        "/mse-players/dash.all.min.js",
        &["Range: bytes=2-4"],
        b"",
    );
    assert_eq!(part.status(), 206);
    assert_eq!(part.text(), "234");
    assert_eq!(get(fixture.port, "/mse-players/nope.js").status(), 404);
}

#[test]
fn the_wpt_tree_is_served_at_wpt_and_at_the_root_names_its_pages_use() {
    let fixture = fixture();
    for path in [
        "/wpt/media-source/a.html",
        "/media-source/a.html",
        "/media-source/a.html?__x=1",
    ] {
        let page = get(fixture.port, path);
        assert_eq!(page.status(), 200, "{path}");
        assert_eq!(page.text(), "<title>a</title>");
    }
    assert_eq!(
        get(fixture.port, "/resources/testharness.js").text(),
        "// testharness"
    );
    assert_eq!(
        get(fixture.port, "/wpt/resources/testharness.js").text(),
        "// testharness"
    );
    assert_eq!(
        get(fixture.port, "/media-source/missing.html").status(),
        404
    );
}

#[test]
fn testharnessreport_is_replaced_by_the_report_script() {
    let fixture = fixture();
    for path in [
        "/resources/testharnessreport.js",
        "/wpt/resources/testharnessreport.js",
    ] {
        let script = get(fixture.port, path);
        assert_eq!(script.status(), 200, "{path}");
        let text = script.text();
        assert!(
            text.contains("/__report") && text.contains("add_completion_callback"),
            "{path}"
        );
        assert!(!text.contains("the original report"), "{path}");
    }
}

#[test]
fn any_and_window_pages_are_made_from_their_js_files() {
    let fixture = fixture();
    let any = get(fixture.port, "/media-source/idl.any.html");
    assert_eq!(any.status(), 200);
    assert_eq!(any.header("Content-Type"), Some("text/html"));
    let page = any.text();
    assert!(page.contains("self.GLOBAL"), "{page}");
    assert!(
        page.contains("<script src=\"/resources/WebIDLParser.js\">"),
        "{page}"
    );
    assert!(page.contains("<script src=\"helper.js\">"), "{page}");
    assert!(page.contains("<script src=\"idl.any.js\">"), "{page}");
    assert!(page.contains("content=\"long\""), "{page}");
    let window = get(fixture.port, "/media-source/w.window.html");
    assert_eq!(window.status(), 200);
    assert!(!window.text().contains("self.GLOBAL"));
    assert!(window.text().contains("w.window.js"));
    assert_eq!(
        get(fixture.port, "/media-source/none.any.html").status(),
        404
    );
    // The .js file itself is a file like any other.
    assert!(
        get(fixture.port, "/media-source/idl.any.js")
            .text()
            .contains("META: script")
    );
}

#[test]
fn missing_tools_say_how_to_fetch_them() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("tests")).unwrap();
    let port = Server::bind(&dir.path().join("tests"), 0)
        .unwrap()
        .with_mse_tools_dir(dir.path().join("none"))
        .spawn();
    for path in [
        "/mse-players/dash.all.min.js",
        "/wpt/media-source/a.html",
        "/resources/testharness.js",
    ] {
        let response = get(port, path);
        assert_eq!(response.status(), 404, "{path}");
        assert!(response.text().contains("fetch-mse-tools.sh"), "{path}");
    }
}

#[test]
fn reports_go_to_the_sink_and_answer_no_content() {
    let fixture = fixture();
    let body = br#"{"token":"t","kind":"hello"}"#;
    let response = request(
        fixture.port,
        "POST",
        "/__report",
        &[
            "Content-Type: application/json",
            &format!("Content-Length: {}", body.len()),
        ],
        body,
    );
    assert_eq!(response.status(), 204);
    assert!(response.body.is_empty());
    assert_eq!(fixture.reports.lock().unwrap().as_slice(), [body.to_vec()]);

    let no_length = request(fixture.port, "POST", "/__report", &[], b"");
    assert_eq!(no_length.status(), 400);
    let too_big = request(
        fixture.port,
        "POST",
        "/__report",
        &["Content-Length: 99999999"],
        b"",
    );
    assert_eq!(too_big.status(), 413);
    assert_eq!(
        fixture.reports.lock().unwrap().len(),
        1,
        "refused reports reach nobody"
    );
    assert_eq!(
        get(fixture.port, "/__report").status(),
        404,
        "a GET is no report"
    );
}

#[test]
fn a_page_opened_with_a_token_gets_the_reporter_first_in_its_head() {
    let fixture = fixture();
    let plain = get(fixture.port, "/pages/p.html");
    assert!(!plain.text().contains("reporter.js"));
    let instrumented = get(fixture.port, "/pages/p.html?__mse_run=tok-1");
    assert_eq!(instrumented.status(), 200);
    assert_eq!(
        instrumented.text(),
        "<!DOCTYPE html>\n<html>\n<head><script src=\"/__mse/reporter.js\"></script>\n<title>p</title>\n</head>\n<body>x</body>\n</html>\n"
    );
    assert_eq!(
        instrumented.header("Content-Length"),
        Some(instrumented.body.len().to_string().as_str())
    );
    assert_eq!(
        get(fixture.port, "/pages/absent.html?__mse_run=t").status(),
        404
    );
    let script = get(fixture.port, "/__mse/reporter.js");
    assert_eq!(script.status(), 200);
    let text = script.text();
    assert!(text.contains("__mse_run") && text.contains("/__report") && text.contains("(done)"));
}
