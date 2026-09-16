use std::path::Path;

use changesette::{
    bump::Bump,
    config::UpdateInternalDependencies,
    dependency::{Spec, parse_spec},
    range::{Target, rewrite},
    workspace::{RelDir, rel_dir_between},
};
use nodejs_semver::Version;

const SNAPSHOT: &str = "0.0.0-canary-20250822000000";

fn rel_dir(rel: &str) -> RelDir {
    rel_dir_between(Path::new("/root"), &Path::new("/root").join(rel))
}

fn spec(text: &str) -> Spec {
    parse_spec(text, &rel_dir("packages/a"), &rel_dir("packages/b"), false)
}

fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn release(
    spec_text: &str,
    new: &str,
    bump: Bump,
    min: UpdateInternalDependencies,
) -> Option<String> {
    rewrite(&spec(spec_text), &Target::Release(version(new)), bump, min)
}

fn patch(spec_text: &str, new: &str) -> Option<String> {
    release(
        spec_text,
        new,
        Bump::Patch,
        UpdateInternalDependencies::Patch,
    )
}

#[test]
fn raises_the_lower_bound_within_the_range() {
    for (spec_text, new, expected) in [
        ("^1.0.0", "1.0.1", "^1.0.1"),
        ("1.x", "1.3.0", "^1.3.0"),
        (">=1.0.0 <2.0.0", "1.5.0", ">=1.5.0 <2.0.0"),
        ("^1 || ^2", "2.1.0", "^2.1.0"),
        ("^1 || ^2", "1.5.0", ">=1.5.0 <2.0.0-0||>=2.0.0 <3.0.0-0"),
        ("1.0.0 - 2.0.0", "1.5.0", ">=1.5.0 <=2.0.0"),
        ("workspace:^1.0.0", "1.0.1", "workspace:^1.0.1"),
    ] {
        assert_eq!(
            patch(spec_text, new).as_deref(),
            Some(expected),
            "{spec_text} {new}"
        );
    }
}

#[test]
fn keeps_the_shape_when_the_new_version_leaves_the_range() {
    for (spec_text, new, expected) in [
        ("^1.0.0", "2.0.0", "^2.0.0"),
        ("~1.0.0", "1.1.0", "~1.1.0"),
        ("1.0.0", "1.0.1", "1.0.1"),
        (">=1.0.0 <2.0.0", "2.0.0", ">=2.0.0 <3.0.0-0"),
        ("^1.2.0", "2.0.0", "^2.0.0"),
    ] {
        assert_eq!(
            patch(spec_text, new).as_deref(),
            Some(expected),
            "{spec_text} {new}"
        );
    }
}

#[test]
fn pins_any_only_to_a_prerelease() {
    assert_eq!(
        patch("^1.0.0", "1.1.0-beta.0").as_deref(),
        Some("^1.1.0-beta.0")
    );
    assert_eq!(patch("*", "1.1.0-beta.0").as_deref(), Some("1.1.0-beta.0"));
    assert_eq!(patch("*", "1.0.1"), None);
}

#[test]
fn pins_the_snapshot_version() {
    for (spec_text, expected) in [
        ("^1.0.0", SNAPSHOT.to_owned()),
        ("workspace:^1.0.0", format!("workspace:{SNAPSHOT}")),
        ("*", SNAPSHOT.to_owned()),
    ] {
        assert_eq!(
            rewrite(
                &spec(spec_text),
                &Target::Snapshot(version(SNAPSHOT)),
                Bump::Patch,
                UpdateInternalDependencies::Patch
            ),
            Some(expected),
            "{spec_text}"
        );
    }
}

#[test]
fn leaves_aliases_paths_and_external_specs_alone() {
    for spec_text in [
        "workspace:*",
        "workspace:^",
        "workspace:~",
        "workspace:../b",
        "file:../b",
        "latest",
    ] {
        assert_eq!(patch(spec_text, "2.0.0"), None, "{spec_text}");
        assert_eq!(
            rewrite(
                &spec(spec_text),
                &Target::Snapshot(version(SNAPSHOT)),
                Bump::Patch,
                UpdateInternalDependencies::Patch
            ),
            None,
            "{spec_text}"
        );
    }
}

#[test]
fn the_minor_setting_skips_patch_raises_within_the_range() {
    for (spec_text, new, bump, expected) in [
        ("^1.0.0", "1.0.1", Bump::Patch, None),
        ("^1.0.0", "1.1.0", Bump::Minor, Some("^1.1.0")),
        ("1.0.0", "1.0.1", Bump::Patch, Some("1.0.1")),
        ("~1.0.0", "1.1.0", Bump::Minor, Some("~1.1.0")),
    ] {
        assert_eq!(
            release(spec_text, new, bump, UpdateInternalDependencies::Minor).as_deref(),
            expected,
            "{spec_text} {new}"
        );
    }
}
