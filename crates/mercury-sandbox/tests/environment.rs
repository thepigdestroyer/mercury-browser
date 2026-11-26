// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use std::ffi::{OsStr, OsString};
use std::path::Path;

use mercury_sandbox::{content_environment, content_process_command};

fn vars(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs
        .iter()
        .map(|&(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

fn value<'a>(env: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsString> {
    env.iter()
        .find(|(key, _)| key == OsStr::new(name))
        .map(|(_, value)| value)
}

#[test]
fn secrets_and_injection_variables_are_dropped() {
    let kept = content_environment(vars(&[
        ("HOME", "/Users/a"),
        ("PATH", "/usr/bin:/bin"),
        ("SSH_AUTH_SOCK", "/private/tmp/agent.sock"),
        ("AWS_SECRET_ACCESS_KEY", "secret"),
        ("DYLD_INSERT_LIBRARIES", "/private/tmp/evil.dylib"),
    ]));
    for name in [
        "HOME",
        "PATH",
        "SSH_AUTH_SOCK",
        "AWS_SECRET_ACCESS_KEY",
        "DYLD_INSERT_LIBRARIES",
    ] {
        assert!(
            value(&kept, name).is_none(),
            "{name} must not reach the content process"
        );
    }
}

#[test]
fn the_names_the_content_process_reads_survive() {
    let names = [
        ("MERCURY_ERROR_PAGE_SECRET", "page"),
        ("MERCURY_BROWSER_PROCESS_ID", "4242"),
        ("MERCURY_FAKE_MEDIA_DEVICES", "1"),
        ("MERCURY_MSE_AV1", "1"),
        ("GST_REGISTRY_1_0", "/opt/gst/registry.bin"),
        ("GIO_MODULE_DIR", "/opt/gio"),
        ("XDG_CACHE_HOME", "/Users/a/Library/Caches/x"),
        ("XDG_DATA_HOME", "/Users/a/Library/Data/x"),
        ("XDG_CONFIG_HOME", "/Users/a/Library/Config/x"),
        ("XDG_RUNTIME_DIR", "/private/var/folders/run/x"),
        ("RUST_LOG", "info"),
        ("RUST_BACKTRACE", "1"),
        ("LANG", "en_US.UTF-8"),
        ("LC_ALL", "C"),
        ("TZ", "Europe/Berlin"),
        ("__CF_USER_TEXT_ENCODING", "0x1F5:0:0"),
        ("DYLD_FALLBACK_LIBRARY_PATH", "/opt/gst/lib"),
    ];
    let kept = content_environment(vars(&names));
    for (name, expected) in names {
        assert_eq!(
            value(&kept, name),
            Some(&OsString::from(expected)),
            "{name} must reach the content process"
        );
    }
}

#[test]
fn registry_update_is_forced_off() {
    let inherited = content_environment(vars(&[("GST_REGISTRY_UPDATE", "yes")]));
    assert_eq!(
        value(&inherited, "GST_REGISTRY_UPDATE"),
        Some(&OsString::from("no"))
    );
    let absent = content_environment(vars(&[]));
    assert_eq!(
        value(&absent, "GST_REGISTRY_UPDATE"),
        Some(&OsString::from("no"))
    );
}

#[test]
fn order_is_sorted_and_unique() {
    let kept = content_environment(vars(&[
        ("TZ", "UTC"),
        ("MERCURY_MSE_AV1", "1"),
        ("LANG", "C"),
        ("LANG", "en_US.UTF-8"),
        ("GST_PLUGIN_PATH", "/opt/gst"),
        ("MERCURY_BROWSER_PROCESS_ID", "7"),
    ]));
    let names: Vec<&OsString> = kept.iter().map(|(name, _)| name).collect();
    assert!(
        names.windows(2).all(|pair| pair[0] < pair[1]),
        "names must be strictly increasing: {names:?}"
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_value_is_kept_byte_exact() {
    use std::os::unix::ffi::OsStringExt;

    let raw = OsString::from_vec(vec![b'/', 0xff, 0xfe, b'x']);
    let kept = content_environment(vec![(OsString::from("MERCURY_RAW"), raw.clone())]);
    assert_eq!(value(&kept, "MERCURY_RAW"), Some(&raw));
}

#[test]
fn command_has_exactly_the_allow_list_and_no_arguments_of_its_own() {
    let inherited = vars(&[
        ("HOME", "/Users/a"),
        ("MERCURY_BROWSER_PROCESS_ID", "42"),
        ("DYLD_INSERT_LIBRARIES", "/private/tmp/evil.dylib"),
        ("LANG", "en_US.UTF-8"),
    ]);
    let expected = content_environment(inherited.clone());
    let executable = Path::new("/Applications/Mercury.app/Contents/MacOS/mercury-browser");

    let command = content_process_command(executable, inherited);

    assert_eq!(command.get_program(), executable.as_os_str());
    assert_eq!(command.get_args().count(), 0);
    let set: Vec<(OsString, OsString)> = command
        .get_envs()
        .filter_map(|(name, value)| value.map(|value| (name.to_os_string(), value.to_os_string())))
        .collect();
    assert_eq!(set, expected);
}
