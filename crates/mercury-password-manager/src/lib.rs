// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The password manager's logic between `mercury-passwords` (storage) and the browser's
//! interface: the list the settings window shows, editing, CSV import and export, the
//! authentication gate in front of revealing and copying passwords, the logins offered for a
//! manual fill, and saving HTTP sign-ins once they have worked.
//!
//! Nothing here touches a window, the engine or a real operating-system service: the
//! authenticator and the pasteboard are traits with fakes in [`testing`], and the store is any
//! `SecretBackend`. Time is passed in.

#![warn(missing_docs)]

mod auth;
mod backend;
mod display;
mod edit;
mod error;
mod export;
mod fill;
mod http_sign_in;
mod import;
mod list;
mod pasteboard;
pub mod testing;
mod worker;

pub use auth::{AUTH_GRACE_PERIOD, AuthGate, AuthOutcome, Authenticator, RefusingAuthenticator};
pub use backend::AnyBackend;
pub use display::{display_password, display_text, site_label};
pub use edit::update_login;
pub use error::ManagerError;
pub use export::{ExportedCsv, export_csv, write_private_file};
pub use fill::fill_candidates;
pub use http_sign_in::{HTTP_SAVE_TIMEOUT, PendingHttpSave, SignInSaves};
pub use import::{ImportReport, MAX_IMPORT_BYTES, import_csv, read_import_file};
pub use list::{LoginRow, filter_rows, login_rows};
pub use pasteboard::{CLIPBOARD_CLEAR_DELAY, CopiedSecret, Pasteboard};
pub use worker::{Deliver, Interaction, JobResult, StoreWorker, WorkerFailure};
