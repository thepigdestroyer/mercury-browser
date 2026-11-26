// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Network policy data and pure logic that needs no engine.
//!
//! - [`signin`][]: the built-in sign-in exception pairs, which keep third-party cookies for a
//!   company's own sites embedded in each other (Google and YouTube, Microsoft, Apple).
//! - [`tls`][]: [`TlsFacts`] (the connection facts the engine reports for a main-frame document)
//!   and [`summarize_tls`], which turns them into the text the padlock details show.

#![forbid(unsafe_code)]

pub mod signin;
pub mod tls;

pub use signin::{SigninPair, signin_exceptions};
pub use tls::{TlsFacts, TlsSummary, summarize_tls};
