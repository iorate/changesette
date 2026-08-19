use anyhow::{Context, Result};
use jsonc_parser::cst::{CstObject, CstStringLit};

/// Returns the string literal at `object[key]`, or `Ok(None)` if the key is
/// absent; a non-string value is an error naming `location`.
pub(crate) fn string_prop(
    object: &CstObject,
    key: &str,
    location: &str,
) -> Result<Option<CstStringLit>> {
    let Some(prop) = object.get(key) else {
        return Ok(None);
    };
    let lit = prop
        .value()
        .and_then(|value| value.as_string_lit())
        .with_context(|| format!("{location} must be a string"))?;
    Ok(Some(lit))
}

/// Replaces the literal's raw text with `"<value>"`; `value` must not contain
/// characters that need escaping (semver versions and validated pre tags never
/// do).
pub(crate) fn set_string_value(lit: &CstStringLit, value: &str) {
    lit.set_raw_value(format!("\"{value}\""));
}
