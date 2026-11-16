//! Keeping the display awake while a fullscreen video plays.

use talaria_core::BrowserController;

/// A "prevent display sleep" power assertion. The macOS implementation is only created in an
/// interactive launch; automated runs and tests use fakes.
pub(crate) trait PowerAssertions {
    /// Returns false if the assertion could not be created.
    fn prevent_display_sleep(&mut self) -> bool;
    fn allow_display_sleep(&mut self);
}

/// Held exactly while the foreground tab's page is fullscreen and plays video there. Leaving
/// fullscreen, pausing, switching tabs, closing the tab and quitting all release it.
#[derive(Debug, Default)]
pub(crate) struct DisplaySleepPolicy {
    held: bool,
}

impl DisplaySleepPolicy {
    pub(crate) fn wanted(controller: &BrowserController) -> bool {
        let state = controller.active_state();
        state.is_fullscreen() && state.media().plays_fullscreen_video()
    }

    pub(crate) fn update(&mut self, wanted: bool, power: &mut dyn PowerAssertions) {
        if wanted && !self.held {
            self.held = power.prevent_display_sleep();
        } else if !wanted && self.held {
            power.allow_display_sleep();
            self.held = false;
        }
    }

    pub(crate) fn release(&mut self, power: &mut dyn PowerAssertions) {
        self.update(false, power);
    }

    #[cfg(test)]
    pub(crate) const fn is_held(&self) -> bool {
        self.held
    }
}

#[cfg(test)]
mod tests {
    use talaria_core::{BrowserEvent, MediaPlaybackState, MediaProbe, TabId};

    use super::*;

    #[derive(Default)]
    struct FakePower {
        calls: Vec<&'static str>,
        refuse: bool,
    }

    impl PowerAssertions for FakePower {
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
                    fullscreen_video: true,
                },
            },
        );
    }

    fn run(policy: &mut DisplaySleepPolicy, controller: &BrowserController, power: &mut FakePower) {
        policy.update(DisplaySleepPolicy::wanted(controller), power);
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
    fn a_refused_assertion_is_asked_for_again_later() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut policy = DisplaySleepPolicy::default();
        let mut power = FakePower {
            refuse: true,
            ..FakePower::default()
        };
        fullscreen_video(&mut controller, tab_id);
        run(&mut policy, &controller, &mut power);
        assert!(!policy.is_held());

        power.refuse = false;
        run(&mut policy, &controller, &mut power);
        policy.release(&mut power);

        assert_eq!(power.calls, ["prevent", "prevent", "allow"]);
    }
}
