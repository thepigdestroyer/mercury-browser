// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Downloading one list: HTTPS only (or a loopback server named by a diagnostic option),
//! conditional requests, a size cap and time limits, cancellable at any moment.

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use tokio::sync::watch;
use url::Url;

use crate::cache::bounded;
use crate::list_text::MAX_LIST_BYTES;

/// Time limits for one download.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    pub connect: Duration,
    /// For the whole request, body included.
    pub total: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(15),
            total: Duration::from_secs(120),
        }
    }
}

/// Where lists are fetched from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sources {
    /// Each list's publisher URL from the manifest. Only `https` is fetched.
    Publishers,
    /// Diagnostics and tests: `<base><file name>` on a loopback server (`127.0.0.1`, `[::1]`
    /// or `localhost`), over `http` or `https`. Nothing else is contacted.
    LocalServer(Url),
}

impl Sources {
    /// A local server, if `base` is a loopback `http`/`https` URL.
    pub fn local_server(base: &Url) -> Option<Self> {
        is_loopback(base).then(|| Self::LocalServer(base.clone()))
    }

    /// The URL a list is fetched from.
    pub fn url_for(&self, source_url: &Url, file: &str) -> Option<Url> {
        let url = match self {
            Self::Publishers => source_url.clone(),
            Self::LocalServer(base) => base.join(file).ok()?,
        };
        self.allows(&url).then_some(url)
    }

    /// Whether a request (or a redirect) may go to `url`.
    pub fn allows(&self, url: &Url) -> bool {
        let no_credentials = url.username().is_empty() && url.password().is_none();
        no_credentials
            && match self {
                Self::Publishers => url.scheme() == "https" && url.host().is_some(),
                Self::LocalServer(_) => is_loopback(url),
            }
    }
}

fn is_loopback(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && match url.host() {
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
            None => false,
        }
}

/// The validators of the copy we have, sent so an unchanged list costs no download.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// What the server answered.
#[derive(Debug, PartialEq, Eq)]
pub enum Fetched {
    /// 304: the copy we have is current.
    NotModified,
    /// 200 with the list.
    Body {
        bytes: Vec<u8>,
        validators: Validators,
    },
}

/// Why a download failed.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    #[error("{0} is not an address lists may be fetched from")]
    NotAllowed(String),
    #[error("the server answered with status {0}")]
    Status(u16),
    #[error("the list is larger than {MAX_LIST_BYTES} bytes")]
    TooLarge,
    #[error("the download took too long")]
    TimedOut,
    #[error("network error: {0}")]
    Network(String),
    #[error("cancelled")]
    Cancelled,
}

/// Stops the downloads of the fetchers it was handed to.
#[derive(Clone, Debug)]
pub struct Canceller(watch::Sender<bool>);

impl Canceller {
    pub fn new() -> Self {
        Self(watch::channel(false).0)
    }

    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    fn watch(&self) -> watch::Receiver<bool> {
        self.0.subscribe()
    }
}

impl Default for Canceller {
    fn default() -> Self {
        Self::new()
    }
}

/// Downloads lists one at a time on a small runtime of its own.
pub struct Fetcher {
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
    sources: Sources,
    timeouts: Timeouts,
    canceller: Canceller,
}

impl Fetcher {
    pub fn new(
        sources: Sources,
        timeouts: Timeouts,
        canceller: Canceller,
    ) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let redirect_sources = sources.clone();
        let client = {
            let _context = runtime.enter();
            reqwest::Client::builder()
                .connect_timeout(timeouts.connect)
                .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                    if attempt.previous().len() >= 5 || !redirect_sources.allows(attempt.url()) {
                        attempt.stop()
                    } else {
                        attempt.follow()
                    }
                }))
                .referer(false)
                // Servo and the downloads make their own connections without a proxy.
                .no_proxy()
                .user_agent(concat!(
                    "MercuryBrowser/",
                    env!("CARGO_PKG_VERSION"),
                    " (filter-list updater)"
                ))
                .build()
                .map_err(std::io::Error::other)?
        };
        Ok(Self {
            runtime,
            client,
            sources,
            timeouts,
            canceller,
        })
    }

    pub fn sources(&self) -> &Sources {
        &self.sources
    }

    /// Fetches `url`, conditionally on `validators`.
    pub fn fetch(&self, url: &Url, validators: &Validators) -> Result<Fetched, FetchError> {
        if !self.sources.allows(url) {
            return Err(FetchError::NotAllowed(url.to_string()));
        }
        let mut cancelled = self.canceller.watch();
        if *cancelled.borrow() {
            return Err(FetchError::Cancelled);
        }
        self.runtime.block_on(async {
            tokio::select! {
                result = tokio::time::timeout(self.timeouts.total, self.download(url, validators)) => {
                    result.unwrap_or(Err(FetchError::TimedOut))
                }
                _ = cancelled.wait_for(|cancelled| *cancelled) => Err(FetchError::Cancelled),
            }
        })
    }

    async fn download(&self, url: &Url, validators: &Validators) -> Result<Fetched, FetchError> {
        let mut request = self.client.get(url.clone());
        if let Some(etag) = &validators.etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = &validators.last_modified {
            request = request.header(IF_MODIFIED_SINCE, last_modified);
        }
        let mut response = request.send().await.map_err(network_error)?;
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(Fetched::NotModified);
        }
        if status != StatusCode::OK {
            return Err(FetchError::Status(status.as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_LIST_BYTES as u64)
        {
            return Err(FetchError::TooLarge);
        }
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|value: &reqwest::header::HeaderValue| value.to_str().ok())
                .map(bounded)
        };
        let validators = Validators {
            etag: header(ETAG),
            last_modified: header(LAST_MODIFIED),
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            if bytes.len() + chunk.len() > MAX_LIST_BYTES {
                return Err(FetchError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Fetched::Body { bytes, validators })
    }
}

fn network_error(error: reqwest::Error) -> FetchError {
    if error.is_timeout() {
        FetchError::TimedOut
    } else {
        FetchError::Network(bounded(&error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{Reply, TestServer};

    fn fetcher(server: &TestServer) -> Fetcher {
        fetcher_with(server, Timeouts::default(), Canceller::new())
    }

    fn fetcher_with(server: &TestServer, timeouts: Timeouts, canceller: Canceller) -> Fetcher {
        let sources = Sources::local_server(&server.url("/lists/")).expect("loopback");
        Fetcher::new(sources, timeouts, canceller).expect("a fetcher")
    }

    #[test]
    fn publishers_are_fetched_over_https_only() {
        let sources = Sources::Publishers;
        let https = Url::parse("https://easylist.to/easylist/easylist.txt").unwrap();
        assert_eq!(sources.url_for(&https, "easylist.txt"), Some(https.clone()));
        assert!(sources.allows(&https));
        let http = Url::parse("http://easylist.to/easylist/easylist.txt").unwrap();
        assert_eq!(sources.url_for(&http, "easylist.txt"), None);
        assert!(!sources.allows(&http));
        assert!(!sources.allows(&Url::parse("file:///etc/hosts").unwrap()));
    }

    #[test]
    fn a_local_server_must_be_loopback_and_is_the_only_host_contacted() {
        for base in [
            "http://127.0.0.1:8080/lists/",
            "http://[::1]:8080/",
            "http://localhost:9/",
        ] {
            assert!(
                Sources::local_server(&Url::parse(base).unwrap()).is_some(),
                "{base}"
            );
        }
        for base in [
            "http://192.168.1.2/",
            "http://example.org/",
            "https://easylist.to/",
            "file:///tmp/",
        ] {
            assert!(
                Sources::local_server(&Url::parse(base).unwrap()).is_none(),
                "{base}"
            );
        }
        let local =
            Sources::local_server(&Url::parse("http://127.0.0.1:8080/lists/").unwrap()).unwrap();
        let publisher = Url::parse("https://easylist.to/easylist/easylist.txt").unwrap();
        assert_eq!(
            local.url_for(&publisher, "easylist.txt").unwrap().as_str(),
            "http://127.0.0.1:8080/lists/easylist.txt"
        );
        assert!(!local.allows(&publisher));
    }

    #[test]
    fn a_list_is_downloaded_with_its_validators() {
        let server = TestServer::start(|_| {
            Reply::ok("||ads.example^\n")
                .header("ETag", "\"v2\"")
                .header("Last-Modified", "Tue, 17 Nov 2026 08:00:00 GMT")
        });

        let fetched = fetcher(&server)
            .fetch(&server.url("/lists/a.txt"), &Validators::default())
            .expect("fetched");

        assert_eq!(
            fetched,
            Fetched::Body {
                bytes: b"||ads.example^\n".to_vec(),
                validators: Validators {
                    etag: Some("\"v2\"".to_owned()),
                    last_modified: Some("Tue, 17 Nov 2026 08:00:00 GMT".to_owned()),
                },
            }
        );
        let request = &server.requests()[0];
        assert_eq!(request.path, "/lists/a.txt");
        assert!(!request.headers.contains_key("if-none-match"));
    }

    #[test]
    fn requests_are_conditional_and_not_modified_is_reported() {
        let server = TestServer::start(|request| {
            if request.headers.get("if-none-match").map(String::as_str) == Some("\"v2\"") {
                Reply::status(304)
            } else {
                Reply::ok("changed")
            }
        });
        let validators = Validators {
            etag: Some("\"v2\"".to_owned()),
            last_modified: Some("Tue, 17 Nov 2026 08:00:00 GMT".to_owned()),
        };

        let fetched = fetcher(&server).fetch(&server.url("/lists/a.txt"), &validators);

        assert_eq!(fetched, Ok(Fetched::NotModified));
        let request = &server.requests()[0];
        assert_eq!(
            request.headers.get("if-modified-since").map(String::as_str),
            Some("Tue, 17 Nov 2026 08:00:00 GMT")
        );
    }

    #[test]
    fn error_statuses_fail() {
        let server = TestServer::start(|_| Reply::status(404));
        assert_eq!(
            fetcher(&server).fetch(&server.url("/lists/a.txt"), &Validators::default()),
            Err(FetchError::Status(404))
        );
    }

    #[test]
    fn a_body_over_the_cap_fails_whether_announced_or_not() {
        let announced = TestServer::start(|_| {
            Reply::ok("x").header("Content-Length", &(MAX_LIST_BYTES + 1).to_string())
        });
        assert_eq!(
            fetcher(&announced).fetch(&announced.url("/lists/a.txt"), &Validators::default()),
            Err(FetchError::TooLarge)
        );
        // No Content-Length: the body runs until the server closes the connection.
        let streamed = TestServer::start(|_| {
            Reply::ok(vec![b'!'; MAX_LIST_BYTES + 1]).header("X-Length", "none")
        });
        assert_eq!(
            fetcher(&streamed).fetch(&streamed.url("/lists/a.txt"), &Validators::default()),
            Err(FetchError::TooLarge)
        );
    }

    #[test]
    fn a_slow_server_times_out() {
        let server = TestServer::start(|_| Reply::ok("late").delayed(Duration::from_secs(3)));
        let timeouts = Timeouts {
            connect: Duration::from_secs(1),
            total: Duration::from_millis(200),
        };

        let result = fetcher_with(&server, timeouts, Canceller::new())
            .fetch(&server.url("/lists/a.txt"), &Validators::default());

        assert_eq!(result, Err(FetchError::TimedOut));
    }

    #[test]
    fn a_download_is_cancelled_at_once() {
        let server = TestServer::start(|_| Reply::ok("late").delayed(Duration::from_secs(3)));
        let canceller = Canceller::new();
        let fetcher = fetcher_with(&server, Timeouts::default(), canceller.clone());
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            canceller.cancel();
        });

        let result = fetcher.fetch(&server.url("/lists/a.txt"), &Validators::default());

        assert_eq!(result, Err(FetchError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn redirects_away_from_allowed_hosts_are_not_followed() {
        let server = TestServer::start(|request| {
            if request.path == "/lists/a.txt" {
                Reply::status(302).header("Location", "https://easylist.to/easylist/easylist.txt")
            } else {
                Reply::ok("unexpected")
            }
        });

        let result = fetcher(&server).fetch(&server.url("/lists/a.txt"), &Validators::default());

        assert_eq!(result, Err(FetchError::Status(302)));
        assert_eq!(server.requests().len(), 1);
    }

    #[test]
    fn redirects_within_allowed_hosts_are_followed() {
        let server = TestServer::start(|request| {
            if request.path == "/lists/a.txt" {
                Reply::status(301).header("Location", "/lists/b.txt")
            } else {
                Reply::ok("moved")
            }
        });

        let result = fetcher(&server).fetch(&server.url("/lists/a.txt"), &Validators::default());

        assert!(matches!(result, Ok(Fetched::Body { bytes, .. }) if bytes == b"moved"));
    }

    #[test]
    fn a_url_that_is_not_allowed_is_never_requested() {
        let server = TestServer::start(|_| Reply::ok("x"));
        let publisher = Url::parse("https://easylist.to/easylist/easylist.txt").unwrap();

        let result = fetcher(&server).fetch(&publisher, &Validators::default());

        assert!(matches!(result, Err(FetchError::NotAllowed(_))));
    }
}
