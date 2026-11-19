// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The filter-list service: one background thread that builds the engine at start-up and
//! updates the lists on schedule or on request.
//!
//! It never touches the UI. It reports through a callback, on its own thread: the compiled
//! engine whenever the copies in use change (the first time right after start), and its
//! status after every step.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::cache::{Cache, CacheState};
use crate::engine::{ListInUse, Origin, choose_lists, engine_for, read_manifest};
use crate::fetch::{Canceller, Fetcher, Sources, Timeouts};
use crate::schedule::wait_before_check;
use crate::update::{NetworkRules, check_lists};

/// Whether and how lists are updated in this run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Updates {
    /// No updates; the reason is shown in the settings window.
    Off(String),
    On {
        sources: Sources,
        /// Check on the schedule (the "Update filter lists automatically" setting).
        automatic: bool,
        /// How long after start-up the first automatic check waits.
        settle_delay: Duration,
        /// Check once as soon as the engine is built, whatever the schedule (diagnostics).
        check_at_start: bool,
    },
}

/// Where the lists are and what the service may do.
#[derive(Clone, Debug)]
pub struct ServiceConfig {
    /// The directory holding `manifest.json` and the bundled lists; `None` if none was found.
    pub bundled_dir: Option<PathBuf>,
    /// The profile's cache directory; `None` keeps nothing (and allows no updates).
    pub cache_dir: Option<PathBuf>,
    pub updates: Updates,
    pub timeouts: Timeouts,
}

/// What the service reports.
#[derive(Debug)]
pub enum ServiceEvent {
    /// A serialized engine for the copies now in use (adblock-rust =0.13.5).
    Engine(Vec<u8>),
    Status(Status),
}

/// The service's state, for the settings window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The copies in use, in manifest order.
    pub lists: Vec<ListStatus>,
    /// When the last check started / last reached every list, in seconds since the epoch.
    pub last_check: Option<u64>,
    pub last_success: Option<u64>,
    /// What went wrong in the last check.
    pub last_error: Option<String>,
    /// A check is running.
    pub checking: bool,
    /// Why updates are not possible in this run, if they are not.
    pub updates_unavailable: Option<String>,
    /// Why there is no engine, if there is none.
    pub problem: Option<String>,
}

/// One list as the settings window shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListStatus {
    pub name: String,
    pub version: Option<String>,
    /// When the copy was taken (bundled) or last confirmed current (updated).
    pub date: u64,
    pub updated: bool,
    /// The host the list is fetched from (the publisher's, not a diagnostic server's).
    pub host: String,
}

enum Command {
    CheckNow,
    SetAutomatic(bool),
}

/// How the worker builds its downloader (a runtime and an HTTP client, which can fail, for
/// example when the process is out of file descriptors). A seam so that a test can make it fail.
type MakeFetcher = Box<dyn Fn(Sources, Timeouts, Canceller) -> std::io::Result<Fetcher> + Send>;

/// The running service. Dropping it without [`shut_down`](Self::shut_down) stops it too, but
/// without waiting.
pub struct FilterListService {
    commands: Option<Sender<Command>>,
    canceller: Canceller,
    thread: Option<JoinHandle<()>>,
}

impl FilterListService {
    /// Starts the thread; `on_event` is called on it.
    pub fn start(
        config: ServiceConfig,
        on_event: impl Fn(ServiceEvent) + Send + 'static,
    ) -> std::io::Result<Self> {
        Self::start_with(config, on_event, Box::new(Fetcher::new))
    }

    fn start_with(
        config: ServiceConfig,
        on_event: impl Fn(ServiceEvent) + Send + 'static,
        make_fetcher: MakeFetcher,
    ) -> std::io::Result<Self> {
        let (commands, receiver) = mpsc::channel();
        let canceller = Canceller::new();
        let worker = Worker {
            config,
            commands: receiver,
            canceller: canceller.clone(),
            on_event: Box::new(on_event),
            make_fetcher,
            started: Instant::now(),
            status: Status::default(),
        };
        let thread = std::thread::Builder::new()
            .name("filter-lists".to_owned())
            .spawn(move || worker.run())?;
        Ok(Self {
            commands: Some(commands),
            canceller,
            thread: Some(thread),
        })
    }

    /// Checks for updates now, if updates are possible in this run ("Update now").
    pub fn check_now(&self) {
        self.send(Command::CheckNow);
    }

    /// Turns the schedule on or off.
    pub fn set_automatic(&self, automatic: bool) {
        self.send(Command::SetAutomatic(automatic));
    }

    fn send(&self, command: Command) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(command);
        }
    }

    /// Cancels any download and waits for the thread until `deadline`. Returns whether it
    /// ended. Every file it writes is replaced atomically, so one it leaves running is safe.
    pub fn shut_down(&mut self, deadline: Instant) -> bool {
        self.canceller.cancel();
        self.commands.take();
        let Some(thread) = self.thread.take() else {
            return true;
        };
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if thread.is_finished() {
            if thread.join().is_err() {
                log::error!("filter lists: the updater thread panicked");
            }
            true
        } else {
            log::warn!("filter lists: the updater did not stop in time");
            false
        }
    }
}

impl Drop for FilterListService {
    fn drop(&mut self) {
        self.canceller.cancel();
    }
}

struct Worker {
    config: ServiceConfig,
    commands: Receiver<Command>,
    canceller: Canceller,
    on_event: Box<dyn Fn(ServiceEvent) + Send>,
    make_fetcher: MakeFetcher,
    started: Instant,
    status: Status,
}

/// What the worker has once the engine is built.
struct Loaded {
    bundled_dir: PathBuf,
    manifest: crate::manifest::Manifest,
    cache: Option<Cache>,
    state: CacheState,
    lists: Vec<ListInUse>,
    fetcher: Option<Fetcher>,
    /// The usable rules of each bundled list, for the shrink rule.
    baselines: crate::update::Baselines,
}

/// The usable rules of each bundled list that can be read.
fn bundled_baselines(
    manifest: &crate::manifest::Manifest,
    bundled_dir: &std::path::Path,
) -> crate::update::Baselines {
    use crate::update::ListKind as _;
    choose_lists(manifest, bundled_dir, None)
        .iter()
        .map(|list| (list.entry.id.clone(), NetworkRules.usable_rules(&list.text)))
        .collect()
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

impl Worker {
    fn run(mut self) {
        if let Updates::Off(reason) = &self.config.updates {
            self.status.updates_unavailable = Some(reason.clone());
        } else if self.config.cache_dir.is_none() {
            self.status.updates_unavailable =
                Some("Updates need a profile to keep the lists in.".to_owned());
        }
        let Some(mut loaded) = self.load() else {
            self.emit_status();
            // Nothing to update without a manifest; wait to be shut down.
            while self.commands.recv().is_ok() {}
            return;
        };
        let (mut automatic, check_at_start, settle_delay) = match &self.config.updates {
            Updates::On {
                automatic,
                check_at_start,
                settle_delay,
                ..
            } => (*automatic, *check_at_start, *settle_delay),
            Updates::Off(_) => (false, false, Duration::ZERO),
        };
        if check_at_start {
            self.check(&mut loaded);
        }
        loop {
            let wait = (automatic && self.updates_possible()).then(|| {
                wait_before_check(
                    loaded.state.last_check,
                    loaded.state.consecutive_failures,
                    now_seconds(),
                    self.started.elapsed(),
                    settle_delay,
                )
            });
            let command = match wait {
                Some(wait) => self.commands.recv_timeout(wait),
                None => self
                    .commands
                    .recv()
                    .map_err(|_| RecvTimeoutError::Disconnected),
            };
            match command {
                Ok(Command::CheckNow) => {
                    self.check(&mut loaded);
                    // "Update now" pressed during that check asks for nothing more.
                    automatic = self.drain_check_requests(automatic);
                }
                Ok(Command::SetAutomatic(on)) => automatic = on,
                Err(RecvTimeoutError::Timeout) => {
                    let due = wait_before_check(
                        loaded.state.last_check,
                        loaded.state.consecutive_failures,
                        now_seconds(),
                        self.started.elapsed(),
                        settle_delay,
                    );
                    if due.is_zero() {
                        // Whatever `check` does, it must leave the next check not due (it
                        // records the check, or ends the loop); otherwise this would spin.
                        self.check(&mut loaded);
                        automatic = self.drain_check_requests(automatic);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if self.canceller.is_cancelled() {
                break;
            }
        }
    }

    /// Drops "check now" requests that queued up during a check, keeping the last switch
    /// setting among them.
    fn drain_check_requests(&self, mut automatic: bool) -> bool {
        while let Ok(command) = self.commands.try_recv() {
            if let Command::SetAutomatic(on) = command {
                automatic = on;
            }
        }
        automatic
    }

    fn updates_possible(&self) -> bool {
        self.status.updates_unavailable.is_none()
    }

    fn emit_status(&self) {
        (self.on_event)(ServiceEvent::Status(self.status.clone()));
    }

    /// Builds the first engine: from the cache's compiled copy if it matches, otherwise
    /// compiled now.
    fn load(&mut self) -> Option<Loaded> {
        let Some(bundled_dir) = self.config.bundled_dir.clone() else {
            self.fail("Mercury Browser's filter lists were not found next to the application.");
            return None;
        };
        let manifest = match read_manifest(&bundled_dir) {
            Ok(manifest) => manifest,
            Err(error) => {
                self.fail(&error.to_string());
                return None;
            }
        };
        let cache = self.config.cache_dir.as_ref().and_then(|directory| {
            Cache::open(directory)
                .inspect_err(|error| {
                    log::warn!(
                        "filter lists: the cache at {} cannot be used: {error}",
                        directory.display()
                    );
                })
                .ok()
        });
        if cache.is_none() && self.status.updates_unavailable.is_none() {
            self.status.updates_unavailable =
                Some("Updates need a profile to keep the lists in.".to_owned());
        }
        let state = cache
            .as_ref()
            .map_or_else(CacheState::default, Cache::load_state);
        let mut loaded = Loaded {
            bundled_dir,
            manifest,
            cache,
            state,
            lists: Vec::new(),
            fetcher: None,
            baselines: crate::update::Baselines::new(),
        };
        if !self.rebuild(&mut loaded) {
            self.fail("None of the filter lists could be read.");
            return None;
        }
        self.status.last_check = loaded.state.last_check;
        self.status.last_success = loaded.state.last_success;
        self.status.last_error = loaded.state.last_error.clone();
        self.emit_status();
        Some(loaded)
    }

    fn fail(&mut self, problem: &str) {
        log::error!("filter lists: {problem} Ad and tracker blocking stays off.");
        self.status.problem = Some(problem.to_owned());
    }

    /// Chooses the copies, builds or reads the engine, reports it, saves the state and sweeps
    /// the cache. Returns false if no list could be read.
    fn rebuild(&mut self, loaded: &mut Loaded) -> bool {
        let lists = choose_lists(
            &loaded.manifest,
            &loaded.bundled_dir,
            loaded.cache.as_ref().map(|cache| (cache, &loaded.state)),
        );
        if lists.is_empty() {
            return false;
        }
        let started = Instant::now();
        let bytes = engine_for(
            &lists,
            loaded
                .cache
                .as_ref()
                .map(|cache| (cache, &mut loaded.state)),
        );
        log::info!(
            "filter lists: engine ready from {} lists ({} updated) in {} ms",
            lists.len(),
            lists
                .iter()
                .filter(|list| list.origin == Origin::Updated)
                .count(),
            started.elapsed().as_millis()
        );
        (self.on_event)(ServiceEvent::Engine(bytes));
        if let Some(cache) = &loaded.cache {
            if let Err(error) = cache.save_state(&loaded.state) {
                log::warn!("filter lists: could not save the cache state: {error}");
            }
            cache.sweep(&loaded.state);
        }
        self.status.lists = lists
            .iter()
            .map(|list| ListStatus {
                name: list.entry.name.clone(),
                version: list.version.clone(),
                date: list.date,
                updated: list.origin == Origin::Updated,
                host: list
                    .entry
                    .source_url
                    .host_str()
                    .unwrap_or_default()
                    .to_owned(),
            })
            .collect();
        loaded.lists = lists;
        true
    }

    /// Records a check that failed before it began, because the downloader could not be built,
    /// like one that reached no publisher: it is counted as a failure, so the next automatic
    /// check waits out the back-off, and the settings window says what went wrong.
    fn record_failed_start(&mut self, loaded: &mut Loaded, error: &std::io::Error) {
        loaded.state.last_check = Some(now_seconds());
        loaded.state.consecutive_failures = loaded.state.consecutive_failures.saturating_add(1);
        loaded.state.last_error = Some(crate::cache::bounded(&format!(
            "a problem starting the updater: {error}"
        )));
        if let Some(Err(error)) = loaded
            .cache
            .as_ref()
            .map(|cache| cache.save_state(&loaded.state))
        {
            log::warn!("filter lists: could not save the cache state: {error}");
        }
        self.status.last_check = loaded.state.last_check;
        self.status.last_error = loaded.state.last_error.clone();
        self.emit_status();
    }

    fn check(&mut self, loaded: &mut Loaded) {
        if !self.updates_possible() || self.canceller.is_cancelled() {
            return;
        }
        let (Some(cache), Updates::On { sources, .. }) = (&loaded.cache, &self.config.updates)
        else {
            return;
        };
        if loaded.fetcher.is_none() {
            match (self.make_fetcher)(
                sources.clone(),
                self.config.timeouts,
                self.canceller.clone(),
            ) {
                Ok(fetcher) => loaded.fetcher = Some(fetcher),
                Err(error) => {
                    log::warn!("filter lists: cannot start the updater: {error}");
                    // A check that could not start still counts as a failed one: without that,
                    // it would stay due, and the loop in `run` would retry it without pause.
                    self.record_failed_start(loaded, &error);
                    return;
                }
            }
        }
        let Some(fetcher) = &loaded.fetcher else {
            return;
        };
        if loaded.baselines.is_empty() {
            loaded.baselines = bundled_baselines(&loaded.manifest, &loaded.bundled_dir);
        }
        let now = now_seconds();
        let previous_check = loaded.state.last_check;
        loaded.state.last_check = Some(now);
        if let Err(error) = cache.save_state(&loaded.state) {
            log::warn!("filter lists: could not save the cache state: {error}");
        }
        self.status.checking = true;
        self.status.last_check = Some(now);
        self.emit_status();
        log::info!(
            "filter lists: checking {} lists for updates",
            loaded.lists.len()
        );

        let report = check_lists(
            fetcher,
            &NetworkRules,
            &loaded.baselines,
            &loaded.lists,
            cache,
            &mut loaded.state,
            now,
        );
        self.status.checking = false;
        if report.cancelled {
            // Not a check: the next one is due as if this one had never started.
            log::info!("filter lists: the update check was cancelled");
            loaded.state.last_check = previous_check;
            self.status.last_check = previous_check;
            let _ = cache.save_state(&loaded.state);
            return;
        }
        loaded.state.consecutive_failures = if report.reached_publishers() {
            0
        } else {
            loaded.state.consecutive_failures.saturating_add(1)
        };
        match report.problem(&loaded.lists) {
            None => {
                loaded.state.last_success = Some(now_seconds());
                loaded.state.last_error = None;
            }
            Some(problem) => {
                log::warn!("filter lists: {problem}");
                loaded.state.last_error = Some(crate::cache::bounded(&problem));
            }
        }
        self.status.last_success = loaded.state.last_success;
        self.status.last_error = loaded.state.last_error.clone();
        let updated = report
            .outcomes
            .iter()
            .filter(|(_, outcome)| *outcome == crate::update::ListOutcome::Updated)
            .count();
        log::info!("filter lists: update check done, {updated} list(s) changed");
        if report.changed {
            if !self.rebuild(loaded) {
                log::error!("filter lists: no list could be read after the update");
            }
        } else if let Err(error) = cache.save_state(&loaded.state) {
            log::warn!("filter lists: could not save the cache state: {error}");
        } else {
            // Dates of lists confirmed current changed.
            self.status.lists = choose_lists(
                &loaded.manifest,
                &loaded.bundled_dir,
                Some((cache, &loaded.state)),
            )
            .iter()
            .map(|list| ListStatus {
                name: list.entry.name.clone(),
                version: list.version.clone(),
                date: list.date,
                updated: list.origin == Origin::Updated,
                host: list
                    .entry
                    .source_url
                    .host_str()
                    .unwrap_or_default()
                    .to_owned(),
            })
            .collect();
        }
        self.emit_status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::tests::{SNAPSHOT, blocks, bundle};
    use crate::test_server::{Reply, TestServer};
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const ADS: &str = "||ads.example^\n||ads2.example^\n";

    fn collect() -> (
        Arc<Mutex<Vec<ServiceEvent>>>,
        impl Fn(ServiceEvent) + Send + 'static,
    ) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        (events, move |event| sink.lock().unwrap().push(event))
    }

    fn engines(events: &Mutex<Vec<ServiceEvent>>) -> Vec<Vec<u8>> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                ServiceEvent::Engine(bytes) => Some(bytes.clone()),
                ServiceEvent::Status(_) => None,
            })
            .collect()
    }

    fn last_status(events: &Mutex<Vec<ServiceEvent>>) -> Option<Status> {
        events
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|event| match event {
                ServiceEvent::Status(status) => Some(status.clone()),
                ServiceEvent::Engine(_) => None,
            })
    }

    fn wait_until(what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn saved_state(cache: &Path) -> CacheState {
        Cache::open(cache).unwrap().load_state()
    }

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let bundled = directory.path().join("bundled");
        bundle(&bundled, &[("ads", ADS), ("unbreak", "! nothing\n")]);
        let cache = directory.path().join("profile/filter-lists");
        (directory, bundled, cache)
    }

    fn off(bundled: &Path, cache: &Path) -> ServiceConfig {
        ServiceConfig {
            bundled_dir: Some(bundled.to_path_buf()),
            cache_dir: Some(cache.to_path_buf()),
            updates: Updates::Off("automated run".to_owned()),
            timeouts: Timeouts::default(),
        }
    }

    fn local(server: &TestServer, bundled: &Path, cache: &Path) -> ServiceConfig {
        ServiceConfig {
            updates: Updates::On {
                sources: Sources::local_server(&server.url("/lists/")).unwrap(),
                automatic: false,
                settle_delay: Duration::ZERO,
                check_at_start: true,
            },
            ..off(bundled, cache)
        }
    }

    #[test]
    fn the_engine_is_built_from_the_bundled_files_and_reported_with_the_lists() {
        let (_directory, bundled, cache) = setup();
        let (events, sink) = collect();

        let mut service = FilterListService::start(off(&bundled, &cache), sink).unwrap();
        wait_until("the engine", || !engines(&events).is_empty());
        wait_until("the status", || {
            last_status(&events).is_some_and(|s| !s.lists.is_empty())
        });
        assert!(service.shut_down(Instant::now() + Duration::from_secs(5)));

        assert!(blocks(&engines(&events)[0], "https://ads.example/a.js"));
        let status = last_status(&events).unwrap();
        assert_eq!(status.lists[0].name, "List ads");
        assert!(!status.lists[0].updated);
        assert_eq!(status.lists[0].date, SNAPSHOT);
        assert_eq!(status.lists[0].host, "lists.example");
        assert_eq!(status.updates_unavailable.as_deref(), Some("automated run"));
        assert_eq!(status.problem, None);
        assert!(
            cache.join("state.json").exists(),
            "the compiled engine is cached"
        );
    }

    #[test]
    fn without_bundled_lists_there_is_no_engine_and_the_status_says_why() {
        let directory = tempfile::tempdir().unwrap();
        let (events, sink) = collect();
        let config = ServiceConfig {
            bundled_dir: Some(directory.path().join("missing")),
            cache_dir: None,
            updates: Updates::Off("x".to_owned()),
            timeouts: Timeouts::default(),
        };

        let mut service = FilterListService::start(config, sink).unwrap();
        wait_until("the status", || last_status(&events).is_some());
        assert!(service.shut_down(Instant::now() + Duration::from_secs(5)));

        assert!(engines(&events).is_empty());
        assert!(last_status(&events).unwrap().problem.is_some());
    }

    #[test]
    fn a_diagnostic_update_from_a_local_server_changes_the_engine_and_persists() {
        let (_directory, bundled, cache) = setup();
        let changed = format!("! Version: 2\n{ADS}||update-test.example^\n");
        let server = TestServer::start(move |request| match request.path.as_str() {
            "/lists/ads.txt" => Reply::ok(changed.clone()).header("ETag", "\"v2\""),
            "/lists/unbreak.txt" => Reply::ok("! nothing\n"),
            _ => Reply::status(404),
        });
        let (events, sink) = collect();

        let mut service = FilterListService::start(local(&server, &bundled, &cache), sink).unwrap();
        wait_until("the updated engine", || engines(&events).len() >= 2);
        wait_until("the check to end", || {
            last_status(&events)
                .is_some_and(|status| !status.checking && status.last_success.is_some())
        });
        assert!(service.shut_down(Instant::now() + Duration::from_secs(5)));

        let engines = engines(&events);
        assert!(!blocks(&engines[0], "https://update-test.example/x.js"));
        assert!(blocks(&engines[1], "https://update-test.example/x.js"));
        let status = last_status(&events).unwrap();
        assert!(status.lists[0].updated);
        assert_eq!(status.lists[0].version.as_deref(), Some("2"));
        assert_eq!(status.last_error, None);

        // The next run, with updates off, starts from the updated copy in the cache.
        let (events, sink) = collect();
        let mut service = FilterListService::start(off(&bundled, &cache), sink).unwrap();
        wait_until("the engine", || !self::engines(&events).is_empty());
        assert!(service.shut_down(Instant::now() + Duration::from_secs(5)));
        assert!(blocks(
            &self::engines(&events)[0],
            "https://update-test.example/x.js"
        ));
        assert_eq!(
            server.requests().len(),
            2,
            "the second run contacted nothing"
        );
    }

    #[test]
    fn a_failed_check_is_reported_and_keeps_the_engine() {
        let (_directory, bundled, cache) = setup();
        let server = TestServer::start(|_| Reply::status(503));
        let (events, sink) = collect();

        let mut service = FilterListService::start(local(&server, &bundled, &cache), sink).unwrap();
        wait_until("the check to end", || {
            last_status(&events)
                .is_some_and(|status| !status.checking && status.last_check.is_some())
        });
        assert!(service.shut_down(Instant::now() + Duration::from_secs(5)));

        let status = last_status(&events).unwrap();
        assert!(status.last_error.unwrap().contains("503"));
        assert_eq!(status.last_success, None);
        assert_eq!(engines(&events).len(), 1);
        // A server error reached no publisher: the next check backs off from one hour.
        assert_eq!(saved_state(&cache).consecutive_failures, 1);
    }

    #[test]
    fn update_now_does_nothing_when_updates_are_off() {
        let (_directory, bundled, cache) = setup();
        let (events, sink) = collect();

        let mut service = FilterListService::start(off(&bundled, &cache), sink).unwrap();
        wait_until("the engine", || !engines(&events).is_empty());
        service.check_now();
        std::thread::sleep(Duration::from_millis(200));
        assert!(service.shut_down(Instant::now() + Duration::from_secs(5)));

        assert_eq!(last_status(&events).unwrap().last_check, None);
    }

    #[test]
    fn shutting_down_cancels_a_download_in_flight() {
        let (_directory, bundled, cache) = setup();
        let server = TestServer::start(|_| Reply::ok(ADS).delayed(Duration::from_secs(10)));
        let (events, sink) = collect();
        let mut service = FilterListService::start(local(&server, &bundled, &cache), sink).unwrap();
        wait_until("the download to start", || !server.requests().is_empty());

        let started = Instant::now();
        assert!(service.shut_down(Instant::now() + Duration::from_secs(5)));

        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(engines(&events).len(), 1);
        // A cancelled check does not count as a check.
        assert_eq!(saved_state(&cache).last_check, None);
        assert_eq!(saved_state(&cache).consecutive_failures, 0);
    }

    #[test]
    fn a_successful_check_resets_the_failure_count() {
        let (_directory, bundled, cache) = setup();
        let directory = Cache::open(&cache).unwrap();
        directory
            .save_state(&CacheState {
                consecutive_failures: 3,
                ..CacheState::default()
            })
            .unwrap();
        let server = TestServer::start(|request| match request.path.as_str() {
            "/lists/ads.txt" => Reply::ok(ADS),
            _ => Reply::ok("! nothing\n"),
        });
        let (events, sink) = collect();

        let mut service = FilterListService::start(local(&server, &bundled, &cache), sink).unwrap();
        wait_until("the check to end", || {
            last_status(&events).is_some_and(|status| status.last_success.is_some())
        });
        assert!(service.shut_down(Instant::now() + Duration::from_secs(5)));

        assert_eq!(saved_state(&cache).consecutive_failures, 0);
    }

    /// A downloader that cannot be built, and how many times it was tried.
    fn broken_fetcher() -> (Arc<AtomicUsize>, MakeFetcher) {
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempts);
        let make: MakeFetcher = Box::new(move |_, _, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            Err(std::io::Error::other("too many open files"))
        });
        (attempts, make)
    }

    #[test]
    fn a_downloader_that_cannot_be_built_is_a_failed_check_and_not_a_busy_loop() {
        let (_directory, bundled, cache) = setup();
        let (attempts, make_fetcher) = broken_fetcher();
        let (events, sink) = collect();
        // Automatic, settled, never checked: due as soon as the engine is built.
        let config = ServiceConfig {
            updates: Updates::On {
                sources: Sources::Publishers,
                automatic: true,
                settle_delay: Duration::ZERO,
                check_at_start: false,
            },
            ..off(&bundled, &cache)
        };

        let mut service = FilterListService::start_with(config, sink, make_fetcher).unwrap();
        wait_until("the first attempt", || attempts.load(Ordering::SeqCst) >= 1);
        // Long enough for a loop that retries at once to show.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "the next check waits out the back-off"
        );
        wait_until("the failure to be reported", || {
            last_status(&events).is_some_and(|status| status.last_error.is_some())
        });
        assert!(service.shut_down(Instant::now() + Duration::from_secs(5)));

        let status = last_status(&events).unwrap();
        assert!(status.last_check.is_some());
        assert_eq!(status.last_success, None);
        assert!(
            status
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("too many open files"))
        );
        assert_eq!(engines(&events).len(), 1, "the engine is kept");
        let state = saved_state(&cache);
        assert!(state.last_check.is_some());
        assert_eq!(state.consecutive_failures, 1);
        assert!(state.last_error.is_some());
        // So the next check is an hour away, not due.
        let wait = wait_before_check(
            state.last_check,
            state.consecutive_failures,
            now_seconds(),
            Duration::from_secs(600),
            Duration::ZERO,
        );
        assert!(wait > Duration::from_secs(30 * 60), "waits {wait:?}");
    }

    #[test]
    fn update_now_tries_a_downloader_that_could_not_be_built_again_and_says_so() {
        let (_directory, bundled, cache) = setup();
        let (attempts, make_fetcher) = broken_fetcher();
        let (events, sink) = collect();
        let config = ServiceConfig {
            updates: Updates::On {
                sources: Sources::Publishers,
                automatic: false,
                settle_delay: Duration::ZERO,
                check_at_start: true,
            },
            ..off(&bundled, &cache)
        };

        let mut service = FilterListService::start_with(config, sink, make_fetcher).unwrap();
        wait_until("the check at start", || {
            attempts.load(Ordering::SeqCst) == 1
        });
        service.check_now();
        wait_until("the second attempt", || {
            attempts.load(Ordering::SeqCst) == 2
        });
        wait_until("both failures to be saved", || {
            saved_state(&cache).consecutive_failures == 2
        });
        std::thread::sleep(Duration::from_millis(200));
        assert!(service.shut_down(Instant::now() + Duration::from_secs(5)));

        assert_eq!(attempts.load(Ordering::SeqCst), 2, "one try per request");
        assert!(!last_status(&events).unwrap().checking);
    }

    #[test]
    fn the_schedule_checks_when_due_and_only_while_automatic() {
        let (_directory, bundled, cache) = setup();
        let server =
            TestServer::start(|_| Reply::ok("! nothing\n||ads.example^\n||ads2.example^\n"));
        let (events, sink) = collect();
        let config = ServiceConfig {
            updates: Updates::On {
                sources: Sources::local_server(&server.url("/lists/")).unwrap(),
                automatic: false,
                settle_delay: Duration::ZERO,
                check_at_start: false,
            },
            ..off(&bundled, &cache)
        };

        let mut service = FilterListService::start(config, sink).unwrap();
        wait_until("the engine", || !engines(&events).is_empty());
        std::thread::sleep(Duration::from_millis(300));
        assert!(server.requests().is_empty(), "not automatic: no check");

        service.set_automatic(true);
        wait_until("the scheduled check", || {
            last_status(&events).is_some_and(|status| status.last_success.is_some())
        });
        assert!(service.shut_down(Instant::now() + Duration::from_secs(5)));
        assert_eq!(server.requests().len(), 2);
    }
}
