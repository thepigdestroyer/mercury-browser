// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![allow(dead_code)]

use mercury_omnibox::{Context, answer};

pub fn ctx() -> Context {
    Context {
        now_unix_secs: 1795348800,
        local_zone: "Europe/London".into(),
        clock24: false,
    }
}

pub fn text(input: &str) -> Option<String> {
    answer(input, &ctx()).map(|found| found.text)
}

pub fn copy(input: &str) -> Option<String> {
    answer(input, &ctx()).map(|found| found.copy)
}
