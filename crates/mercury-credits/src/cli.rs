// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The command line: `generate` and `check`.
use crate::cargo::Options;
use crate::error::{Error, Result};
use std::path::PathBuf;

pub const USAGE: &str = "\
usage: mercury-credits <command> --package <name> --target <triple> [options]

  generate --out <dir>   write <dir>/credits.html: the licence text, copyright line and notices of
                         every crate built into the app (creates <dir>; writes nothing on a problem)
  check                  the same analysis; write nothing; fail if a crate cannot be credited

options:
  --package <name>         the app's package (mercury-desktop)
  --target <triple>        aarch64-apple-darwin or x86_64-unknown-linux-gnu
  --runtime-only           leave out build-script edges (the default includes them)
  --manifest-path <path>   the workspace Cargo.toml (default: Cargo.toml in the current directory)
  --out <dir>              (generate) the directory for credits.html

Run it from the repository root: cargo run --release -p mercury-credits -- check --package mercury-desktop --target aarch64-apple-darwin
Exit status: 0 success (warnings are printed), 1 a crate cannot be credited or a tool failed, 2 bad usage.
The tool reads cargo tree, cargo metadata, deny.toml, third_party/vendor.toml, NOTICE, THIRD-PARTY-NOTICES.md,
LICENSES/ and crates/mercury-credits/credits.toml, and uses no network of its own; the crate sources must be
in cargo's cache (run `cargo fetch --locked` first). $CARGO names the cargo to run. Messages go to standard
error; the last line of a successful run is
`mercury-credits: N crates credited (R registry, V vendored, M Mercury's own), T distinct licence texts,
S crates on a standard text, X bytes`.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Generate,
    Check,
}

/// A parsed command line.
#[derive(Debug, Clone)]
pub struct Invocation {
    pub command: Command,
    pub options: Options,
    pub out: Option<PathBuf>,
}

/// What the arguments ask for.
#[derive(Debug)]
pub enum Parsed {
    Help,
    Run(Invocation),
}

fn usage(message: impl Into<String>) -> Error {
    Error::Usage(message.into())
}

/// Parses the arguments after the program name. `cargo` is the `cargo` to run.
pub fn parse<I, S>(args: I, cargo: PathBuf) -> Result<Parsed>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter().map(|arg| arg.as_ref().to_owned());
    let command = match args.next().as_deref() {
        None => return Err(usage("no command given")),
        Some("-h" | "--help" | "help") => return Ok(Parsed::Help),
        Some("generate") => Command::Generate,
        Some("check") => Command::Check,
        Some(other) => return Err(usage(format!("unknown command {other:?}"))),
    };
    let mut package = None;
    let mut target = None;
    let mut out = None;
    let mut manifest_path = None;
    let mut runtime_only = false;
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_owned(), Some(value.to_owned()))
            }
            _ => (arg, None),
        };
        let mut value = |flag: &str| -> Result<String> {
            inline
                .clone()
                .or_else(|| args.next())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| usage(format!("{flag} needs a value")))
        };
        match flag.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "--runtime-only" => {
                if inline.is_some() {
                    return Err(usage("--runtime-only takes no value"));
                }
                runtime_only = true;
            }
            "--package" | "-p" => package = Some(value("--package")?),
            "--target" => target = Some(value("--target")?),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--manifest-path" => manifest_path = Some(PathBuf::from(value("--manifest-path")?)),
            other => return Err(usage(format!("unknown option {other:?}"))),
        }
    }
    let package = package.ok_or_else(|| usage("--package is required"))?;
    let target = target.ok_or_else(|| usage("--target is required"))?;
    match (command, &out) {
        (Command::Generate, None) => return Err(usage("generate needs --out <dir>")),
        (Command::Check, Some(_)) => return Err(usage("check writes nothing and takes no --out")),
        _ => {}
    }
    let manifest_path = manifest_path.unwrap_or_else(|| PathBuf::from("Cargo.toml"));
    let manifest_path = std::path::absolute(&manifest_path).unwrap_or(manifest_path);
    Ok(Parsed::Run(Invocation {
        command,
        options: Options {
            package,
            target,
            runtime_only,
            manifest_path,
            cargo,
        },
        out,
    }))
}
