use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use tracing::{debug, warn};

use super::pattern::{Pattern, Seg, is_plain_component, seg_matches};
use super::{probe_is_file, report_fs_error};
use crate::output::display_path;

/// Collects the member candidate directories under `root` matching
/// `positives` minus `negations`, keyed by the root-relative `/`-separated
/// directory (`.` for the root itself); reading the manifests is left to the
/// caller. Directories named in `excluded_names` are never entered nor
/// matched, and `reject_pnpm_manifests` makes a matched directory carrying
/// only a pnpm-specific manifest an error.
pub(crate) fn collect(
    root: &Path,
    positives: &[Pattern],
    negations: &[Pattern],
    excluded_names: &[&str],
    reject_pnpm_manifests: bool,
) -> Result<BTreeMap<String, PathBuf>> {
    let mut walker = Walker {
        patterns: positives,
        negations,
        excluded_names,
        reject_pnpm_manifests,
        candidates: BTreeMap::new(),
    };
    if !positives.is_empty() {
        let states = closure(
            positives,
            (0..positives.len()).map(|pattern| (pattern, 0)).collect(),
        );
        walker.walk(root, "", &states)?;
    }
    Ok(walker.candidates)
}

/// Whether `dir` holds a `package.json`; with `reject_pnpm_manifests`, a
/// `package.yaml` or `package.json5` found instead is an error.
pub(crate) fn has_manifest(dir: &Path, reject_pnpm_manifests: bool) -> Result<bool> {
    if probe_is_file(&dir.join("package.json")) {
        return Ok(true);
    }
    // pnpm reads these as first-class manifests, and honoring them would
    // mean writing versions back into them too; stopping loudly beats
    // silently dropping the package.
    if reject_pnpm_manifests {
        for name in ["package.yaml", "package.json5"] {
            let path = dir.join(name);
            if probe_is_file(&path) {
                bail!(
                    "{}: only package.json manifests are supported",
                    display_path(&path)
                )
            }
        }
    }
    Ok(false)
}

// The candidate check runs before this, so the debug line only names real
// candidates.
fn excluded(rel_dir: &str, negations: &[Pattern]) -> bool {
    let rel_manifest = if rel_dir == "." {
        "package.json".to_owned()
    } else {
        format!("{rel_dir}/package.json")
    };
    let excluded = negations
        .iter()
        .any(|negation| negation.matches(&rel_manifest, true));
    if excluded {
        debug!("{rel_dir}: excluded by a negative workspace pattern");
    }
    excluded
}

// A walker state is a pattern (as an index into the patterns) and the index
// of its next unconsumed segment.
type State = (usize, usize);

// Adds the epsilon transitions: a globstar can consume zero segments, so a
// state resting on one also rests past it. The last segment is always
// `Literal("package.json")`, so `seg + 1` stays in bounds.
fn closure(patterns: &[Pattern], mut states: Vec<State>) -> Vec<State> {
    let mut i = 0;
    while i < states.len() {
        let (pattern, seg) = states[i];
        if matches!(patterns[pattern].segs()[seg], Seg::Globstar) {
            let next = (pattern, seg + 1);
            if !states.contains(&next) {
                states.push(next);
            }
        }
        i += 1;
    }
    states
}

fn child_rel(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_owned()
    } else {
        format!("{rel}/{name}")
    }
}

struct Walker<'a> {
    patterns: &'a [Pattern],
    negations: &'a [Pattern],
    excluded_names: &'a [&'a str],
    reject_pnpm_manifests: bool,
    candidates: BTreeMap<String, PathBuf>,
}

impl<'a> Walker<'a> {
    fn walk(&mut self, dir: &Path, rel: &str, states: &[State]) -> Result<()> {
        let patterns = self.patterns;
        // The candidate check runs after the epsilon closure so that `x/**`
        // covers `x` itself.
        if states
            .iter()
            .any(|&(pattern, seg)| seg == patterns[pattern].segs().len() - 1)
        {
            let rel_dir = if rel.is_empty() { "." } else { rel };
            if has_manifest(dir, self.reject_pnpm_manifests)? && !excluded(rel_dir, self.negations)
            {
                self.candidates
                    .entry(rel_dir.to_owned())
                    .or_insert_with(|| dir.to_path_buf());
            }
        }
        // Probing a literal by name rather than comparing it with the
        // directory entries gives it its symlink transparency, its dot
        // matching, and on a case-insensitive filesystem the loose matching
        // every package manager has, and spares reading a directory no
        // wildcard needs.
        let mut literals: BTreeMap<&'a str, Vec<State>> = BTreeMap::new();
        let mut dynamic = Vec::new();
        for &(pattern, seg_index) in states {
            let segs = patterns[pattern].segs();
            // The final `package.json` segment names a file, so consuming it
            // with a directory can never lead to a candidate.
            if seg_index == segs.len() - 1 {
                continue;
            }
            match &segs[seg_index] {
                Seg::Literal(name) => literals
                    .entry(name)
                    .or_default()
                    .push((pattern, seg_index + 1)),
                _ => dynamic.push((pattern, seg_index)),
            }
        }
        if !dynamic.is_empty() {
            self.read_entries(dir, rel, &dynamic, &mut literals)?;
        }
        for (name, next) in literals {
            if self.excluded_names.contains(&name) {
                continue;
            }
            // Such a name (a `C:` drive prefix) can never be a real entry,
            // so the pattern cannot match and skipping it changes nothing;
            // pushing it would replace the accumulated path and probe
            // outside the root.
            if !is_plain_component(name) {
                continue;
            }
            let path = dir.join(name);
            let is_dir = match fs::metadata(&path) {
                Ok(metadata) => metadata.is_dir(),
                Err(err) => {
                    report_fs_error(&path, &err);
                    false
                }
            };
            if !is_dir {
                continue;
            }
            let next = closure(patterns, next);
            self.walk(&path, &child_rel(rel, name), &next)?;
        }
        Ok(())
    }

    fn read_entries(
        &mut self,
        dir: &Path,
        rel: &str,
        dynamic: &[State],
        literals: &mut BTreeMap<&'a str, Vec<State>>,
    ) -> Result<()> {
        let patterns = self.patterns;
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            // An unreadable directory (permissions, a concurrent removal)
            // skips its whole subtree, like the upstream globbers do.
            Err(err) => {
                report_fs_error(dir, &err);
                return Ok(());
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    report_fs_error(dir, &err);
                    continue;
                }
            };
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                // Not a filesystem error, but the entry is invisible to
                // every pattern, which can drop a package just as silently.
                warn!(
                    "{}: the file name is not valid UTF-8",
                    display_path(&entry.path())
                );
                continue;
            };
            if self.excluded_names.contains(&name) {
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(err) => {
                    report_fs_error(&entry.path(), &err);
                    continue;
                }
            };
            let is_symlink = file_type.is_symlink();
            let is_dir = if is_symlink {
                match fs::metadata(entry.path()) {
                    Ok(metadata) => metadata.is_dir(),
                    Err(err) => {
                        report_fs_error(&entry.path(), &err);
                        false
                    }
                }
            } else {
                file_type.is_dir()
            };
            if !is_dir {
                continue;
            }
            // Consumed here rather than probed again, so that the subtree is
            // walked once with every state reaching it.
            let mut next = literals.remove(name).unwrap_or_default();
            let mut globstar_skipped = false;
            for &(pattern, seg_index) in dynamic {
                let seg = &patterns[pattern].segs()[seg_index];
                if !seg_matches(seg, name, false) {
                    continue;
                }
                // A literal or wildcard segment enters a symlinked directory
                // one level, as every package manager does; a globstar does
                // not, which keeps a symlink cycle finite: every other
                // consumption advances the index, bounding the descent by
                // the pattern length.
                if is_symlink && matches!(seg, Seg::Globstar) {
                    globstar_skipped = true;
                    continue;
                }
                let advanced = match seg {
                    Seg::Globstar => (pattern, seg_index),
                    _ => (pattern, seg_index + 1),
                };
                if !next.contains(&advanced) {
                    next.push(advanced);
                }
            }
            if next.is_empty() {
                // Reported only when nothing else descends: a literal or
                // wildcard segment elsewhere may still enter the symlink.
                // The path is built inside the macro so that this hot path
                // allocates nothing at the default level.
                if globstar_skipped {
                    debug!(
                        "{}: a symlinked directory is not entered by `**`",
                        child_rel(rel, name)
                    );
                }
                continue;
            }
            let next = closure(patterns, next);
            self.walk(&entry.path(), &child_rel(rel, name), &next)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::pattern;

    fn compile(patterns: &[&str]) -> (Vec<Pattern>, Vec<Pattern>) {
        let mut positives = Vec::new();
        let mut negations = Vec::new();
        for original in patterns {
            let (negated, compiled) = pattern::compile(original).unwrap().unwrap();
            if negated {
                negations.push(compiled);
            } else {
                positives.push(compiled);
            }
        }
        (positives, negations)
    }

    fn rel_dirs_with(root: &Path, patterns: &[&str], excluded_names: &[&str]) -> Vec<String> {
        let (positives, negations) = compile(patterns);
        collect(root, &positives, &negations, excluded_names, false)
            .unwrap()
            .into_keys()
            .collect()
    }

    fn rel_dirs(root: &Path, patterns: &[&str]) -> Vec<String> {
        rel_dirs_with(root, patterns, &["node_modules"])
    }

    fn touch(root: &Path, rel_manifest: &str) {
        let path = root.join(rel_manifest);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "{}").unwrap();
    }

    #[test]
    fn a_double_star_includes_the_base_directory() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "x/package.json");
        touch(dir.path(), "x/y/package.json");
        assert_eq!(rel_dirs(dir.path(), &["x/**"]), ["x", "x/y"]);
        assert_eq!(rel_dirs(dir.path(), &["x/**/*"]), ["x/y"]);
    }

    #[test]
    fn a_mid_pattern_double_star_matches_zero_or_more_segments() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "a/z/package.json");
        touch(dir.path(), "a/b/z/package.json");
        touch(dir.path(), "a/b/c/z/package.json");
        touch(dir.path(), "a/y/package.json");
        assert_eq!(
            rel_dirs(dir.path(), &["a/**/z"]),
            ["a/b/c/z", "a/b/z", "a/z"]
        );
    }

    #[test]
    fn a_doubled_double_star_matches_like_a_single_one() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "package.json");
        touch(dir.path(), "x/package.json");
        touch(dir.path(), "x/y/package.json");
        assert_eq!(rel_dirs(dir.path(), &["**/**"]), [".", "x", "x/y"]);
    }

    #[test]
    fn a_double_star_matches_a_nested_package() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "packages/a/package.json");
        touch(dir.path(), "packages/a/inner/package.json");
        assert_eq!(
            rel_dirs(dir.path(), &["packages/**"]),
            ["packages/a", "packages/a/inner"]
        );
    }

    #[test]
    fn a_negation_excludes_wildcard_and_literal_candidates() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "packages/a/package.json");
        touch(dir.path(), "packages/b/package.json");
        assert_eq!(
            rel_dirs(dir.path(), &["packages/*", "!packages/a"]),
            ["packages/b"]
        );
        assert_eq!(
            rel_dirs(dir.path(), &["packages/a", "!packages/a"]),
            [] as [String; 0]
        );
    }

    #[test]
    fn an_excluded_name_is_never_entered_nor_named() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "package.json");
        touch(dir.path(), "a/package.json");
        touch(dir.path(), "node_modules/evil/package.json");
        touch(dir.path(), "bower_components/old/package.json");
        touch(dir.path(), ".yarn/x/package.json");
        assert_eq!(
            rel_dirs(dir.path(), &["**"]),
            [".", "a", "bower_components/old"]
        );
        assert_eq!(
            rel_dirs(dir.path(), &["node_modules/evil"]),
            [] as [String; 0]
        );
        assert_eq!(rel_dirs(dir.path(), &["node_modules/*"]), [] as [String; 0]);
        let pnpm = ["node_modules", "bower_components"];
        assert_eq!(rel_dirs_with(dir.path(), &["**"], &pnpm), [".", "a"]);
        assert_eq!(
            rel_dirs_with(dir.path(), &["bower_components/old"], &pnpm),
            [] as [String; 0]
        );
        let yarn = ["node_modules", ".git", ".yarn"];
        assert_eq!(
            rel_dirs_with(dir.path(), &[".yarn/x"], &yarn),
            [] as [String; 0]
        );
        assert_eq!(rel_dirs(dir.path(), &[".yarn/x"]), [".yarn/x"]);
    }

    #[test]
    fn a_dotted_pattern_reaches_dot_directories() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), ".github/actions/x/package.json");
        touch(dir.path(), "examples/.hidden/y/package.json");
        touch(dir.path(), "examples/plain/z/package.json");
        assert_eq!(
            rel_dirs(dir.path(), &[".github/actions/*"]),
            [".github/actions/x"]
        );
        assert_eq!(
            rel_dirs(dir.path(), &["examples/.*/*"]),
            ["examples/.hidden/y"]
        );
        assert_eq!(
            rel_dirs(dir.path(), &["examples/*/*"]),
            ["examples/plain/z"]
        );
    }

    #[test]
    fn deduplicates_by_path() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "packages/a/package.json");
        touch(dir.path(), "packages/b/package.json");
        assert_eq!(
            rel_dirs(dir.path(), &["packages/a", "packages/*", "packages/**"]),
            ["packages/a", "packages/b"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_double_star_does_not_enter_a_symlinked_directory() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "packages/a/package.json");
        touch(dir.path(), "target/package.json");
        touch(dir.path(), "target/sub/package.json");
        std::os::unix::fs::symlink("../target", dir.path().join("packages/link")).unwrap();
        std::os::unix::fs::symlink(".", dir.path().join("packages/loop")).unwrap();
        assert_eq!(rel_dirs(dir.path(), &["packages/**"]), ["packages/a"]);
        assert_eq!(rel_dirs(dir.path(), &["**/sub"]), ["target/sub"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_wildcard_enters_a_symlinked_directory_one_level() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "packages/a/package.json");
        touch(dir.path(), "target/package.json");
        touch(dir.path(), "target/sub/package.json");
        std::os::unix::fs::symlink("../target", dir.path().join("packages/link")).unwrap();
        assert_eq!(
            rel_dirs(dir.path(), &["packages/*"]),
            ["packages/a", "packages/link"]
        );
        assert_eq!(
            rel_dirs(dir.path(), &["packages/*/sub"]),
            ["packages/link/sub"]
        );
        assert_eq!(
            rel_dirs(dir.path(), &["packages/*/**"]),
            ["packages/a", "packages/link", "packages/link/sub"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_cycle_is_entered_one_level_by_a_wildcard() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "packages/package.json");
        touch(dir.path(), "packages/a/package.json");
        std::os::unix::fs::symlink(".", dir.path().join("packages/loop")).unwrap();
        assert_eq!(
            rel_dirs(dir.path(), &["packages/*"]),
            ["packages/a", "packages/loop"]
        );
        assert_eq!(
            rel_dirs(dir.path(), &["packages/**"]),
            ["packages", "packages/a"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_prefixed_literal_is_never_pushed() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "packages/a/package.json");
        assert_eq!(
            rel_dirs(dir.path(), &["packages/a", "packages/C:/x", "./C:/x"]),
            ["packages/a"]
        );
        assert_eq!(
            rel_dirs(dir.path(), &["packages/*", "*/C:/x"]),
            ["packages/a"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_literal_matches_loosely_on_a_case_insensitive_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "a/lib/package.json");
        assert_eq!(rel_dirs(dir.path(), &["A/*"]), ["A/lib"]);
        assert_eq!(rel_dirs(dir.path(), &["*/Lib"]), ["a/Lib"]);
        assert_eq!(rel_dirs(dir.path(), &["A/Lib"]), ["A/Lib"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_drive_like_literal_is_an_ordinary_name_on_unix() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "C:/x/package.json");
        assert_eq!(rel_dirs(dir.path(), &["C:/x"]), ["C:/x"]);
    }

    #[test]
    fn a_permissive_negation_excludes_a_dot_directory_candidate() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), ".tools/a/package.json");
        assert_eq!(rel_dirs(dir.path(), &[".tools/a"]), [".tools/a"]);
        assert_eq!(
            rel_dirs(dir.path(), &[".tools/a", "!*/a"]),
            [] as [String; 0]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_double_star_descends_into_a_literally_entered_symlink() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "real/package.json");
        touch(dir.path(), "real/sub/package.json");
        std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
        assert_eq!(rel_dirs(dir.path(), &["link/**"]), ["link", "link/sub"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_literal_segment_sees_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "real/a/package.json");
        std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
        assert_eq!(rel_dirs(dir.path(), &["link/*"]), ["link/a"]);
    }

    #[test]
    fn a_pnpm_only_manifest_is_an_error_when_rejected() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "packages/a/package.yaml");
        touch(dir.path(), "packages/b/package.json");
        touch(dir.path(), "packages/b/package.json5");
        let (positives, negations) = compile(&["packages/*"]);
        let err = collect(dir.path(), &positives, &negations, &["node_modules"], true).unwrap_err();
        assert!(
            format!("{err:#}").contains("package.yaml: only package.json manifests are supported"),
            "{err:#}"
        );
        assert_eq!(rel_dirs(dir.path(), &["packages/*"]), ["packages/b"]);
        touch(dir.path(), "packages/a/package.json");
        let candidates =
            collect(dir.path(), &positives, &negations, &["node_modules"], true).unwrap();
        assert_eq!(
            candidates.into_keys().collect::<Vec<_>>(),
            ["packages/a", "packages/b"]
        );
    }
}
