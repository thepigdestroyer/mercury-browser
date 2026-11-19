// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The errors that stop the tool before it can say anything about the crates. Problems found in
//! the crates themselves are not errors: they are collected in [`crate::collect::Report`].
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The command line is wrong (exit status 2).
    #[error("{0}")]
    Usage(String),
    #[error("cannot read {}: {source}", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write {}: {source}", .path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A file the tool reads (`deny.toml`, `vendor.toml`, `credits.toml`, `Cargo.toml`, the output
    /// of cargo) does not have the expected shape.
    #[error("{what}: {message}")]
    Parse { what: String, message: String },
    /// A `cargo` command could not be run or failed.
    #[error("{0}")]
    Cargo(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn parse(what: impl Into<String>, message: impl ToString) -> Self {
        Self::Parse {
            what: what.into(),
            message: message.to_string(),
        }
    }
}
