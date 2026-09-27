use std::{
    collections::BTreeMap,
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use saphyr::{LoadableYamlNode, MarkedYaml, Scalar, Yaml, YamlData, YamlEmitter};
use serde_json::Value;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogFormat {
    PnpmWorkspace,
    Yarnrc,
    PackageJson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub spec: String,
    pub path: Vec<String>,
}

#[derive(Debug)]
pub struct CatalogSource {
    pub path: PathBuf,
    pub format: CatalogFormat,
}

#[derive(Debug, Default)]
pub struct Catalogs {
    source: Option<CatalogSource>,
    entries: BTreeMap<String, BTreeMap<String, CatalogEntry>>,
}

#[must_use]
pub fn reference(spec: &str) -> Option<&str> {
    spec.strip_prefix("catalog:")
}

#[must_use]
pub fn describe(name: &str) -> String {
    if name.is_empty() {
        "the default catalog".to_owned()
    } else {
        format!("catalog {name:?}")
    }
}

impl Catalogs {
    #[must_use]
    pub fn from_yaml(path: PathBuf, doc: &Yaml, format: CatalogFormat) -> Catalogs {
        let mut catalogs = Catalogs {
            source: Some(CatalogSource { path, format }),
            entries: BTreeMap::new(),
        };
        // The unnamed catalog of a yarnrc is a catalog of its own, not
        // `catalogs.default`.
        let unnamed = match format {
            CatalogFormat::Yarnrc => "",
            CatalogFormat::PnpmWorkspace | CatalogFormat::PackageJson => "default",
        };
        catalogs.collect(doc, &[], unnamed);
        catalogs
    }

    #[must_use]
    pub fn from_json(path: PathBuf, value: &Value) -> Catalogs {
        let mut catalogs = Catalogs {
            source: Some(CatalogSource {
                path,
                format: CatalogFormat::PackageJson,
            }),
            entries: BTreeMap::new(),
        };
        // The top-level keys count only for a workspace root whose
        // `workspaces` object declares no catalog of its own.
        if let Some(workspaces) = value.get("workspaces") {
            if workspaces.get("catalog").is_some() || workspaces.get("catalogs").is_some() {
                catalogs.collect(workspaces, &["workspaces"], "default");
            } else {
                catalogs.collect(value, &[], "default");
            }
        }
        catalogs
    }

    #[must_use]
    pub fn source(&self) -> Option<&CatalogSource> {
        self.source.as_ref()
    }

    #[must_use]
    pub fn lookup(&self, reference: &str, name: &str) -> Option<(&str, &CatalogEntry)> {
        let key = match self.source.as_ref()?.format {
            CatalogFormat::PnpmWorkspace => match reference.trim() {
                "" => "default",
                trimmed => trimmed,
            },
            CatalogFormat::PackageJson if reference.is_empty() => "default",
            CatalogFormat::PackageJson | CatalogFormat::Yarnrc => reference,
        };
        let (key, catalog) = self.entries.get_key_value(key)?;
        Some((key, catalog.get(name)?))
    }

    fn collect<N: Node>(&mut self, root: &N, prefix: &[&str], unnamed: &str) {
        let key_path = |keys: &[&str]| -> Vec<String> {
            prefix
                .iter()
                .chain(keys)
                .map(|key| (*key).to_owned())
                .collect()
        };
        if let Some(node) = root.get("catalog") {
            self.collect_catalog(node, unnamed, &key_path(&["catalog"]));
        }
        if let Some(node) = root.get("catalogs") {
            let Some(catalogs) = node.entries() else {
                self.warn_shape(&key_path(&["catalogs"]), "a mapping");
                return;
            };
            for (name, node) in catalogs {
                self.collect_catalog(node, name, &key_path(&["catalogs", name]));
            }
        }
    }

    fn collect_catalog<N: Node>(&mut self, node: &N, name: &str, key_path: &[String]) {
        let Some(entries) = node.entries() else {
            self.warn_shape(key_path, "a mapping");
            return;
        };
        for (dep, value) in entries {
            let mut path = key_path.to_vec();
            path.push(dep.to_owned());
            let Some(spec) = value.as_str() else {
                self.warn_shape(&path, "a string");
                continue;
            };
            // `catalog` is collected before `catalogs.default`, so it wins
            // the tie that the package managers reject.
            self.entries
                .entry(name.to_owned())
                .or_default()
                .entry(dep.to_owned())
                .or_insert(CatalogEntry {
                    spec: spec.to_owned(),
                    path,
                });
        }
    }

    fn warn_shape(&self, path: &[String], expected: &str) {
        warn!(
            "{}: {:?} is not {expected}: ignored",
            self.file(),
            path.join(".")
        );
    }

    fn file(&self) -> String {
        self.source
            .as_ref()
            .map(|source| source.path.display().to_string())
            .unwrap_or_default()
    }
}

trait Node {
    fn get(&self, key: &str) -> Option<&Self>;
    fn entries(&self) -> Option<Vec<(&str, &Self)>>;
    fn as_str(&self) -> Option<&str>;
}

impl Node for Yaml<'_> {
    fn get(&self, key: &str) -> Option<&Self> {
        self.as_mapping_get(key)
    }

    fn entries(&self) -> Option<Vec<(&str, &Self)>> {
        Some(
            self.as_mapping()?
                .iter()
                .filter_map(|(key, value)| Some((key.as_str()?, value)))
                .collect(),
        )
    }

    fn as_str(&self) -> Option<&str> {
        Yaml::as_str(self)
    }
}

impl Node for Value {
    fn get(&self, key: &str) -> Option<&Self> {
        Value::get(self, key)
    }

    fn entries(&self) -> Option<Vec<(&str, &Self)>> {
        Some(
            self.as_object()?
                .iter()
                .map(|(key, value)| (key.as_str(), value))
                .collect(),
        )
    }

    fn as_str(&self) -> Option<&str> {
        Value::as_str(self)
    }
}

pub struct CatalogYaml {
    path: PathBuf,
    bom: bool,
    text: String,
    edits: Vec<(Range<usize>, String)>,
}

impl CatalogYaml {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| path.display().to_string())?;
        let (bom, text) = match text.strip_prefix('\u{feff}') {
            Some(rest) => (true, rest.to_owned()),
            None => (false, text),
        };
        Ok(Self {
            path: path.to_owned(),
            bom,
            text,
            edits: Vec::new(),
        })
    }

    pub fn set(&mut self, key_path: &[String], value: &str) -> Result<()> {
        let context = || format!("{}: {:?}", self.path.display(), key_path.join("."));
        let range = self.locate(key_path).with_context(context)?;
        let value = emit_scalar(value)?;
        // Two key paths land on one range when a mapping is reached through
        // an alias, and every dependent of an entry asks for the same text.
        if let Some((_, existing)) = self.edits.iter().find(|(edit, _)| *edit == range) {
            if *existing != value {
                bail!("{}: rewritten twice with different values", context());
            }
            return Ok(());
        }
        self.edits.push((range, value));
        Ok(())
    }

    // The markers count characters, and the end of a quoted scalar takes the
    // spaces and the comment after it on its line along. The span of a block
    // scalar starts after the header and ends past the line break, so it is
    // told apart by that break and refused.
    fn locate(&self, key_path: &[String]) -> Result<Range<usize>> {
        let docs = MarkedYaml::load_from_str(&self.text)
            .map_err(|err| anyhow::anyhow!("invalid YAML: {err}"))?;
        let mut node = docs.first().context("the file holds no document")?;
        for key in key_path {
            node = node
                .data
                .as_mapping_get(key)
                .with_context(|| format!("missing {key:?}"))?;
        }
        if !matches!(node.data, YamlData::Value(_)) {
            bail!("not a scalar");
        }
        let byte_offset = |index: usize| {
            self.text
                .char_indices()
                .nth(index)
                .map_or(self.text.len(), |(offset, _)| offset)
        };
        let range = byte_offset(node.span.start.index())..byte_offset(node.span.end.index());
        if self.text[range.clone()]
            .trim_end_matches(' ')
            .ends_with('\n')
        {
            bail!("a block scalar cannot be rewritten");
        }
        Ok(range)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn text(&self) -> String {
        let mut edits: Vec<_> = self.edits.iter().collect();
        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        let mut text = self.text.clone();
        for (range, value) in edits {
            text.replace_range(range.clone(), value);
        }
        if self.bom {
            text.insert(0, '\u{feff}');
        }
        text
    }
}

// The emitter quotes what a plain scalar could not carry (`>=1.0.0`, `*`),
// but only writes whole documents, so its leading marker is dropped.
fn emit_scalar(value: &str) -> Result<String> {
    let mut out = String::new();
    YamlEmitter::new(&mut out).dump(&Yaml::Value(Scalar::String(value.into())))?;
    Ok(out
        .strip_prefix("---\n")
        .expect("the emitter starts a document with `---`")
        .to_owned())
}
