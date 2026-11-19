// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A program with the same dependencies as `probe_route` and no libwebrtc call: the baseline of
//! the size measurement (G7). It holds the tokio runtime, as `probe_route` does.

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        println!("baseline");
    });
}
