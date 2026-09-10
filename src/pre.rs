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
pub enum PreMode {
    Pre,
    Exit,
}

impl PreMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PreMode::Pre => "pre",
            PreMode::Exit => "exit",
        }
    }
}

pub struct PreJson {
    path: PathBuf,
    root: CstRootNode,
    mode_lit: CstStringLit,
    tag_lit: CstStringLit,
    mode: PreMode,
    tag: String,
}

impl PreJson {
    pub fn load(changeset_dir: &Path) -> Result<Option<Self>> {
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

    #[must_use]
    pub fn mode(&self) -> PreMode {
        self.mode
    }

    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn set_mode(&mut self, mode: PreMode) {
        set_string_value(&self.mode_lit, mode.as_str());
        self.mode = mode;
    }

    // `tag` must have passed `validate_tag`.
    pub fn set_tag(&mut self, tag: &str) {
        set_string_value(&self.tag_lit, tag);
        tag.clone_into(&mut self.tag);
    }

    #[must_use]
    pub fn text(&self) -> String {
        self.root.to_string()
    }
}

pub fn validate_tag(tag: &str) -> Result<()> {
    // An empty pre-release parses, so the counter is appended before the
    // check to reject an empty tag along with the invalid ones.
    if let Err(err) = semver::Prerelease::new(&format!("{tag}.0")) {
        bail!("invalid pre tag {tag:?}: {err}");
    }
    Ok(())
}

pub fn write_new(changeset_dir: &Path, tag: &str) -> Result<()> {
    let path = changeset_dir.join("pre.json");
    let text = format!("{{\n  \"mode\": \"pre\",\n  \"tag\": \"{tag}\"\n}}\n");
    fs::write(&path, text).with_context(|| path.display().to_string())
}
