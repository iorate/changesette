use std::ops::Range;

use anyhow::{Context, Result};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use semver::Version;

use crate::bump::Bump;

/// A changelog entry derived from one changeset.
#[derive(Debug)]
pub(crate) struct Entry {
    /// The pull request numbers to link, in link order.
    pub(crate) prs: Vec<u64>,
    /// The entry's Markdown body (the changeset summary).
    pub(crate) body: String,
}

/// Renders a `## <version>` section, grouping `entries` under Major/Minor/
/// Patch headings and omitting empty groups. `repository` (`owner/repo`)
/// enables PR links. The result has no surrounding newlines.
pub(crate) fn render_section(
    version: &Version,
    entries: &[(Bump, Entry)],
    repository: Option<&str>,
) -> String {
    let mut blocks = vec![format!("## {version}")];
    for (bump, heading) in [
        (Bump::Major, "### Major Changes"),
        (Bump::Minor, "### Minor Changes"),
        (Bump::Patch, "### Patch Changes"),
    ] {
        let group = entries.iter().filter(|(b, _)| *b == bump);
        let mut has_heading = false;
        for (_, entry) in group {
            if !has_heading {
                blocks.push(heading.to_owned());
                has_heading = true;
            }
            blocks.push(render_entry(entry, repository));
        }
    }
    blocks.join("\n\n")
}

fn render_entry(entry: &Entry, repository: Option<&str>) -> String {
    let mut text = String::from("- ");
    if let Some(repository) = repository {
        for pr in &entry.prs {
            text.push_str(&format!(
                "[#{pr}](https://github.com/{repository}/pull/{pr}) "
            ));
        }
        if !entry.prs.is_empty() {
            text.push_str("- ");
        }
    }
    let mut lines = entry.body.lines();
    text.push_str(lines.next().unwrap_or_default());
    for line in lines {
        text.push('\n');
        if !line.trim().is_empty() {
            text.push_str("  ");
            text.push_str(line);
        }
    }
    text
}

/// Inserts `section` (a `render_section` result whose first line is
/// `## <version>`) into the CHANGELOG text and returns the new text.
///
/// The version sections of a changelog are delimited by top-level `##`
/// headings. The new section goes right before the first existing section, so
/// an H1 title and any preamble prose stay above it; a document without
/// sections gets the new section appended at the end. A section of the same
/// version, if present, is removed first, making a re-run with the same inputs
/// idempotent. Passing `text = ""` generates a well-formed new file.
///
/// The surroundings of the insertion/removal points are spliced with
/// single-blank-line separation and a single trailing newline; everything else
/// is copied verbatim, byte for byte.
pub(crate) fn upsert_section(
    text: &str,
    package_name: &str,
    version: &str,
    section: &str,
) -> String {
    // The collected byte ranges are invalidated whenever `text` changes, so
    // each mutation below re-parses; the happy path parses only once.
    let mut text = text.to_owned();
    let mut headings = parse_headings(&text);

    // Remove existing sections of the same version. Normally there is at most
    // one.
    while let Some(index) = find_h2(&headings, version) {
        let end = next_h2_start(&headings, index, text.len());
        text.replace_range(headings[index].range.start..end, "");
        headings = parse_headings(&text);
    }

    // A changelog is expected to open with an H1 title (`# <package_name>`);
    // supplement one if the document does not start with an H1 (counting
    // documents with no headings at all, and empty ones).
    let has_top_h1 = headings.first().is_some_and(|heading| {
        heading.level == HeadingLevel::H1 && text[..heading.range.start].trim().is_empty()
    });
    if !has_top_h1 {
        text = format!("# {package_name}\n\n{text}");
        headings = parse_headings(&text);
    }

    // Insert before the first section, or at the end if there is none,
    // normalizing to one blank line between the new section and each
    // neighbor. `before` holds at least the H1, so the blank line after it
    // never turns into a leading newline; `section` has no surrounding
    // newlines, so the result ends with exactly one.
    let position = headings
        .iter()
        .find(|heading| heading.level == HeadingLevel::H2)
        .map_or(text.len(), |heading| heading.range.start);
    let (before, after) = text.split_at(position);
    let mut result = String::new();
    result.push_str(before.trim_end_matches('\n'));
    result.push_str("\n\n");
    result.push_str(section);
    result.push('\n');
    if !after.is_empty() {
        result.push('\n');
        result.push_str(after);
    }
    result
}

/// Returns the body of the `## <version>` section — the text between that
/// heading and the next top-level `##` heading (or the end of the document) —
/// with surrounding blank lines trimmed and no trailing newline. Errors if
/// the section is not found.
pub(crate) fn extract_section(text: &str, version: &str) -> Result<String> {
    let headings = parse_headings(text);
    let index = find_h2(&headings, version)
        .with_context(|| format!("version {version} not found in CHANGELOG.md"))?;
    let end = next_h2_start(&headings, index, text.len());
    Ok(trim_blank_lines(&text[headings[index].range.end..end]).to_owned())
}

struct Heading {
    level: HeadingLevel,
    // Byte range of the whole heading in the source text, trailing newline
    // included (pinned by a unit test).
    range: Range<usize>,
    // The heading's inner text with inline markup dropped, e.g. "1.0.0".
    text: String,
}

// Parses the whole document and collects its top-level headings in source
// order.
//
// Positions come from the parser's byte ranges, never from line scanning, so
// a `## ...` line inside a code block is not mistaken for a heading. Headings
// nested inside a container (list item or block quote) are skipped: an entry
// body's indented `## ...` continuation line parses as a heading *inside* the
// entry's list item, and must not act as a section boundary.
fn parse_headings(text: &str) -> Vec<Heading> {
    let mut headings = Vec::new();
    let mut container_depth = 0usize;
    // The heading currently being collected; None also while inside a nested
    // (skipped) heading, whose Text/Code events must not be picked up.
    let mut current: Option<Heading> = None;
    for (event, range) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(Tag::List(_) | Tag::Item | Tag::BlockQuote(_)) => container_depth += 1,
            Event::End(TagEnd::List(_) | TagEnd::Item | TagEnd::BlockQuote(_)) => {
                container_depth -= 1;
            }
            Event::Start(Tag::Heading { level, .. }) if container_depth == 0 => {
                current = Some(Heading {
                    level,
                    range,
                    text: String::new(),
                });
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(heading) = &mut current {
                    heading.text.push_str(&text);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                headings.extend(current.take());
            }
            _ => {}
        }
    }
    headings
}

fn find_h2(headings: &[Heading], text: &str) -> Option<usize> {
    headings
        .iter()
        .position(|heading| heading.level == HeadingLevel::H2 && heading.text == text)
}

// The section starting at `headings[index]` ends where the next H2 begins,
// or at `end_of_text` if it is the last section.
fn next_h2_start(headings: &[Heading], index: usize, end_of_text: usize) -> usize {
    headings[index + 1..]
        .iter()
        .find(|heading| heading.level == HeadingLevel::H2)
        .map_or(end_of_text, |heading| heading.range.start)
}

// Trims leading and trailing blank (empty or whitespace-only) lines, plus the
// trailing newline of the last remaining line. Unlike `str::trim`, keeps the
// indentation and trailing whitespace of non-blank lines.
fn trim_blank_lines(mut text: &str) -> &str {
    while let Some(index) = text.find('\n') {
        if !text[..index].trim().is_empty() {
            break;
        }
        text = &text[index + 1..];
    }
    while let Some(index) = text.rfind('\n') {
        if !text[index + 1..].trim().is_empty() {
            break;
        }
        text = &text[..index];
    }
    // All-blank text without a newline falls through the loops above.
    if text.trim().is_empty() { "" } else { text }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;
    use crate::bump::Bump;

    fn entry(prs: &[u64], body: &str) -> (Bump, Entry) {
        (
            Bump::Minor,
            Entry {
                prs: prs.to_vec(),
                body: body.to_owned(),
            },
        )
    }

    fn render(entries: &[(Bump, Entry)], repository: Option<&str>) -> String {
        render_section(&"10.1.0".parse().unwrap(), entries, repository)
    }

    fn read_fixture(area: &str, case: &str) -> String {
        fs::read_to_string(
            Path::new("tests/fixtures")
                .join(area)
                .join(case)
                .join("CHANGELOG.md"),
        )
        .unwrap()
    }

    fn upsert(case: &str, version: &str) -> String {
        let section = render_section(
            &version.parse().unwrap(),
            &[entry(&[42], "Add SERPINFO satellites support")],
            Some("iorate/ublacklist"),
        );
        upsert_section(
            &read_fixture("changelog-insert", case),
            "ublacklist",
            version,
            &section,
        )
    }

    fn extract(case: &str, version: &str) -> Result<String> {
        extract_section(&read_fixture("changelog-extract", case), version)
    }

    #[test]
    fn section_has_no_leading_or_trailing_newline() {
        let section = render(&[entry(&[12], "Add SERPINFO satellites support")], None);
        assert!(!section.starts_with('\n'));
        assert!(!section.ends_with('\n'));
    }

    #[test]
    fn renders_multiple_pr_links() {
        insta::assert_snapshot!(render(
            &[entry(&[12, 23], "Add SERPINFO satellites support")],
            Some("iorate/ublacklist"),
        ));
    }

    #[test]
    fn renders_an_entry_without_prs() {
        insta::assert_snapshot!(render(
            &[entry(&[], "Add SERPINFO satellites support")],
            Some("iorate/ublacklist"),
        ));
    }

    #[test]
    fn renders_without_links_when_integration_is_off() {
        insta::assert_snapshot!(render(
            &[entry(&[12, 23], "Add SERPINFO satellites support")],
            None,
        ));
    }

    #[test]
    fn indents_multi_line_bodies() {
        insta::assert_snapshot!(render(
            &[entry(
                &[12],
                "First line of body\nsecond line of body\n\nline after a blank line",
            )],
            Some("iorate/ublacklist"),
        ));
    }

    #[test]
    fn orders_groups_and_omits_empty_ones() {
        let entries = [
            (
                Bump::Patch,
                Entry {
                    prs: vec![23],
                    body: "First patch change".to_owned(),
                },
            ),
            (
                Bump::Major,
                Entry {
                    prs: vec![12],
                    body: "Major change".to_owned(),
                },
            ),
            (
                Bump::Patch,
                Entry {
                    prs: vec![],
                    body: "Second patch change".to_owned(),
                },
            ),
        ];
        insta::assert_snapshot!(render(&entries, Some("iorate/ublacklist")));
    }

    #[test]
    fn heading_ranges_cover_the_whole_line() {
        let text = "# ublacklist\n\n## 1.0.0\n\nbody\n";
        let headings = parse_headings(text);
        assert_eq!(&text[headings[0].range.clone()], "# ublacklist\n");
        assert_eq!(&text[headings[1].range.clone()], "## 1.0.0\n");
    }

    #[test]
    fn upserted_text_ends_with_a_single_newline() {
        for case in ["preamble", "no-h2", "empty"] {
            let result = upsert(case, "1.0.0");
            assert!(result.ends_with('\n'));
            assert!(!result.ends_with("\n\n"));
        }
    }

    #[test]
    fn inserts_before_the_first_h2_keeping_the_preamble() {
        insta::assert_snapshot!(upsert("preamble", "1.0.0"));
    }

    #[test]
    fn supplements_a_missing_h1() {
        insta::assert_snapshot!(upsert("no-h1", "1.0.0"));
    }

    #[test]
    fn appends_at_the_end_without_an_h2() {
        insta::assert_snapshot!(upsert("no-h2", "1.0.0"));
    }

    #[test]
    fn replaces_an_existing_section_of_the_same_version() {
        insta::assert_snapshot!(upsert("same-version-exists", "1.0.0"));
    }

    #[test]
    fn ignores_h2_lines_inside_a_code_block() {
        insta::assert_snapshot!(upsert("code-block-hashes", "1.0.0"));
    }

    #[test]
    fn ignores_an_h2_inside_a_list_item() {
        insta::assert_snapshot!(upsert("h2-inside-list-item", "1.0.0"));
    }

    #[test]
    fn generates_a_new_file_from_empty_text() {
        insta::assert_snapshot!(upsert("empty", "1.0.0"));
    }

    #[test]
    fn extends_the_real_ublacklist_changelog() {
        insta::assert_snapshot!(upsert("ublacklist-head", "10.1.0"));
    }

    #[test]
    fn extracts_the_first_section() {
        insta::assert_snapshot!(extract("basic", "2.0.0").unwrap());
    }

    #[test]
    fn extracts_a_later_section() {
        insta::assert_snapshot!(extract("basic", "1.0.0").unwrap());
    }

    #[test]
    fn keeps_a_code_block_inside_a_section() {
        insta::assert_snapshot!(extract("code-block", "2.0.0").unwrap());
    }

    #[test]
    fn rejects_a_version_not_found() {
        insta::assert_snapshot!(format!("{:#}", extract("basic", "3.0.0").unwrap_err()));
    }

    #[test]
    fn rejects_a_changelog_without_sections() {
        insta::assert_snapshot!(format!("{:#}", extract("no-h2", "1.0.0").unwrap_err()));
    }
}
