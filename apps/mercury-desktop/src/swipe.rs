// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The two-finger swipe for back and forward (gestures lane, R13 W1-7).
//!
//! `talaria_core::gestures::SwipeRecogniser` decides; this file feeds it. The wheel's phase is
//! lost on the way through Slint's `TouchArea`, so the one `WindowEvent::MouseWheel` arm of the
//! winit hook in `keyboard.rs` calls [`on_wheel`] before Slint sees the event. **The event is never
//! consumed**: the page still gets every wheel event, and what it answers (Servo's
//! `notify_input_event_handled`, carried as `BrowserEvent::WheelHandled` and counted in
//! `BrowserState::wheel_prevented_count`) tells the recogniser when the page has taken the
//! scroll for itself. Pinch is not read here (input-hooks owns it).
//!
//! Everything that decides is in [`Core`], [`guard`] and [`wheel_sample`], which take plain
//! values; [`on_wheel`] only reads the window and the controller and applies the answer.
//!
//! `MERCURY_WHEEL_TRACE=<file>` appends every wheel event in the recorder format of
//! `talaria_core::gestures::fixture`, so a real trackpad session can become a test fixture.

use std::cell::Cell;
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mercury_ui::MainWindow;
use slint::ComponentHandle;
use slint::winit_030::winit::event::{MouseScrollDelta, TouchPhase};
use slint::winit_030::winit::keyboard::ModifiersState;
use talaria_core::gestures::fixture;
use talaria_core::{
    BrowserCommand, BrowserController, Feedback, Reach, SwipeDirection, SwipeRecogniser, TabId,
    Tuning, WheelPhase, WheelSample,
};

use crate::settings::SettingsHandle;

/// The logical pixels one notch of a line-based wheel is worth (what Slint uses).
const LINE_PIXELS: f32 = 60.0;

/// What a trackpad sample means to the recogniser. `PixelDelta` is physical pixels from a
/// trackpad or a Magic Mouse (precise); `LineDelta` is a notched wheel. `natural` is the system's
/// "Natural scrolling" switch: with it off AppKit hands over the opposite sign, and the
/// recogniser wants the direction of the fingers.
pub(crate) fn wheel_sample(
    delta: &MouseScrollDelta,
    phase: TouchPhase,
    scale_factor: f32,
    at: Duration,
    natural: bool,
) -> WheelSample {
    let (x, y, precise) = match delta {
        MouseScrollDelta::PixelDelta(pixels) => {
            let scale = if scale_factor > 0.0 { scale_factor } else { 1.0 };
            (pixels.x as f32 / scale, pixels.y as f32 / scale, true)
        }
        MouseScrollDelta::LineDelta(x, y) => (x * LINE_PIXELS, y * LINE_PIXELS, false),
    };
    let sign = if natural { 1.0 } else { -1.0 };
    WheelSample {
        at,
        delta_x: x * sign,
        delta_y: y * sign,
        phase: match phase {
            TouchPhase::Started => WheelPhase::Began,
            TouchPhase::Moved => WheelPhase::Changed,
            TouchPhase::Ended => WheelPhase::Ended,
            TouchPhase::Cancelled => WheelPhase::Cancelled,
        },
        precise,
    }
}

/// The winit name of a phase, for the trace file.
fn phase_name(phase: TouchPhase) -> &'static str {
    match phase {
        TouchPhase::Started => "Started",
        TouchPhase::Moved => "Moved",
        TouchPhase::Ended => "Ended",
        TouchPhase::Cancelled => "Cancelled",
    }
}

/// What the system's "Natural scrolling" value says; absent means the default, which is on.
pub(crate) fn natural_scrolling(defaults: Option<bool>) -> bool {
    defaults.unwrap_or(true)
}

/// `com.apple.swipescrolldirection` in the global defaults, if it is set.
#[cfg(target_os = "macos")]
fn read_natural_scrolling() -> Option<bool> {
    use objc2_foundation::{NSString, NSUserDefaults};
    let key = NSString::from_str("com.apple.swipescrolldirection");
    let defaults = NSUserDefaults::standardUserDefaults();
    defaults
        .objectForKey(&key)
        .map(|_| defaults.boolForKey(&key))
}

#[cfg(not(target_os = "macos"))]
fn read_natural_scrolling() -> Option<bool> {
    None
}

#[cfg(target_os = "macos")]
fn reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(not(target_os = "macos"))]
fn reduce_motion() -> bool {
    false
}

/// Everything that must hold for a wheel event to count towards a swipe.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GuardInputs {
    /// `web.swipe_navigation`.
    pub(crate) enabled: bool,
    /// Orbit and any other full cover hide the page.
    pub(crate) web_view_visible: bool,
    /// The pointer is over the page, not over the tab strip or another panel.
    pub(crate) web_view_hovered: bool,
    /// The PDF viewer takes the wheel itself.
    pub(crate) pdf_viewer_shown: bool,
    /// A page sheet dims the page and takes the wheel.
    pub(crate) sheet_shown: bool,
    /// ⌘, ⌃ or ⌥ is down: those wheel gestures are zoom or page-level elsewhere.
    pub(crate) modifiers_down: bool,
}

pub(crate) fn guard(inputs: GuardInputs) -> bool {
    inputs.enabled
        && inputs.web_view_visible
        && inputs.web_view_hovered
        && !inputs.pdf_viewer_shown
        && !inputs.sheet_shown
        && !inputs.modifiers_down
}

/// One wheel event and what the window and controller say about it.
pub(crate) struct Step {
    pub(crate) sample: WheelSample,
    pub(crate) reach: Reach,
    pub(crate) tab: TabId,
    /// `BrowserState::wheel_prevented_count` of the active tab, now.
    pub(crate) prevented_count: u64,
    pub(crate) guard_ok: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Outcome {
    pub(crate) feedback: Feedback,
    pub(crate) commit: Option<SwipeDirection>,
}

/// The decision logic, without a window.
pub(crate) struct Core {
    recogniser: SwipeRecogniser,
    /// The prevented-wheel count when the gesture began.
    baseline: u64,
    /// The tab the gesture began in.
    tab: Option<TabId>,
}

impl Core {
    pub(crate) fn new() -> Self {
        Self {
            recogniser: SwipeRecogniser::new(Tuning::DEFAULT),
            baseline: 0,
            tab: None,
        }
    }

    pub(crate) fn step(&mut self, step: Step) -> Outcome {
        if !step.guard_ok {
            return self.cancel();
        }
        if self.tab.is_some_and(|tab| tab != step.tab) {
            self.cancel();
        }
        if step.sample.phase == WheelPhase::Began {
            self.baseline = step.prevented_count;
            self.tab = Some(step.tab);
        }
        if step.prevented_count > self.baseline {
            // The page cancelled a wheel event of this gesture: it owns the scroll. Idempotent
            // once the gesture is ignored.
            self.recogniser.page_consumed();
            self.baseline = step.prevented_count;
        }
        let update = self.recogniser.on_wheel(step.sample, step.reach);
        if update.commit.is_some() || matches!(step.sample.phase, WheelPhase::Ended | WheelPhase::Cancelled) {
            self.tab = None;
        }
        Outcome {
            feedback: update.feedback,
            commit: update.commit,
        }
    }

    /// The gesture cannot continue (focus lost, a guard turned false).
    pub(crate) fn cancel(&mut self) -> Outcome {
        self.tab = None;
        let update = self.recogniser.cancel();
        Outcome {
            feedback: update.feedback,
            commit: None,
        }
    }
}

/// What the window keeps between wheel events.
pub(crate) struct SwipeState {
    core: Core,
    clock_origin: Option<Instant>,
    natural: bool,
    shown: Feedback,
    trace: Option<std::fs::File>,
}

impl SwipeState {
    pub(crate) fn new() -> Self {
        let trace = std::env::var_os("MERCURY_WHEEL_TRACE").and_then(|path| {
            match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                Ok(file) => Some(file),
                Err(error) => {
                    log::warn!("gestures: cannot open the wheel trace {path:?}: {error}");
                    None
                }
            }
        });
        Self {
            core: Core::new(),
            clock_origin: None,
            natural: true,
            shown: Feedback::Hidden,
            trace,
        }
    }

    fn now(&mut self) -> Duration {
        self.clock_origin.get_or_insert_with(Instant::now).elapsed()
    }

    fn write_trace(&mut self, sample: &WheelSample, phase: TouchPhase) {
        let Some(file) = self.trace.as_mut() else {
            return;
        };
        if writeln!(file, "{}", fixture::format(sample, phase_name(phase))).is_err() {
            log::warn!("gestures: the wheel trace cannot be written; tracing stops");
            self.trace = None;
        }
    }
}

/// The `swipe_navigation` switch, kept current by the settings' listener.
pub(crate) fn follow_settings(settings: &Rc<SettingsHandle>) -> Rc<Cell<bool>> {
    let enabled = Rc::new(Cell::new(settings.settings().web.swipe_navigation));
    let weak = Rc::downgrade(settings);
    let flag = Rc::clone(&enabled);
    settings.subscribe_while(move || {
        let Some(settings) = weak.upgrade() else {
            return false;
        };
        flag.set(settings.settings().web.swipe_navigation);
        true
    });
    enabled
}

/// Called from the `WindowEvent::MouseWheel` arm of `keyboard.rs`. Never consumes the event.
pub(crate) fn on_wheel(
    window: &MainWindow,
    controller: &Rc<RefCell<BrowserController>>,
    state: &mut SwipeState,
    enabled: &Cell<bool>,
    delta: &MouseScrollDelta,
    phase: TouchPhase,
    modifiers: ModifiersState,
) {
    let scale = window.window().scale_factor();
    let at = state.now();
    if state.trace.is_some() {
        let raw = wheel_sample(delta, phase, scale, at, true);
        state.write_trace(&raw, phase);
    }
    if phase == TouchPhase::Started {
        state.natural = natural_scrolling(read_natural_scrolling());
        window.set_swipe_reduce_motion(reduce_motion());
    }
    let sample = wheel_sample(delta, phase, scale, at, state.natural);

    let (reach, tab, prevented_count, sheet_shown) = {
        let controller = controller.borrow();
        let active = controller.active_state();
        (
            Reach {
                back: active.can_go_back(),
                forward: active.can_go_forward(),
            },
            controller.active_tab_id(),
            active.wheel_prevented_count(),
            crate::page_requests::sheet_shown(&controller),
        )
    };
    let guard_ok = guard(GuardInputs {
        enabled: enabled.get(),
        web_view_visible: window.get_web_view_visible(),
        web_view_hovered: window.get_web_view_hovered(),
        pdf_viewer_shown: window.get_pdf_viewer_shown(),
        sheet_shown,
        modifiers_down: modifiers.super_key() || modifiers.control_key() || modifiers.alt_key(),
    });
    let outcome = state.core.step(Step {
        sample,
        reach,
        tab,
        prevented_count,
        guard_ok,
    });
    apply(window, controller, state, outcome);
}

/// A focus loss, a close or an occlusion: the gesture is over and the indicator goes.
pub(crate) fn reset(window: &MainWindow, state: &mut SwipeState) {
    let outcome = state.core.cancel();
    show(window, state, outcome.feedback);
}

fn apply(
    window: &MainWindow,
    controller: &Rc<RefCell<BrowserController>>,
    state: &mut SwipeState,
    outcome: Outcome,
) {
    show(window, state, outcome.feedback);
    if let Some(direction) = outcome.commit {
        // As the keyboard shortcut does: the address draft ends, then the navigation is queued.
        window.invoke_accept_address_draft();
        queue_navigation(&mut controller.borrow_mut(), direction);
        window.window().request_redraw();
    }
}

pub(crate) fn queue_navigation(controller: &mut BrowserController, direction: SwipeDirection) {
    controller.queue_command(match direction {
        SwipeDirection::Back => BrowserCommand::GoBack,
        SwipeDirection::Forward => BrowserCommand::GoForward,
    });
}

/// Writes the indicator's properties when they changed.
fn show(window: &MainWindow, state: &mut SwipeState, feedback: Feedback) {
    if state.shown == feedback {
        return;
    }
    state.shown = feedback;
    match feedback {
        Feedback::Hidden => window.set_swipe_shown(false),
        Feedback::Shown {
            direction,
            progress,
            armed,
        } => {
            window.set_swipe_forward(direction == SwipeDirection::Forward);
            window.set_swipe_progress(progress);
            window.set_swipe_armed(armed);
            window.set_swipe_shown(true);
        }
    }
    window.window().request_redraw();
}

#[cfg(test)]
mod tests {
    use slint::winit_030::winit::dpi::PhysicalPosition;
    use talaria_core::{BrowserEvent, Stage};

    use super::*;

    const BACK_SLOW: &str =
        include_str!("../../../crates/talaria-core/tests/fixtures/wheel/swipe-back-slow.txt");

    fn pixels(x: f64, y: f64) -> MouseScrollDelta {
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(x, y))
    }

    fn all_guards() -> GuardInputs {
        GuardInputs {
            enabled: true,
            web_view_visible: true,
            web_view_hovered: true,
            pdf_viewer_shown: false,
            sheet_shown: false,
            modifiers_down: false,
        }
    }

    fn both() -> Reach {
        Reach {
            back: true,
            forward: true,
        }
    }

    /// Feeds `fixture` to a core; `prevented` maps the sample index to the page's count then.
    fn run(
        core: &mut Core,
        fixture: &str,
        reach: Reach,
        tab: TabId,
        prevented: impl Fn(usize) -> u64,
        guard_ok: impl Fn(usize) -> bool,
    ) -> Vec<Outcome> {
        fixture::parse(fixture)
            .expect("the fixture parses")
            .into_iter()
            .enumerate()
            .map(|(index, sample)| {
                core.step(Step {
                    sample,
                    reach,
                    tab,
                    prevented_count: prevented(index),
                    guard_ok: guard_ok(index),
                })
            })
            .collect()
    }

    fn tab() -> TabId {
        BrowserController::default().active_tab_id()
    }

    fn commits(outcomes: &[Outcome]) -> Vec<SwipeDirection> {
        outcomes.iter().filter_map(|outcome| outcome.commit).collect()
    }

    #[test]
    fn wheel_sample_maps_pixels_and_lines() {
        let sample = wheel_sample(
            &pixels(30.0, 6.0),
            TouchPhase::Started,
            2.0,
            Duration::from_millis(5),
            true,
        );
        assert_eq!((sample.delta_x, sample.delta_y), (15.0, 3.0));
        assert!(sample.precise);
        assert_eq!(sample.phase, WheelPhase::Began);
        assert_eq!(sample.at, Duration::from_millis(5));

        let sample = wheel_sample(
            &MouseScrollDelta::LineDelta(1.0, 0.0),
            TouchPhase::Moved,
            2.0,
            Duration::ZERO,
            true,
        );
        assert_eq!((sample.delta_x, sample.delta_y), (60.0, 0.0));
        assert!(!sample.precise);
    }

    #[test]
    fn natural_off_flips_the_finger_direction() {
        let sample = wheel_sample(&pixels(10.0, -4.0), TouchPhase::Moved, 1.0, Duration::ZERO, false);
        assert_eq!((sample.delta_x, sample.delta_y), (-10.0, 4.0));
    }

    #[test]
    fn phase_names_map() {
        for (winit, expected) in [
            (TouchPhase::Started, WheelPhase::Began),
            (TouchPhase::Moved, WheelPhase::Changed),
            (TouchPhase::Ended, WheelPhase::Ended),
            (TouchPhase::Cancelled, WheelPhase::Cancelled),
        ] {
            let sample = wheel_sample(&pixels(1.0, 0.0), winit, 1.0, Duration::ZERO, true);
            assert_eq!(sample.phase, expected);
        }
    }

    #[test]
    fn a_zero_scale_factor_does_not_divide_by_zero() {
        let sample = wheel_sample(&pixels(8.0, 0.0), TouchPhase::Moved, 0.0, Duration::ZERO, true);
        assert_eq!(sample.delta_x, 8.0);
    }

    #[test]
    fn guard_needs_every_condition() {
        assert!(guard(all_guards()));
        let breakers: [fn(&mut GuardInputs); 6] = [
            |g| g.enabled = false,
            |g| g.web_view_visible = false,
            |g| g.web_view_hovered = false,
            |g| g.pdf_viewer_shown = true,
            |g| g.sheet_shown = true,
            |g| g.modifiers_down = true,
        ];
        for (index, breaker) in breakers.iter().enumerate() {
            let mut inputs = all_guards();
            breaker(&mut inputs);
            assert!(!guard(inputs), "condition {index} must stop the swipe");
        }
    }

    #[test]
    fn no_defaults_value_means_natural() {
        assert!(natural_scrolling(None));
        assert!(natural_scrolling(Some(true)));
        assert!(!natural_scrolling(Some(false)));
    }

    #[test]
    fn core_commits_back_on_the_slow_swipe_fixture() {
        let mut core = Core::new();
        let outcomes = run(&mut core, BACK_SLOW, both(), tab(), |_| 0, |_| true);
        assert_eq!(commits(&outcomes), vec![SwipeDirection::Back]);
        assert_eq!(outcomes.last().map(|o| o.feedback), Some(Feedback::Hidden));
    }

    #[test]
    fn a_prevented_wheel_result_cancels_the_swipe() {
        let mut core = Core::new();
        let outcomes = run(&mut core, BACK_SLOW, both(), tab(), |i| u64::from(i >= 5), |_| true);
        assert!(commits(&outcomes).is_empty(), "the page owns the scroll");
        assert_eq!(outcomes.last().map(|o| o.feedback), Some(Feedback::Hidden));
        assert!(
            outcomes.iter().skip(6).all(|o| o.feedback == Feedback::Hidden),
            "the indicator is gone as soon as the page takes the scroll"
        );
    }

    #[test]
    fn a_prevented_count_from_before_the_gesture_is_not_this_gestures() {
        let mut core = Core::new();
        let outcomes = run(&mut core, BACK_SLOW, both(), tab(), |_| 7, |_| true);
        assert_eq!(commits(&outcomes), vec![SwipeDirection::Back]);
    }

    #[test]
    fn a_guard_turning_false_cancels_and_hides() {
        let mut core = Core::new();
        let outcomes = run(&mut core, BACK_SLOW, both(), tab(), |_| 0, |i| i < 10);
        assert!(commits(&outcomes).is_empty());
        assert_eq!(outcomes.last().map(|o| o.feedback), Some(Feedback::Hidden));
    }

    #[test]
    fn a_swipe_started_while_guarded_off_never_starts_halfway() {
        let mut core = Core::new();
        // The guard is off for the first 6 samples (a modifier was down), then on.
        let outcomes = run(&mut core, BACK_SLOW, both(), tab(), |_| 0, |i| i >= 6);
        assert!(commits(&outcomes).is_empty(), "a gesture needs its own start");
    }

    #[test]
    fn a_tab_change_cancels_the_gesture() {
        let mut controller = BrowserController::default();
        let first = controller.active_tab_id();
        let second = controller.open_tab();
        let samples = fixture::parse(BACK_SLOW).expect("the fixture parses");
        let mut core = Core::new();
        let mut committed = false;
        for (index, sample) in samples.into_iter().enumerate() {
            let outcome = core.step(Step {
                sample,
                reach: both(),
                tab: if index < 10 { first } else { second },
                prevented_count: 0,
                guard_ok: true,
            });
            committed |= outcome.commit.is_some();
        }
        assert!(!committed);
    }

    #[test]
    fn reset_hides_the_indicator() {
        let mut core = Core::new();
        let samples = fixture::parse(BACK_SLOW).expect("the fixture parses");
        let mut last = Feedback::Hidden;
        for sample in samples.into_iter().take(15) {
            last = core
                .step(Step {
                    sample,
                    reach: both(),
                    tab: tab(),
                    prevented_count: 0,
                    guard_ok: true,
                })
                .feedback;
        }
        assert!(matches!(last, Feedback::Shown { .. }), "mid-swipe: {last:?}");
        let outcome = core.cancel();
        assert_eq!(outcome.feedback, Feedback::Hidden);
        assert_eq!(outcome.commit, None);
        assert_eq!(core.recogniser.stage(), Stage::Idle);
    }

    #[test]
    fn back_with_no_history_does_nothing() {
        let mut core = Core::new();
        let none = Reach {
            back: false,
            forward: false,
        };
        let outcomes = run(&mut core, BACK_SLOW, none, tab(), |_| 0, |_| true);
        assert!(commits(&outcomes).is_empty());
        assert!(outcomes.iter().all(|o| o.feedback == Feedback::Hidden));
    }

    #[test]
    fn a_committed_swipe_queues_go_back() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        controller.handle_event(
            tab_id,
            BrowserEvent::HistoryChanged {
                can_go_back: true,
                can_go_forward: false,
            },
        );
        assert!(controller.active_state().can_go_back());

        queue_navigation(&mut controller, SwipeDirection::Back);
        assert_eq!(
            controller.next_command().map(|c| c.into_command()),
            Some(BrowserCommand::GoBack)
        );
        queue_navigation(&mut controller, SwipeDirection::Forward);
        assert_eq!(
            controller.next_command().map(|c| c.into_command()),
            Some(BrowserCommand::GoForward)
        );
    }

    #[test]
    fn the_wheel_state_reads_the_page_answers_of_the_active_tab() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        controller.handle_event(tab_id, BrowserEvent::WheelHandled { prevented: false });
        assert_eq!(controller.active_state().wheel_prevented_count(), 0);
        controller.handle_event(tab_id, BrowserEvent::WheelHandled { prevented: true });
        assert_eq!(controller.active_state().wheel_prevented_count(), 1);
    }

    #[test]
    fn trace_lines_read_back_as_the_same_sample() {
        let sample = wheel_sample(&pixels(12.5, -1.0), TouchPhase::Moved, 1.0, Duration::from_micros(8200), true);
        let line = fixture::format(&sample, phase_name(TouchPhase::Moved));
        assert_eq!(fixture::parse(&line).expect("the line parses"), vec![sample]);
    }
}
