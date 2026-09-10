#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs, io,
    path::Path,
    sync::{Arc, Mutex},
};

use changesette::{output::Formatter, workspace::Workspace};
use tempfile::TempDir;
use tracing::level_filters::LevelFilter;

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
    let frontmatter = releases.iter().fold(String::new(), |mut s, (name, bump)| {
        let _ = writeln!(s, "\"{name}\": {bump}");
        s
    });
    fs::write(
        changeset_dir.join(file_name),
        format!("---\n{frontmatter}---\n\n{summary}\n"),
    )
    .unwrap();
}

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

pub(crate) fn expected_path(dir: &Path, rel: &str) -> String {
    let mut path = dunce::canonicalize(dir).unwrap();
    path.extend(rel.split('/').filter(|part| !part.is_empty()));
    path.display().to_string()
}

pub(crate) fn package_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"ublacklist\",\n  \"version\": \"1.2.3\"\n}\n",
    )
    .unwrap();
    dir
}

pub(crate) fn workspace_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"workspaces\": [\"packages/*\"]\n}\n",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("packages/a")).unwrap();
    fs::write(
        dir.path().join("packages/a/package.json"),
        "{\n  \"name\": \"pkg-a\",\n  \"version\": \"3.1.4\"\n}\n",
    )
    .unwrap();
    dir
}

pub(crate) fn two_package_workspace_dir() -> TempDir {
    let dir = workspace_dir();
    fs::create_dir_all(dir.path().join("packages/b")).unwrap();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.0\"\n}\n",
    )
    .unwrap();
    dir
}

pub(crate) fn private_two_package_workspace_dir() -> TempDir {
    let dir = workspace_dir();
    fs::create_dir_all(dir.path().join("packages/b")).unwrap();
    fs::write(
        dir.path().join("packages/b/package.json"),
        "{\n  \"name\": \"pkg-b\",\n  \"version\": \"2.0.0\",\n  \"private\": true\n}\n",
    )
    .unwrap();
    dir
}

pub(crate) fn write_config(dir: &Path, text: &str) {
    fs::create_dir_all(dir.join(".changeset")).unwrap();
    fs::write(dir.join(".changeset/config.json"), text).unwrap();
}

pub(crate) fn write_pre_json(dir: &Path, text: &str) {
    let changeset_dir = dir.join(".changeset");
    fs::create_dir_all(&changeset_dir).unwrap();
    fs::write(changeset_dir.join("pre.json"), text).unwrap();
}

pub(crate) fn read_pre_json(dir: &Path) -> String {
    fs::read_to_string(dir.join(".changeset/pre.json")).unwrap()
}

pub(crate) fn prerelease_package_dir(version: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("package.json"),
        format!("{{\n  \"name\": \"ublacklist\",\n  \"version\": \"{version}\"\n}}\n"),
    )
    .unwrap();
    dir
}

pub(crate) fn manifest_version(dir: &Path, rel: &str) -> String {
    let text = fs::read_to_string(dir.join(rel)).unwrap();
    let start = text.find("\"version\": \"").unwrap() + "\"version\": \"".len();
    let len = text[start..].find('"').unwrap();
    text[start..start + len].to_owned()
}

pub(crate) fn capture_output(f: impl FnOnce()) -> String {
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    // tracing-core caches each callsite's interest on its first hit, and with
    // at most one registered dispatcher it computes that interest from the
    // hitting thread's own dispatcher. A parallel test without a subscriber
    // would thus cache `never` for a callsite and starve the capture here, so
    // a global DEBUG-level sink keeps every callsite enabled while the
    // thread-local `with_default` below decides where the events go.
    let _ = tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .event_format(Formatter)
            .with_max_level(LevelFilter::DEBUG)
            .with_writer(io::sink)
            .finish(),
    );
    let buffer = Buffer::default();
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .event_format(Formatter)
        .with_max_level(LevelFilter::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    let bytes = buffer.0.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

pub(crate) fn write_file(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

pub(crate) fn names_and_rel_dirs(workspace: &Workspace) -> Vec<(&str, &str)> {
    workspace
        .members()
        .iter()
        .map(|member| (member.name(), member.rel_dir()))
        .collect()
}
