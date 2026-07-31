use std::collections::BTreeMap;

use skiff_artifact_model::{
    contract_types::{ContractLiteral, ContractTypeRef, PackageTypeRef},
    FunctionTypeParamIr, InterfaceInstantiationRef, LiteralIr, NominalTypeRefBaseIr, PackageRefIr,
    PackageSymbolRef, TypeRefIr,
};

pub fn walk_type_ref(ty: &TypeRefIr, visit: &mut impl FnMut(&TypeRefIr)) {
    visit(ty);
    for child in type_ref_children(ty) {
        walk_type_ref(child.ty, visit);
    }
}

pub fn any_type_ref(ty: &TypeRefIr, predicate: &mut impl FnMut(&TypeRefIr) -> bool) -> bool {
    if predicate(ty) {
        return true;
    }
    type_ref_children(ty)
        .into_iter()
        .any(|child| any_type_ref(child.ty, predicate))
}

pub fn map_type_ref(ty: TypeRefIr, map: &mut impl FnMut(TypeRefIr) -> TypeRefIr) -> TypeRefIr {
    let ty = match ty {
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name,
            args: args.into_iter().map(|arg| map_type_ref(arg, map)).collect(),
        },
        TypeRefIr::LocalType { type_index } => TypeRefIr::LocalType { type_index },
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => TypeRefIr::PublicationType {
            module_path,
            type_index,
        },
        TypeRefIr::ServiceSymbol { symbol } => TypeRefIr::ServiceSymbol { symbol },
        TypeRefIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol { symbol },
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base,
            arguments: arguments
                .into_iter()
                .map(|argument| map_type_ref(argument, map))
                .collect(),
        },
        TypeRefIr::DbObjectSymbol { symbol } => TypeRefIr::DbObjectSymbol { symbol },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .into_iter()
                .map(|(name, field_ty)| (name, map_type_ref(field_ty, map)))
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .into_iter()
                .map(|item| map_type_ref(item, map))
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(map_type_ref(*inner, map)),
        },
        TypeRefIr::Literal { value } => TypeRefIr::Literal { value },
        TypeRefIr::TypeParam { name } => TypeRefIr::TypeParam { name },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id,
                canonical_type_args: interface
                    .canonical_type_args
                    .into_iter()
                    .map(|arg| map_type_ref(arg, map))
                    .collect(),
            },
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .into_iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name,
                    ty: map_type_ref(param.ty, map),
                })
                .collect(),
            return_type: Box::new(map_type_ref(*return_type, map)),
        },
    };
    map(ty)
}

pub fn substitute_type_params_in_type_ref(
    ty: TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> TypeRefIr {
    map_type_ref(ty, &mut |ty| match ty {
        TypeRefIr::TypeParam { name } => substitutions
            .get(&name)
            .cloned()
            .unwrap_or(TypeRefIr::TypeParam { name }),
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin { name, args },
        TypeRefIr::LocalType { type_index } => TypeRefIr::LocalType { type_index },
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => TypeRefIr::PublicationType {
            module_path,
            type_index,
        },
        TypeRefIr::ServiceSymbol { symbol } => TypeRefIr::ServiceSymbol { symbol },
        TypeRefIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol { symbol },
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        },
        TypeRefIr::AppliedNominal { base, arguments } => {
            TypeRefIr::AppliedNominal { base, arguments }
        }
        TypeRefIr::DbObjectSymbol { symbol } => TypeRefIr::DbObjectSymbol { symbol },
        TypeRefIr::Record { fields } => TypeRefIr::Record { fields },
        TypeRefIr::Union { items } => TypeRefIr::Union { items },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable { inner },
        TypeRefIr::Literal { value } => TypeRefIr::Literal { value },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface { interface },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params,
            return_type,
        },
    })
}

pub fn substitute_type_params_in_type_ref_ref(
    ty: &TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> TypeRefIr {
    substitute_type_params_in_type_ref(ty.clone(), substitutions)
}

pub fn contains_any_interface(ty: &TypeRefIr) -> bool {
    any_type_ref(ty, &mut |ty| matches!(ty, TypeRefIr::AnyInterface { .. }))
}

pub fn contains_boundary_unsafe_type(ty: &TypeRefIr) -> bool {
    contains_any_interface(ty)
}

/// Renders the canonical debug/display text for a type ref.
///
/// Absorbs the former private implementations in `compiler/source`
/// (`type_resolution_model.rs` and `expression_type_model.rs`). The two
/// copies differed only in how they formatted an `AppliedNominal` base; this
/// core version uses the direct nominal-base formatting variant, which is
/// byte-identical to the intermediate-`TypeRefIr` variant for every nominal
/// base (locked by differential evidence in the Phase 3 leaf task).
pub fn debug_text(ty: &TypeRefIr) -> String {
    match ty {
        TypeRefIr::Builtin { name, args } if args.is_empty() => name.clone(),
        TypeRefIr::Builtin { name, args } => format!(
            "{name}<{}>",
            args.iter().map(debug_text).collect::<Vec<_>>().join(", ")
        ),
        TypeRefIr::Nullable { inner } => format!("{}?", debug_text(inner)),
        TypeRefIr::Union { items } => items.iter().map(debug_text).collect::<Vec<_>>().join(" | "),
        TypeRefIr::Literal {
            value: LiteralIr::String { value },
        } => serde_json::to_string(value).unwrap_or_else(|_| "\"<string>\"".to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::Null,
        } => "null".to_string(),
        TypeRefIr::Literal { .. } => "<literal>".to_string(),
        TypeRefIr::LocalType { type_index } => format!("#{type_index}"),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => format!("{module_path}#{type_index}"),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            symbol.symbol_path()
        }
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => format!("{package_id}::{stable_schema_key}"),
        TypeRefIr::AppliedNominal { base, arguments } => format!(
            "{}<{}>",
            nominal_base_debug_text(base),
            arguments
                .iter()
                .map(debug_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::AnyInterface { interface } => {
            let interface_name = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                .map_or_else(
                    |_| interface.interface_abi_id.clone(),
                    |identity| debug_text(&identity),
                );
            if interface.canonical_type_args.is_empty() {
                format!("any {interface_name}")
            } else {
                format!(
                    "any {}<{}>",
                    interface_name,
                    interface
                        .canonical_type_args
                        .iter()
                        .map(debug_text)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        TypeRefIr::Record { .. } => "{}".to_string(),
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::Function { .. } => "fn".to_string(),
    }
}

fn nominal_base_debug_text(base: &NominalTypeRefBaseIr) -> String {
    match base {
        NominalTypeRefBaseIr::LocalType { type_index } => format!("#{type_index}"),
        NominalTypeRefBaseIr::PublicationType {
            module_path,
            type_index,
        } => format!("{module_path}#{type_index}"),
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => symbol.symbol_path(),
        NominalTypeRefBaseIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
        NominalTypeRefBaseIr::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => format!("{package_id}::{stable_schema_key}"),
    }
}

/// Returns the type of a record/union field, including synthetic fields of
/// the `CatchResult` / `DbUpsertResult` / `Exception` native shapes.
///
/// Absorbs the union of the two private copies
/// (`type_resolution_model.rs` `record_field_type_from_ir` and
/// `expression_type_model.rs` `record_field_type_from_ir`): record lookup,
/// recursive union combine via canonical `normalize_union`, plus the
/// shape-specific fields.
pub fn record_field_type(ty: &TypeRefIr, field: &str) -> Option<TypeRefIr> {
    match ty {
        TypeRefIr::Record { fields } => fields.get(field).cloned(),
        TypeRefIr::Union { items } => {
            let mut field_types = Vec::new();
            for item in items {
                field_types.push(record_field_type(item, field)?);
            }
            Some(normalize_union(TypeRefIr::Union { items: field_types }))
        }
        TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2 => {
            match field {
                "tag" => Some(normalize_union(TypeRefIr::Union {
                    items: vec![literal_string("ok"), literal_string("err")],
                })),
                _ => None,
            }
        }
        TypeRefIr::Builtin { name, args } if name == "DbUpsertResult" && args.len() == 1 => {
            match field {
                "inserted" => Some(TypeRefIr::builtin("bool")),
                "value" => Some(args[0].clone()),
                _ => None,
            }
        }
        TypeRefIr::Builtin { name, args } if name == "Exception" && args.len() == 1 => {
            match field {
                "error" => Some(args[0].clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Canonical union normalization: recursive flatten of nested unions, null
/// folding (including `Nullable`), sort by debug text, dedup, and
/// single-item / null-only collapsing.
///
/// This is the canonical semantics selected by the design: the trm private
/// copy (`type_resolution_model.rs` `normalize_source_type_ref` /
/// `normalize_source_union` / `collect_source_union_member`).
pub fn normalize_union(ty: TypeRefIr) -> TypeRefIr {
    match ty {
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name,
            args: args.into_iter().map(normalize_union).collect(),
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base,
            arguments: arguments.into_iter().map(normalize_union).collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .into_iter()
                .map(|(name, ty)| (name, normalize_union(ty)))
                .collect(),
        },
        TypeRefIr::Union { items } => normalize_union_items(items, false),
        TypeRefIr::Nullable { inner } => normalize_union_items(vec![*inner], true),
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id,
                canonical_type_args: interface
                    .canonical_type_args
                    .into_iter()
                    .map(normalize_union)
                    .collect(),
            },
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .into_iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name,
                    ty: normalize_union(param.ty),
                })
                .collect(),
            return_type: Box::new(normalize_union(*return_type)),
        },
        other => other,
    }
}

fn normalize_union_items(items: Vec<TypeRefIr>, force_nullable: bool) -> TypeRefIr {
    let mut flattened = Vec::new();
    let mut has_null = force_nullable;
    for item in items {
        collect_normalized_union_member(normalize_union(item), &mut flattened, &mut has_null);
    }
    flattened.sort_by_key(debug_text);
    flattened.dedup();
    let base = match flattened.as_slice() {
        [] if has_null => return TypeRefIr::builtin("null"),
        [] => return TypeRefIr::Union { items: flattened },
        [only] => only.clone(),
        _ => TypeRefIr::Union { items: flattened },
    };
    if has_null {
        TypeRefIr::Nullable {
            inner: Box::new(base),
        }
    } else {
        base
    }
}

fn collect_normalized_union_member(
    item: TypeRefIr,
    flattened: &mut Vec<TypeRefIr>,
    has_null: &mut bool,
) {
    match item {
        TypeRefIr::Union { items } => {
            for item in items {
                collect_normalized_union_member(item, flattened, has_null);
            }
        }
        TypeRefIr::Nullable { inner } => {
            *has_null = true;
            collect_normalized_union_member(*inner, flattened, has_null);
        }
        item if is_null_type(&item) => *has_null = true,
        item => flattened.push(item),
    }
}

/// Returns the single item type of an `Array` / `Stream` / `Map` native
/// container: the sole argument for 1-argument containers, or the key type
/// for a 2-argument `Map`.
///
/// Absorbs `single_for_item_type` / `single_for_item_projection`
/// (`expression_type_model.rs` 4776 / 4827), including the `std.*` full names.
pub fn single_item(ty: &TypeRefIr) -> Option<&TypeRefIr> {
    let TypeRefIr::Builtin { name, args } = ty else {
        return None;
    };
    match name.as_str() {
        "Array" | "Stream" | "std.collection.Array" | "std.stream.Stream" if args.len() == 1 => {
            Some(&args[0])
        }
        "Map" | "std.collection.Map" if args.len() == 2 => Some(&args[0]),
        _ => None,
    }
}

/// Returns `(key, value)` for a 2-argument `Map` native container.
///
/// Absorbs `map_entry_types` / `map_entry_projections`
/// (`expression_type_model.rs` 4808 / 4842), including the `std.collection.Map`
/// full name.
pub fn map_entry(ty: &TypeRefIr) -> Option<(&TypeRefIr, &TypeRefIr)> {
    let TypeRefIr::Builtin { name, args } = ty else {
        return None;
    };
    (matches!(name.as_str(), "Map" | "std.collection.Map") && args.len() == 2)
        .then(|| (&args[0], &args[1]))
}

/// Returns the payload type of a 1-argument `Exception` native shape.
pub fn exception_payload(ty: &TypeRefIr) -> Option<&TypeRefIr> {
    let TypeRefIr::Builtin { name, args } = ty else {
        return None;
    };
    (name == "Exception" && args.len() == 1).then(|| &args[0])
}

/// Returns the tag-branch types of a discriminated record-like shape:
/// `Union` items, `CatchResult` ok/err records, or a single `Record`.
///
/// Absorbs `discriminated_record_branches` and `catch_result_branch_types`
/// (`expression_type_model.rs` 5154 / 5165).
pub fn catch_result_branches(ty: &TypeRefIr) -> Option<Vec<TypeRefIr>> {
    match ty {
        TypeRefIr::Union { items } => Some(items.clone()),
        TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2 => {
            Some(catch_result_branch_types(&args[0], &args[1]))
        }
        TypeRefIr::Record { .. } => Some(vec![ty.clone()]),
        _ => None,
    }
}

fn catch_result_branch_types(value: &TypeRefIr, error: &TypeRefIr) -> Vec<TypeRefIr> {
    vec![
        TypeRefIr::Record {
            fields: BTreeMap::from([
                ("tag".to_string(), literal_string("ok")),
                ("value".to_string(), value.clone()),
            ]),
        },
        TypeRefIr::Record {
            fields: BTreeMap::from([
                ("tag".to_string(), literal_string("err")),
                ("exception".to_string(), exception_type_ir(error.clone())),
            ]),
        },
    ]
}

fn literal_string(value: &str) -> TypeRefIr {
    TypeRefIr::Literal {
        value: LiteralIr::String {
            value: value.to_string(),
        },
    }
}

fn exception_type_ir(error: TypeRefIr) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: "Exception".to_string(),
        args: vec![error],
    }
}

/// Returns whether the type is the null type (builtin `"null"` or the null
/// literal).
///
/// Absorbs the former private implementations `is_null_type_ir` and
/// `type_ir_is_null` in `compiler/source`.
pub fn is_null_type(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, .. } if name == "null")
        || matches!(
            ty,
            TypeRefIr::Literal {
                value: LiteralIr::Null
            }
        )
}

/// Returns whether the type contains a type parameter anywhere.
///
/// Absorbs the former private implementations `type_contains_type_param`,
/// `type_contains_unresolved_param`, and
/// `type_ref_contains_type_parameter` in `compiler/source` and
/// `compiler/projection`.
pub fn contains_type_param(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::TypeParam { .. } => true,
        TypeRefIr::Builtin { args, .. } | TypeRefIr::Union { items: args } => {
            args.iter().any(contains_type_param)
        }
        TypeRefIr::AppliedNominal { arguments, .. } => arguments.iter().any(contains_type_param),
        TypeRefIr::Nullable { inner } => contains_type_param(inner),
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .any(contains_type_param),
        TypeRefIr::Record { fields } => fields.values().any(contains_type_param),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params.iter().any(|param| contains_type_param(&param.ty))
                || contains_type_param(return_type)
        }
        TypeRefIr::Literal { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. } => false,
    }
}

/// Canonical builtin shape classification for the names used across the
/// compiler source type models. Replaces bare name-string literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinShape {
    Array,
    Stream,
    Map,
    Exception,
    CatchResult,
    DbUpsertResult,
    Json,
    JsonObject,
    Null,
    Void,
    Never,
    Unknown,
    String,
    Integer,
    Number,
    Bool,
}

impl BuiltinShape {
    /// Resolves a builtin type name to its shape, including the
    /// `std.collection.Array` / `std.stream.Stream` / `std.collection.Map`
    /// full names.
    pub fn of_name(name: &str) -> Option<BuiltinShape> {
        Some(match name {
            "Array" | "std.collection.Array" => BuiltinShape::Array,
            "Stream" | "std.stream.Stream" => BuiltinShape::Stream,
            "Map" | "std.collection.Map" => BuiltinShape::Map,
            "Exception" => BuiltinShape::Exception,
            "CatchResult" => BuiltinShape::CatchResult,
            "DbUpsertResult" => BuiltinShape::DbUpsertResult,
            "Json" => BuiltinShape::Json,
            "JsonObject" => BuiltinShape::JsonObject,
            "null" => BuiltinShape::Null,
            "void" => BuiltinShape::Void,
            "never" => BuiltinShape::Never,
            "unknown" => BuiltinShape::Unknown,
            "string" => BuiltinShape::String,
            "integer" => BuiltinShape::Integer,
            "number" => BuiltinShape::Number,
            "bool" => BuiltinShape::Bool,
            _ => return None,
        })
    }
}

/// Projects an ABI/wire `PackageTypeRef` into the canonical `TypeRefIr` using
/// the folded strategy: `PackageSchema` becomes `PackageSymbol`, `Local`
/// stays verbatim (no recursive rewriting), and `AnyInterface` identity uses
/// `serde_json::to_string`.
///
/// Absorbs the folded copies (`type_projection.rs`
/// `contract_type_ref_to_ir_from_package` and `lowering`
/// `executable_type_projection.rs` `execution_type_ref`).
pub fn package_type_ref_to_ir(ty: &PackageTypeRef) -> TypeRefIr {
    match ty {
        PackageTypeRef::Local { local_type } => local_type.clone(),
        PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.clone(),
                },
                symbol_path: stable_schema_key.clone(),
                abi_expectation: None,
            },
        },
        PackageTypeRef::Container { name, arguments } => TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments.iter().map(package_type_ref_to_ir).collect(),
        },
        PackageTypeRef::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(package_type_ref_to_ir(inner)),
        },
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: serde_json::to_string(&package_type_ref_to_ir(interface))
                    .expect("PackageTypeRef interface identity must serialize"),
                canonical_type_args: arguments.iter().map(package_type_ref_to_ir).collect(),
            },
        },
    }
}

/// Projects an ABI/wire `PackageTypeRef` into the canonical `TypeRefIr` using
/// the exact strategy: `PackageSchema` is preserved, and `AnyInterface`
/// identity uses canonical JSON (via `skiff_canonical_json::canonical_json_bytes`,
/// byte-identical to the artifact-identity `type_ref_abi_key` helper).
///
/// Absorbs the exact copy (`projection/.../public_instances/interfaces.rs`
/// `package_type_ref_to_ir`).
pub fn package_type_ref_to_ir_exact(ty: &PackageTypeRef) -> TypeRefIr {
    match ty {
        PackageTypeRef::Local { local_type } => local_type.clone(),
        PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => TypeRefIr::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
        PackageTypeRef::Container { name, arguments } => TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments.iter().map(package_type_ref_to_ir_exact).collect(),
        },
        PackageTypeRef::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(package_type_ref_to_ir_exact(inner)),
        },
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            let interface_ir = package_type_ref_to_ir_exact(interface);
            TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id: canonical_json_key(&interface_ir),
                    canonical_type_args: arguments
                        .iter()
                        .map(package_type_ref_to_ir_exact)
                        .collect(),
                },
            }
        }
    }
}

fn canonical_json_key(ty: &TypeRefIr) -> String {
    let bytes = skiff_canonical_json::canonical_json_bytes(ty)
        .expect("TypeRefIr must serialize for canonical identity");
    String::from_utf8(bytes).expect("canonical JSON must be UTF-8")
}

/// Projects a `ContractTypeRef` into the canonical `TypeRefIr` using the
/// folded strategy.
///
/// Absorbs `contract_type_ref_to_ir` / `contract_type_ref_to_ir_from_package`
/// (`type_projection.rs` 261 / 303).
pub fn contract_type_ref_to_ir(ty: &ContractTypeRef) -> TypeRefIr {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments.iter().map(contract_type_ref_to_ir).collect(),
        },
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.clone(),
                },
                symbol_path: stable_schema_key.clone(),
                abi_expectation: None,
            },
        },
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: serde_json::to_string(&contract_type_ref_to_ir(interface))
                    .expect("ContractTypeRef interface identity must serialize"),
                canonical_type_args: arguments.iter().map(contract_type_ref_to_ir).collect(),
            },
        },
        ContractTypeRef::TypeParam { name } => TypeRefIr::TypeParam { name: name.clone() },
        ContractTypeRef::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| (name.clone(), contract_type_ref_to_ir(ty)))
                .collect(),
        },
        ContractTypeRef::StructuralUnion { variants } => TypeRefIr::Union {
            items: variants.iter().map(contract_type_ref_to_ir).collect(),
        },
        ContractTypeRef::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(contract_type_ref_to_ir(inner)),
        },
        ContractTypeRef::Literal { value } => TypeRefIr::Literal {
            value: match value {
                ContractLiteral::String { value } => LiteralIr::String {
                    value: value.clone(),
                },
            },
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeRefVisitPathSegment {
    NativeArg { name: String, index: usize },
    AppliedNominalArgument { index: usize },
    RecordField { name: String },
    UnionItem { index: usize },
    NullableInner,
    AnyInterfaceTypeArg { index: usize },
    FunctionParam { name: String, index: usize },
    FunctionReturn,
}

#[derive(Clone, Debug)]
pub struct TypeRefChild<'a> {
    pub ty: &'a TypeRefIr,
    pub segment: TypeRefVisitPathSegment,
}

pub fn type_ref_children(ty: &TypeRefIr) -> Vec<TypeRefChild<'_>> {
    match ty {
        TypeRefIr::Builtin { name, args } => args
            .iter()
            .enumerate()
            .map(|(index, ty)| TypeRefChild {
                ty,
                segment: TypeRefVisitPathSegment::NativeArg {
                    name: name.clone(),
                    index,
                },
            })
            .collect(),
        TypeRefIr::AppliedNominal { arguments, .. } => arguments
            .iter()
            .enumerate()
            .map(|(index, ty)| TypeRefChild {
                ty,
                segment: TypeRefVisitPathSegment::AppliedNominalArgument { index },
            })
            .collect(),
        TypeRefIr::Record { fields } => fields
            .iter()
            .map(|(name, ty)| TypeRefChild {
                ty,
                segment: TypeRefVisitPathSegment::RecordField { name: name.clone() },
            })
            .collect(),
        TypeRefIr::Union { items } => items
            .iter()
            .enumerate()
            .map(|(index, ty)| TypeRefChild {
                ty,
                segment: TypeRefVisitPathSegment::UnionItem { index },
            })
            .collect(),
        TypeRefIr::Nullable { inner } => vec![TypeRefChild {
            ty: inner,
            segment: TypeRefVisitPathSegment::NullableInner,
        }],
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .enumerate()
            .map(|(index, ty)| TypeRefChild {
                ty,
                segment: TypeRefVisitPathSegment::AnyInterfaceTypeArg { index },
            })
            .collect(),
        TypeRefIr::Function {
            params,
            return_type,
        } => params
            .iter()
            .enumerate()
            .map(|(index, param)| TypeRefChild {
                ty: &param.ty,
                segment: TypeRefVisitPathSegment::FunctionParam {
                    name: param.name.clone(),
                    index,
                },
            })
            .chain(std::iter::once(TypeRefChild {
                ty: return_type,
                segment: TypeRefVisitPathSegment::FunctionReturn,
            }))
            .collect(),
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => Vec::new(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeRefVisitPath {
    segments: Vec<TypeRefVisitPathSegment>,
}

impl TypeRefVisitPath {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn segments(&self) -> &[TypeRefVisitPathSegment] {
        &self.segments
    }

    pub fn child(&self, segment: TypeRefVisitPathSegment) -> Self {
        let mut segments = self.segments.clone();
        segments.push(segment);
        Self { segments }
    }
}

#[derive(Clone, Debug)]
pub struct TypeRefVisit<'a> {
    pub ty: &'a TypeRefIr,
    pub path: TypeRefVisitPath,
}

pub fn walk_type_ref_with_path(ty: &TypeRefIr, visit: &mut impl FnMut(TypeRefVisit<'_>)) {
    walk_type_ref_with_path_at(ty, TypeRefVisitPath::empty(), visit);
}

fn walk_type_ref_with_path_at(
    ty: &TypeRefIr,
    path: TypeRefVisitPath,
    visit: &mut impl FnMut(TypeRefVisit<'_>),
) {
    visit(TypeRefVisit {
        ty,
        path: path.clone(),
    });
    for child in type_ref_children(ty) {
        walk_type_ref_with_path_at(child.ty, path.child(child.segment), visit);
    }
}

#[cfg(test)]
mod tests;
