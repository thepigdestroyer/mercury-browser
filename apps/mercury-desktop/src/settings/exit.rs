// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What happens to the settings and the user's data when the browser quits.

use mercury_settings::Settings;

/// The history lane's hook: clears the browsing history when the user asked for that at quit
/// (`privacy.clear_on_exit.history`). Called once by
/// `SettingsHandle::clear_history_on_exit_if_asked`, after the event loop has ended and before
/// the history database closes, both within the quit's one deadline. Cookies, site data and the
/// cache are cleared by the engine itself as it shuts down (`talaria_servo::ExitClearing`).
pub(crate) fn clear_history_on_exit(_settings: &Settings) {
    crate::profile_data::clear_history_on_exit();
}
