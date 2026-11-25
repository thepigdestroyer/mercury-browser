// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Cutting a segment into the pieces a page's network code would append.

use crate::ChunkPlan;

/// Cuts `data` into the pieces `plan` describes, without copying. The pieces always reassemble to
/// `data`; empty data gives no piece. A `Fixed(0)` size, and a `Sizes` list that is empty or ends in 0,
/// are read as "the rest in one piece"; a 0 elsewhere in `Sizes` is skipped.
pub fn chunks<'a>(data: &'a [u8], plan: &ChunkPlan) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    if data.is_empty() {
        return out;
    }
    match plan {
        ChunkPlan::Whole => out.push(data),
        ChunkPlan::Fixed(0) => out.push(data),
        ChunkPlan::Fixed(n) => out.extend(data.chunks(*n)),
        ChunkPlan::Sizes(sizes) => {
            let last = sizes.last().copied().unwrap_or(0);
            let mut rest = data;
            let mut i = 0;
            while !rest.is_empty() {
                let size = sizes.get(i).copied().unwrap_or(last);
                i += 1;
                if size == 0 {
                    if i >= sizes.len() {
                        out.push(rest);
                        break;
                    }
                    continue;
                }
                let (head, tail) = rest.split_at(size.min(rest.len()));
                out.push(head);
                rest = tail;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(parts: &[&[u8]]) -> Vec<u8> {
        parts.iter().flat_map(|p| p.iter().copied()).collect()
    }

    #[test]
    fn every_plan_reassembles() {
        let data: Vec<u8> = (0..=250u8).cycle().take(10_007).collect();
        for plan in [
            ChunkPlan::Whole,
            ChunkPlan::Fixed(1),
            ChunkPlan::Fixed(1024),
            ChunkPlan::Fixed(65_536),
            ChunkPlan::Fixed(0),
            ChunkPlan::Sizes(vec![]),
            ChunkPlan::Sizes(vec![13, 4096, 1]),
            ChunkPlan::Sizes(vec![0, 7, 0, 5]),
            ChunkPlan::Sizes(vec![3, 0]),
            ChunkPlan::Sizes(vec![20_000]),
        ] {
            let parts = chunks(&data, &plan);
            assert_eq!(joined(&parts), data, "{plan:?}");
            assert!(parts.iter().all(|p| !p.is_empty()), "{plan:?}");
        }
    }

    #[test]
    fn shapes() {
        let data = [0u8; 10];
        assert_eq!(chunks(&data, &ChunkPlan::Whole).len(), 1);
        let lens: Vec<usize> = chunks(&data, &ChunkPlan::Fixed(4))
            .iter()
            .map(|c| c.len())
            .collect();
        assert_eq!(lens, [4, 4, 2]);
        let lens: Vec<usize> = chunks(&data, &ChunkPlan::Sizes(vec![3, 2]))
            .iter()
            .map(|c| c.len())
            .collect();
        assert_eq!(
            lens,
            [3, 2, 2, 2, 1],
            "the last size repeats until the data ends"
        );
        let lens: Vec<usize> = chunks(&data, &ChunkPlan::Sizes(vec![13, 4096, 1]))
            .iter()
            .map(|c| c.len())
            .collect();
        assert_eq!(lens, [10]);
        assert!(chunks(&[], &ChunkPlan::Fixed(3)).is_empty());
    }
}
