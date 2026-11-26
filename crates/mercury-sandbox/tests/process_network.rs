// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What a confined content process can do with the network: nothing. Each test starts a child
//! that confines itself and then tries; the child's exit code is the verdict.

#[path = "support/mod.rs"]
mod support;

use support::{Test, Tree};

const TESTS: &[Test] = &[
    (
        "loopback_connect_is_eperm_and_listener_sees_nothing",
        loopback_connect_is_eperm_and_listener_sees_nothing,
    ),
    (
        "udp_send_bind_and_unix_connect_are_refused",
        udp_send_bind_and_unix_connect_are_refused,
    ),
    ("name_lookup_fails_fast", name_lookup_fails_fast),
];

const JOBS: &[Test] = &[
    ("loopback_connect", job_loopback_connect),
    ("udp", job_udp),
    ("lookup", job_lookup),
];

fn main() {
    support::run(TESTS, JOBS);
}

#[cfg(not(target_os = "macos"))]
fn loopback_connect_is_eperm_and_listener_sees_nothing() {}
#[cfg(not(target_os = "macos"))]
fn udp_send_bind_and_unix_connect_are_refused() {}
#[cfg(not(target_os = "macos"))]
fn name_lookup_fails_fast() {}

#[cfg(target_os = "macos")]
fn loopback_connect_is_eperm_and_listener_sees_nothing() {
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback listener");
    listener
        .set_nonblocking(true)
        .expect("make the listener non-blocking");
    let port = listener
        .local_addr()
        .expect("the listener's address")
        .port();
    let (sender, receiver) = mpsc::channel();
    let accepting = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok((stream, _)) = listener.accept() {
                drop(stream);
                let _ = sender.send(());
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
    });

    let tree = Tree::new("loopback", "home");
    let port_text = port.to_string();
    let (child, output) = support::spawn_child(
        "loopback_connect",
        &tree,
        &[("MERCURY_TEST_PORT", &port_text)],
    );
    support::expect_pass("loopback_connect", &support::finish(child, &output));

    accepting.join().expect("the listener thread ended");
    assert!(
        receiver.try_recv().is_err(),
        "the listener accepted a connection from the confined child"
    );
}

#[cfg(target_os = "macos")]
fn udp_send_bind_and_unix_connect_are_refused() {
    use std::os::unix::net::UnixListener;

    let tree = Tree::new("udp", "home");
    let socket = tree.root.join("listener.sock");
    let _listener =
        UnixListener::bind(&socket).expect("bind a Unix listener in the temporary directory");
    let path = socket.to_string_lossy().into_owned();
    support::verdict("udp", &tree, &[("MERCURY_TEST_SOCKET", &path)]);
}

#[cfg(target_os = "macos")]
fn name_lookup_fails_fast() {
    let tree = Tree::new("lookup", "home");
    support::verdict("lookup", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn job_loopback_connect() {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    support::child_confine();
    let port: u16 = support::child_env("MERCURY_TEST_PORT")
        .parse()
        .unwrap_or_else(|_| support::fail("MERCURY_TEST_PORT is not a port"));
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    support::expect_eperm(
        TcpStream::connect_timeout(&address, Duration::from_secs(5)),
        "a TCP connection to the loopback listener",
    );
}

#[cfg(target_os = "macos")]
fn job_udp() {
    use std::net::UdpSocket;
    use std::os::unix::net::UnixStream;

    support::child_confine();
    support::expect_eperm(UdpSocket::bind("127.0.0.1:0"), "binding a UDP socket");
    let socket = support::child_env("MERCURY_TEST_SOCKET");
    support::expect_eperm(UnixStream::connect(socket), "connecting a Unix socket");
}

#[cfg(target_os = "macos")]
fn job_lookup() {
    use std::net::ToSocketAddrs;
    use std::time::{Duration, Instant};

    support::child_confine();
    let started = Instant::now();
    let result = ("example.invalid", 80).to_socket_addrs();
    support::check(result.is_err(), "a name lookup succeeded");
    support::check(
        started.elapsed() < Duration::from_secs(20),
        "the name lookup took longer than a refusal should",
    );
}
