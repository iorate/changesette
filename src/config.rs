use std::{collections::BTreeSet, path::Path};

use anyhow::{Context, Result, bail};
use fast_glob::{glob_match, validate};
use serde_json::{Map, Value};
use tracing::warn;

use crate::workspace::read_json;

#[derive(Debug, Default)]
pub struct Config {
    ignore: Vec<String>,
    fixed: Vec<Vec<String>>,
    linked: Vec<Vec<String>>,
    pub private_packages_version: bool,
    pub snapshot_use_calculated_version: bool,
    pub snapshot_prerelease_template: Option<String>,
    pub packages: Option<Vec<String>>,
}

impl Config {
    #[must_use]
    pub fn has_ignore(&self) -> bool {
        !self.ignore.is_empty()
    }

    pub fn resolve_ignore<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        expand_patterns(&self.ignore, names)
    }

    pub fn resolve_groups(&self, names: &[&str]) -> Result<ResolvedGroups> {
        let expand = |groups: &[Vec<String>]| -> Vec<Vec<String>> {
            groups
                .iter()
                .map(|patterns| expand_patterns(patterns, names.iter().copied()))
                .collect()
        };
        let fixed = expand(&self.fixed);
        let linked = expand(&self.linked);

        check_group_duplicates("fixed", &fixed)?;
        check_group_duplicates("linked", &linked)?;
        let fixed_names: BTreeSet<&String> = fixed.iter().flatten().collect();
        for name in linked.iter().flatten() {
            if fixed_names.contains(name) {
                bail!(
                    "package `{name}` is in both a \"fixed\" and a \"linked\" group; a package can be in only one of them"
                );
            }
        }

        for (key, groups) in [("fixed", &self.fixed), ("linked", &self.linked)] {
            for pattern in groups.iter().flatten() {
                // Each pattern is judged alone, unlike the ordered expansion
                // above, so it is handed to the matcher as written, its
                // leading `!` included.
                if !names.iter().any(|name| glob_match(pattern, *name)) {
                    warn!(
                        "{key}: the package or glob {pattern:?} does not match any package in the workspace"
                    );
                }
            }
        }

        Ok(ResolvedGroups { fixed, linked })
    }
}

pub struct ResolvedGroups {
    pub fixed: Vec<Vec<String>>,
    pub linked: Vec<Vec<String>>,
}

fn expand_patterns<'a>(
    patterns: &[String],
    names: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut resolved = Vec::new();
    for name in names {
        let mut matched = false;
        for pattern in patterns {
            // A negated pattern un-matches what its body matches, which the
            // negation of fast-glob cannot express, so the leading `!`s are
            // split off instead of letting fast-glob apply them.
            let (negated, body) = split_negation(pattern);
            if glob_match(body, name) {
                matched = !negated;
            }
        }
        if matched {
            resolved.push(name.to_owned());
        }
    }
    resolved
}

fn split_negation(pattern: &str) -> (bool, &str) {
    let body = pattern.trim_start_matches('!');
    ((pattern.len() - body.len()) % 2 == 1, body)
}

// Each name appears at most once per expanded group, so a duplicate can only
// come from another group.
fn check_group_duplicates(key: &str, groups: &[Vec<String>]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for name in groups.iter().flatten() {
        if !seen.insert(name) {
            bail!(
                "package `{name}` is in multiple \"{key}\" groups; a package can belong to only one group"
            );
        }
    }
    Ok(())
}

pub fn load(changeset_dir: &Path) -> Result<Config> {
    let path = changeset_dir.join("config.json");
    let Some(value) = read_json(&path)? else {
        return Ok(Config::default());
    };
    load_value(&value).with_context(|| path.display().to_string())
}

fn load_value(value: &Value) -> Result<Config> {
    let Some(object) = value.as_object() else {
        bail!("the root value must be an object")
    };

    let mut ignore = Vec::new();
    if let Some(value) = object.get("ignore") {
        let patterns = value.as_array().and_then(|items| {
            items
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<&str>>>()
        });
        let Some(patterns) = patterns else {
            bail!("\"ignore\" must be an array of strings")
        };
        for pattern in patterns {
            validate(pattern).with_context(|| format!("invalid ignore pattern {pattern:?}"))?;
            ignore.push(pattern.to_owned());
        }
    }

    let mut groups = [Vec::new(), Vec::new()];
    for (key, parsed) in ["fixed", "linked"].into_iter().zip(&mut groups) {
        let Some(value) = object.get(key) else {
            continue;
        };
        let raw_groups = value.as_array().and_then(|groups| {
            groups
                .iter()
                .map(|group| {
                    group.as_array().and_then(|items| {
                        items
                            .iter()
                            .map(Value::as_str)
                            .collect::<Option<Vec<&str>>>()
                    })
                })
                .collect::<Option<Vec<Vec<&str>>>>()
        });
        let Some(raw_groups) = raw_groups else {
            bail!("\"{key}\" must be an array of arrays of strings")
        };
        for (index, raw_group) in raw_groups.into_iter().enumerate() {
            let mut group = Vec::new();
            for pattern in raw_group {
                validate(pattern).with_context(|| {
                    format!("invalid pattern {pattern:?} in \"{key}\"[{index}]")
                })?;
                group.push(pattern.to_owned());
            }
            parsed.push(group);
        }
    }
    let [fixed, linked] = groups;

    let private_packages_version = match object.get("privatePackages") {
        None => false,
        Some(Value::Bool(version)) => *version,
        Some(Value::Object(object)) => match object.get("version") {
            None => false,
            Some(Value::Bool(version)) => *version,
            Some(_) => bail!("\"version\" in \"privatePackages\" must be a boolean"),
        },
        Some(_) => bail!("\"privatePackages\" must be a boolean or an object"),
    };

    let mut snapshot_use_calculated_version = false;
    let mut snapshot_prerelease_template = None;
    match object.get("snapshot") {
        None => {}
        Some(Value::Object(snapshot)) => {
            match snapshot.get("useCalculatedVersion") {
                None => {}
                Some(Value::Bool(use_calculated_version)) => {
                    snapshot_use_calculated_version = *use_calculated_version;
                }
                Some(_) => bail!("\"useCalculatedVersion\" in \"snapshot\" must be a boolean"),
            }
            match snapshot.get("prereleaseTemplate") {
                None => {}
                Some(Value::String(template)) if !template.is_empty() => {
                    snapshot_prerelease_template = Some(template.clone());
                }
                Some(_) => {
                    bail!("\"prereleaseTemplate\" in \"snapshot\" must be a non-empty string")
                }
            }
        }
        Some(_) => bail!("\"snapshot\" must be an object"),
    }

    let packages = load_packages(object)?;

    Ok(Config {
        ignore,
        fixed,
        linked,
        private_packages_version,
        snapshot_use_calculated_version,
        snapshot_prerelease_template,
        packages,
    })
}

fn load_packages(object: &Map<String, Value>) -> Result<Option<Vec<String>>> {
    let changesette = match object.get("changesette") {
        None => return Ok(None),
        Some(Value::Object(changesette)) => changesette,
        Some(_) => bail!("\"changesette\" must be an object"),
    };
    let Some(value) = changesette.get("packages") else {
        return Ok(None);
    };
    let dirs = value.as_array().and_then(|items| {
        items
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<&str>>>()
    });
    let Some(dirs) = dirs else {
        bail!("\"packages\" in \"changesette\" must be an array of strings")
    };
    let mut packages = Vec::new();
    for dir in dirs {
        validate_rel_dir(dir)
            .with_context(|| format!("invalid \"changesette.packages\" entry {dir:?}"))?;
        packages.push(dir.to_owned());
    }
    Ok(Some(packages))
}

fn validate_rel_dir(text: &str) -> Result<()> {
    if text.is_empty() {
        bail!("an empty path is not supported")
    }
    #[cfg(windows)]
    if text.contains('\\') {
        bail!("`\\` is not supported; use `/` as the separator")
    }
    if text.starts_with('/') {
        bail!("absolute paths are not supported")
    }
    Ok(())
}
