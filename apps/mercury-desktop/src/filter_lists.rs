// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! blocklists: the filter-list service of this run. It builds the content blocker's engine off
//! the UI thread from the list files beside the executable (or their updated copies in the
//! profile), updates them in the background, and reports to the settings window.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use mercury_filter_lists::fetch::{Sources, Timeouts};
use mercury_filter_lists::schedule::SETTLE_DELAY;
use mercury_filter_lists::service::{
    FilterListService, ServiceConfig, ServiceEvent, Status, Updates,
};

use crate::cli::LaunchOptions;
use crate::profile::ActiveProfile;
use crate::settings::SettingsHandle;

/// The directory in the profile that holds updated copies and the compiled engine.
const CACHE_DIRECTORY_NAME: &str = "filter-lists";
/// The directory, beside the executable or in the bundle's resources, that holds the lists.
const BUNDLED_DIRECTORY_NAME: &str = "filter-lists";

const AUTOMATED_RUN: &str = "Screenshot and background runs do not update the filter lists.";

thread_local! {
    static SERVICE: RefCell<Option<FilterListService>> = const { RefCell::new(None) };
    static STATUS: RefCell<Status> = RefCell::new(Status::default());
}

/// Starts the service. Call once, after the engine has started and the settings are loaded.
pub(crate) fn start(
    options: &LaunchOptions,
    profile: &ActiveProfile,
    settings: &Rc<SettingsHandle>,
) {
    let mut slot = None;
    if !crate::engine::with_runtime(|runtime| slot = Some(runtime.filter_engine_slot())) {
        log::error!("filter lists: the engine is not running; ad and tracker blocking stays off");
        return;
    }
    let Some(slot) = slot else {
        return;
    };
    let bundled_dir = std::env::current_exe()
        .ok()
        .and_then(|executable| find_bundled_dir(&executable));
    if bundled_dir.is_none() {
        log::error!(
            "filter lists: no {BUNDLED_DIRECTORY_NAME} directory with a manifest was found"
        );
    }
    let config = ServiceConfig {
        bundled_dir,
        cache_dir: profile
            .paths
            .as_ref()
            .map(|paths| paths.root.join(CACHE_DIRECTORY_NAME)),
        updates: updates_for(options, settings.settings().privacy.update_filter_lists),
        timeouts: Timeouts::default(),
    };
    let started = FilterListService::start(config, move |event| match event {
        ServiceEvent::Engine(bytes) => slot.offer(bytes),
        ServiceEvent::Status(status) => {
            let _queued = slot_status(status);
        }
    });
    match started {
        Ok(service) => SERVICE.with(|current| *current.borrow_mut() = Some(service)),
        Err(error) => {
            log::error!(
                "filter lists: could not start ({error}); ad and tracker blocking stays off"
            );
            return;
        }
    }
    let settings_weak = Rc::downgrade(settings);
    settings.subscribe(move || {
        if let Some(settings) = settings_weak.upgrade() {
            let automatic = settings.settings().privacy.update_filter_lists;
            with_service(|service| service.set_automatic(automatic));
        }
    });
}

/// Hands a status from the service's thread to the UI thread.
fn slot_status(status: Status) -> Result<(), slint::EventLoopError> {
    slint::invoke_from_event_loop(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        STATUS.with(|current| *current.borrow_mut() = status);
        crate::settings::filter_lists_changed();
    })
}

fn with_service(work: impl FnOnce(&FilterListService)) {
    SERVICE.with(|current| {
        if let Some(service) = current.borrow().as_ref() {
            work(service);
        }
    });
}

/// "Update now" in the settings window.
pub(crate) fn update_now() {
    with_service(FilterListService::check_now);
}

/// The last status the service reported.
pub(crate) fn status() -> Status {
    STATUS.with(|current| current.borrow().clone())
}

/// At quit (a post-run slot): cancels any download and waits for the thread until `deadline`.
/// Uses no Slint state.
pub(crate) fn shut_down(deadline: Instant) {
    if let Some(mut service) = SERVICE.with(|current| current.borrow_mut().take()) {
        // A cancelled download ends at once; at worst a compile is finishing.
        service.shut_down(deadline.min(Instant::now() + SHUT_DOWN_WAIT));
    }
}

/// The longest the quit waits for the updater.
const SHUT_DOWN_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// The filter-list part of the settings window's privacy section.
pub(crate) fn fill_settings_window(window: &mercury_ui::SettingsWindow) {
    let view = settings_view(&status(), local_time);
    window.set_filter_lists_publishers(view.publishers.into());
    window.set_filter_lists_status(view.status.into());
    window.set_filter_lists(slint::ModelRc::new(slint::VecModel::from(
        view.lists
            .into_iter()
            .map(slint::SharedString::from)
            .collect::<Vec<_>>(),
    )));
    window.set_filter_lists_can_update(view.can_update);
}

/// How this run updates: from a loopback server when the diagnostic option names one (once,
/// at start-up); never in other automated runs; otherwise from the publishers, on the
/// schedule when the setting asks for it, and on "Update now".
pub(crate) fn updates_for(options: &LaunchOptions, automatic: bool) -> Updates {
    if let Some(server) = &options.filter_list_server
        && let Some(sources) = Sources::local_server(server)
    {
        return Updates::On {
            sources,
            automatic: false,
            settle_delay: std::time::Duration::ZERO,
            check_at_start: true,
        };
    }
    if options.is_automated() {
        return Updates::Off(AUTOMATED_RUN.to_owned());
    }
    Updates::On {
        sources: Sources::Publishers,
        automatic,
        settle_delay: SETTLE_DELAY,
        check_at_start: false,
    }
}

/// Where the bundled lists may be, for the executable at `executable`, in order: the macOS
/// bundle's `Contents/Resources/filter-lists`; `filter-lists` beside the executable (the Linux
/// archive, and `target/<profile>/` after `make build-release`); and
/// `../share/mercury-browser/filter-lists` (a Linux installation under a prefix).
pub(crate) fn bundled_dir_candidates(executable: &Path) -> Vec<PathBuf> {
    let Some(directory) = executable.parent() else {
        return Vec::new();
    };
    vec![
        directory.join("../Resources").join(BUNDLED_DIRECTORY_NAME),
        directory.join(BUNDLED_DIRECTORY_NAME),
        directory
            .join("../share/mercury-browser")
            .join(BUNDLED_DIRECTORY_NAME),
    ]
}

fn find_bundled_dir(executable: &Path) -> Option<PathBuf> {
    bundled_dir_candidates(executable)
        .into_iter()
        .find(|candidate| {
            candidate
                .join(mercury_filter_lists::MANIFEST_FILE)
                .is_file()
        })
}

/// What the settings window shows: a status line and one line per list.
pub(crate) struct SettingsView {
    /// Which hosts Mercury contacts for updates: they see the user's address.
    pub publishers: String,
    pub status: String,
    pub lists: Vec<String>,
    pub can_update: bool,
}

/// "Once a day, Mercury Browser asks <hosts> for newer versions of these lists.", naming each
/// host the lists are fetched from once; empty while no list is known.
fn publishers_line(status: &Status) -> String {
    let mut hosts: Vec<String> = Vec::new();
    for list in &status.lists {
        let host = match list.host.as_str() {
            "" => continue,
            "raw.githubusercontent.com" => "raw.githubusercontent.com (GitHub)".to_owned(),
            other => other.to_owned(),
        };
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    let named = match hosts.as_slice() {
        [] => return String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    format!("Once a day, Mercury Browser asks {named} for newer versions of these lists.")
}

pub(crate) fn settings_view(status: &Status, format_time: impl Fn(u64) -> String) -> SettingsView {
    let lists = status
        .lists
        .iter()
        .map(|list| {
            let version = list
                .version
                .as_ref()
                .map(|version| format!("version {version}, "))
                .unwrap_or_default();
            let origin = if list.updated {
                "updated"
            } else {
                "included with Mercury Browser"
            };
            format!(
                "{}: {version}from {} ({origin})",
                list.name,
                format_time(list.date)
            )
        })
        .collect();
    if let Some(problem) = &status.problem {
        return SettingsView {
            publishers: publishers_line(status),
            status: format!("Ad and tracker blocking is off: {problem}"),
            lists,
            can_update: false,
        };
    }
    let mut line = if status.checking {
        "Checking for updates…".to_owned()
    } else {
        let mut line = match status.last_success {
            Some(success) => format!("Last updated {}.", format_time(success)),
            None => "Not updated yet: using the lists included with Mercury Browser.".to_owned(),
        };
        if let (Some(error), Some(check)) = (&status.last_error, status.last_check) {
            line.push_str(&format!(
                " The last check, {}, failed for {error}.",
                format_time(check)
            ));
        }
        line
    };
    if let Some(reason) = &status.updates_unavailable {
        line.push(' ');
        line.push_str(reason);
    }
    SettingsView {
        publishers: publishers_line(status),
        status: line,
        lists,
        can_update: !status.checking && status.updates_unavailable.is_none(),
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `seconds` since the epoch as local time, for example `17 Nov 2026, 14:05` (UTC where the
/// local time zone is unknown).
pub(crate) fn local_time(seconds: u64) -> String {
    let (year, month, day, hour, minute) = local_parts(seconds);
    let month = MONTHS
        .get(usize::try_from(month.saturating_sub(1)).unwrap_or(0))
        .unwrap_or(&"Jan");
    format!("{day} {month} {year}, {hour:02}:{minute:02}")
}

#[cfg(unix)]
fn local_parts(seconds: u64) -> (u64, u64, u64, u64, u64) {
    let Ok(time) = libc::time_t::try_from(seconds) else {
        return utc_parts(seconds);
    };
    // SAFETY: `localtime_r` only reads `time` and writes the `tm` it is given, which lives on
    // this stack frame; it is the thread-safe form of `localtime`.
    let mut parts = unsafe { std::mem::zeroed::<libc::tm>() };
    let converted = unsafe { libc::localtime_r(&time, &mut parts) };
    if converted.is_null() {
        return utc_parts(seconds);
    }
    let field = |value: libc::c_int| u64::try_from(value).unwrap_or(0);
    (
        field(parts.tm_year) + 1900,
        field(parts.tm_mon) + 1,
        field(parts.tm_mday),
        field(parts.tm_hour),
        field(parts.tm_min),
    )
}

#[cfg(not(unix))]
fn local_parts(seconds: u64) -> (u64, u64, u64, u64, u64) {
    utc_parts(seconds)
}

fn utc_parts(seconds: u64) -> (u64, u64, u64, u64, u64) {
    let (year, month, day) = mercury_filter_lists::timestamp::civil_from_days(seconds / 86_400);
    (
        year,
        month,
        day,
        seconds % 86_400 / 3600,
        seconds % 3600 / 60,
    )
}

#[cfg(test)]
mod tests {
    use mercury_filter_lists::service::ListStatus;

    use super::*;

    const NOV_17_0800_UTC: u64 = 1_794_902_400;

    fn utc(seconds: u64) -> String {
        format!("t{seconds}")
    }

    fn list(name: &str, version: Option<&str>, updated: bool) -> ListStatus {
        ListStatus {
            name: name.to_owned(),
            version: version.map(str::to_owned),
            date: NOV_17_0800_UTC,
            updated,
            host: "easylist.to".to_owned(),
        }
    }

    fn hosted(name: &str, host: &str) -> ListStatus {
        ListStatus {
            host: host.to_owned(),
            ..list(name, None, false)
        }
    }

    #[test]
    fn the_view_names_the_hosts_that_are_contacted() {
        let status = Status {
            lists: vec![
                hosted("EasyList", "easylist.to"),
                hosted("EasyPrivacy", "easylist.to"),
                hosted("Brave Specific", "raw.githubusercontent.com"),
            ],
            ..Status::default()
        };

        assert_eq!(
            settings_view(&status, utc).publishers,
            "Once a day, Mercury Browser asks easylist.to and raw.githubusercontent.com (GitHub) \
             for newer versions of these lists."
        );
        assert_eq!(settings_view(&Status::default(), utc).publishers, "");
    }

    #[test]
    fn automated_runs_never_update_unless_a_local_server_is_named() {
        let automated = LaunchOptions {
            background: true,
            ..LaunchOptions::default()
        };
        assert_eq!(
            updates_for(&automated, true),
            Updates::Off(AUTOMATED_RUN.to_owned())
        );

        let diagnostic = LaunchOptions {
            filter_list_server: Some(Box::new(
                url::Url::parse("http://127.0.0.1:8765/l/").unwrap(),
            )),
            ..automated
        };
        let Updates::On {
            sources,
            check_at_start,
            automatic,
            ..
        } = updates_for(&diagnostic, true)
        else {
            panic!("updates on");
        };
        assert!(matches!(sources, Sources::LocalServer(_)));
        assert!(check_at_start);
        assert!(
            !automatic,
            "a diagnostic run checks once, then never on its own"
        );
    }

    #[test]
    fn normal_runs_update_from_the_publishers_after_start_up_has_settled() {
        let normal = LaunchOptions::default();
        assert_eq!(
            updates_for(&normal, true),
            Updates::On {
                sources: Sources::Publishers,
                automatic: true,
                settle_delay: SETTLE_DELAY,
                check_at_start: false,
            }
        );
        assert!(matches!(
            updates_for(&normal, false),
            Updates::On {
                automatic: false,
                ..
            }
        ));
    }

    #[test]
    fn the_bundled_lists_are_looked_for_in_the_bundle_then_beside_the_executable() {
        let candidates = bundled_dir_candidates(Path::new(
            "/Applications/Mercury Browser.app/Contents/MacOS/mercury-browser",
        ));
        assert_eq!(
            candidates[0],
            Path::new("/Applications/Mercury Browser.app/Contents/MacOS/../Resources/filter-lists")
        );
        assert_eq!(
            candidates[1],
            Path::new("/Applications/Mercury Browser.app/Contents/MacOS/filter-lists")
        );
        let linux = bundled_dir_candidates(Path::new("/opt/mercury/bin/mercury-browser"));
        assert_eq!(
            linux[2],
            Path::new("/opt/mercury/bin/../share/mercury-browser/filter-lists")
        );
    }

    #[test]
    fn a_directory_without_a_manifest_is_skipped() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("bin/mercury-browser");
        std::fs::create_dir_all(directory.path().join("bin/filter-lists")).unwrap();
        assert_eq!(find_bundled_dir(&executable), None);

        std::fs::write(
            directory.path().join("bin/filter-lists/manifest.json"),
            "{}",
        )
        .unwrap();
        assert_eq!(
            find_bundled_dir(&executable),
            Some(directory.path().join("bin/filter-lists"))
        );
    }

    #[test]
    fn the_view_names_each_list_its_version_and_where_it_came_from() {
        let status = Status {
            lists: vec![
                list("EasyList", Some("202611152034"), false),
                list("Brave Specific", None, true),
            ],
            ..Status::default()
        };

        let view = settings_view(&status, utc);

        assert_eq!(
            view.lists,
            [
                format!(
                    "EasyList: version 202611152034, from t{NOV_17_0800_UTC} (included with Mercury Browser)"
                ),
                format!("Brave Specific: from t{NOV_17_0800_UTC} (updated)"),
            ]
        );
        assert_eq!(
            view.status,
            "Not updated yet: using the lists included with Mercury Browser."
        );
        assert!(view.can_update);
    }

    #[test]
    fn the_status_line_shows_the_last_update_and_any_problem() {
        let updated = Status {
            last_check: Some(20),
            last_success: Some(20),
            ..Status::default()
        };
        assert_eq!(settings_view(&updated, utc).status, "Last updated t20.");

        let failed_later = Status {
            last_check: Some(30),
            last_success: Some(20),
            last_error: Some("EasyList: network error".to_owned()),
            ..Status::default()
        };
        assert_eq!(
            settings_view(&failed_later, utc).status,
            "Last updated t20. The last check, t30, failed for EasyList: network error."
        );

        let checking = Status {
            checking: true,
            ..updated
        };
        let view = settings_view(&checking, utc);
        assert_eq!(view.status, "Checking for updates…");
        assert!(!view.can_update);
    }

    #[test]
    fn update_now_is_unavailable_without_updates_or_without_an_engine() {
        let automated = Status {
            updates_unavailable: Some(AUTOMATED_RUN.to_owned()),
            ..Status::default()
        };
        let view = settings_view(&automated, utc);
        assert!(!view.can_update);
        assert!(view.status.ends_with(AUTOMATED_RUN), "{}", view.status);

        let broken = Status {
            problem: Some("The manifest is damaged.".to_owned()),
            ..Status::default()
        };
        let view = settings_view(&broken, utc);
        assert!(!view.can_update);
        assert_eq!(
            view.status,
            "Ad and tracker blocking is off: The manifest is damaged."
        );
    }

    #[test]
    fn local_times_read_as_day_month_year_and_time() {
        let text = local_time(NOV_17_0800_UTC);
        // The day depends on the machine's time zone; the form does not.
        let parts = text.split(' ').collect::<Vec<_>>();
        assert_eq!(parts.len(), 4, "{text}");
        assert!(parts[1] == "Nov", "{text}");
        assert_eq!(parts[2], "2026,");
        assert_eq!(parts[3].len(), 5);
    }
}
