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
