// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Mercury's resource reader: Servo reads its error pages through it, and Mercury serves its
//! own (`mercury_site_security::error_pages`). Every other resource comes from Servo's files
//! (`servo-default-resources`, patched not to register a reader of its own; see
//! `third_party/servo-default-resources/MERCURY.md`), except the HSTS preload list, which is
//! Mercury's own (`resources/hsts_preload.fstmap`, compiled in, refreshed with
//! `make hsts-preload-refresh`; the vendored crate's copy stays as published because the patch
//! tool refuses binary files).
//!
//! The error pages carry a per-run secret, made by the browser process at start-up and handed
//! to the content processes through the environment (`record_error_page_secret`): pages are
//! built in the content process, while the browser process recognises them and accepts
//! certificate exceptions from them.

use std::path::PathBuf;
use std::sync::OnceLock;

use mercury_site_security::{ErrorPageEnvironment, automatic_continue_requested};
use servo::resources::{Resource, ResourceReaderMethods};
use servo_default_resources::DefaultResourceReader;

/// The environment variable that carries the error-page secret to the content processes.
const SECRET_VARIABLE: &str = "MERCURY_ERROR_PAGE_SECRET";

/// For automated tests only: the certificate page continues to the site as soon as it loads,
/// as if the user had pressed "Continue". Read by the browser process, in automated runs only.
const PROCEED_VARIABLE: &str = "MERCURY_TEST_PROCEED_ON_CERTIFICATE_ERROR";

/// The HSTS preload list (an `fst` map from host name to `2 * rank + include_subdomains`), with
/// the manifest beside it (`hsts_preload.manifest.json`) saying where it came from and when.
/// The `hsts-preload` tool in `crates/mercury-devtools` writes both.
const HSTS_PRELOAD_LIST: &[u8] = include_bytes!("../resources/hsts_preload.fstmap");

struct MercuryResources;

static MERCURY_RESOURCES: MercuryResources = MercuryResources;

servo::submit_resource_reader!(&MERCURY_RESOURCES);

impl ResourceReaderMethods for MercuryResources {
    fn read(&self, resource: Resource) -> Vec<u8> {
        if matches!(resource, Resource::HstsPreloadList) {
            return HSTS_PRELOAD_LIST.to_vec();
        }
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
/// start, overwriting any inherited value. `automated` says whether this is a `--screenshot` or
/// `--background` run: only then is the test switch `MERCURY_TEST_PROCEED_ON_CERTIFICATE_ERROR`
/// honoured. Call it in `main`, in the browser process only, before any other thread exists
/// (next to `record_browser_process_id`).
pub fn record_error_page_secret(automated: bool) {
    let mut bytes = [0_u8; 16];
    let secret = match getrandom::fill(&mut bytes) {
        Ok(()) => bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        Err(error) => {
            // Without a secret no error page is recognised and no certificate exception is
            // accepted; pages still show.
            log::error!("no random source for the error-page secret: {error}");
            String::new()
        }
    };
    let requested = std::env::var(PROCEED_VARIABLE).ok();
    let continue_automatically = automatic_continue_requested(automated, requested.as_deref());
    if continue_automatically {
        log::warn!("{PROCEED_VARIABLE}: certificate pages continue by themselves in this run");
    } else if requested.is_some() {
        log::warn!("{PROCEED_VARIABLE} is ignored: it applies to automated runs only");
    }
    let value = ErrorPageEnvironment {
        secret,
        continue_automatically,
    }
    .encode();
    // SAFETY: called on the main thread before any other thread has been started, so nothing
    // can read the environment concurrently.
    unsafe {
        std::env::set_var(SECRET_VARIABLE, value);
    }
}

/// What the browser process recorded, or None (then nothing matches the secret and nothing
/// continues automatically).
fn recorded_environment() -> Option<&'static ErrorPageEnvironment> {
    static RECORDED: OnceLock<Option<ErrorPageEnvironment>> = OnceLock::new();
    RECORDED
        .get_or_init(|| {
            std::env::var(SECRET_VARIABLE)
                .ok()
                .and_then(|value| ErrorPageEnvironment::decode(&value))
        })
        .as_ref()
}

/// This run's error-page secret, or "" if none was recorded (then nothing matches it).
pub(crate) fn error_page_secret() -> &'static str {
    recorded_environment().map_or("", |recorded| recorded.secret.as_str())
}

fn proceeds_automatically() -> bool {
    recorded_environment().is_some_and(|recorded| recorded.continue_automatically)
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
    fn mercury_serves_its_own_hsts_preload_list() {
        let list = MercuryResources.read(Resource::HstsPreloadList);
        // `assert!` rather than `assert_eq!`: a failure must not print 1.9 MB of bytes.
        assert!(list == HSTS_PRELOAD_LIST, "not the compiled-in list");
        // ... and that is the file the `hsts-preload` tool refreshes: its default output
        // (`DEFAULT_DATA` in `mercury-devtools`, pinned to this path by that crate's
        // `tests/hsts_data.rs`), read from disk by that path.
        let refreshed = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/talaria-servo/resources/hsts_preload.fstmap"
        ))
        .expect("read the file the tool refreshes");
        assert!(list == refreshed, "not the file the tool refreshes");
        // The header of an `fst` file: format version 3, as a little-endian u64.
        assert_eq!(list[..8], [3, 0, 0, 0, 0, 0, 0, 0]);
        assert!(list.len() > 1_000_000, "{} bytes", list.len());
    }

    #[test]
    fn servos_reader_serves_the_hsts_preload_list() {
        // The registered reader is Mercury's, so this is what the network stack loads.
        let list = servo::resources::read_bytes(Resource::HstsPreloadList);
        assert!(list == HSTS_PRELOAD_LIST, "not the compiled-in list");
    }

    #[test]
    fn servos_reader_is_mercurys() {
        // The one registered reader is Mercury's: Servo's own would serve its plain page.
        let page = servo::resources::read_string(Resource::NetErrorHTML);
        assert!(page.contains("mercury-error-page"), "{page}");
    }
}
