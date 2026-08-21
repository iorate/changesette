use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use jsonc_parser::{
    ParseOptions,
    cst::{CstRootNode, CstStringLit},
};

use crate::jsonc::{set_string_value, string_prop};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreMode {
    Pre,
    Exit,
}

impl PreMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PreMode::Pre => "pre",
            PreMode::Exit => "exit",
        }
    }
}

/// A loaded `.changeset/pre.json` whose serialization preserves the original
/// formatting and unknown fields, changing only the rewritten values.
pub(crate) struct PreJson {
    path: PathBuf,
    root: CstRootNode,
    mode_lit: CstStringLit,
    tag_lit: CstStringLit,
    mode: PreMode,
    tag: String,
}

impl PreJson {
    /// Loads `changeset_dir/pre.json`, or `Ok(None)` when it does not exist.
    pub(crate) fn load(changeset_dir: &Path) -> Result<Option<Self>> {
        let path = changeset_dir.join("pre.json");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).context(path.display().to_string()),
        };
        let context = path.display().to_string();
        Self::parse(path, &text).context(context).map(Some)
    }

    fn parse(path: PathBuf, text: &str) -> Result<Self> {
        let root = CstRootNode::parse(text, &ParseOptions::default())?;
        let object = root
            .object_value()
            .context("the root value must be an object")?;

        if object.get("initialVersions").is_some() || object.get("changesets").is_some() {
            bail!(
                "in the changesets v2 format; run a changesets v3 CLI command (e.g. `npx @changesets/cli@3 status`) once to migrate it"
            );
        }

        let mode_lit = string_prop(&object, "mode", "\"mode\"")?.context("missing \"mode\"")?;
        let raw_mode = mode_lit
            .decoded_value()
            .context("\"mode\" must be a valid string")?;
        let mode = match raw_mode.as_str() {
            "pre" => PreMode::Pre,
            "exit" => PreMode::Exit,
            other => bail!("\"mode\" must be \"pre\" or \"exit\", not {other:?}"),
        };

        let tag_lit = string_prop(&object, "tag", "\"tag\"")?.context("missing \"tag\"")?;
        let tag = tag_lit
            .decoded_value()
            .context("\"tag\" must be a valid string")?;

        Ok(Self {
            path,
            root,
            mode_lit,
            tag_lit,
            mode,
            tag,
        })
    }

    pub(crate) fn mode(&self) -> PreMode {
        self.mode
    }

    pub(crate) fn tag(&self) -> &str {
        &self.tag
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn set_mode(&mut self, mode: PreMode) {
        set_string_value(&self.mode_lit, mode.as_str());
        self.mode = mode;
    }

    /// Sets the tag; `tag` must have passed `validate_tag`.
    pub(crate) fn set_tag(&mut self, tag: &str) {
        set_string_value(&self.tag_lit, tag);
        self.tag = tag.to_owned();
    }

    pub(crate) fn text(&self) -> String {
        self.root.to_string()
    }
}

/// Checks that `tag` is usable as the leading identifiers of a semver
/// pre-release, so that `-{tag}.{n}` is a valid version.
pub(crate) fn validate_tag(tag: &str) -> Result<()> {
    // An empty pre-release parses, so the counter is appended before the
    // check to reject an empty tag along with the invalid ones.
    if let Err(err) = semver::Prerelease::new(&format!("{tag}.0")) {
        bail!("invalid pre tag {tag:?}: {err}");
    }
    Ok(())
}

/// Writes a fresh `pre.json` in pre mode.
pub(crate) fn write_new(changeset_dir: &Path, tag: &str) -> Result<()> {
    let path = changeset_dir.join("pre.json");
    let text = format!("{{\n  \"mode\": \"pre\",\n  \"tag\": \"{tag}\"\n}}\n");
    fs::write(&path, text).with_context(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Loaded {
        _dir: tempfile::TempDir,
        pre: PreJson,
    }

    fn load_ok(text: &str) -> Loaded {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("pre.json"), text).unwrap();
        let pre = PreJson::load(dir.path()).unwrap().unwrap();
        Loaded { _dir: dir, pre }
    }

    fn load_err(text: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("pre.json"), text).unwrap();
        let err = PreJson::load(dir.path()).err().unwrap();
        format!("{err:#}").replace(
            &dir.path().join("pre.json").display().to_string(),
            ".changeset/pre.json",
        )
    }

    #[test]
    fn loads_a_v3_pre_json() {
        let loaded = load_ok("{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\"\n}\n");
        assert_eq!(loaded.pre.mode(), PreMode::Pre);
        assert_eq!(loaded.pre.tag(), "beta");
        assert!(loaded.pre.path().ends_with("pre.json"));
    }

    #[test]
    fn loads_an_exited_pre_json() {
        let loaded = load_ok("{\n  \"mode\": \"exit\",\n  \"tag\": \"beta\"\n}\n");
        assert_eq!(loaded.pre.mode(), PreMode::Exit);
    }

    #[test]
    fn returns_none_without_pre_json() {
        let dir = tempfile::tempdir().unwrap();
        assert!(PreJson::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn rejects_a_v2_pre_json() {
        insta::assert_snapshot!(load_err(
            "{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\",\n  \"initialVersions\": {\n    \"pkg-a\": \"1.0.0\"\n  }\n}\n"
        ));
    }

    #[test]
    fn rejects_a_v2_changesets_field() {
        insta::assert_snapshot!(load_err(
            "{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\",\n  \"changesets\": []\n}\n"
        ));
    }

    #[test]
    fn rejects_a_non_object_root() {
        insta::assert_snapshot!(load_err("[]\n"));
    }

    #[test]
    fn rejects_a_missing_mode() {
        insta::assert_snapshot!(load_err("{\n  \"tag\": \"beta\"\n}\n"));
    }

    #[test]
    fn rejects_an_unknown_mode() {
        insta::assert_snapshot!(load_err(
            "{\n  \"mode\": \"pré\",\n  \"tag\": \"beta\"\n}\n"
        ));
    }

    #[test]
    fn rejects_a_non_string_mode() {
        insta::assert_snapshot!(load_err("{\n  \"mode\": 1,\n  \"tag\": \"beta\"\n}\n"));
    }

    #[test]
    fn rejects_a_missing_tag() {
        insta::assert_snapshot!(load_err("{\n  \"mode\": \"pre\"\n}\n"));
    }

    #[test]
    fn rejects_a_non_string_tag() {
        insta::assert_snapshot!(load_err("{\n  \"mode\": \"pre\",\n  \"tag\": null\n}\n"));
    }

    #[test]
    fn ignores_unknown_fields() {
        let loaded =
            load_ok("{\n  \"mode\": \"pre\",\n  \"tag\": \"beta\",\n  \"someday\": [1, 2, 3]\n}\n");
        assert_eq!(loaded.pre.mode(), PreMode::Pre);
        assert_eq!(loaded.pre.tag(), "beta");
    }

    #[test]
    fn accepts_comments_and_odd_formatting() {
        let loaded = load_ok("{ // in pre mode\n\t\"tag\":\t\"beta\", \"mode\": \"pre\" }");
        assert_eq!(loaded.pre.mode(), PreMode::Pre);
        assert_eq!(loaded.pre.tag(), "beta");
    }

    #[test]
    fn reenter_rewrite_preserves_unknown_fields_and_formatting() {
        let mut loaded = load_ok(
            "{ // pre state\n\t\"tag\":\t\"alpha\",\n\t\"mode\": \"exit\",\n\t\"someday\": [1, 2, 3]\n}",
        );
        loaded.pre.set_mode(PreMode::Pre);
        loaded.pre.set_tag("beta.2");
        assert_eq!(loaded.pre.mode(), PreMode::Pre);
        assert_eq!(loaded.pre.tag(), "beta.2");
        insta::assert_snapshot!(loaded.pre.text());
    }

    #[test]
    fn validate_tag_accepts_dotted_and_numeric_tags() {
        for tag in ["beta", "beta.2", "1", "rc-0"] {
            assert!(validate_tag(tag).is_ok(), "{tag} should be accepted");
        }
    }

    #[test]
    fn validate_tag_rejects_invalid_tags() {
        for tag in ["", " ", "beta 2", "beta_2", "ベータ", "01", "beta."] {
            assert!(validate_tag(tag).is_err(), "{tag:?} should be rejected");
        }
    }

    #[test]
    fn validate_tag_names_the_tag_in_the_error() {
        insta::assert_snapshot!(format!("{:#}", validate_tag("beta 2").unwrap_err()));
    }
}
