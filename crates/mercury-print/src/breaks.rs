// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Break selection: which rows may start the next printed page.

/// How good a break before a row is. `Forced` always ends the page there. `Cheap` is a measured
/// break (lower is better, `Cheap(0)` is a blank row). `Allowed` is a break nobody measured.
/// `Avoid` is used only when no other break fits the search window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakCost {
    Forced,
    Cheap(u16),
    Allowed,
    Avoid,
}

/// A possible break before the row at `at` (absolute, in device pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakCandidate {
    pub at: u32,
    pub cost: BreakCost,
}

impl BreakCost {
    fn rank(self) -> u32 {
        match self {
            BreakCost::Cheap(cost) => u32::from(cost),
            BreakCost::Allowed => u32::from(u16::MAX) + 1,
            BreakCost::Avoid => u32::from(u16::MAX) + 2,
            BreakCost::Forced => u32::MAX,
        }
    }
}

/// Start rows of every page after the first, for a document of `total` rows cut into pages of
/// `page` rows. A break is looked for in the last `search` rows of each page.
pub fn select_breaks(
    candidates: &[BreakCandidate],
    total: u32,
    page: u32,
    search: u32,
) -> Vec<u32> {
    let mut sorted = candidates.to_vec();
    sorted.sort_by_key(|c| c.at);
    let mut starts = Vec::new();
    let mut top = 0;
    while let Some(next) = next_break(&sorted, top, total, page, search) {
        starts.push(next);
        top = next;
    }
    starts
}

/// The row that starts the page after the one beginning at `top`, or `None` when the rest of the
/// document (`total` rows) fits on that page. `sorted` must be ascending by `at`.
pub fn next_break(
    sorted: &[BreakCandidate],
    top: u32,
    total: u32,
    page: u32,
    search: u32,
) -> Option<u32> {
    let limit = top.saturating_add(page.max(1));
    let from = sorted.partition_point(|c| c.at <= top);
    let to = sorted.partition_point(|c| c.at <= limit);
    if let Some(forced) = sorted[from..to]
        .iter()
        .find(|c| c.cost == BreakCost::Forced && c.at < total)
    {
        return Some(forced.at);
    }
    if limit >= total {
        return None;
    }
    let window_start = limit.saturating_sub(search).max(top.saturating_add(1));
    let first = sorted.partition_point(|c| c.at < window_start);
    let best = sorted[first..to]
        .iter()
        .min_by_key(|c| (c.cost.rank(), limit - c.at));
    Some(best.map_or(limit, |c| c.at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cheap(at: u32, cost: u16) -> BreakCandidate {
        BreakCandidate {
            at,
            cost: BreakCost::Cheap(cost),
        }
    }

    #[test]
    fn no_candidates_cuts_at_page_height() {
        assert_eq!(select_breaks(&[], 2500, 1000, 100), vec![1000, 2000]);
    }

    #[test]
    fn prefers_blank_row_inside_the_window() {
        let candidates = [cheap(950, 0), cheap(990, 500)];
        let starts = select_breaks(&candidates, 3000, 1000, 100);
        assert_eq!(starts, vec![950, 1950, 2950]);
    }

    #[test]
    fn ties_go_to_the_lowest_cost_then_nearest_the_end() {
        let candidates = [cheap(920, 0), cheap(980, 0), cheap(990, 1)];
        assert_eq!(select_breaks(&candidates, 2500, 1000, 100)[0], 980);
    }

    #[test]
    fn forced_break_ends_the_page() {
        let candidates = [BreakCandidate {
            at: 400,
            cost: BreakCost::Forced,
        }];
        assert_eq!(
            select_breaks(&candidates, 2500, 1000, 100),
            vec![400, 1400, 2400]
        );
    }

    #[test]
    fn forced_at_exact_top_is_ignored() {
        let candidates = [
            BreakCandidate {
                at: 0,
                cost: BreakCost::Forced,
            },
            BreakCandidate {
                at: 1000,
                cost: BreakCost::Forced,
            },
        ];
        assert_eq!(
            select_breaks(&candidates, 2500, 1000, 100),
            vec![1000, 2000]
        );
    }

    #[test]
    fn avoid_is_used_only_when_nothing_else() {
        let candidates = [
            BreakCandidate {
                at: 950,
                cost: BreakCost::Avoid,
            },
            BreakCandidate {
                at: 900,
                cost: BreakCost::Allowed,
            },
        ];
        assert_eq!(select_breaks(&candidates, 2500, 1000, 100)[0], 900);
        let alone = [BreakCandidate {
            at: 950,
            cost: BreakCost::Avoid,
        }];
        assert_eq!(select_breaks(&alone, 2500, 1000, 100)[0], 950);
    }

    #[test]
    fn search_window_is_bounded() {
        let candidates = [cheap(100, 0)];
        assert_eq!(
            select_breaks(&candidates, 2500, 1000, 100),
            vec![1000, 2000]
        );
    }

    #[test]
    fn breaks_are_strictly_increasing() {
        let candidates: Vec<BreakCandidate> =
            (0..400).map(|i| cheap(i * 7 + 3, (i % 5) as u16)).collect();
        let starts = select_breaks(&candidates, 3000, 500, 120);
        assert!(!starts.is_empty());
        assert!(starts.windows(2).all(|w| w[0] < w[1]));
        assert!(starts.iter().all(|&s| s > 0 && s < 3000));
    }

    #[test]
    fn total_smaller_than_page_gives_no_breaks() {
        assert!(select_breaks(&[], 500, 1000, 100).is_empty());
    }

    #[test]
    fn page_zero_is_treated_as_one() {
        assert_eq!(select_breaks(&[], 3, 0, 0), vec![1, 2]);
    }
}
