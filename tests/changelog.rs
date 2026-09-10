use std::{fs, path::Path};

use anyhow::Result;
use changesette::{
    bump::Bump,
    changelog::{extract_section, render_entry, render_section, upsert_section},
};

const ENTRY: &str = "### Minor Changes\n\n- Add SERPINFO satellites support";

fn render(summaries: &[(Bump, &str)]) -> String {
    render_section(&"10.1.0".parse().unwrap(), &render_entry(summaries))
}

fn read_fixture(area: &str, case: &str, file: &str) -> String {
    fs::read_to_string(Path::new("tests/fixtures").join(area).join(case).join(file)).unwrap()
}

fn upsert(case: &str, version: &str) -> String {
    let section = render_section(
        &version.parse().unwrap(),
        &render_entry(&[(Bump::Minor, "Add SERPINFO satellites support")]),
    );
    upsert_section(
        &read_fixture("changelog-insert", case, "CHANGELOG.md"),
        "ublacklist",
        version,
        &section,
    )
}

fn extract(case: &str, version: &str) -> Result<String> {
    extract_section(
        &read_fixture("changelog-extract", case, "CHANGELOG.md"),
        version,
    )
}

#[test]
fn rendered_text_has_exact_newlines() {
    let section = render(&[(Bump::Minor, "Add SERPINFO satellites support")]);
    assert!(!section.starts_with('\n'));
    assert!(!section.ends_with('\n'));
    for case in ["preamble", "no-h2", "empty"] {
        let result = upsert(case, "1.0.0");
        assert!(result.ends_with('\n'), "{case}");
        assert!(!result.ends_with("\n\n"), "{case}");
    }
}

#[test]
fn indents_multi_line_bodies() {
    assert_eq!(
        render(&[(
            Bump::Minor,
            "First line of body\nsecond line of body\n\nline after a blank line",
        )]),
        "## 10.1.0\n\n### Minor Changes\n\n- First line of body\n  second line of body\n\n  line after a blank line"
    );
}

#[test]
fn orders_groups_and_omits_empty_ones() {
    assert_eq!(
        render(&[
            (Bump::Patch, "First patch change"),
            (Bump::Major, "Major change"),
            (Bump::Patch, "Second patch change"),
        ]),
        "## 10.1.0\n\n### Major Changes\n\n- Major change\n\n### Patch Changes\n\n- First patch change\n\n- Second patch change"
    );
}

#[test]
fn renders_an_empty_summary_as_a_bare_bullet() {
    let section = render_section(
        &"1.3.0".parse().unwrap(),
        &render_entry(&[(Bump::Minor, "")]),
    );
    assert_eq!(
        upsert_section("", "ublacklist", "1.3.0", &section),
        "# ublacklist\n\n## 1.3.0\n\n### Minor Changes\n\n- \n"
    );
}

#[test]
fn inserts_before_the_first_h2_keeping_the_preamble() {
    assert_eq!(
        upsert("preamble", "1.0.0"),
        format!(
            "# ublacklist\n\nAll notable changes to this project are documented in this file.\n\n## 1.0.0\n\n{ENTRY}\n\n## 0.9.0\n\n### Minor Changes\n\n- Add something\n\n## 0.8.0\n\n### Patch Changes\n\n- Fix something\n"
        )
    );
}

#[test]
fn supplements_a_missing_h1() {
    assert_eq!(
        upsert("no-h1", "1.0.0"),
        format!(
            "# ublacklist\n\n## 1.0.0\n\n{ENTRY}\n\n## 0.9.0\n\n### Minor Changes\n\n- Add something\n"
        )
    );
}

#[test]
fn appends_at_the_end_without_an_h2() {
    assert_eq!(
        upsert("no-h2", "1.0.0"),
        format!(
            "# ublacklist\n\nAll notable changes to this project are documented in this file.\n\n## 1.0.0\n\n{ENTRY}\n"
        )
    );
}

#[test]
fn replaces_an_existing_section_of_the_same_version() {
    assert_eq!(
        upsert("same-version-exists", "1.0.0"),
        format!(
            "# ublacklist\n\n## 1.0.0\n\n{ENTRY}\n\n## 0.9.0\n\n### Patch Changes\n\n- Fix something\n"
        )
    );
}

#[test]
fn ignores_h2_lines_inside_a_code_block() {
    assert_eq!(
        upsert("code-block-hashes", "1.0.0"),
        format!(
            "# ublacklist\n\nExample changelog section:\n\n```md\n## 9.9.9\n\nNot a real section.\n```\n\n## 1.0.0\n\n{ENTRY}\n\n## 0.9.0\n\n### Patch Changes\n\n- Fix something\n"
        )
    );
}

#[test]
fn ignores_an_h2_inside_a_list_item() {
    assert_eq!(
        upsert("h2-inside-list-item", "1.0.0"),
        format!(
            "# ublacklist\n\n## 1.0.0\n\n{ENTRY}\n\n## 0.9.0\n\n### Patch Changes\n\n- Fix something\n"
        )
    );
}

#[test]
fn generates_a_new_file_from_empty_text() {
    assert_eq!(
        upsert("empty", "1.0.0"),
        format!("# ublacklist\n\n## 1.0.0\n\n{ENTRY}\n")
    );
}

#[test]
fn sees_the_h1_behind_a_bom_and_keeps_the_bom() {
    let result = upsert_section(
        "\u{feff}# ublacklist\n\n## 1.0.0\n",
        "ublacklist",
        "1.1.0",
        "## 1.1.0",
    );
    assert_eq!(result, "\u{feff}# ublacklist\n\n## 1.1.0\n\n## 1.0.0\n");
}

#[test]
fn extends_the_real_ublacklist_changelog() {
    assert_eq!(
        upsert("ublacklist-head", "10.1.0"),
        read_fixture("changelog-insert", "ublacklist-head", "expected.md")
    );
}

#[test]
fn extracts_the_first_section() {
    assert_eq!(
        extract("basic", "2.0.0").unwrap(),
        "### Major Changes\n\n- Break something"
    );
}

#[test]
fn extracts_a_later_section() {
    assert_eq!(
        extract("basic", "1.0.0").unwrap(),
        "### Minor Changes\n\n- Add something"
    );
}

#[test]
fn keeps_a_code_block_inside_a_section() {
    assert_eq!(
        extract("code-block", "2.0.0").unwrap(),
        "### Minor Changes\n\n- Document the changelog format\n\n```md\n## 9.9.9\n```"
    );
}

#[test]
fn extracts_a_prerelease_section() {
    let section = extract_section(
        "# ublacklist\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n",
        "1.3.0-beta.0",
    )
    .unwrap();
    assert_eq!(section, "### Minor Changes\n\n- Add feature");
}

#[test]
fn extracts_an_empty_section_as_empty() {
    let section = extract_section(
        "# ublacklist\n\n## 1.3.0\n\n## 1.3.0-beta.0\n\n### Minor Changes\n\n- Add feature\n",
        "1.3.0",
    )
    .unwrap();
    assert_eq!(section, "");
}

#[test]
fn extracts_a_section_behind_a_bom() {
    let section = extract_section("\u{feff}## 1.0.0\n\nbody\n", "1.0.0").unwrap();
    assert_eq!(section, "body");
}

#[test]
fn rejects_a_missing_version() {
    assert!(extract("basic", "3.0.0").is_err());
    assert!(extract("no-h2", "1.0.0").is_err());
}
