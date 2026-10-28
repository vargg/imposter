//! The dependency rules from AGENTS.md, checked against every crate manifest.
#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Normal dependencies each internal crate may have on other internal crates.
const ALLOWED: &[(&str, &[&str])] = &[
    ("imposter-core", &[]),
    ("imposter-http", &["imposter-core"]),
    ("imposter-store", &["imposter-core"]),
    ("imposter-transfer", &["imposter-core"]),
    (
        "imposter-app",
        &[
            "imposter-core",
            "imposter-http",
            "imposter-store",
            "imposter-transfer",
        ],
    ),
    ("imposter-ui", &["imposter-core", "imposter-app"]),
    (
        "imposter",
        &["imposter-core", "imposter-app", "imposter-ui"],
    ),
    ("imposter-testkit", &[]),
];

#[derive(Debug, Default)]
struct Manifest {
    normal: BTreeSet<String>,
    dev: BTreeSet<String>,
    lints_from_workspace: bool,
    /// Dependencies that set their own version instead of inheriting the workspace one.
    own_versions: Vec<String>,
}

/// Enough TOML for our manifests: section headers and `name = …` / `name.workspace = …` keys.
fn read(path: &Path) -> Manifest {
    let text = std::fs::read_to_string(path).unwrap();
    let mut manifest = Manifest::default();
    let mut section = String::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line.trim_matches(['[', ']']).to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let name = key.split('.').next().unwrap().trim().to_owned();
        let inherits = key.ends_with(".workspace") || value.contains("workspace = true");
        if section.ends_with("dependencies") && !inherits {
            manifest.own_versions.push(name.clone());
        }
        match section.as_str() {
            "dependencies" | "build-dependencies" => {
                manifest.normal.insert(name);
            }
            "dev-dependencies" => {
                manifest.dev.insert(name);
            }
            "lints" if key == "workspace" && value.trim() == "true" => {
                manifest.lints_from_workspace = true;
            }
            _ => {}
        }
    }
    manifest
}

fn manifests() -> BTreeMap<String, Manifest> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    std::fs::read_dir(&crates)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|dir| dir.join("Cargo.toml").exists())
        .map(|dir| {
            (
                dir.file_name().unwrap().to_string_lossy().into_owned(),
                read(&dir.join("Cargo.toml")),
            )
        })
        .collect()
}

#[test]
fn every_crate_is_known_and_uses_workspace_lints() {
    let known: BTreeSet<&str> = ALLOWED.iter().map(|(name, _)| *name).collect();
    for (name, manifest) in manifests() {
        assert!(
            known.contains(name.as_str()),
            "{name} is not in the dependency rules; add it to ALLOWED"
        );
        assert!(
            manifest.lints_from_workspace,
            "{name} must set `[lints] workspace = true`"
        );
    }
}

#[test]
fn internal_dependencies_follow_the_allowed_direction() {
    let allowed: BTreeMap<&str, &[&str]> = ALLOWED.iter().copied().collect();
    for (name, manifest) in manifests() {
        let internal: BTreeSet<&str> = manifest
            .normal
            .iter()
            .map(String::as_str)
            .filter(|d| d.starts_with("imposter"))
            .collect();
        let permitted: BTreeSet<&str> = allowed[name.as_str()].iter().copied().collect();
        let extra: Vec<_> = internal.difference(&permitted).collect();
        assert!(
            extra.is_empty(),
            "{name} must not depend on {extra:?} (see AGENTS.md)"
        );
    }
}

#[test]
fn dependency_versions_are_declared_once_in_the_workspace() {
    for (name, manifest) in manifests() {
        assert!(
            manifest.own_versions.is_empty(),
            "{name} sets its own versions for {:?}; declare them in the root Cargo.toml",
            manifest.own_versions
        );
    }
}

#[test]
fn test_support_and_anyhow_stay_where_they_belong() {
    for (name, manifest) in manifests() {
        assert!(
            !manifest.normal.contains("imposter-testkit"),
            "{name}: imposter-testkit is a dev-dependency only"
        );
        if name != "imposter" {
            assert!(
                !manifest.normal.contains("anyhow"),
                "{name}: anyhow belongs only to the binary; use thiserror"
            );
        }
    }
}
