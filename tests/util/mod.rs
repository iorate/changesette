use std::{collections::BTreeMap, fs, path::Path};

/// Writes a changeset naming the given packages under `dir/.changeset/`,
/// creating the directory if needed. An empty `releases` produces an empty
/// frontmatter.
pub(crate) fn write_changeset(
    dir: &Path,
    file_name: &str,
    releases: &[(&str, &str)],
    summary: &str,
) {
    write_changeset_in(&dir.join(".changeset"), file_name, releases, summary);
}

pub(crate) fn write_pre_changeset(
    dir: &Path,
    file_name: &str,
    releases: &[(&str, &str)],
    summary: &str,
) {
    write_changeset_in(&dir.join(".changeset/pre"), file_name, releases, summary);
}

fn write_changeset_in(
    changeset_dir: &Path,
    file_name: &str,
    releases: &[(&str, &str)],
    summary: &str,
) {
    fs::create_dir_all(changeset_dir).unwrap();
    let frontmatter: String = releases
        .iter()
        .map(|(name, bump)| format!("\"{name}\": {bump}\n"))
        .collect();
    fs::write(
        changeset_dir.join(file_name),
        format!("---\n{frontmatter}---\n\n{summary}\n"),
    )
    .unwrap();
}

/// Captures every file under `dir` (recursively) as a relative-path-to-bytes
/// map, for asserting that a command left the tree untouched.
pub(crate) fn dir_snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(dir, dir, &mut files);
    files
}
