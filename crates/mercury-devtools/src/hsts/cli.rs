// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The command line of `hsts-preload`: `build`, `check` and `diff`.
//!
//! The binary (`src/bin/hsts-preload.rs`) is a thin `main` around [`run`], which takes its input and
//! output as arguments so that the tests can drive it without a process. Options are parsed by
//! hand, as `light-crates` does: any order, `--option value`, no abbreviations. A wrong command
//! line (an unknown option, a missing or malformed value, no command) is exit status 2 with the
//! usage on `err`; a data problem (a refused build, a failed check, a file that cannot be read or
//! is not a list) is exit status 1 with a sentence on `err` that names the file; success is 0.
//!
//! `build` never writes anything unless the new list passed every guard
//! ([`review::guard_refresh`]): it reads and checks first, then replaces the data file, then writes
//! the manifest. `check` and `diff` only read. Their questions are answered with "the host, or a
//! parent that includes subdomains" ([`Preload::is_secure`]), not with Servo's own walk from the
//! public suffix; the two agree for every entry of the list except the ones that are public
//! suffixes themselves (`dev`), and neither command looks hosts up.
//!
//! Nothing here touches the network; `scripts/dev/update-hsts-preload.sh` downloads.

use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use super::fstmap::{self, Preload};
use super::manifest::{Date, Manifest, Source, sha256_hex};
use super::review::{self, Limits};
use super::{
    CHROMIUM_FILE, CHROMIUM_HOST_URL, DEFAULT_MAX_FLAG_LOSS_PERCENT, DEFAULT_MAX_SHRINK_PERCENT,
    DEFAULT_MIN_ENTRIES, MAX_INPUT_BYTES, source,
};

/// How many lines of each kind `diff` shows unless `--limit` or `--all` says otherwise.
const DEFAULT_DIFF_LIMIT: usize = 20;
/// The Chromium revision `build` records when `--ref` is not given.
const DEFAULT_REF: &str = "main";
/// What the manifest calls the input.
const SOURCE_NAME: &str = "Chromium net/http/transport_security_state_static.json";

const USAGE: &str = "\
usage:
  hsts-preload build --input <file|-> [--base64] --output <file.fstmap>
                     [--manifest <file.json>] [--source-url URL] [--ref REF]
                     [--retrieved YYYY-MM-DD] [--min-entries N]
                     [--max-shrink-percent N] [--allow-shrink]
                     [--max-flag-loss-percent N] [--allow-flag-loss]
  hsts-preload check <file.fstmap> [--manifest <file.json>] [--min-entries N]
                     [--max-age-days N] [--today YYYY-MM-DD]
  hsts-preload diff <old.fstmap> <new.fstmap> [--limit N | --all]
  hsts-preload -h | --help

build   reads Chromium's transport_security_state_static.json (--base64 when it is
        the Gitiles download, which is base64; - reads standard input) and writes the
        data file and, next to it, its manifest (<name>.manifest.json unless
        --manifest says otherwise). It writes nothing when the input is over
        100 MB or the new list fails a guard: fewer than --min-entries entries
        (default 50000), under 90 % of entries including subdomains, a long-standing
        entry missing, more than --max-shrink-percent (default 10) fewer entries than
        the file it would replace (--allow-shrink waives that one only), or more than
        --max-flag-loss-percent (default 1) of the hosts it shares with that file
        losing include-subdomains (--allow-flag-loss waives that one only).
check   says whether a data file is fit to ship (exit 0) or not (exit 1), and prints what
        it found. A manifest next to the file is read and compared; --max-age-days makes
        an older list a problem.
diff    lists what differs between two data files: hosts added, removed, and whose
        include-subdomains flag changed (default 20 lines of each kind). Exit 0.

exit status: 0 done, 1 a data problem, 2 a wrong command line
";

/// Why a command stopped before it could report success.
enum Failure {
    /// `-h` or `--help`: the usage on standard output, exit 0.
    Help,
    /// A wrong command line: the sentence and the usage on `err`, exit 2.
    Usage(String),
    /// A problem with the data or the files: the sentence on `err`, exit 1.
    Problem(String),
}

fn problem(text: impl Into<String>) -> Failure {
    Failure::Problem(text.into())
}

fn wrong(text: impl Into<String>) -> Failure {
    Failure::Usage(text.into())
}

/// Writes a line to the caller's output; a closed pipe is not worth failing for.
macro_rules! say {
    ($out:expr, $($argument:tt)*) => {{
        let _ = writeln!($out, $($argument)*);
    }};
}

/// Runs one command line (`arguments` without the program name) and returns the exit status:
/// 0 done, 1 a data problem, 2 a wrong command line. `input` is what `--input -` reads, `out`
/// receives the reports, `err` the problems and the usage, and `today` (UTC) is the retrieval date
/// `build` records unless `--retrieved` says otherwise and the day `check` measures age from
/// unless `--today` says otherwise.
pub fn run(
    arguments: &[String],
    input: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
    today: Date,
) -> u8 {
    match dispatch(arguments, input, out, today) {
        Ok(status) => status,
        Err(Failure::Help) => {
            let _ = write!(out, "{USAGE}");
            0
        }
        Err(Failure::Usage(text)) => {
            let _ = write!(err, "hsts-preload: {text}\n\n{USAGE}");
            2
        }
        Err(Failure::Problem(text)) => {
            let _ = writeln!(err, "hsts-preload: {text}");
            1
        }
    }
}

fn dispatch(
    arguments: &[String],
    input: &mut dyn Read,
    out: &mut dyn Write,
    today: Date,
) -> Result<u8, Failure> {
    let Some((command, rest)) = arguments.split_first() else {
        return Err(wrong("no command given"));
    };
    match command.as_str() {
        "-h" | "--help" => Err(Failure::Help),
        "build" => build(&parse(&BUILD, rest)?, input, out, today),
        "check" => check(&parse(&CHECK, rest)?, out, today),
        "diff" => diff(&parse(&DIFF, rest)?, out),
        other => Err(wrong(format!(
            "unknown command {other:?} (expected build, check or diff)"
        ))),
    }
}

/// What a command accepts.
struct Grammar {
    command: &'static str,
    /// Options without a value.
    flags: &'static [&'static str],
    /// Options followed by a value.
    options: &'static [&'static str],
    /// What the command takes besides options, one entry per positional argument.
    positionals: &'static [&'static str],
}

const BUILD: Grammar = Grammar {
    command: "build",
    flags: &["--base64", "--allow-shrink", "--allow-flag-loss"],
    options: &[
        "--input",
        "--output",
        "--manifest",
        "--source-url",
        "--ref",
        "--retrieved",
        "--min-entries",
        "--max-shrink-percent",
        "--max-flag-loss-percent",
    ],
    positionals: &[],
};

const CHECK: Grammar = Grammar {
    command: "check",
    flags: &[],
    options: &["--manifest", "--min-entries", "--max-age-days", "--today"],
    positionals: &["<file.fstmap>"],
};

const DIFF: Grammar = Grammar {
    command: "diff",
    flags: &["--all"],
    options: &["--limit"],
    positionals: &["<old.fstmap>", "<new.fstmap>"],
};

/// A parsed command line.
struct Args {
    command: &'static str,
    flags: Vec<&'static str>,
    options: Vec<(&'static str, String)>,
    positionals: Vec<String>,
}

fn parse(grammar: &Grammar, arguments: &[String]) -> Result<Args, Failure> {
    let mut args = Args {
        command: grammar.command,
        flags: Vec::new(),
        options: Vec::new(),
        positionals: Vec::new(),
    };
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        let argument = argument.as_str();
        if argument == "-h" || argument == "--help" {
            return Err(Failure::Help);
        }
        if let Some(&flag) = grammar.flags.iter().find(|flag| **flag == argument) {
            if args.flags.contains(&flag) {
                return Err(wrong(format!("{flag} is given twice")));
            }
            args.flags.push(flag);
        } else if let Some(&option) = grammar.options.iter().find(|name| **name == argument) {
            // The value is the next argument, whatever it is, unless that is another option
            // (`--input --base64`): a value that starts with `--` is a missing value.
            let value = match rest.next() {
                Some(value) if !value.starts_with("--") => value.clone(),
                _ => return Err(wrong(format!("{option} needs a value"))),
            };
            if args.options.iter().any(|(name, _)| *name == option) {
                return Err(wrong(format!("{option} is given twice")));
            }
            args.options.push((option, value));
        } else if argument.starts_with('-') && argument != "-" {
            return Err(wrong(format!(
                "unknown option {argument} for {}",
                grammar.command
            )));
        } else {
            args.positionals.push(argument.to_string());
        }
    }
    if args.positionals.len() != grammar.positionals.len() {
        let wanted = if grammar.positionals.is_empty() {
            "no file names".to_string()
        } else {
            grammar.positionals.join(" ")
        };
        return Err(wrong(format!(
            "{} takes {wanted} (besides options), found {} argument(s)",
            grammar.command,
            args.positionals.len()
        )));
    }
    Ok(args)
}

impl Args {
    fn flag(&self, name: &str) -> bool {
        self.flags.contains(&name)
    }

    fn option(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(option, _)| *option == name)
            .map(|(_, value)| value.as_str())
    }

    /// An option that must be given.
    fn required(&self, name: &str) -> Result<&str, Failure> {
        self.option(name)
            .ok_or_else(|| wrong(format!("{} needs {name}", self.command)))
    }

    /// An option whose value is a whole number.
    fn number(&self, name: &str) -> Result<Option<usize>, Failure> {
        self.option(name)
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| wrong(format!("{name} needs a whole number, not {value:?}")))
            })
            .transpose()
    }

    /// An option whose value is a day, `YYYY-MM-DD`.
    fn date(&self, name: &str) -> Result<Option<Date>, Failure> {
        self.option(name)
            .map(|value| {
                Date::parse(value).map_err(|error| wrong(format!("{name} {value:?}: {error}")))
            })
            .transpose()
    }
}

/// Reads all of `reader`, but no more than [`MAX_INPUT_BYTES`]: an input over the cap is a
/// problem that names it, and nothing past the cap is ever held in memory.
fn read_capped(reader: &mut dyn Read, name: &str) -> Result<Vec<u8>, Failure> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| problem(format!("cannot read {name}: {error}")))?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(problem(format!(
            "{name} is larger than {MAX_INPUT_BYTES} bytes, far more than Chromium's list (about \
             14 MB as base64); it is not the file this tool expects, so it was not read further"
        )));
    }
    Ok(bytes)
}

fn read_file(path: &str) -> Result<Vec<u8>, Failure> {
    std::fs::read(path).map_err(|error| problem(format!("cannot read {path}: {error}")))
}

/// Reads and opens a data file; the error names the file.
fn open_file(path: &str) -> Result<Preload, Failure> {
    Preload::open(read_file(path)?).map_err(|error| problem(format!("{path}: {error}")))
}

/// The manifest that goes with a data file: its path with `.fstmap` replaced by `.manifest.json`
/// (the whole name plus `.manifest.json` when it does not end in `.fstmap`).
fn default_manifest_path(data: &Path) -> PathBuf {
    let text = data.to_string_lossy();
    PathBuf::from(format!(
        "{}.manifest.json",
        text.strip_suffix(".fstmap").unwrap_or(&text)
    ))
}

/// `part` of `whole` as a percentage with one decimal, rounded to nearest.
fn percent(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "0.0 %".to_string();
    }
    let tenths = (part as u64 * 1_000 + whole as u64 / 2) / whole as u64;
    format!("{}.{} %", tenths / 10, tenths % 10)
}

/// The data file `output` names, when it exists and opens: the file the build is about to replace.
/// A file that cannot be opened (damaged, or an empty placeholder) is replaced without a
/// comparison and the second value says so; one that cannot be read at all is a problem.
fn replaced_file(output: &Path) -> Result<(Option<Preload>, Option<String>), Failure> {
    let bytes = match std::fs::read(output) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok((None, None)),
        Err(error) => {
            return Err(problem(format!(
                "cannot read the file {} would replace: {error}",
                output.display()
            )));
        }
    };
    match Preload::open(bytes) {
        Ok(old) => Ok((Some(old), None)),
        Err(error) => Ok((
            None,
            Some(format!(
                "the existing {} cannot be used as a list ({error}); it is replaced without \
                 a comparison",
                output.display()
            )),
        )),
    }
}

fn ensure_directory(file: &Path) -> Result<(), Failure> {
    let directory = match file.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if directory.is_dir() {
        Ok(())
    } else {
        Err(problem(format!(
            "cannot write {}: the directory {} does not exist",
            file.display(),
            directory.display()
        )))
    }
}

fn build(
    args: &Args,
    input: &mut dyn Read,
    out: &mut dyn Write,
    today: Date,
) -> Result<u8, Failure> {
    // Every option is checked before anything is read or written.
    let input_name = args.required("--input")?;
    let output = args.required("--output")?;
    let git_ref = args.option("--ref").unwrap_or(DEFAULT_REF);
    let retrieved = args.date("--retrieved")?.unwrap_or(today);
    let limits = Limits {
        min_entries: args.number("--min-entries")?.unwrap_or(DEFAULT_MIN_ENTRIES),
        ..Limits::default()
    };
    let max_shrink = args
        .number("--max-shrink-percent")?
        .unwrap_or(DEFAULT_MAX_SHRINK_PERCENT);
    let max_flag_loss = args
        .number("--max-flag-loss-percent")?
        .unwrap_or(DEFAULT_MAX_FLAG_LOSS_PERCENT);
    let output_path = Path::new(output);
    let manifest_path = args
        .option("--manifest")
        .map_or_else(|| default_manifest_path(output_path), PathBuf::from);
    if manifest_path == output_path {
        return Err(wrong("--manifest and --output name the same file"));
    }
    let url = args.option("--source-url").map_or_else(
        || format!("{CHROMIUM_HOST_URL}/{git_ref}/{CHROMIUM_FILE}?format=TEXT"),
        str::to_string,
    );

    // Read, decode, parse.
    let label = if input_name == "-" {
        "standard input"
    } else {
        input_name
    };
    let raw = if input_name == "-" {
        read_capped(input, "standard input")?
    } else {
        let mut file = std::fs::File::open(input_name)
            .map_err(|error| problem(format!("cannot read {input_name}: {error}")))?;
        read_capped(&mut file, input_name)?
    };
    let input_sha256 = sha256_hex(&raw);
    let json = if args.flag("--base64") {
        source::decode_base64(&raw).map_err(|error| problem(format!("{label}: {error}")))?
    } else {
        raw
    };
    let parsed =
        source::parse_chromium(&json).map_err(|error| problem(format!("{label}: {error}")))?;

    // Build, and prove that what was built reads back.
    let bytes = fstmap::build(&parsed.entries)
        .map_err(|error| problem(format!("cannot build the data file: {error}")))?;
    let fresh = Preload::open(bytes.clone()).map_err(|error| {
        problem(format!(
            "internal error: the data just built does not read back ({error}); nothing was written"
        ))
    })?;

    // The guards: nothing is written when the new list fails one.
    let (old, old_note) = replaced_file(output_path)?;
    review::guard_refresh(
        old.as_ref(),
        &fresh,
        &limits,
        max_shrink,
        args.flag("--allow-shrink"),
    )
    .map_err(|why| problem(format!("refusing to replace {output}: {why}")))?;
    if let Some(old) = &old
        && !args.flag("--allow-flag-loss")
    {
        review::guard_flag_loss(old, &fresh, max_flag_loss)
            .map_err(|why| problem(format!("refusing to replace {output}: {why}")))?;
    }
    ensure_directory(output_path)?;
    ensure_directory(&manifest_path)?;

    // Write the data, then the manifest.
    let manifest = Manifest::for_data(
        &bytes,
        &fresh,
        Source {
            name: SOURCE_NAME.to_string(),
            url: Some(url),
            git_ref: Some(git_ref.to_string()),
            retrieved: Some(retrieved),
            input_sha256: Some(input_sha256),
            objects: Some(parsed.report.objects),
            force_https: Some(parsed.report.force_https),
        },
    );
    fstmap::write_atomic(output_path, &bytes).map_err(problem)?;
    fstmap::write_atomic(&manifest_path, manifest.to_json().as_bytes()).map_err(|error| {
        problem(format!(
            "{error} (the data file {output} was already replaced; write its manifest again or \
             restore the data file from git)"
        ))
    })?;

    // The report.
    let report = &parsed.report;
    say!(
        out,
        "read {label}: {} objects, {} force-https, {} skipped (no force-https), {} duplicate \
         listings merged",
        report.objects,
        report.force_https,
        report.skipped,
        report.merged_duplicates
    );
    if !report.unknown_keys.is_empty() {
        let keys: Vec<String> = report
            .unknown_keys
            .iter()
            .map(|(key, count)| format!("{key} ({count})"))
            .collect();
        say!(out, "keys this tool does not know: {}", keys.join(", "));
    }
    if let Some(note) = old_note {
        say!(out, "note: {note}");
    }
    say!(
        out,
        "wrote {output}: {} entries, {} include subdomains ({})",
        fresh.len(),
        fresh.include_subdomains_count(),
        percent(fresh.include_subdomains_count(), fresh.len())
    );
    say!(out, "wrote {}", manifest_path.display());
    say!(out, "SHA-256 {}", manifest.output_sha256);
    match &old {
        Some(old) => {
            let rendered = review::render_diff(&review::diff(old, &fresh), Some(0));
            let summary = rendered.lines().next().unwrap_or_default();
            say!(out, "against the file it replaced: {summary}");
        }
        None => say!(out, "no earlier file to compare with"),
    }
    Ok(0)
}

fn check(args: &Args, out: &mut dyn Write, today: Date) -> Result<u8, Failure> {
    let path = args.positionals[0].as_str();
    let today = args.date("--today")?.unwrap_or(today);
    let max_age_days = args
        .option("--max-age-days")
        .map(|value| {
            value
                .parse::<i64>()
                .ok()
                .filter(|days| *days >= 0)
                .ok_or_else(|| {
                    wrong(format!(
                        "--max-age-days needs a whole number of days, not {value:?}"
                    ))
                })
        })
        .transpose()?;
    let limits = Limits {
        min_entries: args.number("--min-entries")?.unwrap_or(DEFAULT_MIN_ENTRIES),
        max_age_days,
        ..Limits::default()
    };
    let bytes = read_file(path)?;

    // The manifest: the one asked for, which must be there, or the one next to the file, which
    // need not be.
    let explicit = args.option("--manifest");
    let manifest_path =
        explicit.map_or_else(|| default_manifest_path(Path::new(path)), PathBuf::from);
    let mut notes = Vec::new();
    let mut problems = Vec::new();
    let manifest = match std::fs::read(&manifest_path) {
        Ok(text) => match Manifest::parse(&String::from_utf8_lossy(&text)) {
            Ok(manifest) => Some(manifest),
            Err(error) => {
                problems.push(format!(
                    "the manifest {} cannot be used: {error}",
                    manifest_path.display()
                ));
                None
            }
        },
        Err(error) if error.kind() == ErrorKind::NotFound && explicit.is_none() => {
            notes.push(format!(
                "no manifest next to the file ({} does not exist)",
                manifest_path.display()
            ));
            None
        }
        Err(error) => {
            problems.push(format!(
                "cannot read the manifest {}: {error}",
                manifest_path.display()
            ));
            None
        }
    };

    let verdict = review::check(&bytes, manifest.as_ref(), today, &limits);
    say!(out, "checked {path}");
    for note in notes.iter().chain(&verdict.notes) {
        say!(out, "  note: {note}");
    }
    for problem in problems.iter().chain(&verdict.problems) {
        say!(out, "  problem: {problem}");
    }
    if problems.is_empty() && verdict.ok() {
        say!(out, "ok");
        Ok(0)
    } else {
        say!(out, "not ok");
        Ok(1)
    }
}

fn diff(args: &Args, out: &mut dyn Write) -> Result<u8, Failure> {
    let limit = match (args.flag("--all"), args.number("--limit")?) {
        (true, Some(_)) => return Err(wrong("--limit and --all cannot be used together")),
        (true, None) => None,
        (false, limit) => Some(limit.unwrap_or(DEFAULT_DIFF_LIMIT)),
    };
    let old = open_file(&args.positionals[0])?;
    let new = open_file(&args.positionals[1])?;
    let _ = write!(
        out,
        "{}",
        review::render_diff(&review::diff(&old, &new), limit)
    );
    Ok(0)
}
