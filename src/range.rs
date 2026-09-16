use nodejs_semver::{Range, Version};

use crate::{bump::Bump, config::UpdateInternalDependencies, dependency::Spec};

pub enum Target {
    Release(Version),
    Snapshot(Version),
}

#[must_use]
pub fn rewrite(
    spec: &Spec,
    target: &Target,
    bump: Bump,
    min: UpdateInternalDependencies,
) -> Option<String> {
    match (spec, target) {
        (Spec::Any, Target::Release(new) | Target::Snapshot(new)) => {
            new.is_prerelease().then(|| new.to_string())
        }
        (Spec::Range { workspace, .. }, Target::Snapshot(new)) => {
            Some(with_protocol(*workspace, new.to_string()))
        }
        (Spec::Range { range, workspace }, Target::Release(new)) => {
            if range.satisfies(new) && !min.allows(bump) {
                return None;
            }
            Some(with_protocol(*workspace, raise(range, new)))
        }
        (Spec::WorkspaceAlias(_) | Spec::WorkspacePath | Spec::External, _) => None,
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
