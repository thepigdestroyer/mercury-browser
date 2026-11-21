// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! T12, parser fuzz: 50 seeds of 300 mutations each of the committed fixtures (bit flips, overwrites,
//! truncation, insertion, duplication, swaps, size fields set to extreme values), fed under random
//! chunking. Every run must finish (no panic, no hang over 10 s), keep its memory bounded, end in Ok or
//! in a documented error, give the same answer as the whole input in one push, and leave a parser that is
//! usable after `reset`. A failing seed prints its number and mutation, and `MSE_T12_SEED=n` replays it.
//! Run with `cargo test -p mercury-mse --test 'parse_*'` (or the filter `parse_`).

mod parse_support;

mod parse_fuzz {
    use super::parse_support::*;
    use mercury_mse::api::{ByteStreamParser, Container, ParseError, ParseEvent};
    use mercury_mse::parse::mp4::Mp4Parser;
    use mercury_mse::parse::webm::WebmParser;
    use mercury_mse::parse::{FEED_SLICE, MAX_RETAINED_BYTES, new_parser};
    use mercury_mse_testkit::{ChunkPlan, chunks};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    const SEEDS: u64 = 50;
    const MUTATIONS: usize = 300;
    const HANG: Duration = Duration::from_secs(10);

    /// Boxes and elements whose size fields the structure-aware mutation attacks.
    const TAGS: [&[u8]; 14] = [
        b"moov",
        b"moof",
        b"mdat",
        b"trak",
        b"trun",
        b"tfhd",
        b"tfdt",
        b"stsd",
        b"avc1",
        b"mp4a",
        b"esds",
        &[0x1F, 0x43, 0xB6, 0x75], // Cluster
        &[0x16, 0x54, 0xAE, 0x6B], // Tracks
        &[0xA3],                   // SimpleBlock
    ];

    const SPECIAL_SIZES: [u32; 10] = [
        0,
        1,
        2,
        7,
        8,
        9,
        0x7FFF_FFFF,
        0x8000_0000,
        0xFFFF_FFFF,
        0x1000_0000,
    ];

    fn mutate(rng: &mut Rng, base: &[u8]) -> Vec<u8> {
        let mut d = base.to_vec();
        for _ in 0..1 + rng.below(5) {
            if d.is_empty() {
                let n = 1 + rng.below(32);
                d = rng.bytes(n);
            }
            let at = rng.below(d.len());
            match rng.below(10) {
                0 => d[at] ^= 1 << rng.below(8),
                1 => d[at] = rng.next() as u8,
                2 => {
                    let n = (1 + rng.below(16)).min(d.len() - at);
                    for b in &mut d[at..at + n] {
                        *b = rng.next() as u8;
                    }
                }
                3 => d.truncate(at),
                4 => {
                    let n = 1 + rng.below(32);
                    let ins = rng.bytes(n);
                    d.splice(at..at, ins);
                }
                5 => {
                    let n = (1 + rng.below(64)).min(d.len() - at);
                    let copy = d[at..at + n].to_vec();
                    let to = rng.below(d.len());
                    d.splice(to..to, copy);
                }
                6 => {
                    let n = (1 + rng.below(64)).min(d.len() - at);
                    d.drain(at..at + n);
                }
                7 => {
                    let n = (1 + rng.below(64)).min(d.len() - at);
                    for b in &mut d[at..at + n] {
                        *b = 0;
                    }
                }
                8 => {
                    // Swap two regions.
                    let n = (1 + rng.below(32)).min(d.len() - at);
                    let to = rng.below(d.len().saturating_sub(n).max(1));
                    for i in 0..n {
                        d.swap(at + i, to + i);
                    }
                }
                _ => {
                    // A size field set to an extreme value: before a known tag for MP4, after one for EBML.
                    let tag = TAGS[rng.below(TAGS.len())];
                    let hits: Vec<usize> = d
                        .windows(tag.len())
                        .enumerate()
                        .filter(|(_, w)| *w == tag)
                        .map(|(i, _)| i)
                        .collect();
                    if let Some(&i) = hits.get(rng.below(hits.len().max(1))) {
                        let v = SPECIAL_SIZES[rng.below(SPECIAL_SIZES.len())].to_be_bytes();
                        if tag.len() == 4 && tag[0].is_ascii_lowercase() && i >= 4 {
                            d[i - 4..i].copy_from_slice(&v);
                        } else if let Some(slot) = d.get_mut(i + tag.len()..i + tag.len() + 4) {
                            slot.copy_from_slice(&v);
                        }
                    }
                }
            }
        }
        d
    }

    fn documented(e: &ParseError) -> bool {
        match e {
            ParseError::Malformed(m)
            | ParseError::Unsupported(m)
            | ParseError::WrongContainer(m) => !m.is_empty(),
            ParseError::MediaBeforeInit | ParseError::TooManyTracks => true,
        }
    }

    /// What one run of a parser over `data` came to.
    #[derive(Debug)]
    enum Outcome {
        Events(Vec<Ev>),
        Failed(ParseError),
    }

    /// Feeds `data` (cut by `plan`) to a fresh concrete parser, checking the invariants after every push.
    fn run(container: Container, data: &[u8], plan: &ChunkPlan) -> Outcome {
        let mut mp4 = Mp4Parser::new();
        let mut webm = WebmParser::new();
        let mut events = Vec::new();
        for piece in chunks(data, plan) {
            let (result, held) = match container {
                Container::Mp4 => (mp4.push(piece), mp4.retained_bytes()),
                Container::WebM => (webm.push(piece), webm.retained_bytes()),
            };
            assert!(
                held <= MAX_RETAINED_BYTES + FEED_SLICE,
                "{held} bytes retained"
            );
            match result {
                Ok(ev) => {
                    check_events(&ev);
                    events.extend(flatten(ev));
                }
                Err(e) => {
                    assert!(documented(&e), "{e:?}");
                    // Sticky until reset.
                    let again = match container {
                        Container::Mp4 => mp4.push(&[0]),
                        Container::WebM => webm.push(&[0]),
                    };
                    assert_eq!(again.unwrap_err(), e);
                    return Outcome::Failed(e);
                }
            }
        }
        Outcome::Events(events)
    }

    /// Frames are sane whatever the input said.
    fn check_events(events: &[ParseEvent]) {
        for e in events {
            if let ParseEvent::Frames(fs) = e {
                assert!(!fs.is_empty(), "an empty batch");
                for f in fs {
                    assert!(f.duration >= 0, "negative duration {}", f.duration);
                    assert!(f.data.len() <= 64 << 20);
                }
            }
        }
    }

    fn same_outcome(a: &Outcome, b: &Outcome) -> bool {
        match (a, b) {
            (Outcome::Events(x), Outcome::Events(y)) => x == y,
            (Outcome::Failed(x), Outcome::Failed(y)) => x == y,
            _ => false,
        }
    }

    /// How many runs ended in each kind of error, in the order of the `ParseError` variants.
    type Kinds = [usize; 5];

    fn kind_index(e: &ParseError) -> usize {
        match e {
            ParseError::Malformed(_) => 0,
            ParseError::Unsupported(_) => 1,
            ParseError::MediaBeforeInit => 2,
            ParseError::TooManyTracks => 3,
            ParseError::WrongContainer(_) => 4,
        }
    }

    /// One seed: a fixture, 300 mutations of it.
    fn seed(seed: u64) -> (usize, usize, Kinds) {
        let streams = all_streams();
        let (name, stream) = &streams[(seed as usize) % streams.len()];
        let container = container_of(stream.container);
        // The init segment and the first two media segments: enough structure, small enough to mutate fast.
        let mut base = stream.init.bytes.clone();
        for m in stream.media.iter().take(2) {
            base.extend_from_slice(&m.bytes);
        }
        let good = stream.concat();
        let mut rng = Rng(0x7_1200 + seed * 0x9E37_79B9);
        let (mut ok, mut failed) = (0, 0);
        let mut kinds: Kinds = [0; 5];
        let mut recent: Vec<String> = Vec::new();
        for i in 0..MUTATIONS {
            let data = mutate(&mut rng, &base);
            let plan = random_plan(&mut rng);
            recent.push(format!("mutation {i}: {} bytes, {plan:?}", data.len()));
            if recent.len() > 10 {
                recent.remove(0);
            }
            let started = Instant::now();
            let result = catch_unwind(AssertUnwindSafe(|| {
                let whole = run(container, &data, &ChunkPlan::Whole);
                let cut = run(container, &data, &plan);
                // The same answer however the bytes are cut: events if it parses, the error if not.
                assert!(
                    same_outcome(&whole, &cut),
                    "whole and chunked disagree: {:?} versus {:?}",
                    summary(&whole),
                    summary(&cut)
                );
                match whole {
                    Outcome::Events(_) => None,
                    Outcome::Failed(e) => Some(kind_index(&e)),
                }
            }));
            let took = started.elapsed();
            match result {
                Ok(None) => ok += 1,
                Ok(Some(k)) => {
                    failed += 1;
                    kinds[k] += 1;
                }
                Err(panic) => {
                    eprintln!("T12 {name} seed {seed} (MSE_T12_SEED={seed}); the last operations:");
                    for r in &recent {
                        eprintln!("  {r}");
                    }
                    std::panic::resume_unwind(panic);
                }
            }
            assert!(took < HANG, "{name} seed {seed} mutation {i} took {took:?}");
            // Every tenth run: a parser that failed is usable after reset, and gives what a clean one gives.
            if i % 10 == 0 {
                let mut p = new_parser(container);
                for piece in data.chunks(997) {
                    if p.push(piece).is_err() {
                        break;
                    }
                }
                p.reset();
                let again = push_plan(p.as_mut(), &good, &ChunkPlan::Fixed(5000));
                assert!(
                    again.is_ok(),
                    "{name} seed {seed} mutation {i}: not usable after reset: {again:?}"
                );
                // After a reset in the middle of an init segment the configuration may be missing, so only a
                // parser that had read its init segment before the damage must match a clean run exactly.
                if let (Ok(a), Ok(b)) =
                    (&again, parse_parts(container, &[&good], &ChunkPlan::Whole))
                {
                    assert!(frames_of(a).len() <= frames_of(&b).len() + 400);
                }
            }
        }
        (ok, failed, kinds)
    }

    fn summary(o: &Outcome) -> String {
        match o {
            Outcome::Events(e) => format!("{} events", e.len()),
            Outcome::Failed(e) => format!("{e}"),
        }
    }

    /// Runs `f` on a thread and fails if it has not finished in `limit` (a hang is a failure, not a wait).
    fn with_watchdog<T: Send + 'static>(
        what: &str,
        limit: Duration,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let r = catch_unwind(AssertUnwindSafe(f));
            let _ = tx.send(r);
        });
        match rx.recv_timeout(limit) {
            Ok(Ok(v)) => v,
            Ok(Err(panic)) => std::panic::resume_unwind(panic),
            Err(_) => panic!("{what} did not finish within {limit:?}: a hang"),
        }
    }

    #[test]
    fn t12_fifty_seeds_of_three_hundred_mutations() {
        let only: Option<u64> = std::env::var("MSE_T12_SEED")
            .ok()
            .and_then(|s| s.parse().ok());
        let seeds: Vec<u64> = match only {
            Some(s) => vec![s],
            None => (0..SEEDS).collect(),
        };
        let (mut ok, mut failed) = (0usize, 0usize);
        let mut kinds: Kinds = [0; 5];
        for s in seeds {
            let (o, f, k) =
                with_watchdog(&format!("seed {s}"), Duration::from_secs(300), move || {
                    seed(s)
                });
            ok += o;
            failed += f;
            for (total, n) in kinds.iter_mut().zip(k) {
                *total += n;
            }
        }
        println!(
            "T12 parser fuzz: {ok} mutations parsed, {failed} ended in a documented error \
             (malformed {}, unsupported {}, media before init {}, too many tracks {}, wrong container {})",
            kinds[0], kinds[1], kinds[2], kinds[3], kinds[4]
        );
        // The mutations are damaging enough to fail often and gentle enough to parse sometimes.
        if only.is_none() {
            assert!(
                failed > 1000,
                "only {failed} of {} runs failed: the mutations are too gentle",
                ok + failed
            );
            assert!(
                ok > 500,
                "only {ok} of {} runs parsed: the mutations are too harsh",
                ok + failed
            );
        }
    }

    #[test]
    fn t12_extreme_declared_sizes_neither_allocate_nor_hang() {
        // A moov, moof or Cluster whose size field is every special value, at the very start and in the
        // middle of a real stream, fed one byte at a time and whole.
        let streams = all_streams();
        let mut rng = Rng(0xB16);
        for (name, stream) in &streams {
            let container = container_of(stream.container);
            let base = stream.concat();
            for _ in 0..40 {
                let d = mutate(&mut rng, &base[..base.len().min(40_000)]);
                let started = Instant::now();
                let _ = run(container, &d, &ChunkPlan::Fixed(1));
                let _ = run(container, &d, &ChunkPlan::Whole);
                assert!(started.elapsed() < HANG, "{name}");
            }
        }
    }
}
