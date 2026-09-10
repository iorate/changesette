use std::{
    fs,
    path::{Path, PathBuf},
};

use changesette::package_json::PackageJson;

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
        .set_version(&semver::Version::new(10, 1, 0))
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
fn rejects_an_invalid_manifest() {
    for case in [
        "no-package-json",
        "name-missing",
        "version-not-string",
        "version-invalid-semver",
    ] {
        assert!(PackageJson::load(&fixture(case)).is_err(), "{case}");
    }
    let mut package_json = PackageJson::load(&fixture("version-missing")).unwrap();
    assert!(
        package_json
            .set_version(&semver::Version::new(10, 1, 0))
            .is_err()
    );
}
