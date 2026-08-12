use std::collections::BTreeSet;

use super::{fingerprint::FingerprintedCatalog, PlatformErrorProjectionCodegenError};

pub(super) fn projection_rust_names(
    catalog: &FingerprintedCatalog<'_>,
) -> Result<Vec<String>, PlatformErrorProjectionCodegenError> {
    let names = catalog
        .entries
        .iter()
        .map(|entry| pascal_identifier_from_path(entry.resolved.projection_key()))
        .collect::<Vec<_>>();
    let mut unique = BTreeSet::new();
    for (entry, name) in catalog.entries.iter().zip(&names) {
        if !unique.insert(name.clone()) {
            return Err(PlatformErrorProjectionCodegenError::Render(format!(
                "canonical path {} collides at generated Rust name {name}",
                entry.resolved.projection_key()
            )));
        }
    }
    Ok(names)
}

pub(super) fn pascal_identifier(value: &str) -> String {
    let mut rendered = String::new();
    let mut capitalize = true;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if capitalize {
                rendered.extend(character.to_uppercase());
                capitalize = false;
            } else {
                rendered.push(character);
            }
        } else {
            capitalize = true;
        }
    }
    if rendered.is_empty() || rendered.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        rendered.insert_str(0, "Generated");
    }
    rendered
}

pub(super) fn snake_identifier(value: &str) -> String {
    let mut rendered = String::new();
    let mut previous_was_lower_or_digit = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if character.is_ascii_uppercase() && previous_was_lower_or_digit {
                rendered.push('_');
            }
            rendered.push(character.to_ascii_lowercase());
            previous_was_lower_or_digit =
                character.is_ascii_lowercase() || character.is_ascii_digit();
        } else {
            if !rendered.ends_with('_') && !rendered.is_empty() {
                rendered.push('_');
            }
            previous_was_lower_or_digit = false;
        }
    }
    if rendered.is_empty() || rendered.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        rendered.insert_str(0, "generated_");
    }
    if is_rust_keyword(&rendered) {
        rendered.push('_');
    }
    rendered
}

fn pascal_identifier_from_path(path: &str) -> String {
    path.split('.')
        .fold(String::new(), |mut rendered, segment| {
            rendered.push_str(&pascal_identifier(segment));
            rendered
        })
}

fn is_rust_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
    )
}
