use std::iter::Peekable;
use std::path::{Component, Path};
use std::str::CharIndices;

use anyhow::{Result, bail};

#[derive(Debug, PartialEq)]
pub(crate) enum Seg {
    /// A single-segment glob, matched with `fast_glob`; a literal name is
    /// one without wildcards.
    Glob(String),
    Globstar,
}

/// A workspace pattern compiled to `/`-separated segments; the last segment
/// is always `Glob("package.json")`, so the pattern matches manifest file
/// paths.
#[derive(Debug)]
pub(crate) struct Pattern {
    segs: Vec<Seg>,
}

/// Compiles one workspace pattern into its polarity (`true` for a `!`
/// negation) and matcher, or `None` for an empty pattern; the errors name
/// only the offense, and the caller attaches the manifest path and the
/// original pattern. The body is read as one fast-glob glob and split at its
/// top-level separators.
pub(crate) fn compile(original: &str) -> Result<Option<(bool, Pattern)>> {
    // Every leading `!` flips the polarity, as npm counts them; leaving one
    // in the body would hand it to the glob matcher.
    let mut negated = false;
    let mut body = original;
    while let Some(rest) = body.strip_prefix('!') {
        negated = !negated;
        body = rest;
    }
    // An empty pattern matches nothing, as in npm (Yarn and pnpm error on
    // it); reading it as `.` would silently opt the root into versioning,
    // and an intentional root reference has a `.` segment.
    if body.is_empty() {
        return Ok(None);
    }
    // An absolute path and a `..` segment are the patterns the upstream
    // tools silently break on or resolve outside the root, so they are loud
    // errors rather than a silent no-match.
    if body.starts_with('/') || body.starts_with("\\/") {
        bail!("absolute patterns are not supported")
    }
    let parts = split(body)?;
    let mut segs = Vec::new();
    for (index, part) in parts.into_iter().enumerate() {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            bail!("`..` segments are not supported")
        }
        // A leading Windows drive prefix (`C:/x`, or the drive-relative
        // `C:x`) addresses a location outside the root like an absolute
        // path, so it is the same loud error; on Unix nothing parses as a
        // prefix and `C:` stays an ordinary name.
        if index == 0 && has_drive_prefix(part) {
            bail!("drive-prefixed patterns are not supported")
        }
        segs.push(classify(part)?);
    }
    // Appending the manifest name gives the pnpm-style idioms for free: the
    // zero-width `**` makes `x/**` cover `x` itself and `!x/**` exclude it.
    segs.push(Seg::Glob("package.json".to_owned()));
    Ok(Some((negated, Pattern { segs })))
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
    if text.starts_with('!') || text.contains(['*', '?', '[', ']', '{', '}']) {
        bail!("wildcards are not supported; list each directory")
    }
    let mut segs = Vec::new();
    for (index, part) in text.split('/').enumerate() {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            bail!("`..` segments are not supported")
        }
        if index == 0 && has_drive_prefix(part) {
            bail!("drive-prefixed paths are not supported")
        }
        segs.push(part);
    }
    if segs.is_empty() {
        return Ok(".".to_owned());
    }
    Ok(segs.join("/"))
}

fn split(body: &str) -> Result<Vec<&str>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut brace_depth: usize = 0;
    let mut chars = body.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        match c {
            '\\' => {
                // A separator cannot be escaped: fast-glob unescapes `\/`
                // right back into one, so `\/` splits like a bare `/` (with
                // the `\` dropped) rather than diverging from the matcher.
                if let Some(&(slash_index, '/')) = chars.peek() {
                    if brace_depth > 0 {
                        bail!("`/` inside braces is not supported")
                    }
                    chars.next();
                    parts.push(&body[start..index]);
                    start = slash_index + 1;
                } else {
                    // An escaped character, or a trailing `\` that the
                    // per-segment validation rejects.
                    chars.next();
                }
            }
            '/' => {
                // Braces spanning segments are deliberately unsupported
                // (zero occurrences in the wild): an intended error now,
                // where the shattered halves used to fail the glob
                // validation by accident.
                if brace_depth > 0 {
                    bail!("`/` inside braces is not supported")
                }
                parts.push(&body[start..index]);
                start = index + 1;
            }
            '{' => brace_depth += 1,
            // A `}` without a matching `{` is an ordinary character.
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '[' => skip_class(&mut chars),
            _ => {}
        }
    }
    parts.push(&body[start..]);
    Ok(parts)
}

fn skip_class(chars: &mut Peekable<CharIndices<'_>>) {
    // Mirrors fast-glob's class parsing: an optional `^`/`!` prefix, then
    // the first character is a literal member (so a leading `]` does not
    // close the class), and `\` escapes the next character. An unclosed
    // class swallows the rest of the body and is left for the per-segment
    // validation to reject. A `/` inside the class stays a member, as Yarn
    // reads it; no segment name contains one, so it simply never matches.
    if matches!(chars.peek(), Some((_, '^' | '!'))) {
        chars.next();
    }
    let mut first = true;
    while let Some((_, c)) = chars.next() {
        match c {
            ']' if !first => return,
            '\\' => {
                chars.next();
            }
            _ => {}
        }
        first = false;
    }
}

// Only the first component is looked at: a `\` in the part is a separator
// to the std parser on Windows but a glob escape here.
fn has_drive_prefix(part: &str) -> bool {
    matches!(
        Path::new(part).components().next(),
        Some(Component::Prefix(_))
    )
}

fn classify(part: &str) -> Result<Seg> {
    if part == "**" {
        return Ok(Seg::Globstar);
    }
    fast_glob::validate(part)?;
    // fast_glob reads every leading `!` of the string it is handed as a
    // negation, while in the whole pattern this `!` sits mid-glob and is
    // literal — escape it so the matcher reads it that way too.
    if part.starts_with('!') {
        return Ok(Seg::Glob(format!("\\{part}")));
    }
    Ok(Seg::Glob(part.to_owned()))
}

impl Pattern {
    pub(crate) fn segs(&self) -> &[Seg] {
        &self.segs
    }

    /// Matches a root-relative `/`-separated manifest path in full; a
    /// negation passes `dot_permissive` so its wildcards cover dot segments.
    pub(crate) fn matches(&self, rel_manifest: &str, dot_permissive: bool) -> bool {
        let names: Vec<&str> = rel_manifest.split('/').collect();
        matches_from(&self.segs, &names, dot_permissive)
    }
}

fn matches_from(segs: &[Seg], names: &[&str], dot_permissive: bool) -> bool {
    let Some((seg, segs_rest)) = segs.split_first() else {
        return names.is_empty();
    };
    if let Seg::Globstar = seg {
        // A globstar consumes zero or more segments.
        if matches_from(segs_rest, names, dot_permissive) {
            return true;
        }
        return match names.split_first() {
            Some((name, names_rest)) if seg_matches(seg, name, dot_permissive) => {
                matches_from(segs, names_rest, dot_permissive)
            }
            _ => false,
        };
    }
    match names.split_first() {
        Some((name, names_rest)) => {
            seg_matches(seg, name, dot_permissive)
                && matches_from(segs_rest, names_rest, dot_permissive)
        }
        None => false,
    }
}

/// Matches one pattern segment against one path segment, applying the dot
/// rule unless `dot_permissive`: a `.`-leading name matches only a pattern
/// segment that literally starts with `.`.
pub(crate) fn seg_matches(seg: &Seg, name: &str, dot_permissive: bool) -> bool {
    if !dot_permissive && name.starts_with('.') {
        let dot_ok = match seg {
            Seg::Glob(text) => text.starts_with('.'),
            Seg::Globstar => false,
        };
        if !dot_ok {
            return false;
        }
    }
    match seg {
        Seg::Glob(glob) => fast_glob::glob_match(glob, name),
        Seg::Globstar => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str) -> Seg {
        Seg::Glob(text.to_owned())
    }

    fn positive(pattern: &str) -> Pattern {
        let (negated, compiled) = compile(pattern).unwrap().unwrap();
        assert!(!negated, "{pattern}");
        compiled
    }

    fn negation(pattern: &str) -> Pattern {
        let (negated, compiled) = compile(pattern).unwrap().unwrap();
        assert!(negated, "{pattern}");
        compiled
    }

    fn error(pattern: &str) -> String {
        format!("{:#}", compile(pattern).unwrap_err())
    }

    #[test]
    fn normalizes_dot_and_slash_noise() {
        for pattern in ["./x", "x/", "x", "./x/"] {
            assert_eq!(
                positive(pattern).segs(),
                [seg("x"), seg("package.json")],
                "{pattern}"
            );
        }
        assert_eq!(
            positive("x//y").segs(),
            [seg("x"), seg("y"), seg("package.json")]
        );
        for pattern in [".", "./"] {
            assert_eq!(positive(pattern).segs(), [seg("package.json")], "{pattern}");
        }
    }

    #[test]
    fn skips_an_empty_pattern() {
        for pattern in ["", "!", "!!"] {
            assert!(compile(pattern).unwrap().is_none(), "{pattern}");
        }
    }

    #[test]
    fn leading_bangs_toggle_the_polarity_by_parity() {
        assert_eq!(positive("!!x").segs(), [seg("x"), seg("package.json")]);
        assert_eq!(negation("!!!x").segs(), [seg("x"), seg("package.json")]);
    }

    #[test]
    fn rejects_an_absolute_pattern() {
        for pattern in ["/abs", "!/abs", "/"] {
            assert!(error(pattern).contains("absolute"), "{pattern}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn rejects_a_leading_drive_prefix() {
        for pattern in ["C:/packages/*", "C:x", "c:/x", "C:", "!C:/x"] {
            assert!(error(pattern).contains("drive"), "{pattern}");
        }
        assert!(has_drive_prefix("C:"));
        assert!(has_drive_prefix("C:x"));
        assert!(has_drive_prefix("C:*"));
        assert!(!has_drive_prefix("x"));
        assert!(!has_drive_prefix("a\\b"));
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_prefix_after_the_first_raw_segment_compiles() {
        assert_eq!(
            positive("packages/C:/x").segs(),
            [seg("packages"), seg("C:"), seg("x"), seg("package.json")]
        );
        assert_eq!(
            positive("./C:/x").segs(),
            [seg("C:"), seg("x"), seg("package.json")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_drive_like_segment_is_an_ordinary_name_on_unix() {
        assert_eq!(
            positive("C:/x").segs(),
            [seg("C:"), seg("x"), seg("package.json")]
        );
        assert_eq!(positive("C:x").segs(), [seg("C:x"), seg("package.json")]);
        assert!(!has_drive_prefix("C:"));
    }

    #[test]
    fn rejects_a_parent_segment() {
        for pattern in ["../x", "!../x", "a/../b", ".."] {
            assert!(error(pattern).contains("`..`"), "{pattern}");
        }
    }

    #[test]
    fn classifies_segments() {
        assert_eq!(
            positive("packages/**").segs(),
            [seg("packages"), Seg::Globstar, seg("package.json")]
        );
        assert_eq!(positive("f**").segs(), [seg("f**"), seg("package.json")]);
        assert_eq!(
            positive("+(a|b)").segs(),
            [seg("+(a|b)"), seg("package.json")]
        );
        assert_eq!(
            positive("a?c/[xy]").segs(),
            [seg("a?c"), seg("[xy]"), seg("package.json")]
        );
    }

    #[test]
    fn rejects_invalid_glob_syntax() {
        assert!(compile("packages/[").is_err());
        assert!(compile("src/{a,b").is_err());
        assert!(compile("x\\").is_err());
    }

    #[test]
    fn a_bang_after_the_leading_run_is_literal() {
        let pattern = positive("packages/!foo*");
        assert_eq!(
            pattern.segs(),
            [seg("packages"), seg("\\!foo*"), seg("package.json")]
        );
        assert!(pattern.matches("packages/!foox/package.json", false));
        assert!(!pattern.matches("packages/foox/package.json", false));
        assert!(!pattern.matches("packages/bar/package.json", false));
        assert_eq!(
            positive("packages/!foo").segs(),
            [seg("packages"), seg("\\!foo"), seg("package.json")]
        );
        assert!(positive("packages/!foo").matches("packages/!foo/package.json", false));
        assert_eq!(
            positive("a/\\!b*").segs(),
            [seg("a"), seg("\\!b*"), seg("package.json")]
        );
    }

    #[test]
    fn an_escaped_slash_is_a_separator() {
        assert_eq!(
            positive("a\\/b").segs(),
            [seg("a"), seg("b"), seg("package.json")]
        );
        assert_eq!(positive("a\\/").segs(), [seg("a"), seg("package.json")]);
        assert!(error("\\/").contains("absolute"));
        assert!(error("\\/x").contains("absolute"));
    }

    #[test]
    fn rejects_a_slash_inside_braces() {
        for pattern in ["{a,b/c}", "{a,b\\/c}", "x/{a,b/c}", "{a,{b/c,d}}"] {
            assert!(error(pattern).contains("braces"), "{pattern}");
        }
        assert_eq!(
            positive("x/{a,b}/y").segs(),
            [seg("x"), seg("{a,b}"), seg("y"), seg("package.json")]
        );
        assert_eq!(
            positive("\\{a,b\\}/c").segs(),
            [seg("\\{a,b\\}"), seg("c"), seg("package.json")]
        );
        assert_eq!(
            positive("a}b/c").segs(),
            [seg("a}b"), seg("c"), seg("package.json")]
        );
    }

    #[test]
    fn a_slash_inside_a_character_class_is_a_member() {
        let pattern = positive("[a/b]");
        assert_eq!(pattern.segs(), [seg("[a/b]"), seg("package.json")]);
        assert!(pattern.matches("a/package.json", false));
        assert!(pattern.matches("b/package.json", false));
        assert!(!pattern.matches("c/package.json", false));
        assert!(!pattern.matches("[a/b]/package.json", false));
        assert_eq!(
            positive("x/[a/b]").segs(),
            [seg("x"), seg("[a/b]"), seg("package.json")]
        );
        assert_eq!(
            positive("[a\\/b]").segs(),
            [seg("[a\\/b]"), seg("package.json")]
        );
        assert!(!positive("x[/]y").matches("xy/package.json", false));
        assert_eq!(positive("[!/]").segs(), [seg("[!/]"), seg("package.json")]);
        assert_eq!(
            positive("[]]x/y").segs(),
            [seg("[]]x"), seg("y"), seg("package.json")]
        );
        assert_eq!(
            positive("[{]/a").segs(),
            [seg("[{]"), seg("a"), seg("package.json")]
        );
    }

    #[test]
    fn applies_the_dot_rule_per_segment() {
        assert!(!seg_matches(&seg("*"), ".x", false));
        assert!(seg_matches(&seg(".*"), ".x", false));
        assert!(seg_matches(&seg(".github"), ".github", false));
        assert!(!seg_matches(&Seg::Globstar, ".x", false));
        assert!(seg_matches(&seg("*"), ".x", true));
        assert!(seg_matches(&Seg::Globstar, ".x", true));
    }

    #[test]
    fn expands_braces_within_a_segment() {
        assert!(seg_matches(&seg("{a,b}"), "a", false));
        assert!(seg_matches(&seg("{a,b}"), "b", false));
        assert!(!seg_matches(&seg("{a,b}"), "{a,b}", false));
    }

    #[test]
    fn expands_nested_braces() {
        for name in ["a", "b", "c"] {
            assert!(seg_matches(&seg("{a,{b,c}}"), name, false), "{name}");
        }
        assert!(!seg_matches(&seg("{a,{b,c}}"), "d", false));
        assert!(!seg_matches(&seg("{a,{b,c}}"), "{b,c}", false));
    }

    #[test]
    fn a_negated_character_class_excludes_its_members() {
        assert!(seg_matches(&seg("[!b]"), "a", false));
        assert!(!seg_matches(&seg("[!b]"), "b", false));
        assert!(seg_matches(&seg("x[!b]"), "xc", false));
    }

    #[test]
    fn a_double_star_negation_matches_the_base_directory() {
        let pattern = negation("!x/**");
        assert!(pattern.matches("x/package.json", true));
        assert!(pattern.matches("x/y/package.json", true));
        assert!(!pattern.matches("y/package.json", true));
    }

    #[test]
    fn a_negation_matches_dot_segments() {
        let pattern = negation("!**/.vercel/**");
        assert!(pattern.matches(".vercel/package.json", true));
        assert!(pattern.matches("a/.vercel/b/package.json", true));
        assert!(!pattern.matches("a/b/package.json", true));
    }

    #[test]
    fn a_full_match_applies_the_dot_rule_when_not_permissive() {
        let pattern = positive("*");
        assert!(!pattern.matches(".x/package.json", false));
        assert!(pattern.matches(".x/package.json", true));
        assert!(pattern.matches("x/package.json", false));
    }

    #[test]
    fn a_full_match_expands_braces() {
        let pattern = positive("packages/{a,b}");
        assert!(pattern.matches("packages/a/package.json", true));
        assert!(!pattern.matches("packages/c/package.json", true));
        assert!(!pattern.matches("packages/{a,b}/package.json", true));
    }

    fn rel_dir_error(text: &str) -> String {
        format!("{:#}", parse_rel_dir(text).unwrap_err())
    }

    #[test]
    fn normalizes_a_relative_directory_entry() {
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
    fn rejects_an_empty_directory_entry() {
        assert!(rel_dir_error("").contains("empty"));
    }

    #[test]
    fn rejects_an_absolute_directory_entry() {
        for text in ["/a", "/"] {
            assert!(rel_dir_error(text).contains("absolute"), "{text}");
        }
    }

    #[test]
    fn rejects_a_parent_segment_in_a_directory_entry() {
        for text in ["..", "../a", "a/../b", "a/.."] {
            assert!(rel_dir_error(text).contains("`..`"), "{text}");
        }
    }

    #[test]
    fn rejects_a_backslash_in_a_directory_entry() {
        for text in ["a\\b", "\\a", "a\\", "\\"] {
            assert!(rel_dir_error(text).contains("`\\`"), "{text}");
        }
    }

    #[test]
    fn rejects_wildcards_in_a_directory_entry() {
        for text in ["packages/*", "a?", "[a]", "a]", "{a,b}", "a}", "!a", "**"] {
            assert!(rel_dir_error(text).contains("wildcards"), "{text}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn rejects_a_leading_drive_prefix_in_a_directory_entry() {
        for text in ["C:/x", "C:x", "C:"] {
            assert!(rel_dir_error(text).contains("drive"), "{text}");
        }
        assert_eq!(parse_rel_dir("./C:/x").unwrap(), "C:/x");
    }

    #[cfg(unix)]
    #[test]
    fn a_drive_like_directory_entry_is_an_ordinary_name_on_unix() {
        assert_eq!(parse_rel_dir("C:/x").unwrap(), "C:/x");
        assert_eq!(parse_rel_dir("C:x").unwrap(), "C:x");
    }
}
