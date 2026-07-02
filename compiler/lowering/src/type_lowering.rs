use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};

use crate::file_ir::{
    FunctionTypeParamIr, LiteralIr, PackageRefIr, PackageSymbolRef, ServiceSymbolRef, TypeRefIr,
};
use skiff_artifact_model::interface_instantiation_ref;
use skiff_compiler_core::{
    id::SKIFF_STD_PUBLICATION_ID, package_export_resolver::PackageExportResolver,
};
use skiff_compiler_source::{
    prelude_registry::prelude_registry, LocalDbObjectIndex, PublicationDbMetadata,
    PublicationDbMetadataIndex, PublicationTypeSymbolIndex, SourceSymbolKey,
};
use skiff_syntax::{
    ast::TypeRef,
    error::{CompileError, Result},
    type_expr::TypeExpr,
    type_syntax::{
        generic_parts, parse_record_type_fields, split_top_level, string_literal,
        RecordTypeFieldParseError,
    },
};

static EMPTY_TYPE_PARAM_SCOPE: LazyLock<BTreeSet<String>> = LazyLock::new(BTreeSet::new);

pub(super) fn package_scoped_root_path(module_path: &str, service_path: &str) -> String {
    if is_official_std_module_path(module_path)
        && service_path != "std"
        && !service_path.starts_with("std.")
    {
        format!("std.{service_path}")
    } else {
        service_path.to_string()
    }
}

pub(super) fn is_official_std_module_path(module_path: &str) -> bool {
    module_path == "std" || module_path.starts_with("std.")
}

pub(super) fn type_root(ty: &str) -> &str {
    let ty = ty.trim().trim_end_matches('?').trim();
    generic_parts(ty)
        .map(|parts| parts.root)
        .unwrap_or_else(|| ty.split('<').next().unwrap_or(ty).trim())
}

pub(super) fn bare_type_name(root: &str) -> &str {
    root.rsplit('.').next().unwrap_or(root)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TypeLoweringMode {
    Value,
    DbTarget,
}

#[derive(Debug, Clone, Copy)]
pub struct TypeLoweringContext<'a> {
    mode: TypeLoweringMode,
    type_params: &'a BTreeSet<String>,
}

impl TypeLoweringContext<'static> {
    pub fn value() -> Self {
        Self {
            mode: TypeLoweringMode::Value,
            type_params: &EMPTY_TYPE_PARAM_SCOPE,
        }
    }
}

impl<'a> TypeLoweringContext<'a> {
    pub fn value_with_type_params(type_params: &'a BTreeSet<String>) -> Self {
        Self {
            mode: TypeLoweringMode::Value,
            type_params,
        }
    }

    pub(super) fn db_target_with_type_params(type_params: &'a BTreeSet<String>) -> Self {
        Self {
            mode: TypeLoweringMode::DbTarget,
            type_params,
        }
    }

    fn is_type_param(self, name: &str) -> bool {
        self.type_params.contains(name)
    }
}

pub(super) fn service_symbol_ref(current_module_path: &str, path: &str) -> ServiceSymbolRef {
    let path = path.trim();
    if let Some((module_path, symbol)) = path.rsplit_once('.') {
        ServiceSymbolRef {
            module_path: if module_path.is_empty() {
                current_module_path.to_string()
            } else {
                module_path.to_string()
            },
            symbol: symbol.to_string(),
        }
    } else {
        ServiceSymbolRef {
            module_path: current_module_path.to_string(),
            symbol: path.to_string(),
        }
    }
}

pub(super) fn service_symbol_ref_from_source_key(source_key: &SourceSymbolKey) -> ServiceSymbolRef {
    ServiceSymbolRef {
        module_path: source_key.module_path().to_string(),
        symbol: source_key.symbol().to_string(),
    }
}

fn package_type_symbol_ref(
    name: &str,
    package_aliases: &BTreeMap<String, Vec<String>>,
) -> Option<PackageSymbolRef> {
    let symbol = PackageExportResolver::new(package_aliases).resolve_package_symbol_path(name)?;
    Some(PackageSymbolRef {
        package: PackageRefIr::Dependency {
            dependency_ref: symbol.dependency_ref,
        },
        symbol_path: symbol.symbol_path,
        abi_expectation: None,
    })
}

pub(super) fn type_ref_ir_type_text(ty: &TypeRefIr) -> String {
    match ty {
        TypeRefIr::Native { name, args } if args.is_empty() => name.clone(),
        TypeRefIr::Native { name, args } => format!(
            "{name}<{}>",
            args.iter()
                .map(type_ref_ir_type_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::Record { fields } => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", type_ref_ir_type_text(ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::Nullable { inner } => format!("{}?", type_ref_ir_type_text(inner)),
        TypeRefIr::AnyInterface { interface } => {
            if interface.canonical_type_args.is_empty() {
                format!("any {}", interface.interface_abi_id)
            } else {
                format!(
                    "any {}<{}>",
                    interface.interface_abi_id,
                    interface
                        .canonical_type_args
                        .iter()
                        .map(type_ref_ir_type_text)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        TypeRefIr::Union { items } => items
            .iter()
            .map(type_ref_ir_type_text)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeRefIr::Literal { value } => match value {
            LiteralIr::String { value } => format!("{value:?}"),
            LiteralIr::Number { value } => value.to_string(),
            LiteralIr::Bool { value } => value.to_string(),
            LiteralIr::Null => "null".to_string(),
        },
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::Function {
            params,
            return_type,
        } => format!(
            "function({}) -> {}",
            params
                .iter()
                .map(|param| format!("{}: {}", param.name, type_ref_ir_type_text(&param.ty)))
                .collect::<Vec<_>>()
                .join(", "),
            type_ref_ir_type_text(return_type)
        ),
        TypeRefIr::LocalType { type_index } => format!("$localType{type_index}"),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => format!("publicationType({module_path}:{type_index})"),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            symbol.symbol_path()
        }
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
    }
}

pub(super) fn is_unknown_type_ref(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Native { name, args } if name == "unknown" && args.is_empty())
}

pub(super) fn prelude_field_type_text(raw: &str, module_path: &str) -> String {
    TypeExpr::parse_lossy(raw)
        .map_named_types(|name| {
            if name.contains('.')
                || canonical_builtin_std_type_name(name).is_some()
                || is_file_ir_builtin_type(name)
                || is_file_ir_builtin_generic_type(name)
            {
                return name.to_string();
            }
            if prelude_registry().type_decl(name).is_some() {
                return format!("{module_path}.{name}");
            }
            name.to_string()
        })
        .to_type_string()
}

pub(super) fn union_type_ir(mut items: Vec<TypeRefIr>) -> TypeRefIr {
    items.sort_by_key(type_ref_ir_type_text);
    items.dedup();
    match items.len() {
        0 => TypeRefIr::Native {
            name: "never".to_string(),
            args: Vec::new(),
        },
        1 => items.remove(0),
        _ => TypeRefIr::Union { items },
    }
}

pub(super) fn canonical_runtime_receiver_root(root: &str) -> &str {
    skiff_artifact_model::canonical_runtime_receiver_root(root)
}

pub(super) fn runtime_receiver_root_from_type_ref(ty: &TypeRefIr) -> Option<String> {
    match ty {
        TypeRefIr::Native { name, .. } => Some(canonical_runtime_receiver_root(name).to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::String { .. },
        } => Some("string".to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::Number { .. },
        } => Some("number".to_string()),
        TypeRefIr::Nullable { inner } => runtime_receiver_root_from_type_ref(inner),
        _ => None,
    }
}

pub(super) fn db_object_type_ref(symbol: ServiceSymbolRef) -> TypeRefIr {
    TypeRefIr::DbObjectSymbol { symbol }
}

fn resolve_db_object_symbol(
    name: &str,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
) -> Result<Option<ServiceSymbolRef>> {
    if let Some(symbol) = local_db_objects.resolve(name) {
        return Ok(Some(symbol));
    }
    if name.contains('.') {
        return Ok(publication_db_metadata
            .resolve_qualified(name)
            .map(PublicationDbMetadata::object_symbol));
    }
    publication_db_metadata
        .resolve_bare(name)
        .map(|metadata| metadata.map(PublicationDbMetadata::object_symbol))
}

pub(super) fn lower_type_ref(
    ty: &TypeRef,
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
    context: TypeLoweringContext<'_>,
) -> Result<TypeRefIr> {
    lower_type_text(
        &ty.name,
        type_indices,
        local_db_objects,
        publication_db_metadata,
        package_aliases,
        external_type_symbols,
        source_alias_targets,
        context,
    )
}

fn expand_source_alias_type_text(
    ty: &str,
    source_alias_targets: &BTreeMap<String, String>,
) -> Result<String> {
    fn reject_generic_alias_uses(ty: &TypeExpr, aliases: &BTreeMap<String, String>) -> Result<()> {
        match ty {
            TypeExpr::Named { name, args } => {
                if !args.is_empty() && aliases.contains_key(name) {
                    return Err(CompileError::Semantic(format!(
                        "alias {name} does not accept type arguments in type reference {}",
                        ty.to_type_string()
                    )));
                }
                for arg in args {
                    reject_generic_alias_uses(arg, aliases)?;
                }
            }
            TypeExpr::Nullable(inner) => reject_generic_alias_uses(inner, aliases)?,
            TypeExpr::AnyInterface { interface } => reject_generic_alias_uses(interface, aliases)?,
            TypeExpr::Union(parts) => {
                for part in parts {
                    reject_generic_alias_uses(part, aliases)?;
                }
            }
            TypeExpr::Record(fields) => {
                for field in fields {
                    reject_generic_alias_uses(&field.ty, aliases)?;
                }
            }
            TypeExpr::Function {
                params,
                return_type,
            } => {
                for param in params {
                    reject_generic_alias_uses(&param.ty, aliases)?;
                }
                reject_generic_alias_uses(return_type, aliases)?;
            }
            TypeExpr::EmptyRecord | TypeExpr::StringLiteral(_) => {}
        }
        Ok(())
    }

    fn expand_seen(
        raw: &str,
        aliases: &BTreeMap<String, String>,
        seen: &mut Vec<String>,
    ) -> String {
        TypeExpr::parse_lossy(raw)
            .map_named_types(|name| {
                let alias_name = name.strip_prefix("root.").unwrap_or(name);
                let Some(target) = aliases.get(name).or_else(|| aliases.get(alias_name)) else {
                    return name.to_string();
                };
                if seen.iter().any(|entry| entry == alias_name) {
                    return target.clone();
                }
                seen.push(alias_name.to_string());
                let expanded = expand_seen(target, aliases, seen);
                seen.pop();
                expanded
            })
            .to_type_string()
    }

    if source_alias_targets.is_empty() {
        return Ok(ty.to_string());
    }
    let parsed = TypeExpr::parse_lossy(ty);
    reject_generic_alias_uses(&parsed, source_alias_targets)?;
    Ok(expand_seen(ty, source_alias_targets, &mut Vec::new()))
}

pub(super) fn lower_type_text(
    ty: &str,
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
    context: TypeLoweringContext<'_>,
) -> Result<TypeRefIr> {
    let expanded_alias = expand_source_alias_type_text(ty, source_alias_targets)?;
    let ty = expanded_alias.trim();
    if let Some(inner) = ty.strip_suffix('?') {
        return Ok(TypeRefIr::Nullable {
            inner: Box::new(lower_type_text(
                inner,
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
                context,
            )?),
        });
    }

    let union = split_top_level(ty, '|');
    if union.len() > 1 {
        return Ok(TypeRefIr::Union {
            items: union
                .iter()
                .map(|part| {
                    lower_type_text(
                        part,
                        type_indices,
                        local_db_objects,
                        publication_db_metadata,
                        package_aliases,
                        external_type_symbols,
                        source_alias_targets,
                        context,
                    )
                })
                .collect::<Result<Vec<_>>>()?,
        });
    }

    if let Some(value) = string_literal(ty) {
        return Ok(TypeRefIr::Literal {
            value: LiteralIr::String { value },
        });
    }

    let parsed_type = TypeExpr::parse(ty);
    if let TypeExpr::AnyInterface { interface } = &parsed_type {
        return lower_any_interface_type_expr(
            interface,
            type_indices,
            local_db_objects,
            publication_db_metadata,
            package_aliases,
            external_type_symbols,
            source_alias_targets,
            context,
        );
    }
    if let TypeExpr::Function {
        params,
        return_type,
    } = &parsed_type
    {
        return Ok(TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| {
                    Ok(FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: lower_type_text(
                            &param.ty.to_type_string(),
                            type_indices,
                            local_db_objects,
                            publication_db_metadata,
                            package_aliases,
                            external_type_symbols,
                            source_alias_targets,
                            context,
                        )?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            return_type: Box::new(lower_type_text(
                &return_type.to_type_string(),
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
                context,
            )?),
        });
    }

    if ty.starts_with('{') && ty.ends_with('}') {
        return Ok(TypeRefIr::Record {
            fields: lower_record_type_fields(
                ty,
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
                context,
            )?,
        });
    }

    if let Some(parts) = generic_parts(ty) {
        if let Some(name) = canonical_builtin_std_type_name(parts.root) {
            if is_file_ir_native_builtin_type(&name) || is_std_abi_generic_type_name(&name) {
                return Ok(TypeRefIr::Native {
                    name,
                    args: parts
                        .args
                        .iter()
                        .map(|arg| {
                            lower_type_text(
                                arg,
                                type_indices,
                                local_db_objects,
                                publication_db_metadata,
                                package_aliases,
                                external_type_symbols,
                                source_alias_targets,
                                context,
                            )
                        })
                        .collect::<Result<Vec<_>>>()?,
                });
            }
            return Ok(TypeRefIr::PackageSymbol {
                symbol: std_package_symbol_ref(name),
            });
        }
        return lower_generic_type_text(
            parts.root,
            &parts.args,
            type_indices,
            local_db_objects,
            publication_db_metadata,
            package_aliases,
            external_type_symbols,
            source_alias_targets,
            context,
        );
    }

    lower_named_type(
        ty,
        &[],
        type_indices,
        local_db_objects,
        publication_db_metadata,
        package_aliases,
        external_type_symbols,
        source_alias_targets,
        context,
    )
}

fn lower_any_interface_type_expr(
    interface: &TypeExpr,
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
    context: TypeLoweringContext<'_>,
) -> Result<TypeRefIr> {
    let selector_text = interface.to_type_string();
    let TypeExpr::Named { name, args } = interface else {
        return Err(CompileError::Semantic(format!(
            "interface selector `{selector_text}` must be a named interface type"
        )));
    };
    let interface_identity = lower_any_interface_selector_identity(
        name,
        type_indices,
        local_db_objects,
        publication_db_metadata,
        package_aliases,
        external_type_symbols,
        context,
    )?;
    let canonical_type_args = args
        .iter()
        .map(|arg| {
            lower_type_text(
                &arg.to_type_string(),
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
                context,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(TypeRefIr::AnyInterface {
        interface: interface_instantiation_ref(interface_identity, canonical_type_args),
    })
}

fn lower_any_interface_selector_identity(
    name: &str,
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    context: TypeLoweringContext<'_>,
) -> Result<TypeRefIr> {
    let name = name.trim();
    let service_name = name.strip_prefix("root.").unwrap_or(name);
    if context.is_type_param(service_name) {
        return Err(CompileError::Semantic(format!(
            "interface selector `{name}` targets type parameter `{service_name}`, not an interface"
        )));
    }
    if let Some(canonical_name) = canonical_builtin_std_type_name(name) {
        return Err(CompileError::Semantic(format!(
            "interface selector `{name}` targets primitive/builtin type `{canonical_name}`, not an interface"
        )));
    }
    if let Some(symbol) = package_type_symbol_ref(service_name, package_aliases) {
        return Ok(TypeRefIr::PackageSymbol { symbol });
    }
    if let Some(service_path) = name.strip_prefix("root.") {
        let package_path = package_scoped_root_path("", service_path);
        if let Some(symbol) = package_type_symbol_ref(&package_path, package_aliases) {
            return Ok(TypeRefIr::PackageSymbol { symbol });
        }
    }
    if let Some(symbol) = external_type_symbols.resolve_source_text(name) {
        return Ok(TypeRefIr::ServiceSymbol {
            symbol: service_symbol_ref_from_source_key(symbol),
        });
    }
    if let Some(index) = type_indices.get(service_name) {
        return Ok(TypeRefIr::LocalType { type_index: *index });
    }
    if context.mode == TypeLoweringMode::DbTarget {
        if let Some(symbol) =
            resolve_db_object_symbol(service_name, local_db_objects, publication_db_metadata)?
        {
            return Err(CompileError::Semantic(format!(
                "interface selector `{name}` targets db object {}.{}, not an interface",
                symbol.module_path, symbol.symbol
            )));
        }
    }
    if !external_type_symbols.is_empty() && service_name.contains('.') {
        return Ok(TypeRefIr::ServiceSymbol {
            symbol: service_symbol_ref("", service_name),
        });
    }
    Err(CompileError::Semantic(format!(
        "interface selector `{name}` does not resolve to an interface"
    )))
}

fn lower_generic_type_text(
    root: &str,
    args: &[&str],
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
    context: TypeLoweringContext<'_>,
) -> Result<TypeRefIr> {
    let root = root.trim();
    if let Some(index) = type_indices.get(root) {
        return Ok(TypeRefIr::LocalType { type_index: *index });
    }
    if !is_file_ir_builtin_generic_type(root) {
        if let Some(symbol) = package_type_symbol_ref(root, package_aliases) {
            return Ok(TypeRefIr::PackageSymbol { symbol });
        }
        if let Some(service_path) = root.strip_prefix("root.") {
            let package_path = package_scoped_root_path("", service_path);
            if let Some(symbol) = package_type_symbol_ref(&package_path, package_aliases) {
                return Ok(TypeRefIr::PackageSymbol { symbol });
            }
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref("", service_path),
            });
        }
        if root.contains('.') {
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref("", root),
            });
        }
        return Err(unsupported_file_ir_generic_root(root));
    }
    Ok(TypeRefIr::Native {
        name: root.to_string(),
        args: args
            .iter()
            .map(|arg| {
                lower_type_text(
                    arg,
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                    context,
                )
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

pub(super) fn lower_named_type(
    name: &str,
    type_args: &[TypeRef],
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
    context: TypeLoweringContext<'_>,
) -> Result<TypeRefIr> {
    let name = name.trim();
    if let Some(canonical_name) = canonical_builtin_std_type_name(name) {
        if is_file_ir_native_builtin_type(&canonical_name) {
            return Ok(TypeRefIr::Native {
                name: canonical_name,
                args: type_args
                    .iter()
                    .map(|arg| {
                        lower_type_ref(
                            arg,
                            type_indices,
                            local_db_objects,
                            publication_db_metadata,
                            package_aliases,
                            external_type_symbols,
                            source_alias_targets,
                            context,
                        )
                    })
                    .collect::<Result<Vec<_>>>()?,
            });
        }
        return Ok(TypeRefIr::PackageSymbol {
            symbol: std_package_symbol_ref(canonical_name),
        });
    }
    let service_name = name.strip_prefix("root.").unwrap_or(name);
    let package_scoped_root = name
        .strip_prefix("root.")
        .map(|service_path| package_scoped_root_path("", service_path));
    if type_args.is_empty() {
        if context.is_type_param(service_name) {
            return Ok(TypeRefIr::TypeParam {
                name: service_name.to_string(),
            });
        }
        if context.mode == TypeLoweringMode::DbTarget {
            if let Some(symbol) =
                resolve_db_object_symbol(service_name, local_db_objects, publication_db_metadata)?
            {
                return Ok(db_object_type_ref(symbol));
            }
        }
        if let Some(index) = type_indices.get(service_name) {
            return Ok(TypeRefIr::LocalType { type_index: *index });
        }
        if is_file_ir_builtin_type(name) {
            return Ok(TypeRefIr::Native {
                name: service_name.to_string(),
                args: Vec::new(),
            });
        }
        if let Some(symbol) = package_type_symbol_ref(service_name, package_aliases) {
            return Ok(TypeRefIr::PackageSymbol { symbol });
        }
        if let Some(package_scoped_root) = package_scoped_root.as_deref() {
            if let Some(symbol) = package_type_symbol_ref(package_scoped_root, package_aliases) {
                return Ok(TypeRefIr::PackageSymbol { symbol });
            }
        }
        if let Some(symbol) = external_type_symbols.resolve_source_text(service_name) {
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref_from_source_key(symbol),
            });
        }
        if !external_type_symbols.is_empty() {
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref("", service_name),
            });
        }
        if service_name.contains('.') {
            return Err(unsupported(format!(
                "external type reference `{name}` is not supported by the File IR unit emitter yet; package/service type refs require structured resolution"
            )));
        }
        return Err(CompileError::Semantic(format!(
            "unresolved type `{name}` in File IR unit"
        )));
    }

    if type_indices.contains_key(service_name) {
        return Err(unsupported(format!(
            "generic local type `{name}` is not supported by the File IR unit emitter yet"
        )));
    }
    if !is_file_ir_builtin_generic_type(service_name) {
        if let Some(symbol) = package_type_symbol_ref(service_name, package_aliases) {
            return Ok(TypeRefIr::PackageSymbol { symbol });
        }
        if let Some(package_scoped_root) = package_scoped_root.as_deref() {
            if let Some(symbol) = package_type_symbol_ref(package_scoped_root, package_aliases) {
                return Ok(TypeRefIr::PackageSymbol { symbol });
            }
        }
        if let Some(symbol) = external_type_symbols.resolve_source_text(service_name) {
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref_from_source_key(symbol),
            });
        }
        if !external_type_symbols.is_empty() {
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref("", service_name),
            });
        }
        if service_name.contains('.') {
            return Err(unsupported(format!(
                "external type reference `{name}` is not supported by the File IR unit emitter yet; package/service type refs require structured resolution"
            )));
        }
        return Err(CompileError::Semantic(format!(
            "unresolved type `{name}` in File IR unit"
        )));
    }
    Ok(TypeRefIr::Native {
        name: service_name.to_string(),
        args: type_args
            .iter()
            .map(|arg| {
                lower_type_ref(
                    arg,
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                    context,
                )
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

pub(super) fn is_file_ir_builtin_type(name: &str) -> bool {
    let registry = prelude_registry();
    if registry.is_native_type_name(name) {
        return true;
    }
    matches!(
        name,
        "string" | "integer" | "number" | "bool" | "boolean" | "null" | "never" | "void"
    )
}

fn std_package_symbol_ref(symbol_path: impl Into<String>) -> PackageSymbolRef {
    PackageSymbolRef {
        package: PackageRefIr::PackageId {
            package_id: SKIFF_STD_PUBLICATION_ID.to_string(),
        },
        symbol_path: symbol_path.into(),
        abi_expectation: None,
    }
}

fn canonical_builtin_std_type_name(name: &str) -> Option<String> {
    let name = name.trim();
    if prelude_registry().is_native_type_name(name) {
        return Some(name.to_string());
    }
    if let Some(bare) = match name {
        "std.collection.Array" => Some("Array"),
        "std.collection.Map" => Some("Map"),
        "std.stream.Stream" => Some("Stream"),
        "std.bytes.bytes" => Some("bytes"),
        "std.date.Date" => Some("Date"),
        _ => None,
    } {
        return Some(bare.to_string());
    }
    let symbol = prelude_registry().known_type_symbol(name)?;
    if symbol == "config.DecodeError" {
        return Some(symbol);
    }
    if let Some(bare) = match symbol.as_str() {
        "std.collection.Array" => Some("Array"),
        "std.collection.Map" => Some("Map"),
        "std.stream.Stream" => Some("Stream"),
        "std.bytes.bytes" => Some("bytes"),
        "std.date.Date" => Some("Date"),
        _ => None,
    } {
        return Some(bare.to_string());
    }
    symbol.starts_with("std.").then_some(symbol)
}

fn is_file_ir_native_builtin_type(name: &str) -> bool {
    prelude_registry().is_native_type_name(name) || name == "config.DecodeError"
}

pub(super) fn is_file_ir_builtin_generic_type(root: &str) -> bool {
    prelude_registry().is_native_type_name(root) || matches!(root, "DbUpsertResult")
}

fn is_std_abi_generic_type_name(name: &str) -> bool {
    matches!(
        name,
        "std.websocket.WebSocketConnectResult"
            | "std.websocket.WebSocketConnection"
            | "std.websocket.WebSocketReceiveEvent"
    )
}

fn unsupported_file_ir_generic_root(root: &str) -> CompileError {
    if root.contains('.') {
        return unsupported(format!(
            "external generic type reference `{root}` is not supported by the File IR unit emitter yet; package/service type refs require structured resolution"
        ));
    }
    unsupported(format!(
        "unresolved generic type root `{root}` in File IR unit; expected an explicit builtin generic type"
    ))
}

fn lower_record_type_fields(
    ty: &str,
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
    context: TypeLoweringContext<'_>,
) -> Result<BTreeMap<String, TypeRefIr>> {
    parse_record_type_fields(ty)
        .map_err(|error| match error {
            RecordTypeFieldParseError::NotRecordType => {
                CompileError::Semantic(format!("invalid record type `{ty}`"))
            }
            RecordTypeFieldParseError::InvalidField(field) => {
                CompileError::Semantic(format!("invalid record type field `{field}`"))
            }
        })?
        .into_iter()
        .map(|field| {
            Ok((
                field.name.to_string(),
                lower_type_text(
                    field.ty,
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                    context,
                )?,
            ))
        })
        .collect()
}

fn unsupported(message: impl Into<String>) -> CompileError {
    CompileError::Semantic(message.into())
}
