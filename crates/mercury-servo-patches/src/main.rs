// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use mercury_servo_patches::commands::{Repo, UpgradeTarget};
use mercury_servo_patches::registry::Registry;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
usage: mercury-servo-patches <command>

  vendor <crate>@<version>   unpack the published crate into third_party/<crate>/ and record it
  diff <crate>               print Mercury's changes to <crate> against the published crate
  export                     write third_party/patches/<crate>.patch for every vendored crate
  check                      fail unless every vendored crate is the published crate plus its
                             exported patch, with a `Mercury:` comment in every changed file
  upgrade <crate>@<version>  move <crate> to <version>, merging Mercury's changes into it
  upgrade --servo <version>  the same for every vendored crate from Servo's repository

Run it from anywhere in the repository: cargo run --release -p mercury-servo-patches -- check
The network is used only to download a published crate missing from Cargo's cache; set
MERCURY_SERVO_PATCHES_OFFLINE=1 (or CARGO_NET_OFFLINE=true) to forbid it.
";

/// The nearest directory at or above `start` with a `Cargo.lock` and a `third_party/`.
fn find_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join("Cargo.lock").is_file() && dir.join("third_party").is_dir())
        .map(Path::to_owned)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if matches!(args.as_slice(), [] | ["-h" | "--help" | "help", ..]) {
        print!("{USAGE}");
        return if args.is_empty() {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        };
    }
    let Some(root) = std::env::current_dir().ok().and_then(|dir| find_root(&dir)) else {
        eprintln!(
            "mercury-servo-patches: run it inside the Mercury repository (no Cargo.lock and third_party/ found)"
        );
        return ExitCode::FAILURE;
    };
    let repo = Repo {
        registry: Registry::from_env(&root),
        cargo: Some(
            std::env::var_os("CARGO").map_or_else(|| PathBuf::from("cargo"), PathBuf::from),
        ),
        git: true,
        root,
    };
    let mut out = std::io::stdout().lock();
    let result = match args.as_slice() {
        ["vendor", spec] => repo.vendor(spec, &mut out),
        ["diff", name] => repo.diff(name, &mut out),
        ["export"] => repo.export(&mut out),
        ["check"] => repo.check(&mut out),
        ["upgrade", "--servo", version] => repo
            .upgrade(UpgradeTarget::Servo((*version).to_owned()), &mut out)
            .and_then(conflicts_fail),
        ["upgrade", spec] => repo
            .upgrade(UpgradeTarget::Crate((*spec).to_owned()), &mut out)
            .and_then(conflicts_fail),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            for line in error.0.lines() {
                eprintln!("mercury-servo-patches: {line}");
            }
            ExitCode::FAILURE
        }
    }
}

fn conflicts_fail(
    report: mercury_servo_patches::commands::UpgradeReport,
) -> mercury_servo_patches::error::Result<()> {
    match report.conflicts() {
        0 => Ok(()),
        n => Err(mercury_servo_patches::error::Error(format!(
            "the upgrade left {n} conflicting files to resolve (listed above)"
        ))),
    }
}
