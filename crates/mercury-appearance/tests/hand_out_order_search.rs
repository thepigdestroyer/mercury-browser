// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Re-derives the hand-out order of `PRESETS` from the four rules in its documentation, so that
//! the claim "found by an exhaustive search" can be checked and repeated.
//!
//! This is a tool, not a gate, so it is ignored in a normal run: the order is a stored file format
//! and must not move because a colour's hex value was nudged. Run it on purpose when a preset
//! colour changes or a colour is added:
//!
//! ```text
//! cargo test --release -p mercury-appearance --test hand_out_order_search -- --ignored --nocapture
//! ```
//!
//! The rules, in priority order (the 13 coloured presets are ordered; slate goes last):
//!
//! 1. blue first;
//! 2. every set of the first `n` presets keeps its closest pair at least 0.13 apart in OKLab for
//!    `n` up to 11, and 0.10 and 0.085 for 12 and 13 (the most any order can have: the closest
//!    pairs of the whole list are blue/violet at 0.102 and green/teal at 0.086);
//! 3. the smallest hue gap between two presets handed out one after the other is as large as it
//!    can be (a dynamic programme over every subset);
//! 4. among the orders that tie on rule 3, take the earliest groups furthest apart: at each step
//!    the colour that keeps the closest pair of the set so far widest (farthest point first), a
//!    larger hue gap breaking a tie.

use mercury_appearance::oklab::{Oklab, hue};
use mercury_appearance::{PRESETS, Preset};

/// The OKLab spread (closest pair) a set of `n` coloured presets must keep.
fn spread_floor(n: usize) -> f64 {
    match n {
        0..=11 => 0.13,
        12 => 0.10,
        _ => 0.085,
    }
}

/// The smallest angle between two hues, in degrees (0 to 180).
fn hue_gap(a: f64, b: f64) -> f64 {
    let turn = (a - b).abs() % 360.0;
    turn.min(360.0 - turn)
}

fn distance(a: &Preset, b: &Preset) -> f64 {
    Oklab::from_colour(a.base).distance(Oklab::from_colour(b.base))
}

/// The order the rules choose, as preset ids, and the smallest hue gap between neighbours in it.
fn derive() -> (Vec<&'static str>, f64) {
    let slate = PRESETS.iter().position(|p| p.id == "slate").expect("slate");
    let blue = PRESETS.iter().position(|p| p.id == "blue").expect("blue");
    // The coloured presets, as indices into PRESETS; bit `i` of a mask is `coloured[i]`.
    let coloured: Vec<usize> = (0..PRESETS.len()).filter(|&i| i != slate).collect();
    let n = coloured.len();
    let full = (1usize << n) - 1;
    let hues: Vec<f64> = coloured.iter().map(|&i| hue(PRESETS[i].base)).collect();
    let gap = |a: usize, b: usize| hue_gap(hues[a], hues[b]);

    // The closest pair inside each set, and whether the set may be a prefix (rule 2).
    let spread: Vec<f64> = (0..=full)
        .map(|mask| {
            let mut closest = f64::INFINITY;
            for i in 0..n {
                for j in (i + 1)..n {
                    if mask & (1 << i) != 0 && mask & (1 << j) != 0 {
                        closest =
                            closest.min(distance(&PRESETS[coloured[i]], &PRESETS[coloured[j]]));
                    }
                }
            }
            closest
        })
        .collect();
    let allowed: Vec<bool> = (0..=full)
        .map(|mask| mask == 0 || spread[mask] >= spread_floor(mask.count_ones() as usize) - 1e-9)
        .collect();
    let start = coloured.iter().position(|&i| i == blue).expect("blue");

    // Rule 3: the best smallest-gap over every way to continue from (set, last).
    let mut best = vec![vec![f64::NEG_INFINITY; n]; full + 1];
    best[1 << start][start] = f64::INFINITY;
    for mask in 1..=full {
        for last in 0..n {
            let so_far = best[mask][last];
            if so_far == f64::NEG_INFINITY {
                continue;
            }
            for next in (0..n).filter(|next| mask & (1 << next) == 0) {
                let longer = mask | (1 << next);
                if allowed[longer] {
                    let value = so_far.min(gap(last, next));
                    best[longer][next] = best[longer][next].max(value);
                }
            }
        }
    }
    let bottleneck = best[full].iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(bottleneck > 0.0, "no order satisfies rule 2");
    let edge = bottleneck - 1e-6;

    // Which (set, last) can still be finished with every gap at least `bottleneck`.
    let mut finishes = vec![vec![false; n]; full + 1];
    for mask in (1..=full).rev() {
        for last in (0..n).filter(|last| mask & (1 << last) != 0) {
            finishes[mask][last] = mask == full
                || (0..n).any(|next| {
                    let longer = mask | (1 << next);
                    mask & (1 << next) == 0
                        && gap(last, next) >= edge
                        && allowed[longer]
                        && finishes[longer][next]
                });
        }
    }
    assert!(finishes[1 << start][start], "blue cannot start an order");

    // Rule 4: walk forward, always taking the widest-spread colour that can still be finished.
    let (mut mask, mut last) = (1usize << start, start);
    let mut order = vec![PRESETS[coloured[start]].id];
    while mask != full {
        let mut chosen: Option<(f64, f64, usize)> = None;
        let candidates = (0..n).filter(|&next| {
            let longer = mask | (1 << next);
            mask & (1 << next) == 0
                && gap(last, next) >= edge
                && allowed[longer]
                && finishes[longer][next]
        });
        for next in candidates {
            let longer = mask | (1 << next);
            let key = (spread[longer], gap(last, next), next);
            let better = chosen.is_none_or(|(widest, wide_gap, _)| {
                key.0 > widest + 1e-9 || ((key.0 - widest).abs() <= 1e-9 && key.1 > wide_gap)
            });
            if better {
                chosen = Some(key);
            }
        }
        let (_, _, next) = chosen.expect("the walk follows a finishable path");
        order.push(PRESETS[coloured[next]].id);
        mask |= 1 << next;
        last = next;
    }
    order.push(PRESETS[slate].id);
    (order, bottleneck)
}

#[test]
#[ignore = "a tool: re-derives the stored hand-out order; run it when a preset colour changes"]
fn the_rules_choose_the_order_of_presets() {
    let (order, bottleneck) = derive();
    println!("derived order: {}", order.join(" "));
    println!("smallest hue gap between neighbours: {bottleneck:.1} degrees");
    let stored: Vec<&str> = PRESETS.iter().map(|preset| preset.id).collect();
    assert_eq!(order, stored, "the rules now choose a different order");
    assert!(bottleneck > 107.0, "{bottleneck}");
}
