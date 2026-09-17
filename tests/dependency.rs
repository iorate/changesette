mod util;

use std::path::Path;

use changesette::{
    config::Config,
    dependency::{
        Alias, DependentsGraph, InternalDependency, Spec, effective_range, internal_dependencies,
        parse_spec,
    },
    workspace::{DependencyField, RelDir, Root, Workspace, rel_dir_between},
};
use nodejs_semver::{Range, Version};
use tempfile::TempDir;
use util::{capture_output, write_file};

fn rel_dir(rel: &str) -> RelDir {
    rel_dir_between(Path::new("/root"), &Path::new("/root").join(rel))
}

fn range(text: &str) -> Range {
    Range::parse(text).unwrap()
}

fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn spec(text: &str) -> Spec {
    parse_spec(text, &rel_dir("packages/a"), &rel_dir("packages/b"), false)
}

fn plain(text: &str) -> Spec {
    Spec::Range {
        range: range(text),
        workspace: false,
    }
}

fn workspace_dir(manifests: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "pnpm-workspace.yaml",
        "packages:\n  - \"packages/*\"\n",
    );
    for (rel, manifest) in manifests {
        write_file(dir.path(), &format!("{rel}/package.json"), manifest);
    }
    dir
}

fn load(dir: &Path) -> Workspace {
    Workspace::load(Root::find(dir).unwrap(), &Config::default(), &[]).unwrap()
}

fn edges(workspace: &Workspace, workspace_only: bool) -> Vec<(&str, &str, DependencyField)> {
    internal_dependencies(workspace, workspace_only)
        .unwrap()
        .iter()
        .map(|edge| {
            (
                workspace[&edge.dependent].name().unwrap_or("<unnamed>"),
                workspace[&edge.dependency].name().unwrap(),
                edge.field,
            )
        })
        .collect()
}

#[test]
fn parse_spec_classifies_plain_ranges_and_wildcards() {
    assert_eq!(spec("^1.0.0"), plain("^1.0.0"));
    assert_eq!(spec("1.x"), plain("1.x"));
    for text in ["*", "", " ", "x", "X", " * "] {
        assert_eq!(spec(text), Spec::Any, "{text:?}");
    }
    assert_eq!(spec("latest"), Spec::External);
    assert_eq!(spec("file:../b"), Spec::External);
}

#[test]
fn parse_spec_classifies_the_workspace_protocol() {
    assert_eq!(spec("workspace:*"), Spec::WorkspaceAlias(Alias::Exact));
    assert_eq!(spec("workspace:^"), Spec::WorkspaceAlias(Alias::Caret));
    assert_eq!(spec("workspace:~"), Spec::WorkspaceAlias(Alias::Tilde));
    assert_eq!(
        spec("workspace:^1.0.0"),
        Spec::Range {
            range: range("^1.0.0"),
            workspace: true,
        }
    );
    assert_eq!(spec("workspace:../b"), Spec::WorkspacePath);
    assert_eq!(spec("workspace:../c"), Spec::External);
}

#[test]
fn parse_spec_with_workspace_only_keeps_only_the_workspace_protocol() {
    let only = |text| parse_spec(text, &rel_dir("packages/a"), &rel_dir("packages/b"), true);
    assert_eq!(only("^1.0.0"), Spec::External);
    assert_eq!(only("*"), Spec::External);
    assert_eq!(only("workspace:^"), Spec::WorkspaceAlias(Alias::Caret));
}

#[test]
fn effective_range_expands_aliases_against_the_dependency_version() {
    let current = version("1.2.3");
    assert_eq!(effective_range(&Spec::Any, &current), Some(Range::any()));
    assert_eq!(
        effective_range(&plain(">=1.0.0 <2.0.0"), &current),
        Some(range(">=1.0.0 <2.0.0"))
    );
    assert_eq!(
        effective_range(&Spec::WorkspaceAlias(Alias::Exact), &current),
        Some(range("1.2.3"))
    );
    assert_eq!(
        effective_range(&Spec::WorkspaceAlias(Alias::Caret), &current),
        Some(range("^1.2.3"))
    );
    assert_eq!(
        effective_range(&Spec::WorkspaceAlias(Alias::Tilde), &current),
        Some(range("~1.2.3"))
    );
    assert_eq!(
        effective_range(&Spec::WorkspacePath, &current),
        Some(range("1.2.3"))
    );
    assert_eq!(effective_range(&Spec::External, &current), None);
    let pre = version("1.2.3-beta.0");
    assert_eq!(
        effective_range(&Spec::WorkspaceAlias(Alias::Caret), &pre),
        Some(range("^1.2.3-beta.0"))
    );
}

#[test]
fn internal_dependencies_keep_edges_to_versioned_workspace_packages() {
    let dir = workspace_dir(&[
        (
            ".",
            "{ \"dependencies\": { \"pkg-a\": \"workspace:*\" } }\n",
        ),
        (
            "packages/a",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"dependencies\": { \"pkg-b\": \"^2.0.0\", \"left-pad\": \"^1.0.0\", \"pkg-a\": \"1.0.0\" }, \"devDependencies\": { \"pkg-b\": \"workspace:^\" }, \"peerDependencies\": { \"pkg-c\": \"*\" } }\n",
        ),
        (
            "packages/b",
            "{ \"name\": \"pkg-b\", \"version\": \"2.0.0\" }\n",
        ),
        (
            "packages/c",
            "{ \"name\": \"pkg-c\", \"dependencies\": { \"pkg-a\": \"workspace:../a\" } }\n",
        ),
    ]);
    let workspace = load(dir.path());
    let mut found = Vec::new();
    let output = capture_output(|| found = edges(&workspace, false));
    assert_eq!(
        found,
        [
            ("<unnamed>", "pkg-a", DependencyField::Dependencies),
            ("pkg-a", "pkg-b", DependencyField::Dependencies),
            ("pkg-a", "pkg-b", DependencyField::DevDependencies),
            ("pkg-c", "pkg-a", DependencyField::Dependencies),
        ]
    );
    assert!(!output.contains("warning: "), "{output}");
}

#[test]
fn internal_dependencies_ignore_a_range_missing_the_current_version() {
    let dir = workspace_dir(&[
        (
            "packages/a",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"dependencies\": { \"pkg-b\": \"^1.0.0\" }, \"peerDependencies\": { \"pkg-b\": \">=1\" } }\n",
        ),
        (
            "packages/b",
            "{ \"name\": \"pkg-b\", \"version\": \"2.0.0\" }\n",
        ),
    ]);
    let workspace = load(dir.path());
    let mut found = Vec::new();
    let output = capture_output(|| found = edges(&workspace, false));
    assert_eq!(
        found,
        [("pkg-a", "pkg-b", DependencyField::PeerDependencies)]
    );
    assert_eq!(
        output,
        "warning: pkg-a (packages/a): depends on `pkg-b` at \"^1.0.0\", which does not include the workspace's pkg-b@2.0.0; the dependency is ignored\n"
    );
}

#[test]
fn internal_dependencies_fail_on_a_dependency_with_a_duplicated_name() {
    let dir = workspace_dir(&[
        (
            "packages/a",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"dependencies\": { \"dup\": \"^1.0.0\" } }\n",
        ),
        (
            "packages/b",
            "{ \"name\": \"dup\", \"version\": \"1.0.0\" }\n",
        ),
        (
            "packages/c",
            "{ \"name\": \"dup\", \"version\": \"1.0.0\" }\n",
        ),
    ]);
    let workspace = load(dir.path());
    let err = internal_dependencies(&workspace, false).unwrap_err();
    assert_eq!(
        format!("{err:#}"),
        "package `dup` is ambiguous: used by packages/b, packages/c"
    );
}

#[test]
fn internal_dependencies_honor_workspace_only() {
    let dir = workspace_dir(&[
        (
            "packages/a",
            "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"dependencies\": { \"pkg-b\": \"^2.0.0\" }, \"devDependencies\": { \"pkg-b\": \"workspace:^\" } }\n",
        ),
        (
            "packages/b",
            "{ \"name\": \"pkg-b\", \"version\": \"2.0.0\" }\n",
        ),
    ]);
    let workspace = load(dir.path());
    assert_eq!(
        edges(&workspace, true),
        [("pkg-a", "pkg-b", DependencyField::DevDependencies)]
    );
}

#[test]
fn dependents_graph_lists_every_field_of_a_dependent() {
    let edge = |dependent: &str, dependency: &str, field| InternalDependency {
        dependent: rel_dir(dependent),
        dependency: rel_dir(dependency),
        field,
        spec_text: String::new(),
        spec: Spec::Any,
    };
    let graph = DependentsGraph::build(vec![
        edge(
            "packages/a",
            "packages/b",
            DependencyField::PeerDependencies,
        ),
        edge("packages/c", "packages/a", DependencyField::Dependencies),
        edge("packages/a", "packages/b", DependencyField::DevDependencies),
    ]);
    let dependents = |dependency: &str| {
        graph
            .dependents(&rel_dir(dependency))
            .map(|edge| (edge.dependent.as_str().to_owned(), edge.field))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        dependents("packages/b"),
        [
            ("packages/a".to_owned(), DependencyField::PeerDependencies),
            ("packages/a".to_owned(), DependencyField::DevDependencies),
        ]
    );
    assert_eq!(
        dependents("packages/a"),
        [("packages/c".to_owned(), DependencyField::Dependencies)]
    );
    assert!(dependents("packages/c").is_empty());
    assert_eq!(graph.iter().count(), 3);
}
