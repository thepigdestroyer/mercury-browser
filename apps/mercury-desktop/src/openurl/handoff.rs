// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A second launch hands its addresses to the Mercury Browser already using the profile.
//!
//! The running browser listens on a Unix-domain socket in the profile directory (private to this
//! user, like the directory). A launch that finds the profile locked connects, sends the
//! addresses from its command line and exits 0 once the running browser has opened them. On
//! macOS a launch from Finder or another app never gets here (LaunchServices hands the URL to the
//! running app as an Apple event); this is for the command line, and for Linux.
//!
//! The addresses travel as typed and the running browser resolves them as it resolves the first
//! launch's (its search engine included): the command line is the user's own input. The message
//! is UTF-8 text: the line `mercury-open-addresses 1`, then one address per line (no control
//! characters, at most `MAX_ADDRESS_BYTES`), then an empty line; at most `MAX_ADDRESSES`, and at
//! most `MAX_MESSAGE_BYTES` in all. Anything else is refused whole. The running browser answers
//! only once it has opened the tabs: `ok <count>`, or `error <reason>` (then nothing was opened).

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The socket's name in the profile directory.
pub(crate) const SOCKET_NAME: &str = "urls.sock";
const HEADER: &str = "mercury-open-addresses 1";
pub(crate) const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_ADDRESSES: usize = 64;
pub(crate) const MAX_ADDRESS_BYTES: usize = 8 * 1024;
const MAX_REPLY_BYTES: u64 = 1024;
/// How long the running browser waits for a second launch's message.
const IO_TIMEOUT: Duration = Duration::from_secs(3);
/// After answering, the running browser reads and drops at most this much more from the sender,
/// for at most `DRAIN_TIMEOUT`, so that closing the socket does not reset the connection under
/// the reply.
const MAX_DRAIN_BYTES: usize = 1024 * 1024;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
/// How long the second launch waits for the answer, which comes once the tabs are open: longer
/// than the running browser ever waits for its window (`HAND_OVER_TIMEOUT` and
/// `HAND_OVER_FINISH_TIMEOUT` in `openurl`, 10 s each), so it always hears the answer.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_secs(25);
/// A browser that has just locked the profile may not be listening yet.
const CONNECT_ATTEMPTS: u32 = 10;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(200);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HandoffError {
    /// The command line names no address.
    NothingToOpen,
    TooLarge,
    Malformed(String),
    /// The socket, or the process at its other end, is not this user's own; nothing was sent or
    /// accepted.
    UnsafeSocket(String),
    /// No browser answered on the socket.
    NotListening(String),
    /// The running browser did not open them.
    Refused(String),
}

impl std::fmt::Display for HandoffError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingToOpen => formatter.write_str("the command line names no address"),
            Self::TooLarge => write!(
                formatter,
                "more than {MAX_ADDRESSES} addresses or {MAX_MESSAGE_BYTES} bytes"
            ),
            Self::Malformed(reason) => write!(formatter, "malformed message: {reason}"),
            Self::UnsafeSocket(reason) => write!(formatter, "not safe to use: {reason}"),
            Self::NotListening(reason) => {
                write!(formatter, "the browser is not listening: {reason}")
            }
            Self::Refused(reason) => write!(formatter, "it did not open them: {reason}"),
        }
    }
}

/// Whether `line` can travel as one address: not blank, not too long, without control characters
/// (so no line breaks). What it means is decided by the running browser.
fn is_acceptable_address(line: &str) -> bool {
    !line.trim().is_empty()
        && line.len() <= MAX_ADDRESS_BYTES
        && !line.chars().any(char::is_control)
}

pub(crate) fn encode(addresses: &[String]) -> Vec<u8> {
    let mut message = String::from(HEADER);
    message.push('\n');
    for address in addresses {
        message.push_str(address);
        message.push('\n');
    }
    message.push('\n');
    message.into_bytes()
}

pub(crate) fn decode(message: &[u8]) -> Result<Vec<String>, HandoffError> {
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(HandoffError::TooLarge);
    }
    let text = std::str::from_utf8(message)
        .map_err(|_| HandoffError::Malformed("not UTF-8".to_owned()))?;
    let body = text
        .strip_suffix("\n\n")
        .ok_or_else(|| HandoffError::Malformed("not ended by an empty line".to_owned()))?;
    let mut lines = body.split('\n');
    if lines.next() != Some(HEADER) {
        return Err(HandoffError::Malformed("unknown header".to_owned()));
    }
    let addresses = lines.map(str::to_owned).collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(HandoffError::Malformed("no address".to_owned()));
    }
    if addresses.len() > MAX_ADDRESSES {
        return Err(HandoffError::TooLarge);
    }
    if let Some(bad) = addresses
        .iter()
        .position(|address| !is_acceptable_address(address))
    {
        return Err(HandoffError::Malformed(format!(
            "line {} is blank, too long or has control characters",
            bad + 2
        )));
    }
    Ok(addresses)
}

pub(crate) fn socket_path(profile_root: &Path) -> PathBuf {
    profile_root.join(SOCKET_NAME)
}

/// A path is safe to connect to only if it is a socket (not a link) of this user that nobody
/// else can use.
fn check_socket(path: &Path) -> Result<(), HandoffError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| HandoffError::NotListening(format!("{}: {error}", path.display())))?;
    if !metadata.file_type().is_socket() {
        return Err(HandoffError::UnsafeSocket(format!(
            "{} is not a socket",
            path.display()
        )));
    }
    // SAFETY: `getuid` has no preconditions.
    if metadata.uid() != unsafe { libc::getuid() } {
        return Err(HandoffError::UnsafeSocket(format!(
            "{} belongs to another user",
            path.display()
        )));
    }
    if metadata.mode() & 0o077 != 0 {
        return Err(HandoffError::UnsafeSocket(format!(
            "others may use {} (mode {:o})",
            path.display(),
            metadata.mode() & 0o777
        )));
    }
    Ok(())
}

/// Removes a socket left by a run that ended without removing it. Called only while this run
/// holds the profile's lock, so no other browser can be listening on it. Anything that is not a
/// socket of this user is left exactly as it is, and refused.
fn remove_stale_socket(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        // SAFETY: `getuid` has no preconditions.
        Ok(metadata)
            if metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::getuid() } =>
        {
            std::fs::remove_file(path)
        }
        Ok(_) => Err(io::Error::other(format!(
            "{} exists and is not a socket of this user; leaving it alone",
            path.display()
        ))),
    }
}

/// Only this user's processes may hand addresses over.
fn peer_is_allowed(peer_uid: io::Result<u32>, own_uid: u32) -> Result<(), HandoffError> {
    match peer_uid {
        Ok(uid) if uid == own_uid => Ok(()),
        Ok(uid) => Err(HandoffError::UnsafeSocket(format!(
            "the sender runs as user {uid}"
        ))),
        Err(error) => Err(HandoffError::UnsafeSocket(format!(
            "the sender cannot be identified: {error}"
        ))),
    }
}

/// The user id of the process at the other end of `stream`.
fn peer_user_id(stream: &UnixStream) -> io::Result<u32> {
    use std::os::fd::AsRawFd;

    #[cfg(target_os = "macos")]
    {
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        // SAFETY: the descriptor is a connected Unix-domain socket; both pointers are valid.
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(uid)
    }
    #[cfg(target_os = "linux")]
    {
        let mut credentials = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: the descriptor is a connected Unix-domain socket; the buffer and its length
        // match `SO_PEERCRED`'s `struct ucred`.
        if unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&raw mut credentials).cast(),
                &mut length,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(credentials.uid)
    }
}

/// Listens for second launches until dropped; then the socket is removed.
pub(crate) struct HandoffListener {
    path: PathBuf,
    /// Device and inode of the socket this run created, so only it is ever removed.
    identity: (u64, u64),
    stop: Arc<AtomicBool>,
}

/// What the running browser does with accepted addresses: opens them and says how many tabs it
/// opened, or why it opened none. Runs on the listening thread and may block until the window
/// has answered.
pub(crate) type OpenAddresses = fn(Vec<String>) -> Result<usize, String>;

/// Starts listening on the profile's socket.
pub(crate) fn listen(profile_root: &Path, open: OpenAddresses) -> io::Result<HandoffListener> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let path = socket_path(profile_root);
    remove_stale_socket(&path)?;
    let listener = UnixListener::bind(&path)?;
    // The profile directory is already private; the socket is too.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let metadata = std::fs::symlink_metadata(&path)?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    std::thread::Builder::new()
        .name("mercury-handoff".to_owned())
        .spawn(move || {
            for stream in listener.incoming() {
                if thread_stop.load(Ordering::Acquire) {
                    break;
                }
                match stream {
                    Ok(stream) => serve(stream, open),
                    Err(error) => log::warn!("openurl: a second launch could not connect: {error}"),
                }
            }
        })?;
    Ok(HandoffListener {
        path,
        identity: (metadata.dev(), metadata.ino()),
        stop,
    })
}

impl Drop for HandoffListener {
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt;

        self.stop.store(true, Ordering::Release);
        // Wakes the listening thread so that it sees `stop`.
        let _wake = UnixStream::connect(&self.path);
        if let Ok(metadata) = std::fs::symlink_metadata(&self.path)
            && (metadata.dev(), metadata.ino()) == self.identity
        {
            let _removed = std::fs::remove_file(&self.path);
        }
    }
}

/// Reads one message, has it opened, and answers with what happened.
fn serve(mut stream: UnixStream, open: OpenAddresses) {
    let result = receive(&mut stream).and_then(|addresses| {
        log::info!(
            "openurl: a second launch handed over {} address(es)",
            addresses.len()
        );
        open(addresses).map_err(HandoffError::Refused)
    });
    let reply = match &result {
        Ok(count) => format!("ok {count}\n"),
        // The second launch says "it did not open them" itself.
        Err(HandoffError::Refused(reason)) => format!("error {reason}\n"),
        Err(error) => format!("error {error}\n"),
    };
    if let Err(error) = &result {
        log::warn!("openurl: a second launch's addresses were not opened: {error}");
    }
    let _ = stream.write_all(reply.as_bytes());
    let _ = stream.shutdown(std::net::Shutdown::Write);
    // A sender of someone else's is not waited for.
    if !matches!(result, Err(HandoffError::UnsafeSocket(_))) {
        drain(&mut stream);
    }
}

/// Reads and drops what the sender still has in flight, within bounds. On Linux, closing a
/// socket with unread bytes resets the connection, and the sender then loses the reply that was
/// already written (a message over the size limit stops being read at the limit).
fn drain(stream: &mut UnixStream) {
    let deadline = std::time::Instant::now() + DRAIN_TIMEOUT;
    let mut remaining = MAX_DRAIN_BYTES;
    let mut buffer = [0_u8; 8192];
    while remaining > 0 && std::time::Instant::now() < deadline {
        let wanted = buffer.len().min(remaining);
        match stream.read(&mut buffer[..wanted]) {
            Ok(0) | Err(_) => break,
            Ok(read) => remaining -= read,
        }
    }
}

fn receive(stream: &mut UnixStream) -> Result<Vec<String>, HandoffError> {
    let io_error = |error: io::Error| HandoffError::Malformed(error.to_string());
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(io_error)?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(io_error)?;
    // SAFETY: `getuid` has no preconditions.
    peer_is_allowed(peer_user_id(stream), unsafe { libc::getuid() })?;
    let mut message = Vec::new();
    (&mut *stream)
        .take(MAX_MESSAGE_BYTES as u64 + 1)
        .read_to_end(&mut message)
        .map_err(io_error)?;
    decode(&message)
}

/// The second launch's side: sends the command line's addresses as typed. Returns how many tabs
/// the running browser opened.
pub(crate) fn hand_over(profile_root: &Path, addresses: &[String]) -> Result<usize, HandoffError> {
    if addresses.is_empty() {
        return Err(HandoffError::NothingToOpen);
    }
    if let Some(bad) = addresses
        .iter()
        .find(|address| !is_acceptable_address(address))
    {
        return Err(HandoffError::Malformed(format!(
            "'{}' is blank, too long or has control characters",
            bad.escape_debug()
        )));
    }
    let message = encode(addresses);
    if message.len() > MAX_MESSAGE_BYTES || addresses.len() > MAX_ADDRESSES {
        return Err(HandoffError::TooLarge);
    }
    send(&socket_path(profile_root), &message)
}

fn send(path: &Path, message: &[u8]) -> Result<usize, HandoffError> {
    let mut stream = connect(path)?;
    let failed = |error: io::Error| HandoffError::NotListening(error.to_string());
    stream
        .set_read_timeout(Some(REPLY_TIMEOUT))
        .map_err(failed)?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).map_err(failed)?;
    // The browser may answer and close before the whole message is written (it refuses what it
    // cannot accept without reading on), so a failed write or a reset is only held against it
    // when no complete answer arrived.
    let written = stream
        .write_all(message)
        .and_then(|()| stream.shutdown(std::net::Shutdown::Write));
    let mut reply = Vec::new();
    let read = (&mut stream).take(MAX_REPLY_BYTES).read_to_end(&mut reply);
    if !reply.ends_with(b"\n") {
        written.map_err(failed)?;
        read.map_err(failed)?;
    }
    let reply = String::from_utf8_lossy(&reply);
    let reply = reply.trim_end();
    reply
        .strip_prefix("ok ")
        .and_then(|count| count.parse().ok())
        .ok_or_else(|| {
            HandoffError::Refused(reply.strip_prefix("error ").unwrap_or(reply).to_owned())
        })
}

fn connect(path: &Path) -> Result<UnixStream, HandoffError> {
    let mut last_error = None;
    for attempt in 0..CONNECT_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(CONNECT_RETRY_DELAY);
        }
        match check_socket(path) {
            Ok(()) => {}
            Err(HandoffError::NotListening(reason)) => {
                last_error = Some(HandoffError::NotListening(reason));
                continue;
            }
            Err(error) => return Err(error),
        }
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                ) =>
            {
                last_error = Some(HandoffError::NotListening(error.to_string()));
            }
            Err(error) => return Err(HandoffError::NotListening(error.to_string())),
        }
    }
    Err(last_error.unwrap_or_else(|| HandoffError::NotListening("no answer".to_owned())))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    use super::*;

    /// A fresh private directory; short, because socket paths are limited to 104 bytes.
    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("mho-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("the scratch directory is created");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .expect("the scratch directory is made private");
        directory
    }

    fn list(addresses: &[&str]) -> Vec<String> {
        addresses
            .iter()
            .map(|address| (*address).to_owned())
            .collect()
    }

    /// What the test listeners were given, per test (each test uses its own marker address).
    static RECEIVED: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());

    fn received_with(marker: &str) -> Vec<Vec<String>> {
        RECEIVED
            .lock()
            .expect("not poisoned")
            .iter()
            .filter(|addresses| addresses.iter().any(|address| address.contains(marker)))
            .cloned()
            .collect()
    }

    /// Opens everything, like a browser whose address bar accepts it all.
    fn open_all(addresses: Vec<String>) -> Result<usize, String> {
        let count = addresses.len();
        RECEIVED.lock().expect("not poisoned").push(addresses);
        Ok(count)
    }

    /// Opens nothing, like a browser that cannot resolve an address.
    fn open_none(addresses: Vec<String>) -> Result<usize, String> {
        RECEIVED.lock().expect("not poisoned").push(addresses);
        Err("'mailto:a@b.test': unsupported URL scheme: mailto".to_owned())
    }

    #[test]
    fn a_list_survives_encoding() {
        let addresses = list(&[
            "https://a.test/",
            "file:///tmp/x.rs",
            "a.test",
            "two words searched",
        ]);
        assert_eq!(decode(&encode(&addresses)), Ok(addresses));
    }

    #[test]
    fn malformed_messages_are_refused_whole() {
        for message in [
            "",
            "mercury-open-addresses 1\n\n",
            "mercury-open-addresses 2\nhttps://a.test/\n\n",
            "mercury-open-urls 1\nhttps://a.test/\n\n",
            "hello\nhttps://a.test/\n\n",
            "mercury-open-addresses 1\nhttps://a.test/\n",
            "mercury-open-addresses 1\nhttps://a.test/\n\nextra",
            "mercury-open-addresses 1\nhttps://a.test/\n\n\n",
            "mercury-open-addresses 1\n   \n\n",
            "mercury-open-addresses 1\na\u{7}b\n\n",
            "mercury-open-addresses 1\r\nhttps://a.test/\r\n\r\n",
        ] {
            assert!(
                matches!(decode(message.as_bytes()), Err(HandoffError::Malformed(_))),
                "{message:?}"
            );
        }
        let mut not_utf8 = b"mercury-open-addresses 1\nhttps://a.test/".to_vec();
        not_utf8.extend([0xff, b'\n', b'\n']);
        assert!(matches!(decode(&not_utf8), Err(HandoffError::Malformed(_))));
    }

    #[test]
    fn oversized_messages_are_refused() {
        let many = (0..=MAX_ADDRESSES)
            .map(|index| format!("https://a.test/{index}"))
            .collect::<Vec<_>>();
        assert_eq!(decode(&encode(&many)), Err(HandoffError::TooLarge));
        let long = vec![format!("https://a.test/{}", "a".repeat(MAX_MESSAGE_BYTES))];
        assert_eq!(decode(&encode(&long)), Err(HandoffError::TooLarge));
        let one_long = vec![format!("https://a.test/{}", "a".repeat(MAX_ADDRESS_BYTES))];
        assert!(matches!(
            decode(&encode(&one_long)),
            Err(HandoffError::Malformed(_))
        ));
    }

    #[test]
    fn a_second_launch_hands_over_what_it_was_given_and_hears_the_tab_count() {
        let directory = scratch("ok");
        let listener = listen(&directory, open_all).expect("the listener starts");

        let opened = hand_over(&directory, &list(&["file:///tmp/x.rs", "ok-marker.test"]));

        assert_eq!(opened, Ok(2));
        assert_eq!(
            received_with("ok-marker"),
            vec![list(&["file:///tmp/x.rs", "ok-marker.test"])]
        );
        let mode = std::fs::symlink_metadata(socket_path(&directory))
            .expect("the socket exists")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        drop(listener);
        assert!(!socket_path(&directory).exists(), "the socket is removed");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_second_launch_is_told_when_nothing_was_opened() {
        let directory = scratch("none-opened");
        let _listener = listen(&directory, open_none).expect("the listener starts");

        let result = hand_over(
            &directory,
            &list(&["mailto:a@b.test", "refused-marker.test"]),
        );

        assert!(
            matches!(&result, Err(HandoffError::Refused(reason)) if reason.contains("mailto")),
            "{result:?}"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_listener_answers_a_malformed_message_with_an_error_and_opens_nothing() {
        let directory = scratch("bad");
        let _listener = listen(&directory, open_all).expect("the listener starts");

        let result = send(
            &socket_path(&directory),
            b"mercury-open-addresses 1\nbad\x07-marker\n\n",
        );
        assert!(
            matches!(result, Err(HandoffError::Refused(_))),
            "{result:?}"
        );
        let oversized = vec![b'a'; MAX_MESSAGE_BYTES + 10];
        let result = send(&socket_path(&directory), &oversized);
        assert!(
            matches!(result, Err(HandoffError::Refused(_))),
            "{result:?}"
        );
        assert!(received_with("bad").is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_oversized_message_is_still_answered_every_time() {
        let directory = scratch("big");
        let _listener = listen(&directory, open_all).expect("the listener starts");
        let mut oversized = b"mercury-open-addresses 1\n".to_vec();
        oversized.extend(vec![b'a'; MAX_MESSAGE_BYTES * 3]);

        for round in 0..20 {
            let result = send(&socket_path(&directory), &oversized);
            assert!(
                matches!(&result, Err(HandoffError::Refused(reason)) if reason.contains("bytes")),
                "round {round}: {result:?}"
            );
        }
        assert!(received_with("aaaa").is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_answer_that_was_sent_before_the_connection_was_reset_is_heard() {
        let directory = scratch("reset");
        let path = socket_path(&directory);
        let server = UnixListener::bind(&path).expect("a socket is bound");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("the socket is made private");
        // Answers after the first byte and closes with the rest of the message unread: on Linux
        // that resets the connection.
        let answering = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().expect("a sender connects");
            let mut first = [0_u8; 1];
            stream.read_exact(&mut first).expect("a byte arrives");
            stream
                .write_all(b"error too big\n")
                .expect("the answer is written");
        });
        let mut message = b"mercury-open-addresses 1\n".to_vec();
        message.extend(vec![b'a'; MAX_MESSAGE_BYTES * 3]);

        let result = send(&path, &message);

        answering.join().expect("the server thread ends");
        assert_eq!(result, Err(HandoffError::Refused("too big".to_owned())));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn only_this_users_processes_may_hand_over() {
        assert_eq!(peer_is_allowed(Ok(501), 501), Ok(()));
        assert!(matches!(
            peer_is_allowed(Ok(0), 501),
            Err(HandoffError::UnsafeSocket(reason)) if reason.contains("user 0")
        ));
        assert!(matches!(
            peer_is_allowed(Err(io::Error::other("no credentials")), 501),
            Err(HandoffError::UnsafeSocket(_))
        ));
    }

    #[test]
    fn a_stale_socket_is_replaced() {
        let directory = scratch("stale");
        drop(UnixListener::bind(socket_path(&directory)).expect("a socket is bound"));
        assert!(
            socket_path(&directory).exists(),
            "the dead socket stays behind"
        );

        let _listener = listen(&directory, open_all).expect("the stale socket is replaced");

        assert_eq!(hand_over(&directory, &list(&["stale-marker.test"])), Ok(1));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn something_else_at_the_socket_path_is_left_alone() {
        let directory = scratch("file");
        std::fs::write(socket_path(&directory), "not a socket").expect("a file is written");

        assert!(listen(&directory, open_all).is_err());
        assert_eq!(
            std::fs::read_to_string(socket_path(&directory)).expect("the file is still there"),
            "not a socket"
        );
        assert!(matches!(
            hand_over(&directory, &list(&["https://a.test/"])),
            Err(HandoffError::UnsafeSocket(_))
        ));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_socket_others_may_use_is_not_trusted() {
        let directory = scratch("perm");
        let _listener = listen(&directory, open_all).expect("the listener starts");
        std::fs::set_permissions(
            socket_path(&directory),
            std::fs::Permissions::from_mode(0o666),
        )
        .expect("the socket's mode is changed");

        assert!(matches!(
            hand_over(&directory, &list(&["perm-marker.test"])),
            Err(HandoffError::UnsafeSocket(_))
        ));
        assert!(received_with("perm-marker").is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_symbolic_link_is_not_followed() {
        let directory = scratch("link");
        let target = directory.join("elsewhere.sock");
        let _real = UnixListener::bind(&target).expect("a socket is bound");
        std::os::unix::fs::symlink(&target, socket_path(&directory)).expect("a link is made");

        assert!(matches!(
            hand_over(&directory, &list(&["https://a.test/"])),
            Err(HandoffError::UnsafeSocket(_))
        ));
        assert!(listen(&directory, open_all).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn nobody_listening_is_reported_after_retrying() {
        let directory = scratch("nobody");

        assert!(matches!(
            hand_over(&directory, &list(&["https://a.test/"])),
            Err(HandoffError::NotListening(_))
        ));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_command_line_without_usable_addresses_sends_nothing() {
        let directory = scratch("empty");

        assert_eq!(hand_over(&directory, &[]), Err(HandoffError::NothingToOpen));
        assert!(matches!(
            hand_over(&directory, &list(&["a\nb"])),
            Err(HandoffError::Malformed(_))
        ));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
