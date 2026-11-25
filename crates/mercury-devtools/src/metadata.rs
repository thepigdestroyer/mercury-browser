// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Reads `cargo metadata --format-version 1` output: the resolved `servo` crate's directory for
//! the macOS scripts where `jq` is not installed (macOS before 15), and the workspace members
//! the fast CI jobs can build without the browser engine (`scripts/ci/light-crates.sh`).

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::Value;

/// The crates that make a workspace member heavy when anything in its closure depends on them.
/// `gstreamer` is one because the fast CI jobs install no GStreamer: a member that links it (the
/// MSE source element `mercury-mse-gst`) cannot be built or tested there, and its tests need the
/// bundled plugins, so it belongs to the jobs that have them.
pub const HEAVY: [&str; 3] = ["servo", "slint", "gstreamer"];

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

/// A workspace member and the [`HEAVY`] crates in its full dependency closure (normal, build
/// and dev dependencies, for every target platform), sorted; empty when it is light.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub name: String,
    pub heavy: Vec<String>,
}

impl Verdict {
    /// The `--explain` line: the name padded to 24 columns, then `light` or why not.
    pub fn explain(&self) -> String {
        let verdict = if self.heavy.is_empty() {
            "light".to_owned()
        } else {
            format!("heavy (depends on {})", self.heavy.join(" and "))
        };
        format!("{:<24} {verdict}", self.name)
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, String> {
    value
        .get(name)
        .ok_or_else(|| format!("not cargo metadata: no \"{name}\""))
}

fn text<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    field(value, name)?
        .as_str()
        .ok_or_else(|| format!("not cargo metadata: \"{name}\" is not a string"))
}

fn list<'a>(value: &'a Value, name: &str) -> Result<&'a Vec<Value>, String> {
    field(value, name)?
        .as_array()
        .ok_or_else(|| format!("not cargo metadata: \"{name}\" is not a list"))
}

/// Every workspace member, sorted by name, with the heavy crates it pulls in. Needs the
/// `resolve` graph, so the metadata must come from `cargo metadata` without `--no-deps`.
pub fn workspace_verdicts(metadata: &str) -> Result<Vec<Verdict>, String> {
    let metadata: Value =
        serde_json::from_str(metadata).map_err(|error| format!("not cargo metadata: {error}"))?;
    let mut names: HashMap<&str, &str> = HashMap::new();
    for package in list(&metadata, "packages")? {
        names.insert(text(package, "id")?, text(package, "name")?);
    }
    let resolve = field(&metadata, "resolve")?;
    if resolve.is_null() {
        return Err("cargo metadata has no dependency graph (run it without --no-deps)".into());
    }
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for node in list(resolve, "nodes")? {
        let children = list(node, "deps")?
            .iter()
            .map(|dependency| text(dependency, "pkg"))
            .collect::<Result<_, _>>()?;
        edges.insert(text(node, "id")?, children);
    }
    let name_of = |id: &str| {
        names
            .get(id)
            .copied()
            .ok_or_else(|| format!("not cargo metadata: unknown package id {id}"))
    };

    let mut verdicts = Vec::new();
    for member in list(&metadata, "workspace_members")? {
        let member = member
            .as_str()
            .ok_or("not cargo metadata: a workspace member is not a string")?;
        let mut heavy = BTreeSet::new();
        let mut seen = HashSet::from([member]);
        let mut stack = vec![member];
        while let Some(id) = stack.pop() {
            for &child in edges.get(id).map(Vec::as_slice).unwrap_or_default() {
                if seen.insert(child) {
                    stack.push(child);
                    let name = name_of(child)?;
                    if HEAVY.contains(&name) {
                        heavy.insert(name.to_owned());
                    }
                }
            }
        }
        verdicts.push(Verdict {
            name: name_of(member)?.to_owned(),
            heavy: heavy.into_iter().collect(),
        });
    }
    verdicts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(verdicts)
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

    /// Members `app` (through `ui` on slint and, through a dev-dependency, on servo), `ui`
    /// (slint), `media` (gstreamer, through `gstreamer-app`), `core` and `tools` (light; `tools`
    /// depends on `core`), and an unrelated cycle.
    const GRAPH: &str = r#"{
        "packages": [
            {"id": "app 0.1", "name": "app"}, {"id": "ui 0.1", "name": "ui"},
            {"id": "core 0.1", "name": "core"}, {"id": "tools 0.1", "name": "tools"},
            {"id": "slint 1.17", "name": "slint"}, {"id": "servo 0.8", "name": "servo"},
            {"id": "servo-media 0.8", "name": "servo-media"},
            {"id": "media 0.1", "name": "media"}, {"id": "gstreamer-app 0.25", "name": "gstreamer-app"},
            {"id": "gstreamer 0.25", "name": "gstreamer"},
            {"id": "a 1", "name": "a"}, {"id": "b 1", "name": "b"}
        ],
        "workspace_members": ["ui 0.1", "app 0.1", "tools 0.1", "core 0.1", "media 0.1"],
        "resolve": {"nodes": [
            {"id": "app 0.1", "deps": [{"pkg": "ui 0.1"}, {"pkg": "a 1"}]},
            {"id": "ui 0.1", "deps": [{"pkg": "slint 1.17"}]},
            {"id": "a 1", "deps": [{"pkg": "b 1"}]},
            {"id": "b 1", "deps": [{"pkg": "a 1"}, {"pkg": "servo 0.8"}]},
            {"id": "tools 0.1", "deps": [{"pkg": "core 0.1"}, {"pkg": "servo-media 0.8"}]},
            {"id": "core 0.1", "deps": []},
            {"id": "media 0.1", "deps": [{"pkg": "core 0.1"}, {"pkg": "gstreamer-app 0.25"}]},
            {"id": "gstreamer-app 0.25", "deps": [{"pkg": "gstreamer 0.25"}]},
            {"id": "gstreamer 0.25", "deps": []},
            {"id": "slint 1.17", "deps": []}, {"id": "servo 0.8", "deps": []},
            {"id": "servo-media 0.8", "deps": []}
        ]}
    }"#;

    #[test]
    fn sorts_members_into_light_and_heavy() {
        let verdicts = workspace_verdicts(GRAPH).expect("reads");
        let lines: Vec<String> = verdicts.iter().map(Verdict::explain).collect();
        assert_eq!(
            lines,
            [
                "app                      heavy (depends on servo and slint)",
                "core                     light",
                "media                    heavy (depends on gstreamer)",
                "tools                    light",
                "ui                       heavy (depends on slint)",
            ]
        );
    }

    #[test]
    fn needs_the_dependency_graph() {
        assert!(workspace_verdicts("[]").is_err());
        assert!(workspace_verdicts(r#"{"packages":[],"workspace_members":[]}"#).is_err());
        assert_eq!(
            workspace_verdicts(r#"{"packages":[],"workspace_members":[],"resolve":null}"#),
            Err("cargo metadata has no dependency graph (run it without --no-deps)".into())
        );
        assert!(
            workspace_verdicts(
                r#"{"packages":[],"workspace_members":["x"],"resolve":{"nodes":[]}}"#
            )
            .is_err()
        );
    }
}
