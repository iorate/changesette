use semver::{Prerelease, Version};

/// A semver bump type, ordered so that `max` picks the widest one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Bump {
    Patch,
    Minor,
    Major,
}

impl Bump {
    /// The lowercase name used in changeset frontmatter and CLI values.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Bump::Patch => "patch",
            Bump::Minor => "minor",
            Bump::Major => "major",
        }
    }
}

/// Returns `current` incremented by `bump`, following node-semver's `inc`
/// (used by changesets): a pre-release graduates to the release it precedes
/// when that release satisfies the bump, and pre-release and build metadata
/// are always cleared.
pub(crate) fn next_version(current: &Version, bump: Bump) -> Version {
    let pre = !current.pre.is_empty();
    match bump {
        Bump::Major if pre && current.minor == 0 && current.patch == 0 => {
            Version::new(current.major, 0, 0)
        }
        Bump::Major => Version::new(current.major + 1, 0, 0),
        Bump::Minor if pre && current.patch == 0 => Version::new(current.major, current.minor, 0),
        Bump::Minor => Version::new(current.major, current.minor + 1, 0),
        Bump::Patch if pre => Version::new(current.major, current.minor, current.patch),
        Bump::Patch => Version::new(current.major, current.minor, current.patch + 1),
    }
}

/// Returns `next_version(current, bump)` with the pre-release `-{tag}.{n}`
/// attached, where `n` restarts at 0 unless `current`'s pre-release is exactly
/// `{tag}.{m}` with a numeric `m`, in which case it is `m + 1`. `tag` must
/// have passed `pre::validate_tag`.
pub(crate) fn next_pre_version(current: &Version, bump: Bump, tag: &str) -> Version {
    // Counting on the tag, rather than on the second pre-release identifier,
    // keeps a dotted tag (`beta.2`) counting and restarts on a tag switch.
    let counter = current
        .pre
        .as_str()
        .strip_prefix(&format!("{tag}."))
        .and_then(|rest| rest.parse::<u64>().ok())
        .map_or(0, |number| number + 1);
    let mut version = next_version(current, bump);
    version.pre =
        Prerelease::new(&format!("{tag}.{counter}")).expect("a validated tag stays valid");
    version
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next(current: &str, bump: Bump) -> String {
        next_version(&current.parse().unwrap(), bump).to_string()
    }

    fn next_pre(current: &str, bump: Bump, tag: &str) -> String {
        next_pre_version(&current.parse().unwrap(), bump, tag).to_string()
    }

    #[test]
    fn bump_orders_patch_below_minor_below_major() {
        assert!(Bump::Patch < Bump::Minor);
        assert!(Bump::Minor < Bump::Major);
    }

    #[test]
    fn max_picks_the_highest_bump() {
        assert_eq!(
            [Bump::Patch, Bump::Major, Bump::Minor].iter().max(),
            Some(&Bump::Major)
        );
        assert_eq!([Bump::Patch, Bump::Minor].iter().max(), Some(&Bump::Minor));
        assert_eq!([Bump::Patch].iter().max(), Some(&Bump::Patch));
        assert_eq!([Bump::Patch; 0].iter().max(), None);
    }

    #[test]
    fn increments_literally() {
        assert_eq!(next("1.2.3", Bump::Major), "2.0.0");
        assert_eq!(next("1.2.3", Bump::Minor), "1.3.0");
        assert_eq!(next("1.2.3", Bump::Patch), "1.2.4");
    }

    #[test]
    fn increments_literally_on_zero_major() {
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
    }

    #[test]
    fn pre_version_starts_at_zero() {
        assert_eq!(next_pre("1.0.0", Bump::Minor, "beta"), "1.1.0-beta.0");
        assert_eq!(next_pre("1.0.0", Bump::Major, "beta"), "2.0.0-beta.0");
        assert_eq!(next_pre("1.0.0", Bump::Patch, "beta"), "1.0.1-beta.0");
    }

    #[test]
    fn pre_version_increments_the_counter() {
        assert_eq!(
            next_pre("1.1.0-beta.0", Bump::Patch, "beta"),
            "1.1.0-beta.1"
        );
    }

    #[test]
    fn pre_version_keeps_counting_across_base_bumps() {
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
    fn pre_version_handles_a_dotted_tag() {
        assert_eq!(
            next_pre("1.1.0-beta.2.0", Bump::Patch, "beta.2"),
            "1.1.0-beta.2.1"
        );
    }

    #[test]
    fn pre_version_restarts_on_a_tag_switch() {
        assert_eq!(
            next_pre("1.1.0-alpha.3", Bump::Patch, "beta"),
            "1.1.0-beta.0"
        );
    }

    #[test]
    fn pre_version_restarts_on_a_non_numeric_counter() {
        assert_eq!(
            next_pre("1.0.0-alpha.beta", Bump::Patch, "alpha"),
            "1.0.0-alpha.0"
        );
    }

    #[test]
    fn pre_version_with_a_numeric_tag() {
        assert_eq!(next_pre("1.0.0", Bump::Patch, "1"), "1.0.1-1.0");
        assert_eq!(next_pre("1.0.1-1.0", Bump::Patch, "1"), "1.0.1-1.1");
    }

    #[test]
    fn pre_version_clears_build_metadata() {
        assert_eq!(next_pre("1.2.3+abc", Bump::Minor, "beta"), "1.3.0-beta.0");
    }
}
