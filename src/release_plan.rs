use serde::Serialize;

/// The JSON document that `version` prints to stdout, mirroring the upstream
/// `ReleasePlan` type (without `preState`). Serialized as a single line.
#[derive(Serialize)]
pub(crate) struct ReleasePlan {
    /// The consumed changesets, in file-name order.
    pub(crate) changesets: Vec<ChangesetEntry>,
    /// One entry per package named by any changeset, in package-name order.
    pub(crate) releases: Vec<Release>,
}

/// A consumed changeset.
#[derive(Serialize)]
pub(crate) struct ChangesetEntry {
    /// The changeset file name without the `.md` extension.
    pub(crate) id: String,
    /// The summary text of the changeset.
    pub(crate) summary: String,
    /// The packages the changeset names, in frontmatter order.
    pub(crate) releases: Vec<ReleaseRef>,
}

/// One package-to-bump entry in a changeset's frontmatter.
#[derive(Serialize)]
pub(crate) struct ReleaseRef {
    /// The package name.
    pub(crate) name: String,
    /// The requested bump type: `major`, `minor`, `patch`, or `none`.
    #[serde(rename = "type")]
    pub(crate) bump: &'static str,
}

/// The version change planned for one package.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Release {
    /// The package name.
    pub(crate) name: String,
    /// The widest bump requested for the package: `major`, `minor`, `patch`,
    /// or `none`.
    #[serde(rename = "type")]
    pub(crate) bump: &'static str,
    /// The package version before this run.
    pub(crate) old_version: String,
    /// The package version after this run; equals `old_version` for `none`.
    pub(crate) new_version: String,
    /// The ids of the changesets naming this package (`none` entries
    /// included), in file-name order.
    pub(crate) changesets: Vec<String>,
}
