use std::{fs, io, path::Path};

use anyhow::{Context, Result, ensure};
use toml_edit::DocumentMut;

/// Settings read from `.changeset/changesette.toml`.
#[derive(Debug)]
pub(crate) struct Config {
    /// The `github.repo` value (`owner/repo`); `None` turns the GitHub
    /// integration off.
    pub(crate) github_repo: Option<String>,
}

/// Loads `.changeset/changesette.toml` under `dir`; a missing file yields the
/// default config.
pub(crate) fn load(dir: &Path) -> Result<Config> {
    let path = dir.join(".changeset").join("changesette.toml");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(Config { github_repo: None });
        }
        Err(err) => return Err(err).context(path.display().to_string()),
    };
    parse(&text).context(".changeset/changesette.toml")
}

fn parse(text: &str) -> Result<Config> {
    let doc: DocumentMut = text.parse()?;

    let mut github_repo = None;
    for (key, item) in doc.iter() {
        ensure!(key == "github", "unknown key \"{key}\"");
        let table = item.as_table_like().context("\"github\" must be a table")?;
        for (github_key, github_item) in table.iter() {
            ensure!(github_key == "repo", "unknown key \"github.{github_key}\"");
            let repo = github_item
                .as_str()
                .context("\"github.repo\" must be a string")?;
            ensure!(
                is_owner_slash_repo(repo),
                "\"github.repo\" ({repo:?}) must be of the form \"owner/repo\""
            );
            github_repo = Some(repo.to_owned());
        }
        ensure!(
            github_repo.is_some(),
            "[github] is missing the \"repo\" key"
        );
    }
    Ok(Config { github_repo })
}

fn is_owner_slash_repo(value: &str) -> bool {
    matches!(
        value.split_once('/'),
        Some((owner, repo)) if !owner.is_empty() && !repo.is_empty() && !repo.contains('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_from(toml: &str) -> Result<Config> {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".changeset")).unwrap();
        fs::write(dir.path().join(".changeset/changesette.toml"), toml).unwrap();
        load(dir.path())
    }

    fn load_err(toml: &str) -> String {
        format!("{:#}", load_from(toml).err().unwrap())
    }

    #[test]
    fn integration_is_off_without_a_config_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap().github_repo, None);
    }

    #[test]
    fn integration_is_off_with_an_empty_config_file() {
        assert_eq!(load_from("").unwrap().github_repo, None);
    }

    #[test]
    fn reads_the_github_repo() {
        assert_eq!(
            load_from("[github]\nrepo = \"iorate/ublacklist\"\n")
                .unwrap()
                .github_repo
                .as_deref(),
            Some("iorate/ublacklist")
        );
    }

    #[test]
    fn rejects_a_missing_repository_key() {
        insta::assert_snapshot!(load_err("[github]\n"));
    }

    #[test]
    fn rejects_a_non_string_repository() {
        insta::assert_snapshot!(load_err("[github]\nrepo = 1\n"));
    }

    #[test]
    fn rejects_a_malformed_repository() {
        insta::assert_snapshot!(load_err("[github]\nrepo = \"iorate\"\n"));
    }

    #[test]
    fn rejects_a_repository_with_extra_slashes() {
        insta::assert_snapshot!(load_err("[github]\nrepo = \"iorate/ublacklist/extra\"\n"));
    }

    #[test]
    fn rejects_an_unknown_table() {
        insta::assert_snapshot!(load_err("[gitlab]\nrepo = \"iorate/ublacklist\"\n"));
    }

    #[test]
    fn rejects_an_unknown_key_under_github() {
        insta::assert_snapshot!(load_err(
            "[github]\nrepo = \"iorate/ublacklist\"\ntoken = \"secret\"\n"
        ));
    }
}
