// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What the browser process hands its content processes for the error pages: this run's
//! secret and, in automated test runs only, the switch that makes the certificate page continue
//! by itself. Both travel in one environment variable that the browser process writes at
//! start-up, overwriting anything inherited, so nothing outside the browser can turn the switch
//! on in a normal run.

/// The value of the environment variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorPageEnvironment {
    /// 32 hexadecimal digits.
    pub secret: String,
    pub continue_automatically: bool,
}

const CONTINUE_SUFFIX: &str = ":continue";

impl ErrorPageEnvironment {
    pub fn encode(&self) -> String {
        if self.continue_automatically {
            format!("{}{CONTINUE_SUFFIX}", self.secret)
        } else {
            self.secret.clone()
        }
    }

    /// None unless `value` is exactly what `encode` makes.
    pub fn decode(value: &str) -> Option<Self> {
        let (secret, continue_automatically) = match value.strip_suffix(CONTINUE_SUFFIX) {
            Some(secret) => (secret, true),
            None => (value, false),
        };
        let valid = secret.len() == 32 && secret.bytes().all(|byte| byte.is_ascii_hexdigit());
        valid.then(|| Self {
            secret: secret.to_owned(),
            continue_automatically,
        })
    }
}

/// Whether the browser runs without a person (`--screenshot`, `--background`), judged from its
/// arguments before they are parsed: the decision is needed before any thread starts.
pub fn is_automated_run(arguments: impl IntoIterator<Item = String>) -> bool {
    arguments
        .into_iter()
        .any(|argument| argument == "--screenshot" || argument == "--background")
}

/// The test switch (`MERCURY_TEST_PROCEED_ON_CERTIFICATE_ERROR=1`) is honoured only in
/// automated runs.
pub fn automatic_continue_requested(automated: bool, variable: Option<&str>) -> bool {
    automated && variable == Some("1")
}
