// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What can go wrong between opening a PDF and drawing a page.
//!
//! The `Display` text is written for the person reading the viewer: plain sentences, no file
//! paths and no library names. The detail inside a variant is for logs only and may come from a
//! hostile file, so it is never shown as-is.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A failure of the PDF helper process, its backend or a request to them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PdfError {
    /// The file is encrypted and no password was given.
    PasswordRequired,
    /// A password was given and it is wrong.
    PasswordIncorrect,
    /// The file is not a PDF, or too damaged to show.
    Corrupt(String),
    /// The file uses something the viewer does not support.
    Unsupported(String),
    /// The file or one of its parts is beyond a limit in [`crate::types::limits`].
    TooLarge(String),
    /// Reading the file failed.
    Io(String),
    /// The PDFium library could not be found or loaded.
    LibraryMissing(String),
    /// A request that breaks the protocol or the limits (a page that does not exist, a tile
    /// outside its page, a scale out of range).
    InvalidRequest(String),
    /// A request that needs an open document arrived with none open.
    NoDocument,
    /// The helper process ended while the request was pending (it crashed, was killed, or its
    /// pipe broke).
    WorkerLost(String),
    /// The helper process did not answer in time and was ended.
    Timeout,
    /// The request was cancelled by the client before it ran.
    Cancelled,
}

impl fmt::Display for PdfError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::PasswordRequired => "This PDF needs a password.",
            Self::PasswordIncorrect => "That password is not right.",
            Self::Corrupt(_) => {
                "This file is not a PDF Mercury can show: it is damaged or not a PDF."
            }
            Self::Unsupported(_) => "This PDF uses a feature Mercury cannot show yet.",
            Self::TooLarge(_) => "This PDF is too large for Mercury's viewer.",
            Self::Io(_) => "Mercury could not read this PDF.",
            Self::LibraryMissing(_) => "Mercury's PDF viewer is not available in this copy.",
            Self::InvalidRequest(_) => "The PDF viewer made a request it should not have.",
            Self::NoDocument => "No PDF is open.",
            Self::WorkerLost(_) => "The PDF viewer stopped working on this file.",
            Self::Timeout => "This PDF took too long to show and was stopped.",
            Self::Cancelled => "The request was cancelled.",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for PdfError {}

impl PdfError {
    /// The technical detail for logs; empty for the variants that have none.
    pub fn detail(&self) -> &str {
        match self {
            Self::Corrupt(detail)
            | Self::Unsupported(detail)
            | Self::TooLarge(detail)
            | Self::Io(detail)
            | Self::LibraryMissing(detail)
            | Self::InvalidRequest(detail)
            | Self::WorkerLost(detail) => detail,
            Self::PasswordRequired
            | Self::PasswordIncorrect
            | Self::NoDocument
            | Self::Timeout
            | Self::Cancelled => "",
        }
    }

    /// Whether asking the user for a password can fix it.
    pub const fn needs_password(&self) -> bool {
        matches!(self, Self::PasswordRequired | Self::PasswordIncorrect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_never_repeats_the_detail() {
        let error = PdfError::Corrupt("xref at /Users/someone/secret.pdf".to_owned());
        assert!(!error.to_string().contains("secret"));
        assert_eq!(error.detail(), "xref at /Users/someone/secret.pdf");
    }

    #[test]
    fn only_the_password_errors_ask_for_a_password() {
        assert!(PdfError::PasswordRequired.needs_password());
        assert!(PdfError::PasswordIncorrect.needs_password());
        assert!(!PdfError::Timeout.needs_password());
    }
}
