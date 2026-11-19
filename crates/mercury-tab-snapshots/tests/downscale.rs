// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The shrink of a frame to a snapshot: the size it comes to and the pixels it keeps.

use std::time::Instant;

use mercury_tab_snapshots::{Limits, MAX_HEIGHT, MAX_WIDTH, RgbaFrame, downscale, fit_within};

const DEFAULT: Limits = Limits::DEFAULT;

fn limits(width: u32, height: u32) -> Limits {
    Limits::new(width, height).expect("both sides are above zero")
}

/// A frame whose pixel at `(x, y)` is `pixel(x, y)`.
fn frame_from(width: u32, height: u32, mut pixel: impl FnMut(u32, u32) -> [u8; 4]) -> RgbaFrame {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&pixel(x, y));
        }
    }
    RgbaFrame::new(width, height, pixels).expect("a frame of the right length")
}

fn solid(width: u32, height: u32, colour: [u8; 4]) -> RgbaFrame {
    frame_from(width, height, |_, _| colour)
}

fn pixel_at(frame: &RgbaFrame, x: u32, y: u32) -> [u8; 4] {
    assert!(
        x < frame.width() && y < frame.height(),
        "({x}, {y}) is outside the frame"
    );
    let start = (y as usize * frame.width() as usize + x as usize) * 4;
    let bytes = &frame.pixels()[start..start + 4];
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

fn all_pixels(frame: &RgbaFrame) -> impl Iterator<Item = [u8; 4]> + '_ {
    frame.pixels().as_chunks::<4>().0.iter().copied()
}

// fit_within

#[test]
fn a_laptop_page_area_shrinks_to_the_box() {
    assert_eq!(fit_within(1440, 900, DEFAULT), (480, 300));
    assert_eq!(fit_within(2880, 1800, DEFAULT), (480, 300));
}

#[test]
fn a_wide_picture_is_limited_by_its_width() {
    assert_eq!(fit_within(1920, 1080, DEFAULT), (480, 270));
}

#[test]
fn a_tall_picture_is_limited_by_its_height() {
    assert_eq!(fit_within(600, 1200, DEFAULT), (150, 300));
}

#[test]
fn a_picture_that_fits_is_never_enlarged() {
    assert_eq!(fit_within(300, 200, DEFAULT), (300, 200));
    assert_eq!(fit_within(480, 300, DEFAULT), (480, 300));
    assert_eq!(fit_within(1, 1, DEFAULT), (1, 1));
    assert_eq!(fit_within(480, 1, DEFAULT), (480, 1));
    assert_eq!(fit_within(1, 300, DEFAULT), (1, 300));
}

#[test]
fn a_strip_keeps_at_least_one_pixel_on_its_thin_side() {
    assert_eq!(fit_within(1, 4000, DEFAULT), (1, 300));
    assert_eq!(fit_within(4000, 1, DEFAULT), (480, 1));
}

#[test]
fn a_picture_one_pixel_over_the_limit_comes_down_to_the_limit() {
    assert_eq!(fit_within(481, 1, DEFAULT), (480, 1));
    assert_eq!(fit_within(1, 301, DEFAULT), (1, 300));
    assert_eq!(fit_within(481, 300, DEFAULT), (480, 299));
    assert_eq!(fit_within(480, 301, DEFAULT), (478, 300));
    assert_eq!(fit_within(481, 301, DEFAULT), (479, 300));
}

#[test]
fn a_size_with_a_zero_side_gives_one_by_one() {
    assert_eq!(fit_within(0, 0, DEFAULT), (1, 1));
    assert_eq!(fit_within(0, 900, DEFAULT), (1, 1));
    assert_eq!(fit_within(1440, 0, DEFAULT), (1, 1));
}

#[test]
fn the_limits_are_a_parameter() {
    let large = limits(960, 600);
    assert_eq!(fit_within(1440, 900, large), (960, 600));
    assert_eq!(fit_within(1920, 1080, large), (960, 540));
    assert_eq!(fit_within(800, 500, large), (800, 500));
    assert_eq!(fit_within(1440, 900, limits(1, 1)), (1, 1));
    assert_eq!(fit_within(1440, 900, limits(100, 10)), (16, 10));
}

#[test]
fn the_default_limits_are_the_documented_constants() {
    assert_eq!((MAX_WIDTH, MAX_HEIGHT), (480, 300));
    assert_eq!(DEFAULT.max_width(), MAX_WIDTH);
    assert_eq!(DEFAULT.max_height(), MAX_HEIGHT);
    assert_eq!(Limits::default(), DEFAULT);
    assert_eq!(Limits::new(0, 300), None);
    assert_eq!(Limits::new(480, 0), None);
}

#[test]
fn a_huge_size_does_not_overflow() {
    assert_eq!(fit_within(u32::MAX, u32::MAX, DEFAULT), (300, 300));
    assert_eq!(fit_within(u32::MAX, 1, DEFAULT), (480, 1));
    assert_eq!(fit_within(1, u32::MAX, DEFAULT), (1, 300));
    assert_eq!(
        fit_within(u32::MAX, u32::MAX, limits(u32::MAX, u32::MAX)),
        (u32::MAX, u32::MAX)
    );
}

#[test]
fn a_sweep_of_sizes_fits_the_limits_and_keeps_the_proportions() {
    let sizes = [
        1u32, 2, 3, 4, 5, 7, 10, 59, 100, 149, 150, 151, 200, 299, 300, 301, 333, 479, 480, 481,
        500, 599, 600, 601, 640, 767, 768, 800, 1000, 1080, 1280, 1440, 1600, 1920, 2560, 2880,
        3840, 4000, 5120, 7680, 16_384,
    ];
    let all_limits = [
        DEFAULT,
        limits(960, 600),
        limits(1, 1),
        limits(200, 50),
        limits(7, 1000),
        limits(4096, 4096),
    ];
    for limits in all_limits {
        let (max_width, max_height) = (limits.max_width(), limits.max_height());
        for width in sizes {
            for height in sizes {
                let (new_width, new_height) = fit_within(width, height, limits);
                let context = format!(
                    "{width}x{height} in {max_width}x{max_height} gave {new_width}x{new_height}"
                );

                assert!(
                    new_width >= 1 && new_height >= 1,
                    "a side is zero: {context}"
                );
                assert!(
                    new_width <= max_width && new_height <= max_height,
                    "over the limits: {context}"
                );
                assert!(
                    new_width <= width && new_height <= height,
                    "enlarged: {context}"
                );

                // The ideal size is the source times one scale that fits the box and never
                // enlarges; each side of the result is within one pixel of it (the thin side of a
                // strip is lifted to 1, which is the largest possible gap).
                let scale = (f64::from(max_width) / f64::from(width))
                    .min(f64::from(max_height) / f64::from(height))
                    .min(1.0);
                let ideal_width = f64::from(width) * scale;
                let ideal_height = f64::from(height) * scale;
                assert!(
                    (f64::from(new_width) - ideal_width).abs() <= 1.0,
                    "width off by more than one: {context}"
                );
                assert!(
                    (f64::from(new_height) - ideal_height).abs() <= 1.0,
                    "height off by more than one: {context}"
                );

                // The side that decides the scale reaches its limit (or the picture is not shrunk).
                let shrunk = scale < 1.0;
                if shrunk {
                    assert!(
                        new_width == max_width
                            || new_height == max_height
                            || new_width == 1
                            || new_height == 1,
                        "neither side uses the box: {context}"
                    );
                } else {
                    assert_eq!(
                        (new_width, new_height),
                        (width, height),
                        "a fitting size changed: {context}"
                    );
                }
            }
        }
    }
}

// downscale: the size

#[test]
fn a_big_frame_comes_out_at_the_size_fit_within_gives() {
    for (width, height) in [
        (1440, 900),
        (1920, 1080),
        (601, 1201),
        (1, 4000),
        (4000, 1),
        (481, 301),
    ] {
        let shrunk = downscale(solid(width, height, [9, 9, 9, 255]), DEFAULT);
        let expected = fit_within(width, height, DEFAULT);
        assert_eq!(
            (shrunk.width(), shrunk.height()),
            expected,
            "{width}x{height}"
        );
    }
}

#[test]
fn the_pixel_buffer_always_matches_the_size() {
    let sizes = [1u32, 2, 3, 6, 7, 299, 300, 301, 479, 480, 481, 719, 1000];
    for width in sizes {
        for height in sizes {
            let shrunk = downscale(solid(width, height, [1, 2, 3, 4]), DEFAULT);
            assert_eq!(
                shrunk.pixels().len(),
                shrunk.width() as usize * shrunk.height() as usize * 4,
                "{width}x{height}"
            );
            assert_eq!(shrunk.byte_len(), shrunk.pixels().len());
            assert!(shrunk.width() >= 1 && shrunk.height() >= 1);
        }
    }
}

#[test]
fn the_limits_given_to_downscale_are_used() {
    let shrunk = downscale(solid(1920, 1080, [5, 6, 7, 255]), limits(960, 600));
    assert_eq!((shrunk.width(), shrunk.height()), (960, 540));
    let tiny = downscale(solid(1920, 1080, [5, 6, 7, 255]), limits(1, 1));
    assert_eq!((tiny.width(), tiny.height()), (1, 1));
    assert_eq!(pixel_at(&tiny, 0, 0), [5, 6, 7, 255]);
}

// downscale: the colours

#[test]
fn a_solid_colour_stays_exactly_that_colour() {
    let colours = [
        [0, 0, 0, 255],
        [255, 255, 255, 255],
        [254, 1, 128, 255],
        [200, 17, 99, 255],
        [13, 255, 254, 255],
    ];
    // Sizes that divide evenly and sizes that do not, a strip, and a frame barely over the limit.
    let sizes = [
        (1440, 900),
        (1001, 601),
        (481, 301),
        (4000, 1),
        (1, 4000),
        (481, 1),
        (999, 7),
    ];
    for colour in colours {
        for (width, height) in sizes {
            let shrunk = downscale(solid(width, height, colour), DEFAULT);
            assert!(
                all_pixels(&shrunk).all(|pixel| pixel == colour),
                "{width}x{height} of {colour:?} came out with another colour"
            );
        }
    }
}

#[test]
fn four_by_four_to_two_by_two_gives_the_exact_block_averages() {
    // Each 2x2 block holds four values whose sum divides by four, per channel.
    let blocks: [[[u8; 3]; 4]; 4] = [
        // top left
        [[10, 0, 100], [20, 0, 100], [30, 0, 100], [40, 0, 100]],
        // top right
        [
            [255, 255, 255],
            [255, 255, 255],
            [255, 255, 255],
            [255, 255, 255],
        ],
        // bottom left
        [[0, 4, 8], [0, 8, 4], [4, 0, 8], [4, 4, 0]],
        // bottom right
        [[100, 50, 0], [200, 150, 0], [100, 50, 255], [200, 150, 253]],
    ];
    let frame = frame_from(4, 4, |x, y| {
        let block = (y / 2 * 2 + x / 2) as usize;
        let within = (y % 2 * 2 + x % 2) as usize;
        let [r, g, b] = blocks[block][within];
        [r, g, b, 255]
    });

    let shrunk = downscale(frame, limits(2, 2));

    assert_eq!((shrunk.width(), shrunk.height()), (2, 2));
    assert_eq!(pixel_at(&shrunk, 0, 0), [25, 0, 100, 255]);
    assert_eq!(pixel_at(&shrunk, 1, 0), [255, 255, 255, 255]);
    assert_eq!(pixel_at(&shrunk, 0, 1), [2, 4, 5, 255]);
    assert_eq!(pixel_at(&shrunk, 1, 1), [150, 100, 127, 255]);
}

#[test]
fn a_black_and_white_checker_averages_to_mid_grey() {
    // The average is of the stored values, not of linear light (which would give about 188).
    let checker = frame_from(2, 2, |x, y| {
        if (x + y) % 2 == 0 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });

    let shrunk = downscale(checker, limits(1, 1));

    assert_eq!((shrunk.width(), shrunk.height()), (1, 1));
    let [red, green, blue, alpha] = pixel_at(&shrunk, 0, 0);
    assert!(red == 127 || red == 128, "red is {red}");
    assert_eq!((green, blue, alpha), (red, red, 255));
}

#[test]
fn the_top_of_the_picture_stays_on_top() {
    let red = [255, 0, 0, 255];
    let blue = [0, 0, 255, 255];
    let frame = frame_from(600, 400, |_, y| if y < 200 { red } else { blue });

    let shrunk = downscale(frame, DEFAULT);

    assert_eq!((shrunk.width(), shrunk.height()), (450, 300));
    for x in [0, 1, 224, 448, 449] {
        // Rows well away from the middle are the pure colours; the first and last row too.
        for y in [0, 1, 50, 140] {
            assert_eq!(pixel_at(&shrunk, x, y), red, "({x}, {y}) should be red");
        }
        for y in [160, 250, 298, 299] {
            assert_eq!(pixel_at(&shrunk, x, y), blue, "({x}, {y}) should be blue");
        }
    }
}

#[test]
fn the_left_of_the_picture_stays_on_the_left() {
    let green = [0, 255, 0, 255];
    let magenta = [255, 0, 255, 255];
    let frame = frame_from(800, 500, |x, _| if x < 400 { green } else { magenta });

    let shrunk = downscale(frame, DEFAULT);

    assert_eq!((shrunk.width(), shrunk.height()), (480, 300));
    for y in [0, 1, 150, 298, 299] {
        for x in [0, 1, 100, 230] {
            assert_eq!(pixel_at(&shrunk, x, y), green, "({x}, {y}) should be green");
        }
        for x in [250, 380, 478, 479] {
            assert_eq!(
                pixel_at(&shrunk, x, y),
                magenta,
                "({x}, {y}) should be magenta"
            );
        }
    }
}

#[test]
fn a_gradient_stays_in_order_after_the_shrink() {
    // Red grows to the right, green grows downwards; the awkward size makes uneven boxes.
    let frame = frame_from(1001, 667, |x, y| {
        [(x * 255 / 1000) as u8, (y * 255 / 666) as u8, 40, 255]
    });

    let shrunk = downscale(frame, DEFAULT);

    let (width, height) = (shrunk.width(), shrunk.height());
    assert_eq!((width, height), fit_within(1001, 667, DEFAULT));
    for y in 0..height {
        for x in 1..width {
            assert!(
                pixel_at(&shrunk, x, y)[0] >= pixel_at(&shrunk, x - 1, y)[0],
                "red falls at ({x}, {y})"
            );
        }
    }
    for x in 0..width {
        for y in 1..height {
            assert!(
                pixel_at(&shrunk, x, y)[1] >= pixel_at(&shrunk, x, y - 1)[1],
                "green falls at ({x}, {y})"
            );
        }
    }
    assert!(pixel_at(&shrunk, 0, 0)[0] <= 2);
    assert!(pixel_at(&shrunk, width - 1, 0)[0] >= 253);
    assert!(pixel_at(&shrunk, 0, 0)[1] <= 2);
    assert!(pixel_at(&shrunk, 0, height - 1)[1] >= 253);
    assert!(all_pixels(&shrunk).all(|pixel| pixel[2] == 40));
}

// downscale: the alpha

#[test]
fn a_transparent_input_becomes_opaque_with_the_same_colours() {
    for alpha in [0u8, 1, 128, 254] {
        let shrunk = downscale(solid(1440, 900, [10, 120, 230, alpha]), DEFAULT);

        assert_eq!((shrunk.width(), shrunk.height()), (480, 300));
        assert!(
            all_pixels(&shrunk).all(|pixel| pixel == [10, 120, 230, 255]),
            "alpha {alpha} changed the result"
        );
    }
}

#[test]
fn alpha_does_not_weigh_the_colours_of_the_pixels_around_it() {
    // A fully transparent red pixel next to an opaque blue one still counts as red:
    // the page picture has no transparency, so the stored colours are averaged as they are.
    let frame = frame_from(2, 1, |x, _| {
        if x == 0 {
            [255, 0, 0, 0]
        } else {
            [0, 0, 255, 255]
        }
    });

    let shrunk = downscale(frame, limits(1, 1));

    let [red, green, blue, alpha] = pixel_at(&shrunk, 0, 0);
    assert!(red == 127 || red == 128, "red is {red}");
    assert!(blue == 127 || blue == 128, "blue is {blue}");
    assert_eq!((green, alpha), (0, 255));
}

#[test]
fn a_frame_that_fits_keeps_its_size_and_colours_and_becomes_opaque() {
    for (width, height) in [(300, 200), (480, 300), (1, 1), (480, 1), (1, 300)] {
        let frame = frame_from(width, height, |x, y| {
            [
                (x * 7 + y) as u8,
                (x + y * 13) as u8,
                (x ^ y) as u8,
                ((x * 31 + y * 17) % 256) as u8,
            ]
        });
        let original = frame.clone();

        let shrunk = downscale(frame, DEFAULT);

        assert_eq!((shrunk.width(), shrunk.height()), (width, height));
        for y in 0..height {
            for x in 0..width {
                let [red, green, blue, _] = pixel_at(&original, x, y);
                assert_eq!(
                    pixel_at(&shrunk, x, y),
                    [red, green, blue, 255],
                    "({x}, {y}) of {width}x{height}"
                );
            }
        }
    }
}

// downscale: the details a nearest-neighbour sampler loses

#[test]
fn a_one_pixel_dark_line_survives_the_shrink() {
    // 2880x1800 comes down by exactly 6. The lines sit on rows and columns that are neither the
    // first nor the middle of a box of six, which is where a sampler that picks one pixel looks.
    let horizontal = frame_from(2880, 1800, |_, y| {
        if y == 902 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    let vertical = frame_from(2880, 1800, |x, _| {
        if x == 1000 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });

    for (name, frame) in [("horizontal", horizontal), ("vertical", vertical)] {
        let shrunk = downscale(frame, DEFAULT);

        assert_eq!((shrunk.width(), shrunk.height()), (480, 300), "{name}");
        let darkest = all_pixels(&shrunk)
            .map(|pixel| pixel[0])
            .min()
            .expect("pixels");
        // One pixel of six is black: five sixths of white, about 212.
        assert!(darkest < 255, "the {name} line is gone");
        assert!(
            (210..=215).contains(&darkest),
            "the {name} line is {darkest}, expected about 212"
        );
    }
}

#[test]
fn a_thin_line_does_not_vanish_at_an_uneven_ratio_either() {
    let frame = frame_from(1001, 667, |x, _| {
        if x == 500 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });

    let shrunk = downscale(frame, DEFAULT);

    let darkest = all_pixels(&shrunk)
        .map(|pixel| pixel[0])
        .min()
        .expect("pixels");
    assert!(darkest < 255, "the line is gone");
}

#[test]
fn a_one_pixel_strip_is_shrunk_along_its_long_side_only() {
    let tall = frame_from(1, 4000, |_, y| [(y / 16) as u8, 0, 0, 255]);
    let shrunk = downscale(tall, DEFAULT);
    assert_eq!((shrunk.width(), shrunk.height()), (1, 300));
    assert_eq!(pixel_at(&shrunk, 0, 0)[0], 0);
    assert!(pixel_at(&shrunk, 0, 299)[0] >= 248);
    for y in 1..300 {
        assert!(pixel_at(&shrunk, 0, y)[0] >= pixel_at(&shrunk, 0, y - 1)[0]);
    }

    let wide = frame_from(4000, 1, |x, _| [0, (x / 16) as u8, 0, 255]);
    let shrunk = downscale(wide, DEFAULT);
    assert_eq!((shrunk.width(), shrunk.height()), (480, 1));
    assert_eq!(pixel_at(&shrunk, 0, 0)[1], 0);
    assert!(pixel_at(&shrunk, 479, 0)[1] >= 245);
    for x in 1..480 {
        assert!(pixel_at(&shrunk, x, 0)[1] >= pixel_at(&shrunk, x - 1, 0)[1]);
    }
}

#[test]
fn a_frame_barely_over_the_limit_keeps_its_ramp_in_order() {
    // 481 columns into 480: a ramp of the column number must keep growing from the first output
    // pixel to the last, with no output pixel left out or repeated.
    let frame = frame_from(481, 1, |x, _| [(x * 255 / 480) as u8, 0, 0, 255]);

    let shrunk = downscale(frame, DEFAULT);

    assert_eq!((shrunk.width(), shrunk.height()), (480, 1));
    assert_eq!(pixel_at(&shrunk, 0, 0)[0], 0);
    assert!(pixel_at(&shrunk, 479, 0)[0] >= 254);
    for x in 1..480 {
        assert!(
            pixel_at(&shrunk, x, 0)[0] >= pixel_at(&shrunk, x - 1, 0)[0],
            "red falls at {x}"
        );
    }
}

// downscale: every source pixel counts, once

/// Shrinks a `width` by `height` strip (one of them is 1) once for every source pixel, with that
/// pixel white and the others black, and checks that the white lights exactly one new pixel, that
/// the lit pixel never moves back as the source pixel moves on, and that every new pixel is lit
/// by some source pixel: the blocks tile the strip, with no source pixel dropped or counted twice.
fn assert_every_pixel_of_a_strip_counts_once(width: u32, height: u32) {
    let (new_width, new_height) = fit_within(width, height, DEFAULT);
    let mut lit_by_some_pixel = vec![false; new_width as usize * new_height as usize];
    let mut previous = 0;
    for at in 0..width.max(height) {
        let strip = frame_from(width, height, |x, y| {
            if x + y == at {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        });

        let shrunk = downscale(strip, DEFAULT);

        let lit: Vec<usize> = all_pixels(&shrunk)
            .enumerate()
            .filter(|(_, pixel)| pixel[0] > 0)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(
            lit.len(),
            1,
            "source pixel {at} of a {width}x{height} strip lit the new pixels {lit:?}"
        );
        assert!(
            lit[0] >= previous,
            "source pixel {at} of a {width}x{height} strip lit new pixel {}, after {previous}",
            lit[0]
        );
        previous = lit[0];
        lit_by_some_pixel[lit[0]] = true;
    }
    assert!(
        lit_by_some_pixel.iter().all(|lit| *lit),
        "a new pixel of the {width}x{height} strip has no source pixel"
    );
}

#[test]
fn every_column_of_a_wide_strip_counts_in_one_new_pixel() {
    // Widths where a block edge falls exactly on a whole source pixel (512, 1024 and 1152 among
    // them) are where a floating-point box filter drops a column.
    for width in [
        481, 490, 512, 516, 520, 576, 752, 960, 972, 980, 1010, 1024, 1032, 1040, 1092, 1100, 1152,
        1280, 1366, 1440, 1536, 1600, 1920, 2560, 2880, 3840, 4000,
    ] {
        assert_every_pixel_of_a_strip_counts_once(width, 1);
    }
}

#[test]
fn every_row_of_a_tall_strip_counts_in_one_new_pixel() {
    for height in [
        301, 320, 360, 400, 512, 600, 768, 900, 1024, 1080, 1200, 1440, 1800, 2160, 4000,
    ] {
        assert_every_pixel_of_a_strip_counts_once(1, height);
    }
}

/// The contract of `downscale`, written out slowly: new pixel `(i, j)` is the average, rounded to
/// the nearest, of the source block from `i * width / new_width` and `j * height / new_height`
/// (integer division) up to the next block's start, per colour channel, with alpha 255.
fn naive_shrink(frame: &RgbaFrame, new_width: u32, new_height: u32) -> Vec<[u8; 4]> {
    let (width, height) = (frame.width() as usize, frame.height() as usize);
    let (new_width, new_height) = (new_width as usize, new_height as usize);
    let mut shrunk = Vec::new();
    for j in 0..new_height {
        for i in 0..new_width {
            let columns = i * width / new_width..(i + 1) * width / new_width;
            let rows = j * height / new_height..(j + 1) * height / new_height;
            let count = (columns.len() * rows.len()) as u64;
            let mut sums = [0u64; 3];
            for y in rows {
                for x in columns.clone() {
                    let start = (y * width + x) * 4;
                    for (sum, value) in sums.iter_mut().zip(&frame.pixels()[start..start + 3]) {
                        *sum += u64::from(*value);
                    }
                }
            }
            let average = |sum: u64| ((sum + count / 2) / count) as u8;
            shrunk.push([average(sums[0]), average(sums[1]), average(sums[2]), 255]);
        }
    }
    shrunk
}

#[test]
fn the_shrink_equals_the_slow_definition_on_noisy_frames() {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut noise = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 56) as u8
    };
    for (width, height) in [
        (1001, 667),
        (487, 305),
        (481, 301),
        (1920, 1080),
        (600, 1200),
        (1024, 768),
        (977, 1),
        (1, 977),
        (480, 400),
        (640, 300),
    ] {
        let frame = frame_from(width, height, |_, _| [noise(), noise(), noise(), noise()]);
        let (new_width, new_height) = fit_within(width, height, DEFAULT);
        let expected = naive_shrink(&frame, new_width, new_height);

        let shrunk = downscale(frame, DEFAULT);

        assert_eq!((shrunk.width(), shrunk.height()), (new_width, new_height));
        let got: Vec<[u8; 4]> = all_pixels(&shrunk).collect();
        assert!(
            got == expected,
            "{width}x{height} differs from the slow definition"
        );
    }
}

// Timing, run by hand: cargo test --release -p mercury-tab-snapshots --test downscale -- --ignored --nocapture

#[test]
#[ignore = "a timing measurement for the report; run it by hand in release"]
fn timing_of_the_largest_expected_frame() {
    // A busy page: colours that change from pixel to pixel, so no row is a copy of another.
    let frame = frame_from(2880, 1800, |x, y| {
        [(x * 3 + y) as u8, (x + y * 5) as u8, (x ^ y) as u8, 255]
    });

    let mut times = Vec::new();
    for _ in 0..9 {
        let copy = frame.clone();
        let started = Instant::now();
        let shrunk = downscale(copy, DEFAULT);
        times.push(started.elapsed());
        assert_eq!((shrunk.width(), shrunk.height()), (480, 300));
    }
    times.sort();
    println!(
        "downscale 2880x1800 -> 480x300: median {:.2} ms, fastest {:.2} ms, slowest {:.2} ms (target: under 40 ms)",
        times[times.len() / 2].as_secs_f64() * 1000.0,
        times[0].as_secs_f64() * 1000.0,
        times[times.len() - 1].as_secs_f64() * 1000.0,
    );
}
