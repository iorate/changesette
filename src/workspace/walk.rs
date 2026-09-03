use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use tracing::{debug, warn};

use super::pattern::{Pattern, Seg, seg_matches};
use super::{probe_is_file, rel_dir_between, report_fs_error};

pub(crate) fn collect(
    root: &Path,
    positives: &[Pattern],
    negations: &[Pattern],
) -> BTreeMap<String, PathBuf> {
    let mut walker = Walker {
        root,
        patterns: positives,
        negations,
        candidates: BTreeMap::new(),
    };
    let mut groups: BTreeMap<usize, Vec<State>> = BTreeMap::new();
    for (index, pattern) in positives.iter().enumerate() {
        groups.entry(pattern.ascend()).or_default().push((index, 0));
    }
    for (ascend, states) in groups {
        let Some(dir) = ancestor(root, ascend) else {
            debug!(
                "{}: {ascend} leading `..` climb past the filesystem root",
                root.display()
            );
            continue;
        };
        let rel = vec![".."; ascend].join("/");
        let states = closure(positives, states);
        walker.walk(dir, &rel, &states);
    }
    walker.candidates
}

fn ancestor(root: &Path, ascend: usize) -> Option<&Path> {
    let mut dir = root;
    for _ in 0..ascend {
        dir = dir.parent()?;
    }
    Some(dir)
}

// The manifest probe runs before this, so the debug line only names real
// candidates.
fn excluded(rel_dir: &str, negations: &[Pattern]) -> bool {
    let excluded = negations
        .iter()
        .any(|negation| negation.matches(rel_dir, true));
    if excluded {
        debug!("{rel_dir}: excluded by a negative workspace pattern");
    }
    excluded
}

// A walker state is a pattern (as an index into the patterns) and the index
// of its next unconsumed segment.
type State = (usize, usize);

// Adds the epsilon transitions: a globstar can consume zero segments, so a
// state resting on one also rests past it. A state at `segs().len()` is the
// accepting sentinel: the whole pattern is consumed.
fn closure(patterns: &[Pattern], mut states: Vec<State>) -> Vec<State> {
    let mut i = 0;
    while i < states.len() {
        let (pattern, seg) = states[i];
        if seg < patterns[pattern].segs().len()
            && matches!(patterns[pattern].segs()[seg], Seg::Globstar)
        {
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
    root: &'a Path,
    patterns: &'a [Pattern],
    negations: &'a [Pattern],
    candidates: BTreeMap<String, PathBuf>,
}

impl Walker<'_> {
    fn walk(&mut self, dir: &Path, rel: &str, states: &[State]) {
        let patterns = self.patterns;
        // The candidate check runs after the epsilon closure so that `x/**`
        // covers `x` itself.
        if states
            .iter()
            .any(|&(pattern, seg)| seg == patterns[pattern].segs().len())
        {
            // The lexical path from the root rather than the spelling the
            // walk arrived by: a detour through `..` back into the root
            // collapses into the direct spelling, and the negations see the
            // same spelling that `dir` reports.
            let rel_dir = rel_dir_between(self.root, dir);
            if probe_is_file(&dir.join("package.json")) && !excluded(&rel_dir, self.negations) {
                self.candidates
                    .entry(rel_dir)
                    .or_insert_with(|| dir.to_path_buf());
            }
        }
        // The accepting sentinel has nothing left to consume. Every other
        // segment, a literal name included, is matched against the
        // directory entries: comparing the names rather than probing a
        // literal by `stat` keeps it exact on a case-insensitive filesystem.
        let pending: Vec<State> = states
            .iter()
            .copied()
            .filter(|&(pattern, seg_index)| seg_index != patterns[pattern].segs().len())
            .collect();
        if pending.is_empty() {
            return;
        }
        self.read_entries(dir, rel, &pending);
    }

    fn read_entries(&mut self, dir: &Path, rel: &str, pending: &[State]) {
        let patterns = self.patterns;
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            // An unreadable directory (permissions, a concurrent removal)
            // skips its whole subtree rather than aborting the walk.
            Err(err) => {
                report_fs_error(dir, &err);
                return;
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
                    entry.path().display()
                );
                continue;
            };
            if name == "node_modules" {
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
            // The subtree is walked once with every state reaching it.
            let mut next = Vec::new();
            let mut globstar_skipped = false;
            for &(pattern, seg_index) in pending {
                let seg = &patterns[pattern].segs()[seg_index];
                if !seg_matches(seg, name, false) {
                    continue;
                }
                // A single-segment glob enters a symlinked directory one
                // level; a globstar does not, which keeps a symlink cycle
                // finite: every other consumption advances the index,
                // bounding the descent by the pattern length.
                if is_symlink && matches!(seg, Seg::Globstar) {
                    globstar_skipped = true;
                    continue;
                }
                let advanced = match seg {
                    Seg::Globstar => (pattern, seg_index),
                    Seg::Glob(_) => (pattern, seg_index + 1),
                };
                if !next.contains(&advanced) {
                    next.push(advanced);
                }
            }
            if next.is_empty() {
                // Reported only when nothing else descends: a single-segment
                // glob elsewhere may still enter the symlink.
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
            self.walk(&entry.path(), &child_rel(rel, name), &next);
        }
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
            let (negated, compiled) = pattern::compile(original).unwrap();
            if negated {
                negations.extend(compiled);
            } else {
                positives.extend(compiled);
            }
        }
        (positives, negations)
    }

    fn rel_dirs(root: &Path, patterns: &[&str]) -> Vec<String> {
        let (positives, negations) = compile(patterns);
        collect(root, &positives, &negations).into_keys().collect()
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
    fn node_modules_is_never_entered_nor_named() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "package.json");
        touch(dir.path(), "a/package.json");
        touch(dir.path(), "node_modules/evil/package.json");
        touch(dir.path(), "bower_components/old/package.json");
        touch(dir.path(), ".yarn/x/package.json");
        touch(dir.path(), ".git/c/package.json");
        assert_eq!(
            rel_dirs(dir.path(), &["**"]),
            [".", "a", "bower_components/old"]
        );
        assert_eq!(
            rel_dirs(dir.path(), &["node_modules/evil"]),
            [] as [String; 0]
        );
        assert_eq!(rel_dirs(dir.path(), &["node_modules/*"]), [] as [String; 0]);
        assert_eq!(rel_dirs(dir.path(), &[".yarn/x"]), [".yarn/x"]);
        assert_eq!(rel_dirs(dir.path(), &[".git/c"]), [".git/c"]);
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
    fn a_drive_prefixed_segment_matches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "packages/a/package.json");
        assert_eq!(
            rel_dirs(
                dir.path(),
                &["packages/a", "packages/C:/x", "./C:/x", "C:/x", "C:*"]
            ),
            ["packages/a"]
        );
        assert_eq!(
            rel_dirs(dir.path(), &["packages/*", "*/C:/x"]),
            ["packages/a"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_literal_matches_exactly_on_a_case_insensitive_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "a/lib/package.json");
        assert_eq!(rel_dirs(dir.path(), &["A/*"]), [] as [String; 0]);
        assert_eq!(rel_dirs(dir.path(), &["*/Lib"]), [] as [String; 0]);
        assert_eq!(rel_dirs(dir.path(), &["A/Lib"]), [] as [String; 0]);
        assert_eq!(rel_dirs(dir.path(), &["a/lib"]), ["a/lib"]);
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
}
