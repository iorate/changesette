use changesette::bump::{Bump, next_pre_version, next_pre_version_with, next_version, pre_counter};

fn next(current: &str, bump: Bump) -> String {
    next_version(&current.parse().unwrap(), bump).to_string()
}

fn next_pre(current: &str, bump: Bump, tag: &str) -> String {
    next_pre_version(&current.parse().unwrap(), bump, tag).to_string()
}

fn counter(current: &str, tag: &str) -> u64 {
    pre_counter(&current.parse().unwrap(), tag)
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
        next_pre_version_with(&"1.1.0-beta.0".parse().unwrap(), Bump::Patch, "beta", 5).to_string(),
        "1.1.0-beta.5"
    );
}

#[test]
#[should_panic(expected = "version number overflow")]
fn panics_on_a_version_component_overflow() {
    next("18446744073709551615.0.0", Bump::Major);
}

#[test]
#[should_panic(expected = "version number overflow")]
fn pre_counter_panics_on_a_counter_overflow() {
    counter("1.0.0-beta.18446744073709551615", "beta");
}
