mod util;

use std::{fs, io, path::Path};

use anyhow::Result;
use changesette::workspace::{Member, Root, Workspace, resolve_root};
use semver::Version;
use tempfile::TempDir;
use util::{capture_output, names_and_rel_dirs, write_file};

fn pkg(name: &str) -> String {
    format!("{{ \"name\": \"{name}\", \"version\": \"1.0.0\" }}\n")
}

fn discover(cwd: &Path) -> Result<Workspace> {
    Workspace::load(Root::find(cwd)?, None)
}

fn discover_ok(cwd: &Path) -> Workspace {
    discover(cwd).unwrap()
}

fn discover_err(cwd: &Path) -> String {
    format!("{:#}", discover(cwd).unwrap_err())
}

fn discover_captured(cwd: &Path) -> (Workspace, String) {
    let mut workspace = None;
    let output = capture_output(|| workspace = Some(discover(cwd).unwrap()));
    (workspace.unwrap(), output)
}

fn forced(root: &Path) -> Workspace {
    Workspace::load(Root::new(root.to_path_buf()), None).unwrap()
}

fn load_listed(root: &Path, packages: &[&str]) -> Result<Workspace> {
    let packages: Vec<String> = packages.iter().map(|dir| (*dir).to_owned()).collect();
    Workspace::load(Root::new(root.to_path_buf()), Some(&packages))
}

fn load_listed_err(root: &Path, packages: &[&str]) -> String {
    format!("{:#}", load_listed(root, packages).unwrap_err())
}

fn names_and_dirs(workspace: &Workspace) -> Vec<(&str, &Path)> {
    workspace
        .members()
        .iter()
        .map(|member| (member.name(), member.dir()))
        .collect()
}

fn names(workspace: &Workspace) -> Vec<&str> {
    workspace.members().iter().map(Member::name).collect()
}

fn pnpm_dir(patterns: &[&str]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let list: Vec<String> = patterns.iter().map(|p| format!("  - \"{p}\"")).collect();
    write_file(
        dir.path(),
        "pnpm-workspace.yaml",
        &format!("packages:\n{}\n", list.join("\n")),
    );
    dir
}

fn manifest_path(dir: &Path, rel: &str) -> String {
    let mut path = dir.to_path_buf();
    path.extend(rel.split('/').filter(|component| *component != "."));
    path.join("package.json").display().to_string()
}

fn warning_lines(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter(|line| line.starts_with("warning: "))
        .collect()
}

fn debug_lines(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter(|line| line.starts_with("debug: "))
        .collect()
}

// Members and their qualification

#[test]
fn members_are_sorted_by_name() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/one/package.json",
        "{ \"name\": \"zeta\", \"version\": \"1.0.0\" }\n",
    );
    write_file(
        dir.path(),
        "packages/two/package.json",
        "{ \"name\": \"alpha\", \"version\": \"2.0.0\", \"private\": true }\n",
    );
    write_file(
        dir.path(),
        "packages/no-manifest/note.txt",
        "no package.json here\n",
    );
    write_file(dir.path(), "packages/readme.txt", "not a directory\n");
    let workspace = discover_ok(dir.path());
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(workspace.changeset_dir(), dir.path().join(".changeset"));
    assert_eq!(
        names_and_dirs(&workspace),
        [
            ("alpha", dir.path().join("packages/two").as_path()),
            ("zeta", dir.path().join("packages/one").as_path()),
        ]
    );
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("alpha", "packages/two"), ("zeta", "packages/one")]
    );
    let alpha = &workspace.members()[0];
    assert_eq!(alpha.version(), &Version::new(2, 0, 0));
    assert!(alpha.private());
    assert!(!workspace.members()[1].private());
}

#[test]
fn resolves_a_member_by_name() {
    let dir = pnpm_dir(&["packages/*"]);
    write_file(dir.path(), "packages/one/package.json", &pkg("zeta"));
    write_file(dir.path(), "packages/two/package.json", &pkg("alpha"));
    let workspace = discover_ok(dir.path());
    assert_eq!(
        workspace.member("alpha").unwrap().dir(),
        dir.path().join("packages/two")
    );
}

#[test]
fn member_lookup_fails_for_an_unknown_name() {
    let dir = pnpm_dir(&["packages/*"]);
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    let err = format!(
        "{:#}",
        discover_ok(dir.path()).member("missing").unwrap_err()
    );
    assert!(err.contains("not found"), "{err}");
    let empty = tempfile::tempdir().unwrap();
    let err = format!(
        "{:#}",
        discover_ok(empty.path()).member("missing").unwrap_err()
    );
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn qualification_excludes_invalid_manifests() {
    for marker in ["pnpm", "npm"] {
        let dir = tempfile::tempdir().unwrap();
        match marker {
            "pnpm" => write_file(
                dir.path(),
                "pnpm-workspace.yaml",
                "packages:\n  - \"packages/*\"\n",
            ),
            _ => write_file(
                dir.path(),
                "package.json",
                "{ \"workspaces\": [\"packages/*\"] }\n",
            ),
        }
        for (name, manifest) in [
            ("a", "{ \"name\": \"pkg-a\", \"version\": \"3.1.4\" }\n"),
            ("b", "{ \"version\": \"1.0.0\" }\n"),
            ("c", "{ \"name\": \"pkg-c\" }\n"),
            ("d", "{ \"name\": \"pkg-d\", \"version\": \"1.0\" }\n"),
            ("e", "{ \"name\": \"dup\", \"version\": \"1.0.0\" }\n"),
            ("f", "{ \"name\": \"dup\", \"version\": \"2.0.0\" }\n"),
            ("g", "{ \"name\": 1, \"version\": \"1.0.0\" }\n"),
            ("h", "{ \"name\": \"\", \"version\": \"1.0.0\" }\n"),
            ("i", "{ \"name\": \"pkg-i\", \"version\": 1 }\n"),
            ("j", "[1, 2]\n"),
        ] {
            write_file(
                dir.path(),
                &format!("packages/{name}/package.json"),
                manifest,
            );
        }
        let (workspace, output) = discover_captured(dir.path());
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("pkg-a", "packages/a")],
            "{marker}"
        );
        let manifest = |name: &str| manifest_path(dir.path(), &format!("packages/{name}"));
        let warnings = warning_lines(&output);
        let debugs = debug_lines(&output);
        for name in ["d", "e", "f", "g", "h", "i", "j"] {
            assert!(
                warnings.iter().any(|line| line.contains(&manifest(name))),
                "{marker} {name}: {output}"
            );
            assert!(
                !debugs.iter().any(|line| line.contains(&manifest(name))),
                "{marker} {name}: {output}"
            );
        }
        for name in ["b", "c"] {
            assert!(
                debugs.iter().any(|line| line.contains(&manifest(name))),
                "{marker} {name}: {output}"
            );
            assert!(
                !warnings.iter().any(|line| line.contains(&manifest(name))),
                "{marker} {name}: {output}"
            );
        }
        assert!(output.contains("`dup`"), "{marker}: {output}");
    }
}

#[test]
fn reads_private_leniently() {
    let dir = pnpm_dir(&["packages/*"]);
    write_file(
        dir.path(),
        "packages/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"private\": true }\n",
    );
    write_file(
        dir.path(),
        "packages/b/package.json",
        "{ \"name\": \"pkg-b\", \"version\": \"2.0.0\", \"private\": \"true\" }\n",
    );
    write_file(
        dir.path(),
        "packages/c/package.json",
        "{ \"name\": \"pkg-c\", \"version\": \"3.0.0\", \"private\": \"yes\" }\n",
    );
    write_file(dir.path(), "packages/d/package.json", &pkg("pkg-d"));
    let workspace = discover_ok(dir.path());
    assert_eq!(names(&workspace), ["pkg-a", "pkg-b", "pkg-c", "pkg-d"]);
    assert_eq!(
        workspace
            .members()
            .iter()
            .map(Member::private)
            .collect::<Vec<_>>(),
        [true, false, false, false]
    );
}

#[test]
fn excludes_duplicate_package_names() {
    let dir = pnpm_dir(&["packages/*"]);
    write_file(
        dir.path(),
        "packages/a/package.json",
        "{ \"name\": \"dup\", \"version\": \"1.0.0\" }\n",
    );
    write_file(
        dir.path(),
        "packages/b/package.json",
        "{ \"name\": \"dup\", \"version\": \"2.0.0\" }\n",
    );
    write_file(dir.path(), "packages/c/package.json", &pkg("unique"));
    let workspace = discover_ok(dir.path());
    assert_eq!(names_and_rel_dirs(&workspace), [("unique", "packages/c")]);
    assert!(workspace.member("unique").is_ok());
    assert!(workspace.member("dup").is_err());
}

#[test]
fn a_disqualified_duplicate_does_not_evict_the_real_package() {
    let dir = pnpm_dir(&["packages/*"]);
    write_file(dir.path(), "packages/a/package.json", &pkg("dup"));
    write_file(
        dir.path(),
        "packages/b/package.json",
        "{ \"name\": \"dup\" }\n",
    );
    let workspace = discover_ok(dir.path());
    assert_eq!(names_and_rel_dirs(&workspace), [("dup", "packages/a")]);
}

#[cfg(unix)]
#[test]
fn collapses_candidates_aliasing_the_same_directory() {
    let dir = pnpm_dir(&["real/*", "link/*"]);
    write_file(dir.path(), "real/a/package.json", &pkg("pkg-a"));
    std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
    let workspace = discover_ok(dir.path());
    assert_eq!(
        names_and_dirs(&workspace),
        [("pkg-a", dir.path().join("link/a").as_path())]
    );
}

#[cfg(unix)]
#[test]
fn a_true_duplicate_is_excluded_even_beside_an_aliased_pair() {
    let dir = pnpm_dir(&["real/*", "link/*", "other/*"]);
    write_file(dir.path(), "real/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "other/a/package.json", &pkg("pkg-a"));
    std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
    assert_eq!(names(&discover_ok(dir.path())), [] as [&str; 0]);
}

#[test]
fn a_disqualified_nearest_package_yields_an_empty_workspace() {
    for manifest in ["{ \"name\": \"app\" }\n", "{ \"version\": \"1.0.0\" }\n"] {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "package.json", manifest);
        let workspace = discover_ok(dir.path());
        assert_eq!(workspace.root(), dir.path(), "{manifest}");
        assert_eq!(names(&workspace), [] as [&str; 0], "{manifest}");
    }
}

#[test]
fn a_nameless_root_is_not_a_member() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a")]
    );

    let dir = pnpm_dir(&[".", "packages/*"]);
    write_file(dir.path(), "package.json", "{ \"version\": \"1.0.0\" }\n");
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a")]
    );
}

// Root detection

#[test]
fn a_pnpm_manifest_wins_in_the_same_directory() {
    for (yaml, lock) in [
        ("onlyBuiltDependencies: []\n", false),
        ("packages:\n  - \"packages/*\"\n", false),
        ("packages:\n  - \"packages/*\"\n", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "pnpm-workspace.yaml", yaml);
        if lock {
            write_file(dir.path(), "yarn.lock", "");
        }
        write_file(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"apps/*\"] }\n",
        );
        write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
        write_file(dir.path(), "apps/b/package.json", &pkg("app-b"));
        let workspace = discover_ok(dir.path());
        assert_eq!(workspace.root(), dir.path(), "{yaml:?} lock={lock}");
        let expected: &[(&str, &str)] = if yaml.starts_with("packages") {
            &[("pkg-a", "packages/a"), ("root", ".")]
        } else {
            &[("root", ".")]
        };
        assert_eq!(
            names_and_rel_dirs(&workspace),
            expected,
            "{yaml:?} lock={lock}"
        );
    }
}

#[test]
fn ignores_an_invalid_pnpm_manifest_with_a_warning() {
    for (text, warns) in [
        ("", false),
        ("# packages:\n", false),
        ("packages: []\n", false),
        ("packages:\n", true),
        ("---\n", true),
        ("null\n", true),
        ("- packages/*\n", true),
        ("packages: \"packages/*\"\n", true),
        ("packages:\n  - \"packages/*\"\n  - 42\n", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "pnpm-workspace.yaml", text);
        write_file(dir.path(), "package.json", &pkg("root"));
        write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
        let (workspace, output) = discover_captured(dir.path());
        assert_eq!(workspace.root(), dir.path(), "{text:?}");
        assert_eq!(names_and_rel_dirs(&workspace), [("root", ".")], "{text:?}");
        let warnings = warning_lines(&output);
        if warns {
            assert_eq!(warnings.len(), 1, "{text:?}: {output}");
            assert!(
                warnings[0].contains(&dir.path().join("pnpm-workspace.yaml").display().to_string()),
                "{text:?}: {output}"
            );
        } else {
            assert!(warning_lines(&output).is_empty(), "{text:?}: {output}");
        }
    }
}

#[test]
fn rejects_an_unparsable_pnpm_manifest() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "pnpm-workspace.yaml", "packages: [\n");
    let err = discover_err(dir.path());
    assert!(
        err.contains(&dir.path().join("pnpm-workspace.yaml").display().to_string()),
        "{err}"
    );
}

#[test]
fn accepts_a_bom_prefixed_pnpm_manifest() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "pnpm-workspace.yaml",
        "\u{feff}packages:\n  - \"packages/*\"\n",
    );
    write_file(dir.path(), "package.json", &pkg("root"));
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a"), ("root", ".")]
    );
}

#[test]
fn an_inner_settings_only_pnpm_manifest_shadows_an_outer_workspace() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "packages/inner/package.json", &pkg("inner"));
    write_file(
        dir.path(),
        "packages/inner/pnpm-workspace.yaml",
        "onlyBuiltDependencies:\n  - esbuild\n",
    );
    let inner = dir.path().join("packages/inner");
    let workspace = discover_ok(&inner);
    assert_eq!(workspace.root(), inner);
    assert_eq!(names_and_rel_dirs(&workspace), [("inner", ".")]);
}

#[test]
fn an_outer_pnpm_manifest_wins_over_an_inner_workspaces_field() {
    for workspaces in ["[\"nested/*\"]", "{ \"nohoist\": [\"**/foo\"] }"] {
        let dir = pnpm_dir(&["packages/*"]);
        write_file(
            dir.path(),
            "packages/a/package.json",
            &format!(
                "{{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": {workspaces} }}\n"
            ),
        );
        write_file(
            dir.path(),
            "packages/a/nested/x/package.json",
            &pkg("pkg-x"),
        );
        let workspace = discover_ok(&dir.path().join("packages/a/nested/x"));
        assert_eq!(workspace.root(), dir.path(), "{workspaces}");
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("pkg-a", "packages/a")],
            "{workspaces}"
        );
    }
}

#[test]
fn an_outer_yarn_lock_wins_over_an_inner_workspaces_field() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/nested/x/package.json",
        &pkg("pkg-x"),
    );
    let workspace = discover_ok(&dir.path().join("packages/a/nested/x"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [
            ("pkg-a", "packages/a"),
            ("pkg-x", "packages/a/nested/x"),
            ("root", ".")
        ]
    );
}

#[test]
fn the_nearest_yarn_lock_wins() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": [\"packages/*\", \"examples/*\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "examples/e/yarn.lock", "");
    write_file(dir.path(), "examples/e/package.json", &pkg("example"));
    let inner = dir.path().join("examples/e");
    let workspace = discover_ok(&inner.join("src"));
    assert_eq!(workspace.root(), inner);
    assert_eq!(names_and_rel_dirs(&workspace), [("example", ".")]);
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("example", "examples/e"), ("pkg-a", "packages/a")]
    );
}

#[test]
fn resolves_the_workspace_root_from_a_member_directory() {
    let dir = pnpm_dir(&["packages/*"]);
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "packages/b/package.json", &pkg("pkg-b"));
    let workspace = discover_ok(&dir.path().join("packages/a"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("pkg-a", "packages/a"), ("pkg-b", "packages/b")]
    );
}

#[test]
fn falls_back_to_the_nearest_package_json() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "app/package.json", &pkg("app"));
    fs::create_dir_all(dir.path().join("app/src")).unwrap();
    let workspace = discover_ok(&dir.path().join("app/src"));
    assert_eq!(workspace.root(), dir.path().join("app"));
    assert_eq!(names_and_rel_dirs(&workspace), [("app", ".")]);
}

#[test]
fn the_working_directory_is_the_root_without_any_package_json() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a/b")).unwrap();
    let (workspace, output) = discover_captured(&dir.path().join("a/b"));
    assert_eq!(workspace.root(), dir.path().join("a/b"));
    assert_eq!(names(&workspace), [] as [&str; 0]);
    assert!(warning_lines(&output).is_empty(), "{output}");
}

#[test]
fn workspaces_without_a_lockfile_is_a_workspace_root() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"app\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    let workspace = discover_ok(dir.path());
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("app", "."), ("pkg-a", "packages/a")]
    );
}

#[test]
fn an_npm_root_reads_the_workspaces_object_form() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"app\", \"version\": \"1.0.0\", \"workspaces\": { \"packages\": [\"packages/*\"], \"nohoist\": [\"**\"] } }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    let workspace = discover_ok(dir.path());
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("app", "."), ("pkg-a", "packages/a")]
    );
}

#[test]
fn ignores_an_invalid_workspaces() {
    for workspaces in [
        "null",
        "false",
        "0",
        "\"\"",
        "\"packages/*\"",
        "42",
        "true",
        "[42, \"packages/*\"]",
        "[\"packages/*\", null]",
        "{}",
        "{ \"packages\": null }",
        "{ \"packages\": \"packages/*\" }",
        "{ \"packages\": [\"packages/*\", 42] }",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "package.json",
            &format!(
                "{{ \"name\": \"app\", \"version\": \"1.0.0\", \"workspaces\": {workspaces} }}\n"
            ),
        );
        write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
        let (workspace, output) = discover_captured(dir.path());
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("app", ".")],
            "{workspaces}"
        );
        let warnings = warning_lines(&output);
        assert_eq!(warnings.len(), 1, "{workspaces}: {output}");
        assert!(
            warnings[0].contains(&manifest_path(dir.path(), ".")),
            "{workspaces}: {output}"
        );
    }
}

#[test]
fn passes_over_an_invalid_workspaces_while_looking_for_an_npm_root() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "package.json", "{ \"workspaces\": 42 }\n");
    write_file(dir.path(), "pkg/package.json", &pkg("leaf"));
    let workspace = discover_ok(&dir.path().join("pkg"));
    assert_eq!(workspace.root(), dir.path().join("pkg"));
    assert_eq!(names_and_rel_dirs(&workspace), [("leaf", ".")]);
}

#[test]
fn the_npm_root_is_always_a_member_candidate() {
    for workspaces in [
        "[\"packages/*\"]",
        "[\".\"]",
        "[\"packages/*\", \"!.\"]",
        "[]",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "package.json",
            &format!(
                "{{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": {workspaces} }}\n"
            ),
        );
        write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
        let expected: &[(&str, &str)] = if workspaces.contains("packages/*") {
            &[("pkg-a", "packages/a"), ("root", ".")]
        } else {
            &[("root", ".")]
        };
        assert_eq!(
            names_and_rel_dirs(&discover_ok(dir.path())),
            expected,
            "{workspaces}"
        );
    }
}

#[test]
fn the_pnpm_root_is_always_a_member_candidate() {
    for (patterns, expected) in [
        (
            &["packages/*"][..],
            &[("pkg-a", "packages/a"), ("root", ".")][..],
        ),
        (&["."], &[("root", ".")]),
        (
            &["packages/*", "!."],
            &[("pkg-a", "packages/a"), ("root", ".")],
        ),
        (&[], &[("root", ".")]),
    ] {
        let dir = pnpm_dir(patterns);
        write_file(dir.path(), "package.json", &pkg("root"));
        write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
        assert_eq!(
            names_and_rel_dirs(&discover_ok(dir.path())),
            expected,
            "{patterns:?}"
        );
    }
    let dir = pnpm_dir(&["packages/*"]);
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a")]
    );
}

#[test]
fn a_pnpm_only_manifest_is_never_a_member() {
    let dir = pnpm_dir(&["packages/*"]);
    write_file(dir.path(), "package.yaml", "name: root\n");
    write_file(dir.path(), "package.json", &pkg("root"));
    write_file(
        dir.path(),
        "packages/a/package.yaml",
        "name: pkg-a\nversion: 1.0.0\n",
    );
    write_file(
        dir.path(),
        "packages/b/package.json5",
        "{ name: 'pkg-b' }\n",
    );
    write_file(dir.path(), "packages/b/package.json", &pkg("pkg-b"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-b", "packages/b"), ("root", ".")]
    );
}

// Re-rooting to an npm workspace root

#[test]
fn a_member_of_a_nested_npm_workspace_re_roots_only_to_the_inner_root() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"outer\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/nested/x/package.json",
        &pkg("pkg-x"),
    );
    let inner = dir.path().join("packages/a");
    let workspace = discover_ok(&inner.join("nested/x"));
    assert_eq!(workspace.root(), inner);
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("pkg-a", "."), ("pkg-x", "nested/x")]
    );
}

#[test]
fn re_roots_to_the_npm_root_listing_the_nearest_package_as_a_member() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "packages/b/package.json", &pkg("pkg-b"));
    fs::create_dir_all(dir.path().join("packages/a/src")).unwrap();
    let workspace = discover_ok(&dir.path().join("packages/a/src"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [
            ("pkg-a", "packages/a"),
            ("pkg-b", "packages/b"),
            ("root", ".")
        ]
    );
}

#[cfg(unix)]
#[test]
fn re_roots_from_a_member_the_npm_root_lists_through_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"link/*\"] }\n",
    );
    write_file(dir.path(), "real/a/package.json", &pkg("pkg-a"));
    std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
    let workspace = discover_ok(&dir.path().join("real/a"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("pkg-a", "link/a"), ("root", ".")]
    );
}

#[test]
fn re_roots_from_a_versionless_member_listed_by_the_npm_root() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"apps/*\", \"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "apps/web/package.json",
        "{ \"name\": \"web\", \"private\": true }\n",
    );
    write_file(dir.path(), "packages/lib/package.json", &pkg("lib"));
    let workspace = discover_ok(&dir.path().join("apps/web"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("lib", "packages/lib"), ("root", ".")]
    );
}

#[test]
fn re_roots_from_a_member_whose_name_is_duplicated() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\", \"fixtures/*\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("dup"));
    write_file(
        dir.path(),
        "fixtures/a/package.json",
        "{ \"name\": \"dup\", \"version\": \"0.0.0\" }\n",
    );
    write_file(dir.path(), "packages/b/package.json", &pkg("pkg-b"));
    let workspace = discover_ok(&dir.path().join("packages/a"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("pkg-b", "packages/b"), ("root", ".")]
    );
}

#[test]
fn a_package_the_npm_root_above_does_not_list_is_a_single_package() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "examples/x/package.json", &pkg("example-x"));
    write_file(dir.path(), "packages/a/src/package.json", &pkg("stray"));
    for (rel, name) in [("examples/x", "example-x"), ("packages/a/src", "stray")] {
        let single = dir.path().join(rel);
        let workspace = discover_ok(&single);
        assert_eq!(workspace.root(), single, "{rel}");
        assert_eq!(names_and_rel_dirs(&workspace), [(name, ".")], "{rel}");
    }
}

#[test]
fn a_member_with_a_leftover_workspaces_field_re_roots_to_the_npm_root() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/nested/x/package.json",
        &pkg("pkg-x"),
    );
    let workspace = discover_ok(&dir.path().join("packages/a"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("pkg-a", "packages/a"), ("root", ".")]
    );
}

#[test]
fn an_npm_workspace_between_a_member_and_its_root_is_passed_over() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"a/b\"] }\n",
    );
    write_file(
        dir.path(),
        "a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"c\"] }\n",
    );
    write_file(dir.path(), "a/b/package.json", &pkg("pkg-b"));
    write_file(dir.path(), "a/c/package.json", &pkg("pkg-c"));
    let workspace = discover_ok(&dir.path().join("a/b"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("pkg-b", "a/b"), ("root", ".")]
    );
}

#[test]
fn re_roots_to_an_npm_root_with_a_parent_pattern() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "root/package.json",
        "{ \"workspaces\": [\"../ext/*\", \"packages/*\"] }\n",
    );
    write_file(dir.path(), "root/packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "ext/o/package.json", &pkg("pkg-o"));
    let workspace = discover_ok(&dir.path().join("root/packages/a"));
    assert_eq!(workspace.root(), dir.path().join("root"));
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("pkg-a", "packages/a"), ("pkg-o", "../ext/o")]
    );
}

#[test]
fn a_broken_ancestor_manifest_is_passed_over_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\",\n<<<<<<< HEAD\n  \"workspaces\": [\"app\"]\n}\n",
    );
    write_file(dir.path(), "app/package.json", &pkg("app"));
    let (workspace, output) = discover_captured(&dir.path().join("app"));
    assert_eq!(workspace.root(), dir.path().join("app"));
    assert_eq!(names_and_rel_dirs(&workspace), [("app", ".")]);
    let warnings = warning_lines(&output);
    assert_eq!(warnings.len(), 1, "{output}");
    assert!(
        warnings[0].contains(&manifest_path(dir.path(), ".")),
        "{output}"
    );

    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"a/b\"] }\n",
    );
    write_file(dir.path(), "a/package.json", "{ broken");
    write_file(dir.path(), "a/b/package.json", &pkg("pkg-b"));
    let (workspace, output) = discover_captured(&dir.path().join("a/b"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("pkg-b", "a/b"), ("root", ".")]
    );
    let warnings = warning_lines(&output);
    assert_eq!(warnings.len(), 1, "{output}");
    assert!(
        warnings[0].contains(&manifest_path(dir.path(), "a")),
        "{output}"
    );
}

#[test]
fn a_broken_nearest_manifest_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "a/package.json", "{ broken");
    let err = discover_err(&dir.path().join("a"));
    assert!(err.starts_with(&manifest_path(dir.path(), "a")), "{err}");
}

#[test]
fn a_broken_manifest_below_a_pnpm_root_is_never_read() {
    let dir = pnpm_dir(&["a/b"]);
    write_file(dir.path(), "a/package.json", "{ broken");
    write_file(dir.path(), "a/b/package.json", &pkg("pkg-b"));
    let (workspace, output) = discover_captured(&dir.path().join("a/b"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(names_and_rel_dirs(&workspace), [("pkg-b", "a/b")]);
    assert!(warning_lines(&output).is_empty(), "{output}");
}

#[test]
fn odd_manifests_during_the_root_search() {
    for stray in ["[1, 2]", "\"hello\""] {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "package.json",
            "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [] }\n",
        );
        write_file(dir.path(), "sub/package.json", stray);
        let workspace = discover_ok(&dir.path().join("sub"));
        assert_eq!(workspace.root(), dir.path().join("sub"), "{stray}");
        assert_eq!(names(&workspace), [] as [&str; 0], "{stray}");
    }
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [] }\n",
    );
    fs::create_dir_all(dir.path().join("sub/package.json")).unwrap();
    assert_eq!(discover_ok(&dir.path().join("sub")).root(), dir.path());
}

#[test]
fn a_broken_member_manifest_is_an_error_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"app\"] }\n",
    );
    write_file(
        dir.path(),
        "app/package.json",
        "{ \"name\": \"app\",\n<<<<<<< HEAD\n  \"version\": \"1.0.0\"\n}\n",
    );
    let err = discover_err(&dir.path().join("app"));
    assert!(err.contains(&manifest_path(dir.path(), "app")), "{err}");

    let dir = pnpm_dir(&["packages/*"]);
    write_file(dir.path(), "packages/a/package.json", "{ broken");
    let err = discover_err(dir.path());
    assert!(
        err.contains(&manifest_path(dir.path(), "packages/a")),
        "{err}"
    );
}

#[test]
fn reads_manifests_through_a_bom() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "\u{feff}{ \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/package.json",
        "\u{feff}{ \"name\": \"pkg-a\", \"version\": \"1.0.0\" }\n",
    );
    let workspace = discover_ok(dir.path());
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "packages/a")]);
}

// Yarn worktrees

#[test]
fn a_yarn_lock_alone_is_a_workspace_root_without_members() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    write_file(dir.path(), "a/package.json", &pkg("pkg-a"));
    let workspace = discover_ok(&dir.path().join("a"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(names(&workspace), [] as [&str; 0]);
}

#[test]
fn a_yarn_lock_beside_a_manifest_makes_the_root_the_only_member() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    write_file(dir.path(), "package.json", &pkg("root"));
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    let workspace = discover_ok(&dir.path().join("packages/a"));
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(names_and_rel_dirs(&workspace), [("root", ".")]);
}

#[test]
fn a_yarn_lock_beside_workspaces_is_a_yarn_root() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a"), ("root", ".")]
    );
}

#[test]
fn a_yarn_root_reads_the_workspaces_object_form() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": { \"packages\": [\"packages/*\"] } }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a")]
    );
}

#[test]
fn a_yarn_root_ignores_an_invalid_workspaces() {
    for workspaces in [
        "\"packages/*\"",
        "42",
        "true",
        "{}",
        "{ \"packages\": \"packages/*\" }",
        "{ \"nohoist\": [\"**/foo\"] }",
        "[\"packages/*\", 42, null]",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "yarn.lock", "");
        write_file(
            dir.path(),
            "package.json",
            &format!(
                "{{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": {workspaces} }}\n"
            ),
        );
        write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
        let workspace = discover_ok(dir.path());
        assert_eq!(workspace.root(), dir.path(), "{workspaces}");
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("root", ".")],
            "{workspaces}"
        );
    }
}

#[test]
fn a_yarn_member_with_an_invalid_workspaces_declares_no_worktree() {
    for workspaces in [
        "\"nested/*\"",
        "[\"nested/*\", 42]",
        "{ \"nohoist\": [\"**/react-native\"] }",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "yarn.lock", "");
        write_file(
            dir.path(),
            "package.json",
            "{ \"workspaces\": [\"packages/*\"] }\n",
        );
        write_file(
            dir.path(),
            "packages/a/package.json",
            &format!(
                "{{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": {workspaces} }}\n"
            ),
        );
        write_file(
            dir.path(),
            "packages/a/nested/x/package.json",
            &pkg("pkg-x"),
        );
        assert_eq!(
            names_and_rel_dirs(&discover_ok(dir.path())),
            [("pkg-a", "packages/a")],
            "{workspaces}"
        );
    }
}

#[test]
fn a_yarn_member_expands_its_own_workspaces_field() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/x\", \"nested/y\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/nested/x/package.json",
        "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\", \"workspaces\": [\"deep/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/nested/x/deep/z/package.json",
        &pkg("pkg-z"),
    );
    write_file(
        dir.path(),
        "packages/a/nested/y/package.json",
        &pkg("pkg-y"),
    );
    write_file(dir.path(), "packages/b/package.json", &pkg("pkg-b"));
    let workspace = discover_ok(&dir.path().join("packages/b"));
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [
            ("pkg-a", "packages/a"),
            ("pkg-b", "packages/b"),
            ("pkg-x", "packages/a/nested/x"),
            ("pkg-y", "packages/a/nested/y"),
            ("pkg-z", "packages/a/nested/x/deep/z"),
            ("root", "."),
        ]
    );
    assert_eq!(
        workspace.member("pkg-z").unwrap().dir(),
        dir.path().join("packages/a/nested/x/deep/z")
    );
}

#[test]
fn a_yarn_member_reads_the_object_form_and_a_null_field() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": { \"packages\": [\"packages/*\"] } }\n",
    );
    write_file(
        dir.path(),
        "packages/desktop/package.json",
        "{ \"name\": \"desktop\", \"version\": \"1.0.0\", \"workspaces\": { \"packages\": [\"app\"] } }\n",
    );
    write_file(
        dir.path(),
        "packages/desktop/app/package.json",
        &pkg("desktop-app"),
    );
    write_file(
        dir.path(),
        "packages/mobile/package.json",
        "{ \"name\": \"mobile\", \"version\": \"1.0.0\", \"workspaces\": null }\n",
    );
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [
            ("desktop", "packages/desktop"),
            ("desktop-app", "packages/desktop/app"),
            ("mobile", "packages/mobile"),
        ]
    );
}

#[test]
fn a_root_negation_does_not_reach_a_yarn_member_declaration() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": [\"packages/*\", \"!packages/a/nested/x\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/*\", \"!nested/y\"] }\n",
    );
    for name in ["x", "y"] {
        write_file(
            dir.path(),
            &format!("packages/a/nested/{name}/package.json"),
            &pkg(&format!("pkg-{name}")),
        );
    }
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a"), ("pkg-x", "packages/a/nested/x")]
    );
}

#[cfg(unix)]
#[test]
fn a_yarn_member_symlinked_to_itself_is_expanded_once() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/x/package.json",
        "{ \"name\": \"pkg-x\", \"version\": \"1.0.0\", \"workspaces\": [\"loop\", \"*\"] }\n",
    );
    std::os::unix::fs::symlink(".", dir.path().join("packages/x/loop")).unwrap();
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-x", "packages/x")]
    );
}

#[test]
fn a_yarn_root_listed_by_a_parent_pattern_is_expanded_once() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "root/yarn.lock", "");
    write_file(
        dir.path(),
        "root/package.json",
        "{ \"workspaces\": [\"../*\", \"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "root/packages/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0\" }\n",
    );
    let (workspace, output) = discover_captured(&dir.path().join("root"));
    assert_eq!(names(&workspace), [] as [&str; 0]);
    let warnings = warning_lines(&output);
    assert_eq!(warnings.len(), 1, "{output}");
    assert!(
        warnings[0].contains(&manifest_path(dir.path(), "root/packages/a")),
        "{output}"
    );
}

#[test]
fn a_yarn_member_declaration_reaches_outside_its_directory() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "root/yarn.lock", "");
    write_file(
        dir.path(),
        "root/package.json",
        "{ \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "root/packages/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"../b\", \"../../../ext/o\"] }\n",
    );
    write_file(dir.path(), "root/packages/b/package.json", &pkg("pkg-b"));
    write_file(dir.path(), "ext/o/package.json", &pkg("pkg-o"));
    let expected = [
        ("pkg-a", "packages/a"),
        ("pkg-b", "packages/b"),
        ("pkg-o", "../ext/o"),
    ];
    assert_eq!(
        names_and_rel_dirs(&discover_ok(&dir.path().join("root"))),
        expected
    );
    write_file(
        dir.path(),
        "root/package.json",
        "{ \"workspaces\": [\"packages/a\"] }\n",
    );
    let workspace = discover_ok(&dir.path().join("root"));
    assert_eq!(names_and_rel_dirs(&workspace), expected);
    assert_eq!(
        workspace.member("pkg-b").unwrap().dir(),
        dir.path().join("root/packages/b")
    );
    assert_eq!(
        workspace.member("pkg-o").unwrap().dir(),
        dir.path().join("ext/o")
    );
}

#[test]
fn a_yarn_member_declaration_climbing_back_into_the_root_is_spelled_from_the_root() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "root/yarn.lock", "");
    write_file(
        dir.path(),
        "root/package.json",
        "{ \"workspaces\": [\"packages/a\"] }\n",
    );
    write_file(
        dir.path(),
        "root/packages/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"../../other/c\"] }\n",
    );
    write_file(dir.path(), "root/other/c/package.json", &pkg("pkg-c"));
    let workspace = discover_ok(&dir.path().join("root"));
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("pkg-a", "packages/a"), ("pkg-c", "other/c")]
    );
    assert_eq!(
        workspace.member("pkg-c").unwrap().dir(),
        dir.path().join("root/other/c")
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_yarn_member_declaration_climbs_the_lexical_parent() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "root/yarn.lock", "");
    write_file(
        dir.path(),
        "root/package.json",
        "{ \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "real/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"../b\"] }\n",
    );
    write_file(dir.path(), "real/b/package.json", &pkg("physical-b"));
    write_file(
        dir.path(),
        "root/packages/b/package.json",
        &pkg("lexical-b"),
    );
    std::os::unix::fs::symlink("../../real/a", dir.path().join("root/packages/link")).unwrap();
    assert_eq!(
        names_and_rel_dirs(&discover_ok(&dir.path().join("root"))),
        [("lexical-b", "packages/b"), ("pkg-a", "packages/link")]
    );
}

#[test]
fn a_yarn_member_declaration_takes_the_pattern_error() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"x/../b\"] }\n",
    );
    write_file(dir.path(), "packages/b/package.json", &pkg("pkg-b"));
    let err = discover_err(dir.path());
    assert!(err.contains("\"x/../b\""), "{err}");
    assert!(
        err.contains(&manifest_path(dir.path(), "packages/a")),
        "{err}"
    );
}

#[test]
fn an_npm_member_does_not_expand_its_workspaces_field() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/package.json",
        "{ \"name\": \"pkg-a\", \"version\": \"1.0.0\", \"workspaces\": [\"nested/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/a/nested/x/package.json",
        &pkg("pkg-x"),
    );
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a")]
    );
}

// Patterns against the filesystem

#[test]
fn double_star_matches_zero_and_more_components() {
    let dir = pnpm_dir(&["packages/**"]);
    write_file(dir.path(), "packages/package.json", &pkg("direct"));
    write_file(
        dir.path(),
        "packages/nested/deep/package.json",
        &pkg("deep"),
    );
    write_file(
        dir.path(),
        "packages/node_modules/evil/package.json",
        &pkg("evil"),
    );
    write_file(
        dir.path(),
        "packages/bower_components/old/package.json",
        &pkg("old"),
    );
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [
            ("deep", "packages/nested/deep"),
            ("direct", "packages"),
            ("old", "packages/bower_components/old"),
        ]
    );

    let dir = pnpm_dir(&["a/**/z"]);
    for rel in ["a/z", "a/b/z", "a/b/c/z", "a/y"] {
        write_file(
            dir.path(),
            &format!("{rel}/package.json"),
            &pkg(&rel.replace('/', "-")),
        );
    }
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("a-b-c-z", "a/b/c/z"), ("a-b-z", "a/b/z"), ("a-z", "a/z")]
    );

    let dir = pnpm_dir(&["**/**"]);
    write_file(dir.path(), "package.json", &pkg("root"));
    write_file(dir.path(), "x/package.json", &pkg("x"));
    write_file(dir.path(), "x/y/package.json", &pkg("x-y"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("root", "."), ("x", "x"), ("x-y", "x/y")]
    );
}

#[test]
fn node_modules_is_never_entered_nor_named() {
    let dir = pnpm_dir(&["**"]);
    write_file(dir.path(), "package.json", &pkg("root"));
    write_file(dir.path(), "a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "node_modules/evil/package.json", &pkg("evil"));
    write_file(dir.path(), "bower_components/old/package.json", &pkg("old"));
    write_file(dir.path(), ".yarn/x/package.json", &pkg("yarn-x"));
    write_file(dir.path(), ".git/c/package.json", &pkg("git-c"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [
            ("old", "bower_components/old"),
            ("pkg-a", "a"),
            ("root", ".")
        ]
    );
    for (patterns, expected) in [
        (&["node_modules/evil"][..], &[("root", ".")][..]),
        (&["node_modules/*"], &[("root", ".")]),
        (&[".yarn/x"], &[("root", "."), ("yarn-x", ".yarn/x")]),
        (&[".git/c"], &[("git-c", ".git/c"), ("root", ".")]),
    ] {
        let list: Vec<String> = patterns.iter().map(|p| format!("  - \"{p}\"")).collect();
        write_file(
            dir.path(),
            "pnpm-workspace.yaml",
            &format!("packages:\n{}\n", list.join("\n")),
        );
        assert_eq!(
            names_and_rel_dirs(&discover_ok(dir.path())),
            expected,
            "{patterns:?}"
        );
    }
}

#[test]
fn dot_directories_match_only_a_dotted_segment() {
    let dir = pnpm_dir(&[".tools/*"]);
    write_file(dir.path(), ".tools/a/package.json", &pkg("tool-a"));
    write_file(dir.path(), "a/package.json", &pkg("pkg-a"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("tool-a", ".tools/a")]
    );
    write_file(dir.path(), "pnpm-workspace.yaml", "packages:\n  - \"*\"\n");
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "a")]
    );

    let dir = pnpm_dir(&[".github/actions/*", "examples/.*/*"]);
    write_file(
        dir.path(),
        ".github/actions/x/package.json",
        &pkg("action-x"),
    );
    write_file(
        dir.path(),
        "examples/.hidden/y/package.json",
        &pkg("hidden-y"),
    );
    write_file(dir.path(), "examples/plain/z/package.json", &pkg("plain-z"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [
            ("action-x", ".github/actions/x"),
            ("hidden-y", "examples/.hidden/y")
        ]
    );
    write_file(
        dir.path(),
        "pnpm-workspace.yaml",
        "packages:\n  - \"examples/*/*\"\n",
    );
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("plain-z", "examples/plain/z")]
    );
}

#[test]
fn overlapping_patterns_list_a_directory_once() {
    let dir = pnpm_dir(&["packages/a", "packages/*", "packages/**"]);
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "packages/b/package.json", &pkg("pkg-b"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a"), ("pkg-b", "packages/b")]
    );
}

#[test]
fn expands_brace_alternatives() {
    for npm in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        if npm {
            write_file(
                dir.path(),
                "package.json",
                "{ \"workspaces\": [\"packages/{a,b/c}\"] }\n",
            );
        } else {
            write_file(
                dir.path(),
                "pnpm-workspace.yaml",
                "packages:\n  - \"packages/{a,b/c}\"\n",
            );
        }
        write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
        write_file(dir.path(), "packages/b/package.json", &pkg("pkg-b"));
        write_file(dir.path(), "packages/b/c/package.json", &pkg("pkg-bc"));
        write_file(
            dir.path(),
            "packages/{a,b/c}/package.json",
            &pkg("pkg-braced"),
        );
        assert_eq!(
            names_and_rel_dirs(&discover_ok(dir.path())),
            [("pkg-a", "packages/a"), ("pkg-bc", "packages/b/c")],
            "npm={npm}"
        );
    }
}

#[test]
fn a_leading_parent_pattern_reaches_outside_the_root() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "ws/pnpm-workspace.yaml",
        "packages:\n  - \"app\"\n  - \"../shared\"\n",
    );
    write_file(dir.path(), "ws/package.json", &pkg("root"));
    write_file(dir.path(), "ws/app/package.json", &pkg("app"));
    write_file(dir.path(), "shared/package.json", &pkg("shared"));
    let expected = [("app", "app"), ("root", "."), ("shared", "../shared")];
    let workspace = discover_ok(&dir.path().join("ws"));
    assert_eq!(names_and_rel_dirs(&workspace), expected);
    assert_eq!(
        workspace.member("shared").unwrap().dir(),
        dir.path().join("shared")
    );
    assert_eq!(
        names_and_rel_dirs(&forced(&dir.path().join("ws"))),
        expected
    );
}

#[test]
fn a_parent_run_climbing_past_the_filesystem_root_matches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "ws/pnpm-workspace.yaml",
        &format!("packages:\n  - \"{}*\"\n", "../".repeat(64)),
    );
    write_file(dir.path(), "ws/package.json", &pkg("root"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(&dir.path().join("ws"))),
        [("root", ".")]
    );
}

#[test]
fn the_root_listed_by_a_parent_pattern_keeps_its_dot_spelling() {
    for pm in ["pnpm", "yarn", "npm"] {
        let dir = tempfile::tempdir().unwrap();
        let root_manifest = match pm {
            "pnpm" => {
                write_file(
                    dir.path(),
                    "ws/pnpm-workspace.yaml",
                    "packages:\n  - \"../*\"\n",
                );
                "{ \"name\": \"root\", \"version\": \"1.0.0\" }\n"
            }
            "yarn" => {
                write_file(dir.path(), "ws/yarn.lock", "");
                "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"../*\"] }\n"
            }
            _ => {
                "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"../*\", \".\"] }\n"
            }
        };
        write_file(dir.path(), "ws/package.json", root_manifest);
        write_file(dir.path(), "sib/package.json", &pkg("sib"));
        assert_eq!(
            names_and_rel_dirs(&discover_ok(&dir.path().join("ws"))),
            [("root", "."), ("sib", "../sib")],
            "{pm}"
        );
    }
}

#[test]
fn a_parent_detour_to_a_member_collapses_into_the_direct_spelling() {
    for patterns in [["../*/*", "*"], ["*", "../*/*"]] {
        for yarn in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            if yarn {
                write_file(dir.path(), "ws/yarn.lock", "");
                write_file(
                    dir.path(),
                    "ws/package.json",
                    &format!(
                        "{{ \"workspaces\": [\"{}\", \"{}\"] }}\n",
                        patterns[0], patterns[1]
                    ),
                );
            } else {
                write_file(
                    dir.path(),
                    "ws/pnpm-workspace.yaml",
                    &format!(
                        "packages:\n  - \"{}\"\n  - \"{}\"\n",
                        patterns[0], patterns[1]
                    ),
                );
            }
            write_file(dir.path(), "ws/a/package.json", &pkg("pkg-a"));
            assert_eq!(
                names_and_rel_dirs(&discover_ok(&dir.path().join("ws"))),
                [("pkg-a", "a")],
                "{patterns:?} yarn={yarn}"
            );
        }
    }
}

#[test]
fn a_parent_negation_excludes_a_parent_candidate() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["o", "p"] {
        write_file(dir.path(), &format!("ext/{name}/package.json"), &pkg(name));
    }
    write_file(
        dir.path(),
        "ws/pnpm-workspace.yaml",
        "packages:\n  - \"../ext/*\"\n  - \"!../ext/o\"\n",
    );
    assert_eq!(
        names_and_rel_dirs(&discover_ok(&dir.path().join("ws"))),
        [("p", "../ext/p")]
    );
    write_file(
        dir.path(),
        "ws/pnpm-workspace.yaml",
        "packages:\n  - \"../ext/*\"\n  - \"!**/o\"\n",
    );
    assert_eq!(
        names_and_rel_dirs(&discover_ok(&dir.path().join("ws"))),
        [("o", "../ext/o"), ("p", "../ext/p")]
    );
}

#[test]
fn a_negation_matches_the_spelling_relative_to_the_root() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        write_file(
            dir.path(),
            &format!("root/packages/{name}/package.json"),
            &pkg(name),
        );
    }
    for (patterns, expected) in [
        (
            "packages/*, !packages/b, ../root/packages/*",
            vec![("a", "packages/a")],
        ),
        ("../root/packages/*, !packages/b", vec![("a", "packages/a")]),
        (
            "../root/packages/*, !../root/packages/b",
            vec![("a", "packages/a"), ("b", "packages/b")],
        ),
        ("../root/packages/*, !packages/*", vec![]),
        ("../*/packages/*, !packages/b", vec![("a", "packages/a")]),
    ] {
        let list: Vec<String> = patterns
            .split(", ")
            .map(|pattern| format!("\"{pattern}\""))
            .collect();
        write_file(
            dir.path(),
            "root/pnpm-workspace.yaml",
            &format!("packages: [{}]\n", list.join(", ")),
        );
        assert_eq!(
            names_and_rel_dirs(&discover_ok(&dir.path().join("root"))),
            expected,
            "{patterns}"
        );
    }
}

#[test]
fn a_bare_parent_pattern_makes_the_parent_a_member() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "parent/package.json", &pkg("parent"));
    write_file(
        dir.path(),
        "parent/ws/pnpm-workspace.yaml",
        "packages:\n  - \"..\"\n",
    );
    assert_eq!(
        names_and_rel_dirs(&discover_ok(&dir.path().join("parent/ws"))),
        [("parent", "..")]
    );
}

#[test]
fn mixes_patterns_of_different_ascents() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "g/p/ws/pnpm-workspace.yaml",
        "packages:\n  - \"a\"\n  - \"../x\"\n  - \"../../y\"\n",
    );
    for (rel, name) in [("g/p/ws/a", "a"), ("g/p/x", "x"), ("g/y", "y")] {
        write_file(dir.path(), &format!("{rel}/package.json"), &pkg(name));
    }
    assert_eq!(
        names_and_rel_dirs(&discover_ok(&dir.path().join("g/p/ws"))),
        [("a", "a"), ("x", "../x"), ("y", "../../y")]
    );
}

#[test]
fn rejects_an_invalid_pattern_naming_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": [\"packages/[\"] }\n",
    );
    let err = discover_err(dir.path());
    assert!(err.contains("\"packages/[\""), "{err}");
    assert!(err.contains(&manifest_path(dir.path(), ".")), "{err}");

    for pattern in ["packages/../other", "!/packages/a", "/packages/*"] {
        let dir = pnpm_dir(&["packages/*", pattern]);
        write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
        let err = discover_err(dir.path());
        assert!(err.contains(&format!("{pattern:?}")), "{pattern}: {err}");
        assert!(
            err.contains(&dir.path().join("pnpm-workspace.yaml").display().to_string()),
            "{pattern}: {err}"
        );
    }
}

#[test]
fn plain_absence_is_a_silent_no_match() {
    let dir = pnpm_dir(&["packages/*", "apps/*", "docs/pkg"]);
    write_file(dir.path(), "docs", "not a directory\n");
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    let (workspace, output) = discover_captured(dir.path());
    assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "packages/a")]);
    assert!(warning_lines(&output).is_empty(), "{output}");
}

#[cfg(unix)]
#[test]
fn filesystem_errors_warn_but_do_not_abort() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": [\"packages/**\", \"docs/pkg\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "docs", "not a directory\n");
    std::os::unix::fs::symlink("missing", dir.path().join("packages/broken")).unwrap();
    fs::create_dir_all(dir.path().join("packages/denied")).unwrap();
    fs::create_dir_all(dir.path().join("junk/denied")).unwrap();
    std::os::unix::fs::symlink("loop", dir.path().join("junk/loop")).unwrap();
    for denied in ["packages/denied", "junk/denied"] {
        fs::set_permissions(dir.path().join(denied), fs::Permissions::from_mode(0o000)).unwrap();
    }
    let mut result = None;
    let output = capture_output(|| result = Some(discover(dir.path())));
    for denied in ["packages/denied", "junk/denied"] {
        fs::set_permissions(dir.path().join(denied), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let workspace = result.unwrap().unwrap();
    assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "packages/a")]);
    let warnings = warning_lines(&output);
    assert_eq!(warnings.len(), 2, "{output}");
    let denied = dir.path().join("packages/denied").display().to_string();
    assert!(
        warnings.iter().all(|line| line.contains(&denied)),
        "{output}"
    );
}

#[test]
fn excludes_with_a_double_star_negative_pattern() {
    let dir = pnpm_dir(&["packages/**", "!packages/**/fixtures/**"]);
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(
        dir.path(),
        "packages/a/fixtures/x/package.json",
        &pkg("fx-x"),
    );
    write_file(dir.path(), "packages/fixtures/y/package.json", &pkg("fx-y"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a")]
    );
}

#[test]
fn negation_is_order_independent() {
    let yaml_head = "packages:\n  - \"!packages/b\"\n  - \"packages/a\"\n  - \"packages/b\"\n";
    let yaml_tail = "packages:\n  - \"packages/a\"\n  - \"packages/b\"\n  - \"!packages/b\"\n";
    let json_head = "{ \"workspaces\": [\"!packages/b\", \"packages/a\", \"packages/b\"] }\n";
    let json_tail = "{ \"workspaces\": [\"packages/a\", \"packages/b\", \"!packages/b\"] }\n";
    for (marker, text, lock) in [
        ("pnpm-workspace.yaml", yaml_head, false),
        ("pnpm-workspace.yaml", yaml_tail, false),
        ("package.json", json_head, true),
        ("package.json", json_tail, true),
        ("package.json", json_head, false),
        ("package.json", json_tail, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), marker, text);
        if lock {
            write_file(dir.path(), "yarn.lock", "");
        }
        for name in ["a", "b"] {
            write_file(
                dir.path(),
                &format!("packages/{name}/package.json"),
                &pkg(&format!("pkg-{name}")),
            );
        }
        let (workspace, output) = discover_captured(dir.path());
        assert_eq!(
            names_and_rel_dirs(&workspace),
            [("pkg-a", "packages/a")],
            "{marker} (yarn.lock: {lock}): {text}"
        );
        assert!(
            debug_lines(&output)
                .iter()
                .any(|line| line.contains("packages/b")),
            "{marker} (yarn.lock: {lock}): {output}"
        );
    }
    let dir = pnpm_dir(&["packages/*", "!packages/*"]);
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert_eq!(names(&discover_ok(dir.path())), [] as [&str; 0]);
}

#[test]
fn an_empty_pattern_matches_nothing() {
    let dir = pnpm_dir(&["", "!", "packages/*"]);
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a")]
    );
}

#[cfg(windows)]
#[test]
fn a_drive_prefixed_segment_matches_nothing() {
    let dir = pnpm_dir(&["packages/a", "packages/C:/x", "./C:/x", "C:/x", "C:*"]);
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a")]
    );
    let dir = pnpm_dir(&["packages/*", "*/C:/x"]);
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "packages/a")]
    );
}

#[cfg(windows)]
#[test]
fn a_literal_matches_exactly_on_a_case_insensitive_filesystem() {
    for (pattern, expected) in [
        ("A/*", &[][..]),
        ("*/Lib", &[]),
        ("A/Lib", &[]),
        ("a/lib", &[("pkg", "a/lib")]),
    ] {
        let dir = pnpm_dir(&[pattern]);
        write_file(dir.path(), "a/lib/package.json", &pkg("pkg"));
        assert_eq!(
            names_and_rel_dirs(&discover_ok(dir.path())),
            expected,
            "{pattern}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_drive_like_literal_is_an_ordinary_name_on_unix() {
    let dir = pnpm_dir(&["C:/x"]);
    write_file(dir.path(), "C:/x/package.json", &pkg("pkg-x"));
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-x", "C:/x")]
    );
}

// Symlinks

#[cfg(unix)]
#[test]
fn a_wildcard_enters_a_symlink_one_level_and_a_double_star_never_does() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "target/package.json", &pkg("target"));
    write_file(dir.path(), "target/sub/package.json", &pkg("sub"));
    std::os::unix::fs::symlink("../target", dir.path().join("packages/link")).unwrap();
    std::os::unix::fs::symlink(".", dir.path().join("packages/loop")).unwrap();
    for (patterns, expected) in [
        (&["packages/**"][..], &[("pkg-a", "packages/a")][..]),
        (&["**/sub"], &[("sub", "target/sub")]),
        (
            &["packages/*"],
            &[("pkg-a", "packages/a"), ("target", "packages/link")],
        ),
        (&["packages/*/sub"], &[("sub", "packages/link/sub")]),
        (
            &["packages/*/**"],
            &[
                ("pkg-a", "packages/a"),
                ("sub", "packages/link/sub"),
                ("target", "packages/link"),
            ],
        ),
    ] {
        let list: Vec<String> = patterns.iter().map(|p| format!("  - \"{p}\"")).collect();
        write_file(
            dir.path(),
            "pnpm-workspace.yaml",
            &format!("packages:\n{}\n", list.join("\n")),
        );
        let (workspace, output) = discover_captured(dir.path());
        assert_eq!(names_and_rel_dirs(&workspace), expected, "{patterns:?}");
        if patterns == ["packages/**"] {
            assert!(
                debug_lines(&output)
                    .iter()
                    .any(|line| line.contains("packages/link")),
                "{output}"
            );
        }
    }

    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "external/pkg-b/package.json", &pkg("pkg-b"));
    std::os::unix::fs::symlink("../external/pkg-b", dir.path().join("packages/b")).unwrap();
    assert_eq!(
        names_and_dirs(&discover_ok(dir.path())),
        [
            ("pkg-a", dir.path().join("packages/a").as_path()),
            ("pkg-b", dir.path().join("packages/b").as_path()),
        ]
    );

    let dir = pnpm_dir(&["packages/*"]);
    write_file(dir.path(), "packages/package.json", &pkg("packages"));
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    std::os::unix::fs::symlink(".", dir.path().join("packages/loop")).unwrap();
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("packages", "packages/loop"), ("pkg-a", "packages/a")]
    );
    write_file(
        dir.path(),
        "pnpm-workspace.yaml",
        "packages:\n  - \"packages/**\"\n",
    );
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("packages", "packages"), ("pkg-a", "packages/a")]
    );
}

#[cfg(unix)]
#[test]
fn a_literal_segment_follows_a_symlink() {
    let dir = pnpm_dir(&["link/*"]);
    write_file(dir.path(), "real/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "real/package.json", &pkg("real"));
    write_file(dir.path(), "real/a/sub/package.json", &pkg("sub"));
    std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
    let workspace = discover_ok(dir.path());
    assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "link/a")]);
    assert_eq!(
        workspace.member("pkg-a").unwrap().dir(),
        dir.path().join("link/a")
    );
    write_file(
        dir.path(),
        "pnpm-workspace.yaml",
        "packages:\n  - \"link/**\"\n",
    );
    assert_eq!(
        names_and_rel_dirs(&discover_ok(dir.path())),
        [("pkg-a", "link/a"), ("real", "link"), ("sub", "link/a/sub")]
    );
}

// Forced roots and listed packages

#[test]
fn a_forced_root_reads_only_its_own_markers() {
    let dir = pnpm_dir(&["packages/*"]);
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "packages/b/package.json", &pkg("pkg-b"));
    write_file(
        dir.path(),
        "packages/inner/package.json",
        "{ \"name\": \"inner\", \"version\": \"1.0.0\", \"workspaces\": [\"libs/*\"] }\n",
    );
    write_file(
        dir.path(),
        "packages/inner/libs/x/package.json",
        &pkg("pkg-x"),
    );

    let root = dir.path().join("packages/a");
    let workspace = forced(&root);
    assert_eq!(workspace.root(), root);
    assert_eq!(workspace.changeset_dir(), root.join(".changeset"));
    assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", ".")]);

    let root = dir.path().join("packages/inner");
    let workspace = forced(&root);
    assert_eq!(workspace.root(), root);
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("inner", "."), ("pkg-x", "libs/x")]
    );

    let workspace = forced(dir.path());
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [
            ("inner", "packages/inner"),
            ("pkg-a", "packages/a"),
            ("pkg-b", "packages/b")
        ]
    );
}

#[test]
fn a_forced_root_without_a_manifest_has_no_members() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    let mut result = None;
    let output = capture_output(|| result = Some(forced(dir.path())));
    let workspace = result.unwrap();
    assert_eq!(workspace.root(), dir.path());
    assert_eq!(names(&workspace), [] as [&str; 0]);
    assert!(warning_lines(&output).is_empty(), "{output}");

    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "yarn.lock", "");
    assert_eq!(names(&forced(dir.path())), [] as [&str; 0]);
}

#[test]
fn listed_packages_replace_the_enumeration() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "ws/pnpm-workspace.yaml",
        "packages:\n  - \"packages/*\"\n",
    );
    write_file(dir.path(), "ws/package.json", &pkg("root"));
    write_file(dir.path(), "ws/packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "ws/other/c/package.json", &pkg("pkg-c"));
    write_file(dir.path(), "shared/package.json", &pkg("shared"));
    let root = dir.path().join("ws");
    let workspace = load_listed(&root, &["other/c", ".", "../shared"]).unwrap();
    assert_eq!(
        names_and_dirs(&workspace),
        [
            ("pkg-c", root.join("other/c").as_path()),
            ("root", root.as_path()),
            ("shared", dir.path().join("shared").as_path()),
        ]
    );
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("pkg-c", "other/c"), ("root", "."), ("shared", "../shared")]
    );
    assert_eq!(names(&load_listed(&root, &[]).unwrap()), [] as [&str; 0]);

    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"workspaces\": [\"packages/[\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert!(discover(dir.path()).is_err());
    assert_eq!(
        names_and_rel_dirs(&load_listed(dir.path(), &["packages/a"]).unwrap()),
        [("pkg-a", "packages/a")]
    );

    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    assert_eq!(
        names_and_rel_dirs(&load_listed(dir.path(), &["packages/a"]).unwrap()),
        [("pkg-a", "packages/a")]
    );
}

#[test]
fn listed_packages_win_over_the_reroot_enumeration() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "package.json",
        "{ \"name\": \"root\", \"version\": \"1.0.0\", \"workspaces\": [\"packages/*\"] }\n",
    );
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "packages/b/package.json", &pkg("pkg-b"));
    let root = Root::find(&dir.path().join("packages/a")).unwrap();
    assert_eq!(root.dir(), dir.path());
    let packages = vec!["packages/b".to_owned()];
    let workspace = Workspace::load(root, Some(&packages)).unwrap();
    assert_eq!(names_and_rel_dirs(&workspace), [("pkg-b", "packages/b")]);
}

#[test]
fn a_listed_package_without_a_readable_manifest_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "packages/broken/package.json", "{\n");
    write_file(dir.path(), "packages/file", "not a directory\n");
    for rel in ["packages/missing", "packages/broken", "packages/file"] {
        let err = load_listed_err(dir.path(), &["packages/a", rel]);
        assert!(
            err.contains(&manifest_path(dir.path(), rel)),
            "{rel}: {err}"
        );
    }
}

#[test]
fn listed_packages_take_the_member_qualification() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    write_file(
        dir.path(),
        "packages/b/package.json",
        "{ \"name\": \"pkg-b\", \"private\": true }\n",
    );
    write_file(dir.path(), "packages/c/package.json", &pkg("dup"));
    write_file(
        dir.path(),
        "packages/d/package.json",
        "{ \"name\": \"dup\", \"version\": \"2.0.0\" }\n",
    );
    let workspace = load_listed(
        dir.path(),
        &["packages/d", "packages/c", "packages/b", "packages/a"],
    )
    .unwrap();
    assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "packages/a")]);
}

#[cfg(unix)]
#[test]
fn listed_aliases_of_one_directory_collapse_into_one_member() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "real/a/package.json", &pkg("pkg-a"));
    std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
    let workspace = load_listed(dir.path(), &["real/a", "link/a"]).unwrap();
    assert_eq!(
        names_and_dirs(&workspace),
        [("pkg-a", dir.path().join("link/a").as_path())]
    );
}

#[cfg(windows)]
#[test]
fn listed_packages_keep_their_spelling_on_a_case_insensitive_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "packages/a/package.json", &pkg("pkg-a"));
    let workspace = load_listed(dir.path(), &["Packages/A"]).unwrap();
    assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "Packages/A")]);
    let workspace = load_listed(dir.path(), &["packages/a", "Packages/A"]).unwrap();
    assert_eq!(names_and_rel_dirs(&workspace), [("pkg-a", "Packages/A")]);
}

#[test]
fn listed_packages_are_spelled_in_the_normal_form() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "root/packages/a/package.json", &pkg("pkg-a"));
    write_file(dir.path(), "root/b/package.json", &pkg("pkg-b"));
    write_file(dir.path(), "root/package.json", &pkg("root"));
    let root = dir.path().join("root");
    let workspace = load_listed(
        &root,
        &[
            "./../root/packages/./a",
            "a/../b",
            "./",
            "packages//a/",
            "packages/b/../a",
            "packages/..",
            ".",
        ],
    )
    .unwrap();
    assert_eq!(
        names_and_rel_dirs(&workspace),
        [("pkg-a", "packages/a"), ("pkg-b", "b"), ("root", ".")]
    );
    assert_eq!(
        workspace.member("pkg-a").unwrap().dir(),
        root.join("packages/a")
    );
    assert_eq!(workspace.member("pkg-b").unwrap().dir(), root.join("b"));
    assert_eq!(workspace.member("root").unwrap().dir(), root);
}

#[test]
fn listed_packages_reject_escaping_and_drive_prefixed_entries() {
    let dir = tempfile::tempdir().unwrap();
    let entry = format!("{}x", "../".repeat(64));
    let err = load_listed_err(dir.path(), &[&entry]);
    assert!(err.contains(&format!("{entry:?}")), "{err}");
    #[cfg(windows)]
    for entry in ["../C:/x", "C:/x", "C:x", "packages/C:x"] {
        let err = load_listed_err(dir.path(), &[entry]);
        assert!(err.contains(&format!("{entry:?}")), "{err}");
    }
}

#[cfg(unix)]
#[test]
fn a_listed_drive_like_segment_is_an_ordinary_name_on_unix() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "C:/x/package.json", &pkg("pkg-x"));
    let workspace = load_listed(&dir.path().join("root"), &["../C:/x"]).unwrap();
    assert_eq!(names_and_rel_dirs(&workspace), [("pkg-x", "../C:/x")]);
}

#[test]
fn rejects_a_root_that_is_missing_or_a_file() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "file", "");
    fs::create_dir_all(dir.path().join("packages/a")).unwrap();
    let err = resolve_root(&dir.path().join("missing")).unwrap_err();
    assert_eq!(
        err.downcast_ref::<io::Error>().map(io::Error::kind),
        Some(io::ErrorKind::NotFound),
        "{err:#}"
    );
    assert!(resolve_root(&dir.path().join("file")).is_err());
    let canonical = dunce::canonicalize(dir.path()).unwrap();
    assert_eq!(
        resolve_root(&dir.path().join("packages/a")).unwrap(),
        canonical.join("packages/a")
    );
    #[cfg(windows)]
    assert_eq!(
        resolve_root(&dir.path().join("packages\\a")).unwrap(),
        canonical.join("packages/a")
    );
}

#[cfg(unix)]
#[test]
fn resolves_a_symlinked_root_to_its_physical_directory() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("real")).unwrap();
    std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
    assert_eq!(
        resolve_root(&dir.path().join("link")).unwrap(),
        dir.path().canonicalize().unwrap().join("real")
    );
}
