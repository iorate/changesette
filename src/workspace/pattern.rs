use anyhow::{Result, bail};

#[derive(Debug, PartialEq)]
pub(crate) enum Seg {
    Literal(String),
    /// A single-segment glob, matched with `fast_glob`.
    Wildcard(String),
    Globstar,
}

/// A workspace pattern compiled to `/`-separated segments; the last segment
/// is always `Literal("package.json")`, so the pattern matches manifest file
/// paths.
#[derive(Debug)]
pub(crate) struct Pattern {
    segs: Vec<Seg>,
}

/// Compiles one workspace pattern into its polarity (`true` for a `!`
/// negation) and matcher; the errors name only the offense, and the caller
/// attaches the manifest path and the original pattern.
pub(crate) fn compile(original: &str) -> Result<(bool, Pattern)> {
    // Only the first `!` flips the polarity; any further `!` stays in the
    // pattern body.
    let (negated, body) = match original.strip_prefix('!') {
        Some(body) => (true, body),
        None => (false, original),
    };
    // An absolute path and a `..` segment are the patterns the upstream
    // tools silently break on or resolve outside the root, so they are loud
    // errors rather than a silent no-match.
    if body.starts_with('/') {
        bail!("absolute patterns are not supported")
    }
    let mut segs = Vec::new();
    for part in body.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            bail!("`..` segments are not supported")
        }
        segs.push(classify(part)?);
    }
    // Appending the manifest name gives the pnpm-style idioms for free: the
    // zero-width `**` makes `x/**` cover `x` itself and `!x/**` exclude it.
    segs.push(Seg::Literal("package.json".to_owned()));
    Ok((negated, Pattern { segs }))
}

fn classify(part: &str) -> Result<Seg> {
    if part == "**" {
        return Ok(Seg::Globstar);
    }
    if part.contains(['*', '?', '[', ']', '{', '}', '\\']) {
        fast_glob::validate(part)?;
        return Ok(Seg::Wildcard(part.to_owned()));
    }
    Ok(Seg::Literal(part.to_owned()))
}

impl Pattern {
    pub(crate) fn segs(&self) -> &[Seg] {
        &self.segs
    }

    /// Whether every segment is a `Literal`, making the pattern a plain path
    /// the walker can skip in favor of an existence check.
    pub(crate) fn is_literal(&self) -> bool {
        self.segs.iter().all(|seg| matches!(seg, Seg::Literal(_)))
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
            Seg::Literal(text) | Seg::Wildcard(text) => text.starts_with('.'),
            Seg::Globstar => false,
        };
        if !dot_ok {
            return false;
        }
    }
    match seg {
        Seg::Literal(text) => text == name,
        Seg::Wildcard(glob) => fast_glob::glob_match(glob, name),
        Seg::Globstar => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(text: &str) -> Seg {
        Seg::Literal(text.to_owned())
    }

    fn wild(text: &str) -> Seg {
        Seg::Wildcard(text.to_owned())
    }

    fn positive(pattern: &str) -> Pattern {
        let (negated, compiled) = compile(pattern).unwrap();
        assert!(!negated, "{pattern}");
        compiled
    }

    fn negation(pattern: &str) -> Pattern {
        let (negated, compiled) = compile(pattern).unwrap();
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
                [lit("x"), lit("package.json")],
                "{pattern}"
            );
        }
        assert_eq!(
            positive("x//y").segs(),
            [lit("x"), lit("y"), lit("package.json")]
        );
        for pattern in [".", "./", ""] {
            assert_eq!(positive(pattern).segs(), [lit("package.json")], "{pattern}");
        }
    }

    #[test]
    fn rejects_an_absolute_pattern() {
        for pattern in ["/abs", "!/abs", "/"] {
            assert!(error(pattern).contains("absolute"), "{pattern}");
        }
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
            [lit("packages"), Seg::Globstar, lit("package.json")]
        );
        assert_eq!(positive("f**").segs(), [wild("f**"), lit("package.json")]);
        assert_eq!(
            positive("+(a|b)").segs(),
            [lit("+(a|b)"), lit("package.json")]
        );
        assert_eq!(
            positive("a?c/[xy]").segs(),
            [wild("a?c"), wild("[xy]"), lit("package.json")]
        );
    }

    #[test]
    fn rejects_invalid_glob_syntax() {
        assert!(compile("packages/[").is_err());
        assert!(compile("src/{a,b").is_err());
    }

    #[test]
    fn applies_the_dot_rule_per_segment() {
        assert!(!seg_matches(&wild("*"), ".x", false));
        assert!(seg_matches(&wild(".*"), ".x", false));
        assert!(seg_matches(&lit(".github"), ".github", false));
        assert!(!seg_matches(&Seg::Globstar, ".x", false));
        assert!(seg_matches(&wild("*"), ".x", true));
        assert!(seg_matches(&Seg::Globstar, ".x", true));
    }

    #[test]
    fn expands_braces_within_a_segment() {
        assert!(seg_matches(&wild("{a,b}"), "a", false));
        assert!(seg_matches(&wild("{a,b}"), "b", false));
        assert!(!seg_matches(&wild("{a,b}"), "{a,b}", false));
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
}
