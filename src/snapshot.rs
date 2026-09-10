use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use semver::{Prerelease, Version};
use time::OffsetDateTime;

use crate::{
    bump::{self, Bump},
    config::Config,
};

pub struct Snapshot {
    pub tag: Option<String>,
    pub template: Option<String>,
}

pub struct SnapshotVersions {
    suffix: Prerelease,
    use_calculated_version: bool,
}

impl SnapshotVersions {
    pub fn resolve(snapshot: &Snapshot, config: &Config) -> Result<Self> {
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

    #[must_use]
    pub fn apply(&self, old_version: &Version, bump: Bump) -> Version {
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

    fn render(tag: Option<&str>, template: Option<&str>) -> Result<String> {
        render_suffix(tag, template, MILLIS).map(|suffix| suffix.to_string())
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
    fn renders_templates() {
        assert_eq!(render(None, None).unwrap(), DATETIME);
        assert_eq!(
            render(Some("canary"), None).unwrap(),
            format!("canary-{DATETIME}")
        );
        assert_eq!(
            render(Some("canary"), Some("{tag}-{timestamp}-{datetime}")).unwrap(),
            format!("canary-{MILLIS}-{DATETIME}")
        );
        assert_eq!(
            render(Some("canary"), Some("{tag}.{tag}")).unwrap(),
            "canary.canary"
        );
    }

    #[test]
    fn rejects_invalid_templates() {
        for (tag, template, needle) in [
            (None, Some("{commit}-{datetime}"), "{commit}"),
            (None, Some("{commit-short}-{datetime}"), "{commit-short}"),
            (Some("canary"), Some("{datetime}"), "{tag}"),
            (None, Some("{tag}-{datetime}"), "{tag}"),
            (Some("pr#123"), None, "pr#123"),
            (None, Some("{datetime}-{branch}"), "{branch}"),
        ] {
            let err = format!("{:#}", render(tag, template).unwrap_err());
            assert!(err.contains(needle), "{tag:?} {template:?}: {err}");
        }
    }
}
