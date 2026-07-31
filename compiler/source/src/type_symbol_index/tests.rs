use super::*;

#[test]
fn publication_type_symbol_index_resolves_bare_and_qualified_source_names() {
    let mut index = PublicationTypeSymbolIndex::default();
    index.insert("internal.models", "User");

    assert_eq!(
        index.resolve_source_text("User"),
        Some(&SourceSymbolKey::new("internal.models", "User"))
    );
    assert_eq!(
        index.resolve_source_text("internal.models.User"),
        Some(&SourceSymbolKey::new("internal.models", "User"))
    );
    assert_eq!(
        index.resolve_source_text("root.internal.models.User"),
        Some(&SourceSymbolKey::new("internal.models", "User"))
    );
    assert_eq!(index.resolve_source_text("models.User"), None);
}
