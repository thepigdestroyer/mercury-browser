// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! `MeasuredMetrics` in the headless software-renderer platform with system fonts: does the
//! `preferred-width` of a `RingTextMeasure` that was never shown give the real font's widths?
//!
//! One test function owns the platform and every measurement: Slint components are not `Send` and
//! the platform is installed once per thread.

use std::rc::Rc;
use std::time::Instant;

use mercury_ui::ring_bands::{MeasuredMetrics, metrics};
use mercury_ui::ring_geometry::{ApproxMetrics, GlyphMetrics};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter};

struct HeadlessPlatform;

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

fn width_of(metrics: &dyn GlyphMetrics, text: &str, size: f32, weight: u16) -> f32 {
    text.chars()
        .map(|ch| metrics.advance(ch, size, weight))
        .sum()
}

#[test]
fn measured_advances_are_the_real_fonts() {
    // `Err` means a platform is already set on this thread, which is fine for this test.
    let _ = slint::platform::set_platform(Box::new(HeadlessPlatform));

    let created = Instant::now();
    let measured = MeasuredMetrics::new().expect("the measuring component is made");
    println!("MeasuredMetrics::new took {:?}", created.elapsed());

    let w = measured.advance('W', 12.0, 400);
    let i = measured.advance('i', 12.0, 400);
    let m = measured.advance('m', 12.0, 400);
    let l = measured.advance('l', 12.0, 400);
    println!("advance at 12 px, 400: W={w:.3} i={i:.3} m={m:.3} l={l:.3}");
    assert!(i > 0.0, "i has an advance: {i}");
    assert!(w >= 1.5 * i, "W ({w}) is at least 1.5x i ({i})");
    assert!(m > l, "m ({m}) is wider than l ({l})");

    // The weight must reach the measuring Text: measured at 100 px (where the renderer's whole
    // pixel advances are not a big share of the width) a heavier weight is wider in every font that
    // has the faces, so a dropped weight (equal widths for every weight) fails here.
    for ch in ['a', 'e', 'o', 'n', 'W', 'i', 'm', 'R', 'x'] {
        let row: Vec<String> = [400u16, 500, 600, 700, 800, 900]
            .iter()
            .map(|weight| format!("{weight}={:.2}", measured.advance(ch, 100.0, *weight)))
            .collect();
        println!("{ch} at 100 px: {}", row.join("  "));
    }
    let sample_width = |weight: u16| {
        width_of(
            &measured,
            "Research Lisbon trip Reading Shopping",
            100.0,
            weight,
        )
    };
    let totals: Vec<(u16, f32)> = [400u16, 500, 600, 700, 800, 900]
        .iter()
        .map(|&weight| (weight, sample_width(weight)))
        .collect();
    println!("sample text at 100 px by weight: {totals:?}");
    let (regular_total, heavy_total) = (sample_width(400), sample_width(800));
    // That the weight reaches the component is proven on any font by the unit test
    // `ring_bands::metrics::tests::the_weight_reaches_the_measuring_component`; the widths can only
    // show it on a font that has a bold face, so a font with one face skips this part.
    if sample_width(400) == sample_width(900) {
        eprintln!("the system font has one face: skipping the bold width checks ({totals:?})");
    } else {
        assert_ne!(
            regular_total, heavy_total,
            "weight 800 and 400 measure the same"
        );
        assert!(
            heavy_total > regular_total * 1.02,
            "weight 800 ({heavy_total}) is not wider than 400 ({regular_total})"
        );
        for pair in totals.windows(2) {
            assert!(
                pair[1].1 >= pair[0].1 - 1e-3,
                "widths are not monotonic in the weight: {totals:?}"
            );
        }
    }

    // `advance` scales a size-independent measurement, so this checks the scaling and the cache key,
    // not the font.
    for ch in ['W', 'i', 'a'] {
        let small = measured.advance(ch, 12.0, 600);
        let big = measured.advance(ch, 24.0, 600);
        println!(
            "{ch}: 12 px = {small:.3}, 24 px = {big:.3}, ratio {:.3}",
            big / small
        );
        assert!(
            (big / small - 2.0).abs() <= 0.2,
            "{ch}: doubling the size gave x{}",
            big / small
        );
    }

    println!("name widths at 12 px, weight 600 (measured / approx / mockup flat 0.585 em):");
    for name in ["Research", "Lisbon trip", "Reading", "Shopping"] {
        let real = width_of(&measured, name, 12.0, 600);
        let approx = width_of(&ApproxMetrics, name, 12.0, 600);
        let flat = name.chars().count() as f32 * 12.0 * 0.585;
        println!(
            "  {name:<12} measured {real:6.2}  approx {approx:6.2} ({:+5.1}%)  flat {flat:6.2} ({:+5.1}%)",
            (approx / real - 1.0) * 100.0,
            (flat / real - 1.0) * 100.0
        );
        assert!(real > 0.0, "{name} measures {real}");
        assert!(
            (real - approx).abs() <= 0.35 * approx,
            "{name}: measured {real} is not within 35% of approx {approx}"
        );
    }

    let odd = [
        ' ', '\u{00A0}', '😀', '\u{0301}', '\u{200D}', '\u{FE0F}', '\n', '日', '⌥',
    ];
    for ch in odd {
        let advance = measured.advance(ch, 12.0, 600);
        println!("{ch:?}: {advance:.3}");
        assert!(
            advance.is_finite() && advance >= 0.0,
            "{ch:?} gave {advance}"
        );
    }
    assert!(
        measured.advance(' ', 12.0, 400) > 0.0,
        "a space has an advance"
    );
    for ch in ['\u{200D}', '\u{200C}', '\u{FE0F}', '\n'] {
        assert_eq!(measured.advance(ch, 12.0, 400), 0.0, "{ch:?}");
    }
    for size in [0.0, -3.0, f32::NAN, f32::INFINITY] {
        assert_eq!(measured.advance('W', size, 400), 0.0, "size {size}");
    }

    let reads = measured.component_reads();
    let first = measured.advance('Q', 13.0, 500);
    let after_first = measured.component_reads();
    assert!(after_first > reads, "a new key reads the component");
    let second = measured.advance('Q', 13.0, 500);
    assert_eq!(first, second);
    assert_eq!(
        measured.component_reads(),
        after_first,
        "a repeated question is answered from the cache"
    );
    measured.advance('Q', 13.0, 600);
    assert!(
        measured.component_reads() > after_first,
        "another weight is another key"
    );
    let reads_before = measured.component_reads();
    let scaled = measured.advance('Q', 26.0, 500);
    assert_eq!(
        measured.component_reads(),
        reads_before,
        "another size is the same measurement, scaled"
    );
    assert!(
        (scaled / first - 2.0).abs() < 1e-3,
        "26 px is twice 13 px: {scaled} vs {first}"
    );

    let first = metrics();
    let second = metrics();
    assert!(first.advance('W', 12.0, 400) > first.advance('i', 12.0, 400));
    assert!(second.advance('W', 12.0, 400) > second.advance('i', 12.0, 400));
    let measured_w = measured.advance('W', 12.0, 400);
    assert_eq!(first.advance('W', 12.0, 400), measured_w);
    assert_ne!(
        first.advance('W', 12.0, 400),
        ApproxMetrics.advance('W', 12.0, 400),
        "metrics() gave the measured implementation, not the fallback"
    );

    let started = Instant::now();
    for _ in 0..3 {
        let _ = MeasuredMetrics::new().expect("another component");
    }
    println!("3 more MeasuredMetrics::new took {:?}", started.elapsed());
}
