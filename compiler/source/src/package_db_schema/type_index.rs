use std::collections::{BTreeMap, BTreeSet};

use crate::{
    parsed_sources::ParsedCompilerSource,
    shared::{ast::TypeRef, type_expr::TypeExpr},
    SourceSymbolKey,
};

pub(super) struct PackageDbTypeIndex<'a> {
    by_source_key: BTreeMap<SourceSymbolKey, PackageDbTypeRecord<'a>>,
}

#[derive(Clone)]
pub(super) struct PackageDbTypeRecord<'a> {
    pub(super) module_path: &'a str,
    alias_targets: &'a BTreeMap<String, String>,
    representation: Option<&'a TypeRef>,
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
                        alias_targets: parsed.alias_targets(),
                        representation: declaration.alias.as_ref(),
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
        (record.representation.is_none() && record.module_path == parsed.source().module_path)
            .then_some(record)
    }

    pub(super) fn resolve_from_module(
        &self,
        module_path: &str,
        type_name: &str,
    ) -> Option<&PackageDbTypeRecord<'a>> {
        let source_key = source_type_lookup_key(module_path, type_name)?;
        self.by_source_key.get(&source_key)
    }

    pub(super) fn resolve_record_type(
        &self,
        owner: &PackageDbTypeRecord<'a>,
        ty: &TypeRef,
    ) -> Option<&PackageDbTypeRecord<'a>> {
        let expanded = expand_transparent_aliases(&ty.name, owner.alias_targets)?;
        let named = unwrap_nullable_named_type(&TypeExpr::parse(&expanded))?;
        let record = self.resolve_from_module(owner.module_path, named)?;
        record.representation.is_none().then_some(record)
    }

    pub(super) fn is_indexable_scalar(
        &self,
        owner: &PackageDbTypeRecord<'a>,
        ty: &TypeRef,
        nullable: bool,
    ) -> bool {
        self.is_indexable_scalar_expr(
            owner,
            &TypeExpr::parse(&ty.name),
            nullable,
            &mut BTreeSet::new(),
        )
    }

    fn is_indexable_scalar_expr(
        &self,
        owner: &PackageDbTypeRecord<'a>,
        ty: &TypeExpr,
        nullable: bool,
        seen: &mut BTreeSet<(String, String)>,
    ) -> bool {
        match ty {
            TypeExpr::Nullable(inner) if nullable => {
                self.is_indexable_scalar_expr(owner, inner, false, seen)
            }
            TypeExpr::Union(items) if nullable => {
                let mut scalar = None;
                for item in items {
                    if matches!(item, TypeExpr::Named { name, args } if name == "null" && args.is_empty())
                    {
                        continue;
                    }
                    if scalar.replace(item).is_some() {
                        return false;
                    }
                }
                scalar.is_some_and(|item| {
                    items.len() == 2 && self.is_indexable_scalar_expr(owner, item, false, seen)
                })
            }
            TypeExpr::Named { name, args } if args.is_empty() => {
                if is_explicit_indexable_scalar(name) {
                    return true;
                }
                if let Some(target) = owner.alias_targets.get(name) {
                    let key = (owner.module_path.to_string(), name.clone());
                    if !seen.insert(key.clone()) {
                        return false;
                    }
                    let result = self.is_indexable_scalar_expr(
                        owner,
                        &TypeExpr::parse(target),
                        nullable,
                        seen,
                    );
                    seen.remove(&key);
                    return result;
                }
                let Some(record) = self.resolve_from_module(owner.module_path, name) else {
                    return false;
                };
                let Some(representation) = record.representation else {
                    return false;
                };
                let key = (record.module_path.to_string(), name.clone());
                if !seen.insert(key.clone()) {
                    return false;
                }
                let result = self.is_indexable_scalar_expr(
                    record,
                    &TypeExpr::parse(&representation.name),
                    nullable,
                    seen,
                );
                seen.remove(&key);
                result
            }
            _ => false,
        }
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

fn expand_transparent_aliases(raw: &str, aliases: &BTreeMap<String, String>) -> Option<String> {
    let mut current = raw.trim().to_string();
    let mut seen = BTreeSet::new();
    loop {
        let expression = TypeExpr::parse(&current);
        let TypeExpr::Named { name, args } = expression else {
            return Some(current);
        };
        if !args.is_empty() {
            return Some(current);
        }
        let Some(target) = aliases.get(&name) else {
            return Some(current);
        };
        if !seen.insert(name) {
            return None;
        }
        current = target.clone();
    }
}

fn unwrap_nullable_named_type(ty: &TypeExpr) -> Option<&str> {
    match ty {
        TypeExpr::Named { name, args } if args.is_empty() => Some(name),
        TypeExpr::Nullable(inner) => unwrap_nullable_named_type(inner),
        TypeExpr::Union(items)
            if items.len() == 2
                && items.iter().any(
                    |item| matches!(item, TypeExpr::Named { name, args } if name == "null" && args.is_empty()),
                ) =>
        {
            items.iter().find_map(|item| match item {
                TypeExpr::Named { name, args } if name == "null" && args.is_empty() => None,
                other => unwrap_nullable_named_type(other),
            })
        }
        _ => None,
    }
}

fn is_explicit_indexable_scalar(name: &str) -> bool {
    let canonical = if name == "boolean" { "bool" } else { name };
    skiff_artifact_model::is_db_indexable_scalar_builtin(canonical)
}
