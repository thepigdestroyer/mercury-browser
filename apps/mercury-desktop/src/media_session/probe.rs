// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! When to look into a page's document for what its media is doing.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use talaria_core::{BrowserController, TabId};

/// While a tab plays, how often its document is probed, so that a page un-muting a video
/// itself (no engine event reports that) or entering fullscreen is noticed within this time.
pub(crate) const PROBE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy)]
struct Seen {
    playing: bool,
    fullscreen: bool,
    last_probe: Option<Instant>,
}

#[derive(Debug, Default)]
pub(crate) struct ProbeSchedule {
    tabs: HashMap<TabId, Seen>,
}

impl ProbeSchedule {
    /// The playing tabs to probe now: those that just started playing or changed fullscreen,
    /// and those not probed for `PROBE_INTERVAL`. Tabs that do not play are never probed.
    pub(crate) fn due(&mut self, controller: &BrowserController, now: Instant) -> Vec<TabId> {
        self.tabs
            .retain(|tab_id, _| controller.tabs().any(|tab| tab.id() == *tab_id));
        let mut due = Vec::new();
        for tab in controller.tabs() {
            let playing = tab.state().media().is_playing();
            let fullscreen = tab.state().is_fullscreen();
            let seen = self.tabs.entry(tab.id()).or_insert(Seen {
                playing: false,
                fullscreen,
                last_probe: None,
            });
            let probe = playing
                && (!seen.playing
                    || seen.fullscreen != fullscreen
                    || seen
                        .last_probe
                        .is_none_or(|last| now.duration_since(last) >= PROBE_INTERVAL));
            seen.playing = playing;
            seen.fullscreen = fullscreen;
            if probe {
                seen.last_probe = Some(now);
                due.push(tab.id());
            }
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use talaria_core::{BrowserEvent, MediaPlaybackState};

    use super::*;

    fn play(controller: &mut BrowserController, tab_id: TabId, state: MediaPlaybackState) {
        controller.handle_event(tab_id, BrowserEvent::MediaPlaybackChanged { state });
    }

    #[test]
    fn a_tab_is_probed_when_it_starts_playing_then_once_per_interval() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut schedule = ProbeSchedule::default();
        let start = Instant::now();

        assert!(schedule.due(&controller, start).is_empty());
        play(&mut controller, tab_id, MediaPlaybackState::Playing);
        assert_eq!(schedule.due(&controller, start), [tab_id]);
        assert!(
            schedule
                .due(&controller, start + Duration::from_millis(500))
                .is_empty()
        );
        assert_eq!(schedule.due(&controller, start + PROBE_INTERVAL), [tab_id]);
    }

    #[test]
    fn a_paused_tab_is_not_probed_and_is_probed_again_when_it_resumes() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut schedule = ProbeSchedule::default();
        let start = Instant::now();
        play(&mut controller, tab_id, MediaPlaybackState::Playing);
        schedule.due(&controller, start);

        play(&mut controller, tab_id, MediaPlaybackState::Paused);
        assert!(
            schedule
                .due(&controller, start + PROBE_INTERVAL * 3)
                .is_empty()
        );

        play(&mut controller, tab_id, MediaPlaybackState::Playing);
        let resumed = start + PROBE_INTERVAL * 3 + Duration::from_millis(10);
        assert_eq!(schedule.due(&controller, resumed), [tab_id]);
    }

    #[test]
    fn entering_or_leaving_fullscreen_while_playing_probes_at_once() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        let mut schedule = ProbeSchedule::default();
        let start = Instant::now();
        play(&mut controller, tab_id, MediaPlaybackState::Playing);
        schedule.due(&controller, start);

        controller.handle_event(tab_id, BrowserEvent::FullscreenChanged { fullscreen: true });
        assert_eq!(
            schedule.due(&controller, start + Duration::from_millis(1)),
            [tab_id]
        );
        controller.handle_event(
            tab_id,
            BrowserEvent::FullscreenChanged { fullscreen: false },
        );
        assert_eq!(
            schedule.due(&controller, start + Duration::from_millis(2)),
            [tab_id]
        );
    }

    #[test]
    fn only_playing_tabs_are_probed() {
        let mut controller = BrowserController::default();
        let second = controller.open_tab();
        let mut schedule = ProbeSchedule::default();
        play(&mut controller, second, MediaPlaybackState::Playing);

        assert_eq!(schedule.due(&controller, Instant::now()), [second]);
    }
}
