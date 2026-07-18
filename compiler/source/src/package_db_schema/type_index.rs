use std::collections::BTreeMap;

use crate::{parsed_sources::ParsedCompilerSource, shared::ast::TypeRef, SourceSymbolKey};

pub(super) struct PackageDbTypeIndex<'a> {
    by_source_key: BTreeMap<SourceSymbolKey, PackageDbTypeRecord<'a>>,
}

#[derive(Clone)]
pub(super) struct PackageDbTypeRecord<'a> {
    pub(super) module_path: &'a str,
    alias: bool,
    pub(super) fields: Vec<PackageDbFieldRecord<'a>>,
}

#[derive(Clone)]
pub(super) struct PackageDbFieldRecord<'a> {
    pub(super) name: String,
    pub(super) ty: &'a TypeRef,
}

impl<'a> PackageDbTypeIndex<'a> {
    pub(super) fn build(parsed_sources: &'a [ParsedCompilerSource]) -> Self {
        let mut by_source_key = BTreeMap::new();
        for parsed in parsed_sources {
            for declaration in &parsed.ast().types {
                let source_key =
                    SourceSymbolKey::new(&parsed.source().module_path, &declaration.name);
                by_source_key.insert(
                    source_key,
                    PackageDbTypeRecord {
                        module_path: &parsed.source().module_path,
                        alias: declaration.alias.is_some(),
                        fields: declaration
                            .fields
                            .iter()
                            .map(|field| PackageDbFieldRecord {
                                name: field.name.clone(),
                                ty: &field.ty,
                            })
                            .collect(),
                    },
                );
            }
        }
        Self { by_source_key }
    }

    pub(super) fn resolve_local_record(
        &self,
        parsed: &ParsedCompilerSource,
        type_name: &str,
    ) -> Option<&PackageDbTypeRecord<'a>> {
        let record = self.resolve_from_module(&parsed.source().module_path, type_name)?;
        (!record.alias && record.module_path == parsed.source().module_path).then_some(record)
    }

    pub(super) fn resolve_from_module(
        &self,
        module_path: &str,
        type_name: &str,
    ) -> Option<&PackageDbTypeRecord<'a>> {
        let source_key = source_type_lookup_key(module_path, type_name)?;
        self.by_source_key.get(&source_key)
    }
}

fn source_type_lookup_key(module_path: &str, type_name: &str) -> Option<SourceSymbolKey> {
    if let Some((source_module, source_symbol)) = type_name.rsplit_once('.') {
        if source_module.is_empty() || source_symbol.is_empty() {
            return None;
        }
        return Some(SourceSymbolKey::new(source_module, source_symbol));
    }
    Some(SourceSymbolKey::new(module_path, type_name))
}
