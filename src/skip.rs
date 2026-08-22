use std::{collections::BTreeSet, path::Path};

use anyhow::{Context, Result, bail};

use crate::{
    changeset::LoadedChange,
    config::Config,
    workspace::{Member, Workspace},
};

fn should_skip(member: &Member, config: &Config, ignore: &[String]) -> bool {
    ignore.iter().any(|name| name == member.name())
        || (member.private() && !config.private_packages_version)
        || member.version().is_none()
}

/// The names of the workspace members `version` skips.
pub(crate) struct SkipSet {
    names: BTreeSet<String>,
}

impl SkipSet {
    /// Collects the members skipped as private, ignored, or versionless,
    /// resolving the ignore names from the config or `cli_ignore` (using
    /// both is an error).
    pub(crate) fn load(
        workspace: &Workspace,
        config: &Config,
        cli_ignore: &[String],
    ) -> Result<SkipSet> {
        let ignore = if config.has_ignore() {
            if !cli_ignore.is_empty() {
                bail!(
                    "the --ignore option cannot be used while ignore is defined in .changeset/config.json; use only one of them"
                );
            }
            config.resolve_ignore(workspace.members().iter().map(Member::name))
        } else {
            for name in cli_ignore {
                workspace.member(name).context("invalid `--ignore` value")?;
            }
            cli_ignore.to_vec()
        };
        let names = workspace
            .members()
            .iter()
            .filter(|member| should_skip(member, config, &ignore))
            .map(|member| member.name().to_owned())
            .collect();
        Ok(SkipSet { names })
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// Returns the changesets in `changes` whose releases are not all
    /// skipped; a changeset mixing skipped and not skipped packages is an
    /// error, as is one naming a non-member package.
    pub(crate) fn filter_changes(
        &self,
        workspace: &Workspace,
        changeset_dir: &Path,
        changes: &[LoadedChange],
    ) -> Result<Vec<LoadedChange>> {
        // Membership is checked before the skip judgment so that a changeset
        // naming an unknown package always reports that rather than a
        // mixed-changeset error.
        for change in changes {
            for (name, _) in &change.releases {
                workspace
                    .member(name)
                    .with_context(|| changeset_dir.join(change.rel_path()).display().to_string())?;
            }
        }

        let mut consumed = Vec::new();
        for change in changes {
            let (skipped, not_skipped): (Vec<&str>, Vec<&str>) = change
                .releases
                .iter()
                .map(|(name, _)| name.as_str())
                .partition(|name| self.contains(name));
            if skipped.is_empty() {
                consumed.push(change.clone());
            } else if !not_skipped.is_empty() {
                bail!(
                    "{}: cannot mix skipped packages ({}) and not skipped packages ({})",
                    changeset_dir.join(change.rel_path()).display(),
                    quote_list(&skipped),
                    quote_list(&not_skipped)
                );
            }
        }
        Ok(consumed)
    }
}

fn quote_list(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
