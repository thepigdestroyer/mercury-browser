//! Saving the open tabs, restoring them after a restart or a crash, and reopening closed tabs.
//!
//! This is the application side of the contract in `mercury-settings`' `lib.rs`:
//!
//! - `start` runs once, after the profile lock is taken: it sweeps stale temporaries (2), calls
//!   `SessionStore::begin_run` before any session save (3), offers the recovery list whenever it
//!   is not empty (5), and restores a `last` that quit in order only when the start-up setting
//!   asks for it, keeping it for "Restore previous session" otherwise (6).
//! - Every change is marked on the `SaveThrottle`; a timer writes through it off the UI thread,
//!   one write at a time, handing the token back only for a write that landed (8).
//! - `finish_run` waits for the write in flight, then writes with `exited_cleanly: true`, even
//!   when nothing changed, and calls `flushed` (9).
//! - New-tab pages are saved as `about:newtab` (10). A `NewerVersion` refusal stops saving for
//!   the rest of the run and tells the user once (11).

mod model;
mod saver;
mod screens;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mercury_settings::{
    SaveThrottle, SaveToken, SessionSnapshot, SessionStore, SettingsError, SettingsStore,
    StartupBehavior, WindowBounds, WindowSnapshot, sweep_stale_temporaries,
};
use mercury_ui::MainWindow;
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, ModelRc, SharedString, Timer, VecModel,
};
use talaria_core::BrowserController;

use crate::profile::ActiveProfile;
use crate::ui_sync::sync_ui;
use model::{
    RestorePlan, SessionTracker, WindowGeometry, duplicate_of_active, place_window, plan_restore,
    recovery_label, reopen_index, restored_tab,
};

/// The session is written at most this often while it keeps changing.
const SAVE_INTERVAL_MS: i64 = 5_000;
/// How often the window is looked at and a save is considered.
const TICK_INTERVAL: Duration = Duration::from_secs(1);

const NEWER_VERSION_NOTICE: &str = "Your tabs won't be remembered this time: they were saved by a newer version of Mercury Browser.";

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

struct Session {
    store: SessionStore,
    throttle: SaveThrottle,
    tracker: SessionTracker,
    worker: Option<saver::SaveWorker>,
    /// The token of the write the worker is doing; at most one at a time.
    in_flight: Option<SaveToken>,
    /// The session file is from a newer version: never written again this run.
    saving_refused: bool,
    /// The last run's session, which quit in order and was not restored at start-up.
    previous: Option<SessionSnapshot>,
    /// Sessions that did not quit in order, newest first, until the user answers the bar.
    recovery: Vec<SessionSnapshot>,
    controller: Rc<RefCell<BrowserController>>,
    window: slint::Weak<MainWindow>,
    timer: Timer,
}

/// Begins the run: call once, after the profile is open and locked and the start-up tabs from
/// the command line are in the controller, before the window is shown. Without a profile
/// directory nothing is saved or restored.
pub(crate) fn start(
    window: &MainWindow,
    controller: &Rc<RefCell<BrowserController>>,
    profile: &ActiveProfile,
    window_size_given: bool,
) {
    let Some(root) = profile.paths.as_ref().map(|paths| paths.root.as_path()) else {
        return;
    };
    if let Err(error) = sweep_stale_temporaries(root) {
        log::warn!("could not remove stale temporary files: {error}");
    }
    let store = SessionStore::new(root);
    let startup = match store.begin_run() {
        Ok(startup) => startup,
        Err(error) => {
            log::error!("could not read the saved session; tabs will not be saved: {error}");
            return;
        }
    };

    let mut tracker = SessionTracker::default();
    let mut previous = None;
    if let Some(last) = startup.last.filter(|last| last.exited_cleanly) {
        if startup_behavior(root) == StartupBehavior::RestoreSession {
            if let Some(first_window) = last.snapshot.windows.first()
                && !window_size_given
            {
                let geometry = apply_window_geometry(window, first_window);
                tracker.observe_geometry(geometry);
            }
            let plan = plan_restore(&last.snapshot, &controller.borrow(), false);
            apply_plan(controller, &mut tracker, plan);
        } else {
            previous = Some(last.snapshot);
        }
    }

    let mut throttle = SaveThrottle::new(SAVE_INTERVAL_MS);
    if tracker.observe_tabs(&controller.borrow(), now_ms()) {
        throttle.mark_dirty();
    }
    let worker = match saver::SaveWorker::start(store.clone()) {
        Ok(worker) => Some(worker),
        Err(error) => {
            log::warn!("could not start the session saver; saving only at quit: {error}");
            None
        }
    };

    window.set_recovery_sessions(labels(&startup.recovery));
    window.set_can_restore_previous_session(previous.is_some());
    window.set_can_reopen_closed_tab(tracker.has_closed_tabs());
    connect_callbacks(window, controller);

    let timer = Timer::default();
    timer.start(slint::TimerMode::Repeated, TICK_INTERVAL, tick);
    SESSION.with(|session| {
        *session.borrow_mut() = Some(Session {
            store,
            throttle,
            tracker,
            worker,
            in_flight: None,
            saving_refused: false,
            previous,
            recovery: startup.recovery,
            controller: Rc::clone(controller),
            window: window.as_weak(),
            timer,
        });
    });
}

/// The final save of an orderly quit: waits for the write in flight, then writes the session
/// marked as having quit in order. Call once, after the event loop has ended and before the
/// engine shuts down. Does nothing if the session was never started.
pub(crate) fn finish_run() {
    let Some(mut session) = SESSION.with(|session| session.borrow_mut().take()) else {
        return;
    };
    session.timer.stop();
    let now = now_ms();
    if let Ok(controller) = session.controller.try_borrow()
        && session.tracker.observe_tabs(&controller, now)
    {
        session.throttle.mark_dirty();
    }
    if let Some(worker) = session.worker.take() {
        for outcome in worker.finish() {
            session.take_outcome(outcome, now);
        }
    }
    if session.saving_refused {
        return;
    }
    let Ok(controller) = session.controller.try_borrow() else {
        log::error!("the session could not be saved at quit: the browser state is busy");
        return;
    };
    let snapshot = session.tracker.snapshot(&controller, now);
    match session.store.save(&snapshot, true) {
        Ok(()) => session.throttle.flushed(now),
        Err(error) => log::error!("could not save the session at quit: {error}"),
    }
}

/// Notes a change of the tabs; called whenever the window is brought up to date.
pub(crate) fn observe(window: &MainWindow, controller: &BrowserController) {
    with_session(|session| {
        session.observe_tabs(controller, now_ms());
        window.set_can_reopen_closed_tab(session.tracker.has_closed_tabs());
    });
}

/// "Reopen closed tab": the most recently closed tab, back at its old place, selected.
pub(crate) fn reopen_closed_tab(window: &MainWindow, controller: &RefCell<BrowserController>) {
    let reopened = with_session(|session| {
        // A tab closed since the last look is the one to bring back.
        session.observe_tabs(&controller.borrow(), now_ms());
        let closed = session.tracker.pop_closed()?;
        session.throttle.mark_dirty();
        let mut controller = controller.borrow_mut();
        let index = reopen_index(&closed, controller.tabs().len());
        match controller.open_restored_tab(index, &restored_tab(&closed.tab)) {
            Ok(tab_id) => {
                controller.select_tab(tab_id);
            }
            Err(error) => log::warn!("could not reopen a closed tab: {error}"),
        }
        Some(())
    });
    if reopened.flatten().is_some() {
        sync_ui(window, &controller.borrow());
    }
}

/// "Duplicate tab": a new tab at the active tab's address, just after it, selected.
pub(crate) fn duplicate_tab(window: &MainWindow, controller: &RefCell<BrowserController>) {
    {
        let mut controller = controller.borrow_mut();
        let (index, duplicate) = duplicate_of_active(&controller);
        match controller.open_restored_tab(index, &duplicate) {
            Ok(tab_id) => {
                controller.select_tab(tab_id);
            }
            Err(error) => {
                log::warn!("could not duplicate the tab: {error}");
                return;
            }
        }
    }
    sync_ui(window, &controller.borrow());
}

/// "Restore previous session": the last run's tabs, when it quit in order and they were not
/// restored at start-up. Offered once.
pub(crate) fn restore_previous_session(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
) {
    let restored = with_session(|session| {
        let snapshot = session.previous.take()?;
        let plan = plan_restore(&snapshot, &controller.borrow(), true);
        apply_plan(controller, &mut session.tracker, plan);
        session.throttle.mark_dirty();
        Some(())
    });
    window.set_can_restore_previous_session(false);
    if restored.flatten().is_some() {
        sync_ui(window, &controller.borrow());
    }
}

/// The answer to the recovery bar: `Some(index)` restores that session, `None` dismisses them
/// all. Either way the list is resolved.
fn answer_recovery(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
    restore: Option<usize>,
) {
    let restored = with_session(|session| {
        let snapshot = restore.and_then(|index| session.recovery.get(index).cloned());
        if let Some(snapshot) = &snapshot {
            let plan = plan_restore(snapshot, &controller.borrow(), true);
            apply_plan(controller, &mut session.tracker, plan);
            session.throttle.mark_dirty();
        }
        session.recovery.clear();
        if let Err(error) = session.store.resolve_recovery() {
            log::warn!("could not clear the list of sessions to recover: {error}");
        }
        snapshot.is_some()
    });
    window.set_recovery_sessions(labels(&[]));
    if restored == Some(true) {
        sync_ui(window, &controller.borrow());
    }
}

fn connect_callbacks(window: &MainWindow, controller: &Rc<RefCell<BrowserController>>) {
    let restore_controller = Rc::clone(controller);
    let restore_window = window.as_weak();
    window.on_restore_recovered_session(move |index| {
        if let Some(window) = restore_window.upgrade() {
            answer_recovery(&window, &restore_controller, usize::try_from(index).ok());
        }
    });

    let dismiss_controller = Rc::clone(controller);
    let dismiss_window = window.as_weak();
    window.on_dismiss_recovered_sessions(move || {
        if let Some(window) = dismiss_window.upgrade() {
            answer_recovery(&window, &dismiss_controller, None);
        }
    });

    let previous_controller = Rc::clone(controller);
    let previous_window = window.as_weak();
    window.on_restore_previous_session(move || {
        if let Some(window) = previous_window.upgrade() {
            restore_previous_session(&window, &previous_controller);
        }
    });

    let notice_window = window.as_weak();
    window.on_dismiss_session_notice(move || {
        if let Some(window) = notice_window.upgrade() {
            window.set_session_notice(SharedString::new());
        }
    });
}

/// Opens the planned tabs at the end of the strip, selects the planned one and closes the
/// blank tab they replace (only if at least one tab opened).
fn apply_plan(
    controller: &RefCell<BrowserController>,
    tracker: &mut SessionTracker,
    plan: RestorePlan,
) {
    let mut controller = controller.borrow_mut();
    let mut opened = Vec::with_capacity(plan.tabs.len());
    for tab in &plan.tabs {
        let index = controller.tabs().len();
        match controller.open_restored_tab(index, tab) {
            Ok(tab_id) => opened.push(Some(tab_id)),
            Err(error) => {
                log::warn!("skipping a saved tab: {error}");
                opened.push(None);
            }
        }
    }
    if !opened.iter().any(Option::is_some) {
        return;
    }
    if let Some(select) = plan.select {
        // The planned tab, or the nearest that opened.
        let tab_id = opened[select..]
            .iter()
            .chain(opened[..select].iter().rev())
            .find_map(|tab_id| *tab_id);
        if let Some(tab_id) = tab_id {
            controller.select_tab(tab_id);
        }
    }
    if let Some(blank) = plan.replace {
        controller.close_tab(blank);
    }
    tracker.append_closed(&plan.closed_tabs);
}

/// Puts the window where the saved session had it, before it is shown. Returns what was
/// applied, as the session's starting geometry.
fn apply_window_geometry(window: &MainWindow, saved: &WindowSnapshot) -> WindowGeometry {
    let native = window.window();
    let mut geometry = WindowGeometry {
        bounds: None,
        maximized: saved.maximized,
        fullscreen: saved.fullscreen,
    };
    if let Some(bounds) = saved.bounds {
        let placement = place_window(bounds, &screens::screen_areas());
        if let Some((x, y)) = placement.position {
            native.set_position(LogicalPosition::new(x as f32, y as f32));
        }
        let (width, height) = placement.size;
        native.set_size(LogicalSize::new(width as f32, height as f32));
        let (x, y) = placement.position.unwrap_or((bounds.x, bounds.y));
        geometry.bounds = Some(WindowBounds {
            x,
            y,
            width,
            height,
        });
    }
    if saved.maximized {
        native.set_maximized(true);
    }
    if saved.fullscreen {
        native.set_fullscreen(true);
    }
    geometry
}

impl Session {
    fn observe_tabs(&mut self, controller: &BrowserController, now: i64) {
        if self.tracker.observe_tabs(controller, now) {
            self.throttle.mark_dirty();
        }
    }

    fn observe_window(&mut self, window: &MainWindow) {
        let native = window.window();
        let scale_factor = native.scale_factor();
        let position = native.position().to_logical(scale_factor);
        let size = native.size().to_logical(scale_factor);
        let mut geometry = self.tracker.geometry();
        geometry.update(
            WindowBounds {
                x: position.x.round() as i32,
                y: position.y.round() as i32,
                width: size.width.round().max(0.0) as u32,
                height: size.height.round().max(0.0) as u32,
            },
            native.is_maximized(),
            native.is_fullscreen(),
            window.get_chrome_hidden(),
            native.is_minimized(),
        );
        if self.tracker.observe_geometry(geometry) {
            self.throttle.mark_dirty();
        }
    }

    fn take_outcome(&mut self, outcome: Result<(), SettingsError>, now: i64) {
        let token = self.in_flight.take();
        match outcome {
            Ok(()) => {
                if let Some(token) = token {
                    self.throttle.saved(token, now);
                }
            }
            Err(SettingsError::NewerVersion { .. }) => {
                log::warn!("the session file is from a newer version; not saving tabs this run");
                self.saving_refused = true;
                if let Some(window) = self.window.upgrade() {
                    window.set_session_notice(NEWER_VERSION_NOTICE.into());
                }
            }
            Err(error) => log::warn!("could not save the session: {error}"),
        }
    }

    fn tick(&mut self) {
        let now = now_ms();
        while let Some(outcome) = self
            .worker
            .as_ref()
            .and_then(saver::SaveWorker::try_outcome)
        {
            self.take_outcome(outcome, now);
        }
        if let Some(window) = self.window.upgrade() {
            self.observe_window(&window);
        }
        let controller = Rc::clone(&self.controller);
        let Ok(controller) = controller.try_borrow() else {
            return;
        };
        self.observe_tabs(&controller, now);
        if self.saving_refused || self.in_flight.is_some() {
            return;
        }
        let Some(worker) = &self.worker else {
            return;
        };
        if let Some(token) = self.throttle.should_save(now)
            && worker.save(self.tracker.snapshot(&controller, now))
        {
            self.in_flight = Some(token);
        }
    }
}

fn tick() {
    with_session(Session::tick);
}

/// Runs `f` on the session, unless there is none or it is in use further up the stack.
fn with_session<R>(f: impl FnOnce(&mut Session) -> R) -> Option<R> {
    SESSION.with(|session| {
        let mut session = session.try_borrow_mut().ok()?;
        session.as_mut().map(f)
    })
}

fn labels(sessions: &[SessionSnapshot]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        sessions
            .iter()
            .map(|session| SharedString::from(recovery_label(session)))
            .collect::<Vec<_>>(),
    ))
}

/// The start-up setting.
///
/// Until the settings lane's accessor is merged, this reads `settings.json` through
/// `mercury-settings`' own `SettingsStore` and falls back to its default (`NewTab`); replace
/// the body with a call to that accessor.
fn startup_behavior(profile_root: &Path) -> StartupBehavior {
    match SettingsStore::new(profile_root).load() {
        Ok(loaded) => loaded.value().startup,
        Err(error) => {
            log::warn!("could not read the start-up setting: {error}");
            StartupBehavior::default()
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}
