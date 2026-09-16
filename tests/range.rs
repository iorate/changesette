use std::path::Path;

use changesette::{
    bump::Bump,
    config::UpdateInternalDependencies,
    dependency::{Spec, parse_spec},
    range::{Target, Update, update},
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
) -> Option<Update> {
    update(&spec(spec_text), &Target::Release(version(new)), bump, min)
}

fn snapshot(spec_text: &str) -> Option<Update> {
    update(
        &spec(spec_text),
        &Target::Snapshot(version(SNAPSHOT)),
        Bump::Patch,
        UpdateInternalDependencies::Patch,
    )
}

fn explicit(text: &str) -> Update {
    Update::Explicit(text.to_owned())
}

fn patch(spec_text: &str, new: &str) -> Option<Update> {
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
            patch(spec_text, new),
            Some(explicit(expected)),
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
            patch(spec_text, new),
            Some(explicit(expected)),
            "{spec_text} {new}"
        );
    }
}

#[test]
fn pins_any_to_a_prerelease_and_keeps_it_otherwise() {
    assert_eq!(
        patch("^1.0.0", "1.1.0-beta.0"),
        Some(explicit("^1.1.0-beta.0"))
    );
    assert_eq!(patch("*", "1.1.0-beta.0"), Some(explicit("1.1.0-beta.0")));
    assert_eq!(patch("*", "1.0.1"), Some(Update::Implicit));
}

#[test]
fn pins_the_snapshot_version() {
    for (spec_text, expected) in [
        ("^1.0.0", SNAPSHOT.to_owned()),
        ("workspace:^1.0.0", format!("workspace:{SNAPSHOT}")),
        ("*", SNAPSHOT.to_owned()),
    ] {
        assert_eq!(
            snapshot(spec_text),
            Some(explicit(&expected)),
            "{spec_text}"
        );
    }
}

#[test]
fn keeps_aliases_and_paths_and_ignores_external_specs() {
    for (spec_text, expected) in [
        ("workspace:*", Some(Update::Implicit)),
        ("workspace:^", Some(Update::Implicit)),
        ("workspace:~", Some(Update::Implicit)),
        ("workspace:../b", Some(Update::Implicit)),
        ("file:../b", None),
        ("latest", None),
    ] {
        assert_eq!(patch(spec_text, "2.0.0"), expected, "{spec_text}");
        assert_eq!(snapshot(spec_text), expected, "{spec_text}");
    }
}

#[test]
fn the_minor_setting_skips_patch_raises_within_the_range() {
    for (spec_text, new, bump, expected) in [
        ("^1.0.0", "1.0.1", Bump::Patch, None),
        ("^1.0.0", "1.1.0", Bump::Minor, Some(explicit("^1.1.0"))),
        ("1.0.0", "1.0.1", Bump::Patch, Some(explicit("1.0.1"))),
        ("~1.0.0", "1.1.0", Bump::Minor, Some(explicit("~1.1.0"))),
        ("*", "1.0.1", Bump::Patch, None),
        (
            "workspace:^1.0.0",
            "1.0.1",
            Bump::Patch,
            Some(Update::Implicit),
        ),
    ] {
        assert_eq!(
            release(spec_text, new, bump, UpdateInternalDependencies::Minor),
            expected,
            "{spec_text} {new}"
        );
    }
}
