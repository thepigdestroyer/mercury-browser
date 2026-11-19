// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What `scripts/ci/github-env.sh` hands to the later steps of a GitHub Actions job: the
//! difference between two snapshots of the environment (`env -0`, taken before and after the
//! shell evaluated `scripts/macos/gstreamer.sh env`), checked against an allow-list and
//! written in the forms `$GITHUB_ENV` and `$GITHUB_PATH` take.
//!
//! Only these variables may change, and the first four must be set: `MERCURY_GSTREAMER_PREFIX`,
//! `PKG_CONFIG`, `PKG_CONFIG_LIBDIR`, `PATH` (the old `PATH` with directories added in front;
//! they go to `$GITHUB_PATH`); `PKG_CONFIG_PATH` may only be unset, and is exported as empty,
//! which pkg-config treats the same. Each value is written with GitHub's multi-line form
//! (`NAME<<DELIMITER`, value, `DELIMITER`) using a random delimiter checked not to occur in the
//! value, so spaces, colons, quotes and newlines arrive unchanged.

use std::collections::BTreeMap;

/// Variables the command must set.
pub const MUST_SET: [&str; 4] = [
    "MERCURY_GSTREAMER_PREFIX",
    "PKG_CONFIG",
    "PKG_CONFIG_LIBDIR",
    "PATH",
];
/// Variables the command may unset.
pub const MAY_UNSET: [&str; 1] = ["PKG_CONFIG_PATH"];
/// Variables the shell itself changes between the snapshots; never compared.
const IGNORED: [&str; 4] = ["_", "SHLVL", "PWD", "OLDPWD"];

/// An environment snapshot.
pub type Environment = BTreeMap<String, String>;

/// Reads `env -0` output: `NAME=value` entries, each ended by a NUL byte.
pub fn parse_environment(bytes: &[u8]) -> Result<Environment, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| "the environment is not UTF-8 text".to_owned())?;
    let mut environment = Environment::new();
    for entry in text.split('\0').filter(|entry| !entry.is_empty()) {
        let (name, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("not an environment entry: {entry:?}"))?;
        environment.insert(name.to_owned(), value.to_owned());
    }
    Ok(environment)
}

/// What to export: the text for `$GITHUB_ENV` and for `$GITHUB_PATH`, and the names exported
/// and directories added, for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    pub names: Vec<String>,
    pub path_prefix: Vec<String>,
    pub env_text: String,
    pub path_text: String,
}

/// Compares the snapshots. `delimiter` makes a fresh random heredoc delimiter for a value, or
/// fails with the reason, which is returned at once. `Err` is the reason for refusing (or that
/// failure), in which case nothing may be exported.
pub fn export(
    before: &Environment,
    after: &Environment,
    mut delimiter: impl FnMut() -> Result<String, String>,
) -> Result<Export, String> {
    let kept = |environment: &Environment| -> Environment {
        environment
            .iter()
            .filter(|(name, _)| !IGNORED.contains(&name.as_str()))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    };
    let (before, after) = (kept(before), kept(after));
    let removed: Vec<&String> = before
        .keys()
        .filter(|name| !after.contains_key(*name))
        .collect();
    let changed: Vec<&String> = after
        .iter()
        .filter(|(name, value)| before.get(*name) != Some(*value))
        .map(|(name, _)| name)
        .collect();

    let mut unexpected: Vec<&str> = changed
        .iter()
        .filter(|name| !MUST_SET.contains(&name.as_str()))
        .chain(
            removed
                .iter()
                .filter(|name| !MAY_UNSET.contains(&name.as_str())),
        )
        .map(|name| name.as_str())
        .collect();
    unexpected.sort_unstable();
    if !unexpected.is_empty() {
        return Err(format!(
            "the command changed {}, which is not on the allow-list",
            unexpected.join(", ")
        ));
    }
    let missing: Vec<&str> = MUST_SET
        .into_iter()
        .filter(|name| !changed.iter().any(|changed| changed.as_str() == *name))
        .collect();
    if !missing.is_empty() {
        return Err(format!("the command did not set {}", missing.join(", ")));
    }

    let old_path = before.get("PATH").map(String::as_str).unwrap_or_default();
    let new_path = &after["PATH"];
    let prefix = new_path
        .strip_suffix(old_path)
        .and_then(|prefix| prefix.strip_suffix(':'))
        .filter(|_| !old_path.is_empty())
        .ok_or("PATH is not the old PATH with directories added in front")?;
    let path_prefix: Vec<String> = prefix.split(':').map(str::to_owned).collect();
    if path_prefix
        .iter()
        .any(|entry| entry.is_empty() || entry.contains('\n'))
    {
        return Err("PATH gains an empty directory or one containing a newline".into());
    }

    let values: Vec<(&str, &str)> = changed
        .iter()
        .filter(|name| name.as_str() != "PATH")
        .map(|name| (name.as_str(), after[*name].as_str()))
        .chain(removed.iter().map(|name| (name.as_str(), "")))
        .collect();
    let mut env_text = String::new();
    for (name, value) in &values {
        let delimiter = delimiter()?;
        if value.split('\n').any(|line| line == delimiter) {
            return Err(format!("delimiter collision for {name}"));
        }
        env_text.push_str(&format!("{name}<<{delimiter}\n{value}\n{delimiter}\n"));
    }
    // The runner puts the most recently added line first, so they are written last to first
    // to keep the order the command chose.
    let path_text = path_prefix
        .iter()
        .rev()
        .map(|entry| format!("{entry}\n"))
        .collect();
    Ok(Export {
        names: values.iter().map(|(name, _)| (*name).to_owned()).collect(),
        path_prefix,
        env_text,
        path_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(entries: &[(&str, &str)]) -> Environment {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn counter() -> impl FnMut() -> Result<String, String> {
        let mut next = 0;
        move || {
            next += 1;
            Ok(format!("EOF{next}"))
        }
    }

    fn base() -> Environment {
        environment(&[
            ("PATH", "/usr/bin:/bin"),
            ("HOME", "/Users/runner"),
            ("PKG_CONFIG_PATH", "/opt/homebrew/lib/pkgconfig"),
            ("SHLVL", "1"),
            ("_", "/usr/bin/env"),
        ])
    }

    fn gstreamer(after: &mut Environment) {
        after.insert("MERCURY_GSTREAMER_PREFIX".into(), "/g/My Prefix".into());
        after.insert("PKG_CONFIG".into(), "/g/bin/pkg-config".into());
        after.insert("PKG_CONFIG_LIBDIR".into(), "/g/lib/pkgconfig".into());
        after.insert("PATH".into(), "/g/bin:/g/sbin:/usr/bin:/bin".into());
        after.remove("PKG_CONFIG_PATH");
        after.insert("SHLVL".into(), "2".into());
        after.insert("_".into(), "/usr/bin/env".into());
    }

    #[test]
    fn reads_env_dash_zero() {
        assert_eq!(
            parse_environment(b"A=1\0B=x=y\nz\0EMPTY=\0"),
            Ok(environment(&[("A", "1"), ("B", "x=y\nz"), ("EMPTY", "")]))
        );
        assert!(parse_environment(b"no equals sign\0").is_err());
        assert!(parse_environment(b"A=\xff\0").is_err());
    }

    #[test]
    fn exports_what_gstreamer_env_sets() {
        let before = base();
        let mut after = base();
        gstreamer(&mut after);
        let export = export(&before, &after, counter()).expect("accepted");
        assert_eq!(
            export.names,
            [
                "MERCURY_GSTREAMER_PREFIX",
                "PKG_CONFIG",
                "PKG_CONFIG_LIBDIR",
                "PKG_CONFIG_PATH"
            ]
        );
        assert_eq!(
            export.env_text,
            "MERCURY_GSTREAMER_PREFIX<<EOF1\n/g/My Prefix\nEOF1\n\
             PKG_CONFIG<<EOF2\n/g/bin/pkg-config\nEOF2\n\
             PKG_CONFIG_LIBDIR<<EOF3\n/g/lib/pkgconfig\nEOF3\n\
             PKG_CONFIG_PATH<<EOF4\n\nEOF4\n"
        );
        assert_eq!(export.path_prefix, ["/g/bin", "/g/sbin"]);
        assert_eq!(export.path_text, "/g/sbin\n/g/bin\n");
    }

    #[test]
    fn refuses_anything_else() {
        let before = base();
        let refused = |change: &dyn Fn(&mut Environment)| {
            let mut after = base();
            gstreamer(&mut after);
            change(&mut after);
            export(&before, &after, counter()).expect_err("refused")
        };
        assert_eq!(
            refused(&|after| {
                after.insert("RUSTFLAGS".into(), "-C x".into());
                after.remove("HOME");
            }),
            "the command changed HOME, RUSTFLAGS, which is not on the allow-list"
        );
        assert_eq!(
            refused(&|after| {
                after.remove("PKG_CONFIG");
                after.insert("PATH".into(), "/usr/bin:/bin".into());
            }),
            "the command did not set PKG_CONFIG, PATH"
        );
        for path in [
            "/usr/bin:/bin:/g/bin",
            "/g/bin:/usr/bin",
            "/g/bin/usr/bin:/bin",
        ] {
            assert_eq!(
                refused(&|after| {
                    after.insert("PATH".into(), path.into());
                }),
                "PATH is not the old PATH with directories added in front",
                "{path}"
            );
        }
        for path in ["/g/bin::/usr/bin:/bin", "/g/\nbin:/usr/bin:/bin"] {
            assert_eq!(
                refused(&|after| {
                    after.insert("PATH".into(), path.into());
                }),
                "PATH gains an empty directory or one containing a newline"
            );
        }
        assert_eq!(
            refused(&|after| {
                after.insert("PKG_CONFIG".into(), "a\nEOF2\nb".into());
            }),
            "delimiter collision for PKG_CONFIG"
        );
    }

    #[test]
    fn a_failing_delimiter_is_returned_at_once() {
        let before = base();
        let mut after = base();
        gstreamer(&mut after);
        let mut calls = 0;
        let result = export(&before, &after, || {
            calls += 1;
            Err("no entropy".into())
        });
        assert_eq!(result, Err("no entropy".into()));
        assert_eq!(calls, 1);
    }

    #[test]
    fn an_empty_old_path_is_refused() {
        let mut before = base();
        before.insert("PATH".into(), String::new());
        let mut after = base();
        gstreamer(&mut after);
        after.insert("PATH".into(), "/g/bin:".into());
        assert_eq!(
            export(&before, &after, counter()),
            Err("PATH is not the old PATH with directories added in front".into())
        );
    }
}
