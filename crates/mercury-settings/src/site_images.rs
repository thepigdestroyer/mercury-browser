// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The file `site-images.json`: the sites whose images are turned off. Item 4 (images) writes
//! this module on the model of `SettingsStore` (atomic private writes, bounded reads, a damaged
//! file moved aside, a newer version never saved over). It is a file of its own, not a field of
//! `Settings` (brief decision O12): the per-site table moves into `mercury-site-settings` later,
//! which imports this file and deletes it.
