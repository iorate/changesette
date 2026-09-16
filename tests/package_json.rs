use std::{
    fs,
    path::{Path, PathBuf},
};

use changesette::{package_json::PackageJson, workspace::DependencyField};

fn fixture(case: &str) -> PathBuf {
    Path::new("tests/fixtures/package-json").join(case)
}

fn rewrite(case: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    fs::copy(
        fixture(case).join("package.json"),
        dir.path().join("package.json"),
    )
    .unwrap();
    let mut package_json = PackageJson::load(dir.path()).unwrap();
    package_json
        .set_version(&nodejs_semver::Version::from((10, 1, 0)))
        .unwrap();
    fs::write(package_json.path(), package_json.text()).unwrap();
    fs::read_to_string(dir.path().join("package.json")).unwrap()
}

#[test]
fn rewrites_a_two_space_indented_file() {
    assert_eq!(
        rewrite("two-space"),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"10.1.0\",\n  \"private\": true,\n  \"scripts\": {\n    \"build\": \"vite build\"\n  }\n}\n"
    );
}

#[test]
fn rewrites_a_four_space_indented_file_without_a_final_newline() {
    assert_eq!(
        rewrite("four-space-no-final-newline"),
        "{\n    \"name\": \"ublacklist\",\n    \"version\": \"10.1.0\",\n    \"private\": true\n}"
    );
}

#[test]
fn rewrites_a_tab_indented_file() {
    assert_eq!(
        rewrite("tabs"),
        "{\n\t\"name\": \"ublacklist\",\n\t\"version\": \"10.1.0\",\n\t\"private\": true\n}\n"
    );
}

#[test]
fn leaves_nested_version_keys_untouched() {
    assert_eq!(
        rewrite("scripts-version-key"),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"10.1.0\",\n  \"scripts\": {\n    \"version\": \"echo version\",\n    \"postversion\": \"git push\"\n  },\n  \"config\": {\n    \"version\": \"not-touched\"\n  }\n}\n"
    );
}

#[test]
fn rewrites_a_dependency_range_of_the_given_field_only() {
    let dir = tempfile::tempdir().unwrap();
    fs::copy(
        fixture("dependencies").join("package.json"),
        dir.path().join("package.json"),
    )
    .unwrap();
    let mut package_json = PackageJson::load(dir.path()).unwrap();
    package_json
        .set_dependency(DependencyField::Dependencies, "pkg-a", "^1.0.1")
        .unwrap();
    assert_eq!(
        package_json.text(),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"10.0.2\",\n  \"dependencies\": {\n    \"pkg-a\": \"^1.0.1\",\n    \"pkg-b\": \"workspace:*\"\n  },\n  \"devDependencies\": {\n    \"pkg-a\": \"^1.0.0\"\n  }\n}\n"
    );
    assert!(
        package_json
            .set_dependency(DependencyField::PeerDependencies, "pkg-a", "^1.0.1")
            .is_err()
    );
    assert!(
        package_json
            .set_dependency(DependencyField::Dependencies, "pkg-c", "^1.0.1")
            .is_err()
    );
}

#[test]
fn loads_a_manifest_without_a_name() {
    assert!(PackageJson::load(&fixture("name-missing")).is_ok());
}

#[test]
fn rejects_an_invalid_manifest() {
    assert!(PackageJson::load(&fixture("no-package-json")).is_err());
    for case in ["version-missing", "version-not-string"] {
        let mut package_json = PackageJson::load(&fixture(case)).unwrap();
        assert!(
            package_json
                .set_version(&nodejs_semver::Version::from((10, 1, 0)))
                .is_err(),
            "{case}"
        );
    }
}
