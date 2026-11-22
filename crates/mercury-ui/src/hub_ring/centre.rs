// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the centre disc shows (SPEC §4.3 table, the four `ring-*` mockups): a pure function from
//! the ring's state to the strings and the mode of the `HubRingBridge`. No window.

use super::TabMedia;
use crate::CentreMode;

/// The tab the centre is about (the hovered one, else the focused one, else the active one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CentreFocus {
    pub tab: i32,
    pub title: String,
    pub host: String,
    pub group_name: String,
    /// "D" or empty.
    pub key_label: String,
    /// 1-based among the shown tabs of its group; 0 for a pinned tab.
    pub index_in_group: usize,
    pub pinned: bool,
    pub media: Option<TabMedia>,
}

/// The group picked while ⌥ is held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldGroup {
    pub name: String,
    pub key_label: String,
    /// Shown tabs of the group; the help says "press 1–N" with N = min(9, this).
    pub tab_count: usize,
}

/// The inputs of [`view`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CentreInput {
    /// The search query has terms.
    pub query_active: bool,
    /// ⌥ is held (the key look).
    pub held: bool,
    /// The group chosen by its letter while held.
    pub held_group: Option<HeldGroup>,
    /// Every tab of the window, pinned included ("Search 25 tabs").
    pub total_tabs: usize,
    pub focus: Option<CentreFocus>,
    /// Matches beyond the two result rows.
    pub more_matches: usize,
    /// Pinned tabs that have no bead on the ring (reserve 0): the footer names them.
    pub pinned_collapsed: usize,
}

/// The centre's lines. `mode` is `Held` while held, else `Searching` while a query is active, else
/// `Audio` when the focused tab plays sound or is muted, else `Rest`.
#[derive(Clone, Debug, PartialEq)]
pub struct CentreView {
    pub mode: CentreMode,
    /// "Essay on maps"; empty when there is no tab.
    pub title: String,
    /// "Reading · ⌥D 2 · essays.example.net"; a loose tab "Loose · host"; a pinned tab "Pinned · host".
    pub subtitle: String,
    /// "Playing · 12:40 of 48:00", "Playing", "Paused" or "Muted" (audio mode).
    pub play_state: String,
    pub can_pause: bool,
    pub paused: bool,
    pub muted: bool,
    /// "Search 22 tabs" ("Search 1 tab").
    pub field_placeholder: String,
    /// "⌥ S" (held mode; "⌥" alone while no group is chosen).
    pub held_key: String,
    pub held_name: String,
    /// "press 1–4 for a tab" (the third line, "release ⌥ to switch", is drawn by Slint).
    pub held_help: String,
    /// "Hold ⌥ to see group keys", "↑ ↓ choose · esc clears", "+3 more lit in the ring",
    /// "2 pinned tabs".
    pub footer: String,
}

/// The centre's content for `input`.
///
/// The audio lines (`play_state`, `can_pause`, `paused`, `muted`) are filled only in the `Audio`
/// mode, the held lines only in the `Held` mode. The `Audio` mode keeps the rest footer.
pub fn view(input: &CentreInput) -> CentreView {
    let media = input
        .focus
        .as_ref()
        .and_then(|focus| focus.media)
        .filter(|media| media.playing || media.paused || media.muted);
    let mode = if input.held {
        CentreMode::Held
    } else if input.query_active {
        CentreMode::Searching
    } else if media.is_some() {
        CentreMode::Audio
    } else {
        CentreMode::Rest
    };

    let (title, subtitle) = input
        .focus
        .as_ref()
        .map(|focus| (focus.title.clone(), subtitle(focus)))
        .unwrap_or_default();

    let mut view = CentreView {
        mode,
        title,
        subtitle,
        play_state: String::new(),
        can_pause: false,
        paused: false,
        muted: false,
        field_placeholder: placeholder(input.total_tabs),
        held_key: String::new(),
        held_name: String::new(),
        held_help: String::new(),
        footer: footer(mode, input),
    };
    if let (CentreMode::Audio, Some(media)) = (mode, media) {
        view.play_state = play_state(&media);
        view.can_pause = media.can_pause;
        view.paused = media.paused;
        view.muted = media.muted;
    }
    if mode == CentreMode::Held {
        match &input.held_group {
            Some(group) => {
                view.held_key = if group.key_label.is_empty() {
                    "⌥".to_owned()
                } else {
                    format!("⌥ {}", group.key_label)
                };
                view.held_name = group.name.clone();
                view.held_help = match group.tab_count.min(9) {
                    0 => String::new(),
                    1 => "press 1 for a tab".to_owned(),
                    n => format!("press 1\u{2013}{n} for a tab"),
                };
            }
            None => {
                view.held_key = "⌥".to_owned();
                view.held_help = "press a group key".to_owned();
            }
        }
    }
    view
}

/// "Reading · ⌥D 2 · essays.example.net"; without a key "Loose · host"; pinned "Pinned · host".
fn subtitle(focus: &CentreFocus) -> String {
    let place = if focus.pinned {
        "Pinned".to_owned()
    } else if focus.key_label.is_empty() {
        focus.group_name.clone()
    } else {
        format!(
            "{} · ⌥{} {}",
            focus.group_name, focus.key_label, focus.index_in_group
        )
    };
    [place.as_str(), focus.host.as_str()]
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// "Search 22 tabs", "Search 1 tab", "Search tabs" for an empty window.
fn placeholder(total_tabs: usize) -> String {
    match total_tabs {
        0 => "Search tabs".to_owned(),
        1 => "Search 1 tab".to_owned(),
        n => format!("Search {n} tabs"),
    }
}

/// Muted wins, then paused, then playing (with the position when the media session has one).
fn play_state(media: &TabMedia) -> String {
    if media.muted {
        "Muted".to_owned()
    } else if media.paused {
        "Paused".to_owned()
    } else {
        match media.position {
            Some((position, 0)) => format!("Playing · {}", clock(position)),
            Some((position, duration)) => {
                format!("Playing · {} of {}", clock(position), clock(duration))
            }
            None => "Playing".to_owned(),
        }
    }
}

/// `m:ss`, or `h:mm:ss` from one hour on.
fn clock(seconds: u32) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn footer(mode: CentreMode, input: &CentreInput) -> String {
    match mode {
        CentreMode::Held => String::new(),
        CentreMode::Searching if input.more_matches > 0 => {
            format!("+{} more lit in the ring", input.more_matches)
        }
        CentreMode::Searching => "↑ ↓ choose · esc clears".to_owned(),
        _ => match input.pinned_collapsed {
            0 => "Hold ⌥ to see group keys".to_owned(),
            1 => "1 pinned tab".to_owned(),
            n => format!("{n} pinned tabs"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_has_hours_only_from_an_hour() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(5), "0:05");
        assert_eq!(clock(760), "12:40");
        assert_eq!(clock(3599), "59:59");
        assert_eq!(clock(3600), "1:00:00");
        assert_eq!(clock(36_000 + 61), "10:01:01");
    }

    #[test]
    fn an_unknown_duration_shows_the_position_alone() {
        let media = TabMedia {
            playing: true,
            position: Some((65, 0)),
            ..TabMedia::default()
        };
        assert_eq!(play_state(&media), "Playing · 1:05");
    }

    #[test]
    fn a_group_without_tabs_has_no_digit_help() {
        let input = CentreInput {
            held: true,
            held_group: Some(HeldGroup {
                name: "Empty".to_owned(),
                key_label: "Z".to_owned(),
                tab_count: 0,
            }),
            ..CentreInput::default()
        };
        assert_eq!(view(&input).held_help, "");
    }

    #[test]
    fn a_group_without_a_key_shows_the_bare_modifier() {
        let input = CentreInput {
            held: true,
            held_group: Some(HeldGroup {
                name: "Loose".to_owned(),
                key_label: String::new(),
                tab_count: 3,
            }),
            ..CentreInput::default()
        };
        let view = view(&input);
        assert_eq!(view.held_key, "⌥");
        assert_eq!(view.held_name, "Loose");
        assert_eq!(view.held_help, "press 1\u{2013}3 for a tab");
    }

    #[test]
    fn a_subtitle_leaves_out_an_empty_host() {
        let focus = CentreFocus {
            tab: 1,
            title: "New tab".to_owned(),
            host: String::new(),
            group_name: "Reading".to_owned(),
            key_label: "D".to_owned(),
            index_in_group: 2,
            pinned: false,
            media: None,
        };
        assert_eq!(subtitle(&focus), "Reading · ⌥D 2");
    }
}
