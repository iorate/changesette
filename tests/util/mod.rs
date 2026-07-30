use std::{collections::BTreeMap, fs, path::Path};

/// Writes a changeset for the package `ublacklist` under `dir/.changeset/`,
/// creating the directory if needed.
pub(crate) fn write_changeset(dir: &Path, file_name: &str, bump: &str, summary: &str) {
    let changeset_dir = dir.join(".changeset");
    fs::create_dir_all(&changeset_dir).unwrap();
    fs::write(
        changeset_dir.join(file_name),
        format!("---\n\"ublacklist\": {bump}\n---\n\n{summary}\n"),
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
