use anyhow::{Context, Result};
use jsonc_parser::cst::{CstObject, CstObjectProp, CstStringLit};

pub fn object_prop(object: &CstObject, key: &str, location: &str) -> Result<Option<CstObject>> {
    let Some(prop) = last_prop(object, key) else {
        return Ok(None);
    };
    let object = prop
        .value()
        .and_then(|value| value.as_object())
        .with_context(|| format!("{location} must be an object"))?;
    Ok(Some(object))
}

pub fn string_prop(object: &CstObject, key: &str, location: &str) -> Result<Option<CstStringLit>> {
    let Some(prop) = last_prop(object, key) else {
        return Ok(None);
    };
    let lit = prop
        .value()
        .and_then(|value| value.as_string_lit())
        .with_context(|| format!("{location} must be a string"))?;
    Ok(Some(lit))
}

// `value` must not contain characters that need escaping; semver versions
// and ranges and validated pre tags never do.
pub fn set_string_value(lit: &CstStringLit, value: &str) {
    lit.set_raw_value(format!("\"{value}\""));
}

// `CstObject::get` returns the first of duplicate keys, whereas JSON.parse
// and serde_json keep the last, which is the value the workspace reads.
fn last_prop(object: &CstObject, key: &str) -> Option<CstObjectProp> {
    object.properties().into_iter().rev().find(|prop| {
        prop.name()
            .and_then(|name| name.decoded_value().ok())
            .is_some_and(|name| name == key)
    })
}
