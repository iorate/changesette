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

/// Returns `current` incremented by `bump`, clearing any pre-release and
/// build metadata.
pub(crate) fn next_version(current: &Version, bump: Bump) -> Version {
    match bump {
        Bump::Major => Version::new(current.major + 1, 0, 0),
        Bump::Minor => Version::new(current.major, current.minor + 1, 0),
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
    fn clears_pre_and_build() {
        assert_eq!(next("1.2.3-rc.1+abc", Bump::Major), "2.0.0");
        assert_eq!(next("1.2.3-rc.1+abc", Bump::Minor), "1.3.0");
        assert_eq!(next("1.2.3-rc.1+abc", Bump::Patch), "1.2.4");
    }
}
