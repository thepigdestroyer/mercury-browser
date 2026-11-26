// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Screen-geometry rounding (R14c MH49, spec §5.7).

use mercury_protection::{COMMON_SCREEN_SIZES, Geometry, Point, Setting, Size, round_geometry};

use Setting::{Off, Standard, Strict};

const ORIGIN: Point = Point { x: 0, y: 0 };
const CASES: u32 = 20_000;
const SEED: u64 = 0x6d65_7263_7572_7934;

fn size(width: u32, height: u32) -> Size {
    Size { width, height }
}

/// A geometry as a page would see it: the available area below a menu bar, and the window at (80, 60).
fn geometry(screen: Size, available: Size, window: Size) -> Geometry {
    Geometry {
        screen,
        available_origin: Point { x: 0, y: 25 },
        available,
        window_origin: Point { x: 80, y: 60 },
        window,
    }
}

/// The expected rounded geometry: the available area is the screen and both origins are (0, 0).
fn rounded(screen: Size, window: Size) -> Geometry {
    Geometry {
        screen,
        available_origin: ORIGIN,
        available: screen,
        window_origin: ORIGIN,
        window,
    }
}

/// SplitMix64: a seeded generator, so the property cases are the same on every run.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A value in `low..=high`.
    fn range(&mut self, low: u32, high: u32) -> u32 {
        low + (self.next_u64() % u64::from(high - low + 1)) as u32
    }
}

fn random_geometry(rng: &mut Rng) -> Geometry {
    let origin = Point {
        x: rng.range(0, 400) as i32 - 200,
        y: rng.range(0, 400) as i32 - 200,
    };
    Geometry {
        screen: size(rng.range(100, 8000), rng.range(100, 8000)),
        available_origin: origin,
        available: size(rng.range(100, 8000), rng.range(100, 8000)),
        window_origin: origin,
        window: size(rng.range(100, 8000), rng.range(100, 8000)),
    }
}

fn dimensions(g: Geometry) -> [u32; 6] {
    [
        g.screen.width,
        g.screen.height,
        g.available.width,
        g.available.height,
        g.window.width,
        g.window.height,
    ]
}

#[test]
fn off_returns_the_input() {
    let g = geometry(size(1728, 1117), size(1728, 1050), size(1200, 800));
    assert_eq!(round_geometry(g, Off), g);
}

#[test]
fn standard_rounds_the_screen_and_the_window_and_places_both_at_the_origin() {
    let g = geometry(size(1728, 1117), size(1728, 1050), size(1200, 800));
    assert_eq!(
        round_geometry(g, Standard),
        rounded(size(1700, 1100), size(1200, 800))
    );
}

#[test]
fn standard_rounds_a_common_laptop_screen_and_window() {
    let g = geometry(size(1470, 956), size(1470, 931), size(1013, 677));
    assert_eq!(
        round_geometry(g, Standard),
        rounded(size(1400, 900), size(1000, 600))
    );
}

#[test]
fn standard_keeps_the_screen_at_least_100_by_100_and_the_window_at_least_200_by_100() {
    let small = geometry(size(640, 480), size(640, 455), size(120, 90));
    assert_eq!(
        round_geometry(small, Standard),
        rounded(size(600, 400), size(200, 100))
    );

    let tiny = geometry(size(50, 50), size(50, 25), size(50, 50));
    assert_eq!(
        round_geometry(tiny, Standard),
        rounded(size(100, 100), size(100, 100))
    );
}

#[test]
fn strict_picks_the_largest_common_size_that_fits_the_screen() {
    let big = geometry(size(1728, 1117), size(1728, 1050), size(1200, 800));
    assert_eq!(
        round_geometry(big, Strict),
        rounded(size(1680, 1050), size(1200, 800))
    );

    let exact = geometry(size(1366, 768), size(1366, 743), size(1200, 800));
    assert_eq!(
        round_geometry(exact, Strict),
        rounded(size(1366, 768), size(1200, 700))
    );

    let between = geometry(size(1360, 760), size(1360, 735), size(1200, 800));
    assert_eq!(
        round_geometry(between, Strict),
        rounded(size(1280, 720), size(1200, 700))
    );
}

#[test]
fn strict_falls_back_to_the_standard_result_when_no_common_size_fits() {
    let g = geometry(size(800, 600), size(800, 575), size(1200, 800));
    assert_eq!(round_geometry(g, Strict), round_geometry(g, Standard));
    assert_eq!(
        round_geometry(g, Strict),
        rounded(size(800, 600), size(800, 600))
    );
}

#[test]
fn rounding_is_idempotent_for_both_settings() {
    let mut rng = Rng(SEED);
    for _ in 0..CASES {
        let g = random_geometry(&mut rng);
        for setting in [Standard, Strict] {
            let once = round_geometry(g, setting);
            assert_eq!(round_geometry(once, setting), once, "{g:?} {setting:?}");
        }
    }
}

#[test]
fn rounding_is_never_larger_than_the_input_and_never_zero() {
    let mut rng = Rng(SEED ^ 1);
    for _ in 0..CASES {
        let g = random_geometry(&mut rng);
        for setting in [Standard, Strict] {
            let out = round_geometry(g, setting);
            assert!(
                out.screen.width <= g.screen.width && out.screen.height <= g.screen.height,
                "{g:?} {setting:?}"
            );
            assert!(
                out.window.width <= out.screen.width && out.window.height <= out.screen.height,
                "{g:?} {setting:?}"
            );
            for part in [out.screen, out.available, out.window] {
                assert!(part.width > 0 && part.height > 0, "{g:?} {setting:?}");
            }
        }
    }
}

#[test]
fn strict_screen_is_a_common_size_or_the_standard_fallback() {
    let mut rng = Rng(SEED ^ 2);
    for _ in 0..CASES {
        let g = random_geometry(&mut rng);
        let screen = round_geometry(g, Strict).screen;
        assert!(
            COMMON_SCREEN_SIZES.contains(&screen) || screen == round_geometry(g, Standard).screen,
            "{g:?}"
        );
    }
}

#[test]
fn standard_is_monotone_in_each_dimension() {
    let mut rng = Rng(SEED ^ 3);
    for _ in 0..CASES {
        let g = random_geometry(&mut rng);
        let bump = rng.range(0, 500);
        let mut bigger = g;
        match rng.range(0, 3) {
            0 => bigger.screen.width += bump,
            1 => bigger.screen.height += bump,
            2 => bigger.window.width += bump,
            _ => bigger.window.height += bump,
        }
        let before = dimensions(round_geometry(g, Standard));
        let after = dimensions(round_geometry(bigger, Standard));
        for (small, large) in before.iter().zip(after) {
            assert!(*small <= large, "{g:?} -> {bigger:?}");
        }
    }
}
