pub use skiff_syntax::type_syntax::*;

pub(crate) fn generic_type_parameter_names(name: &str) -> Vec<String> {
    generic_parts(name)
        .map(|parts| {
            parts
                .args
                .iter()
                .map(|argument| argument.trim())
                .filter(|argument| {
                    !argument.is_empty()
                        && argument
                            .chars()
                            .all(|character| character == '_' || character.is_ascii_alphanumeric())
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
