use changesette::catalog::CatalogYaml;

fn keys(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|key| (*key).to_owned()).collect()
}

#[test]
fn catalog_yaml_replaces_scalars_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pnpm-workspace.yaml");
    std::fs::write(
        &path,
        "\u{feff}# カタログ\npackages:\n  - 'packages/*'\ncatalog:\n  pkg-a: \"^1.0.0\" # pinned\n  pkg-b: '~1.0.0'\n  pkg-c: >-\n    ^1.0.0\ncatalogs:\n  legacy: &legacy\n    pkg-a: \"1.0.0\"\n  alias: *legacy\n",
    )
    .unwrap();
    let mut yaml = CatalogYaml::load(&path).unwrap();
    yaml.set(&keys(&["catalog", "pkg-a"]), "^1.0.1").unwrap();
    yaml.set(&keys(&["catalog", "pkg-b"]), ">=1.0.1 <2.0.0")
        .unwrap();
    yaml.set(&keys(&["catalogs", "legacy", "pkg-a"]), "1.0.1")
        .unwrap();
    yaml.set(&keys(&["catalogs", "alias", "pkg-a"]), "1.0.1")
        .unwrap();
    assert!(yaml.set(&keys(&["catalog", "pkg-c"]), "1.0.1").is_err());
    assert!(yaml.set(&keys(&["catalog", "pkg-d"]), "1.0.1").is_err());
    assert!(yaml.set(&keys(&["catalogs", "legacy"]), "1.0.1").is_err());
    assert_eq!(yaml.path(), path);
    assert_eq!(
        yaml.text(),
        "\u{feff}# カタログ\npackages:\n  - 'packages/*'\ncatalog:\n  pkg-a: ^1.0.1\n  pkg-b: \">=1.0.1 <2.0.0\"\n  pkg-c: >-\n    ^1.0.0\ncatalogs:\n  legacy: &legacy\n    pkg-a: 1.0.1\n  alias: *legacy\n"
    );
}
