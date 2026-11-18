// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The bookmarks bar's overflow layout, as a pure function of the buttons' widths.

/// Pixels between two bar buttons.
pub(crate) const SPACING: f32 = 4.0;
/// The "»" button's width, and its gap to the last button shown.
pub(crate) const OVERFLOW_BUTTON: f32 = 28.0;

/// Where the bar's buttons go.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BarLayout {
    /// The x of each button shown, from the bar's left edge (`positions.len()` are shown, in
    /// order; the rest go into the "»" popup).
    pub(crate) positions: Vec<f32>,
    /// Whether the "»" button is needed.
    pub(crate) overflow: bool,
}

impl BarLayout {
    pub(crate) fn shown(&self) -> usize {
        self.positions.len()
    }
}

/// Lays out buttons of natural `widths` in `available` pixels: all of them when they fit;
/// otherwise as many as fit before the "»" button. A width not measured yet (`None`) ends the
/// buttons shown, as if it did not fit.
pub(crate) fn bar_layout(widths: &[Option<f32>], available: f32) -> BarLayout {
    // Lays out from the left while each button ends within `limit`.
    let fit = |limit: f32| {
        let mut positions = Vec::new();
        let mut x = 0.0;
        for width in widths {
            let Some(width) = width else { break };
            if x + width > limit {
                break;
            }
            positions.push(x);
            x += width + SPACING;
        }
        positions
    };
    let all = fit(available);
    if all.len() == widths.len() {
        return BarLayout {
            positions: all,
            overflow: false,
        };
    }
    BarLayout {
        positions: fit(available - OVERFLOW_BUTTON - SPACING),
        overflow: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(values: &[f32]) -> Vec<Option<f32>> {
        values.iter().copied().map(Some).collect()
    }

    #[test]
    fn everything_that_fits_is_shown_without_the_overflow_button() {
        let layout = bar_layout(&widths(&[100.0, 50.0, 80.0]), 500.0);
        assert_eq!(layout.positions, vec![0.0, 104.0, 158.0]);
        assert!(!layout.overflow);
    }

    #[test]
    fn an_exact_fit_needs_no_overflow_button() {
        let layout = bar_layout(&widths(&[100.0, 100.0]), 204.0);
        assert_eq!(layout.shown(), 2);
        assert!(!layout.overflow);
    }

    #[test]
    fn what_does_not_fit_goes_behind_the_overflow_button() {
        // 100 + 4 + 100 = 204 fits in 300 - 28 - 4 = 268; a third button would end at 308.
        let layout = bar_layout(&widths(&[100.0, 100.0, 100.0, 10.0]), 300.0);
        assert_eq!(layout.positions, vec![0.0, 104.0]);
        assert!(layout.overflow);
    }

    #[test]
    fn the_overflow_button_takes_room_from_the_last_button_that_would_fit() {
        // All four would fit in 320 (100+4+100+4+100 = 308 ... + 4 + 10 = 322 does not).
        let layout = bar_layout(&widths(&[100.0, 100.0, 100.0, 10.0]), 320.0);
        assert_eq!(layout.shown(), 2);
        assert!(layout.overflow);
    }

    #[test]
    fn later_small_buttons_do_not_jump_ahead_of_a_wide_one() {
        let layout = bar_layout(&widths(&[50.0, 400.0, 20.0]), 300.0);
        assert_eq!(layout.shown(), 1);
        assert!(layout.overflow);
    }

    #[test]
    fn a_bar_too_narrow_for_anything_shows_only_the_overflow_button() {
        let layout = bar_layout(&widths(&[100.0]), 60.0);
        assert_eq!(layout.shown(), 0);
        assert!(layout.overflow);
        let none = bar_layout(&widths(&[100.0]), 0.0);
        assert_eq!(none.shown(), 0);
    }

    #[test]
    fn an_empty_bar_has_no_overflow() {
        assert_eq!(
            bar_layout(&[], 300.0),
            BarLayout {
                positions: Vec::new(),
                overflow: false,
            }
        );
    }

    #[test]
    fn an_unmeasured_button_ends_the_buttons_shown() {
        let layout = bar_layout(&[Some(50.0), None, Some(50.0)], 1000.0);
        assert_eq!(layout.shown(), 1);
        assert!(layout.overflow);
    }
}
