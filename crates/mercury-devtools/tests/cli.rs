// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The `serve-tests` and `package-manifest` executables: the command line, the port on
//! standard output, and how the server ends. `serve-tests` handles signals and watches its
//! parent process, which are Unix APIs, so its tests run on Unix only (elsewhere it must say so).

use std::io::Write;
use std::process::{Command, Stdio};

#[cfg(unix)]
mod serve_tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    const SERVE_TESTS: &str = env!("CARGO_BIN_EXE_serve-tests");

    fn start(arguments: &[&str]) -> (Child, u16) {
        let mut child = Command::new(SERVE_TESTS)
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start serve-tests");
        let mut line = String::new();
        BufReader::new(child.stdout.as_mut().expect("stdout"))
            .read_line(&mut line)
            .expect("read the port");
        let port = line
            .trim_end()
            .parse()
            .unwrap_or_else(|_| panic!("a port, not {line:?}"));
        (child, port)
    }

    fn fetch(port: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.write_all(request.as_bytes()).expect("send");
        stream.shutdown(Shutdown::Write).expect("shutdown");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read");
        response
    }

    fn wait_with_timeout(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait().expect("wait") {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    fn signal(child: &Child, signal: libc::c_int) {
        // SAFETY: the pid is our own child, still running or a zombie we have not reaped.
        assert_eq!(unsafe { libc::kill(child.id() as libc::pid_t, signal) }, 0);
    }

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|address| address.port())
            .expect("a free port")
    }

    #[test]
    fn prints_the_port_serves_tests_and_stops_on_each_signal() {
        for stop in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            let (mut child, port) = start(&[]);
            let response = fetch(port, "GET /pages/http-auth.html HTTP/1.0\r\n\r\n");
            assert!(response.starts_with("HTTP/1.0 200 OK\r\n"), "{response}");
            assert!(response.contains("\r\nContent-type: text/html\r\n"));
            let private = fetch(port, "GET /private HTTP/1.0\r\n\r\n");
            assert!(private.starts_with("HTTP/1.0 401 Unauthorized\r\n"));
            let range = fetch(
                port,
                "GET /media/tone.wav HTTP/1.0\r\nRange: bytes=0-3\r\n\r\n",
            );
            assert!(range.starts_with("HTTP/1.0 206 Partial Content\r\n"));
            assert!(range.ends_with("\r\n\r\nRIFF"));
            signal(&child, stop);
            let status = wait_with_timeout(&mut child, Duration::from_secs(5)).expect("it stops");
            assert_eq!(status.code(), Some(0), "signal {stop}");
        }
    }

    #[test]
    fn binds_the_port_it_is_given() {
        for form in ["--port", "--port="] {
            let wanted = free_port();
            let arguments: Vec<String> = if form == "--port" {
                vec![form.to_owned(), wanted.to_string()]
            } else {
                vec![format!("{form}{wanted}")]
            };
            let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
            let (mut child, port) = start(&arguments);
            assert_eq!(port, wanted);
            assert!(fetch(port, "HEAD / HTTP/1.0\r\n\r\n").starts_with("HTTP/1.0 200 OK\r\n"));
            // The port is taken now: a second server on it fails with status 1.
            let second = Command::new(SERVE_TESTS)
                .args(&arguments)
                .output()
                .expect("run");
            assert_eq!(second.status.code(), Some(1));
            assert!(second.stdout.is_empty());
            assert!(String::from_utf8_lossy(&second.stderr).contains("cannot serve"));
            signal(&child, libc::SIGTERM);
            assert_eq!(
                wait_with_timeout(&mut child, Duration::from_secs(5)).and_then(|s| s.code()),
                Some(0)
            );
        }
    }

    #[test]
    fn command_line_errors_and_help() {
        let help = Command::new(SERVE_TESTS)
            .arg("--help")
            .output()
            .expect("run");
        assert_eq!(help.status.code(), Some(0));
        assert!(
            String::from_utf8_lossy(&help.stdout)
                .starts_with("usage: serve-tests [-h] [--port PORT] [--log LOG]\n")
        );
        for arguments in [
            &["--port"][..],
            &["--port", "eighty"],
            &["--log"],
            &["extra"],
        ] {
            let output = Command::new(SERVE_TESTS)
                .args(arguments)
                .output()
                .expect("run");
            assert_eq!(output.status.code(), Some(2), "{arguments:?}");
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains("serve-tests: error: "));
        }
        let out_of_range = Command::new(SERVE_TESTS)
            .args(["--port", "70000"])
            .output()
            .expect("run");
        assert_eq!(out_of_range.status.code(), Some(1));
        assert!(out_of_range.stdout.is_empty());
    }

    #[test]
    fn appends_a_json_line_per_request_to_the_log_file() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let log = dir.path().join("requests.jsonl");
        let (mut child, port) = start(&["--log", log.to_str().expect("a UTF-8 path")]);
        let page = fetch(
            port,
            "GET /pages/http-auth.html HTTP/1.0\r\nCookie: a=b\r\nUser-Agent: probe\r\n\r\n",
        );
        assert!(page.starts_with("HTTP/1.0 200 OK\r\n"), "{page}");
        let download = fetch(port, "GET /download/x.bin?size=5 HTTP/1.0\r\n\r\n");
        assert!(
            download.ends_with("\r\n\r\n\0\x01\x02\x03\x04"),
            "{download:?}"
        );
        signal(&child, libc::SIGTERM);
        assert_eq!(
            wait_with_timeout(&mut child, Duration::from_secs(5)).and_then(|s| s.code()),
            Some(0)
        );
        let text = std::fs::read_to_string(&log).expect("the log file");
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).expect("a JSON line"))
            .collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert_eq!(lines[0]["method"], "GET");
        assert_eq!(lines[0]["path"], "/pages/http-auth.html");
        assert_eq!(lines[0]["cookie"], "a=b");
        assert_eq!(lines[0]["user_agent"], "probe");
        assert!(lines[0]["sent"].is_null() && lines[0]["complete"].is_null());
        assert_eq!(lines[1]["path"], "/download/x.bin?size=5");
        assert_eq!(
            (lines[1]["sent"].as_u64(), lines[1]["complete"].as_bool()),
            (Some(5), Some(true))
        );
    }

    #[test]
    fn exits_when_the_starting_process_goes_away() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let port_file = dir.path().join("serve.port");
        // The shell starts the server in the background, prints its pid and exits once the
        // server has written its port.
        let mut shell = Command::new("/bin/sh")
            .arg("-c")
            .arg("\"$0\" >\"$1\" & echo $!; while [ ! -s \"$1\" ]; do sleep 0.05; done")
            .arg(SERVE_TESTS)
            .arg(&port_file)
            .stdout(Stdio::piped())
            .spawn()
            .expect("start the shell");
        let mut pid = String::new();
        BufReader::new(shell.stdout.as_mut().expect("stdout"))
            .read_line(&mut pid)
            .expect("the pid");
        let pid: libc::pid_t = pid.trim().parse().expect("a pid");
        assert!(shell.wait().expect("the shell ends").success());
        let port = std::fs::read_to_string(&port_file).expect("the port file");
        assert!(port.trim_end().parse::<u16>().is_ok(), "{port:?}");
        let deadline = Instant::now() + Duration::from_secs(5);
        // SAFETY: signal 0 only checks that the process exists.
        while unsafe { libc::kill(pid, 0) } == 0 {
            if Instant::now() >= deadline {
                panic!("the server outlived the process that started it");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Started by a process that has already gone (so its parent is launchd, pid 1), the server
    /// exits at once without printing a port. Linux may hand orphans to a subreaper instead of
    /// pid 1, so this runs on macOS only.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_orphan_exits_at_once() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let port_file = dir.path().join("serve.port");
        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg("( sleep 0.3; exec \"$0\" >\"$1\" 2>/dev/null ) & echo $!")
            .arg(SERVE_TESTS)
            .arg(&port_file)
            .output()
            .expect("run the shell");
        let pid: libc::pid_t = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .expect("a pid");
        let deadline = Instant::now() + Duration::from_secs(10);
        // SAFETY: signal 0 only checks that the process exists.
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(
                Instant::now() < deadline,
                "the orphaned server is still running"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            std::fs::read_to_string(&port_file).expect("the port file"),
            ""
        );
    }
}

/// Without Unix signals and parent-process checks the server cannot run: it says so and fails.
#[cfg(not(unix))]
#[test]
fn serve_tests_says_it_needs_a_unix_system() {
    let output = Command::new(env!("CARGO_BIN_EXE_serve-tests"))
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "serve-tests: needs a Unix system (signal handling and parent-process watch)\n"
    );
}

#[test]
fn package_manifest_prints_matching_paths() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_package-manifest"))
        .arg("servo")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(br#"{"packages":[{"name":"servo","manifest_path":"/r/servo-0.8.0/Cargo.toml"},{"name":"url","manifest_path":"/r/url/Cargo.toml"}]}"#)
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"/r/servo-0.8.0/Cargo.toml\n");

    let usage = Command::new(env!("CARGO_BIN_EXE_package-manifest"))
        .output()
        .expect("run");
    assert_eq!(usage.status.code(), Some(2));
}
