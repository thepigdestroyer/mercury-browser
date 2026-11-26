// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! How the dock's pills are fitted into the window (brief D8: five tiers, applied only as far as
//! needed, with room left for the magnification). Private to `group_dock`; `data.rs` measures the
//! text and calls [`choose`], then draws what the plans say. Pure: no Slint types.
//!
//! All numbers here are unscaled (scale 1.0) pill-local pixels. A [`Plan`] is one pill's inside:
//! the x and width of its chips or slivers and the pill's width.
//!
//! # The tiers (D8)
//!
//! | tier | current group | folded pills | gap |
//! | --- | --- | --- | --- |
//! | 1 | chips at their natural width (title + 36, 64..88) | name and slivers of 6 px | 10 |
//! | 2 | chips 64 px | name and slivers of 4 px | 10 |
//! | 3 | as 2 | colour dot and count, no name, no slivers | 4 |
//! | 4 | as 2 | colour dot only, 14 px wide | 2 |
//! | 5 | collapsed to slivers of 4 px, no name | as 4 | 2 |
//!
//! The first tier whose **worst case** fits wins; if none does, tier 5 is used and the bar is wider
//! than the container (nothing here scrolls). The worst case is the largest bar width over every
//! pointer position: the pill under the pointer is 1.45 times as wide, its two neighbours 1.12
//! times, the rest 1.0, and the bar must still be at most `container - 2 * HEADROOM` wide.

use crate::ring_geometry::dock::{NEIGHBOUR, UNDER_POINTER};

/// Space left on each side of the bar while it is magnified.
pub(super) const HEADROOM: f32 = 20.0;
/// Space between the bar's edge and its first and last entry.
pub(super) const BAR_PAD_X: f32 = 10.0;
/// The "+" and the x are 26 px wide (a touch target of at least 24).
pub(super) const BUTTON: f32 = 26.0;
/// Space between the trailing "+" and the x.
pub(super) const BUTTON_GAP: f32 = 4.0;
/// The trailing block: the "+" and the x, one entry of the resting layout.
pub(super) const TRAILING_WIDTH: f32 = BUTTON + BUTTON_GAP + BUTTON;
/// The text size of a group name, and its weight.
pub(super) const NAME_SIZE: f32 = 13.0;
pub(super) const NAME_WEIGHT: u16 = 600;
/// The text size of a key badge, and its weight.
pub(super) const KEY_SIZE: f32 = 11.0;
pub(super) const KEY_WEIGHT: u16 = 600;
/// The text size of a chip's title, and its weight.
pub(super) const CHIP_SIZE: f32 = 12.5;
pub(super) const CHIP_WEIGHT: u16 = 400;
/// The text size of the count of a tier 3 pill, and its weight.
pub(super) const COUNT_SIZE: f32 = 11.0;
pub(super) const COUNT_WEIGHT: u16 = 600;
/// A name is never reserved more than this (the dock elides the rest).
pub(super) const NAME_MAX: f32 = 120.0;

const PILL_PAD_X: f32 = 12.0;
const BADGE_GAP: f32 = 6.0;
const BADGE_PAD: f32 = 10.0;
const BADGE_MIN: f32 = 16.0;
const BODY_GAP: f32 = 8.0;
const CHIP_GAP: f32 = 2.0;
const PLUS_PAD_R: f32 = 4.0;
const CHIP_PAD: f32 = 36.0;
/// The narrowest and the widest a chip gets at tier 1; the width of every chip from tier 2.
pub(super) const CHIP_MIN: f32 = 64.0;
pub(super) const CHIP_MAX: f32 = 88.0;
/// The widest the active chip gets (it carries its x and is the one whose title matters).
pub(super) const CHIP_MAX_ACTIVE: f32 = 120.0;
/// The room the active chip's x takes.
const CHIP_CLOSE_ROOM: f32 = 16.0;
/// At tier 2 every chip is `CHIP_MIN` wide but the active one, which gets at most this.
const CHIP_ACTIVE_TIER_TWO: f32 = 104.0;
const DOT: f32 = 8.0;
const DOT_PAD: f32 = 10.0;
const DOT_GAP: f32 = 5.0;
const DOT_ONLY_WIDTH: f32 = 14.0;
const COLLAPSED_PAD: f32 = 6.0;
/// The narrowest a sliver of a folded pill gets, and the gap then.
const MIN_SLIVER: f32 = 2.0;
const TIGHT_GAP: f32 = 1.0;

/// The last tier.
pub(super) const LAST_TIER: u8 = 5;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Folded {
    Named,
    Count,
    Dot,
}

#[derive(Clone, Copy, Debug)]
struct Spec {
    gap: f32,
    chips_fixed: bool,
    sliver_width: f32,
    sliver_gap: f32,
    sliver_area_cap: f32,
    folded: Folded,
    collapse_current: bool,
}

fn spec(tier: u8) -> Spec {
    let named = |tier_two: bool| Spec {
        gap: 10.0,
        chips_fixed: tier_two,
        sliver_width: if tier_two { 4.0 } else { 6.0 },
        sliver_gap: if tier_two { 1.0 } else { 2.0 },
        sliver_area_cap: if tier_two { 72.0 } else { 96.0 },
        folded: Folded::Named,
        collapse_current: false,
    };
    match tier {
        0 | 1 => named(false),
        2 => named(true),
        3 => Spec {
            gap: 4.0,
            folded: Folded::Count,
            ..named(true)
        },
        4 => Spec {
            gap: 2.0,
            folded: Folded::Dot,
            ..named(true)
        },
        _ => Spec {
            gap: 2.0,
            folded: Folded::Dot,
            collapse_current: true,
            ..named(true)
        },
    }
}

/// What the text measurement of one group gave.
#[derive(Clone, Debug, Default)]
pub(super) struct Measure {
    /// The advance of the name, at most [`NAME_MAX`].
    pub name: f32,
    /// The width of the key badge; `None` for a group with an empty key label.
    pub badge: Option<f32>,
    /// The advance of the count text.
    pub count: f32,
    /// The natural width of each tab's chip: title advance + 36, clamped to 64..88; the active
    /// chip also has its x to make room for (+ 16) and may be up to [`CHIP_MAX_ACTIVE`] wide.
    pub chips: Vec<f32>,
    /// The index of the active tab's chip, if the group has one.
    pub active_chip: Option<usize>,
}

/// The width of a key badge for a key text `advance` wide.
pub(super) fn badge_width(advance: f32) -> f32 {
    (advance + BADGE_PAD).max(BADGE_MIN)
}

/// The natural width of a chip for a title `advance` wide. The active chip shows its x always and
/// gets more room (up to [`CHIP_MAX_ACTIVE`]) so that its title stays readable.
pub(super) fn chip_natural(advance: f32, active: bool) -> f32 {
    if active {
        (advance + CHIP_PAD + CHIP_CLOSE_ROOM).clamp(CHIP_MIN, CHIP_MAX_ACTIVE)
    } else {
        (advance + CHIP_PAD).clamp(CHIP_MIN, CHIP_MAX)
    }
}

/// One chip or sliver inside a pill: the index of its tab in the group, x and width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Span {
    pub index: usize,
    pub x: f32,
    pub width: f32,
}

/// The inside of one pill.
#[derive(Clone, Debug, Default)]
pub(super) struct Plan {
    pub width: f32,
    /// The current group, drawn as name and chips (and the pill's own "+" at its right end).
    pub expanded: bool,
    pub show_key: bool,
    pub chips: Vec<Span>,
    pub slivers: Vec<Span>,
}

/// The widths of the pill's "+" at the right end of an expanded pill, and the space after it.
pub(super) const PLUS_RIGHT_INSET: f32 = PLUS_PAD_R;

/// The slivers of `n` tabs: width, gap and the area they take.
fn sliver_run(n: usize, spec: &Spec) -> (f32, f32, f32) {
    if n == 0 {
        return (spec.sliver_width, spec.sliver_gap, 0.0);
    }
    let n = n as f32;
    let natural = n * spec.sliver_width + (n - 1.0) * spec.sliver_gap;
    if natural <= spec.sliver_area_cap {
        return (spec.sliver_width, spec.sliver_gap, natural);
    }
    let width =
        ((spec.sliver_area_cap - (n - 1.0) * TIGHT_GAP) / n).clamp(MIN_SLIVER, spec.sliver_width);
    (width, TIGHT_GAP, n * width + (n - 1.0) * TIGHT_GAP)
}

fn sliver_spans(n: usize, start: f32, width: f32, gap: f32) -> Vec<Span> {
    (0..n)
        .map(|index| Span {
            index,
            x: start + index as f32 * (width + gap),
            width,
        })
        .collect()
}

/// One pill's inside at `tier`. `badge` asks for the key badge (ignored without a key label and
/// from tier 3, where there is no name to put it after).
pub(super) fn plan(measure: &Measure, tier: u8, current: bool, badge: bool) -> Plan {
    let spec = spec(tier);
    let n = measure.chips.len();
    let named = if current {
        !spec.collapse_current
    } else {
        spec.folded == Folded::Named
    };
    let badge_width = measure.badge.filter(|_| badge && named);
    let mut x = PILL_PAD_X + measure.name;
    if let Some(badge_width) = badge_width {
        x += BADGE_GAP + badge_width;
    }
    if current && !spec.collapse_current {
        x += BODY_GAP;
        let mut chips = Vec::with_capacity(n);
        for (index, natural) in measure.chips.iter().enumerate() {
            let width = if !spec.chips_fixed {
                *natural
            } else if measure.active_chip == Some(index) {
                natural.clamp(CHIP_MIN, CHIP_ACTIVE_TIER_TWO)
            } else {
                CHIP_MIN
            };
            chips.push(Span { index, x, width });
            x += width + CHIP_GAP;
        }
        return Plan {
            width: x + BUTTON + PLUS_PAD_R,
            expanded: true,
            show_key: badge_width.is_some(),
            chips,
            slivers: Vec::new(),
        };
    }
    if current {
        let (width, gap, area) = sliver_run(n, &spec);
        return Plan {
            width: (COLLAPSED_PAD * 2.0 + area).max(DOT_ONLY_WIDTH),
            expanded: false,
            show_key: false,
            chips: Vec::new(),
            slivers: sliver_spans(n, COLLAPSED_PAD, width, gap),
        };
    }
    match spec.folded {
        Folded::Named => {
            let (width, gap, area) = sliver_run(n, &spec);
            let total = x + BODY_GAP + area + PILL_PAD_X;
            Plan {
                width: total,
                expanded: false,
                show_key: badge_width.is_some(),
                chips: Vec::new(),
                slivers: sliver_spans(n, total - PILL_PAD_X - area, width, gap),
            }
        }
        Folded::Count => Plan {
            width: DOT_PAD + DOT + DOT_GAP + measure.count + DOT_PAD,
            ..Plan::default()
        },
        Folded::Dot => Plan {
            width: DOT_ONLY_WIDTH,
            ..Plan::default()
        },
    }
}

/// The width of the whole bar for these pill widths and `gap`: the bar's padding, the pills, a gap
/// after each (the last one before the trailing block) and the trailing block.
pub(super) fn bar_width(widths: &[f32], gap: f32) -> f32 {
    2.0 * BAR_PAD_X + widths.iter().sum::<f32>() + gap * widths.len() as f32 + TRAILING_WIDTH
}

/// The largest bar width over every pointer position (the pointer on the trailing block gives the
/// resting width, which is never larger).
#[cfg(test)]
pub(super) fn worst_bar_width(widths: &[f32], gap: f32) -> f32 {
    worst_bar_width_fixed(widths, gap, None)
}

/// [`worst_bar_width`] when the pill `fixed` never grows (the current group's: brief D16), neither
/// under the pointer nor as a neighbour of the pill that is.
pub(super) fn worst_bar_width_fixed(widths: &[f32], gap: f32, fixed: Option<usize>) -> f32 {
    let rest = bar_width(widths, gap);
    let mut worst = rest;
    let grows = |i: usize| Some(i) != fixed;
    for (i, width) in widths.iter().enumerate() {
        if !grows(i) {
            continue;
        }
        let mut growth = (UNDER_POINTER - 1.0) * width;
        if i > 0 && grows(i - 1) {
            growth += (NEIGHBOUR - 1.0) * widths[i - 1];
        }
        if let Some(next) = widths.get(i + 1).filter(|_| grows(i + 1)) {
            growth += (NEIGHBOUR - 1.0) * next;
        }
        worst = worst.max(rest + growth);
    }
    worst
}

/// The tier and the pills' insides that fit the container.
#[derive(Clone, Debug)]
pub(super) struct Fit {
    pub tier: u8,
    pub gap: f32,
    pub plans: Vec<Plan>,
}

fn build(measures: &[Measure], current: usize, tier: u8, badge: bool) -> Fit {
    let plans: Vec<Plan> = measures
        .iter()
        .enumerate()
        .map(|(i, measure)| plan(measure, tier, i == current, badge))
        .collect();
    Fit {
        tier,
        gap: spec(tier).gap,
        plans,
    }
}

fn fits(fit: &Fit, limit: f32, current: usize) -> bool {
    let widths: Vec<f32> = fit.plans.iter().map(|plan| plan.width).collect();
    worst_bar_width_fixed(&widths, fit.gap, Some(current)) <= limit
}

/// The first tier that fits `container` (a width that is not finite counts as 0). Key badges:
/// every pill that has a key shows one while `held` (tiers 1 and 2); `show_keys` shows them at
/// rest only if tier 1 still fits with them (D13).
pub(super) fn choose(
    measures: &[Measure],
    current: usize,
    container: f32,
    held: bool,
    show_keys: bool,
) -> Fit {
    let container = if container.is_finite() {
        container.max(0.0)
    } else {
        0.0
    };
    let limit = container - 2.0 * HEADROOM;
    if show_keys {
        let fit = build(measures, current, 1, true);
        if fits(&fit, limit, current) {
            return fit;
        }
    }
    for tier in 1..=LAST_TIER {
        let fit = build(measures, current, tier, held && tier <= 2);
        if tier == LAST_TIER || fits(&fit, limit, current) {
            return fit;
        }
    }
    build(measures, current, LAST_TIER, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(n: usize) -> Measure {
        Measure {
            name: 50.0,
            badge: Some(16.0),
            count: 12.0,
            chips: vec![100.0; n],
            active_chip: None,
        }
    }

    #[test]
    fn the_worst_case_is_the_widest_pill_magnified() {
        let widths = [100.0, 300.0, 100.0];
        let rest = bar_width(&widths, 10.0);
        // The wide pill under the pointer: +45 % of 300 and +12 % of each neighbour.
        let expected = rest + 0.45 * 300.0 + 0.12 * 200.0;
        assert!((worst_bar_width(&widths, 10.0) - expected).abs() < 1e-3);
        assert_eq!(worst_bar_width(&[], 10.0), bar_width(&[], 10.0));
    }

    #[test]
    fn a_folded_pill_at_tier_one_is_name_and_slivers() {
        let plan = plan(&measure(3), 1, false, false);
        assert_eq!(plan.slivers.len(), 3);
        // 12 + 50 + 8 + (3 * 6 + 2 * 2) + 12
        assert!((plan.width - 104.0).abs() < 1e-3, "{}", plan.width);
        assert!(plan.slivers.iter().all(|s| (s.width - 6.0).abs() < 1e-3));
        let last = plan.slivers[2];
        assert!((last.x + last.width + 12.0 - plan.width).abs() < 1e-3);
    }

    #[test]
    fn many_tabs_squeeze_their_slivers_down_to_two_pixels() {
        let plan = plan(&measure(40), 1, false, false);
        assert_eq!(plan.slivers.len(), 40);
        assert!(plan.slivers.iter().all(|s| s.width >= 2.0));
        let first = plan.slivers[0];
        let last = plan.slivers[39];
        assert!(last.x + last.width <= plan.width);
        assert!(first.x >= 0.0);
    }

    #[test]
    fn tiers_three_and_four_drop_the_name_and_the_slivers() {
        assert!(plan(&measure(3), 3, false, true).slivers.is_empty());
        let dot = plan(&measure(3), 4, false, true);
        assert_eq!(dot.width, 14.0);
        assert!(!dot.show_key);
    }

    #[test]
    fn the_current_group_collapses_to_slivers_at_tier_five_only() {
        assert!(plan(&measure(3), 4, true, false).expanded);
        let collapsed = plan(&measure(3), 5, true, false);
        assert!(!collapsed.expanded);
        assert_eq!(collapsed.slivers.len(), 3);
    }

    #[test]
    fn nothing_fitting_gives_the_last_tier_and_finite_widths() {
        let measures: Vec<Measure> = (0..64).map(|_| measure(3)).collect();
        for container in [360.0, 0.0, f32::NAN, f32::INFINITY, -5.0] {
            let fit = choose(&measures, 0, container, false, false);
            assert_eq!(fit.tier, LAST_TIER, "{container}");
            assert!(
                fit.plans
                    .iter()
                    .all(|p| p.width.is_finite() && p.width > 0.0)
            );
        }
    }
}
