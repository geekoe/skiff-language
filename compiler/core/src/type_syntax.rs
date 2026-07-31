pub use skiff_syntax::type_syntax::*;

/// Extracts the generic type parameter names from a type text such as
/// `Box<string>` or `Map<string, number>`.
///
/// Absorbs the former private implementations `generic_type_params_from_text`
/// and `generic_type_params` in `compiler/source`
/// (`type_resolution_model.rs`, `expression_type_model.rs`,
/// `alias_resolution.rs`, `prelude_registry/validation.rs`), which were all
/// behavior-identical to this function.
pub fn generic_type_parameter_names(name: &str) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::generic_type_parameter_names;

    #[test]
    fn extracts_simple_type_parameter_names() {
        assert_eq!(
            generic_type_parameter_names("Box<string>"),
            vec!["string".to_string()]
        );
        assert_eq!(
            generic_type_parameter_names("Map<string, number>"),
            vec!["string".to_string(), "number".to_string()]
        );
    }

    #[test]
    fn filters_non_identifier_arguments() {
        // Top-level args that are not plain identifiers are dropped as a
        // whole; digits are alphanumeric and therefore kept (matching the
        // former private copies' behavior).
        assert_eq!(
            generic_type_parameter_names("Fn<(string) -> number>"),
            Vec::<String>::new()
        );
        assert_eq!(
            generic_type_parameter_names("Wrapper<T, _U, 7, a-b>"),
            vec!["T".to_string(), "_U".to_string(), "7".to_string()]
        );
    }

    #[test]
    fn non_generic_or_malformed_text_returns_empty() {
        assert_eq!(generic_type_parameter_names("string"), Vec::<String>::new());
        assert_eq!(generic_type_parameter_names("Open<"), Vec::<String>::new());
        assert_eq!(generic_type_parameter_names(""), Vec::<String>::new());
    }
}
