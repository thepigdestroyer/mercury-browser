// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Finding a crate's published `.crate` archive and checksum.
//!
//! Archives come from Cargo's own download cache when it has them, then from
//! the tool's cache in `target/mercury-servo-patches/`, then from crates.io.
//! A crate that is patched is never downloaded by Cargo, so on a fresh machine
//! the archive comes from crates.io. Every archive is checked against a known
//! SHA-256 before it is used. Downloads use `curl` over HTTPS only.

use crate::archive::{sha256_hex, verify};
use crate::error::{Context, Result};
use crate::fail;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Registry {
    /// Cargo's home; its `registry/cache/*/` directories are searched.
    pub cargo_home: Option<PathBuf>,
    /// Where downloaded archives are kept.
    pub cache_dir: PathBuf,
    /// Whether crates.io may be contacted.
    pub network: bool,
}

fn file_name(name: &str, version: &str) -> String {
    format!("{name}-{version}.crate")
}

/// The path of `name` in the crates.io sparse index.
pub fn index_path(name: &str) -> String {
    let name = name.to_ascii_lowercase();
    match name.len() {
        1 => format!("1/{name}"),
        2 => format!("2/{name}"),
        3 => format!("3/{}/{name}", &name[..1]),
        _ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
    }
}

/// The checksum of `version` in a sparse-index file (one JSON object per line).
pub fn checksum_in_index(index: &str, name: &str, version: &str) -> Result<String> {
    for line in index.lines().filter(|line| !line.trim().is_empty()) {
        let record: serde_json::Value = serde_json::from_str(line)
            .map_err(|error| fail!("crates.io index for {name}: {error}"))?;
        if record["vers"] == version {
            return record["cksum"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| fail!("crates.io index for {name} {version} has no checksum"));
        }
    }
    Err(fail!("{name} {version} is not published on crates.io"))
}

fn curl(url: &str, output: Option<&Path>) -> Result<Vec<u8>> {
    let mut command = Command::new("curl");
    command.args([
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--retry",
        "2",
    ]);
    if let Some(output) = output {
        command.arg("--output").arg(output);
    }
    let result = command
        .arg(url)
        .output()
        .context(|| format!("running curl for {url} (is curl installed?)"))?;
    if !result.status.success() {
        return Err(fail!(
            "downloading {url} failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    Ok(result.stdout)
}

impl Registry {
    /// The usual locations: `$CARGO_HOME` (or `~/.cargo`) and
    /// `<root>/target/mercury-servo-patches/crates`. The network is off when
    /// `CARGO_NET_OFFLINE=true` or `MERCURY_SERVO_PATCHES_OFFLINE` is set.
    pub fn from_env(root: &Path) -> Registry {
        let cargo_home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")));
        let offline = std::env::var("CARGO_NET_OFFLINE").is_ok_and(|value| value == "true")
            || std::env::var_os("MERCURY_SERVO_PATCHES_OFFLINE").is_some();
        Registry {
            cargo_home,
            cache_dir: root.join("target/mercury-servo-patches/crates"),
            network: !offline,
        }
    }

    fn cached(&self, name: &str, version: &str) -> Vec<PathBuf> {
        let file = file_name(name, version);
        let mut found = Vec::new();
        if let Some(home) = &self.cargo_home
            && let Ok(entries) = fs::read_dir(home.join("registry/cache"))
        {
            let mut dirs: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
            dirs.sort();
            found.extend(
                dirs.into_iter()
                    .map(|dir| dir.join(&file))
                    .filter(|p| p.is_file()),
            );
        }
        let own = self.cache_dir.join(&file);
        if own.is_file() {
            found.push(own);
        }
        found
    }

    /// Whether some cache holds an archive for `name` `version` (unchecked).
    pub fn is_cached(&self, name: &str, version: &str) -> bool {
        !self.cached(name, version).is_empty()
    }

    /// The `.crate` archive of `name` `version`, whose SHA-256 must be `checksum`.
    pub fn archive(&self, name: &str, version: &str, checksum: &str) -> Result<Vec<u8>> {
        let what = format!("{name} {version}");
        let mut mismatched = Vec::new();
        for path in self.cached(name, version) {
            let bytes = fs::read(&path).context(|| format!("reading {}", path.display()))?;
            if sha256_hex(&bytes) == checksum {
                return Ok(bytes);
            }
            mismatched.push(path.display().to_string());
        }
        if !mismatched.is_empty() && !self.network {
            return Err(fail!(
                "{what}: the cached archive does not have the expected SHA-256 {checksum}: {}",
                mismatched.join(", ")
            ));
        }
        if !self.network {
            return Err(fail!(
                "{what}: no archive in Cargo's cache or {}, and the network is off",
                self.cache_dir.display()
            ));
        }
        fs::create_dir_all(&self.cache_dir)
            .context(|| format!("creating {}", self.cache_dir.display()))?;
        let file = file_name(name, version);
        let partial = self.cache_dir.join(format!("{file}.partial"));
        let url = format!("https://static.crates.io/crates/{name}/{file}");
        curl(&url, Some(&partial))?;
        let bytes = fs::read(&partial).context(|| format!("reading {}", partial.display()))?;
        if let Err(error) = verify(&bytes, checksum, &what) {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
        let target = self.cache_dir.join(&file);
        fs::rename(&partial, &target).context(|| format!("writing {}", target.display()))?;
        Ok(bytes)
    }

    /// The checksum crates.io publishes for `name` `version`.
    pub fn index_checksum(&self, name: &str, version: &str) -> Result<String> {
        if !self.network {
            return Err(fail!(
                "{name} {version}: the checksum comes from the crates.io index, and the network is off"
            ));
        }
        let url = format!("https://index.crates.io/{}", index_path(name));
        let index = String::from_utf8(curl(&url, None)?)
            .map_err(|_| fail!("the crates.io index for {name} is not UTF-8"))?;
        checksum_in_index(&index, name, version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_index_paths_follow_the_crates_io_layout() {
        assert_eq!(index_path("a"), "1/a");
        assert_eq!(index_path("ab"), "2/ab");
        assert_eq!(index_path("abc"), "3/a/abc");
        assert_eq!(index_path("gaol"), "ga/ol/gaol");
        assert_eq!(index_path("Servo-Canvas"), "se/rv/servo-canvas");
    }

    #[test]
    fn index_lines_give_the_checksum_of_the_version() {
        let index = "{\"name\":\"gaol\",\"vers\":\"0.2.0\",\"cksum\":\"aa\"}\n{\"name\":\"gaol\",\"vers\":\"0.2.1\",\"cksum\":\"bb\"}\n";
        assert_eq!(checksum_in_index(index, "gaol", "0.2.1").unwrap(), "bb");
        assert!(
            checksum_in_index(index, "gaol", "9.9.9")
                .unwrap_err()
                .0
                .contains("not published")
        );
    }

    #[test]
    fn archives_come_from_a_cache_and_must_match_the_checksum() {
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join("registry/cache/index.crates.io-0000");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("demo-1.0.0.crate"), b"archive").unwrap();
        let registry = Registry {
            cargo_home: Some(home.path().to_owned()),
            cache_dir: home.path().join("own"),
            network: false,
        };
        assert!(registry.is_cached("demo", "1.0.0"));
        assert_eq!(
            registry
                .archive("demo", "1.0.0", &sha256_hex(b"archive"))
                .unwrap(),
            b"archive"
        );
        let error = registry
            .archive("demo", "1.0.0", &"0".repeat(64))
            .unwrap_err();
        assert!(error.0.contains("expected SHA-256"), "{error}");
        let error = registry
            .archive("other", "1.0.0", &"0".repeat(64))
            .unwrap_err();
        assert!(error.0.contains("network is off"), "{error}");
        assert!(registry.index_checksum("demo", "1.0.0").is_err());
    }
}
