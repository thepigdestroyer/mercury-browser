// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Property tests: 1,000 seeds of random appends, removes, resets and evictions on one video track,
//! checked step by step against a naive model (a plain `Vec` of frames and the spec's steps written the
//! slow way). After every operation the processor's frames (in decode order), bytes, highest end and
//! buffered ranges must equal the model's, and the ranges must be sorted and disjoint. A failing seed
//! prints the last operations; `FRAMES_PROPERTY_SEED=n` replays one seed.

mod frames_support;

mod frames_property {
    use super::frames_support::*;
    use mercury_mse::{
        CodedFrame, Limits, Ns, SourceBufferProcessor, TimeRange, TrackKind, ranges,
    };

    const V: TrackKind = TrackKind::Video;
    const EPS: Ns = 1_000;
    const MAX_TOL: Ns = 250 * MS;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct NF {
        dts: Ns,
        seq: u64,
        pts: Ns,
        dur: Ns,
        key: bool,
        bytes: usize,
    }

    impl NF {
        fn end(&self) -> Ns {
            self.pts + self.dur
        }
    }

    /// The model: every step of the processor, with linear scans.
    struct Naive {
        frames: Vec<NF>,
        next_seq: u64,
        last_dts: Option<Ns>,
        last_dur: Option<Ns>,
        highest_end: Option<Ns>,
        need_rap: bool,
        group_end: Ns,
        longest: Ns,
        last_inserted: Option<Ns>,
        limits: Limits,
    }

    impl Naive {
        fn new(limits: Limits) -> Self {
            Naive {
                frames: Vec::new(),
                next_seq: 0,
                last_dts: None,
                last_dur: None,
                highest_end: None,
                need_rap: true,
                group_end: 0,
                longest: 0,
                last_inserted: None,
                limits,
            }
        }

        fn sorted(&self) -> Vec<NF> {
            let mut v = self.frames.clone();
            v.sort_by_key(|f| (f.dts, f.seq));
            v
        }

        fn bytes(&self) -> usize {
            self.frames.iter().map(|f| f.bytes).sum()
        }

        fn ranges(&self) -> Vec<TimeRange> {
            let mut by_pts: Vec<&NF> = self.frames.iter().collect();
            by_pts.sort_by_key(|f| (f.pts, f.dts, f.seq));
            let mut out: Vec<TimeRange> = Vec::new();
            for f in by_pts {
                if f.pts < f.end() {
                    match out.last_mut() {
                        Some(last) if f.pts <= last.end + EPS => last.end = last.end.max(f.end()),
                        _ => out.push(TimeRange::new(f.pts, f.end())),
                    }
                }
            }
            out
        }

        fn tolerance(&self) -> Ns {
            self.longest.clamp(0, MAX_TOL)
        }

        fn reset_group(&mut self) {
            self.last_dts = None;
            self.last_dur = None;
            self.highest_end = None;
        }

        /// Removes the frames in `idx` (indexes into `self.frames`) and every frame that may depend on
        /// one of them (the frames after each in decode order up to the next keyframe).
        fn remove_with_dependents(&mut self, mut doomed: Vec<usize>) -> (usize, usize, Option<Ns>) {
            if doomed.is_empty() {
                return (0, 0, None);
            }
            let mut order: Vec<usize> = (0..self.frames.len()).collect();
            order.sort_by_key(|&i| (self.frames[i].dts, self.frames[i].seq));
            let pos_of: std::collections::HashMap<usize, usize> =
                order.iter().enumerate().map(|(p, &i)| (i, p)).collect();
            let mut marked = vec![false; self.frames.len()];
            for &i in &doomed {
                marked[i] = true;
            }
            doomed.sort_by_key(|i| pos_of[i]);
            for &i in &doomed {
                let mut p = pos_of[&i] + 1;
                while p < order.len() && !self.frames[order[p]].key {
                    marked[order[p]] = true;
                    p += 1;
                }
            }
            let mut count = 0;
            let mut bytes = 0;
            let mut hit = None;
            let mut keep = Vec::new();
            for (i, f) in self.frames.iter().enumerate() {
                if marked[i] {
                    count += 1;
                    bytes += f.bytes;
                    if hit.is_none() && self.last_dts == Some(f.dts) {
                        hit = Some(f.pts);
                    }
                } else {
                    keep.push(f.clone());
                }
            }
            self.frames = keep;
            if self.frames.is_empty() {
                self.longest = 0;
                self.last_inserted = None;
            }
            (count, bytes, hit)
        }

        fn indexes_in_pts(&self, lo: Ns, hi: Option<Ns>) -> Vec<usize> {
            (0..self.frames.len())
                .filter(|&i| {
                    let p = self.frames[i].pts;
                    p >= lo && hi.is_none_or(|h| p < h)
                })
                .collect()
        }

        fn group_over(&mut self, pts: Ns) {
            self.group_end = pts;
            self.reset_group();
            self.need_rap = true;
        }

        fn process(&mut self, input: &[CodedFrame]) -> (usize, usize) {
            let (mut accepted, mut dropped) = (0, 0);
            for f in input {
                if self.process_one(f) {
                    accepted += 1;
                } else {
                    dropped += 1;
                }
            }
            (accepted, dropped)
        }

        fn process_one(&mut self, f: &CodedFrame) -> bool {
            let (pts, dts, dur) = (f.pts, f.dts, f.duration);
            if let Some(last) = self.last_dts
                && (dts < last || dts - last > self.last_dur.unwrap() * 2)
            {
                self.group_over(pts);
            }
            let end = pts + dur;
            if self.need_rap {
                if !f.keyframe {
                    return false;
                }
                self.need_rap = false;
            }
            if self.last_dts.is_none() {
                // the video overlap rule: replace a frame that starts within a microsecond
                let covering = (0..self.frames.len())
                    .filter(|&i| self.frames[i].pts <= pts)
                    .max_by_key(|&i| (self.frames[i].pts, self.frames[i].dts, self.frames[i].seq))
                    .filter(|&i| pts < self.frames[i].end());
                if let Some(i) = covering
                    && pts < self.frames[i].pts + 1_000
                {
                    self.remove_with_dependents(vec![i]);
                }
            }
            let window = match self.highest_end {
                None => Some((pts, end)),
                Some(h) if h <= pts => Some((h, end)),
                Some(_) => None,
            };
            if let Some((lo, hi)) = window
                && lo < hi
            {
                let idx = self.indexes_in_pts(lo, Some(hi));
                self.remove_with_dependents(idx);
            }
            self.frames.push(NF {
                dts,
                seq: self.next_seq,
                pts,
                dur,
                key: f.keyframe,
                bytes: f.data.len() + mercury_mse::FRAME_OVERHEAD_BYTES,
            });
            self.next_seq += 1;
            self.longest = self.longest.max(dur);
            self.last_inserted = Some(pts);
            self.last_dts = Some(dts);
            self.last_dur = Some(dur);
            if self.highest_end.is_none_or(|h| end > h) {
                self.highest_end = Some(end);
            }
            if end > self.group_end {
                self.group_end = end;
            }
            true
        }

        fn last_frame_removed(&mut self, pts: Ns) {
            self.group_end = pts;
            self.reset_group();
            self.need_rap = true;
        }

        fn remove(&mut self, start: Ns, end: Option<Ns>) -> (usize, usize) {
            if end.is_some_and(|e| e <= start) {
                return (0, 0);
            }
            let remove_end = end.and_then(|e| {
                self.frames
                    .iter()
                    .filter(|f| f.key && f.pts >= e)
                    .map(|f| f.pts)
                    .min()
            });
            let idx = self.indexes_in_pts(start, remove_end);
            let (count, bytes, hit) = self.remove_with_dependents(idx);
            if let Some(pts) = hit {
                self.last_frame_removed(pts);
            }
            (count, bytes)
        }

        fn range_end_at(&self, t: Ns) -> Option<Ns> {
            self.ranges()
                .iter()
                .find(|r| r.start <= t && t < r.end)
                .map(|r| r.end)
        }

        /// The group of pictures at the front or the back: (indexes, bytes, min pts, max end).
        fn gop(&self, front: bool) -> Option<(Vec<usize>, usize, Ns, Ns)> {
            let mut order: Vec<usize> = (0..self.frames.len()).collect();
            order.sort_by_key(|&i| (self.frames[i].dts, self.frames[i].seq));
            let group: Vec<usize> = if front {
                let mut g = Vec::new();
                for &i in &order {
                    if self.frames[i].key && !g.is_empty() {
                        break;
                    }
                    g.push(i);
                }
                g
            } else {
                let mut g = Vec::new();
                for &i in order.iter().rev() {
                    g.push(i);
                    if self.frames[i].key {
                        break;
                    }
                }
                g
            };
            if group.is_empty() {
                return None;
            }
            let bytes = group.iter().map(|&i| self.frames[i].bytes).sum();
            let min_pts = group.iter().map(|&i| self.frames[i].pts).min().unwrap();
            let max_end = group.iter().map(|&i| self.frames[i].end()).max().unwrap();
            Some((group, bytes, min_pts, max_end))
        }

        /// Removes exactly the frames at `idx`, no dependents (whole groups go).
        fn drop_indexes(&mut self, idx: &[usize]) -> (usize, usize, Option<Ns>) {
            let mut count = 0;
            let mut bytes = 0;
            let mut hit = None;
            let mut keep = Vec::new();
            for (i, f) in self.frames.iter().enumerate() {
                if idx.contains(&i) {
                    count += 1;
                    bytes += f.bytes;
                    if hit.is_none() && self.last_dts == Some(f.dts) {
                        hit = Some(f.pts);
                    }
                } else {
                    keep.push(f.clone());
                }
            }
            self.frames = keep;
            if self.frames.is_empty() {
                self.longest = 0;
                self.last_inserted = None;
            }
            (count, bytes, hit)
        }

        fn evict(&mut self, incoming: usize, position: Ns) -> (bool, usize, usize) {
            let quota = self.limits.video_bytes;
            let need = (self.bytes() + incoming).saturating_sub(quota);
            let (mut frames, mut bytes) = (0, 0);
            let mut hit = None;
            if need > 0 {
                let cut = position - self.limits.keep_behind;
                let limit = position + self.limits.keep_ahead;
                while bytes < need {
                    let Some((idx, b, _, max_end)) = self.gop(true) else {
                        break;
                    };
                    if max_end > cut {
                        break;
                    }
                    let (c, rb, h) = self.drop_indexes(&idx);
                    if c == 0 {
                        break;
                    }
                    let _ = b;
                    frames += c;
                    bytes += rb;
                    hit = hit.or(h);
                }
                let protected = [
                    self.range_end_at(position),
                    self.last_inserted.and_then(|t| self.range_end_at(t)),
                ]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or(Ns::MIN);
                let line = limit.max(protected);
                while bytes < need {
                    let Some((idx, _, min_pts, _)) = self.gop(false) else {
                        break;
                    };
                    if min_pts < line {
                        break;
                    }
                    let (c, rb, h) = self.drop_indexes(&idx);
                    if c == 0 {
                        break;
                    }
                    frames += c;
                    bytes += rb;
                    hit = hit.or(h);
                }
                if let Some(pts) = hit {
                    self.last_frame_removed(pts);
                }
            }
            let ok = self.bytes() + incoming <= quota;
            (ok, frames, bytes)
        }

        fn reset_parser_state(&mut self) {
            self.reset_group();
            self.need_rap = true;
        }
    }

    // ---- the random operations ----

    fn random_segment(rng: &mut Rng, model_end: Ns) -> Vec<CodedFrame> {
        if rng.chance(25) {
            // a stream with reordering: groups of 10 frames in decode order I P B B P B B P B B,
            // presentation times out of order, starting on the 40 ms grid
            let groups = rng.range(1, 3) as usize;
            let dts_start = rng.range(0, 200) * 40 * MS;
            let mut frames = bframes(dts_start, groups, 40 * MS, rng.range(100, 900) as usize);
            if rng.chance(10) {
                frames[0].keyframe = false;
            }
            return frames;
        }
        let dur = [20, 40, 40, 40, 60][rng.below(5) as usize] * MS;
        let gop = [1usize, 5, 10, 25, 50][rng.below(5) as usize];
        let count = rng.range(1, 60) as usize;
        // where it starts: right after the last segment, or anywhere on the 20 ms grid
        let start = if rng.chance(40) {
            model_end
        } else {
            rng.range(0, 400) * 20 * MS
        };
        let key_bytes = rng.range(500, 3000) as usize;
        let delta_bytes = rng.range(100, 800) as usize;
        let mut frames = video(start, count, dur, gop, key_bytes, delta_bytes);
        if rng.chance(10) {
            frames[0].keyframe = false;
        }
        frames
    }

    fn compare(p: &SourceBufferProcessor, m: &Naive, what: &str) -> Result<(), String> {
        p.check_invariants()
            .map_err(|e| format!("{what}: invariants: {e}"))?;
        let got: Vec<NF> = p
            .track_buffer(V)
            .unwrap()
            .iter()
            .map(|(k, sf)| NF {
                dts: k.0,
                seq: k.1,
                pts: sf.frame.pts,
                dur: sf.frame.duration,
                key: sf.frame.keyframe,
                bytes: sf.frame.data.len() + mercury_mse::FRAME_OVERHEAD_BYTES,
            })
            .collect();
        let want = m.sorted();
        // sequence numbers differ only by gaps; compare everything else in order
        let strip = |v: &[NF]| -> Vec<(Ns, Ns, Ns, bool, usize)> {
            v.iter()
                .map(|f| (f.dts, f.pts, f.dur, f.key, f.bytes))
                .collect()
        };
        if strip(&got) != strip(&want) {
            return Err(format!(
                "{what}: frames differ\n got {} frames, want {}\n first got  {:?}\n first want {:?}",
                got.len(),
                want.len(),
                got.iter().take(4).collect::<Vec<_>>(),
                want.iter().take(4).collect::<Vec<_>>()
            ));
        }
        if p.bytes() != m.bytes() {
            return Err(format!("{what}: bytes {} want {}", p.bytes(), m.bytes()));
        }
        let exact = p.track_ranges_exact(V);
        if !ranges::is_normalized(&exact) {
            return Err(format!("{what}: ranges not sorted and disjoint: {exact:?}"));
        }
        let want_ranges = m.ranges();
        if exact != want_ranges {
            return Err(format!("{what}: ranges {exact:?} want {want_ranges:?}"));
        }
        let shown = p.buffered();
        if !ranges::is_normalized(&shown) {
            return Err(format!("{what}: buffered not normalised: {shown:?}"));
        }
        let want_shown = ranges::intersect(
            &[TimeRange::new(0, want_ranges.last().map_or(0, |r| r.end))],
            &ranges::coalesce(&want_ranges, m.tolerance()),
        );
        if shown != want_shown {
            return Err(format!("{what}: buffered {shown:?} want {want_shown:?}"));
        }
        if p.highest_end() != want_ranges.last().map(|r| r.end) {
            return Err(format!("{what}: highest_end {:?}", p.highest_end()));
        }
        Ok(())
    }

    fn run_seed(seed: u64) -> Result<(), String> {
        let limits = Limits {
            video_bytes: 120_000,
            audio_bytes: 1,
            text_bytes: 1,
            keep_behind: 3 * SEC,
            keep_ahead: 6 * SEC,
        };
        let mut rng = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0xDEAD_BEEF);
        let mut p = processor_with(&[V], limits);
        let mut m = Naive::new(limits);
        let mut log: Vec<String> = Vec::new();
        for step in 0..70 {
            let what;
            let model_end = m.ranges().last().map_or(0, |r| r.end);
            match rng.below(100) {
                0..=54 => {
                    let seg = random_segment(&mut rng, model_end);
                    what = format!(
                        "append {} frames from {} ms, dur {} ms, first key {}",
                        seg.len(),
                        ms(seg[0].pts),
                        ms(seg[0].duration),
                        seg[0].keyframe
                    );
                    let got = p.process_frames(seg.clone()).map_err(|e| e.to_string())?;
                    let (acc, dropped) = m.process(&seg);
                    if (got.accepted, got.dropped) != (acc, dropped) {
                        return Err(format!(
                            "step {step} {what}: accepted/dropped {:?} want {:?}",
                            (got.accepted, got.dropped),
                            (acc, dropped)
                        ));
                    }
                }
                55..=79 => {
                    let start = rng.range(-1000, 12_000) * MS + rng.range(0, 3) * 7;
                    let end = if rng.chance(20) {
                        None
                    } else {
                        Some(start + rng.range(-200, 9000) * MS)
                    };
                    what = format!("remove({} ms, {:?} ms)", ms(start), end.map(ms));
                    let got = p.remove(start, end);
                    let (c, b) = m.remove(start, end);
                    if (got.frames, got.bytes) != (c, b) {
                        return Err(format!(
                            "step {step} {what}: removed {:?} want {:?}",
                            (got.frames, got.bytes),
                            (c, b)
                        ));
                    }
                }
                80..=94 => {
                    let incoming = rng.range(0, 60_000) as usize;
                    let position = rng.range(0, 12_000) * MS;
                    what = format!("evict({incoming} bytes at {} ms)", ms(position));
                    let report = p.evict_report(incoming, position);
                    let (ok, c, b) = m.evict(incoming, position);
                    if (report.ok, report.removed.frames, report.removed.bytes) != (ok, c, b) {
                        return Err(format!(
                            "step {step} {what}: evict {:?} want {:?}",
                            (report.ok, report.removed.frames, report.removed.bytes),
                            (ok, c, b)
                        ));
                    }
                }
                _ => {
                    what = "reset_parser_state".to_string();
                    p.reset_parser_state();
                    m.reset_parser_state();
                }
            }
            log.push(format!("{step}: {what}"));
            if let Err(e) = compare(&p, &m, &what) {
                let tail: Vec<&String> = log.iter().rev().take(10).rev().collect();
                return Err(format!(
                    "{e}\nlast operations:\n  {}",
                    tail.iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join("\n  ")
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn a_thousand_seeds_of_random_operations_match_the_naive_model() {
        if let Ok(seed) = std::env::var("FRAMES_PROPERTY_SEED") {
            let seed: u64 = seed.parse().expect("FRAMES_PROPERTY_SEED is a number");
            run_seed(seed).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            return;
        }
        // `FRAMES_PROPERTY_SEEDS=20000` runs more seeds than the default 1,000
        let seeds: u64 = std::env::var("FRAMES_PROPERTY_SEEDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000);
        let mut failures = Vec::new();
        for seed in 0..seeds {
            if let Err(e) = run_seed(seed) {
                failures.push(format!("seed {seed}: {e}"));
                if failures.len() >= 3 {
                    break;
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} failing seeds (replay with FRAMES_PROPERTY_SEED=n):\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }

    #[test]
    fn the_ranges_are_always_sorted_and_disjoint_under_heavier_load() {
        // fewer comparisons, many more operations: nothing but the invariants is checked
        for seed in 0..40u64 {
            let limits = Limits {
                video_bytes: 400_000,
                keep_behind: 2 * SEC,
                keep_ahead: 4 * SEC,
                ..Limits::default()
            };
            let mut rng = Rng(seed + 7_000);
            let mut p = processor_with(&[V], limits);
            for _ in 0..400 {
                match rng.below(10) {
                    0..=6 => {
                        let end = p.highest_end().unwrap_or(0);
                        let seg = random_segment(&mut rng, end);
                        p.process_frames(seg).unwrap();
                    }
                    7 => {
                        let s = rng.range(0, 20_000) * MS;
                        p.remove(s, Some(s + rng.range(1, 5000) * MS));
                    }
                    8 => {
                        p.evict(rng.range(0, 100_000) as usize, rng.range(0, 20_000) * MS);
                    }
                    _ => p.reset_parser_state(),
                }
                p.check_invariants()
                    .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
                let shown = p.buffered();
                assert!(ranges::is_normalized(&shown), "seed {seed}: {shown:?}");
            }
        }
    }
}
