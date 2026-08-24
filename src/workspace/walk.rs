use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use super::pattern::{Pattern, Seg, seg_matches};

/// Collects the member candidate directories under `root` matching
/// `positives` minus `negations`, keyed by the root-relative `/`-separated
/// directory (`.` for the root itself); reading the manifests is left to the
/// caller.
pub(crate) fn collect(
    root: &Path,
    positives: &[Pattern],
    negations: &[Pattern],
) -> BTreeMap<String, PathBuf> {
    let mut candidates = BTreeMap::new();
    let mut walked = Vec::new();
    for pattern in positives {
        if pattern.is_literal() {
            literal_fast_path(root, pattern, negations, &mut candidates);
        } else {
            walked.push(pattern);
        }
    }
    if !walked.is_empty() {
        let states = closure(
            &walked,
            (0..walked.len()).map(|pattern| (pattern, 0)).collect(),
        );
        walk(root, "", &walked, negations, &states, &mut candidates);
    }
    candidates
}

// An all-literal pattern needs no walk: checking the manifest path directly
// also gives literal segments their symlink transparency and dot matching
// for free. The fast path skips the traversal, not the rules, so the
// node_modules exclusion and the negations still apply.
fn literal_fast_path(
    root: &Path,
    pattern: &Pattern,
    negations: &[Pattern],
    candidates: &mut BTreeMap<String, PathBuf>,
) {
    let segs = pattern.segs();
    let mut dir = root.to_path_buf();
    let mut rel_parts = Vec::new();
    for seg in &segs[..segs.len() - 1] {
        let Seg::Literal(name) = seg else {
            unreachable!()
        };
        if name == "node_modules" {
            return;
        }
        dir.push(name);
        rel_parts.push(name.as_str());
    }
    if !dir.join("package.json").is_file() {
        return;
    }
    let rel_dir = if rel_parts.is_empty() {
        ".".to_owned()
    } else {
        rel_parts.join("/")
    };
    if excluded(&rel_dir, negations) {
        return;
    }
    candidates.entry(rel_dir).or_insert(dir);
}

fn excluded(rel_dir: &str, negations: &[Pattern]) -> bool {
    let rel_manifest = if rel_dir == "." {
        "package.json".to_owned()
    } else {
        format!("{rel_dir}/package.json")
    };
    negations
        .iter()
        .any(|negation| negation.matches(&rel_manifest, true))
}

// A walker state is a pattern (as an index into the walked patterns) and the
// index of its next unconsumed segment.
type State = (usize, usize);

// Adds the epsilon transitions: a globstar can consume zero segments, so a
// state resting on one also rests past it. The last segment is always
// `Literal("package.json")`, so `seg + 1` stays in bounds.
fn closure(patterns: &[&Pattern], mut states: Vec<State>) -> Vec<State> {
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

fn walk(
    dir: &Path,
    rel: &str,
    patterns: &[&Pattern],
    negations: &[Pattern],
    states: &[State],
    candidates: &mut BTreeMap<String, PathBuf>,
) {
    // The candidate check runs after the epsilon closure so that `x/**`
    // covers `x` itself.
    if states
        .iter()
        .any(|&(pattern, seg)| seg == patterns[pattern].segs().len() - 1)
    {
        let rel_dir = if rel.is_empty() { "." } else { rel };
        if !excluded(rel_dir, negations) && dir.join("package.json").is_file() {
            candidates
                .entry(rel_dir.to_owned())
                .or_insert_with(|| dir.to_path_buf());
        }
    }
    let Ok(entries) = fs::read_dir(dir) else {
        // An unreadable directory (permissions, a concurrent removal) skips
        // its whole subtree, like the upstream globbers do.
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if name == "node_modules" {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let is_symlink = file_type.is_symlink();
        let is_dir = if is_symlink {
            fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_dir())
        } else {
            file_type.is_dir()
        };
        if !is_dir {
            continue;
        }
        let mut next = Vec::new();
        for &(pattern, seg_index) in states {
            let segs = patterns[pattern].segs();
            // The final `package.json` segment names a file, so consuming it
            // with a directory entry can never lead to a candidate.
            if seg_index == segs.len() - 1 {
                continue;
            }
            let seg = &segs[seg_index];
            if !seg_matches(seg, name, false) {
                continue;
            }
            // A symlinked directory is only entered by consuming a literal
            // segment; wildcards and globstars do not see through it. The
            // literal consumption always advances the index, which bounds
            // symlink descent by the pattern length and keeps cycles safe.
            if is_symlink && !matches!(seg, Seg::Literal(_)) {
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
            continue;
        }
        let next = closure(patterns, next);
        let child_rel = if rel.is_empty() {
            name.to_owned()
        } else {
            format!("{rel}/{name}")
        };
        walk(
            &entry.path(),
            &child_rel,
            patterns,
            negations,
            &next,
            candidates,
        );
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
                negations.push(compiled);
            } else {
                positives.push(compiled);
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
    fn a_negation_excludes_walker_and_fast_path_candidates() {
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
        assert_eq!(rel_dirs(dir.path(), &["**"]), [".", "a"]);
        assert_eq!(
            rel_dirs(dir.path(), &["node_modules/evil"]),
            [] as [String; 0]
        );
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
    fn a_wildcard_matched_symlink_is_not_entered_nor_a_candidate() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "packages/a/package.json");
        touch(dir.path(), "target/package.json");
        touch(dir.path(), "target/sub/package.json");
        std::os::unix::fs::symlink("../target", dir.path().join("packages/link")).unwrap();
        std::os::unix::fs::symlink(".", dir.path().join("packages/loop")).unwrap();
        assert_eq!(rel_dirs(dir.path(), &["packages/**"]), ["packages/a"]);
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
