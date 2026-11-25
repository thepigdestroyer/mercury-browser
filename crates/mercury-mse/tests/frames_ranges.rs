// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `ranges.rs`: normalise, union, intersect, subtract, coalesce, insert and find, against hand-made cases
//! and against a bitmap model over random sets.

mod frames_support;

mod frames_ranges {
    use super::frames_support::*;
    use mercury_mse::TimeRange;
    use mercury_mse::ranges::*;

    fn r(a: i64, b: i64) -> TimeRange {
        TimeRange::new(a, b)
    }

    fn pairs(v: &[TimeRange]) -> Vec<(i64, i64)> {
        v.iter().map(|r| (r.start, r.end)).collect()
    }

    #[test]
    fn normalize_sorts_drops_empty_and_merges() {
        assert!(normalize(&[]).is_empty());
        assert_eq!(
            pairs(&normalize(&[r(5, 6), r(1, 3), r(3, 4)])),
            [(1, 4), (5, 6)]
        );
        assert_eq!(pairs(&normalize(&[r(1, 10), r(2, 3), r(9, 12)])), [(1, 12)]);
        assert_eq!(pairs(&normalize(&[r(4, 4), r(7, 2), r(0, 1)])), [(0, 1)]);
        assert_eq!(pairs(&normalize(&[r(1, 2), r(1, 2), r(1, 2)])), [(1, 2)]);
        assert_eq!(
            pairs(&normalize(&[r(0, i64::MAX), r(5, 9)])),
            [(0, i64::MAX)]
        );
        assert!(is_normalized(&normalize(&[
            r(5, 6),
            r(1, 3),
            r(3, 4),
            r(2, 2)
        ])));
    }

    #[test]
    fn union_and_intersect() {
        let a = [r(0, 4), r(6, 10)];
        let b = [r(3, 7), r(12, 14)];
        assert_eq!(pairs(&union(&a, &b)), [(0, 10), (12, 14)]);
        assert_eq!(pairs(&intersect(&a, &b)), [(3, 4), (6, 7)]);
        assert_eq!(pairs(&union(&a, &[])), pairs(&a));
        assert!(intersect(&a, &[]).is_empty());
        assert!(
            intersect(&[r(0, 2)], &[r(2, 4)]).is_empty(),
            "touching ranges share no time"
        );
        assert_eq!(
            pairs(&union(&[r(0, 2)], &[r(2, 4)])),
            [(0, 4)],
            "but their union is one range"
        );
        assert_eq!(pairs(&intersect(&[r(0, 100)], &a)), pairs(&a));
    }

    #[test]
    fn subtract_cuts_holes_and_edges() {
        assert_eq!(pairs(&subtract(&[r(0, 10)], &[r(3, 5)])), [(0, 3), (5, 10)]);
        assert_eq!(pairs(&subtract(&[r(0, 10)], &[r(0, 5)])), [(5, 10)]);
        assert_eq!(pairs(&subtract(&[r(0, 10)], &[r(5, 20)])), [(0, 5)]);
        assert!(subtract(&[r(0, 10)], &[r(-5, 20)]).is_empty());
        assert_eq!(
            pairs(&subtract(&[r(0, 4), r(6, 10)], &[r(2, 8)])),
            [(0, 2), (8, 10)]
        );
        // one cut spanning two ranges and cutting inside the second
        assert_eq!(
            pairs(&subtract(
                &[r(0, 4), r(6, 10), r(12, 14)],
                &[r(3, 7), r(13, 20)]
            )),
            [(0, 3), (7, 10), (12, 13)]
        );
        assert_eq!(pairs(&subtract(&[r(0, 4)], &[])), [(0, 4)]);
    }

    #[test]
    fn coalesce_closes_small_gaps_only() {
        let v = [r(0, 10), r(12, 20), r(30, 40)];
        assert_eq!(pairs(&coalesce(&v, 0)), pairs(&v));
        assert_eq!(pairs(&coalesce(&v, 1)), pairs(&v));
        assert_eq!(pairs(&coalesce(&v, 2)), [(0, 20), (30, 40)]);
        assert_eq!(pairs(&coalesce(&v, 10)), [(0, 40)]);
        assert_eq!(pairs(&coalesce(&v, -5)), pairs(&v));
        assert!(coalesce(&[], 10).is_empty());
        assert_eq!(
            pairs(&coalesce(&[r(0, 1), r(i64::MAX - 1, i64::MAX)], i64::MAX)),
            [(0, i64::MAX)]
        );
    }

    #[test]
    fn insert_interval_merges_neighbours() {
        let mut v = vec![r(0, 10), r(20, 30), r(40, 50)];
        insert_interval(&mut v, r(10, 20), 0);
        assert_eq!(pairs(&v), [(0, 30), (40, 50)]);
        insert_interval(&mut v, r(31, 32), 0);
        assert_eq!(pairs(&v), [(0, 30), (31, 32), (40, 50)]);
        insert_interval(&mut v, r(33, 35), 0);
        assert_eq!(pairs(&v), [(0, 30), (31, 32), (33, 35), (40, 50)]);
        insert_interval(&mut v, r(35, 40), 2);
        assert_eq!(pairs(&v), [(0, 30), (31, 32), (33, 50)]);
        insert_interval(&mut v, r(-10, 100), 0);
        assert_eq!(pairs(&v), [(-10, 100)]);
        insert_interval(&mut v, r(5, 5), 0);
        insert_interval(&mut v, r(9, 3), 0);
        assert_eq!(pairs(&v), [(-10, 100)]);
        // with an epsilon, near neighbours join
        let mut w = vec![r(0, 10)];
        insert_interval(&mut w, r(11, 20), 1);
        assert_eq!(pairs(&w), [(0, 20)]);
        insert_interval(&mut w, r(22, 30), 1);
        assert_eq!(pairs(&w), [(0, 20), (22, 30)]);
        // appending at the end of a long set is a plain push
        let mut long: Vec<TimeRange> = (0..1000).map(|i| r(i * 10, i * 10 + 5)).collect();
        insert_interval(&mut long, r(10_000, 10_005), 0);
        assert_eq!(long.len(), 1001);
        assert!(is_normalized(&long));
    }

    #[test]
    fn find_contains_and_the_small_helpers() {
        let v = [r(0, 4), r(6, 10)];
        assert_eq!(find(&v, 0), Some(0));
        assert_eq!(find(&v, 3), Some(0));
        assert_eq!(find(&v, 4), None, "ranges are half open");
        assert_eq!(find(&v, 5), None);
        assert_eq!(find(&v, 6), Some(1));
        assert_eq!(find(&v, 10), None);
        assert_eq!(find(&v, -1), None);
        assert!(contains(&v, 9) && !contains(&v, 10));
        assert_eq!(total_len(&v), 8);
        assert_eq!(total_len(&[r(i64::MIN, i64::MAX)]), i64::MAX, "saturates");
        assert_eq!(last_end(&v), Some(10));
        assert_eq!(first_start(&v), Some(0));
        assert_eq!(last_end(&[]), None);
        assert_eq!(first_start(&[]), None);
    }

    // ---- against a bitmap ----

    const GRID: usize = 64;

    fn random_set(rng: &mut Rng) -> Vec<TimeRange> {
        let n = rng.below(6) as usize;
        let mut v = Vec::new();
        for _ in 0..n {
            let a = rng.below(GRID as u64) as i64;
            let b = (a + rng.range(-2, 16)).min(GRID as i64);
            v.push(r(a, b));
        }
        v
    }

    fn bitmap(v: &[TimeRange]) -> [bool; GRID] {
        let mut bits = [false; GRID];
        for r in v {
            for t in r.start.max(0)..r.end.min(GRID as i64) {
                bits[t as usize] = true;
            }
        }
        bits
    }

    fn from_bitmap(bits: &[bool; GRID]) -> Vec<TimeRange> {
        let mut out = Vec::new();
        let mut start = None;
        for (i, b) in bits.iter().enumerate() {
            match (b, start) {
                (true, None) => start = Some(i as i64),
                (false, Some(s)) => {
                    out.push(r(s, i as i64));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            out.push(r(s, GRID as i64));
        }
        out
    }

    #[test]
    fn random_sets_agree_with_a_bitmap() {
        for seed in 0..2000u64 {
            let mut rng = Rng(seed);
            let a = normalize(&random_set(&mut rng));
            let b = normalize(&random_set(&mut rng));
            let (ba, bb) = (bitmap(&a), bitmap(&b));
            assert_eq!(from_bitmap(&ba), a, "seed {seed}: normalize");
            let mut or = [false; GRID];
            let mut and = [false; GRID];
            let mut minus = [false; GRID];
            for i in 0..GRID {
                or[i] = ba[i] || bb[i];
                and[i] = ba[i] && bb[i];
                minus[i] = ba[i] && !bb[i];
            }
            assert_eq!(
                union(&a, &b),
                from_bitmap(&or),
                "seed {seed}: union of {a:?} {b:?}"
            );
            assert_eq!(
                intersect(&a, &b),
                from_bitmap(&and),
                "seed {seed}: intersect of {a:?} {b:?}"
            );
            assert_eq!(
                subtract(&a, &b),
                from_bitmap(&minus),
                "seed {seed}: subtract {b:?} from {a:?}"
            );
            // inserting each interval one at a time gives the normalised set
            let raw = random_set(&mut Rng(seed + 99));
            let mut built = Vec::new();
            for iv in &raw {
                insert_interval(&mut built, *iv, 0);
                assert!(is_normalized(&built), "seed {seed}: {built:?}");
            }
            assert_eq!(
                built,
                normalize(&raw),
                "seed {seed}: insert_interval of {raw:?}"
            );
            // coalescing never loses time and never merges more than the gap allows
            for gap in [0, 1, 3] {
                let c = coalesce(&a, gap);
                assert!(is_normalized(&c));
                assert!(
                    subtract(&a, &c).is_empty(),
                    "seed {seed}: coalesce lost time"
                );
                for w in c.windows(2) {
                    assert!(w[1].start - w[0].end > gap);
                }
            }
            // find agrees with the bitmap
            for t in 0..GRID as i64 {
                assert_eq!(contains(&a, t), ba[t as usize], "seed {seed}: contains {t}");
            }
        }
    }
}
