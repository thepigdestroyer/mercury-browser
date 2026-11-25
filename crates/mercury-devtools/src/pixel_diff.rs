// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Compares two folders of PNG pictures, file by file: the proof that a change to the interface
//! draws what it drew before (the theme-plumbing lane's gate, `scripts/dev/render-scenes.sh`
//! renders the folders).
//!
//! ```text
//! cargo run --release -p mercury-devtools --bin pixel-diff -- <before> <after> \
//!     [--tolerance N] [--write-diffs <dir>]
//! ```
//!
//! Every `*.png` is matched by its path relative to the folder (subfolders included). A pixel
//! differs when any of its red, green or blue values differs by more than the tolerance (0 by
//! default: bit for bit); alpha is not compared (the software renderer's pictures are opaque).
//! For each file the report says whether it is identical, how many pixels differ, the largest
//! difference of a channel and the box around the differing pixels, or that it is only in one
//! folder or has changed size. `<before>` and `<after>` may also both be single PNG files.
//!
//! Exit status: 0 when every file is identical, 1 when anything differs or is missing on one
//! side, 2 for a wrong command line, a folder with no PNG files, or a file that is not a usable
//! PNG (the message names it).

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::png::{self, Image};

/// Why a comparison could not be made: a folder that is missing, a file that cannot be read, or
/// one that is not a usable PNG. The message names the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// What the comparison found for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every pixel is within the tolerance.
    Identical,
    /// The pictures have the same size and some pixels differ.
    Differs {
        /// How many pixels differ by more than the tolerance.
        pixels: u64,
        /// The largest difference of any channel among those pixels.
        max_delta: u8,
        /// The smallest box that holds them, as `(left, top, right, bottom)`, both ends included.
        bounds: (usize, usize, usize, usize),
    },
    /// The pictures are not the same size, as `(width, height)`.
    SizeChanged {
        /// The size before.
        before: (usize, usize),
        /// The size after.
        after: (usize, usize),
    },
    /// There is no such file after.
    OnlyBefore,
    /// There is no such file before.
    OnlyAfter,
}

/// One file and what was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The path relative to the compared folder.
    pub path: PathBuf,
    /// What was found.
    pub outcome: Outcome,
}

/// The result of comparing two folders: one entry per file, sorted by path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The tolerance used.
    pub tolerance: u8,
    /// Every file on either side.
    pub entries: Vec<Entry>,
}

impl Report {
    /// Whether every file is identical (so the exit status is 0).
    pub fn is_identical(&self) -> bool {
        self.entries
            .iter()
            .all(|entry| entry.outcome == Outcome::Identical)
    }

    /// How many files have an outcome that is not [`Outcome::Identical`].
    pub fn problems(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.outcome != Outcome::Identical)
            .count()
    }

    /// The entry for a path, as it is spelled relative to the folder.
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.path == Path::new(path))
    }
}

impl fmt::Display for Report {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut identical = 0;
        for entry in &self.entries {
            let path = entry.path.display();
            match &entry.outcome {
                Outcome::Identical => identical += 1,
                Outcome::Differs {
                    pixels,
                    max_delta,
                    bounds: (left, top, right, bottom),
                } => writeln!(
                    formatter,
                    "DIFFERENT  {path}: {pixels} pixels differ, largest channel difference \
                     {max_delta}, within x {left}..={right}, y {top}..={bottom}"
                )?,
                Outcome::SizeChanged { before, after } => writeln!(
                    formatter,
                    "SIZE       {path}: {}x{} before, {}x{} after",
                    before.0, before.1, after.0, after.1
                )?,
                Outcome::OnlyBefore => writeln!(formatter, "MISSING    {path}: only before")?,
                Outcome::OnlyAfter => writeln!(formatter, "MISSING    {path}: only after")?,
            }
        }
        write!(
            formatter,
            "{} files compared (tolerance {}): {identical} identical, {} not",
            self.entries.len(),
            self.tolerance,
            self.problems()
        )
    }
}

/// Pixels to leave out of a comparison: wherever the same-named pictures of two other folders
/// differ (`with` and `without` a thing, for example a drawn mark), grown by `grow` pixels on
/// every side. The pictures of `with` and `without` are rendered from the *after* code, so the
/// mask is exactly what that code draws, not a rectangle someone typed in.
#[derive(Debug, Clone, Copy)]
pub struct Mask<'a> {
    /// The folder with the thing drawn.
    pub with: &'a Path,
    /// The folder without it.
    pub without: &'a Path,
    /// How many pixels around each differing pixel are also ignored (edge antialiasing).
    pub grow: usize,
}

/// Compares every `*.png` of `before` with the one at the same relative path in `after`.
pub fn compare_dirs(before: &Path, after: &Path, tolerance: u8) -> Result<Report, Error> {
    compare(before, after, tolerance, None)
}

/// Like [`compare_dirs`]; also writes a picture for every file that differs (and has the same
/// size) into `diffs`, under the same relative path: the old picture dimmed, with the differing
/// pixels in red.
pub fn compare(
    before: &Path,
    after: &Path,
    tolerance: u8,
    diffs: Option<&Path>,
) -> Result<Report, Error> {
    compare_masked(before, after, tolerance, diffs, None)
}

/// Like [`compare`], leaving out the pixels of `mask` (see [`Mask`]). A file that has no picture
/// of the same name and size in both mask folders is compared whole.
pub fn compare_masked(
    before: &Path,
    after: &Path,
    tolerance: u8,
    diffs: Option<&Path>,
    mask: Option<Mask<'_>>,
) -> Result<Report, Error> {
    let (before_files, after_files) = if before.is_file() && after.is_file() {
        let name = PathBuf::from(after.file_name().unwrap_or_default());
        (
            BTreeSet::from([name.clone()]),
            BTreeSet::from([name.clone()]),
        )
    } else {
        for folder in [before, after] {
            if !folder.is_dir() {
                return Err(Error(format!("{}: not a folder", folder.display())));
            }
        }
        let lists = (list(before)?, list(after)?);
        // A folder with no pictures (a mistyped path that exists, a render that wrote nothing)
        // would compare as "nothing differs": that must not pass the gate.
        for (folder, files) in [(before, &lists.0), (after, &lists.1)] {
            if files.is_empty() {
                return Err(Error(format!("{}: no PNG files", folder.display())));
            }
        }
        lists
    };
    let single = before.is_file();
    let locate = |root: &Path, relative: &Path| {
        if single {
            root.to_path_buf()
        } else {
            root.join(relative)
        }
    };

    let mut entries = Vec::new();
    for relative in before_files.union(&after_files) {
        let outcome = match (
            before_files.contains(relative),
            after_files.contains(relative),
        ) {
            (true, false) => Outcome::OnlyBefore,
            (false, true) => Outcome::OnlyAfter,
            _ => {
                let old = locate(before, relative);
                let new = locate(after, relative);
                let (old_image, new_image) = (read(&old)?, read(&new)?);
                let ignore = match mask {
                    Some(mask) => ignore_plane(&mask, relative, &old_image)?,
                    None => None,
                };
                let outcome = compare_images(&old_image, &new_image, tolerance, ignore.as_deref());
                if let (Some(directory), Outcome::Differs { .. }) = (diffs, &outcome) {
                    write_diff(&directory.join(relative), &old_image, &new_image, tolerance)?;
                }
                outcome
            }
        };
        entries.push(Entry {
            path: relative.clone(),
            outcome,
        });
    }
    Ok(Report { tolerance, entries })
}

/// The relative paths of every `*.png` under `root`, subfolders included.
fn list(root: &Path) -> Result<BTreeSet<PathBuf>, Error> {
    let mut found = BTreeSet::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let read = fs::read_dir(&folder)
            .map_err(|error| Error(format!("{}: {error}", folder.display())))?;
        for item in read {
            let item = item.map_err(|error| Error(format!("{}: {error}", folder.display())))?;
            let path = item.path();
            // `metadata` follows symbolic links, so a linked picture or folder counts.
            let kind = fs::metadata(&path)
                .map_err(|error| Error(format!("{}: {error}", path.display())))?;
            if kind.is_dir() {
                folders.push(path);
            } else if kind.is_file()
                && is_png_name(&path)
                && let Ok(relative) = path.strip_prefix(root)
            {
                found.insert(relative.to_path_buf());
            }
        }
    }
    Ok(found)
}

fn is_png_name(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
}

fn read(path: &Path) -> Result<Image, Error> {
    let data = fs::read(path).map_err(|error| Error(format!("{}: {error}", path.display())))?;
    png::decode(&data).map_err(|error| Error(format!("{}: {error}", path.display())))
}

/// Which pixels of a picture the mask leaves out, row by row, or `None` when the mask has no
/// usable pair of pictures for it.
fn ignore_plane(
    mask: &Mask<'_>,
    relative: &Path,
    like: &Image,
) -> Result<Option<Vec<bool>>, Error> {
    let (with, without) = (mask.with.join(relative), mask.without.join(relative));
    if !with.is_file() || !without.is_file() {
        return Ok(None);
    }
    let (with, without) = (read(&with)?, read(&without)?);
    let same_size = |other: &Image| (other.width, other.height) == (like.width, like.height);
    if !same_size(&with) || !same_size(&without) {
        return Ok(None);
    }
    let (width, height) = (like.width, like.height);
    let mut plane = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            if largest_delta(with.pixel(x, y), without.pixel(x, y)) == 0 {
                continue;
            }
            for yy in y.saturating_sub(mask.grow)..(y + mask.grow + 1).min(height) {
                for xx in x.saturating_sub(mask.grow)..(x + mask.grow + 1).min(width) {
                    plane[yy * width + xx] = true;
                }
            }
        }
    }
    Ok(Some(plane))
}

fn compare_images(
    before: &Image,
    after: &Image,
    tolerance: u8,
    ignore: Option<&[bool]>,
) -> Outcome {
    if (before.width, before.height) != (after.width, after.height) {
        return Outcome::SizeChanged {
            before: (before.width, before.height),
            after: (after.width, after.height),
        };
    }
    let mut pixels = 0u64;
    let mut max_delta = 0u8;
    let mut bounds = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..before.height {
        for x in 0..before.width {
            if ignore.is_some_and(|plane| plane[y * before.width + x]) {
                continue;
            }
            let delta = largest_delta(before.pixel(x, y), after.pixel(x, y));
            if delta > tolerance {
                pixels += 1;
                max_delta = max_delta.max(delta);
                bounds = (
                    bounds.0.min(x),
                    bounds.1.min(y),
                    bounds.2.max(x),
                    bounds.3.max(y),
                );
            }
        }
    }
    if pixels == 0 {
        Outcome::Identical
    } else {
        Outcome::Differs {
            pixels,
            max_delta,
            bounds,
        }
    }
}

fn largest_delta(before: [u8; 3], after: [u8; 3]) -> u8 {
    before
        .into_iter()
        .zip(after)
        .map(|(old, new)| old.abs_diff(new))
        .max()
        .unwrap_or(0)
}

fn write_diff(path: &Path, before: &Image, after: &Image, tolerance: u8) -> Result<(), Error> {
    let fail = |error: &dyn fmt::Display| Error(format!("{}: {error}", path.display()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| fail(&error))?;
    }
    let mut picture = image::RgbImage::new(before.width as u32, before.height as u32);
    for (x, y, pixel) in picture.enumerate_pixels_mut() {
        let (x, y) = (x as usize, y as usize);
        let old = before.pixel(x, y);
        *pixel = if largest_delta(old, after.pixel(x, y)) > tolerance {
            image::Rgb([255, 0, 0])
        } else {
            image::Rgb(old.map(|value| value / 4 + 150))
        };
    }
    picture.save(path).map_err(|error| fail(&error))
}

const USAGE: &str = "usage: pixel-diff <before> <after> [--tolerance N] [--write-diffs <dir>]\n  \
     [--ignore-where <with> <without> [--grow N]]\n  \
     <before> and <after> are two folders of PNG files, or two PNG files;\n  \
     --ignore-where leaves out the pixels (grown by N, default 2) where the same-named pictures of\n  \
     the folders <with> and <without> differ\n";

/// Runs the command line `arguments` (without the program name) and returns the exit status and
/// the text for standard output and standard error.
pub fn run(arguments: &[String]) -> (i32, String, String) {
    let mut paths = Vec::new();
    let mut tolerance = 0u8;
    let mut diffs = None;
    let mut ignore_where: Option<(PathBuf, PathBuf)> = None;
    let mut grow = 2usize;
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--tolerance" => match rest.next().map(|value| value.parse::<u8>()) {
                Some(Ok(value)) => tolerance = value,
                _ => return usage("--tolerance takes a number from 0 to 255"),
            },
            "--write-diffs" => match rest.next() {
                Some(directory) => diffs = Some(PathBuf::from(directory)),
                None => return usage("--write-diffs takes a folder"),
            },
            "--ignore-where" => match (rest.next(), rest.next()) {
                (Some(with), Some(without)) => {
                    ignore_where = Some((PathBuf::from(with), PathBuf::from(without)));
                }
                _ => return usage("--ignore-where takes two folders"),
            },
            "--grow" => match rest.next().map(|value| value.parse::<usize>()) {
                Some(Ok(value)) => grow = value,
                _ => return usage("--grow takes a number"),
            },
            "-h" | "--help" => return (0, USAGE.to_owned(), String::new()),
            flag if flag.starts_with("--") => return usage(&format!("unknown option {flag}")),
            path => paths.push(PathBuf::from(path)),
        }
    }
    let [before, after] = paths.as_slice() else {
        return usage("give exactly two paths");
    };
    let mask = ignore_where.as_ref().map(|(with, without)| Mask {
        with,
        without,
        grow,
    });
    match compare_masked(before, after, tolerance, diffs.as_deref(), mask) {
        Ok(report) => {
            let status = i32::from(!report.is_identical());
            (status, format!("{report}\n"), String::new())
        }
        Err(error) => (2, String::new(), format!("pixel-diff: {error}\n")),
    }
}

fn usage(problem: &str) -> (i32, String, String) {
    (2, String::new(), format!("pixel-diff: {problem}\n{USAGE}"))
}
