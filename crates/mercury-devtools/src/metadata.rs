// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Reads `cargo metadata --format-version 1` output, for the macOS scripts that need the
//! resolved `servo` crate's directory where `jq` is not installed (macOS before 15).

use serde_json::Value;

/// The `manifest_path` of every package called `name`, in the order cargo lists them.
pub fn manifest_paths(metadata: &str, name: &str) -> Result<Vec<String>, String> {
    let metadata: Value =
        serde_json::from_str(metadata).map_err(|error| format!("not cargo metadata: {error}"))?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("not cargo metadata: no \"packages\" list")?;
    Ok(packages
        .iter()
        .filter(|package| package.get("name").and_then(Value::as_str) == Some(name))
        .filter_map(|package| package.get("manifest_path").and_then(Value::as_str))
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_packages_by_name_only() {
        let metadata = r#"{"packages":[
            {"name":"servo-media","manifest_path":"/r/servo-media-0.8.0/Cargo.toml","dependencies":[]},
            {"name":"mercury-desktop","manifest_path":"/w/apps/mercury-desktop/Cargo.toml",
             "dependencies":[{"name":"servo","req":"=0.8.0","source":null}]},
            {"name":"servo","manifest_path":"/r/servo-0.8.0/Cargo.toml","license":null}
        ],"resolve":null}"#;
        assert_eq!(
            manifest_paths(metadata, "servo"),
            Ok(vec!["/r/servo-0.8.0/Cargo.toml".to_owned()])
        );
        assert_eq!(manifest_paths(metadata, "nothing"), Ok(vec![]));
    }

    #[test]
    fn refuses_other_input() {
        assert!(manifest_paths("", "servo").is_err());
        assert!(manifest_paths("{}", "servo").is_err());
    }
}
