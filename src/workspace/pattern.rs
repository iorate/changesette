use std::iter::Peekable;
use std::str::CharIndices;

use anyhow::{Result, bail};

#[derive(Debug, PartialEq)]
pub(crate) enum Seg {
    Glob(String),
    Globstar,
}

#[derive(Debug)]
pub(crate) struct Pattern {
    ascend: usize,
    segs: Vec<Seg>,
}

// The same limit as fast_glob's, which never sees the braces once they are
// expanded here.
const MAX_BRACE_NESTING: usize = 10;
// Chained braces multiply, and this expansion is the one place a pattern's
// cost stops being linear in its length.
const MAX_BRACE_EXPANSIONS: usize = 100_000;

pub(crate) fn compile(original: &str) -> Result<(bool, Vec<Pattern>)> {
    // Every leading `!` flips the polarity; leaving one in the body would
    // hand it to the glob matcher. A `!` inside a brace alternative is
    // literal, so the polarity is read before the braces are expanded.
    let mut negated = false;
    let mut body = original;
    while let Some(rest) = body.strip_prefix('!') {
        negated = !negated;
        body = rest;
    }
    // Braces are expanded before the split into segments so that an
    // alternative containing a `/` goes through the same per-segment rules
    // as any other pattern.
    let mut patterns = Vec::new();
    for alternative in expand(body)? {
        if let Some(pattern) = compile_alternative(&alternative)? {
            patterns.push(pattern);
        }
    }
    Ok((negated, patterns))
}

fn expand(body: &str) -> Result<Vec<String>> {
    struct Frame {
        outer: Vec<String>,
        alternatives: Vec<String>,
    }
    let mut stack: Vec<Frame> = Vec::new();
    let mut current = vec![String::new()];
    let mut literal_start = 0;
    let mut chars = body.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '[' => skip_class(&mut chars),
            '{' => {
                if stack.len() == MAX_BRACE_NESTING {
                    bail!(
                        "brace expansions nested deeper than {MAX_BRACE_NESTING} levels are not supported"
                    )
                }
                append(&mut current, &body[literal_start..index]);
                stack.push(Frame {
                    outer: current,
                    alternatives: Vec::new(),
                });
                current = vec![String::new()];
                literal_start = index + 1;
            }
            ',' if !stack.is_empty() => {
                append(&mut current, &body[literal_start..index]);
                stack.last_mut().unwrap().alternatives.append(&mut current);
                current = vec![String::new()];
                literal_start = index + 1;
            }
            '}' if !stack.is_empty() => {
                append(&mut current, &body[literal_start..index]);
                let mut frame = stack.pop().unwrap();
                frame.alternatives.append(&mut current);
                if frame.outer.len() * frame.alternatives.len() > MAX_BRACE_EXPANSIONS {
                    bail!(
                        "brace expansions into more than {MAX_BRACE_EXPANSIONS} patterns are not supported"
                    )
                }
                current = frame
                    .outer
                    .iter()
                    .flat_map(|prefix| {
                        frame
                            .alternatives
                            .iter()
                            .map(move |suffix| format!("{prefix}{suffix}"))
                    })
                    .collect();
                literal_start = index + 1;
            }
            _ => {}
        }
    }
    // The matcher rejects an unclosed `{` too, so it is an error rather than
    // a literal.
    if !stack.is_empty() {
        bail!("unclosed `{{`")
    }
    append(&mut current, &body[literal_start..]);
    Ok(current)
}

fn append(results: &mut [String], literal: &str) {
    for result in results {
        result.push_str(literal);
    }
}

fn compile_alternative(body: &str) -> Result<Option<Pattern>> {
    // An empty pattern matches nothing: reading it as `.` would silently opt
    // the root into versioning, and an intentional root reference has a `.`
    // segment.
    if body.is_empty() {
        return Ok(None);
    }
    // A loud error rather than a silent no-match.
    if body.starts_with('/') || body.starts_with("\\/") {
        bail!("absolute patterns are not supported")
    }
    let parts = split(body);
    let mut ascend = 0;
    let mut segs = Vec::new();
    for part in parts {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if !segs.is_empty() {
                bail!("`..` segments are only supported at the start")
            }
            ascend += 1;
            continue;
        }
        segs.push(classify(part)?);
    }
    Ok(Some(Pattern { ascend, segs }))
}

fn split(body: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut chars = body.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        match c {
            '\\' => {
                // A separator cannot be escaped in the glob dialect, so `\/`
                // splits like a bare `/` (with the `\` dropped) rather than
                // diverging from the matcher.
                if let Some(&(slash_index, '/')) = chars.peek() {
                    chars.next();
                    parts.push(&body[start..index]);
                    start = slash_index + 1;
                } else {
                    chars.next();
                }
            }
            '/' => {
                parts.push(&body[start..index]);
                start = index + 1;
            }
            '[' => skip_class(&mut chars),
            _ => {}
        }
    }
    parts.push(&body[start..]);
    parts
}

fn skip_class(chars: &mut Peekable<CharIndices<'_>>) {
    // An optional `^`/`!` prefix, then the first character is a literal
    // member (so a leading `]` does not close the class), and `\` escapes the
    // next character. An unclosed class swallows the rest of the body; the
    // per-segment validation rejects it, as does the brace expansion when
    // the class swallows a `}`. A `/` inside the class stays a member; no
    // segment name contains one, so it simply never matches.
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

fn classify(part: &str) -> Result<Seg> {
    if part == "**" {
        return Ok(Seg::Globstar);
    }
    fast_glob::validate(part)?;
    // fast_glob reads every leading `!` of the string it is handed as a
    // negation, while in the whole pattern this `!` follows a `/` or sits in
    // a brace alternative and is literal — escape it so the matcher reads it
    // that way too.
    if part.starts_with('!') {
        return Ok(Seg::Glob(format!("\\{part}")));
    }
    Ok(Seg::Glob(part.to_owned()))
}

impl Pattern {
    pub(crate) fn ascend(&self) -> usize {
        self.ascend
    }

    pub(crate) fn segs(&self) -> &[Seg] {
        &self.segs
    }

    pub(crate) fn matches(&self, rel_dir: &str, dot_permissive: bool) -> bool {
        let names: Vec<&str> = if rel_dir == "." {
            Vec::new()
        } else {
            rel_dir.split('/').collect()
        };
        let ascend = names.iter().take_while(|name| **name == "..").count();
        ascend == self.ascend && matches_from(&self.segs, &names[ascend..], dot_permissive)
    }
}

fn matches_from(segs: &[Seg], names: &[&str], dot_permissive: bool) -> bool {
    let Some((seg, segs_rest)) = segs.split_first() else {
        return names.is_empty();
    };
    if let Seg::Globstar = seg {
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

    fn positives(pattern: &str) -> Vec<Pattern> {
        let (negated, compiled) = compile(pattern).unwrap();
        assert!(!negated, "{pattern}");
        compiled
    }

    fn negations(pattern: &str) -> Vec<Pattern> {
        let (negated, compiled) = compile(pattern).unwrap();
        assert!(negated, "{pattern}");
        compiled
    }

    fn single(mut compiled: Vec<Pattern>, pattern: &str) -> Pattern {
        assert_eq!(compiled.len(), 1, "{pattern}");
        compiled.pop().unwrap()
    }

    fn positive(pattern: &str) -> Pattern {
        single(positives(pattern), pattern)
    }

    fn negation(pattern: &str) -> Pattern {
        single(negations(pattern), pattern)
    }

    fn segs_of(compiled: &[Pattern]) -> Vec<&[Seg]> {
        compiled.iter().map(Pattern::segs).collect()
    }

    fn error(pattern: &str) -> String {
        format!("{:#}", compile(pattern).unwrap_err())
    }

    #[test]
    fn normalizes_dot_and_slash_noise() {
        for pattern in ["./x", "x/", "x", "./x/"] {
            assert_eq!(positive(pattern).segs(), [seg("x")], "{pattern}");
        }
        assert_eq!(positive("x//y").segs(), [seg("x"), seg("y")]);
        for pattern in [".", "./"] {
            assert_eq!(positive(pattern).segs(), [] as [Seg; 0], "{pattern}");
        }
    }

    #[test]
    fn skips_an_empty_pattern() {
        for pattern in ["", "!", "!!", "{}", "{,}", "!{}"] {
            assert!(compile(pattern).unwrap().1.is_empty(), "{pattern}");
        }
        assert_eq!(segs_of(&positives("{,a}")), [&[seg("a")][..]]);
    }

    #[test]
    fn leading_bangs_toggle_the_polarity_by_parity() {
        assert_eq!(positive("!!x").segs(), [seg("x")]);
        assert_eq!(negation("!!!x").segs(), [seg("x")]);
    }

    #[test]
    fn rejects_an_absolute_pattern() {
        for pattern in ["/abs", "!/abs", "/"] {
            assert!(error(pattern).contains("absolute"), "{pattern}");
        }
    }

    #[test]
    fn a_drive_like_segment_is_an_ordinary_glob() {
        assert_eq!(positive("C:/x").segs(), [seg("C:"), seg("x")]);
        assert_eq!(positive("C:x").segs(), [seg("C:x")]);
        assert_eq!(positive("C:*").segs(), [seg("C:*")]);
        assert_eq!(positive("C:").segs(), [seg("C:")]);
        assert_eq!(negation("!C:/x").segs(), [seg("C:"), seg("x")]);
        assert_eq!(
            positive("packages/C:/x").segs(),
            [seg("packages"), seg("C:"), seg("x")]
        );
        assert_eq!(positive("../C:/x").segs(), [seg("C:"), seg("x")]);
        assert_eq!(
            segs_of(&positives("{C:/x,y}")),
            [&[seg("C:"), seg("x")][..], &[seg("y")]]
        );
    }

    #[test]
    fn a_leading_parent_run_ascends() {
        for pattern in ["../x", "./../x", "..//x"] {
            let compiled = positive(pattern);
            assert_eq!(compiled.ascend(), 1, "{pattern}");
            assert_eq!(compiled.segs(), [seg("x")], "{pattern}");
        }
        assert_eq!(negation("!../x").ascend(), 1);
        let dot_dot = positive("..");
        assert_eq!(dot_dot.ascend(), 1);
        assert_eq!(dot_dot.segs(), [] as [Seg; 0]);
        assert_eq!(positive("../../x").ascend(), 2);
        assert_eq!(positive("x").ascend(), 0);
        for pattern in ["a/../b", "../a/../b", "a/.."] {
            assert!(
                error(pattern).contains("only supported at the start"),
                "{pattern}"
            );
        }
    }

    #[test]
    fn a_match_requires_the_same_number_of_leading_parents() {
        assert!(negation("!../ext/o").matches("../ext/o", true));
        assert!(!negation("!**/o").matches("../ext/o", true));
        assert!(negation("!../*/o").matches("../ext/o", true));
        assert!(negation("!..").matches("..", true));
        assert!(!negation("!..").matches(".", true));
        assert!(!negation("!../x").matches("x", true));
        assert!(!negation("!../x").matches("../../x", true));
        assert!(!positive("x").matches("../x", false));
    }

    #[test]
    fn classifies_segments() {
        assert_eq!(
            positive("packages/**").segs(),
            [seg("packages"), Seg::Globstar]
        );
        assert_eq!(positive("f**").segs(), [seg("f**")]);
        assert_eq!(positive("+(a|b)").segs(), [seg("+(a|b)")]);
        assert_eq!(positive("a?c/[xy]").segs(), [seg("a?c"), seg("[xy]")]);
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
        assert_eq!(pattern.segs(), [seg("packages"), seg("\\!foo*")]);
        assert!(pattern.matches("packages/!foox", false));
        assert!(!pattern.matches("packages/foox", false));
        assert!(!pattern.matches("packages/bar", false));
        assert_eq!(
            positive("packages/!foo").segs(),
            [seg("packages"), seg("\\!foo")]
        );
        assert!(positive("packages/!foo").matches("packages/!foo", false));
        assert_eq!(positive("a/\\!b*").segs(), [seg("a"), seg("\\!b*")]);
    }

    #[test]
    fn an_escaped_slash_is_a_separator() {
        assert_eq!(positive("a\\/b").segs(), [seg("a"), seg("b")]);
        assert_eq!(positive("a\\/").segs(), [seg("a")]);
        assert!(error("\\/").contains("absolute"));
        assert!(error("\\/x").contains("absolute"));
    }

    #[test]
    fn expands_braces_before_the_split() {
        assert_eq!(
            segs_of(&positives("{a,b/c}")),
            [&[seg("a")][..], &[seg("b"), seg("c")]]
        );
        assert_eq!(
            segs_of(&positives("{a,b\\/c}")),
            [&[seg("a")][..], &[seg("b"), seg("c")]]
        );
        assert_eq!(
            segs_of(&positives("x/{a,b}/y")),
            [
                &[seg("x"), seg("a"), seg("y")][..],
                &[seg("x"), seg("b"), seg("y")]
            ]
        );
        assert_eq!(
            segs_of(&positives("a{,/x}")),
            [&[seg("a")][..], &[seg("a"), seg("x")]]
        );
        assert_eq!(
            segs_of(&positives("{a,b}{c,d}")),
            [&[seg("ac")][..], &[seg("ad")], &[seg("bc")], &[seg("bd")]]
        );
        assert_eq!(
            segs_of(&positives("{a,{b,c}}")),
            [&[seg("a")][..], &[seg("b")], &[seg("c")]]
        );
        assert_eq!(
            segs_of(&positives("{a,{b/c,d}}")),
            [&[seg("a")][..], &[seg("b"), seg("c")], &[seg("d")]]
        );
        assert_eq!(
            segs_of(&positives("{**/a,b}")),
            [&[Seg::Globstar, seg("a")][..], &[seg("b")]]
        );
        assert_eq!(segs_of(&positives("{1..3}")), [&[seg("1..3")][..]]);
    }

    #[test]
    fn brace_syntax_inside_a_class_or_after_an_escape_is_literal() {
        assert_eq!(
            segs_of(&positives("{a,[}]}")),
            [&[seg("a")][..], &[seg("[}]")]]
        );
        assert_eq!(
            segs_of(&positives("{[,],a}")),
            [&[seg("[,]")][..], &[seg("a")]]
        );
        assert_eq!(positive("\\{a,b\\}/c").segs(), [seg("\\{a,b\\}"), seg("c")]);
        assert_eq!(positive("\\{a,b}").segs(), [seg("\\{a,b}")]);
        assert_eq!(positive("{a\\,b}").segs(), [seg("a\\,b")]);
        assert_eq!(
            segs_of(&positives("{a,b\\}c}")),
            [&[seg("a")][..], &[seg("b\\}c")]]
        );
        assert_eq!(positive("[{]/a").segs(), [seg("[{]"), seg("a")]);
    }

    #[test]
    fn an_unmatched_closing_brace_or_comma_is_literal() {
        assert_eq!(positive("a}b/c").segs(), [seg("a}b"), seg("c")]);
        assert_eq!(positive("a,b/c").segs(), [seg("a,b"), seg("c")]);
        assert_eq!(
            segs_of(&positives("{a,b}}")),
            [&[seg("a}")][..], &[seg("b}")]]
        );
    }

    #[test]
    fn expands_around_multi_byte_characters() {
        assert_eq!(
            segs_of(&positives("あ{い,う/え}お")),
            [&[seg("あいお")][..], &[seg("あう"), seg("えお")]]
        );
        assert_eq!(
            segs_of(&positives("{\\あ,[い]}")),
            [&[seg("\\あ")][..], &[seg("[い]")]]
        );
    }

    #[test]
    fn a_trailing_backslash_after_a_brace_is_an_error() {
        assert!(compile("{a,b}\\").is_err());
    }

    #[test]
    fn an_unclosed_brace_is_an_error() {
        for pattern in ["{a,b", "{a,b/c", "{a,{b}", "src/{a,b", "{a,[b}"] {
            assert!(error(pattern).contains("unclosed"), "{pattern}");
        }
    }

    #[test]
    fn applies_the_polarity_to_every_alternative() {
        assert_eq!(
            segs_of(&negations("!{a,b/c}")),
            [&[seg("a")][..], &[seg("b"), seg("c")]]
        );
        assert_eq!(
            segs_of(&positives("{!a,b}")),
            [&[seg("\\!a")][..], &[seg("b")]]
        );
    }

    #[test]
    fn normalizes_each_alternative_on_its_own() {
        let compiled = positives("{../x,y}");
        assert_eq!(compiled[0].ascend(), 1);
        assert_eq!(compiled[0].segs(), [seg("x")]);
        assert_eq!(compiled[1].ascend(), 0);
        assert_eq!(compiled[1].segs(), [seg("y")]);
        assert!(error("a/{..,b}").contains("only supported at the start"));
        for pattern in ["{,a}/b", "{/a,b}", "{a,/b}"] {
            assert!(error(pattern).contains("absolute"), "{pattern}");
        }
        assert_eq!(
            segs_of(&positives("{./a,b/}")),
            [&[seg("a")][..], &[seg("b")]]
        );
    }

    #[test]
    fn limits_the_number_of_expansions() {
        assert_eq!(positives(&"{a,b}".repeat(16)).len(), 65536);
        assert!(error(&"{a,b}".repeat(17)).contains("more than 100000 patterns"));
        assert!(
            error(&format!("{{{}}}", "a,".repeat(100_000))).contains("more than 100000 patterns")
        );
    }

    #[test]
    fn limits_brace_nesting_like_the_matcher() {
        let nested = |n: usize| format!("{}k{}", "{".repeat(n), "}".repeat(n));
        assert_eq!(positive(&nested(10)).segs(), [seg("k")]);
        assert!(error(&nested(11)).contains("nested deeper than 10 levels"));
    }

    #[test]
    fn a_slash_inside_a_character_class_is_a_member() {
        let pattern = positive("[a/b]");
        assert_eq!(pattern.segs(), [seg("[a/b]")]);
        assert!(pattern.matches("a", false));
        assert!(pattern.matches("b", false));
        assert!(!pattern.matches("c", false));
        assert!(!pattern.matches("[a/b]", false));
        assert_eq!(positive("x/[a/b]").segs(), [seg("x"), seg("[a/b]")]);
        assert_eq!(positive("[a\\/b]").segs(), [seg("[a\\/b]")]);
        assert!(!positive("x[/]y").matches("xy", false));
        assert_eq!(positive("[!/]").segs(), [seg("[!/]")]);
        assert_eq!(positive("[]]x/y").segs(), [seg("[]]x"), seg("y")]);
        assert_eq!(positive("[{]/a").segs(), [seg("[{]"), seg("a")]);
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
    fn a_brace_within_a_segment_is_expanded_too() {
        assert_eq!(segs_of(&positives("{a,b}")), [&[seg("a")][..], &[seg("b")]]);
        assert_eq!(
            segs_of(&positives("x{a,b}y")),
            [&[seg("xay")][..], &[seg("xby")]]
        );
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
        assert!(pattern.matches("x", true));
        assert!(pattern.matches("x/y", true));
        assert!(!pattern.matches("y", true));
    }

    #[test]
    fn a_negation_matches_dot_segments() {
        let pattern = negation("!**/.vercel/**");
        assert!(pattern.matches(".vercel", true));
        assert!(pattern.matches("a/.vercel/b", true));
        assert!(!pattern.matches("a/b", true));
    }

    #[test]
    fn a_full_match_applies_the_dot_rule_when_not_permissive() {
        let pattern = positive("*");
        assert!(!pattern.matches(".x", false));
        assert!(pattern.matches(".x", true));
        assert!(pattern.matches("x", false));
    }

    #[test]
    fn a_full_match_uses_the_expanded_alternatives() {
        let compiled = negations("!packages/{a,b/c}");
        let matches = |rel_dir: &str| compiled.iter().any(|p| p.matches(rel_dir, true));
        assert!(matches("packages/a"));
        assert!(matches("packages/b/c"));
        assert!(!matches("packages/b"));
        assert!(!matches("packages/c"));
        assert!(!matches("packages/{a,b/c}"));
    }

    #[test]
    fn a_class_holding_only_a_slash_matches_nothing() {
        let pattern = positive("{a/.b/*/[/]}");
        assert_eq!(pattern.segs(), [seg("a"), seg(".b"), seg("*"), seg("[/]")]);
        for rel_dir in ["a/.b/x", "a/.b/x/y", "a/.b/x/[/]", "a/.b/x//"] {
            assert!(!pattern.matches(rel_dir, true), "{rel_dir}");
        }
    }
}
