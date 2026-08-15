use semver::Version;

/// A semver bump type, ordered so that `max` picks the widest one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn next(current: &str, bump: Bump) -> String {
        next_version(&current.parse().unwrap(), bump).to_string()
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
}
