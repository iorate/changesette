use std::{fmt, sync::LazyLock};

use anyhow::{Result, bail};
use nodejs_semver::Version;
use regex::Regex;

// `\d` would also match non-ASCII digits, hence the explicit `[0-9]`.
const PRE_RELEASE: &str = r"(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*";

static VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^v?(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-{PRE_RELEASE})?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
    ))
    .unwrap()
});

static PRERELEASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^{PRE_RELEASE}$")).unwrap());

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
    let (major, minor, patch) = (current.major(), current.minor(), current.patch());
    let pre = current.is_prerelease();
    match bump {
        Bump::Major if pre && minor == 0 && patch == 0 => Version::from((major, 0, 0)),
        Bump::Major => Version::from((checked_inc(major), 0, 0)),
        Bump::Minor if pre && patch == 0 => Version::from((major, minor, 0)),
        Bump::Minor => Version::from((major, checked_inc(minor), 0)),
        Bump::Patch if pre => Version::from((major, minor, patch)),
        Bump::Patch => Version::from((major, minor, checked_inc(patch))),
    }
}

// Release builds do not check arithmetic overflow, so `+ 1` on a pathological
// u64::MAX value would silently wrap and write a rewound version; keep the
// failure loud instead.
fn checked_inc(number: u64) -> u64 {
    number.checked_add(1).expect("version number overflow")
}

// `Version::parse` ignores the input past the first byte it cannot read and
// folds a leading zero away (`01` becomes `1`), so it cannot validate a version
// written by the user. What it accepts here is what npm's semver accepts in
// strict mode: semver 2.0 plus a `v` prefix and surrounding whitespace.
pub fn parse_version(text: &str) -> Result<Version> {
    let text = text.trim();
    if !VERSION.is_match(text) {
        bail!("not a valid semver version");
    }
    Ok(Version::parse(text)?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prerelease(String);

impl Prerelease {
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        PRERELEASE.is_match(text).then(|| Self(text.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn with_counter(&self, counter: u64) -> Self {
        Self(format!("{}.{}", self.0, counter))
    }
}

impl fmt::Display for Prerelease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub fn pre_counter(current: &Version, tag: &Prerelease) -> u64 {
    // Counting on the tag, rather than on the second pre-release identifier,
    // keeps a dotted tag (`beta.2`) counting and restarts on a tag switch.
    let pre = current
        .pre_release()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(".");
    pre.strip_prefix(&format!("{tag}."))
        .and_then(|rest| rest.parse::<u64>().ok())
        .map_or(0, checked_inc)
}

#[must_use]
pub fn next_pre_version(current: &Version, bump: Bump, tag: &Prerelease) -> Version {
    next_pre_version_with(current, bump, tag, pre_counter(current, tag))
}

#[must_use]
pub fn next_pre_version_with(
    current: &Version,
    bump: Bump,
    tag: &Prerelease,
    counter: u64,
) -> Version {
    with_pre(&next_version(current, bump), &tag.with_counter(counter))
}

#[must_use]
pub fn with_pre(version: &Version, pre: &Prerelease) -> Version {
    Version::parse(format!(
        "{}.{}.{}-{pre}",
        version.major(),
        version.minor(),
        version.patch()
    ))
    .expect("a valid pre-release parses")
}
