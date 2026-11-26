// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Hiding the visible page while its window is covered or minimised (energy-1 lane): the
//! glue between the controller's state, the window's cover, the power source and
//! `talaria_core::VisibilityPolicy`. The engine owns the window cover and the web views; this
//! decides, and `PageVisibility` is how a decision reaches a page.

use std::time::Instant;

use talaria_core::{
    BrowserController, TabAudio, ViewVisibility, VisibilityInputs, VisibilityPolicy, WindowCover,
};

use crate::power::{ChangeListener, PowerSource};

/// The visible tab's page, as far as visibility goes. `ServoWebView` implements it in
/// `engine.rs`; tests use a fake.
pub(crate) trait PageVisibility {
    fn show_page(&mut self);
    fn hide_page(&mut self);
}

pub(crate) struct EnergyGovernor {
    policy: VisibilityPolicy,
    /// What the visible page was last told. The policy can change its mind while there is no
    /// page to tell (start-up, a page being replaced); the next pump with a page reconciles.
    applied: ViewVisibility,
    /// The grace-period end `after_pump` last returned, so each one is returned once.
    announced: Option<Instant>,
    power: Box<dyn PowerSource>,
}

impl EnergyGovernor {
    /// `wake` is called when the power state changes, so the engine looks again at once instead
    /// of waiting out a grace period that Low Power Mode has just cancelled.
    pub(crate) fn new(mut power: Box<dyn PowerSource>, wake: ChangeListener) -> Self {
        power.on_change(wake);
        Self {
            policy: VisibilityPolicy::default(),
            applied: ViewVisibility::Shown,
            announced: None,
            power,
        }
    }

    pub(crate) fn inputs(
        &self,
        controller: &BrowserController,
        cover: WindowCover,
    ) -> VisibilityInputs {
        VisibilityInputs {
            cover,
            // A tab the user muted makes no sound, so it is not exempt.
            playing_audio: controller.active_state().audio_indicator() == TabAudio::Playing,
            capturing: !controller
                .capture_access(controller.active_tab_id())
                .is_empty(),
            low_power_mode: self.power.low_power_mode(),
        }
    }

    /// Applies the policy to the visible tab's page (`None` while it has none). Returns when
    /// another pump is wanted: the end of a grace period that has started, once per end (the
    /// engine schedules a timer for it; later pumps in the same period get `None`).
    pub(crate) fn after_pump(
        &mut self,
        controller: &BrowserController,
        cover: WindowCover,
        view: Option<&mut dyn PageVisibility>,
        now: Instant,
    ) -> Option<Instant> {
        let inputs = self.inputs(controller, cover);
        let wanted = self.policy.update(inputs, now);
        if let Some(view) = view
            && wanted != self.applied
        {
            match wanted {
                ViewVisibility::Hidden => view.hide_page(),
                ViewVisibility::Shown => view.show_page(),
            }
            log::debug!("the visible page is now {wanted:?} ({inputs:?})");
            self.applied = wanted;
        }
        let due = self.policy.hide_due(inputs);
        if due.is_some() && due == self.announced {
            return None;
        }
        self.announced = due;
        due
    }

    /// The engine showed a new or replaced web view for the active tab: it is shown, whatever
    /// the previous page was told.
    pub(crate) fn active_view_changed(&mut self) {
        self.applied = ViewVisibility::Shown;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use talaria_core::{BrowserEvent, MediaPlaybackState, MediaProbe, OCCLUDED_HIDE_DELAY, TabId};

    use super::*;
    use crate::power::FakePowerSource;

    #[derive(Default)]
    struct FakeView {
        calls: Vec<&'static str>,
    }

    impl PageVisibility for FakeView {
        fn show_page(&mut self) {
            self.calls.push("show");
        }
        fn hide_page(&mut self) {
            self.calls.push("hide");
        }
    }

    struct Rig {
        governor: EnergyGovernor,
        power: FakePowerSource,
        wakes: Arc<AtomicUsize>,
        controller: BrowserController,
        tab_id: TabId,
        view: FakeView,
        start: Instant,
    }

    impl Rig {
        fn new() -> Self {
            let power = FakePowerSource::default();
            let wakes = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&wakes);
            let governor = EnergyGovernor::new(
                Box::new(power.clone()),
                Box::new(move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                }),
            );
            let controller = BrowserController::default();
            let tab_id = controller.active_tab_id();
            Self {
                governor,
                power,
                wakes,
                controller,
                tab_id,
                view: FakeView::default(),
                start: Instant::now(),
            }
        }

        fn pump(&mut self, cover: WindowCover, milliseconds: u64) -> Option<Instant> {
            self.governor.after_pump(
                &self.controller,
                cover,
                Some(&mut self.view),
                self.start + Duration::from_millis(milliseconds),
            )
        }

        fn at(&self, milliseconds: u64) -> Instant {
            self.start + Duration::from_millis(milliseconds)
        }

        fn event(&mut self, event: BrowserEvent) {
            let tab_id = self.tab_id;
            self.controller.handle_event(tab_id, event);
        }

        fn play(&mut self) {
            self.event(BrowserEvent::AddressChanged {
                url: "https://music.test/".to_owned(),
            });
            self.event(BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Playing,
            });
        }
    }

    #[test]
    fn a_covered_window_hides_the_page_once_after_the_grace_period() {
        let mut rig = Rig::new();

        rig.pump(WindowCover::Occluded, 0);
        rig.pump(WindowCover::Occluded, 999);
        assert!(rig.view.calls.is_empty());

        rig.pump(WindowCover::Occluded, 1000);
        rig.pump(WindowCover::Occluded, 5000);

        assert_eq!(rig.view.calls, ["hide"]);
    }

    #[test]
    fn uncovering_shows_the_page_once() {
        let mut rig = Rig::new();
        rig.pump(WindowCover::Minimised, 0);

        rig.pump(WindowCover::Uncovered, 50);
        rig.pump(WindowCover::Uncovered, 100);

        assert_eq!(rig.view.calls, ["hide", "show"]);
    }

    #[test]
    fn an_uncovered_window_never_touches_the_page() {
        let mut rig = Rig::new();

        for milliseconds in [0, 1000, 60_000] {
            rig.pump(WindowCover::Uncovered, milliseconds);
        }

        assert!(rig.view.calls.is_empty());
    }

    #[test]
    fn an_audible_tab_is_never_hidden() {
        let mut rig = Rig::new();
        rig.play();

        rig.pump(WindowCover::Occluded, 0);
        rig.pump(WindowCover::Minimised, 10_000);

        assert!(rig.view.calls.is_empty());
    }

    #[test]
    fn a_tab_the_user_muted_is_hidden() {
        let mut rig = Rig::new();
        rig.play();
        let tab_id = rig.tab_id;
        rig.controller.toggle_audio_muted(tab_id);

        rig.pump(WindowCover::Minimised, 0);

        assert_eq!(rig.view.calls, ["hide"]);
    }

    #[test]
    fn playing_media_the_probe_found_silent_is_hidden() {
        let mut rig = Rig::new();
        rig.play();
        rig.event(BrowserEvent::MediaProbed {
            probe: MediaProbe {
                playing: 1,
                audible: 0,
                unseen: false,
                fullscreen_video: false,
            },
        });

        rig.pump(WindowCover::Minimised, 0);

        assert_eq!(rig.view.calls, ["hide"]);
    }

    #[test]
    fn audio_that_starts_while_the_page_is_hidden_shows_it() {
        let mut rig = Rig::new();
        rig.pump(WindowCover::Minimised, 0);
        rig.play();

        rig.pump(WindowCover::Minimised, 100);

        assert_eq!(rig.view.calls, ["hide", "show"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_page_that_may_capture_is_never_hidden() {
        use talaria_core::{
            PagePermission, PageRequest, PageRequestAnswer, PageRequestId, PageRequestKind,
            PageRequestReply,
        };

        let mut rig = Rig::new();
        let url = "https://meet.test/room";
        rig.event(BrowserEvent::AddressChanged {
            url: url.to_owned(),
        });
        rig.event(BrowserEvent::PageRequested(PageRequest {
            id: PageRequestId::new(1),
            url: url.to_owned(),
            kind: PageRequestKind::Permission(PagePermission::Camera),
            received_at: Duration::ZERO,
        }));
        let id = rig
            .controller
            .pending_page_request(rig.tab_id)
            .expect("the camera is asked")
            .id();
        let tab_id = rig.tab_id;
        assert!(rig.controller.answer_page_request(
            tab_id,
            id,
            PageRequestReply::new(PageRequestAnswer::Accept),
            Duration::from_secs(1),
        ));
        assert!(!rig.controller.capture_access(tab_id).is_empty());

        rig.pump(WindowCover::Occluded, 0);
        rig.pump(WindowCover::Minimised, 60_000);

        assert!(rig.view.calls.is_empty());
    }

    #[test]
    fn low_power_mode_hides_a_covered_window_at_once() {
        let mut rig = Rig::new();
        rig.power.set_low_power(true);

        rig.pump(WindowCover::Occluded, 0);

        assert_eq!(rig.view.calls, ["hide"]);
    }

    #[test]
    fn a_power_change_wakes_the_engine_and_the_pending_hide_follows_it() {
        let mut rig = Rig::new();
        assert!(rig.pump(WindowCover::Occluded, 0).is_some());
        assert!(rig.view.calls.is_empty());

        rig.power.set_low_power(true);
        assert_eq!(rig.wakes.load(Ordering::SeqCst), 1);
        rig.pump(WindowCover::Occluded, 10);

        assert_eq!(rig.view.calls, ["hide"], "10 ms into a 1 s grace period");
        rig.power.set_low_power(false);
        assert_eq!(rig.wakes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn no_view_applies_nothing_and_the_next_pump_reconciles() {
        let mut rig = Rig::new();
        let at = |milliseconds| rig.start + Duration::from_millis(milliseconds);

        rig.governor
            .after_pump(&rig.controller, WindowCover::Minimised, None, at(0));
        assert!(rig.view.calls.is_empty());

        rig.pump(WindowCover::Minimised, 100);

        assert_eq!(rig.view.calls, ["hide"]);
    }

    #[test]
    fn a_new_active_view_is_hidden_again_while_covered() {
        let mut rig = Rig::new();
        rig.pump(WindowCover::Minimised, 0);
        assert_eq!(rig.view.calls, ["hide"]);

        // The engine shows the web view of a tab switched to while the window was covered.
        rig.governor.active_view_changed();
        rig.pump(WindowCover::Minimised, 50);

        assert_eq!(rig.view.calls, ["hide", "hide"]);
    }

    #[test]
    fn a_new_active_view_stays_shown_when_the_window_is_uncovered() {
        let mut rig = Rig::new();
        rig.pump(WindowCover::Minimised, 0);
        rig.pump(WindowCover::Uncovered, 10);
        assert_eq!(rig.view.calls, ["hide", "show"]);

        rig.governor.active_view_changed();
        rig.pump(WindowCover::Uncovered, 20);

        assert_eq!(rig.view.calls, ["hide", "show"]);
    }

    #[test]
    fn the_deadline_is_returned_once_while_a_hide_is_pending_and_not_after() {
        let mut rig = Rig::new();

        assert_eq!(
            rig.pump(WindowCover::Occluded, 0),
            Some(rig.at(0) + OCCLUDED_HIDE_DELAY)
        );
        assert_eq!(
            rig.pump(WindowCover::Occluded, 400),
            None,
            "already announced"
        );
        assert_eq!(rig.pump(WindowCover::Occluded, 1000), None, "hidden now");
        assert_eq!(rig.pump(WindowCover::Uncovered, 1100), None);
        // A new cover is a new grace period with a new end.
        assert_eq!(
            rig.pump(WindowCover::Occluded, 2000),
            Some(rig.at(2000) + OCCLUDED_HIDE_DELAY)
        );
    }

    #[test]
    fn the_active_tab_decides_not_a_background_one() {
        let mut rig = Rig::new();
        rig.play();
        let playing_tab = rig.tab_id;
        rig.controller.open_tab();
        assert_ne!(rig.controller.active_tab_id(), playing_tab);

        rig.pump(WindowCover::Minimised, 0);

        assert_eq!(
            rig.view.calls,
            ["hide"],
            "the audible tab is in the background"
        );
    }
}
