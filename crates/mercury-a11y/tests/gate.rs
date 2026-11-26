// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The update gate (`mercury_a11y::gate`, EE11): at most one tree update per interval, with
//! layout-only damage waiting for the next slot and urgent damage going at once.

use mercury_a11y::gate::{Damage, Decision, UpdateGate};

const INTERVAL: u64 = 250;

#[test]
fn the_first_layout_damage_sends() {
    let mut gate = UpdateGate::new(INTERVAL);
    assert_eq!(gate.offer(0, Some(Damage::LayoutOnly)), Decision::Send);
}

#[test]
fn layout_damage_within_250ms_waits_the_rest() {
    let mut gate = UpdateGate::new(INTERVAL);
    assert_eq!(gate.offer(0, Some(Damage::LayoutOnly)), Decision::Send);
    gate.sent(0);
    assert_eq!(
        gate.offer(100, Some(Damage::LayoutOnly)),
        Decision::WaitMs(150)
    );
}

#[test]
fn urgent_always_sends() {
    let mut gate = UpdateGate::new(INTERVAL);
    assert_eq!(gate.offer(0, Some(Damage::Urgent)), Decision::Send);
    gate.sent(0);
    assert_eq!(gate.offer(1, Some(Damage::Urgent)), Decision::Send);
}

#[test]
fn no_damage_is_nothing() {
    let mut gate = UpdateGate::new(INTERVAL);
    assert_eq!(gate.offer(0, None), Decision::Nothing);
    gate.sent(0);
    assert_eq!(gate.offer(10, None), Decision::Nothing);
}

#[test]
fn after_the_wait_it_sends() {
    let mut gate = UpdateGate::new(INTERVAL);
    gate.sent(0);
    assert_eq!(
        gate.offer(100, Some(Damage::LayoutOnly)),
        Decision::WaitMs(150)
    );
    assert_eq!(gate.offer(250, None), Decision::Send);
    gate.sent(250);
    assert_eq!(gate.offer(300, None), Decision::Nothing);
}

#[test]
fn urgent_resets_the_interval() {
    let mut gate = UpdateGate::new(INTERVAL);
    gate.sent(0);
    assert_eq!(gate.offer(100, Some(Damage::Urgent)), Decision::Send);
    gate.sent(100);
    assert_eq!(
        gate.offer(150, Some(Damage::LayoutOnly)),
        Decision::WaitMs(200)
    );
}
