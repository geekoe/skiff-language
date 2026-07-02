use std::collections::BTreeMap;

use crate::parsed_sources::ParsedCompilerSource;

use super::SourceSymbolKey;

#[derive(Debug, Clone, Default)]
pub struct PublicationTypeSymbolIndex {
    by_bare_name: BTreeMap<String, SourceSymbolKey>,
    by_source_key: BTreeMap<SourceSymbolKey, SourceSymbolKey>,
}

impl PublicationTypeSymbolIndex {
    pub fn is_empty(&self) -> bool {
        self.by_bare_name.is_empty() && self.by_source_key.is_empty()
    }

    pub fn resolve_source_text(&self, name: &str) -> Option<&SourceSymbolKey> {
        let name = name.trim().strip_prefix("root.").unwrap_or(name.trim());
        if let Some((module_path, symbol)) = name.rsplit_once('.') {
            return self.resolve_source_key(&SourceSymbolKey::new(module_path, symbol));
        }
        self.resolve_bare(name)
    }

    pub fn insert_resolved_symbol(&mut self, visible_name: &str, symbol: SourceSymbolKey) {
        let visible_name = visible_name.trim();
        if let Some((module_path, source_symbol)) = visible_name.rsplit_once('.') {
            self.by_source_key
                .entry(SourceSymbolKey::new(module_path, source_symbol))
                .or_insert_with(|| symbol.clone());
        } else if !visible_name.is_empty() {
            self.by_bare_name
                .entry(visible_name.to_string())
                .or_insert_with(|| symbol.clone());
        }
        self.by_source_key.entry(symbol.clone()).or_insert(symbol);
    }

    fn insert(&mut self, module_path: &str, name: &str) {
        let symbol = SourceSymbolKey::new(module_path, name);
        self.by_bare_name
            .entry(name.to_string())
            .or_insert_with(|| symbol.clone());
        self.by_source_key
            .entry(SourceSymbolKey::new(module_path, name))
            .or_insert(symbol);
    }

    fn resolve_bare(&self, name: &str) -> Option<&SourceSymbolKey> {
        self.by_bare_name.get(name)
    }

    fn resolve_source_key(&self, key: &SourceSymbolKey) -> Option<&SourceSymbolKey> {
        self.by_source_key.get(key)
    }
}

pub fn publication_type_symbols(
    parsed_sources: &[ParsedCompilerSource],
) -> PublicationTypeSymbolIndex {
    let mut symbols = PublicationTypeSymbolIndex::default();
    for parsed in parsed_sources {
        for name in parsed
            .ast()
            .types
            .iter()
            .map(|decl| decl.name.as_str())
            .chain(parsed.ast().aliases.iter().map(|decl| decl.name.as_str()))
            .chain(
                parsed
                    .ast()
                    .interfaces
                    .iter()
                    .map(|decl| decl.name.as_str()),
            )
        {
            symbols.insert(&parsed.source().module_path, name);
        }
    }
    symbols
}

#[cfg(test)]
mod tests {
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
}
