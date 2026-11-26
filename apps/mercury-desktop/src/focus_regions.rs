// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! F6 and ⇧F6: the focus-region order (lane brief `lane-input-hooks-brief.md` D20; SPEC §5
//! "Group dock: F6 reaches it"; R12 §7 item 4).
//!
//! F6 moves the keyboard focus through the regions of the window in the order of [`Region`]:
//! address field, toolbar, tab strip, dock, page; ⇧F6 goes the other way. This module decides
//! *which key* and *which region*; the `FocusHost` (layout-switch) knows what is on screen and
//! how to focus it. Everything here takes plain values, so it is tested without a window.
//!
//! The hook in `keyboard.rs` passes F6 on unchanged when no host is registered, Orbit is open, a
//! sheet or popup has the keys, or an IME composition is open (D20); this module has no opinion on
//! those, the hook checks them before calling [`run`].

#![allow(dead_code)] // wired by the input-hooks `wire` item

use slint::winit_030::winit;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

use crate::input_hooks::{FocusHost, Region};

/// Whether this key event is an F6 press to act on: `Some(backwards)` for a real, non-repeat
/// key-down of F6 with no ⌘, ⌃ or ⌥ (⇧ means backwards), `None` for anything else.
pub(crate) fn f6_key(
    physical: &PhysicalKey,
    state: ElementState,
    repeat: bool,
    mods: ModifiersState,
    synthetic: bool,
) -> Option<bool> {
    let wanted = *physical == PhysicalKey::Code(KeyCode::F6)
        && state.is_pressed()
        && !repeat
        && !synthetic
        && !mods.super_key()
        && !mods.control_key()
        && !mods.alt_key();
    wanted.then_some(mods.shift_key())
}

/// [`f6_key`] for a winit `KeyEvent` (one cannot be built in a test).
pub(crate) fn f6_key_from_winit(
    event: &KeyEvent,
    mods: ModifiersState,
    synthetic: bool,
) -> Option<bool> {
    f6_key(
        &event.physical_key,
        event.state,
        event.repeat,
        mods,
        synthetic,
    )
}

/// The region F6 moves to. `available` is what the host reports; `current` is where the focus is.
///
/// Regions are visited in [`Region`] order and wrap around. When `current` is not in `available`
/// (a popup has the focus, or the dock just vanished under it) the move continues from where
/// `current` would sit in the order: the next available region after it going forward, the
/// previous one going backwards; with no `current` it goes to the first (forward) or the last
/// (backwards) region. A single available region is its own successor; an empty list gives `None`.
pub(crate) fn next_region(
    available: &[Region],
    current: Option<Region>,
    backwards: bool,
) -> Option<Region> {
    let listed = available.iter().copied();
    if backwards {
        let before = current.and_then(|current| listed.clone().filter(|r| *r < current).max());
        before.or_else(|| listed.max())
    } else {
        let after = current.and_then(|current| listed.clone().filter(|r| *r > current).min());
        after.or_else(|| listed.min())
    }
}

/// Moves the focus one region and returns whether it did (`false` only when the host reports no
/// region at all). A single region "moves" to itself: the key was handled.
pub(crate) fn run(host: &dyn FocusHost, backwards: bool) -> bool {
    match next_region(&host.regions(), host.current(), backwards) {
        Some(region) => {
            host.focus(region);
            true
        }
        None => false,
    }
}

/// What the hook must do with one key event, as [`F6Memory::event`] decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum F6Event {
    /// Not ours: the event goes on.
    Pass,
    /// The repeat or the up of an F6 press this memory saw consumed: consume it, do nothing.
    FollowUp,
    /// An F6 press: if the hook acts on it (a host is registered and nothing else has the keys) it
    /// calls [`run`] and then [`F6Memory::consumed`].
    Press { backwards: bool },
}

/// Remembers whether the last F6 down was consumed, so that its repeats and its up are consumed
/// too and the page never sees a key-up for a key-down it did not get.
#[derive(Debug, Default)]
pub(crate) struct F6Memory {
    consumed: bool,
}

impl F6Memory {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Classifies one key event. A new F6 press forgets the previous one.
    pub(crate) fn event(
        &mut self,
        physical: &PhysicalKey,
        state: ElementState,
        repeat: bool,
        mods: ModifiersState,
        synthetic: bool,
    ) -> F6Event {
        if synthetic || *physical != PhysicalKey::Code(KeyCode::F6) {
            return F6Event::Pass;
        }
        if state.is_pressed() && !repeat {
            self.consumed = false;
        }
        if let Some(backwards) = f6_key(physical, state, repeat, mods, synthetic) {
            return F6Event::Press { backwards };
        }
        if !state.is_pressed() {
            if std::mem::take(&mut self.consumed) {
                return F6Event::FollowUp;
            }
        } else if repeat && self.consumed {
            return F6Event::FollowUp;
        }
        F6Event::Pass
    }

    /// [`F6Memory::event`] for a winit `KeyEvent`.
    pub(crate) fn event_from_winit(
        &mut self,
        event: &KeyEvent,
        mods: ModifiersState,
        synthetic: bool,
    ) -> F6Event {
        self.event(
            &event.physical_key,
            event.state,
            event.repeat,
            mods,
            synthetic,
        )
    }

    /// The hook consumed the press that [`F6Memory::event`] just returned as [`F6Event::Press`].
    pub(crate) fn consumed(&mut self) {
        self.consumed = true;
    }

    /// Forget everything (focus lost, window closing, occlusion).
    pub(crate) fn reset(&mut self) {
        self.consumed = false;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use winit::keyboard::NativeKeyCode;

    use super::*;

    const ALL: [Region; 5] = [
        Region::Address,
        Region::Toolbar,
        Region::Tabs,
        Region::Dock,
        Region::Page,
    ];

    fn f6() -> PhysicalKey {
        PhysicalKey::Code(KeyCode::F6)
    }

    fn press(code: KeyCode, mods: ModifiersState) -> Option<bool> {
        f6_key(
            &PhysicalKey::Code(code),
            ElementState::Pressed,
            false,
            mods,
            false,
        )
    }

    /// The regions of a bit mask, in `Region` order (bit 0 is Address).
    fn subset(mask: u32) -> Vec<Region> {
        ALL.iter()
            .enumerate()
            .filter(|(bit, _)| mask & (1 << bit) != 0)
            .map(|(_, region)| *region)
            .collect()
    }

    /// The straight-line model: walk the five regions one step at a time, wrapping, until one is
    /// available; written without any of `next_region`'s comparisons.
    fn model(available: &[Region], current: Option<Region>, backwards: bool) -> Option<Region> {
        if available.is_empty() {
            return None;
        }
        let listed = |region: Region| available.contains(&region);
        let Some(current) = current else {
            let order: Vec<Region> = if backwards {
                ALL.iter().rev().copied().collect()
            } else {
                ALL.to_vec()
            };
            return order.into_iter().find(|region| listed(*region));
        };
        let start = ALL.iter().position(|region| *region == current).unwrap();
        for step in 1..=5i32 {
            let offset = if backwards { -step } else { step };
            let at = (start as i32 + offset).rem_euclid(5) as usize;
            if listed(ALL[at]) {
                return Some(ALL[at]);
            }
        }
        unreachable!("a non-empty list has an available region within five steps")
    }

    // ---- f6_key -------------------------------------------------------------------------

    #[test]
    fn f6_down_moves_forward_and_shift_f6_backwards() {
        assert_eq!(press(KeyCode::F6, ModifiersState::empty()), Some(false));
        assert_eq!(press(KeyCode::F6, ModifiersState::SHIFT), Some(true));
    }

    #[test]
    fn f6_with_command_control_or_option_is_not_ours_in_every_combination() {
        for bits in 0..16u32 {
            let shift = bits & 1 != 0;
            let command = bits & 2 != 0;
            let control = bits & 4 != 0;
            let alt = bits & 8 != 0;
            let mut mods = ModifiersState::empty();
            mods.set(ModifiersState::SHIFT, shift);
            mods.set(ModifiersState::SUPER, command);
            mods.set(ModifiersState::CONTROL, control);
            mods.set(ModifiersState::ALT, alt);
            let expected = if command || control || alt {
                None
            } else {
                Some(shift)
            };
            assert_eq!(press(KeyCode::F6, mods), expected, "{mods:?}");
        }
    }

    #[test]
    fn an_f6_key_up_is_not_a_press() {
        assert_eq!(
            f6_key(
                &f6(),
                ElementState::Released,
                false,
                ModifiersState::empty(),
                false
            ),
            None
        );
    }

    #[test]
    fn an_f6_repeat_is_not_a_press() {
        for mods in [ModifiersState::empty(), ModifiersState::SHIFT] {
            assert_eq!(
                f6_key(&f6(), ElementState::Pressed, true, mods, false),
                None
            );
        }
    }

    #[test]
    fn a_synthetic_f6_is_not_a_press() {
        for state in [ElementState::Pressed, ElementState::Released] {
            assert_eq!(
                f6_key(&f6(), state, false, ModifiersState::empty(), true),
                None
            );
        }
    }

    #[test]
    fn other_keys_are_not_f6() {
        for code in [
            KeyCode::F5,
            KeyCode::F7,
            KeyCode::F1,
            KeyCode::Tab,
            KeyCode::KeyA,
            KeyCode::Escape,
        ] {
            assert_eq!(press(code, ModifiersState::empty()), None, "{code:?}");
            assert_eq!(press(code, ModifiersState::SHIFT), None, "{code:?}");
        }
        assert_eq!(
            f6_key(
                &PhysicalKey::Unidentified(NativeKeyCode::Unidentified),
                ElementState::Pressed,
                false,
                ModifiersState::empty(),
                false
            ),
            None
        );
    }

    // ---- next_region --------------------------------------------------------------------

    #[test]
    fn next_region_matches_the_straight_line_model_for_every_subset_current_and_direction() {
        let mut checked = 0;
        for mask in 1..32u32 {
            let available = subset(mask);
            let mut currents: Vec<Option<Region>> = ALL.iter().map(|r| Some(*r)).collect();
            currents.push(None);
            assert_eq!(currents.len(), 6);
            for current in currents {
                for backwards in [false, true] {
                    assert_eq!(
                        next_region(&available, current, backwards),
                        model(&available, current, backwards),
                        "available {available:?}, current {current:?}, backwards {backwards}"
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 31 * 6 * 2);
    }

    #[test]
    fn the_model_itself_gives_the_expected_answers() {
        // a guard against the test and the code sharing one mistake
        let all = ALL.to_vec();
        assert_eq!(
            model(&all, Some(Region::Address), false),
            Some(Region::Toolbar)
        );
        assert_eq!(model(&all, Some(Region::Address), true), Some(Region::Page));
        assert_eq!(
            model(&all, Some(Region::Page), false),
            Some(Region::Address)
        );
        let no_tabs = vec![Region::Address, Region::Toolbar, Region::Dock, Region::Page];
        assert_eq!(
            model(&no_tabs, Some(Region::Toolbar), false),
            Some(Region::Dock)
        );
        assert_eq!(
            model(&no_tabs, Some(Region::Tabs), false),
            Some(Region::Dock)
        );
        assert_eq!(
            model(&no_tabs, Some(Region::Tabs), true),
            Some(Region::Toolbar)
        );
    }

    #[test]
    fn forward_wraps_from_the_last_region_to_the_first() {
        assert_eq!(
            next_region(&ALL, Some(Region::Page), false),
            Some(Region::Address)
        );
        let some = [Region::Toolbar, Region::Dock];
        assert_eq!(
            next_region(&some, Some(Region::Dock), false),
            Some(Region::Toolbar)
        );
    }

    #[test]
    fn backwards_wraps_from_the_first_region_to_the_last() {
        assert_eq!(
            next_region(&ALL, Some(Region::Address), true),
            Some(Region::Page)
        );
        let some = [Region::Toolbar, Region::Dock];
        assert_eq!(
            next_region(&some, Some(Region::Toolbar), true),
            Some(Region::Dock)
        );
    }

    #[test]
    fn unavailable_regions_are_skipped() {
        let no_tabs = [Region::Address, Region::Toolbar, Region::Dock, Region::Page];
        assert_eq!(
            next_region(&no_tabs, Some(Region::Toolbar), false),
            Some(Region::Dock)
        );
        assert_eq!(
            next_region(&no_tabs, Some(Region::Dock), true),
            Some(Region::Toolbar)
        );
    }

    #[test]
    fn a_single_region_is_its_own_successor_in_both_directions() {
        for region in ALL {
            let only = [region];
            assert_eq!(next_region(&only, Some(region), false), Some(region));
            assert_eq!(next_region(&only, Some(region), true), Some(region));
            assert_eq!(next_region(&only, None, false), Some(region));
            assert_eq!(next_region(&only, None, true), Some(region));
        }
    }

    #[test]
    fn an_empty_list_has_no_next_region() {
        for current in [None, Some(Region::Address), Some(Region::Page)] {
            for backwards in [false, true] {
                assert_eq!(next_region(&[], current, backwards), None);
            }
        }
    }

    #[test]
    fn with_no_current_region_forward_goes_to_the_first_and_backwards_to_the_last() {
        assert_eq!(next_region(&ALL, None, false), Some(Region::Address));
        assert_eq!(next_region(&ALL, None, true), Some(Region::Page));
        let some = [Region::Toolbar, Region::Tabs, Region::Dock];
        assert_eq!(next_region(&some, None, false), Some(Region::Toolbar));
        assert_eq!(next_region(&some, None, true), Some(Region::Dock));
    }

    #[test]
    fn a_current_region_that_is_not_listed_continues_from_its_place_in_the_order() {
        // the Dock was focused and has just gone (shrunk away, layout changed)
        let no_dock = [Region::Address, Region::Toolbar, Region::Tabs, Region::Page];
        assert_eq!(
            next_region(&no_dock, Some(Region::Dock), false),
            Some(Region::Page),
            "forward: the first available region after the Dock"
        );
        assert_eq!(
            next_region(&no_dock, Some(Region::Dock), true),
            Some(Region::Tabs),
            "backwards: the last available region before the Dock"
        );
        // past either end the move wraps
        let only_early = [Region::Address, Region::Toolbar];
        assert_eq!(
            next_region(&only_early, Some(Region::Page), false),
            Some(Region::Address)
        );
        let only_late = [Region::Dock, Region::Page];
        assert_eq!(
            next_region(&only_late, Some(Region::Address), true),
            Some(Region::Page)
        );
    }

    #[test]
    fn an_unsorted_or_repeated_list_gives_the_same_answer_as_a_sorted_one() {
        // the host promises Region order; the function does not depend on it
        let messy = [Region::Page, Region::Toolbar, Region::Page, Region::Address];
        let sorted = [Region::Address, Region::Toolbar, Region::Page];
        for current in [
            None,
            Some(Region::Address),
            Some(Region::Tabs),
            Some(Region::Page),
        ] {
            for backwards in [false, true] {
                assert_eq!(
                    next_region(&messy, current, backwards),
                    next_region(&sorted, current, backwards)
                );
            }
        }
    }

    // ---- run ----------------------------------------------------------------------------

    struct FakeFocus {
        regions: Vec<Region>,
        current: RefCell<Option<Region>>,
        focused: RefCell<Vec<Region>>,
    }

    impl FakeFocus {
        fn new(regions: &[Region], current: Option<Region>) -> Self {
            Self {
                regions: regions.to_vec(),
                current: RefCell::new(current),
                focused: RefCell::new(Vec::new()),
            }
        }
    }

    impl FocusHost for FakeFocus {
        fn regions(&self) -> Vec<Region> {
            self.regions.clone()
        }
        fn current(&self) -> Option<Region> {
            *self.current.borrow()
        }
        fn focus(&self, region: Region) {
            *self.current.borrow_mut() = Some(region);
            self.focused.borrow_mut().push(region);
        }
    }

    #[test]
    fn run_walks_through_all_five_regions_forward_and_wraps_in_ten_steps() {
        let host = FakeFocus::new(&ALL, Some(Region::Address));
        for _ in 0..10 {
            assert!(run(&host, false));
        }
        let expected: Vec<Region> = [
            Region::Toolbar,
            Region::Tabs,
            Region::Dock,
            Region::Page,
            Region::Address,
        ]
        .repeat(2);
        assert_eq!(*host.focused.borrow(), expected);
        assert_eq!(host.current(), Some(Region::Address));
    }

    #[test]
    fn run_walks_through_all_five_regions_backwards_and_wraps_in_ten_steps() {
        let host = FakeFocus::new(&ALL, Some(Region::Address));
        for _ in 0..10 {
            assert!(run(&host, true));
        }
        let expected: Vec<Region> = [
            Region::Page,
            Region::Dock,
            Region::Tabs,
            Region::Toolbar,
            Region::Address,
        ]
        .repeat(2);
        assert_eq!(*host.focused.borrow(), expected);
    }

    #[test]
    fn run_from_no_region_starts_at_the_first_or_the_last() {
        let host = FakeFocus::new(&ALL, None);
        assert!(run(&host, false));
        assert_eq!(*host.focused.borrow(), vec![Region::Address]);
        let host = FakeFocus::new(&ALL, None);
        assert!(run(&host, true));
        assert_eq!(*host.focused.borrow(), vec![Region::Page]);
    }

    #[test]
    fn run_with_one_region_focuses_it_again_and_says_it_moved() {
        let host = FakeFocus::new(&[Region::Address], Some(Region::Address));
        assert!(run(&host, false));
        assert_eq!(*host.focused.borrow(), vec![Region::Address]);
    }

    #[test]
    fn run_with_no_regions_does_nothing_and_says_so() {
        let host = FakeFocus::new(&[], Some(Region::Address));
        assert!(!run(&host, false));
        assert!(!run(&host, true));
        assert!(host.focused.borrow().is_empty());
    }

    #[test]
    fn run_skips_what_the_host_does_not_have() {
        let host = FakeFocus::new(
            &[Region::Address, Region::Toolbar, Region::Dock, Region::Page],
            Some(Region::Toolbar),
        );
        assert!(run(&host, false));
        assert!(run(&host, false));
        assert!(run(&host, true));
        assert_eq!(
            *host.focused.borrow(),
            vec![Region::Dock, Region::Page, Region::Dock]
        );
    }

    // ---- F6Memory -----------------------------------------------------------------------

    fn event(
        memory: &mut F6Memory,
        code: KeyCode,
        state: ElementState,
        repeat: bool,
        mods: ModifiersState,
    ) -> F6Event {
        memory.event(&PhysicalKey::Code(code), state, repeat, mods, false)
    }

    const NO_MODS: ModifiersState = ModifiersState::empty();

    #[test]
    fn an_f6_press_is_reported_with_its_direction() {
        let mut memory = F6Memory::new();
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Pressed,
                false,
                NO_MODS
            ),
            F6Event::Press { backwards: false }
        );
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Pressed,
                false,
                ModifiersState::SHIFT
            ),
            F6Event::Press { backwards: true }
        );
    }

    #[test]
    fn the_repeats_and_the_up_of_a_consumed_f6_are_consumed() {
        let mut memory = F6Memory::new();
        assert!(matches!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Pressed,
                false,
                NO_MODS
            ),
            F6Event::Press { .. }
        ));
        memory.consumed();
        for _ in 0..3 {
            assert_eq!(
                event(
                    &mut memory,
                    KeyCode::F6,
                    ElementState::Pressed,
                    true,
                    NO_MODS
                ),
                F6Event::FollowUp
            );
        }
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Released,
                false,
                NO_MODS
            ),
            F6Event::FollowUp
        );
        // the up spent the memory
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Released,
                false,
                NO_MODS
            ),
            F6Event::Pass
        );
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Pressed,
                true,
                NO_MODS
            ),
            F6Event::Pass
        );
    }

    #[test]
    fn the_up_of_an_f6_that_was_not_consumed_passes() {
        let mut memory = F6Memory::new();
        // a press the hook did not act on (no host, a popup): it never calls `consumed`
        assert!(matches!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Pressed,
                false,
                NO_MODS
            ),
            F6Event::Press { .. }
        ));
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Pressed,
                true,
                NO_MODS
            ),
            F6Event::Pass
        );
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Released,
                false,
                NO_MODS
            ),
            F6Event::Pass
        );
    }

    #[test]
    fn an_up_with_no_press_before_it_passes() {
        let mut memory = F6Memory::new();
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Released,
                false,
                NO_MODS
            ),
            F6Event::Pass
        );
    }

    #[test]
    fn a_new_press_forgets_the_previous_one() {
        let mut memory = F6Memory::new();
        event(
            &mut memory,
            KeyCode::F6,
            ElementState::Pressed,
            false,
            NO_MODS,
        );
        memory.consumed();
        // the up never came (focus moved); the next press the hook does not act on
        event(
            &mut memory,
            KeyCode::F6,
            ElementState::Pressed,
            false,
            NO_MODS,
        );
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Released,
                false,
                NO_MODS
            ),
            F6Event::Pass
        );
    }

    #[test]
    fn other_keys_do_not_disturb_the_memory() {
        let mut memory = F6Memory::new();
        event(
            &mut memory,
            KeyCode::F6,
            ElementState::Pressed,
            false,
            NO_MODS,
        );
        memory.consumed();
        for code in [KeyCode::F5, KeyCode::KeyA, KeyCode::Tab] {
            for state in [ElementState::Pressed, ElementState::Released] {
                for repeat in [false, true] {
                    assert_eq!(
                        event(&mut memory, code, state, repeat, NO_MODS),
                        F6Event::Pass,
                        "{code:?} {state:?} {repeat}"
                    );
                }
            }
        }
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Released,
                false,
                NO_MODS
            ),
            F6Event::FollowUp
        );
    }

    #[test]
    fn a_synthetic_f6_is_never_remembered_or_followed() {
        let mut memory = F6Memory::new();
        let replay = |memory: &mut F6Memory, state, repeat| {
            memory.event(&f6(), state, repeat, NO_MODS, true)
        };
        assert_eq!(
            replay(&mut memory, ElementState::Pressed, false),
            F6Event::Pass
        );
        memory.consumed();
        assert_eq!(
            replay(&mut memory, ElementState::Released, false),
            F6Event::Pass
        );
        // the real memory is untouched by the replay
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Released,
                false,
                NO_MODS
            ),
            F6Event::FollowUp
        );
    }

    #[test]
    fn f6_with_command_is_not_a_press_and_its_up_follows_only_a_consumed_press() {
        let mut memory = F6Memory::new();
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Pressed,
                false,
                ModifiersState::SUPER
            ),
            F6Event::Pass
        );
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Released,
                false,
                NO_MODS
            ),
            F6Event::Pass
        );
    }

    #[test]
    fn reset_forgets_a_consumed_press() {
        let mut memory = F6Memory::new();
        event(
            &mut memory,
            KeyCode::F6,
            ElementState::Pressed,
            false,
            NO_MODS,
        );
        memory.consumed();
        memory.reset();
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Pressed,
                true,
                NO_MODS
            ),
            F6Event::Pass
        );
        assert_eq!(
            event(
                &mut memory,
                KeyCode::F6,
                ElementState::Released,
                false,
                NO_MODS
            ),
            F6Event::Pass
        );
    }
}
