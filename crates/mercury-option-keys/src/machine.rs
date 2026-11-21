// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The ⌥ state machine (lane brief §5.2 to §5.4; SPEC §5.1).
//!
//! One private [`State`] and one private function per kind of event. Nothing here reads a clock:
//! every call that can open the card is told the time, and the hold deadline is applied lazily at
//! the start of [`OptionKeys::on_event`] and [`OptionKeys::poll`], so a timer that fires late never
//! changes what a key does. The functions are total: no input panics.

use std::time::Duration;

use crate::types::{
    CardChange, CardView, Config, Event, Focus, Key, KeyCapture, KeyedGroup, LetterKey, MAX_DIGIT,
    MAX_KEYED_GROUPS, Mods, Outcome, Phase, Target,
};

/// Where a ⌥ hold stands (the phases of the brief's section 5.2, with their data).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// ⌥ is not down, or its press was ignored.
    Idle,
    /// ⌥ went down alone at this time and nothing else has happened.
    Armed(Duration),
    /// This hold belongs to the page; nothing is captured until ⌥ comes up.
    PassThrough,
    /// An immediate switch to this group happened; digits pick one of its tabs.
    Switched(u64),
    /// The card is up with this selection.
    Overlay { group: Option<u64>, tab: Option<u8> },
}

/// The state machine of the ⌥ group keys. See the brief's section 5.4 for every transition.
#[derive(Debug)]
pub struct OptionKeys {
    config: Config,
    /// The keyed groups in ring order: one per letter, the first group keeps a duplicated letter.
    groups: Vec<KeyedGroup>,
    focus: Focus,
    /// The modifiers as the machine last saw them (`Event::Modifiers`, or a key event's own).
    mods: Mods,
    state: State,
    /// The keys whose down was consumed and whose up has not come yet (the key-up memory).
    captured: Vec<Key>,
    /// The latest time seen: time that goes backwards counts as not having moved.
    clock: Duration,
}

impl OptionKeys {
    /// A machine in [`Phase::Idle`] with no groups and the focus outside any text field.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            groups: Vec::new(),
            focus: Focus::default(),
            mods: Mods::NONE,
            state: State::Idle,
            captured: Vec::new(),
            clock: Duration::ZERO,
        }
    }

    /// Follows a changed setting. Disabling the keys, or choosing a mode without a hold path,
    /// closes the card (`Hide`) and ends any capture.
    pub fn set_config(&mut self, config: Config) -> Outcome {
        let before = self.view();
        self.config = config;
        if !config.enabled {
            self.state = State::Idle;
        } else if matches!(self.state, State::Overlay { .. }) && !config.hold_path() {
            self.state = self.resting();
        }
        card_only(before, self.view())
    }

    /// Follows the groups that have keys, in ring order: at most 26, the first group keeps a
    /// duplicated letter. A selection that no longer exists is cleared (`Update`).
    pub fn set_groups(&mut self, groups: &[KeyedGroup]) -> Outcome {
        let before = self.view();
        let mut kept: Vec<KeyedGroup> = Vec::with_capacity(groups.len().min(MAX_KEYED_GROUPS));
        for group in groups {
            if kept.len() < MAX_KEYED_GROUPS && kept.iter().all(|k| k.key != group.key) {
                kept.push(*group);
            }
        }
        self.groups = kept;
        if let State::Overlay { group, tab } = self.state {
            self.state = self.overlay_checked(group, tab);
        }
        card_only(before, self.view())
    }

    /// Follows the keyboard focus. `composing` ends every kind of capture: Armed and Switched go
    /// to `PassThrough` and an open card is hidden (`Hide`), so an input method never loses a key.
    pub fn set_focus(&mut self, focus: Focus) -> Outcome {
        let before = self.view();
        self.focus = focus;
        if focus.composing
            && matches!(
                self.state,
                State::Armed(_) | State::Switched(_) | State::Overlay { .. }
            )
        {
            self.state = self.resting();
        }
        card_only(before, self.view())
    }

    /// Handles one event at `now`. The hold deadline is applied first, so a late timer never
    /// changes what a key does.
    pub fn on_event(&mut self, now: Duration, event: Event) -> Outcome {
        let before = self.view();
        let now = self.advance(now);
        let outcome = match event {
            Event::Modifiers(mods) => self.modifiers(now, mods),
            Event::Key {
                key,
                mods,
                down,
                repeat,
            } => self.key(now, key, mods, down, repeat),
            Event::PointerDown => self.pointer_down(),
            Event::TileClicked { group } => self.tile_clicked(group),
            Event::ChipClicked { index } => self.chip_clicked(index),
            Event::Dismiss => self.dismiss(),
            Event::Reset => self.reset(),
        };
        Outcome {
            card: change(before, self.view()),
            ..outcome
        }
    }

    /// The hold timer: opens the card when `now` has reached the deadline.
    pub fn poll(&mut self, now: Duration) -> Outcome {
        let before = self.view();
        self.advance(now);
        card_only(before, self.view())
    }

    /// When the card will open if nothing else happens: `Some(t0 + hold)` only in Armed with a
    /// hold path (the keys are on, the mode has one, and some group has a key).
    pub fn deadline(&self) -> Option<Duration> {
        match self.state {
            State::Armed(t0) if self.has_hold_path() => Some(t0.saturating_add(self.config.hold)),
            _ => None,
        }
    }

    /// What the card, the dock and the ring show now.
    pub fn view(&self) -> CardView {
        match self.state {
            State::Overlay { group, tab } => CardView {
                visible: true,
                selected_group: group,
                selected_tab: tab,
            },
            _ => CardView::default(),
        }
    }

    /// The current phase.
    pub fn phase(&self) -> Phase {
        match self.state {
            State::Idle => Phase::Idle,
            State::Armed(_) => Phase::Armed,
            State::PassThrough => Phase::PassThrough,
            State::Switched(_) => Phase::Switched,
            State::Overlay { .. } => Phase::Overlay,
        }
    }

    // --- shared helpers ---------------------------------------------------------------------

    /// Moves the clock forward (never back) and applies the hold deadline. Returns the time the
    /// rest of the call works at.
    fn advance(&mut self, now: Duration) -> Duration {
        self.clock = self.clock.max(now);
        if let Some(due) = self.deadline()
            && self.clock >= due
        {
            self.state = if self.focus.composing {
                State::PassThrough
            } else {
                State::Overlay {
                    group: None,
                    tab: None,
                }
            };
        }
        self.clock
    }

    /// The hold path: the keys are on, the mode has one, and some group has a key.
    fn has_hold_path(&self) -> bool {
        self.config.hold_path() && !self.groups.is_empty()
    }

    /// What a hold that stops being captured becomes: the page's while ⌥ is down, else Idle.
    fn resting(&self) -> State {
        if self.mods.alt {
            State::PassThrough
        } else {
            State::Idle
        }
    }

    /// The group that owns this letter.
    fn assigned(&self, letter: LetterKey) -> Option<u64> {
        self.groups.iter().find(|g| g.key == letter).map(|g| g.id)
    }

    /// How many tabs a group has, or `None` when the group is not keyed.
    fn tab_count(&self, id: u64) -> Option<usize> {
        self.groups.iter().find(|g| g.id == id).map(|g| g.tab_count)
    }

    /// An Overlay whose selection names a group that is gone, or a tab above the group's count,
    /// loses that part of the selection.
    fn overlay_checked(&self, group: Option<u64>, tab: Option<u8>) -> State {
        let group = group.filter(|&id| self.tab_count(id).is_some());
        let tab = tab.filter(|&n| {
            group
                .and_then(|id| self.tab_count(id))
                .is_some_and(|count| usize::from(n) <= count)
        });
        State::Overlay { group, tab }
    }

    /// Remembers a consumed key until its up.
    fn remember(&mut self, key: Key) {
        if !self.captured.contains(&key) {
            self.captured.push(key);
        }
    }

    /// Forgets a key at its up; whether it was remembered.
    fn forget(&mut self, key: Key) -> bool {
        match self.captured.iter().position(|k| *k == key) {
            Some(at) => {
                self.captured.swap_remove(at);
                true
            }
            None => false,
        }
    }

    // --- one function per kind of event -----------------------------------------------------

    /// `Event::Modifiers`: ⌥ edges and the other modifiers. Never consumed.
    fn modifiers(&mut self, now: Duration, mods: Mods) -> Outcome {
        let alt_rose = mods.alt && !self.mods.alt;
        self.mods = mods;
        if self.state == State::Idle {
            if alt_rose && mods.alt_only() && self.config.enabled {
                self.state = if self.focus.composing {
                    State::PassThrough
                } else {
                    State::Armed(now)
                };
            }
            return Outcome::PASS;
        }
        let mut outcome = Outcome::PASS;
        if !mods.alt {
            // ⌥ came up: it wins over anything else that changed in the same event.
            if let State::Overlay { group, tab } = self.state {
                outcome.activate = commit(group, tab);
            }
            self.state = State::Idle;
        } else if mods.any_other() {
            self.state = State::PassThrough;
        }
        outcome
    }

    /// `Event::Key`: the event's own modifiers first, then the key itself.
    fn key(&mut self, now: Duration, key: Key, mods: Mods, down: bool, repeat: bool) -> Outcome {
        if key == Key::Modifier {
            // The modifier state arrives as `Event::Modifiers`; the key of a modifier is nothing.
            return Outcome::PASS;
        }
        let mut outcome = if mods == self.mods {
            Outcome::PASS
        } else {
            self.modifiers(now, mods)
        };
        if !down {
            outcome.consume = self.forget(key);
            return outcome;
        }
        if repeat && self.captured.contains(&key) {
            outcome.consume = true;
            return outcome;
        }
        let step = match self.state {
            State::Idle | State::PassThrough => Outcome::PASS,
            State::Armed(_) => self.armed_key(key),
            State::Switched(group) => self.switched_key(group, key, repeat),
            State::Overlay { group, .. } => self.overlay_key(group, key, repeat),
        };
        if step.consume {
            self.remember(key);
        }
        outcome.consume = step.consume;
        outcome.activate = step.activate.or(outcome.activate);
        outcome
    }

    /// A key down while Armed: an assigned letter switches at once unless the page is typing;
    /// anything else makes this hold the page's.
    fn armed_key(&mut self, key: Key) -> Outcome {
        if let Key::Letter(letter) = key
            && let Some(group) = self.assigned(letter)
        {
            let typing = self.focus.composing
                || (self.focus.text_field && !matches!(self.config.capture, KeyCapture::Always));
            if !typing {
                self.state = State::Switched(group);
                return Outcome {
                    consume: true,
                    activate: Some(Target::Group(group)),
                    card: CardChange::None,
                };
            }
        }
        self.state = State::PassThrough;
        Outcome::PASS
    }

    /// A key down after an immediate switch to `current`: letters switch again, digits pick a
    /// tab of the current group.
    fn switched_key(&mut self, current: u64, key: Key, repeat: bool) -> Outcome {
        let mut outcome = Outcome::PASS;
        match key {
            Key::Letter(letter) => {
                let Some(group) = self.assigned(letter) else {
                    return outcome;
                };
                outcome.consume = true;
                if !repeat {
                    outcome.activate = Some(Target::Group(group));
                    self.state = State::Switched(group);
                }
            }
            Key::Digit(n) if is_tab_digit(n) => {
                outcome.consume = true;
                let tabs = self.tab_count(current).unwrap_or(0);
                if !repeat && usize::from(n) <= tabs {
                    outcome.activate = Some(Target::Tab {
                        group: current,
                        index: usize::from(n).saturating_sub(1),
                    });
                }
            }
            _ => {}
        }
        outcome
    }

    /// A key down while the card is up, `group` being the chosen one. The card owns every key but
    /// arrows and modifier keys, so that nothing typed behind it reaches the page.
    fn overlay_key(&mut self, group: Option<u64>, key: Key, repeat: bool) -> Outcome {
        let swallowed = Outcome {
            consume: true,
            ..Outcome::PASS
        };
        match key {
            Key::Arrow | Key::Modifier => Outcome::PASS,
            Key::Escape => {
                self.state = State::Idle;
                swallowed
            }
            Key::Letter(letter) => {
                if !repeat && let Some(chosen) = self.assigned(letter) {
                    self.state = State::Overlay {
                        group: Some(chosen),
                        tab: None,
                    };
                }
                swallowed
            }
            Key::Digit(n) if is_tab_digit(n) => {
                let fits = group
                    .and_then(|id| self.tab_count(id))
                    .is_some_and(|tabs| usize::from(n) <= tabs);
                if !repeat && fits {
                    self.state = State::Overlay {
                        group,
                        tab: Some(n),
                    };
                }
                swallowed
            }
            Key::Digit(_) | Key::Other(_) => swallowed,
        }
    }

    /// `Event::PointerDown`: a press outside the tiles and chips.
    fn pointer_down(&mut self) -> Outcome {
        match self.state {
            State::Armed(_) => self.state = State::PassThrough,
            State::Overlay { .. } => self.state = State::Idle,
            _ => {}
        }
        Outcome::PASS
    }

    /// `Event::TileClicked`: a click on a tile of the card (or a VoiceOver press) chooses it.
    fn tile_clicked(&mut self, group: u64) -> Outcome {
        if matches!(self.state, State::Overlay { .. }) && self.tab_count(group).is_some() {
            self.state = State::Idle;
            return Outcome {
                activate: Some(Target::Group(group)),
                ..Outcome::PASS
            };
        }
        Outcome::PASS
    }

    /// `Event::ChipClicked`: a click on a tab chip of the chosen group opens that tab.
    fn chip_clicked(&mut self, index: usize) -> Outcome {
        if let State::Overlay {
            group: Some(group), ..
        } = self.state
            && index < usize::from(MAX_DIGIT)
            && self.tab_count(group).is_some_and(|tabs| index < tabs)
        {
            self.state = State::Idle;
            return Outcome {
                activate: Some(Target::Tab { group, index }),
                ..Outcome::PASS
            };
        }
        Outcome::PASS
    }

    /// `Event::Dismiss`: close the card with no effect.
    fn dismiss(&mut self) -> Outcome {
        if matches!(self.state, State::Overlay { .. }) {
            self.state = State::Idle;
        }
        Outcome::PASS
    }

    /// `Event::Reset`: forget everything transient.
    fn reset(&mut self) -> Outcome {
        self.state = State::Idle;
        self.mods = Mods::NONE;
        self.captured.clear();
        Outcome::PASS
    }
}

/// Whether a digit key picks a tab (`1..=9`); any other value behaves as an unassigned key.
fn is_tab_digit(n: u8) -> bool {
    (1..=MAX_DIGIT).contains(&n)
}

/// What ⌥ coming up commits: the chosen tab, else the chosen group, else nothing.
fn commit(group: Option<u64>, tab: Option<u8>) -> Option<Target> {
    let group = group?;
    Some(match tab {
        Some(n) => Target::Tab {
            group,
            index: usize::from(n).saturating_sub(1),
        },
        None => Target::Group(group),
    })
}

/// How the card changed between two views.
fn change(before: CardView, after: CardView) -> CardChange {
    match (before.visible, after.visible) {
        (false, true) => CardChange::Show,
        (true, false) => CardChange::Hide,
        (true, true) if before != after => CardChange::Update,
        _ => CardChange::None,
    }
}

/// The answer of a call that only configures: nothing consumed or activated, the card as it moved.
fn card_only(before: CardView, after: CardView) -> Outcome {
    Outcome {
        card: change(before, after),
        ..Outcome::PASS
    }
}
