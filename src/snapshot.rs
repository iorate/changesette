use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use semver::{Prerelease, Version};
use time::OffsetDateTime;

use crate::{
    bump::{self, Bump},
    config::Config,
};

pub(crate) struct Snapshot {
    pub(crate) tag: Option<String>,
    pub(crate) template: Option<String>,
}

pub(crate) struct SnapshotVersions {
    suffix: Prerelease,
    use_calculated_version: bool,
}

impl SnapshotVersions {
    /// Computes the suffix following upstream changesets: renders the
    /// template (`--snapshot-prerelease-template`, or the config's
    /// `snapshot.prereleaseTemplate`, or `{tag}-{datetime}` / `{datetime}`
    /// by default), reading the clock, and validates the result as a semver
    /// pre-release; the upstream `{commit}` / `{commit-short}` placeholders
    /// are an error.
    pub(crate) fn resolve(snapshot: &Snapshot, config: &Config) -> Result<Self> {
        let template = snapshot
            .template
            .as_deref()
            .or(config.snapshot_prerelease_template.as_deref());
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the current time is after the Unix epoch")
            .as_millis();
        let suffix = render_suffix(snapshot.tag.as_deref(), template, millis)?;
        Ok(Self {
            suffix,
            use_calculated_version: config.snapshot_use_calculated_version,
        })
    }

    /// Returns the snapshot version replacing the normal `next_version`
    /// result: the suffix on `0.0.0`, or on the normally calculated version
    /// when `snapshot.useCalculatedVersion` is set.
    pub(crate) fn apply(&self, old_version: &Version, bump: Bump) -> Version {
        let mut version = if self.use_calculated_version {
            bump::next_version(old_version, bump)
        } else {
            Version::new(0, 0, 0)
        };
        version.pre = self.suffix.clone();
        version
    }
}

fn render_suffix(tag: Option<&str>, template: Option<&str>, millis: u128) -> Result<Prerelease> {
    let datetime = utc_datetime(
        u64::try_from(millis / 1_000).expect("the current time in seconds fits in u64"),
    );
    let suffix = match template {
        None => match tag {
            Some(tag) => format!("{tag}-{datetime}"),
            None => datetime,
        },
        Some(template) => {
            for placeholder in ["{commit}", "{commit-short}"] {
                if template.contains(placeholder) {
                    bail!(
                        "the template contains \"{placeholder}\", which changesette does not support: changesette performs no git operations"
                    );
                }
            }
            match (tag, template.contains("{tag}")) {
                (Some(tag), false) => bail!(
                    "the snapshot tag {tag:?} is given but the template does not contain \"{{tag}}\""
                ),
                (None, true) => {
                    bail!("the template contains \"{{tag}}\" but no snapshot tag is given")
                }
                _ => {}
            }
            let timestamp = millis.to_string();
            template
                .replace("{tag}", tag.unwrap_or_default())
                .replace("{timestamp}", &timestamp)
                .replace("{datetime}", &datetime)
        }
    };
    // Deliberately stricter than upstream, which writes the suffix into
    // package.json unchecked.
    Prerelease::new(&suffix).with_context(|| format!("invalid snapshot suffix {suffix:?}"))
}

fn utc_datetime(secs: u64) -> String {
    let datetime = OffsetDateTime::from_unix_timestamp(secs.cast_signed())
        .expect("the current time is within time's supported range");
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        datetime.year(),
        datetime.month() as u8,
        datetime.day(),
        datetime.hour(),
        datetime.minute(),
        datetime.second()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MILLIS: u128 = 1_755_820_800_123;
    const DATETIME: &str = "20250822000000";

    fn render_ok(tag: Option<&str>, template: Option<&str>) -> String {
        render_suffix(tag, template, MILLIS).unwrap().to_string()
    }

    fn render_err(tag: Option<&str>, template: Option<&str>) -> String {
        format!("{:#}", render_suffix(tag, template, MILLIS).unwrap_err())
    }

    #[test]
    fn formats_utc_datetimes() {
        assert_eq!(utc_datetime(0), "19700101000000");
        assert_eq!(utc_datetime(951_782_400), "20000229000000");
        assert_eq!(utc_datetime(1_740_787_199), "20250228235959");
        assert_eq!(utc_datetime(4_102_444_799), "20991231235959");
        assert_eq!(utc_datetime(68_169_553_622), "41300317110702");
    }

    #[test]
    fn renders_the_default_template() {
        assert_eq!(render_ok(None, None), DATETIME);
        assert_eq!(
            render_ok(Some("canary"), None),
            format!("canary-{DATETIME}")
        );
    }

    #[test]
    fn renders_every_placeholder() {
        assert_eq!(
            render_ok(Some("canary"), Some("{tag}-{timestamp}-{datetime}")),
            format!("canary-{MILLIS}-{DATETIME}")
        );
    }

    #[test]
    fn renders_a_repeated_placeholder() {
        assert_eq!(
            render_ok(Some("canary"), Some("{tag}.{tag}")),
            "canary.canary"
        );
    }

    #[test]
    fn rejects_the_commit_placeholder() {
        insta::assert_snapshot!(render_err(None, Some("{commit}-{datetime}")));
    }

    #[test]
    fn rejects_the_commit_short_placeholder() {
        insta::assert_snapshot!(render_err(None, Some("{commit-short}-{datetime}")));
    }

    #[test]
    fn rejects_a_tag_without_the_tag_placeholder() {
        insta::assert_snapshot!(render_err(Some("canary"), Some("{datetime}")));
    }

    #[test]
    fn rejects_the_tag_placeholder_without_a_tag() {
        insta::assert_snapshot!(render_err(None, Some("{tag}-{datetime}")));
    }

    #[test]
    fn rejects_an_invalid_tag() {
        insta::assert_snapshot!(render_err(Some("pr#123"), None));
    }

    #[test]
    fn rejects_an_unknown_placeholder() {
        insta::assert_snapshot!(render_err(None, Some("{datetime}-{branch}")));
    }
}
