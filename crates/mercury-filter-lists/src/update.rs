// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! One update check: fetch every list, keep what passes its checks, record the outcome.
//!
//! The updater is generic over the kind of list: a [`ListKind`] says how many usable rules a
//! text holds, which is what the shrink rule compares. The ad and tracker lists use
//! [`NetworkRules`]; another kind of list with a URL, a licence and its own measure (an open
//! malware list, say) can reuse the same fetching, checking and caching.

use crate::cache::{Cache, CacheState, CachedList};
use crate::engine::ListInUse;
use crate::fetch::{FetchError, Fetched, Fetcher, Validators};
use crate::list_text::{Rejection, check_shrink, check_text, count_network_rules, header_version};

/// How the updater measures one kind of list.
pub trait ListKind {
    /// The rules in `text` that would be used. A download with fewer than half of the copy in
    /// use, or of the bundled copy, is refused.
    fn usable_rules(&self, text: &str) -> usize;
}

/// The ad and tracker lists: network rules as adblock-rust parses them.
pub struct NetworkRules;

impl ListKind for NetworkRules {
    fn usable_rules(&self, text: &str) -> usize {
        count_network_rules(text)
    }
}

/// The usable rules of each bundled list, by id: a download must also keep at least half of
/// these, so that a list cannot shrink by half on every update.
pub type Baselines = std::collections::BTreeMap<String, usize>;

/// What happened to one list.
#[derive(Debug, PartialEq, Eq)]
pub enum ListOutcome {
    /// The publisher has nothing newer than the copy in use.
    Unchanged,
    /// A new copy was stored; it differs from the one in use.
    Updated,
    /// A download arrived but was not used.
    Rejected(Rejection),
    /// No download.
    Failed(FetchError),
}

/// The outcome of a check.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CheckReport {
    /// By list id, in manifest order.
    pub outcomes: Vec<(String, ListOutcome)>,
    /// Whether the copies to use changed, so the engine must be built again.
    pub changed: bool,
    /// The check was cancelled (the browser is quitting) before it reached every list.
    pub cancelled: bool,
}

impl CheckReport {
    /// Whether every list reached its publisher: each was answered (a list, "not modified", or a
    /// client error such as 404) or refused after download. False after a network error, a
    /// time-out or a server error (5xx), when the updater retries sooner (see `schedule`).
    pub fn reached_publishers(&self) -> bool {
        !self.outcomes.iter().any(|(_, outcome)| match outcome {
            ListOutcome::Failed(FetchError::Network(_) | FetchError::TimedOut) => true,
            ListOutcome::Failed(FetchError::Status(status)) => *status >= 500,
            _ => false,
        })
    }

    /// A short description of what went wrong, for the settings window; `None` if every list
    /// was reached and accepted.
    pub fn problem(&self, lists: &[ListInUse]) -> Option<String> {
        let problems = self
            .outcomes
            .iter()
            .filter_map(|(id, outcome)| {
                let reason = match outcome {
                    ListOutcome::Unchanged | ListOutcome::Updated => return None,
                    ListOutcome::Rejected(rejection) => format!("not used: {rejection}"),
                    ListOutcome::Failed(error) => error.to_string(),
                };
                let name = lists
                    .iter()
                    .find(|list| list.entry.id == *id)
                    .map_or(id.as_str(), |list| list.entry.name.as_str());
                Some(format!("{name}: {reason}"))
            })
            .collect::<Vec<_>>();
        (!problems.is_empty()).then(|| problems.join("; "))
    }
}

/// Fetches every list in `lists` (the copies in use), stores accepted downloads in `cache`
/// and records them in `state` (`now` is the time in seconds since the Unix epoch). The caller
/// saves the state and, when `changed`, chooses the lists again and rebuilds the engine.
pub fn check_lists(
    fetcher: &Fetcher,
    kind: &dyn ListKind,
    baselines: &Baselines,
    lists: &[ListInUse],
    cache: &Cache,
    state: &mut CacheState,
    now: u64,
) -> CheckReport {
    let mut report = CheckReport::default();
    for list in lists {
        let baseline = baselines.get(&list.entry.id).copied().unwrap_or(0);
        let outcome = check_one(fetcher, kind, list, baseline, cache, state, now);
        if outcome == ListOutcome::Failed(FetchError::Cancelled) {
            report.cancelled = true;
            break;
        }
        report.changed |= outcome == ListOutcome::Updated;
        report.outcomes.push((list.entry.id.clone(), outcome));
    }
    report
}

fn check_one(
    fetcher: &Fetcher,
    kind: &dyn ListKind,
    list: &ListInUse,
    baseline: usize,
    cache: &Cache,
    state: &mut CacheState,
    now: u64,
) -> ListOutcome {
    let entry = &list.entry;
    let Some(url) = fetcher.sources().url_for(&entry.source_url, &entry.file) else {
        return ListOutcome::Failed(FetchError::NotAllowed(entry.source_url.to_string()));
    };
    // Validators only for an intact copy of the same source: a "not modified" must refer to a
    // file we can still read.
    let cached = state
        .lists
        .get(&entry.id)
        .filter(|cached| cached.source_url == entry.source_url.as_str())
        .filter(|cached| cache.read_list(cached).is_some())
        .cloned();
    let validators = cached
        .as_ref()
        .map_or_else(Validators::default, |cached| Validators {
            etag: cached.etag.clone(),
            last_modified: cached.last_modified.clone(),
        });
    match fetcher.fetch(&url, &validators) {
        Err(error) => ListOutcome::Failed(error),
        Ok(Fetched::NotModified) => {
            let Some(mut cached) = cached else {
                // Never asked conditionally, so a 304 is the server's mistake.
                return ListOutcome::Failed(FetchError::Status(304));
            };
            let was_in_use = cached.sha256 == list.sha256;
            cached.current_as_of = now;
            state.lists.insert(entry.id.clone(), cached);
            if was_in_use {
                ListOutcome::Unchanged
            } else {
                // The cached copy is current again and newer than the bundled one in use.
                ListOutcome::Updated
            }
        }
        Ok(Fetched::Body { bytes, validators }) => {
            let text = match check_text(&bytes) {
                Ok(text) => text,
                Err(rejection) => return ListOutcome::Rejected(rejection),
            };
            let got = kind.usable_rules(&text);
            let current = kind.usable_rules(&list.text).max(baseline);
            if let Err(rejection) = check_shrink(got, current) {
                return ListOutcome::Rejected(rejection);
            }
            let (file, sha256) = match cache.write_list(&entry.id, &text) {
                Ok(written) => written,
                Err(error) => {
                    return ListOutcome::Failed(FetchError::Network(format!(
                        "could not store it: {error}"
                    )));
                }
            };
            let changed = sha256 != list.sha256;
            state.lists.insert(
                entry.id.clone(),
                CachedList {
                    file,
                    sha256,
                    source_url: entry.source_url.to_string(),
                    etag: validators.etag,
                    last_modified: validators.last_modified,
                    version: header_version(&text),
                    current_as_of: now,
                },
            );
            if changed {
                ListOutcome::Updated
            } else {
                ListOutcome::Unchanged
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::tests::{SNAPSHOT, blocks, bundle};
    use crate::engine::{Origin, choose_lists, engine_for};
    use crate::fetch::{Canceller, Sources, Timeouts};
    use crate::test_server::{Reply, TestServer};
    use std::sync::{Arc, Mutex};

    const ADS: &str = "||ads.example^\n||ads2.example^\n";
    const TRACKERS: &str = "||trackers.example^\n";

    struct Setup {
        directory: tempfile::TempDir,
        manifest: crate::manifest::Manifest,
        cache: Cache,
    }

    impl Setup {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let manifest = bundle(
                &directory.path().join("bundled"),
                &[("ads", ADS), ("trackers", TRACKERS)],
            );
            let cache = Cache::open(&directory.path().join("cache")).unwrap();
            Self {
                directory,
                manifest,
                cache,
            }
        }

        fn lists(&self, state: &CacheState) -> Vec<ListInUse> {
            choose_lists(
                &self.manifest,
                &self.directory.path().join("bundled"),
                Some((&self.cache, state)),
            )
        }
    }

    /// Serves `/lists/<id>.txt` from a table the test can change.
    fn server(files: &Arc<Mutex<Vec<(String, Reply)>>>) -> TestServer {
        let files = Arc::clone(files);
        TestServer::start(move |request| {
            let files = files.lock().unwrap();
            match files.iter().find(|(path, _)| *path == request.path) {
                Some((_, reply)) => Reply {
                    status: reply.status,
                    headers: reply.headers.clone(),
                    body: reply.body.clone(),
                    delay: reply.delay,
                },
                None => Reply::status(404),
            }
        })
    }

    fn fetcher(server: &TestServer) -> Fetcher {
        Fetcher::new(
            Sources::local_server(&server.url("/lists/")).unwrap(),
            Timeouts::default(),
            Canceller::new(),
        )
        .unwrap()
    }

    fn files(entries: Vec<(&str, Reply)>) -> Arc<Mutex<Vec<(String, Reply)>>> {
        Arc::new(Mutex::new(
            entries
                .into_iter()
                .map(|(path, reply)| (path.to_owned(), reply))
                .collect(),
        ))
    }

    #[test]
    fn a_changed_list_is_stored_and_used_after_the_check() {
        let setup = Setup::new();
        let changed = format!("! Version: 202611170800\n{ADS}||new-ad.example^\n");
        let files = files(vec![
            (
                "/lists/ads.txt",
                Reply::ok(changed.clone()).header("ETag", "\"v3\""),
            ),
            ("/lists/trackers.txt", Reply::ok(TRACKERS)),
        ]);
        let server = server(&files);
        let mut state = CacheState::default();
        let lists = setup.lists(&state);

        let report = check_lists(
            &fetcher(&server),
            &NetworkRules,
            &Baselines::new(),
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 100,
        );

        assert!(report.changed);
        assert_eq!(report.outcomes[0], ("ads".to_owned(), ListOutcome::Updated));
        assert_eq!(report.problem(&lists), None);
        let cached = &state.lists["ads"];
        assert_eq!(cached.etag.as_deref(), Some("\"v3\""));
        assert_eq!(cached.version.as_deref(), Some("202611170800"));
        assert_eq!(cached.source_url, "https://lists.example/ads.txt");
        let lists = setup.lists(&state);
        assert_eq!(lists[0].origin, Origin::Updated);
        let engine = engine_for(&lists, None);
        assert!(blocks(&engine, "https://new-ad.example/x.js"));
    }

    #[test]
    fn an_identical_download_does_not_ask_for_a_rebuild() {
        let setup = Setup::new();
        let files = files(vec![
            ("/lists/ads.txt", Reply::ok(ADS)),
            ("/lists/trackers.txt", Reply::ok(TRACKERS)),
        ]);
        let server = server(&files);
        let mut state = CacheState::default();
        let lists = setup.lists(&state);

        let report = check_lists(
            &fetcher(&server),
            &NetworkRules,
            &Baselines::new(),
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 100,
        );

        assert!(!report.changed);
        assert_eq!(report.outcomes[0].1, ListOutcome::Unchanged);
    }

    #[test]
    fn the_next_check_is_conditional_and_not_modified_keeps_the_copy() {
        let setup = Setup::new();
        let changed = format!("{ADS}||new-ad.example^\n");
        let files = files(vec![
            (
                "/lists/ads.txt",
                Reply::ok(changed).header("ETag", "\"v3\""),
            ),
            ("/lists/trackers.txt", Reply::ok(TRACKERS)),
        ]);
        let server = server(&files);
        let mut state = CacheState::default();
        let fetcher = fetcher(&server);
        let lists = setup.lists(&state);
        check_lists(
            &fetcher,
            &NetworkRules,
            &Baselines::new(),
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 100,
        );
        files.lock().unwrap()[0].1 = Reply::status(304);
        let lists = setup.lists(&state);

        let report = check_lists(
            &fetcher,
            &NetworkRules,
            &Baselines::new(),
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 200,
        );

        assert!(!report.changed);
        assert_eq!(report.outcomes[0].1, ListOutcome::Unchanged);
        assert_eq!(state.lists["ads"].current_as_of, SNAPSHOT + 200);
        let requests = server.requests();
        let last_ads = requests
            .iter()
            .rev()
            .find(|r| r.path == "/lists/ads.txt")
            .unwrap();
        assert_eq!(
            last_ads.headers.get("if-none-match").map(String::as_str),
            Some("\"v3\"")
        );
    }

    #[test]
    fn a_truncated_download_is_refused_and_the_copy_in_use_stays() {
        let setup = Setup::new();
        let files = files(vec![
            (
                "/lists/ads.txt",
                Reply::ok("||ads.example^\n".repeat(0) + "! cut\n"),
            ),
            ("/lists/trackers.txt", Reply::ok(TRACKERS)),
        ]);
        let server = server(&files);
        let mut state = CacheState::default();
        let lists = setup.lists(&state);

        let report = check_lists(
            &fetcher(&server),
            &NetworkRules,
            &Baselines::new(),
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 100,
        );

        assert!(!report.changed);
        assert_eq!(
            report.outcomes[0].1,
            ListOutcome::Rejected(Rejection::TooFewRules { got: 0, current: 2 })
        );
        assert!(!state.lists.contains_key("ads"));
        assert!(report.problem(&lists).unwrap().contains("List ads"));
        assert_eq!(setup.lists(&state)[0].origin, Origin::Bundled);
    }

    #[test]
    fn a_failed_list_does_not_stop_the_others() {
        let setup = Setup::new();
        let files = files(vec![(
            "/lists/trackers.txt",
            Reply::ok("||trackers.example^\n||t2.example^\n"),
        )]);
        let server = server(&files);
        let mut state = CacheState::default();
        let lists = setup.lists(&state);

        let report = check_lists(
            &fetcher(&server),
            &NetworkRules,
            &Baselines::new(),
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 100,
        );

        assert_eq!(
            report.outcomes[0].1,
            ListOutcome::Failed(FetchError::Status(404))
        );
        assert_eq!(report.outcomes[1].1, ListOutcome::Updated);
        assert!(report.changed);
        assert!(report.problem(&lists).unwrap().contains("404"));
    }

    #[test]
    fn a_damaged_cached_copy_is_fetched_whole_again() {
        let setup = Setup::new();
        let changed = format!("{ADS}||new-ad.example^\n");
        let files = files(vec![
            (
                "/lists/ads.txt",
                Reply::ok(changed).header("ETag", "\"v3\""),
            ),
            ("/lists/trackers.txt", Reply::ok(TRACKERS)),
        ]);
        let server = server(&files);
        let fetcher = fetcher(&server);
        let mut state = CacheState::default();
        let lists = setup.lists(&state);
        check_lists(
            &fetcher,
            &NetworkRules,
            &Baselines::new(),
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 100,
        );
        std::fs::write(
            setup.cache.directory().join(&state.lists["ads"].file),
            "dam",
        )
        .unwrap();
        let lists = setup.lists(&state);
        assert_eq!(lists[0].origin, Origin::Bundled);

        let report = check_lists(
            &fetcher,
            &NetworkRules,
            &Baselines::new(),
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 200,
        );

        let requests = server.requests();
        let last_ads = requests
            .iter()
            .rev()
            .find(|r| r.path == "/lists/ads.txt")
            .unwrap();
        assert!(!last_ads.headers.contains_key("if-none-match"));
        assert_eq!(report.outcomes[0].1, ListOutcome::Updated);
        assert_eq!(setup.lists(&state)[0].origin, Origin::Updated);
    }

    #[test]
    fn another_kind_of_list_brings_its_own_measure() {
        struct Lines;
        impl ListKind for Lines {
            fn usable_rules(&self, text: &str) -> usize {
                text.lines().filter(|line| !line.starts_with('#')).count()
            }
        }
        let setup = Setup::new();
        let files = files(vec![
            (
                "/lists/ads.txt",
                Reply::ok("# hosts\nads.example\nads2.example\n"),
            ),
            ("/lists/trackers.txt", Reply::ok("t.example\n")),
        ]);
        let server = server(&files);
        let mut state = CacheState::default();
        let lists = setup.lists(&state);

        let report = check_lists(
            &fetcher(&server),
            &Lines,
            &Baselines::new(),
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 100,
        );

        assert_eq!(report.outcomes[0].1, ListOutcome::Updated);
    }

    #[test]
    fn network_errors_time_outs_and_server_errors_did_not_reach_the_publishers() {
        let report = |outcome| CheckReport {
            outcomes: vec![
                ("a".to_owned(), ListOutcome::Unchanged),
                ("b".to_owned(), outcome),
            ],
            ..CheckReport::default()
        };
        for transient in [
            ListOutcome::Failed(FetchError::Network("refused".to_owned())),
            ListOutcome::Failed(FetchError::TimedOut),
            ListOutcome::Failed(FetchError::Status(503)),
        ] {
            assert!(!report(transient).reached_publishers());
        }
        for answered in [
            ListOutcome::Updated,
            ListOutcome::Unchanged,
            ListOutcome::Failed(FetchError::Status(404)),
            ListOutcome::Failed(FetchError::TooLarge),
            ListOutcome::Rejected(Rejection::LooksLikeHtml),
        ] {
            assert!(report(answered).reached_publishers());
        }
    }

    #[test]
    fn a_download_must_keep_half_the_bundled_rules_too() {
        let setup = Setup::new();
        // The copy in use is an update with one rule; the bundled copy has four (the baselines).
        let mut state = CacheState::default();
        let (file, sha256) = setup.cache.write_list("ads", "||ads.example^\n").unwrap();
        state.lists.insert(
            "ads".to_owned(),
            CachedList {
                file,
                sha256,
                source_url: "https://lists.example/ads.txt".to_owned(),
                etag: None,
                last_modified: None,
                version: None,
                current_as_of: SNAPSHOT + 50,
            },
        );
        let files = files(vec![
            ("/lists/ads.txt", Reply::ok("! empty\n")),
            ("/lists/trackers.txt", Reply::ok(TRACKERS)),
        ]);
        let server = server(&files);
        let lists = setup.lists(&state);
        assert_eq!(lists[0].origin, Origin::Updated);
        let baselines = Baselines::from([("ads".to_owned(), 4), ("trackers".to_owned(), 1)]);

        let report = check_lists(
            &fetcher(&server),
            &NetworkRules,
            &baselines,
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 100,
        );

        assert_eq!(
            report.outcomes[0].1,
            ListOutcome::Rejected(Rejection::TooFewRules { got: 0, current: 4 })
        );

        // One rule against a copy in use of one and a bundled copy of two passes; against a
        // bundled copy of four it does not.
        files.lock().unwrap()[0].1 = Reply::ok("||y.example^\n");
        let lists = setup.lists(&state);
        let report = check_lists(
            &fetcher(&server),
            &NetworkRules,
            &baselines,
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 200,
        );
        assert_eq!(
            report.outcomes[0].1,
            ListOutcome::Rejected(Rejection::TooFewRules { got: 1, current: 4 })
        );
        let lenient = Baselines::from([("ads".to_owned(), 2)]);
        let report = check_lists(
            &fetcher(&server),
            &NetworkRules,
            &lenient,
            &lists,
            &setup.cache,
            &mut state,
            SNAPSHOT + 300,
        );
        assert_eq!(report.outcomes[0].1, ListOutcome::Updated);
    }
}
