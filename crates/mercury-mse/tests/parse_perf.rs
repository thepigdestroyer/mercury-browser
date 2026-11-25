// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Parse throughput, recorded (not gated): a fixture's media segments repeated until about 64 MB have gone
//! through a parser in 16 KiB pieces, the size a network read gives. The numbers are for the lane report;
//! the only assertion is that every frame came out.
//! Run with `cargo test --release -p mercury-mse --test parse_perf -- --nocapture`.

mod parse_support;

mod parse_perf {
    use super::parse_support::*;
    use mercury_mse::parse::new_parser;
    use mercury_mse_testkit::load;
    use std::time::Instant;

    #[test]
    fn throughput_is_recorded() {
        for name in [
            "h264-high",
            "h264-bframes",
            "h264-aac",
            "vp9-high",
            "av1-low",
            "opus",
            "aac",
        ] {
            let stream = load(name).unwrap();
            let mut parser = new_parser(container_of(stream.container));
            let mut frames = 0usize;
            frames += frames_of(
                &push_plan(
                    parser.as_mut(),
                    &stream.init.bytes,
                    &mercury_mse_testkit::ChunkPlan::Whole,
                )
                .unwrap(),
            )
            .len();
            let one: usize = stream.media.iter().map(|m| m.bytes.len()).sum();
            let rounds = (64 << 20) / one + 1;
            let plan = mercury_mse_testkit::ChunkPlan::Fixed(16 * 1024);
            // Parse once to count what a round should give.
            let per_round: usize = frames_of(
                &feed_stream(&stream, &mercury_mse_testkit::ChunkPlan::Whole)
                    .unwrap()
                    .events,
            )
            .len();
            let started = Instant::now();
            let mut bytes = 0usize;
            let mut got = 0usize;
            for _ in 0..rounds {
                for m in &stream.media {
                    for piece in mercury_mse_testkit::chunks(&m.bytes, &plan) {
                        let events = parser.push(piece).unwrap();
                        for e in events {
                            if let mercury_mse::api::ParseEvent::Frames(fs) = e {
                                got += fs.len();
                            }
                        }
                    }
                    bytes += m.bytes.len();
                }
            }
            let secs = started.elapsed().as_secs_f64();
            println!(
                "PERF parse {name}: {:.0} MB/s, {:.0} frames/s ({} MB in {:.2} s)",
                bytes as f64 / 1e6 / secs,
                got as f64 / secs,
                bytes >> 20,
                secs
            );
            // Every round gives its frames (the last cluster of a WebM stream waits for its end, so allow one cluster's worth).
            assert!(
                got + 50 >= per_round * rounds && got <= per_round * rounds,
                "{name}: {got} of {}",
                per_round * rounds
            );
            let _ = frames;
        }
    }
}
