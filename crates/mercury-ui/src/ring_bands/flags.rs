// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Which marker a tab gets and where it sits (SPEC §3.2; ring-bands brief §3.2 "Flag rules").
//!
//! - One *state* flag per tab, by priority `capture` > `crashed` > `spinner` (loading) > `muted` >
//!   `audible` > `unread`, plus a `current` dot for the active tab.
//! - Every pin sits on the tab's mid-angle line, inside the band's inner edge (radii in the
//!   constants below, from the mockup `ring-open-light.svg`).
//! - A state flag is as wide as `opts.flag_size` allows, but never wider than the tab's arc at its
//!   own radius less 2 px. Under 8 px it is not drawn, except `capture` and `crashed`, which are
//!   never hidden and stay at 8 px.
//! - Held (⌥): no state flags; the first nine tabs of `selected_group` get a digit chip instead
//!   (their number is the tab's position in its group on the ring) and the current dot stays, as in
//!   the mockup `ring-keys`. When the current tab has a chip, chip and dot sit at the insets of a
//!   state flag and a dot (18 and 5 px) so that they do not touch.

use slint::SharedString;

use crate::ring_geometry::slint_models::RingModel;
use crate::ring_geometry::{TabArc, point_on_circle};
use crate::{FlagKind, FlagPin, TabRow};

use super::RingOptions;

/// Distance of a lone state flag's centre inside the band's inner edge.
const STATE_INSET: f32 = 14.0;
/// The same when the tab also has the current dot (so that the two do not touch).
const STATE_INSET_WITH_DOT: f32 = 18.0;
/// Distance of a lone current dot's centre inside the inner edge.
const DOT_INSET: f32 = 9.0;
/// The same when the tab also has a state flag.
const DOT_INSET_WITH_STATE: f32 = 5.0;
/// Diameter of the current dot, and the least it shrinks to on a very narrow arc.
const DOT_SIZE: f32 = 8.0;
const DOT_MIN_SIZE: f32 = 4.0;
/// Smallest drawn state flag; `capture` and `crashed` are drawn at this size when there is less room.
const MIN_FLAG_SIZE: f32 = 8.0;
/// Space kept free between a flag and the ends of its arc.
const ARC_MARGIN: f32 = 2.0;
/// Diameter of a digit chip while ⌥ is held, and how many tabs of the group get one.
const DIGIT_SIZE: f32 = 18.0;
const DIGIT_COUNT: usize = 9;

/// The pins of the shown `tabs` (ring order, parallel to `model`'s flat tab positions): `tabs[i]`
/// is the tab at flat position `i` of `model.layout.tabs`. Returns no pins when the lengths differ.
/// Order: by tab position, the state flag before the current dot.
pub(super) fn flag_pins(model: &RingModel, tabs: &[TabRow], opts: &RingOptions) -> Vec<FlagPin> {
    let layout = &model.layout;
    if layout.tabs.len() != tabs.len() {
        return Vec::new();
    }
    let spec = &layout.spec;
    let inner = spec.inner_radius();
    let mut pins = Vec::new();

    for (arc, tab) in layout.tabs.iter().zip(tabs) {
        let mid_deg = (arc.start_deg + arc.end_deg) / 2.0;
        let place = |inset: f32, wanted: f32| {
            let radius = (inner - inset).max(0.0);
            let size = wanted.min(arc_width(arc, radius) - ARC_MARGIN);
            (radius, size)
        };
        let pin = |kind: FlagKind, radius: f32, size: f32, progress: f32, label: SharedString| {
            let at = point_on_circle(spec.centre, radius, mid_deg);
            FlagPin {
                tab: tab.id,
                kind,
                x: at.x,
                y: at.y,
                size,
                progress,
                label,
            }
        };

        let mut with_state = false;
        if opts.held {
            let in_selected = opts.selected_group.is_some()
                && model.group_id(arc.group) == opts.selected_group
                && arc.index_in_group < DIGIT_COUNT;
            if in_selected {
                let inset = if tab.active {
                    STATE_INSET_WITH_DOT
                } else {
                    STATE_INSET
                };
                let (radius, size) = place(inset, DIGIT_SIZE);
                if size >= MIN_FLAG_SIZE {
                    let label = SharedString::from((arc.index_in_group + 1).to_string());
                    pins.push(pin(FlagKind::Digit, radius, size, -1.0, label));
                    with_state = true;
                }
            }
        } else if let Some(kind) = state_kind(tab) {
            let inset = if tab.active {
                STATE_INSET_WITH_DOT
            } else {
                STATE_INSET
            };
            let (radius, size) = place(inset, opts.flag_size);
            let never_hidden = matches!(kind, FlagKind::Capture | FlagKind::Crashed);
            let size = if size >= MIN_FLAG_SIZE {
                Some(size)
            } else {
                never_hidden.then_some(MIN_FLAG_SIZE)
            };
            if let Some(size) = size {
                let progress = if kind == FlagKind::Spinner && (0.0..=1.0).contains(&tab.progress) {
                    tab.progress
                } else {
                    -1.0
                };
                pins.push(pin(kind, radius, size, progress, SharedString::new()));
                with_state = true;
            }
        }
        if tab.active {
            let inset = if with_state {
                DOT_INSET_WITH_STATE
            } else {
                DOT_INSET
            };
            let radius = (inner - inset).max(0.0);
            let size = DOT_SIZE.min(arc_width(arc, radius)).max(DOT_MIN_SIZE);
            pins.push(pin(
                FlagKind::Current,
                radius,
                size,
                -1.0,
                SharedString::new(),
            ));
        }
    }
    pins
}

/// The tab's highest-priority state, if it has any.
fn state_kind(tab: &TabRow) -> Option<FlagKind> {
    if tab.capture {
        Some(FlagKind::Capture)
    } else if tab.crashed {
        Some(FlagKind::Crashed)
    } else if tab.loading {
        Some(FlagKind::Spinner)
    } else if tab.muted {
        Some(FlagKind::Muted)
    } else if tab.audible {
        Some(FlagKind::Audible)
    } else if tab.unread {
        Some(FlagKind::Unread)
    } else {
        None
    }
}

/// Length of the tab's drawn arc at `radius`.
fn arc_width(arc: &TabArc, radius: f32) -> f32 {
    (arc.end_deg - arc.start_deg).max(0.0).to_radians() * radius
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GroupRow;
    use crate::ring_bands::fixtures::{generated, mockup_groups, mockup_tabs};
    use crate::ring_geometry::{Point, RingSpec};

    const CENTRE: Point = Point::new(258.0, 258.0);
    const TOLERANCE: f32 = 0.05;

    fn model(groups: &[GroupRow], tabs: &[TabRow]) -> RingModel {
        RingModel::new(RingSpec::hub(CENTRE), groups, tabs)
    }

    fn pins_for(groups: &[GroupRow], tabs: &[TabRow], opts: &RingOptions) -> Vec<FlagPin> {
        flag_pins(&model(groups, tabs), tabs, opts)
    }

    fn mockup_pins() -> Vec<FlagPin> {
        pins_for(
            &mockup_groups(false),
            &mockup_tabs(false),
            &RingOptions::default(),
        )
    }

    fn pins_of(pins: &[FlagPin], tab: i32) -> Vec<&FlagPin> {
        pins.iter().filter(|pin| pin.tab == tab).collect()
    }

    fn state_of(pins: &[FlagPin], tab: i32) -> Vec<FlagKind> {
        pins_of(pins, tab)
            .into_iter()
            .map(|pin| pin.kind)
            .filter(|kind| *kind != FlagKind::Current)
            .collect()
    }

    /// Distance of `pin`'s centre from the ring's centre, and its distance from the radial line
    /// through the middle of the tab's arc (sideways, so a wrong angle shows as a number of px).
    fn radius_and_sideways(model: &RingModel, flat: usize, pin: &FlagPin) -> (f32, f32) {
        let arc = &model.layout.tabs[flat];
        let mid = ((arc.start_deg + arc.end_deg) / 2.0).to_radians();
        let (dx, dy) = (pin.x - CENTRE.x, pin.y - CENTRE.y);
        let along = dx * mid.sin() - dy * mid.cos();
        let sideways = dx * mid.cos() + dy * mid.sin();
        (along, sideways)
    }

    /// A one-group ring of one tab per entry, so every tab has room.
    fn single(tab_with: impl Fn(&mut TabRow)) -> (Vec<GroupRow>, Vec<TabRow>) {
        let (groups, mut tabs) = generated(1, 1, false);
        tabs[0].active = false;
        tab_with(&mut tabs[0]);
        (groups, tabs)
    }

    #[test]
    fn the_priority_table_holds_for_every_pair() {
        // Highest priority first.
        type Setter = fn(&mut TabRow);
        let order: [(FlagKind, Setter); 6] = [
            (FlagKind::Capture, |t| t.capture = true),
            (FlagKind::Crashed, |t| t.crashed = true),
            (FlagKind::Spinner, |t| t.loading = true),
            (FlagKind::Muted, |t| t.muted = true),
            (FlagKind::Audible, |t| t.audible = true),
            (FlagKind::Unread, |t| t.unread = true),
        ];
        for (i, (kind, set)) in order.iter().enumerate() {
            let (groups, tabs) = single(set);
            let pins = pins_for(&groups, &tabs, &RingOptions::default());
            assert_eq!(state_of(&pins, tabs[0].id), vec![*kind], "{kind:?} alone");
            for (_, lower) in &order[i + 1..] {
                let (groups, tabs) = single(|t| {
                    set(t);
                    lower(t);
                });
                let pins = pins_for(&groups, &tabs, &RingOptions::default());
                assert_eq!(
                    state_of(&pins, tabs[0].id),
                    vec![*kind],
                    "{kind:?} beats a lower state"
                );
            }
        }
        let (groups, tabs) = single(|t| {
            for (_, set) in &order {
                set(t);
            }
        });
        let pins = pins_for(&groups, &tabs, &RingOptions::default());
        assert_eq!(state_of(&pins, tabs[0].id), vec![FlagKind::Capture]);
        let (groups, tabs) = single(|_| {});
        assert!(pins_for(&groups, &tabs, &RingOptions::default()).is_empty());
    }

    #[test]
    fn the_mockup_has_its_flags_and_no_others() {
        let pins = mockup_pins();
        let kinds: Vec<(i32, FlagKind)> = pins.iter().map(|pin| (pin.tab, pin.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                (101, FlagKind::Current),
                (102, FlagKind::Spinner),
                (107, FlagKind::Unread),
                (115, FlagKind::Audible),
                (119, FlagKind::Unread),
            ]
        );
        let spinner = &pins[1];
        assert!((spinner.progress - 0.64).abs() < 1e-6);
        assert_eq!(spinner.size, 20.0);
        assert!(pins.iter().all(|pin| pin.label.is_empty()));
        assert!(pins[2..].iter().all(|pin| pin.progress == -1.0));
        assert_eq!(pins[0].size, 8.0);
    }

    #[test]
    fn a_spinner_without_a_known_progress_says_so() {
        for progress in [-0.5, 1.5, f32::NAN] {
            let (groups, mut tabs) = single(|t| t.loading = true);
            tabs[0].progress = progress;
            let pins = pins_for(&groups, &tabs, &RingOptions::default());
            assert_eq!(pins[0].kind, FlagKind::Spinner);
            assert_eq!(pins[0].progress, -1.0, "progress {progress}");
        }
        let (groups, mut tabs) = single(|t| t.loading = true);
        for progress in [0.0, 1.0] {
            tabs[0].progress = progress;
            let pins = pins_for(&groups, &tabs, &RingOptions::default());
            assert_eq!(pins[0].progress, progress);
        }
    }

    #[test]
    fn flags_sit_on_the_mid_angle_line_at_the_stated_radius() {
        let (groups, tabs) = (mockup_groups(false), mockup_tabs(false));
        let model = model(&groups, &tabs);
        let inner = model.layout.spec.inner_radius();
        assert_eq!(inner, 165.0);
        let pins = flag_pins(&model, &tabs, &RingOptions::default());
        assert_eq!(pins.len(), 5);
        for pin in &pins {
            let flat = tabs.iter().position(|t| t.id == pin.tab).expect("a tab");
            let inset = match pin.kind {
                FlagKind::Current => 9.0,
                _ => 14.0,
            };
            let (along, sideways) = radius_and_sideways(&model, flat, pin);
            assert!(
                (along - (inner - inset)).abs() < TOLERANCE,
                "{:?} of tab {} is {along} from the centre",
                pin.kind,
                pin.tab
            );
            assert!(
                sideways.abs() < TOLERANCE,
                "tab {} is {sideways} off",
                pin.tab
            );
        }
    }

    #[test]
    fn the_dot_and_a_state_flag_on_one_tab_sit_5_and_18_px_inside() {
        let (groups, mut tabs) = (mockup_groups(false), mockup_tabs(false));
        // Tab 101 is current; give it an unread state, and tab 102 (spinner) the current dot too.
        tabs[0].unread = true;
        tabs[1].active = true;
        let model = model(&groups, &tabs);
        let inner = model.layout.spec.inner_radius();
        let pins = flag_pins(&model, &tabs, &RingOptions::default());
        for (flat, id, state) in [(0, 101, FlagKind::Unread), (1, 102, FlagKind::Spinner)] {
            let own = pins_of(&pins, id);
            assert_eq!(own.len(), 2);
            assert_eq!(own[0].kind, state, "the state flag comes first");
            assert_eq!(own[1].kind, FlagKind::Current);
            let (badge, _) = radius_and_sideways(&model, flat, own[0]);
            let (dot, sideways) = radius_and_sideways(&model, flat, own[1]);
            assert!(
                (badge - (inner - 18.0)).abs() < TOLERANCE,
                "badge at {badge}"
            );
            assert!((dot - (inner - 5.0)).abs() < TOLERANCE, "dot at {dot}");
            assert!(sideways.abs() < TOLERANCE);
            // The mockup's numbers: centres 13 px apart, radii 10 and 4, so 1 px of the dot's
            // edge lies under the badge's border.
            let gap = (badge - dot).abs() - own[0].size / 2.0 - own[1].size / 2.0;
            assert!((gap + 1.0).abs() < TOLERANCE, "gap {gap}");
        }
    }

    #[test]
    fn the_flag_shrinks_with_the_arc_and_is_dropped_under_8_px() {
        // 5 groups of 12: every tab's arc leaves room for a flag, narrower than the full 20 px.
        let (groups, mut tabs) = generated(5, 60, false);
        for tab in &mut tabs {
            tab.unread = true;
            tab.active = false;
        }
        let model = model(&groups, &tabs);
        let inner = model.layout.spec.inner_radius();
        let pins = flag_pins(&model, &tabs, &RingOptions::default());
        assert_eq!(pins.len(), 60, "every flag of 60 tabs is drawn");
        for (arc, pin) in model.layout.tabs.iter().zip(&pins) {
            let width = (arc.end_deg - arc.start_deg).to_radians() * (inner - 14.0);
            let want = 20.0_f32.min(width - 2.0);
            assert!(want >= 8.0, "the 60 tab ring has room for flags");
            assert!(
                (pin.size - want).abs() < 1e-3,
                "{} against {want}",
                pin.size
            );
        }

        // 64 tabs in 64 groups: the arcs are far too narrow.
        let (groups, mut tabs) = generated(64, 64, false);
        for tab in &mut tabs {
            tab.unread = true;
            tab.active = false;
        }
        assert!(pins_for(&groups, &tabs, &RingOptions::default()).is_empty());
        for tab in &mut tabs {
            tab.unread = false;
            tab.capture = true;
        }
        let pins = pins_for(&groups, &tabs, &RingOptions::default());
        assert_eq!(pins.len(), 64);
        assert!(
            pins.iter()
                .all(|p| p.kind == FlagKind::Capture && p.size == 8.0)
        );
        for tab in &mut tabs {
            tab.capture = false;
            tab.crashed = true;
        }
        let pins = pins_for(&groups, &tabs, &RingOptions::default());
        assert_eq!(pins.len(), 64);
        assert!(
            pins.iter()
                .all(|p| p.kind == FlagKind::Crashed && p.size == 8.0)
        );
        for tab in &mut tabs {
            tab.crashed = false;
            tab.loading = true;
            tab.audible = true;
            tab.muted = true;
        }
        assert!(pins_for(&groups, &tabs, &RingOptions::default()).is_empty());
    }

    #[test]
    fn a_small_flag_size_is_a_floor_failure_too() {
        let (groups, tabs) = single(|t| t.unread = true);
        let small = RingOptions {
            flag_size: 6.0,
            ..RingOptions::default()
        };
        assert!(pins_for(&groups, &tabs, &small).is_empty());
        let (groups, tabs) = single(|t| t.capture = true);
        let pins = pins_for(&groups, &tabs, &small);
        assert_eq!((pins.len(), pins[0].size), (1, 8.0));
        let orbit = RingOptions {
            flag_size: 12.0,
            ..RingOptions::default()
        };
        let (groups, tabs) = single(|t| t.audible = true);
        assert_eq!(pins_for(&groups, &tabs, &orbit)[0].size, 12.0);
    }

    #[test]
    fn the_current_dot_shrinks_on_a_narrow_arc_but_is_never_under_4_px_and_always_drawn() {
        let (groups, tabs) = generated(64, 64, false);
        assert!(tabs[0].active);
        let model = model(&groups, &tabs);
        let pins = flag_pins(&model, &tabs, &RingOptions::default());
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].kind, FlagKind::Current);
        let arc = &model.layout.tabs[0];
        let radius = model.layout.spec.inner_radius() - 9.0;
        let width = (arc.end_deg - arc.start_deg).to_radians() * radius;
        assert!(width < 8.0, "the arc is narrower than the dot");
        assert_eq!(pins[0].size, width.max(4.0));
        // A wide ring keeps the full 8 px.
        assert_eq!(mockup_pins()[0].size, 8.0);
    }

    fn digits_of(pins: &[FlagPin]) -> Vec<&FlagPin> {
        pins.iter()
            .filter(|pin| pin.kind == FlagKind::Digit)
            .collect()
    }

    #[test]
    fn held_shows_digits_for_the_selected_group_and_the_current_dot() {
        let (groups, mut tabs) = (mockup_groups(false), mockup_tabs(false));
        tabs[0].unread = true;
        let held = |selected_group| RingOptions {
            held: true,
            selected_group,
            ..RingOptions::default()
        };
        let model = model(&groups, &tabs);
        let inner = model.layout.spec.inner_radius();

        // Reading (group 3) has six tabs, 110 to 115; the current tab (101) is in Research.
        let pins = flag_pins(&model, &tabs, &held(Some(3)));
        let digits: Vec<(i32, &str)> = digits_of(&pins)
            .iter()
            .map(|p| (p.tab, p.label.as_str()))
            .collect();
        assert_eq!(
            digits,
            vec![
                (110, "1"),
                (111, "2"),
                (112, "3"),
                (113, "4"),
                (114, "5"),
                (115, "6"),
            ]
        );
        for pin in digits_of(&pins) {
            assert_eq!(pin.size, 18.0);
            assert_eq!(pin.progress, -1.0);
            let flat = tabs.iter().position(|t| t.id == pin.tab).expect("a tab");
            let (along, sideways) = radius_and_sideways(&model, flat, pin);
            assert!(
                (along - (inner - 14.0)).abs() < TOLERANCE,
                "a lone chip is 14 px inside"
            );
            assert!(sideways.abs() < TOLERANCE);
        }
        // No state flag while held (tab 101 is unread, 102 is loading, 115 plays sound), and the
        // current tab keeps its dot, alone, at 9 px.
        assert_eq!(pins.len(), 7, "six chips and the current dot: {pins:?}");
        let dots = pins_of(&pins, 101);
        assert_eq!(dots.len(), 1);
        assert_eq!(dots[0].kind, FlagKind::Current);
        let (along, sideways) = radius_and_sideways(&model, 0, dots[0]);
        assert!((along - (inner - 9.0)).abs() < TOLERANCE);
        assert!(sideways.abs() < TOLERANCE);
    }

    #[test]
    fn held_with_the_current_tab_in_the_selected_group_puts_chip_and_dot_at_18_and_5() {
        let (groups, mut tabs) = (mockup_groups(false), mockup_tabs(false));
        tabs[0].unread = true;
        let opts = RingOptions {
            held: true,
            selected_group: Some(1),
            ..RingOptions::default()
        };
        let model = model(&groups, &tabs);
        let inner = model.layout.spec.inner_radius();
        let pins = flag_pins(&model, &tabs, &opts);
        // Research has five tabs: five chips and the dot; the unread state flag is not drawn.
        assert_eq!(pins.len(), 6, "{pins:?}");
        assert_eq!(digits_of(&pins).len(), 5);
        let own = pins_of(&pins, 101);
        assert_eq!(
            own.iter().map(|p| p.kind).collect::<Vec<_>>(),
            vec![FlagKind::Digit, FlagKind::Current],
            "the chip before the dot"
        );
        let (chip, _) = radius_and_sideways(&model, 0, own[0]);
        let (dot, _) = radius_and_sideways(&model, 0, own[1]);
        assert!((chip - (inner - 18.0)).abs() < TOLERANCE, "chip at {chip}");
        assert!((dot - (inner - 5.0)).abs() < TOLERANCE, "dot at {dot}");
        // The two do not touch: gap between the circles along the radial line.
        assert!((dot - own[1].size / 2.0) - (chip + own[0].size / 2.0) >= 0.0);
        // The other chips of the group are at 14.
        let (other, _) = radius_and_sideways(&model, 1, pins_of(&pins, 102)[0]);
        assert!((other - (inner - 14.0)).abs() < TOLERANCE);
    }

    #[test]
    fn held_without_a_selected_group_still_shows_the_current_dot() {
        let (groups, tabs) = (mockup_groups(false), mockup_tabs(false));
        let model = model(&groups, &tabs);
        for selected_group in [None, Some(99)] {
            let opts = RingOptions {
                held: true,
                selected_group,
                ..RingOptions::default()
            };
            let pins = flag_pins(&model, &tabs, &opts);
            assert_eq!(pins.len(), 1, "{selected_group:?}: {pins:?}");
            assert_eq!((pins[0].tab, pins[0].kind), (101, FlagKind::Current));
        }
    }

    #[test]
    fn held_numbers_at_most_nine_tabs() {
        let (groups, tabs) = generated(1, 14, false);
        let opts = RingOptions {
            held: true,
            selected_group: Some(1),
            ..RingOptions::default()
        };
        let pins = pins_for(&groups, &tabs, &opts);
        let labels: Vec<&str> = digits_of(&pins).iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["1", "2", "3", "4", "5", "6", "7", "8", "9"]);
    }

    #[test]
    fn held_digits_follow_the_position_in_the_group_when_ids_are_not_contiguous() {
        let (groups, mut tabs) = generated(1, 3, false);
        for (tab, index) in tabs.iter_mut().zip([4, 7, 9]) {
            tab.index_in_group = index;
        }
        let opts = RingOptions {
            held: true,
            selected_group: Some(1),
            ..RingOptions::default()
        };
        let pins = pins_for(&groups, &tabs, &opts);
        let labels: Vec<&str> = digits_of(&pins).iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["1", "2", "3"]);
    }

    #[test]
    fn mismatched_lengths_give_no_pins() {
        let (groups, tabs) = (mockup_groups(false), mockup_tabs(false));
        let model = model(&groups, &tabs);
        let opts = RingOptions::default();
        assert!(!flag_pins(&model, &tabs, &opts).is_empty());
        assert!(flag_pins(&model, &tabs[..21], &opts).is_empty());
        assert!(flag_pins(&model, &[], &opts).is_empty());
        let empty = RingModel::new(RingSpec::hub(CENTRE), &[], &[]);
        assert!(flag_pins(&empty, &[], &opts).is_empty());
        assert!(flag_pins(&empty, &tabs, &opts).is_empty());
    }

    #[test]
    fn pins_come_in_tab_order_with_the_state_before_the_dot() {
        let (groups, mut tabs) = (mockup_groups(false), mockup_tabs(false));
        tabs[0].capture = true;
        let pins = pins_for(&groups, &tabs, &RingOptions::default());
        let order: Vec<(i32, FlagKind)> = pins.iter().map(|p| (p.tab, p.kind)).collect();
        assert_eq!(order[0], (101, FlagKind::Capture));
        assert_eq!(order[1], (101, FlagKind::Current));
        assert!(order.windows(2).all(|w| w[0].0 <= w[1].0));
    }
}
