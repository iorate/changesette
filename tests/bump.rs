use changesette::bump::{
    Bump, Prerelease, next_pre_version, next_pre_version_with, next_version, parse_version,
    pre_counter,
};

fn next(current: &str, bump: Bump) -> String {
    next_version(&current.parse().unwrap(), bump).to_string()
}

fn next_pre(current: &str, bump: Bump, tag: &str) -> String {
    next_pre_version(&current.parse().unwrap(), bump, &pre(tag)).to_string()
}

fn counter(current: &str, tag: &str) -> u64 {
    pre_counter(&current.parse().unwrap(), &pre(tag))
}

fn pre(text: &str) -> Prerelease {
    Prerelease::new(text).unwrap()
}

#[test]
fn next_version_increments_literally() {
    assert_eq!(next("1.2.3", Bump::Major), "2.0.0");
    assert_eq!(next("1.2.3", Bump::Minor), "1.3.0");
    assert_eq!(next("1.2.3", Bump::Patch), "1.2.4");
    assert_eq!(next("0.5.2", Bump::Major), "1.0.0");
    assert_eq!(next("0.5.2", Bump::Minor), "0.6.0");
    assert_eq!(next("0.5.2", Bump::Patch), "0.5.3");
    assert_eq!(next("0.0.1", Bump::Major), "1.0.0");
}

#[test]
fn graduates_a_pre_release_that_satisfies_the_bump() {
    assert_eq!(next("2.0.0-beta.1", Bump::Major), "2.0.0");
    assert_eq!(next("1.2.0-rc.1", Bump::Minor), "1.2.0");
    assert_eq!(next("1.2.3-rc.1", Bump::Patch), "1.2.3");
}

#[test]
fn increments_past_a_pre_release_that_does_not_satisfy_the_bump() {
    assert_eq!(next("2.1.0-beta.1", Bump::Major), "3.0.0");
    assert_eq!(next("2.0.1-beta.1", Bump::Major), "3.0.0");
    assert_eq!(next("1.2.3-rc.1", Bump::Minor), "1.3.0");
}

#[test]
fn clears_build_metadata() {
    assert_eq!(next("1.2.3+abc", Bump::Major), "2.0.0");
    assert_eq!(next("1.2.3+abc", Bump::Minor), "1.3.0");
    assert_eq!(next("1.2.3+abc", Bump::Patch), "1.2.4");
    assert_eq!(next("1.2.3-rc.1+abc", Bump::Patch), "1.2.3");
    assert_eq!(next_pre("1.2.3+abc", Bump::Minor, "beta"), "1.3.0-beta.0");
}

#[test]
fn next_pre_version_counts_on_the_tag() {
    assert_eq!(next_pre("1.0.0", Bump::Minor, "beta"), "1.1.0-beta.0");
    assert_eq!(next_pre("1.0.0", Bump::Major, "beta"), "2.0.0-beta.0");
    assert_eq!(next_pre("1.0.0", Bump::Patch, "beta"), "1.0.1-beta.0");
    assert_eq!(
        next_pre("1.1.0-beta.0", Bump::Patch, "beta"),
        "1.1.0-beta.1"
    );
    assert_eq!(
        next_pre("1.1.0-beta.1", Bump::Major, "beta"),
        "2.0.0-beta.2"
    );
    assert_eq!(
        next_pre("1.0.1-beta.0", Bump::Minor, "beta"),
        "1.1.0-beta.1"
    );
}

#[test]
fn next_pre_version_restarts_when_the_tag_does_not_match() {
    assert_eq!(
        next_pre("1.1.0-beta.2.0", Bump::Patch, "beta.2"),
        "1.1.0-beta.2.1"
    );
    assert_eq!(
        next_pre("1.1.0-alpha.3", Bump::Patch, "beta"),
        "1.1.0-beta.0"
    );
    assert_eq!(
        next_pre("1.0.0-alpha.beta", Bump::Patch, "alpha"),
        "1.0.0-alpha.0"
    );
    assert_eq!(next_pre("1.0.0", Bump::Patch, "1"), "1.0.1-1.0");
    assert_eq!(next_pre("1.0.1-1.0", Bump::Patch, "1"), "1.0.1-1.1");
}

#[test]
fn pre_counter_feeds_next_pre_version_with() {
    assert_eq!(counter("1.1.0-beta.0", "beta"), 1);
    assert_eq!(counter("1.1.0-beta.2.3", "beta.2"), 4);
    assert_eq!(counter("1.0.0", "beta"), 0);
    assert_eq!(counter("1.1.0-alpha.3", "beta"), 0);
    assert_eq!(counter("1.0.0-alpha.beta", "alpha"), 0);
    assert_eq!(
        next_pre_version_with(
            &"1.1.0-beta.0".parse().unwrap(),
            Bump::Patch,
            &pre("beta"),
            5
        )
        .to_string(),
        "1.1.0-beta.5"
    );
}

#[test]
#[should_panic(expected = "version number overflow")]
fn panics_on_a_version_component_overflow() {
    let _ = next_version(&nodejs_semver::Version::from((u64::MAX, 0, 0)), Bump::Major);
}

#[test]
#[should_panic(expected = "version number overflow")]
fn pre_counter_panics_on_a_counter_overflow() {
    counter("1.0.0-beta.18446744073709551615", "beta");
}

#[test]
fn parse_version_accepts_strict_semver() {
    for text in [
        "0.0.0",
        "10.20.30",
        "1.0.0-beta.2",
        "1.0.0-0",
        "1.0.0+build.1",
        "1.0.0-rc-1+x-y",
    ] {
        let version = parse_version(text).unwrap_or_else(|_| panic!("{text:?} should be accepted"));
        assert_eq!(version.to_string(), text);
    }
}

#[test]
fn parse_version_accepts_a_v_prefix_and_surrounding_whitespace() {
    for text in ["v1.0.0", " 1.0.0", "1.0.0\t", "\n v1.0.0-beta.1+b \n"] {
        let version = parse_version(text).unwrap_or_else(|_| panic!("{text:?} should be accepted"));
        assert_eq!(version.to_string(), text.trim().trim_start_matches('v'));
    }
}

#[test]
fn parse_version_rejects_loose_input() {
    for text in [
        "",
        "1.0",
        "V1.0.0",
        "=v1.0.0",
        "v 1.0.0",
        "01.0.0",
        "1.0.0 garbage",
        "1.0.0-",
        "1.0.0-.beta",
        "1.0.0-beta..2",
        "1.0.0-beta.01",
        "1.0.0-b_1",
        "1.0.0+",
        "１.0.0",
    ] {
        assert!(parse_version(text).is_err(), "{text:?} should be rejected");
    }
}

#[test]
fn prerelease_accepts_dotted_and_numeric_identifiers() {
    for text in ["beta", "beta.2", "1", "rc-0", "0", "01a", "-"] {
        let pre = Prerelease::new(text).unwrap_or_else(|| panic!("{text:?} should be accepted"));
        assert_eq!(pre.as_str(), text);
        assert_eq!(pre.to_string(), text);
    }
}

#[test]
fn prerelease_rejects_invalid_identifiers() {
    for text in [
        "",
        " ",
        "beta 2",
        "beta_2",
        "ベータ",
        "01",
        "beta.",
        ".beta",
        "beta..2",
    ] {
        assert!(
            Prerelease::new(text).is_none(),
            "{text:?} should be rejected"
        );
    }
}

#[test]
fn prerelease_with_counter_appends_an_identifier() {
    assert_eq!(pre("beta").with_counter(0).as_str(), "beta.0");
    assert_eq!(pre("beta.2").with_counter(10).as_str(), "beta.2.10");
}
