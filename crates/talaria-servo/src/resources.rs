// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Mercury's resource reader: Servo reads its error pages through it, and Mercury serves its
//! own (`mercury_site_security::error_pages`). Every other resource comes from Servo's files
//! (`servo-default-resources`, patched not to register a reader of its own; see
//! `third_party/servo-default-resources/MERCURY.md`).
//!
//! The error pages carry a per-run secret, made by the browser process at start-up and handed
//! to the content processes through the environment (`record_error_page_secret`): pages are
//! built in the content process, while the browser process recognises them and accepts
//! certificate exceptions from them.

use std::path::PathBuf;
use std::sync::OnceLock;

use servo::resources::{Resource, ResourceReaderMethods};
use servo_default_resources::DefaultResourceReader;

/// The environment variable that carries the error-page secret to the content processes.
const SECRET_VARIABLE: &str = "MERCURY_ERROR_PAGE_SECRET";

/// For automated tests only: the certificate page continues to the site as soon as it loads,
/// as if the user had pressed "Continue". Read in the content process.
const PROCEED_VARIABLE: &str = "MERCURY_TEST_PROCEED_ON_CERTIFICATE_ERROR";

struct MercuryResources;

static MERCURY_RESOURCES: MercuryResources = MercuryResources;

servo::submit_resource_reader!(&MERCURY_RESOURCES);

impl ResourceReaderMethods for MercuryResources {
    fn read(&self, resource: Resource) -> Vec<u8> {
        mercury_resource(&resource)
            .map(String::into_bytes)
            .unwrap_or_else(|| DefaultResourceReader.read(resource))
    }

    fn sandbox_access_files(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn sandbox_access_files_dirs(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// Mercury's version of `resource`, or None to use Servo's.
fn mercury_resource(resource: &Resource) -> Option<String> {
    match resource {
        Resource::BadCertHTML => Some(mercury_site_security::certificate_error_page(
            error_page_secret(),
            proceeds_automatically(),
        )),
        Resource::NetErrorHTML => Some(mercury_site_security::network_error_page(
            error_page_secret(),
        )),
        Resource::CrashHTML => Some(mercury_site_security::crash_page()),
        _ => None,
    }
}

/// Makes this run's error-page secret and records it for the content processes Servo will
/// start. Call it in `main`, in the browser process only, before any other thread exists
/// (next to `record_browser_process_id`).
pub fn record_error_page_secret() {
    let mut bytes = [0_u8; 16];
    if let Err(error) = getrandom::fill(&mut bytes) {
        // Without a secret no error page is recognised and no certificate exception is
        // accepted; pages still show.
        log::error!("no random source for the error-page secret: {error}");
        return;
    }
    let secret = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    // SAFETY: called on the main thread before any other thread has been started, so nothing
    // can read the environment concurrently.
    unsafe {
        std::env::set_var(SECRET_VARIABLE, secret);
    }
}

/// This run's error-page secret, or "" if none was recorded (then nothing matches it).
pub(crate) fn error_page_secret() -> &'static str {
    static SECRET: OnceLock<String> = OnceLock::new();
    SECRET.get_or_init(|| {
        std::env::var(SECRET_VARIABLE)
            .ok()
            .filter(|secret| secret.len() == 32 && secret.bytes().all(|b| b.is_ascii_hexdigit()))
            .unwrap_or_default()
    })
}

fn proceeds_automatically() -> bool {
    std::env::var_os(PROCEED_VARIABLE).is_some_and(|value| value == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercury_serves_its_error_pages_and_servos_other_resources() {
        for resource in [
            Resource::BadCertHTML,
            Resource::NetErrorHTML,
            Resource::CrashHTML,
        ] {
            let page = String::from_utf8(MercuryResources.read(resource)).expect("text");
            assert!(page.contains("Mercury Browser"), "{page}");
        }
        assert_eq!(
            MercuryResources.read(Resource::DomainList),
            DefaultResourceReader.read(Resource::DomainList)
        );
        assert!(!MercuryResources.read(Resource::HstsPreloadList).is_empty());
        assert!(!MercuryResources.read(Resource::BrokenImageIcon).is_empty());
    }

    #[test]
    fn servos_reader_is_mercurys() {
        // The one registered reader is Mercury's: Servo's own would serve its plain page.
        let page = servo::resources::read_string(Resource::NetErrorHTML);
        assert!(page.contains("mercury-error-page"), "{page}");
    }
}
