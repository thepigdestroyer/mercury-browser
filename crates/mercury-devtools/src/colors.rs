// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The CI smoke test's check of a screenshot of `tests/pages/colors.html`.
//!
//! The page is white with a 160x160 CSS-pixel red square at the top left, a blue one at the
//! top right, an 80 CSS-pixel green bar along the bottom and a black heading 200 CSS pixels
//! from the top. The screenshot is of the whole window, so the browser's own tab strip and
//! toolbar sit above the page; their height is not assumed. The checker finds the top of the
//! page by looking for the red square's top edge, measures the square to learn how many image
//! pixels make a CSS pixel (so any display scale and window size work), and then samples well
//! inside every region: red, blue, green, and white in the gaps between them.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::png::{self, Image};

// Geometry of tests/pages/colors.html, in CSS pixels.
const SQUARE: f64 = 160.0;
const BAR: f64 = 80.0;
const HEADING_TOP: f64 = 200.0;
/// The 40px bold heading's line box ends well before this.
const HEADING_BOTTOM: f64 = 260.0;

/// What a sample looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    White,
    Red,
    Green,
    Blue,
    Other,
}

impl Colour {
    fn name(self) -> &'static str {
        match self {
            Colour::White => "white",
            Colour::Red => "red",
            Colour::Green => "green",
            Colour::Blue => "blue",
            Colour::Other => "other",
        }
    }
}

pub fn classify([r, g, b]: [u8; 3]) -> Colour {
    if r >= 235 && g >= 235 && b >= 235 {
        Colour::White
    } else if r >= 200 && g <= 60 && b <= 60 {
        Colour::Red
    } else if g >= 200 && r <= 60 && b <= 60 {
        Colour::Green
    } else if b >= 200 && r <= 60 && g <= 60 {
        Colour::Blue
    } else {
        Colour::Other
    }
}

/// One line of the report.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// Number of consecutive pixels of `colour` starting at (x, y) and stepping by (dx, dy).
fn run_length(image: &Image, x: i64, y: i64, dx: i64, dy: i64, colour: Colour) -> i64 {
    let (mut x, mut y) = (x, y);
    let mut count = 0;
    while (0..image.width as i64).contains(&x)
        && (0..image.height as i64).contains(&y)
        && classify(image.pixel(x as usize, y as usize)) == colour
    {
        count += 1;
        x += dx;
        y += dy;
    }
    count
}

fn rgb_text([r, g, b]: [u8; 3]) -> String {
    format!("({r}, {g}, {b})")
}

/// Runs every check in order; the first three stop the run when they fail.
pub fn check(image: &Image) -> Vec<Check> {
    let mut results = Vec::new();
    let record = |results: &mut Vec<Check>, name, ok, detail| {
        results.push(Check { name, ok, detail });
        ok
    };
    let width_px = image.width as i64;
    let height_px = image.height as i64;

    // 1. The top edge of the red square is the top of the page.
    let probe_x = (width_px / 100).max(1);
    let top = (0..height_px / 2).find(|&y| {
        probe_x < width_px && classify(image.pixel(probe_x as usize, y as usize)) == Colour::Red
    });
    let top_text = top.map_or_else(|| "None".to_owned(), |top| top.to_string());
    let found = record(
        &mut results,
        "red square found at the top left",
        top.is_some(),
        format!("first red pixel in column {probe_x} at y={top_text}"),
    );
    let Some(top) = top.filter(|_| found) else {
        return results;
    };

    // 2. Its size gives the scale; it must be square.
    let middle_y = top + 2;
    let left = probe_x - run_length(image, probe_x, middle_y, -1, 0, Colour::Red) + 1;
    let width = run_length(image, left, middle_y, 1, 0, Colour::Red);
    let height = run_length(image, probe_x, top, 0, 1, Colour::Red);
    let square = (width - height).abs() <= (width / 50).max(2);
    if !record(
        &mut results,
        "red square is square and starts at the left edge",
        square && left <= (width_px / 200).max(2),
        format!("{width}x{height} image pixels, left edge at x={left}"),
    ) {
        return results;
    }
    let scale = width as f64 / SQUARE;
    let content_width = image.width as f64 / scale;
    let content_height = (height_px - top) as f64 / scale;
    if !record(
        &mut results,
        "page is large enough to check",
        (0.5..=4.0).contains(&scale)
            && content_width >= 2.0 * SQUARE + 80.0
            && content_height >= HEADING_BOTTOM + BAR + 60.0,
        format!(
            "{scale:.2} image pixels per CSS pixel; page {content_width:.0}x{content_height:.0} CSS pixels"
        ),
    ) {
        return results;
    }

    let at = |css_x: f64, css_y: f64| {
        let x = (css_x * scale).round_ties_even() as i64;
        let y = top + (css_y * scale).round_ties_even() as i64;
        (
            x.clamp(0, width_px - 1) as usize,
            y.clamp(0, height_px - 1) as usize,
        )
    };
    let radius = (scale as usize).max(1);
    let expect = |results: &mut Vec<Check>, name, colour: Colour, points: &[(f64, f64)]| {
        let mut bad = Vec::new();
        for &(css_x, css_y) in points {
            let (x, y) = at(css_x, css_y);
            let rgb = image.average(x, y, radius);
            if classify(rgb) != colour {
                bad.push(format!("({x},{y})=rgb{}", rgb_text(rgb)));
            }
        }
        let detail = if bad.is_empty() {
            format!("{} samples", points.len())
        } else {
            format!("not {} at {}", colour.name(), bad.join(", "))
        };
        record(results, name, bad.is_empty(), detail);
    };

    let cw = content_width;
    let ch = content_height;
    let inset = 0.15 * SQUARE;
    let near = [inset, SQUARE / 2.0, SQUARE - inset];
    let grid = |xs: &[f64], ys: &[f64]| -> Vec<(f64, f64)> {
        xs.iter()
            .flat_map(|&x| ys.iter().map(move |&y| (x, y)))
            .collect()
    };
    expect(&mut results, "red square", Colour::Red, &grid(&near, &near));
    let blue_columns = near.map(|x| cw - SQUARE + x);
    expect(
        &mut results,
        "blue square at the top right",
        Colour::Blue,
        &grid(&blue_columns, &near),
    );
    let bar_rows = [ch - BAR + 0.2 * BAR, ch - BAR / 2.0, ch - 0.2 * BAR];
    let bar_columns = [0.03 * cw, 0.25 * cw, 0.5 * cw, 0.75 * cw, 0.97 * cw];
    expect(
        &mut results,
        "green bar along the bottom",
        Colour::Green,
        &grid(&bar_columns, &bar_rows),
    );

    let gap = 24.0;
    let lower_middle = (HEADING_BOTTOM + ch - BAR) / 2.0;
    expect(
        &mut results,
        "white everywhere else",
        Colour::White,
        &[
            (cw / 2.0, SQUARE / 2.0),                          // between the squares
            (SQUARE + gap, SQUARE / 2.0),                      // just right of the red square
            (cw - SQUARE - gap, SQUARE / 2.0),                 // just left of the blue square
            (SQUARE / 2.0, (SQUARE + HEADING_TOP) / 2.0),      // below the red square
            (cw - SQUARE / 2.0, (SQUARE + HEADING_TOP) / 2.0), // below the blue square
            (0.05 * cw, lower_middle),                         // left margin, below the heading
            (cw / 2.0, lower_middle),
            (0.95 * cw, lower_middle),
            (cw / 2.0, ch - BAR - gap), // just above the green bar
        ],
    );
    results
}

fn read_limited(path: &Path) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut data = Vec::new();
    file.take(png::MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|error| error.to_string())?;
    if data.len() as u64 > png::MAX_FILE_BYTES {
        return Err(format!("file is larger than {} bytes", png::MAX_FILE_BYTES));
    }
    Ok(data)
}

/// The whole command: `check-colors-screenshot <screenshot.png>`. Returns the exit status
/// and what to print on standard output; usage errors go to standard error.
///
/// Exit status: 0 when every check passes, 1 when the picture is wrong, 2 when the file is
/// not a usable PNG (truncated, corrupt, malformed, larger than a screenshot can be, or in a
/// form this does not read) or the command line is wrong.
pub fn run(program: &str, arguments: &[String]) -> (i32, String, String) {
    if arguments.len() != 1 || arguments[0] == "-h" || arguments[0] == "--help" {
        return (
            2,
            String::new(),
            format!("usage: {program} <screenshot.png>\n"),
        );
    }
    let path = &arguments[0];
    let image = match read_limited(Path::new(path))
        .and_then(|data| png::decode(&data).map_err(|error| error.0))
    {
        Ok(image) => image,
        Err(message) => {
            return (
                2,
                format!("FAIL {path}: not a usable PNG: {message}\n"),
                String::new(),
            );
        }
    };
    let mut out = format!("image {path}: {}x{}\n", image.width, image.height);
    let results = check(&image);
    for result in &results {
        out.push_str(&format!(
            "{} {}: {}\n",
            if result.ok { "PASS" } else { "FAIL" },
            result.name,
            result.detail
        ));
    }
    if results.iter().any(|result| !result.ok) {
        out.push_str("screenshot does not show tests/pages/colors.html as expected\n");
        return (1, out, String::new());
    }
    out.push_str("screenshot matches tests/pages/colors.html\n");
    (0, out, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [u8; 3] = [255, 255, 255];
    const RED: [u8; 3] = [255, 0, 0];
    const GREEN: [u8; 3] = [0, 128 + 100, 0];
    const BLUE: [u8; 3] = [0, 0, 255];
    const BLACK: [u8; 3] = [0, 0, 0];
    const CHROME: [u8; 3] = [40, 40, 48];

    /// A window like the browser's: `chrome` rows of toolbar, then colors.html at `scale`.
    struct Window {
        width: usize,
        height: usize,
        pixels: Vec<[u8; 3]>,
    }

    impl Window {
        fn new(css_width: f64, css_height: f64, scale: f64, chrome: usize) -> Window {
            let width = (css_width * scale) as usize;
            let height = chrome + (css_height * scale) as usize;
            let mut window = Window {
                width,
                height,
                pixels: vec![WHITE; width * height],
            };
            window.fill(0, 0, width, chrome, CHROME);
            let s = |css: f64| (css * scale) as usize;
            let square = s(SQUARE);
            window.fill(0, chrome, square, square, RED);
            window.fill(width - square, chrome, square, square, BLUE);
            window.fill(0, height - s(BAR), width, s(BAR), GREEN);
            window.fill(s(8.0), chrome + s(HEADING_TOP), s(300.0), s(40.0), BLACK);
            window
        }

        fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, rgb: [u8; 3]) {
            for yy in y..(y + h).min(self.height) {
                for xx in x..(x + w).min(self.width) {
                    self.pixels[yy * self.width + xx] = rgb;
                }
            }
        }

        fn png(&self, channels: usize) -> Vec<u8> {
            let data: Vec<u8> = self
                .pixels
                .iter()
                .flat_map(|&[r, g, b]| match channels {
                    4 => vec![r, g, b, 255],
                    _ => vec![r, g, b],
                })
                .collect();
            png::encode(self.width as u32, self.height as u32, channels, &data)
        }

        fn image(&self) -> Image {
            png::decode(&self.png(3)).expect("decodes")
        }
    }

    fn all_pass(results: &[Check]) -> bool {
        results.len() == 7 && results.iter().all(|result| result.ok)
    }

    #[test]
    fn classifies_like_the_script() {
        assert_eq!(classify(WHITE), Colour::White);
        assert_eq!(classify([235, 235, 235]), Colour::White);
        assert_eq!(classify([234, 235, 235]), Colour::Other);
        assert_eq!(classify([200, 60, 60]), Colour::Red);
        assert_eq!(classify(GREEN), Colour::Green);
        assert_eq!(classify(BLUE), Colour::Blue);
        assert_eq!(classify(BLACK), Colour::Other);
    }

    #[test]
    fn passes_at_every_scale() {
        for (scale, chrome) in [(1.0, 80), (2.0, 160), (1.5, 117), (3.0, 0)] {
            let window = Window::new(900.0, 620.0, scale, chrome);
            let results = check(&window.image());
            assert!(all_pass(&results), "scale {scale}: {results:#?}");
            assert_eq!(
                results[2].detail,
                format!("{scale:.2} image pixels per CSS pixel; page 900x620 CSS pixels")
            );
        }
    }

    #[test]
    fn reads_rgba_screenshots() {
        let window = Window::new(900.0, 620.0, 2.0, 100);
        let image = png::decode(&window.png(4)).expect("decodes");
        assert!(all_pass(&check(&image)));
    }

    #[test]
    fn reports_a_missing_square() {
        let mut window = Window::new(900.0, 620.0, 1.0, 80);
        window.fill(0, 80, 160, 160, WHITE);
        let results = check(&window.image());
        assert_eq!(results.len(), 1);
        assert!(!results[0].ok);
        assert_eq!(results[0].detail, "first red pixel in column 9 at y=None");
    }

    #[test]
    fn reports_a_missing_blue_square_and_bar() {
        let mut window = Window::new(900.0, 620.0, 1.0, 80);
        window.fill(740, 80, 160, 160, WHITE);
        window.fill(0, 620, 900, 80, CHROME);
        let results = check(&window.image());
        let failed: Vec<&str> = results.iter().filter(|r| !r.ok).map(|r| r.name).collect();
        assert_eq!(
            failed,
            ["blue square at the top right", "green bar along the bottom"]
        );
        assert!(
            results[4]
                .detail
                .starts_with("not blue at (764,104)=rgb(255, 255, 255)")
        );
    }

    #[test]
    fn refuses_a_page_too_small_or_not_square() {
        let small = Window::new(390.0, 620.0, 1.0, 40);
        let results = check(&small.image());
        assert_eq!(
            results.last().map(|r| (r.name, r.ok)),
            Some(("page is large enough to check", false))
        );

        let mut stretched = Window::new(900.0, 620.0, 1.0, 80);
        stretched.fill(0, 240, 160, 60, RED);
        let results = check(&stretched.image());
        assert_eq!(
            results.last().map(|r| (r.name, r.ok, r.detail.as_str())),
            Some((
                "red square is square and starts at the left edge",
                false,
                "160x220 image pixels, left edge at x=0"
            ))
        );
    }

    #[test]
    fn command_reports_and_exits() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let good = dir.path().join("good.png");
        std::fs::write(&good, Window::new(900.0, 620.0, 2.0, 150).png(3)).expect("write");
        let good_text = good.display().to_string();
        let (status, out, err) = run("check", std::slice::from_ref(&good_text));
        assert_eq!((status, err.as_str()), (0, ""));
        assert!(out.starts_with(&format!("image {good_text}: 1800x1390\nPASS red square found at the top left: first red pixel in column 18 at y=150\n")));
        assert!(out.ends_with("screenshot matches tests/pages/colors.html\n"));

        let wrong = dir.path().join("wrong.png");
        let mut window = Window::new(900.0, 620.0, 1.0, 80);
        window.fill(400, 120, 100, 80, BLACK);
        std::fs::write(&wrong, window.png(3)).expect("write");
        let (status, out, _) = run("check", &[wrong.display().to_string()]);
        assert_eq!(status, 1);
        assert!(out.contains("FAIL white everywhere else: not white at (450,160)=rgb(0, 0, 0)"));
        assert!(out.ends_with("screenshot does not show tests/pages/colors.html as expected\n"));

        let broken = dir.path().join("broken.png");
        std::fs::write(&broken, b"\x89PNG\r\n\x1a\n").expect("write");
        let broken_text = broken.display().to_string();
        assert_eq!(
            run("check", std::slice::from_ref(&broken_text)),
            (
                2,
                format!("FAIL {broken_text}: not a usable PNG: unexpected end of file\n"),
                String::new()
            )
        );
        let missing = dir.path().join("missing.png").display().to_string();
        let (status, out, _) = run("check", std::slice::from_ref(&missing));
        assert_eq!(status, 2);
        assert!(out.starts_with(&format!("FAIL {missing}: not a usable PNG: ")));

        let usage = (
            2,
            String::new(),
            "usage: check <screenshot.png>\n".to_owned(),
        );
        assert_eq!(run("check", &[]), usage);
        assert_eq!(run("check", &["--help".to_owned()]), usage);
        assert_eq!(run("check", &["a".to_owned(), "b".to_owned()]), usage);
    }
}
