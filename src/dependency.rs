use std::collections::BTreeMap;

use nodejs_semver::{Range, Version};
use tracing::{debug, warn};

use crate::{
    catalog,
    workspace::{DependencyField, RelDir, Workspace},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    Any,
    Range { range: Range, workspace: bool },
    WorkspaceAlias(Alias),
    WorkspacePath,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alias {
    Exact,
    Caret,
    Tilde,
}

#[must_use]
pub fn parse_spec(
    spec: &str,
    dependent: &RelDir,
    dependency: &RelDir,
    workspace_only: bool,
) -> Spec {
    if let Some(rest) = spec.strip_prefix("workspace:") {
        return match rest {
            "*" => Spec::WorkspaceAlias(Alias::Exact),
            "^" => Spec::WorkspaceAlias(Alias::Caret),
            "~" => Spec::WorkspaceAlias(Alias::Tilde),
            _ => match Range::parse(rest) {
                Ok(range) => Spec::Range {
                    range,
                    workspace: true,
                },
                Err(_) if dependent.join(rest) == *dependency => Spec::WorkspacePath,
                Err(_) => Spec::External,
            },
        };
    }
    if workspace_only || spec.contains(':') {
        return Spec::External;
    }
    if matches!(spec.trim(), "" | "*" | "x" | "X") {
        return Spec::Any;
    }
    match Range::parse(spec) {
        Ok(range) => Spec::Range {
            range,
            workspace: false,
        },
        Err(_) => Spec::External,
    }
}

#[must_use]
pub fn effective_range(spec: &Spec, dependency_version: &Version) -> Option<Range> {
    let text = match spec {
        Spec::Any => return Some(Range::any()),
        Spec::Range { range, .. } => return Some(range.clone()),
        Spec::WorkspaceAlias(Alias::Exact) | Spec::WorkspacePath => dependency_version.to_string(),
        Spec::WorkspaceAlias(Alias::Caret) => format!("^{dependency_version}"),
        Spec::WorkspaceAlias(Alias::Tilde) => format!("~{dependency_version}"),
        Spec::External => return None,
    };
    Some(Range::parse(&text).expect("a version makes a valid range"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecLocation {
    Manifest,
    Catalog { name: String, path: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalDependency {
    pub dependent: RelDir,
    pub dependency: RelDir,
    pub field: DependencyField,
    pub spec_text: String,
    pub spec: Spec,
    pub location: SpecLocation,
}

#[must_use]
pub fn internal_dependencies(
    workspace: &Workspace,
    workspace_only: bool,
) -> Vec<InternalDependency> {
    let mut internal = Vec::new();
    for dependent in workspace.packages() {
        for dependency in dependent.dependencies() {
            let name = &dependency.name;
            let Some(target) = workspace.package(name) else {
                continue;
            };
            if target.rel_dir() == dependent.rel_dir() {
                continue;
            }
            let Some(version) = target.version() else {
                debug!(
                    "{dependent}: depends on `{name}`, which has no version and is never released; the dependency is ignored"
                );
                continue;
            };
            let (spec_text, location) = match catalog::reference(&dependency.spec) {
                None => (dependency.spec.clone(), SpecLocation::Manifest),
                Some(reference) => match workspace.catalogs().lookup(reference, name) {
                    Some((catalog, entry)) if catalog::reference(&entry.spec).is_none() => (
                        entry.spec.clone(),
                        SpecLocation::Catalog {
                            name: catalog.to_owned(),
                            path: entry.path.clone(),
                        },
                    ),
                    Some((catalog, entry)) => {
                        warn!(
                            "{dependent}: depends on `{name}` at {:?}, whose entry in {} is {:?}, another catalog reference; the dependency is ignored",
                            dependency.spec,
                            catalog::describe(catalog),
                            entry.spec
                        );
                        continue;
                    }
                    None => {
                        warn!(
                            "{dependent}: depends on `{name}` at {:?}, which has no catalog entry; the dependency is ignored",
                            dependency.spec
                        );
                        continue;
                    }
                },
            };
            let spec = parse_spec(
                &spec_text,
                dependent.rel_dir(),
                target.rel_dir(),
                workspace_only,
            );
            let Some(range) = effective_range(&spec, version) else {
                continue;
            };
            if !range.satisfies(version) {
                warn!(
                    "{dependent}: depends on `{name}` at {spec_text:?}, which does not include the workspace's {name}@{version}; the dependency is ignored"
                );
                continue;
            }
            internal.push(InternalDependency {
                dependent: dependent.rel_dir().clone(),
                dependency: target.rel_dir().clone(),
                field: dependency.field,
                spec_text,
                spec,
                location,
            });
        }
    }
    internal
}

#[derive(Debug, Default)]
pub struct DependentsGraph {
    edges: BTreeMap<RelDir, Vec<InternalDependency>>,
}

impl DependentsGraph {
    #[must_use]
    pub fn build(dependencies: Vec<InternalDependency>) -> DependentsGraph {
        let mut edges: BTreeMap<RelDir, Vec<InternalDependency>> = BTreeMap::new();
        for dependency in dependencies {
            edges
                .entry(dependency.dependency.clone())
                .or_default()
                .push(dependency);
        }
        DependentsGraph { edges }
    }

    pub fn dependents(&self, dependency: &RelDir) -> impl Iterator<Item = &InternalDependency> {
        self.edges.get(dependency).into_iter().flatten()
    }

    pub fn iter(&self) -> impl Iterator<Item = &InternalDependency> {
        self.edges.values().flatten()
    }
}
