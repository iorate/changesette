use semver::{Prerelease, Version};

// Ordered so that `max` picks the widest bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

impl Bump {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Bump::Patch => "patch",
            Bump::Minor => "minor",
            Bump::Major => "major",
        }
    }
}

#[must_use]
pub fn next_version(current: &Version, bump: Bump) -> Version {
    let pre = !current.pre.is_empty();
    match bump {
        Bump::Major if pre && current.minor == 0 && current.patch == 0 => {
            Version::new(current.major, 0, 0)
        }
        Bump::Major => Version::new(checked_inc(current.major), 0, 0),
        Bump::Minor if pre && current.patch == 0 => Version::new(current.major, current.minor, 0),
        Bump::Minor => Version::new(current.major, checked_inc(current.minor), 0),
        Bump::Patch if pre => Version::new(current.major, current.minor, current.patch),
        Bump::Patch => Version::new(current.major, current.minor, checked_inc(current.patch)),
    }
}

// Release builds do not check arithmetic overflow, so `+ 1` on a pathological
// u64::MAX value would silently wrap and write a rewound version; keep the
// failure loud instead.
fn checked_inc(number: u64) -> u64 {
    number.checked_add(1).expect("version number overflow")
}

pub fn pre_counter(current: &Version, tag: &str) -> u64 {
    // Counting on the tag, rather than on the second pre-release identifier,
    // keeps a dotted tag (`beta.2`) counting and restarts on a tag switch.
    current
        .pre
        .as_str()
        .strip_prefix(&format!("{tag}."))
        .and_then(|rest| rest.parse::<u64>().ok())
        .map_or(0, checked_inc)
}

#[must_use]
pub fn next_pre_version(current: &Version, bump: Bump, tag: &str) -> Version {
    next_pre_version_with(current, bump, tag, pre_counter(current, tag))
}

#[must_use]
pub fn next_pre_version_with(current: &Version, bump: Bump, tag: &str, counter: u64) -> Version {
    let mut version = next_version(current, bump);
    version.pre =
        Prerelease::new(&format!("{tag}.{counter}")).expect("a validated tag stays valid");
    version
}
