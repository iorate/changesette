use std::path::{Component, Path};

use anyhow::{Result, bail};

pub(crate) fn is_plain_component(text: &str) -> bool {
    let mut components = Path::new(text).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

pub(crate) fn parse_rel_dir(text: &str) -> Result<String> {
    if text.is_empty() {
        bail!("an empty path is not supported")
    }
    if text.starts_with('/') {
        bail!("absolute paths are not supported")
    }
    if text.contains('\\') {
        bail!("`\\` is not supported; use `/` as the separator")
    }
    let mut segs = Vec::new();
    for part in text.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            bail!("`..` segments are not supported")
        }
        // Non-plain here means a disk prefix: `.` and `..` are handled
        // above, and with `/` split off and `\` rejected no separator is
        // left, so no verbatim, UNC, or device prefix can parse and the one
        // remaining non-Normal shape is a leading `C:`.
        if !is_plain_component(part) {
            bail!("drive-prefixed segments are not supported")
        }
        segs.push(part);
    }
    if segs.is_empty() {
        return Ok(".".to_owned());
    }
    Ok(segs.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(text: &str) -> String {
        format!("{:#}", parse_rel_dir(text).unwrap_err())
    }

    #[test]
    fn normalizes_a_relative_directory() {
        for text in ["a", "./a", "a/", "a//", "./a/"] {
            assert_eq!(parse_rel_dir(text).unwrap(), "a", "{text}");
        }
        for text in ["a/b", "a//b", "a/./b", "./a/b/"] {
            assert_eq!(parse_rel_dir(text).unwrap(), "a/b", "{text}");
        }
        for text in [".", "./", "./.", "././"] {
            assert_eq!(parse_rel_dir(text).unwrap(), ".", "{text}");
        }
        assert_eq!(parse_rel_dir("a/!b").unwrap(), "a/!b");
    }

    #[test]
    fn rejects_an_empty_path() {
        assert!(error("").contains("empty"));
    }

    #[test]
    fn rejects_an_absolute_path() {
        for text in ["/a", "/"] {
            assert!(error(text).contains("absolute"), "{text}");
        }
    }

    #[test]
    fn rejects_a_parent_segment() {
        for text in ["..", "../a", "a/../b", "a/.."] {
            assert!(error(text).contains("`..`"), "{text}");
        }
        assert!(!is_plain_component(".."));
    }

    #[test]
    fn rejects_a_backslash() {
        for text in ["a\\b", "\\a", "a\\", "\\"] {
            assert!(error(text).contains("`\\`"), "{text}");
        }
    }

    #[test]
    fn accepts_glob_like_names() {
        for text in ["packages/*", "a?", "[a]", "a]", "{a,b}", "a}", "!a", "**"] {
            assert_eq!(parse_rel_dir(text).unwrap(), text, "{text}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn rejects_a_drive_prefix_in_any_segment() {
        for text in ["C:/x", "C:x", "C:", "./C:/x", "packages/C:/x", "a/C:x"] {
            assert!(error(text).contains("drive"), "{text}");
        }
        assert!(!is_plain_component("C:"));
        assert!(!is_plain_component("C:x"));
        assert!(!is_plain_component("a\\b"));
        assert!(is_plain_component("a"));
    }

    #[cfg(unix)]
    #[test]
    fn a_drive_like_segment_is_an_ordinary_name_on_unix() {
        assert_eq!(parse_rel_dir("C:/x").unwrap(), "C:/x");
        assert_eq!(parse_rel_dir("./C:/x").unwrap(), "C:/x");
        assert_eq!(parse_rel_dir("C:x").unwrap(), "C:x");
        assert!(is_plain_component("C:"));
        assert!(is_plain_component("a\\b"));
    }
}
