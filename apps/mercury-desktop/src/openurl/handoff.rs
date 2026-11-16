//! A second launch hands its URLs to the Mercury Browser already using the profile.
//!
//! The running browser listens on a Unix-domain socket in the profile directory (private to this
//! user, like the directory). A launch that finds the profile locked connects, sends its URLs and
//! exits 0 once they are accepted. On macOS a launch from Finder or another app never gets here
//! (LaunchServices hands the URL to the running app as an Apple event); this is for the command
//! line, and for Linux.
//!
//! The message is UTF-8 text: the line `mercury-open-urls 1`, then one URL per line, then an
//! empty line. Every URL must be absolute, `http`, `https` or `file`, and written exactly as the
//! URL parser writes it; at most `MAX_URLS`, and at most `MAX_MESSAGE_BYTES` in all. Anything
//! else is refused whole, and nothing is opened. The answer is `ok <count>` or `error <reason>`.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use url::Url;

/// The socket's name in the profile directory.
pub(crate) const SOCKET_NAME: &str = "urls.sock";
const HEADER: &str = "mercury-open-urls 1";
pub(crate) const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_URLS: usize = 64;
const MAX_REPLY_BYTES: u64 = 256;
/// How long either side waits for the other before giving up on a connection.
const IO_TIMEOUT: Duration = Duration::from_secs(3);
/// A browser that has just locked the profile may not be listening yet.
const CONNECT_ATTEMPTS: u32 = 10;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(200);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HandoffError {
    /// No address on the command line could be handed over.
    NothingToOpen,
    TooLarge,
    Malformed(String),
    /// The socket is not a private socket of this user; nothing was sent.
    UnsafeSocket(String),
    /// No browser answered on the socket.
    NotListening(String),
    /// The running browser refused the URLs.
    Refused(String),
}

impl std::fmt::Display for HandoffError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingToOpen => {
                formatter.write_str("no address on the command line can be opened")
            }
            Self::TooLarge => write!(
                formatter,
                "more than {MAX_URLS} URLs or {MAX_MESSAGE_BYTES} bytes"
            ),
            Self::Malformed(reason) => write!(formatter, "malformed message: {reason}"),
            Self::UnsafeSocket(reason) => {
                write!(formatter, "the socket is not safe to use: {reason}")
            }
            Self::NotListening(reason) => {
                write!(formatter, "the browser is not listening: {reason}")
            }
            Self::Refused(reason) => write!(formatter, "the browser refused them: {reason}"),
        }
    }
}

/// Whether the running browser accepts `url` from a second launch.
fn is_acceptable_url(line: &str) -> bool {
    Url::parse(line)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "file") && url.as_str() == line)
}

pub(crate) fn encode(urls: &[String]) -> Vec<u8> {
    let mut message = String::from(HEADER);
    message.push('\n');
    for url in urls {
        message.push_str(url);
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
    let urls = lines.map(str::to_owned).collect::<Vec<_>>();
    if urls.is_empty() {
        return Err(HandoffError::Malformed("no URL".to_owned()));
    }
    if urls.len() > MAX_URLS {
        return Err(HandoffError::TooLarge);
    }
    if let Some(bad) = urls.iter().position(|url| !is_acceptable_url(url)) {
        return Err(HandoffError::Malformed(format!(
            "line {} is not an http, https or file URL",
            bad + 2
        )));
    }
    Ok(urls)
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

/// Starts listening on the profile's socket. `deliver` runs on the listening thread with each
/// accepted list of URLs.
pub(crate) fn listen(
    profile_root: &Path,
    deliver: impl Fn(Vec<String>) + Send + 'static,
) -> io::Result<HandoffListener> {
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
                    Ok(stream) => serve(stream, &deliver),
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

fn serve(mut stream: UnixStream, deliver: &impl Fn(Vec<String>)) {
    let result = receive(&mut stream);
    let reply = match &result {
        Ok(urls) => format!("ok {}\n", urls.len()),
        Err(error) => format!("error {error}\n"),
    };
    let _ = stream.write_all(reply.as_bytes());
    match result {
        Ok(urls) => {
            log::info!("openurl: a second launch handed over {} URL(s)", urls.len());
            deliver(urls);
        }
        Err(error) => log::warn!("openurl: refused a second launch's message: {error}"),
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
    let own_uid = unsafe { libc::getuid() };
    match peer_user_id(stream) {
        Ok(uid) if uid == own_uid => {}
        Ok(uid) => {
            return Err(HandoffError::UnsafeSocket(format!(
                "the sender runs as user {uid}"
            )));
        }
        Err(error) => return Err(HandoffError::UnsafeSocket(error.to_string())),
    }
    let mut message = Vec::new();
    (&mut *stream)
        .take(MAX_MESSAGE_BYTES as u64 + 1)
        .read_to_end(&mut message)
        .map_err(io_error)?;
    decode(&message)
}

/// The second launch's side: resolves the command line's addresses as the first launch would,
/// and hands them over. Returns how many the running browser accepted.
pub(crate) fn hand_over(profile_root: &Path, addresses: &[String]) -> Result<usize, HandoffError> {
    let urls = addresses
        .iter()
        .filter_map(|address| match talaria_core::resolve_address(address) {
            Ok(url) if is_acceptable_url(url.as_str()) => Some(url.to_string()),
            Ok(url) => {
                eprintln!("mercury-browser: skipping '{address}': {url} cannot be handed over");
                None
            }
            Err(error) => {
                eprintln!("mercury-browser: skipping '{address}': {error}");
                None
            }
        })
        .collect::<Vec<_>>();
    if urls.is_empty() {
        return Err(HandoffError::NothingToOpen);
    }
    let message = encode(&urls);
    if message.len() > MAX_MESSAGE_BYTES || urls.len() > MAX_URLS {
        return Err(HandoffError::TooLarge);
    }
    send(&socket_path(profile_root), &message)
}

fn send(path: &Path, message: &[u8]) -> Result<usize, HandoffError> {
    let mut stream = connect(path)?;
    let failed = |error: io::Error| HandoffError::NotListening(error.to_string());
    stream.set_read_timeout(Some(IO_TIMEOUT)).map_err(failed)?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).map_err(failed)?;
    stream.write_all(message).map_err(failed)?;
    stream.shutdown(std::net::Shutdown::Write).map_err(failed)?;
    let mut reply = String::new();
    (&mut stream)
        .take(MAX_REPLY_BYTES)
        .read_to_string(&mut reply)
        .map_err(failed)?;
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
    use std::sync::mpsc;

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

    fn urls(list: &[&str]) -> Vec<String> {
        list.iter().map(|url| (*url).to_owned()).collect()
    }

    #[test]
    fn a_list_survives_encoding() {
        let list = urls(&[
            "https://a.test/",
            "file:///tmp/a%20b.html",
            "http://127.0.0.1:8/x?y#z",
        ]);
        assert_eq!(decode(&encode(&list)), Ok(list));
    }

    #[test]
    fn malformed_messages_are_refused_whole() {
        for message in [
            "",
            "mercury-open-urls 1\n\n",
            "mercury-open-urls 2\nhttps://a.test/\n\n",
            "hello\nhttps://a.test/\n\n",
            "mercury-open-urls 1\nhttps://a.test/\n",
            "mercury-open-urls 1\nhttps://a.test/\n\nextra",
            "mercury-open-urls 1\nhttps://a.test/\n\n\n",
            "mercury-open-urls 1\nhttps://a.test/\nmailto:a@b.test\n\n",
            "mercury-open-urls 1\njavascript:alert(1)\n\n",
            "mercury-open-urls 1\nhttps://A.test\n\n",
            "mercury-open-urls 1\na.test\n\n",
            "mercury-open-urls 1\r\nhttps://a.test/\r\n\r\n",
        ] {
            assert!(
                matches!(decode(message.as_bytes()), Err(HandoffError::Malformed(_))),
                "{message:?}"
            );
        }
        let mut not_utf8 = b"mercury-open-urls 1\nhttps://a.test/".to_vec();
        not_utf8.extend([0xff, b'\n', b'\n']);
        assert!(matches!(decode(&not_utf8), Err(HandoffError::Malformed(_))));
    }

    #[test]
    fn oversized_messages_are_refused() {
        let many = (0..=MAX_URLS)
            .map(|index| format!("https://a.test/{index}"))
            .collect::<Vec<_>>();
        assert_eq!(decode(&encode(&many)), Err(HandoffError::TooLarge));
        let long = vec![format!("https://a.test/{}", "a".repeat(MAX_MESSAGE_BYTES))];
        assert_eq!(decode(&encode(&long)), Err(HandoffError::TooLarge));
    }

    #[test]
    fn a_second_launch_hands_its_urls_to_the_listener() {
        let directory = scratch("ok");
        let (sender, receiver) = mpsc::channel();
        let listener = listen(&directory, move |urls| {
            let _ = sender.send(urls);
        })
        .expect("the listener starts");

        let accepted = hand_over(&directory, &urls(&["a.test", "https://b.test/page"]));

        assert_eq!(accepted, Ok(2));
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)),
            Ok(urls(&["https://a.test/", "https://b.test/page"]))
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
    fn the_listener_answers_a_malformed_message_with_an_error_and_opens_nothing() {
        let directory = scratch("bad");
        let (sender, receiver) = mpsc::channel();
        let _listener = listen(&directory, move |urls| {
            let _ = sender.send(urls);
        })
        .expect("the listener starts");

        let result = send(
            &socket_path(&directory),
            b"mercury-open-urls 1\nmailto:a@b.test\n\n",
        );

        assert!(
            matches!(result, Err(HandoffError::Refused(_))),
            "{result:?}"
        );
        assert!(receiver.recv_timeout(Duration::from_millis(300)).is_err());
        let oversized = vec![b'a'; MAX_MESSAGE_BYTES + 10];
        let result = send(&socket_path(&directory), &oversized);
        assert!(
            matches!(result, Err(HandoffError::Refused(_))),
            "{result:?}"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_stale_socket_is_replaced() {
        let directory = scratch("stale");
        drop(UnixListener::bind(socket_path(&directory)).expect("a socket is bound"));
        assert!(
            socket_path(&directory).exists(),
            "the dead socket stays behind"
        );

        let (sender, receiver) = mpsc::channel();
        let _listener = listen(&directory, move |urls| {
            let _ = sender.send(urls);
        })
        .expect("the stale socket is replaced");

        assert_eq!(hand_over(&directory, &urls(&["https://a.test/"])), Ok(1));
        assert!(receiver.recv_timeout(Duration::from_secs(5)).is_ok());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn something_else_at_the_socket_path_is_left_alone() {
        let directory = scratch("file");
        std::fs::write(socket_path(&directory), "not a socket").expect("a file is written");

        assert!(listen(&directory, |_| {}).is_err());
        assert_eq!(
            std::fs::read_to_string(socket_path(&directory)).expect("the file is still there"),
            "not a socket"
        );
        assert!(matches!(
            hand_over(&directory, &urls(&["https://a.test/"])),
            Err(HandoffError::UnsafeSocket(_))
        ));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_socket_others_may_use_is_not_trusted() {
        let directory = scratch("perm");
        let (sender, receiver) = mpsc::channel();
        let _listener = listen(&directory, move |urls| {
            let _ = sender.send(urls);
        })
        .expect("the listener starts");
        std::fs::set_permissions(
            socket_path(&directory),
            std::fs::Permissions::from_mode(0o666),
        )
        .expect("the socket's mode is changed");

        assert!(matches!(
            hand_over(&directory, &urls(&["https://a.test/"])),
            Err(HandoffError::UnsafeSocket(_))
        ));
        assert!(receiver.recv_timeout(Duration::from_millis(300)).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_symbolic_link_is_not_followed() {
        let directory = scratch("link");
        let target = directory.join("elsewhere.sock");
        let _real = UnixListener::bind(&target).expect("a socket is bound");
        std::os::unix::fs::symlink(&target, socket_path(&directory)).expect("a link is made");

        assert!(matches!(
            hand_over(&directory, &urls(&["https://a.test/"])),
            Err(HandoffError::UnsafeSocket(_))
        ));
        assert!(listen(&directory, |_| {}).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn nobody_listening_is_reported_after_retrying() {
        let directory = scratch("none");

        assert!(matches!(
            hand_over(&directory, &urls(&["https://a.test/"])),
            Err(HandoffError::NotListening(_))
        ));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn addresses_that_cannot_be_handed_over_are_skipped() {
        let directory = scratch("skip");

        assert_eq!(
            hand_over(&directory, &urls(&["mailto:a@b.test", "about:blank"])),
            Err(HandoffError::NothingToOpen)
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}
