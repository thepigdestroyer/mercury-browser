// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The plain data the ⌥ state machine takes and gives (lane brief §5.1).
//!
//! **Frozen.** The lane lead wrote this file so that the items of the lane compile against the
//! same types. Nobody else edits it: a change goes through the integrator.

use std::time::Duration;

pub use mercury_appearance::LetterKey;
pub use mercury_settings::KeyCapture;
use mercury_settings::{DEFAULT_KEY_HOLD_MS, GroupKeySettings, MAX_KEY_HOLD_MS, MIN_KEY_HOLD_MS};

/// The most groups that can have a key: one per letter.
pub const MAX_KEYED_GROUPS: usize = 26;
/// The largest digit that picks a tab: `1` to `9`.
pub const MAX_DIGIT: u8 = 9;
/// The shortest hold of ⌥ before the card opens (`MIN_KEY_HOLD_MS`, 250 ms).
pub const MIN_HOLD: Duration = Duration::from_millis(MIN_KEY_HOLD_MS as u64);
/// The longest hold of ⌥ before the card opens (`MAX_KEY_HOLD_MS`, 1000 ms).
pub const MAX_HOLD: Duration = Duration::from_millis(MAX_KEY_HOLD_MS as u64);
/// The hold when the user has not chosen one (`DEFAULT_KEY_HOLD_MS`, 400 ms).
pub const DEFAULT_HOLD: Duration = Duration::from_millis(DEFAULT_KEY_HOLD_MS as u64);

/// Which modifier keys are down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    /// ⌥ (Option, Alt), either side.
    pub alt: bool,
    /// ⇧.
    pub shift: bool,
    /// ⌃.
    pub control: bool,
    /// ⌘ (Super).
    pub command: bool,
}

impl Mods {
    /// No modifier.
    pub const NONE: Mods = Mods {
        alt: false,
        shift: false,
        control: false,
        command: false,
    };
    /// ⌥ alone.
    pub const ALT: Mods = Mods {
        alt: true,
        ..Mods::NONE
    };

    /// Whether ⇧, ⌃ or ⌘ is down (⌥ is not counted).
    pub const fn any_other(self) -> bool {
        self.shift || self.control || self.command
    }

    /// Whether ⌥ is down and no other modifier is: the only state in which ⌥ keys are captured.
    pub const fn alt_only(self) -> bool {
        self.alt && !self.any_other()
    }
}

/// A physical key, as far as the machine cares. The desktop maps winit's `PhysicalKey` to it, so
/// the keyboard layout never changes which key is meant (⌥S types ß, and is still `KeyS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// `KeyA` to `KeyZ`.
    Letter(LetterKey),
    /// `Digit1` to `Digit9` and `Numpad1` to `Numpad9`. The adapter never sends 0; a value outside
    /// 1 to 9 behaves like [`Key::Other`].
    Digit(u8),
    /// Escape.
    Escape,
    /// ↑ ↓ ← →: never captured, because ⌥ with an arrow moves by word.
    Arrow,
    /// A modifier key itself (⌥, ⇧, ⌘, ⌃, Caps Lock, Fn): never captured, never changes the state
    /// (the modifier state arrives as [`Event::Modifiers`]).
    Modifier,
    /// Any other key. The number only has to tell two different keys apart.
    Other(u32),
}

/// What happened, as the hook sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// `WindowEvent::ModifiersChanged`: the new state.
    Modifiers(Mods),
    /// A real (not synthetic) key event, with the modifiers it had.
    Key {
        /// The physical key.
        key: Key,
        /// The modifiers at that moment.
        mods: Mods,
        /// Pressed (`true`) or released.
        down: bool,
        /// Auto-repeat of a held key.
        repeat: bool,
    },
    /// A pointer press outside the card's tiles and chips: its backdrop, or the page.
    PointerDown,
    /// A tile of the card was clicked, or pressed through VoiceOver.
    TileClicked {
        /// The group's id.
        group: u64,
    },
    /// A tab chip of the card was clicked or pressed.
    ChipClicked {
        /// The 0-based position of the tab in the chosen group.
        index: usize,
    },
    /// Close the card with no effect ("Change keys…", a window going away).
    Dismiss,
    /// Focus lost, window closed, tab or page replaced: forget everything transient.
    Reset,
}

/// The settings the machine follows (`appearance.tab_groups.keys`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Whether ⌥ group keys work at all.
    pub enabled: bool,
    /// When ⌥ and a letter is captured.
    pub capture: KeyCapture,
    /// How long ⌥ is held before the card opens: [`MIN_HOLD`] to [`MAX_HOLD`].
    pub hold: Duration,
}

impl Config {
    /// A configuration with `hold` clamped to [`MIN_HOLD`]..=[`MAX_HOLD`].
    pub fn new(enabled: bool, capture: KeyCapture, hold: Duration) -> Self {
        Self {
            enabled,
            capture,
            hold: hold.clamp(MIN_HOLD, MAX_HOLD),
        }
    }

    /// Whether the card can open on a hold: the keys are on and the capture mode has a hold path
    /// (`OutsideText` has none). It also needs at least one group with a key; the machine adds
    /// that.
    pub const fn hold_path(&self) -> bool {
        self.enabled && !matches!(self.capture, KeyCapture::OutsideText)
    }
}

impl Default for Config {
    /// On, `OutsideTextOrHold`, 400 ms.
    fn default() -> Self {
        Self::new(true, KeyCapture::default(), DEFAULT_HOLD)
    }
}

impl From<&GroupKeySettings> for Config {
    fn from(settings: &GroupKeySettings) -> Self {
        Self::new(
            settings.enabled,
            settings.capture,
            Duration::from_millis(u64::from(settings.hold_ms)),
        )
    }
}

/// Where the keyboard focus is, as far as ⌥ capture cares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Focus {
    /// A chrome text field has focus, or the page has an editable element focused.
    pub text_field: bool,
    /// An IME composition is open (the `txt-1` signal, injected: nothing is imported from it).
    pub composing: bool,
}

/// A group that has a key, as the machine sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyedGroup {
    /// The group's id (`GroupId::value()`, never above `i32::MAX`).
    pub id: u64,
    /// Its letter.
    pub key: LetterKey,
    /// How many tabs it has now (digits `1..=min(tab_count, 9)` pick one).
    pub tab_count: usize,
}

impl KeyedGroup {
    /// The keyed groups of a window, in the order of `entries` (ring order): the groups that have
    /// a key, with the tab count `tab_count` gives for each. `entries` is
    /// `mercury_appearance::KeyMap::entries()`.
    pub fn list(
        entries: &[(u64, Option<LetterKey>)],
        tab_count: impl Fn(u64) -> usize,
    ) -> Vec<KeyedGroup> {
        entries
            .iter()
            .filter_map(|&(id, key)| {
                key.map(|key| KeyedGroup {
                    id,
                    key,
                    tab_count: tab_count(id),
                })
            })
            .collect()
    }
}

/// Where to switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The group's most recently shown tab (`talaria-core`: `select_group`).
    Group(u64),
    /// The `index`-th tab of the group in strip order (0-based; digit `n` is index `n - 1`).
    Tab {
        /// The group's id.
        group: u64,
        /// The position in the group.
        index: usize,
    },
}

/// What happened to the card during a call, from [`CardView`] before and after.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CardChange {
    /// The card is as it was.
    #[default]
    None,
    /// It was hidden and is visible now.
    Show,
    /// It was visible and is visible, with another selection.
    Update,
    /// It was visible and is hidden now.
    Hide,
}

/// The answer to one call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// `true`: `PreventDefault`, and do not forward the key to Servo or Slint.
    pub consume: bool,
    /// Switch there now.
    pub activate: Option<Target>,
    /// What happened to the card.
    pub card: CardChange,
}

impl Outcome {
    /// Nothing consumed, nothing activated, the card unchanged: the answer to most calls.
    pub const PASS: Outcome = Outcome {
        consume: false,
        activate: None,
        card: CardChange::None,
    };
}

/// What the card, the dock and the ring show: the card's visibility is their "⌥ held".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CardView {
    /// The card is up.
    pub visible: bool,
    /// The chosen group (only while visible).
    pub selected_group: Option<u64>,
    /// The chosen tab as a digit, `1..=9` (only while visible).
    pub selected_tab: Option<u8>,
}

/// The machine's phase (for tests and diagnostics; the hook acts on [`Outcome`] and [`CardView`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// ⌥ is not down, or its press was ignored.
    Idle,
    /// ⌥ went down alone and nothing else has happened.
    Armed,
    /// This ⌥ hold belongs to the page or the text field; nothing is captured until ⌥ comes up.
    PassThrough,
    /// An immediate switch happened (no card); digits now pick a tab of that group.
    Switched,
    /// The card is shown.
    Overlay,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(alt: bool, shift: bool, control: bool, command: bool) -> Mods {
        Mods {
            alt,
            shift,
            control,
            command,
        }
    }

    #[test]
    fn alt_only_means_option_and_nothing_else() {
        assert!(Mods::ALT.alt_only());
        assert!(!Mods::NONE.alt_only());
        assert!(!mods(false, true, false, false).alt_only());
        for (shift, control, command) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
            (true, true, true),
        ] {
            let held = mods(true, shift, control, command);
            assert!(!held.alt_only(), "{held:?}");
            assert!(held.any_other(), "{held:?}");
        }
        assert!(!Mods::ALT.any_other());
        assert!(!Mods::NONE.any_other());
        assert_eq!(Mods::ALT, mods(true, false, false, false));
    }

    #[test]
    fn the_hold_is_clamped_to_the_settings_range() {
        let build = |millis| Config::new(true, KeyCapture::Always, Duration::from_millis(millis));
        assert_eq!(build(0).hold, MIN_HOLD);
        assert_eq!(build(100).hold, MIN_HOLD);
        assert_eq!(build(250).hold, Duration::from_millis(250));
        assert_eq!(build(400).hold, DEFAULT_HOLD);
        assert_eq!(build(1000).hold, Duration::from_millis(1000));
        assert_eq!(build(60_000).hold, MAX_HOLD);
        assert_eq!(MIN_HOLD, Duration::from_millis(250));
        assert_eq!(MAX_HOLD, Duration::from_millis(1000));
        assert_eq!(DEFAULT_HOLD, Duration::from_millis(400));
    }

    #[test]
    fn the_default_config_is_the_default_settings() {
        let config = Config::default();
        assert!(config.enabled);
        assert_eq!(config.capture, KeyCapture::OutsideTextOrHold);
        assert_eq!(config.hold, Duration::from_millis(400));
        assert_eq!(Config::from(&GroupKeySettings::default()), config);
    }

    #[test]
    fn a_config_follows_the_stored_settings() {
        let settings = GroupKeySettings {
            enabled: false,
            capture: KeyCapture::Always,
            hold_ms: 700,
            ..GroupKeySettings::default()
        };
        let config = Config::from(&settings);
        assert!(!config.enabled);
        assert_eq!(config.capture, KeyCapture::Always);
        assert_eq!(config.hold, Duration::from_millis(700));
        // A hold the settings file should never hold is still clamped here.
        let wild = GroupKeySettings {
            hold_ms: 5,
            ..GroupKeySettings::default()
        };
        assert_eq!(Config::from(&wild).hold, MIN_HOLD);
    }

    #[test]
    fn only_some_modes_have_a_hold_path() {
        let config = |enabled, capture| Config::new(enabled, capture, DEFAULT_HOLD);
        assert!(config(true, KeyCapture::OutsideTextOrHold).hold_path());
        assert!(config(true, KeyCapture::Always).hold_path());
        assert!(!config(true, KeyCapture::OutsideText).hold_path());
        assert!(!config(false, KeyCapture::OutsideTextOrHold).hold_path());
        assert!(!config(false, KeyCapture::Always).hold_path());
    }

    #[test]
    fn keyed_groups_are_the_groups_with_a_key_in_ring_order() {
        let entries = [
            (7, Some(LetterKey::A)),
            (3, None),
            (9, Some(LetterKey::S)),
            (4, Some(LetterKey::Z)),
        ];
        let list = KeyedGroup::list(&entries, |id| id as usize * 2);
        assert_eq!(
            list,
            vec![
                KeyedGroup {
                    id: 7,
                    key: LetterKey::A,
                    tab_count: 14
                },
                KeyedGroup {
                    id: 9,
                    key: LetterKey::S,
                    tab_count: 18
                },
                KeyedGroup {
                    id: 4,
                    key: LetterKey::Z,
                    tab_count: 8
                },
            ]
        );
        assert!(KeyedGroup::list(&[], |_| 1).is_empty());
        assert!(KeyedGroup::list(&[(1, None)], |_| 1).is_empty());
    }

    #[test]
    fn the_empty_answers_are_empty() {
        let pass = Outcome::PASS;
        assert!(!pass.consume);
        assert_eq!(pass.activate, None);
        assert_eq!(pass.card, CardChange::None);
        assert_eq!(Outcome::default(), pass);
        assert!(!CardView::default().visible);
        assert_eq!(CardView::default().selected_group, None);
        assert_eq!(CardView::default().selected_tab, None);
        assert_eq!(MAX_KEYED_GROUPS, 26);
        assert_eq!(MAX_DIGIT, 9);
    }
}
