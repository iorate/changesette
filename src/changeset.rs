use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::{Context, Result, bail};
use regex::Regex;
use saphyr::{
    LoadableYamlNode, Mapping, MappingOwned, Scalar, ScalarOwned, Yaml, YamlEmitter, YamlOwned,
};
use serde_json::{Map, Number, Value};

use crate::bump::Bump;

const IGNORED_FILE_NAMES: [&str; 3] = ["AGENTS.md", "CLAUDE.md", "GEMINI.md"];

static FRONTMATTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\s*---(.*?)\n\s*---(\s*(?:\n|$).*)").unwrap());

#[derive(Clone, Debug)]
pub struct LoadedChange {
    pub file_name: String,
    pub in_pre: bool,
    pub releases: Vec<(String, Option<Bump>)>,
    pub summary: String,
    pub metadata: Option<Metadata>,
}

#[derive(Clone, Debug)]
pub struct Metadata {
    yaml: YamlOwned,
    json: Map<String, Value>,
}

impl Metadata {
    #[must_use]
    pub fn json(&self) -> &Map<String, Value> {
        &self.json
    }
}

impl LoadedChange {
    #[must_use]
    pub fn id(&self) -> String {
        let stem = self
            .file_name
            .strip_suffix(".md")
            .unwrap_or(&self.file_name);
        if self.in_pre {
            format!("pre/{stem}")
        } else {
            stem.to_owned()
        }
    }

    #[must_use]
    pub fn rel_path(&self) -> PathBuf {
        if self.in_pre {
            Path::new("pre").join(&self.file_name)
        } else {
            PathBuf::from(self.file_name.clone())
        }
    }
}

pub fn load(changeset_dir: &Path) -> Result<Vec<LoadedChange>> {
    let file_names = scan(changeset_dir)?.unwrap_or_default();
    let pre_dir = changeset_dir.join("pre");
    let pre_file_names = scan(&pre_dir)?.unwrap_or_default();

    file_names
        .iter()
        .map(|file_name| load_one(changeset_dir, file_name, false))
        .chain(
            pre_file_names
                .iter()
                .map(|file_name| load_one(&pre_dir, file_name, true)),
        )
        .collect()
}

fn scan(dir: &Path) -> Result<Option<Vec<String>>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context(dir.display().to_string()),
    };

    let mut file_names = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| dir.display().to_string())?;
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        // Selecting entries by name alone means a symlink is followed and a
        // directory with an adopted name is a read error.
        #[expect(clippy::case_sensitive_file_extension_comparisons)]
        if file_name.starts_with('.')
            || !file_name.ends_with(".md")
            || file_name.eq_ignore_ascii_case("README.md")
            || IGNORED_FILE_NAMES.contains(&file_name.as_str())
        {
            continue;
        }
        file_names.push(file_name);
    }
    file_names.sort();
    Ok(Some(file_names))
}

#[must_use]
pub fn max_bumps(changes: &[LoadedChange]) -> BTreeMap<&str, Option<Bump>> {
    let mut bumps = BTreeMap::new();
    for change in changes {
        for (name, bump) in &change.releases {
            let entry = bumps.entry(name.as_str()).or_insert(None);
            *entry = (*entry).max(*bump);
        }
    }
    bumps
}

pub fn render(
    releases: &[(String, Option<Bump>)],
    metadata: Option<&Metadata>,
    summary: &str,
) -> Result<String> {
    let summary = summary.trim();
    let mut content = if releases.is_empty() && metadata.is_none() {
        String::from("---\n---\n")
    } else {
        let mut mapping = Mapping::new();
        for (name, bump) in releases {
            mapping.insert(
                Yaml::Value(Scalar::String(name.as_str().into())),
                Yaml::Value(Scalar::String(bump.map_or("none", Bump::as_str).into())),
            );
        }
        if let Some(metadata) = metadata {
            mapping.insert(
                Yaml::Value(Scalar::String("_".into())),
                Yaml::from(&metadata.yaml),
            );
        }
        let mut frontmatter = String::new();
        YamlEmitter::new(&mut frontmatter).dump(&Yaml::Mapping(mapping))?;
        format!("{frontmatter}\n---\n")
    };
    if !summary.is_empty() {
        content.push('\n');
        content.push_str(summary);
        content.push('\n');
    }
    Ok(content)
}

fn load_one(dir: &Path, file_name: &str, in_pre: bool) -> Result<LoadedChange> {
    let file_path = dir.join(file_name);

    let content = fs::read_to_string(&file_path)
        .with_context(|| file_path.display().to_string())?
        .replace("\r\n", "\n");
    let Some(captures) = FRONTMATTER.captures(&content) else {
        bail!(
            "{}: missing frontmatter (expected `---`-delimited YAML)",
            file_path.display()
        )
    };
    let frontmatter = &captures[1];
    let summary = captures[2].trim();

    let docs = match YamlOwned::load_from_str(frontmatter) {
        Ok(docs) => docs,
        Err(err) => bail!(
            "{}: invalid YAML in frontmatter: {err}",
            file_path.display()
        ),
    };

    let mut releases = Vec::new();
    let mut metadata = None;
    match docs.into_iter().next() {
        None => {}
        Some(doc) if doc.is_null() => {}
        Some(YamlOwned::Mapping(mapping)) => {
            for (key, value) in mapping {
                let Some(name) = key.as_str() else {
                    bail!(
                        "{}: invalid package name in frontmatter",
                        file_path.display()
                    )
                };
                if name == "_" {
                    let YamlOwned::Mapping(entries) = &value else {
                        bail!(
                            "{}: `_` in frontmatter must be a mapping",
                            file_path.display()
                        )
                    };
                    let json = match mapping_to_json(entries) {
                        Ok(json) => json,
                        Err(reason) => bail!(
                            "{}: metadata in frontmatter contains {reason}",
                            file_path.display()
                        ),
                    };
                    metadata = Some(Metadata { yaml: value, json });
                    continue;
                }
                let bump = match value.as_str() {
                    Some("major") => Some(Bump::Major),
                    Some("minor") => Some(Bump::Minor),
                    Some("patch") => Some(Bump::Patch),
                    Some("none") => None,
                    Some(other) => bail!(
                        "{}: unknown bump type {other:?}; expected major, minor, patch, or none",
                        file_path.display()
                    ),
                    None => bail!(
                        "{}: invalid bump type; expected major, minor, patch, or none",
                        file_path.display()
                    ),
                };
                releases.push((name.to_owned(), bump));
            }
        }
        Some(_) => bail!(
            "{}: frontmatter must be a mapping of package names to bump types",
            file_path.display()
        ),
    }

    Ok(LoadedChange {
        file_name: file_name.to_owned(),
        in_pre,
        releases,
        summary: summary.to_owned(),
        metadata,
    })
}

fn mapping_to_json(mapping: &MappingOwned) -> Result<Map<String, Value>, &'static str> {
    let mut json = Map::new();
    for (key, value) in mapping {
        let Some(key) = key.as_str() else {
            return Err("a non-string key");
        };
        json.insert(key.to_owned(), to_json(value)?);
    }
    Ok(json)
}

fn to_json(yaml: &YamlOwned) -> Result<Value, &'static str> {
    match yaml {
        YamlOwned::Value(ScalarOwned::Null) => Ok(Value::Null),
        YamlOwned::Value(ScalarOwned::Boolean(value)) => Ok(Value::Bool(*value)),
        YamlOwned::Value(ScalarOwned::Integer(value)) => Ok(Value::from(*value)),
        YamlOwned::Value(ScalarOwned::FloatingPoint(value)) => Number::from_f64(value.into_inner())
            .map(Value::Number)
            .ok_or("a non-finite number"),
        YamlOwned::Value(ScalarOwned::String(value)) => Ok(Value::String(value.clone())),
        YamlOwned::Sequence(sequence) => sequence.iter().map(to_json).collect(),
        YamlOwned::Mapping(mapping) => mapping_to_json(mapping).map(Value::Object),
        _ => Err("a tagged or invalid value"),
    }
}
