use std::ops::Range;

use anyhow::{Context, Result};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use semver::Version;

use crate::bump::Bump;

pub(crate) fn render_entry(summaries: &[(Bump, &str)]) -> String {
    let mut blocks = Vec::new();
    for (bump, heading) in [
        (Bump::Major, "### Major Changes"),
        (Bump::Minor, "### Minor Changes"),
        (Bump::Patch, "### Patch Changes"),
    ] {
        let group = summaries.iter().filter(|(b, _)| *b == bump);
        let mut has_heading = false;
        for (_, body) in group {
            if !has_heading {
                blocks.push(heading.to_owned());
                has_heading = true;
            }
            blocks.push(render_release_line(body));
        }
    }
    blocks.join("\n\n")
}

pub(crate) fn render_section(version: &Version, entry: &str) -> String {
    if entry.is_empty() {
        format!("## {version}")
    } else {
        format!("## {version}\n\n{entry}")
    }
}

fn render_release_line(body: &str) -> String {
    let mut text = String::from("- ");
    let mut lines = body.lines();
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

pub(crate) fn upsert_section(
    text: &str,
    package_name: &str,
    version: &str,
    section: &str,
) -> String {
    // pulldown-cmark does not skip a UTF-8 BOM, which would hide a leading
    // `# <package_name>` title and prepend a second one; parse without the
    // BOM and restore it afterwards to keep the copy verbatim.
    if let Some(stripped) = text.strip_prefix('\u{feff}') {
        return format!(
            "\u{feff}{}",
            upsert_section(stripped, package_name, version, section)
        );
    }

    // The collected byte ranges are invalidated whenever `text` changes, so
    // each mutation below re-parses; the happy path parses only once.
    let mut text = text.to_owned();
    let mut headings = parse_headings(&text);

    while let Some(index) = find_h2(&headings, version) {
        let end = next_h2_start(&headings, index, text.len());
        text.replace_range(headings[index].range.start..end, "");
        headings = parse_headings(&text);
    }

    let has_top_h1 = headings.first().is_some_and(|heading| {
        heading.level == HeadingLevel::H1 && text[..heading.range.start].trim().is_empty()
    });
    if !has_top_h1 {
        text = format!("# {package_name}\n\n{text}");
        headings = parse_headings(&text);
    }

    // `before` holds at least the H1, so the blank line after it never turns
    // into a leading newline; `section` has no surrounding newlines, so the
    // result ends with exactly one.
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

pub(crate) fn extract_section(text: &str, version: &str) -> Result<String> {
    // A BOM only hides a `## <version>` heading on the very first line, but
    // strip it as upsert_section does.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let headings = parse_headings(text);
    let index =
        find_h2(&headings, version).with_context(|| format!("version {version} not found"))?;
    let end = next_h2_start(&headings, index, text.len());
    Ok(trim_blank_lines(&text[headings[index].range.end..end]).to_owned())
}

struct Heading {
    level: HeadingLevel,
    // Trailing newline included (pinned by a unit test).
    range: Range<usize>,
    text: String,
}

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

fn next_h2_start(headings: &[Heading], index: usize, end_of_text: usize) -> usize {
    headings[index + 1..]
        .iter()
        .find(|heading| heading.level == HeadingLevel::H2)
        .map_or(end_of_text, |heading| heading.range.start)
}

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

    fn render(summaries: &[(Bump, &str)]) -> String {
        render_section(&"10.1.0".parse().unwrap(), &render_entry(summaries))
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
            &render_entry(&[(Bump::Minor, "Add SERPINFO satellites support")]),
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
        let section = render(&[(Bump::Minor, "Add SERPINFO satellites support")]);
        assert!(!section.starts_with('\n'));
        assert!(!section.ends_with('\n'));
    }

    #[test]
    fn indents_multi_line_bodies() {
        insta::assert_snapshot!(render(&[(
            Bump::Minor,
            "First line of body\nsecond line of body\n\nline after a blank line",
        )]));
    }

    #[test]
    fn orders_groups_and_omits_empty_ones() {
        let entries = [
            (Bump::Patch, "First patch change"),
            (Bump::Major, "Major change"),
            (Bump::Patch, "Second patch change"),
        ];
        insta::assert_snapshot!(render(&entries));
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
    fn extracts_a_section_behind_a_bom() {
        let section = extract_section("\u{feff}## 1.0.0\n\nbody\n", "1.0.0").unwrap();
        assert_eq!(section, "body");
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
