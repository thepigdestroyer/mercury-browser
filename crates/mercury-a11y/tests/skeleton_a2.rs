// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The A11Y-2 scaffold: every module and stub the later tasks fill in exists and has the spec's shape.

use mercury_a11y::gate::{Decision, UpdateGate};

#[test]
fn scaffold_modules_exist_with_neutral_stubs() {
    assert!(mercury_a11y::geometry::union_rects([]).is_none());
    assert_eq!(UpdateGate::new(250).offer(0, None), Decision::Nothing);
}
