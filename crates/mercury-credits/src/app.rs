// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The two commands, `generate` and `check`, as functions the binary and the tests share.
use crate::cargo::{self, Metadata, Options};
use crate::collect::{self, Collected, Inputs, Report};
use crate::error::{Error, Result};
use crate::page;
use std::path::{Path, PathBuf};

/// What a run found and, when nothing is wrong, the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub report: Report,
    pub collected: Option<Collected>,
    /// The page; absent when there are problems.
    pub page: Option<String>,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        self.report.problems.is_empty()
    }

    /// `mercury-credits: N crates credited (...)`, for a run without problems.
    pub fn summary(&self) -> Option<String> {
        let collected = self.collected.as_ref()?;
        let page = self.page.as_ref()?;
        Some(format!(
            "mercury-credits: {} crates credited ({} registry, {} vendored, {} Mercury's own), {} distinct licence texts, {} crates on a standard text, {} bytes",
            collected.credited_count(),
            collected.registry_count(),
            collected.vendored_count(),
            collected.own.len(),
            collected.blocks.len(),
            collected.standard_text_crates.len(),
            page.len()
        ))
    }
}

/// Works out the page from Cargo's output and the repository files; a pure function of its
/// inputs, so two runs give the same bytes.
pub fn analyse_inputs(inputs: &Inputs<'_>) -> Outcome {
    match collect::collect(inputs) {
        Ok(collected) => {
            let report = collected.report.clone();
            let page = report.problems.is_empty().then(|| page::render(&collected));
            Outcome {
                report,
                collected: Some(collected),
                page,
            }
        }
        Err(error) => failed(&error),
    }
}

fn failed(error: &Error) -> Outcome {
    Outcome {
        report: Report {
            problems: vec![error.to_string()],
            warnings: Vec::new(),
        },
        collected: None,
        page: None,
    }
}

/// Asks Cargo for the tree and the metadata and analyses them. A Cargo failure is a problem.
pub fn analyse(options: &Options) -> Outcome {
    let loaded = cargo::run_tree(options).and_then(|tree| {
        let metadata: Metadata = cargo::run_metadata(options)?;
        Ok((tree, metadata))
    });
    match loaded {
        Ok((tree, metadata)) => {
            let root = options.root();
            analyse_inputs(&Inputs {
                root: &root,
                tree: &tree,
                metadata: &metadata,
                target: &options.target,
                runtime_only: options.runtime_only,
            })
        }
        Err(error) => failed(&error),
    }
}

/// Writes `credits.html` into `out` (created if needed) through a temporary file and a rename.
pub fn write_page(out: &Path, page: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(out).map_err(|source| Error::Write {
        path: out.to_owned(),
        source,
    })?;
    let temporary = out.join(format!(".credits.html.{}.tmp", std::process::id()));
    let target = out.join("credits.html");
    std::fs::write(&temporary, page).map_err(|source| Error::Write {
        path: temporary.clone(),
        source,
    })?;
    std::fs::rename(&temporary, &target).map_err(|source| {
        let _ = std::fs::remove_file(&temporary);
        Error::Write {
            path: target.clone(),
            source,
        }
    })?;
    Ok(target)
}
