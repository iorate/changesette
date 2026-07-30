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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn has_no_leading_or_trailing_newline() {
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
}
