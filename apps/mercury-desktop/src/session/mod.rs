//! Saving the open tabs, restoring them after a restart or a crash, and reopening closed tabs.
//!
//! This is the application side of the contract in `mercury-settings`' `lib.rs`:
//!
//! - `start` runs once, after the profile lock is taken and the settings (which sweep stale
//!   temporaries, 2) are loaded: it calls `SessionStore::begin_run` before any session save (3), offers the recovery list whenever it
//!   is not empty (5), and restores a `last` that quit in order only when the start-up setting
//!   asks for it, keeping it for "Restore previous session" otherwise (6).
//! - Every change is marked; a timer writes through the throttle off the UI thread, one write at
//!   a time, the token handed back only for a write that landed (8). See `saving::SaveState`.
//! - Restored tabs are written before the recovery entry they came from is removed (5); the
//!   UI thread does not wait for either.
//! - The quit sequence (`post_run.rs`) calls `begin_final_save`, which queues a write with
//!   `exited_cleanly: true` behind the write in flight, even when nothing changed, and later
//!   `collect_final_save(deadline)`, which waits for it until the shared deadline (9). Neither
//!   runs Slint or waits on its event loop: both also run inside AppKit's terminate callback.
//! - New-tab pages are saved as `about:newtab` (10). A `NewerVersion` refusal stops saving for
//!   the rest of the run and tells the user once (11).

mod model;
mod saver;
mod saving;
mod screens;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mercury_settings::{
    SessionSnapshot, SessionStore, StartupBehavior, WindowBounds, WindowSnapshot,
};
use mercury_ui::MainWindow;
use slint::winit_030::WinitWindowAccessor;
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, ModelRc, SharedString, Timer, VecModel,
};
use talaria_core::BrowserController;

use crate::profile::ActiveProfile;
use crate::settings::SettingsHandle;
use crate::shortcuts::Platform;
use crate::ui_sync::sync_ui;
use model::{
    RestorePlan, SavedOrigin, SessionTracker, WindowGeometry, duplicate_of_active, observed_bounds,
    place_window, plan_restore, recovery_label, reopen_index, restored_tab, saved_origin,
};
use saving::{Resolve, SaveState, Written};

/// The session is written at most this often while it keeps changing.
const SAVE_INTERVAL_MS: i64 = 5_000;
/// How often the window is looked at and a save is considered.
const TICK_INTERVAL: Duration = Duration::from_secs(1);

const NEWER_VERSION_NOTICE: &str = "Your tabs won't be remembered this time: they were saved by a newer version of Mercury Browser.";

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

struct Session {
    saving: SaveState<saver::SaveWorker>,
    tracker: SessionTracker,
    /// The last run's session, which quit in order and was not restored at start-up.
    previous: Option<SessionSnapshot>,
    /// Sessions that did not quit in order, newest first, as the bar lists them.
    recovery: Vec<SessionSnapshot>,
    controller: Rc<RefCell<BrowserController>>,
    window: slint::Weak<MainWindow>,
    final_save_begun: bool,
}

/// Begins the run: call once, after the profile is open and locked, the settings loaded, the
/// controller prepared by them (its search builder resolves restored text entries) and the
/// start-up tabs from the command line opened, before the window is shown. Returns the timer
/// that saves the session, for the quit sequence to stop with the others. Without a profile
/// directory nothing is saved or restored.
pub(crate) fn start(
    window: &MainWindow,
    controller: &Rc<RefCell<BrowserController>>,
    profile: &ActiveProfile,
    settings: &SettingsHandle,
    window_size_given: bool,
) -> Option<Timer> {
    let root = profile.paths.as_ref().map(|paths| paths.root.as_path())?;
    let store = SessionStore::new(root);
    // Before `begin_run`, which marks the session file as running: without a saver this run
    // could never write the final save, and every later start would report a crash.
    let worker = match saver::SaveWorker::start(store.clone()) {
        Ok(worker) => worker,
        Err(error) => {
            log::error!("could not start the session saver; tabs will not be saved: {error}");
            return None;
        }
    };
    let startup = match store.begin_run() {
        Ok(startup) => startup,
        Err(error) => {
            log::error!("could not read the saved session; tabs will not be saved: {error}");
            return None;
        }
    };

    let mut tracker = SessionTracker::default();
    let mut previous = None;
    // The window goes back where the last run had it (or, without one, the newest crash).
    let geometry_from = startup
        .last
        .as_ref()
        .map(|last| &last.snapshot)
        .or(startup.recovery.first())
        .and_then(|session| session.windows.first());
    if let Some(last) = startup.last.as_ref().filter(|last| last.exited_cleanly) {
        if settings.startup_behavior() == StartupBehavior::RestoreSession {
            let plan = plan_restore(&last.snapshot, &controller.borrow(), false);
            apply_plan(controller, &mut tracker, plan);
        } else {
            previous = Some(last.snapshot.clone());
        }
    }
    if let Some(saved) = geometry_from
        && !window_size_given
    {
        let geometry = apply_window_geometry(window, saved);
        tracker.observe_geometry(geometry);
    }

    let mut saving = SaveState::new(worker, SAVE_INTERVAL_MS);
    saving.set_holding_previous(previous.is_some());
    if tracker.observe_tabs(&controller.borrow(), now_ms()) {
        saving.mark_dirty();
    }

    window.set_recovery_sessions(labels(&startup.recovery));
    window.set_can_restore_previous_session(previous.is_some());
    window.set_can_reopen_closed_tab(tracker.has_closed_tabs());
    connect_callbacks(window, controller);

    let timer = Timer::default();
    timer.start(slint::TimerMode::Repeated, TICK_INTERVAL, tick);
    SESSION.with(|session| {
        *session.borrow_mut() = Some(Session {
            saving,
            tracker,
            previous,
            recovery: startup.recovery,
            controller: Rc::clone(controller),
            window: window.as_weak(),
            final_save_begun: false,
        });
    });
    Some(timer)
}

/// Starts the final save: ends the periodic saves and queues the session, marked as having
/// quit in order — even when nothing changed — behind any write in flight. Returns at once,
/// so the quit sequence can do other work before `collect_final_save`. Does nothing if the
/// session was never started or the final save was already begun.
pub(crate) fn begin_final_save() {
    with_session(|session| {
        if session.final_save_begun {
            return;
        }
        // The timer itself is stopped by the quit sequence with the others; it does nothing
        // from now on.
        session.final_save_begun = true;
        let now = now_ms();
        let Ok(controller) = session.controller.try_borrow() else {
            log::error!("the session could not be saved at quit: the browser state is busy");
            return;
        };
        if session.tracker.observe_tabs(&controller, now) {
            session.saving.mark_dirty();
        }
        let snapshot = session.tracker.snapshot(&controller, now);
        drop(controller);
        session.saving.begin_final(snapshot);
    });
}

/// Waits until `deadline` at most for the final save begun by `begin_final_save`, then ends
/// the session. If it has not landed by then, the run stays marked as running and is offered
/// as a crash next time — the safe state.
pub(crate) fn collect_final_save(deadline: Instant) {
    let Some(mut session) = SESSION.with(|session| session.borrow_mut().take()) else {
        return;
    };
    match session.saving.collect_final(now_ms(), deadline) {
        Written::Saved | Written::Refused => {}
        Written::TimedOut | Written::Failed => {
            log::error!("the session was not saved at quit; it will be offered as a crash");
        }
    }
}

/// Notes a change of the tabs; called whenever the window is brought up to date. While the
/// active tab is a restored tab without its page, the engine is woken so that the page is
/// created at once, even if the window is covered and skips its redraw.
pub(crate) fn observe(window: &MainWindow, controller: &BrowserController) {
    with_session(|session| {
        session.observe_tabs(controller, now_ms());
        window.set_can_reopen_closed_tab(session.tracker.has_closed_tabs());
    });
    if controller.is_restored_without_page(controller.active_tab_id()) {
        crate::engine::request_pump();
    }
}

/// "Reopen closed tab": the most recently closed tab, back at its old place, selected.
pub(crate) fn reopen_closed_tab(window: &MainWindow, controller: &RefCell<BrowserController>) {
    let reopened = with_session(|session| {
        // A tab closed since the last look is the one to bring back.
        session.observe_tabs(&controller.borrow(), now_ms());
        let closed = session.tracker.pop_closed()?;
        session.saving.mark_dirty();
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
/// restored at start-up. Offered once. The restored tabs are written at once.
pub(crate) fn restore_previous_session(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
) {
    let restored = with_session(|session| {
        let snapshot = session.previous.take()?;
        session.saving.set_holding_previous(false);
        let plan = plan_restore(&snapshot, &controller.borrow(), true);
        session.restore(controller, plan, None);
        Some(())
    });
    // Busy (`None`): nothing happened, and the command stays available.
    if let Some(restored) = restored {
        window.set_can_restore_previous_session(false);
        if restored.is_some() {
            sync_ui(window, &controller.borrow());
        }
    }
}

/// The answer to the recovery bar: `Some(index)` restores that session and keeps the others
/// offered; `None` dismisses them all.
fn answer_recovery(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
    restore: Option<usize>,
) {
    let answered = with_session(|session| {
        let Some(index) = restore else {
            session.recovery.clear();
            session.saving.dismiss_all();
            return false;
        };
        if index >= session.recovery.len() {
            return false;
        }
        let snapshot = session.recovery.remove(index);
        let plan = plan_restore(&snapshot, &controller.borrow(), true);
        session.restore(controller, plan, Some(Resolve::Entry(snapshot)));
        true
    });
    let Some(restored) = answered else {
        return;
    };
    SESSION.with(|session| {
        if let Some(session) = session.borrow().as_ref() {
            window.set_recovery_sessions(labels(&session.recovery));
        }
    });
    if restored {
        sync_ui(window, &controller.borrow());
    }
}

fn connect_callbacks(window: &MainWindow, controller: &Rc<RefCell<BrowserController>>) {
    let restore_controller = Rc::clone(controller);
    let restore_window = window.as_weak();
    window.on_restore_recovered_session(move |index| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(window) = restore_window.upgrade() {
            answer_recovery(&window, &restore_controller, usize::try_from(index).ok());
        }
    });

    let dismiss_controller = Rc::clone(controller);
    let dismiss_window = window.as_weak();
    window.on_dismiss_recovered_sessions(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(window) = dismiss_window.upgrade() {
            answer_recovery(&window, &dismiss_controller, None);
        }
    });

    let previous_controller = Rc::clone(controller);
    let previous_window = window.as_weak();
    window.on_restore_previous_session(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(window) = previous_window.upgrade() {
            restore_previous_session(&window, &previous_controller);
        }
    });

    let notice_window = window.as_weak();
    window.on_dismiss_session_notice(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
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
        log::debug!(
            "session: placing the window's content at {:?}, {width} x {height}",
            placement.position
        );
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
            self.saving.mark_dirty();
        }
    }

    /// Puts a session's tabs back and queues them to be written; `resolve` touches the recovery
    /// list only once they are on disk.
    fn restore(
        &mut self,
        controller: &RefCell<BrowserController>,
        plan: RestorePlan,
        resolve: Option<Resolve>,
    ) {
        apply_plan(controller, &mut self.tracker, plan);
        let now = now_ms();
        let snapshot = {
            let controller = controller.borrow();
            self.observe_tabs(&controller, now);
            self.tracker.snapshot(&controller, now)
        };
        self.saving.persist_restore(snapshot, resolve);
    }

    fn observe_window(&mut self, window: &MainWindow) {
        let native = window.window();
        let scale_factor = native.scale_factor();
        // The origin saved is the one a position set before the window exists means on this
        // platform (see `saved_origin`). Where none is known (Wayland), the last position is
        // kept and the size and state are still followed.
        let position = native
            .with_winit_window(|winit_window| match saved_origin(Platform::current()) {
                SavedOrigin::Content => winit_window.inner_position().ok(),
                SavedOrigin::Outer => winit_window.outer_position().ok(),
            })
            .flatten()
            .map(|position| {
                let logical =
                    slint::PhysicalPosition::new(position.x, position.y).to_logical(scale_factor);
                (logical.x.round() as i32, logical.y.round() as i32)
            });
        let size = native.size().to_logical(scale_factor);
        let mut geometry = self.tracker.geometry();
        geometry.update(
            observed_bounds(
                position,
                (
                    size.width.round().max(0.0) as u32,
                    size.height.round().max(0.0) as u32,
                ),
                geometry.bounds,
            ),
            native.is_maximized(),
            native.is_fullscreen(),
            window.get_chrome_hidden(),
            native.is_minimized(),
        );
        if self.tracker.observe_geometry(geometry) {
            log::debug!("session: the window is now {geometry:?}");
            self.saving.mark_dirty();
        }
    }

    fn tell_refusal(&mut self) {
        if self.saving.take_refusal_notice()
            && let Some(window) = self.window.upgrade()
        {
            window.set_session_notice(NEWER_VERSION_NOTICE.into());
        }
    }

    fn tick(&mut self) {
        if self.final_save_begun {
            return;
        }
        let now = now_ms();
        if let Some(window) = self.window.upgrade() {
            self.observe_window(&window);
        }
        let controller = Rc::clone(&self.controller);
        let Ok(controller) = controller.try_borrow() else {
            return;
        };
        self.observe_tabs(&controller, now);
        let tracker = &self.tracker;
        self.saving.tick(now, || tracker.snapshot(&controller, now));
        self.tell_refusal();
    }
}

fn tick() {
    let _slint = crate::callback_scope::InSlintCallback::enter();
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}
