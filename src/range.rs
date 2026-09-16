use nodejs_semver::{Range, Version};

use crate::{bump::Bump, config::UpdateInternalDependencies, dependency::Spec};

pub enum Target {
    Release(Version),
    Snapshot(Version),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Update {
    Explicit(String),
    // The text stays, but the version it resolves to changes, as with
    // `workspace:*` or `*`, so the dependent still reports the release.
    Implicit,
}

#[must_use]
pub fn update(
    spec: &Spec,
    target: &Target,
    bump: Bump,
    min: UpdateInternalDependencies,
) -> Option<Update> {
    match (spec, target) {
        (Spec::Any, Target::Release(new) | Target::Snapshot(new)) => {
            if new.is_prerelease() {
                Some(Update::Explicit(new.to_string()))
            } else {
                min.allows(bump).then_some(Update::Implicit)
            }
        }
        (Spec::Range { workspace, .. }, Target::Snapshot(new)) => {
            Some(Update::Explicit(with_protocol(*workspace, new.to_string())))
        }
        (Spec::Range { range, workspace }, Target::Release(new)) => {
            if range.satisfies(new) && !min.allows(bump) {
                workspace.then_some(Update::Implicit)
            } else {
                Some(Update::Explicit(with_protocol(
                    *workspace,
                    raise(range, new),
                )))
            }
        }
        (Spec::WorkspaceAlias(_) | Spec::WorkspacePath, _) => Some(Update::Implicit),
        (Spec::External, _) => None,
    }
}

fn with_protocol(workspace: bool, text: String) -> String {
    if workspace {
        format!("workspace:{text}")
    } else {
        text
    }
}

fn parse(text: &str) -> Range {
    Range::parse(text).expect("a version with a known prefix makes a valid range")
}

// nodejs-semver exposes no comparators and its Display is the normalized
// form, so the short notations are recovered by comparing ranges as sets.
fn raise(range: &Range, new: &Version) -> String {
    if let Some(raised) = range.intersect(&parse(&format!(">={new}"))) {
        return ["^", "~", "", ">="]
            .into_iter()
            .map(|prefix| format!("{prefix}{new}"))
            .find(|candidate| parse(candidate) == raised)
            .unwrap_or_else(|| raised.to_string());
    }
    if let Some(min) = range.min_version() {
        for prefix in ["^", "~", ""] {
            if parse(&format!("{prefix}{min}")) == *range {
                return format!("{prefix}{new}");
            }
        }
    }
    format!(">={new} <{}.0.0-0", new.major() + 1)
}
