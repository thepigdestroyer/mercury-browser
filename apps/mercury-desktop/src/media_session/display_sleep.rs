//! Keeping the display awake while a fullscreen video plays.

use std::time::{Duration, Instant};

use talaria_core::BrowserController;

/// After the system refuses the assertion, how long before it is asked again (and so how
/// often a refusal can be logged).
pub(crate) const RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// A "prevent display sleep" power assertion. The macOS implementation is only created in an
/// interactive launch; automated runs and tests use fakes.
pub(crate) trait PowerAssertions {
    fn kind(&self) -> super::ServiceKind;
    /// Returns false if the assertion could not be created.
    fn prevent_display_sleep(&mut self) -> bool;
    fn allow_display_sleep(&mut self);
}

/// Held exactly while the foreground tab's page is fullscreen and plays video there. Leaving
/// fullscreen, pausing, switching tabs, closing the tab and quitting all release it.
#[derive(Debug, Default)]
pub(crate) struct DisplaySleepPolicy {
    held: bool,
    refused_at: Option<Instant>,
}

impl DisplaySleepPolicy {
    pub(crate) fn wanted(controller: &BrowserController) -> bool {
        let state = controller.active_state();
        state.is_fullscreen() && state.media().plays_fullscreen_video()
    }

    pub(crate) fn update(&mut self, wanted: bool, power: &mut dyn PowerAssertions, now: Instant) {
        if wanted && !self.held {
            let waiting = self
                .refused_at
                .is_some_and(|refused| now.saturating_duration_since(refused) < RETRY_INTERVAL);
            if waiting {
                return;
            }
            self.held = power.prevent_display_sleep();
            self.refused_at = (!self.held).then_some(now);
        } else if !wanted {
            self.refused_at = None;
            if self.held {
                power.allow_display_sleep();
                self.held = false;
            }
        }
    }

    pub(crate) fn release(&mut self, power: &mut dyn PowerAssertions) {
        self.update(false, power, Instant::now());
    }

    #[cfg(test)]
    pub(crate) const fn is_held(&self) -> bool {
        self.held
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use talaria_core::{BrowserEvent, MediaPlaybackState, MediaProbe, TabId};

    use super::*;

    #[derive(Default)]
    struct FakePower {
        calls: Vec<&'static str>,
        refuse: bool,
    }

    impl PowerAssertions for FakePower {
        fn kind(&self) -> crate::media_session::ServiceKind {
            crate::media_session::ServiceKind::Test
        }
        fn prevent_display_sleep(&mut self) -> bool {
            self.calls.push("prevent");
            !self.refuse
        }
        fn allow_display_sleep(&mut self) {
            self.calls.push("allow");
        }
    }

    fn fullscreen_video(controller: &mut BrowserController, tab_id: TabId) {
        controller.handle_event(
            tab_id,
            BrowserEvent::AddressChanged {
                url: "https://v.test/".to_owned(),
            },
        );
        controller.handle_event(tab_id, BrowserEvent::FullscreenChanged { fullscreen: true });
        controller.handle_event(
            tab_id,
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Playing,
            },
        );
        controller.handle_event(
            tab_id,
            BrowserEvent::MediaProbed {
                probe: MediaProbe {
                    playing: 1,
                    audible: 1,
                    unseen: false,
                    fullscreen_video: true,
                },
            },
        );
    }

    fn run(policy: &mut DisplaySleepPolicy, controller: &BrowserController, power: &mut FakePower) {
        policy.update(
            DisplaySleepPolicy::wanted(controller),
            power,
            Instant::now(),
        );
    }

    #[test]
    fn a_fullscreen_video_holds_the_assertion_once() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut policy = DisplaySleepPolicy::default();
        let mut power = FakePower::default();
        fullscreen_video(&mut controller, tab_id);

        run(&mut policy, &controller, &mut power);
        run(&mut policy, &controller, &mut power);

        assert!(policy.is_held());
        assert_eq!(power.calls, ["prevent"]);
    }

    #[test]
    fn fullscreen_without_playing_video_does_not_hold_it() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut policy = DisplaySleepPolicy::default();
        let mut power = FakePower::default();
        controller.handle_event(tab_id, BrowserEvent::FullscreenChanged { fullscreen: true });
        run(&mut policy, &controller, &mut power);
        // A fullscreen element that is not a video, with audio playing.
        controller.handle_event(
            tab_id,
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Playing,
            },
        );
        controller.handle_event(
            tab_id,
            BrowserEvent::MediaProbed {
                probe: MediaProbe {
                    playing: 1,
                    audible: 1,
                    unseen: false,
                    fullscreen_video: false,
                },
            },
        );
        run(&mut policy, &controller, &mut power);

        assert!(power.calls.is_empty());
    }

    #[test]
    fn pausing_releases_it() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut policy = DisplaySleepPolicy::default();
        let mut power = FakePower::default();
        fullscreen_video(&mut controller, tab_id);
        run(&mut policy, &controller, &mut power);

        controller.handle_event(
            tab_id,
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Paused,
            },
        );
        run(&mut policy, &controller, &mut power);

        assert_eq!(power.calls, ["prevent", "allow"]);
        assert!(!policy.is_held());
    }

    #[test]
    fn leaving_fullscreen_releases_it() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut policy = DisplaySleepPolicy::default();
        let mut power = FakePower::default();
        fullscreen_video(&mut controller, tab_id);
        run(&mut policy, &controller, &mut power);

        controller.handle_event(
            tab_id,
            BrowserEvent::FullscreenChanged { fullscreen: false },
        );
        run(&mut policy, &controller, &mut power);

        assert_eq!(power.calls, ["prevent", "allow"]);
    }

    #[test]
    fn a_background_tab_does_not_hold_it() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut policy = DisplaySleepPolicy::default();
        let mut power = FakePower::default();
        fullscreen_video(&mut controller, tab_id);
        run(&mut policy, &controller, &mut power);

        let other = controller.open_tab();
        run(&mut policy, &controller, &mut power);
        assert_eq!(power.calls, ["prevent", "allow"]);

        assert!(controller.select_tab(tab_id));
        run(&mut policy, &controller, &mut power);
        assert_eq!(power.calls, ["prevent", "allow", "prevent"]);

        // Closing the tab that holds it.
        assert!(controller.close_tab(tab_id));
        assert_eq!(controller.active_tab_id(), other);
        run(&mut policy, &controller, &mut power);
        assert_eq!(power.calls, ["prevent", "allow", "prevent", "allow"]);
    }

    #[test]
    fn a_crash_releases_it() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut policy = DisplaySleepPolicy::default();
        let mut power = FakePower::default();
        fullscreen_video(&mut controller, tab_id);
        run(&mut policy, &controller, &mut power);

        controller.handle_event(
            tab_id,
            BrowserEvent::LoadStateChanged {
                state: talaria_core::LoadState::Failed {
                    message: "crashed".to_owned(),
                },
            },
        );
        run(&mut policy, &controller, &mut power);

        assert_eq!(power.calls, ["prevent", "allow"]);
    }

    #[test]
    fn quitting_releases_it_and_releasing_twice_is_harmless() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut policy = DisplaySleepPolicy::default();
        let mut power = FakePower::default();
        fullscreen_video(&mut controller, tab_id);
        run(&mut policy, &controller, &mut power);

        policy.release(&mut power);
        policy.release(&mut power);

        assert_eq!(power.calls, ["prevent", "allow"]);
    }

    #[test]
    fn a_refused_assertion_is_asked_for_again_once_per_interval() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut policy = DisplaySleepPolicy::default();
        let mut power = FakePower {
            refuse: true,
            ..FakePower::default()
        };
        fullscreen_video(&mut controller, tab_id);
        let start = Instant::now();
        policy.update(true, &mut power, start);
        assert!(!policy.is_held());

        // Every pump in between asks again only once the interval has passed.
        policy.update(true, &mut power, start + RETRY_INTERVAL / 2);
        policy.update(
            true,
            &mut power,
            start + RETRY_INTERVAL - Duration::from_millis(1),
        );
        assert_eq!(power.calls, ["prevent"]);

        power.refuse = false;
        policy.update(true, &mut power, start + RETRY_INTERVAL);
        policy.release(&mut power);

        assert_eq!(power.calls, ["prevent", "prevent", "allow"]);
    }
}
