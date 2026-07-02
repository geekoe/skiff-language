use std::collections::{BTreeMap, BTreeSet};

use crate::prelude::PreludeProjection;
use crate::{
    runtime_manifest_model::JsonSchema,
    {
        contract::{ContractProjection, ContractProjectionIndex},
        schema_metadata::with_std_symbol,
    },
};
use skiff_artifact_model::{LiteralIr, TypeRefIr};
use skiff_compiler_projection_input::{EntryTypeSpec, PackageAbiType, PackageAbiTypeDescriptor};

use super::{
    is_connection_message_type, is_websocket_receive_event_root,
    type_ref_ir_source_text_with_local_types,
};

pub fn connection_message_schema(prelude: &PreludeProjection) -> JsonSchema {
    with_std_symbol(
        prelude,
        JsonSchema::one_of(vec![
            text_connection_message_schema(prelude),
            binary_connection_message_schema(prelude),
        ]),
        "ConnectionMessage",
    )
}

pub fn text_connection_message_schema(prelude: &PreludeProjection) -> JsonSchema {
    let mut properties = BTreeMap::new();
    properties.insert(
        "tag".to_string(),
        JsonSchema::string_enum(vec!["text".to_string()]),
    );
    properties.insert("text".to_string(), JsonSchema::string());
    with_std_symbol(
        prelude,
        JsonSchema::object(
            properties,
            vec!["tag".to_string(), "text".to_string()],
            false,
        ),
        "TextConnectionMessage",
    )
}

pub fn binary_connection_message_schema(prelude: &PreludeProjection) -> JsonSchema {
    let mut properties = BTreeMap::new();
    properties.insert(
        "tag".to_string(),
        JsonSchema::string_enum(vec!["binary".to_string()]),
    );
    properties.insert("base64".to_string(), JsonSchema::string());
    with_std_symbol(
        prelude,
        JsonSchema::object(
            properties,
            vec!["tag".to_string(), "base64".to_string()],
            false,
        ),
        "BinaryConnectionMessage",
    )
}

fn websocket_connection_policy_schema() -> JsonSchema {
    let mut properties = BTreeMap::new();
    properties.insert("maxConnections".to_string(), JsonSchema::integer());
    properties.insert(
        "overflow".to_string(),
        JsonSchema::string_enum(vec!["close-oldest".to_string(), "reject-new".to_string()]),
    );
    properties.insert("closeCode".to_string(), JsonSchema::integer().nullable());
    properties.insert("closeReason".to_string(), JsonSchema::string().nullable());

    JsonSchema::object(
        properties,
        vec!["maxConnections".to_string(), "overflow".to_string()],
        false,
    )
}

pub fn gateway_connect_result_schema(context_schema: JsonSchema) -> JsonSchema {
    let mut accept_properties = BTreeMap::new();
    accept_properties.insert(
        "tag".to_string(),
        JsonSchema::string_enum(vec!["accept".to_string()]),
    );
    accept_properties.insert("context".to_string(), context_schema);
    accept_properties.insert(
        "businessIdentity".to_string(),
        JsonSchema::string().nullable(),
    );
    accept_properties.insert(
        "connectionPolicy".to_string(),
        websocket_connection_policy_schema().nullable(),
    );

    let mut reject_properties = BTreeMap::new();
    reject_properties.insert(
        "tag".to_string(),
        JsonSchema::string_enum(vec!["reject".to_string()]),
    );
    reject_properties.insert("code".to_string(), JsonSchema::integer());
    reject_properties.insert("reason".to_string(), JsonSchema::string());

    JsonSchema::one_of(vec![
        JsonSchema::object(
            accept_properties,
            vec!["tag".to_string(), "context".to_string()],
            false,
        ),
        JsonSchema::object(
            reject_properties,
            vec!["tag".to_string(), "code".to_string(), "reason".to_string()],
            false,
        ),
    ])
}

pub fn package_runtime_schema_for_type_spec(
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    source_module: &str,
    ty: &EntryTypeSpec,
    schema_types: &BTreeMap<String, PackageAbiType>,
    service_type_names: &BTreeMap<String, String>,
) -> JsonSchema {
    package_runtime_schema_for_type_ref(
        contract_projection,
        projection_index,
        source_module,
        &ty.ir,
        &ty.local_type_names,
        schema_types,
        service_type_names,
    )
}

pub fn package_runtime_schema_for_type_ref(
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    source_module: &str,
    ty: &TypeRefIr,
    local_type_names: &BTreeMap<u32, String>,
    schema_types: &BTreeMap<String, PackageAbiType>,
    service_type_names: &BTreeMap<String, String>,
) -> JsonSchema {
    let mut seen = BTreeSet::new();
    let schema = package_runtime_schema_for_type_ref_inner(
        contract_projection,
        projection_index,
        source_module,
        ty,
        local_type_names,
        schema_types,
        service_type_names,
        &mut seen,
    );
    rewrite_schema_symbols(schema, service_type_names)
}

#[allow(clippy::too_many_arguments)]
fn package_runtime_schema_for_type_ref_inner(
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    source_module: &str,
    ty: &TypeRefIr,
    local_type_names: &BTreeMap<u32, String>,
    schema_types: &BTreeMap<String, PackageAbiType>,
    service_type_names: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
) -> JsonSchema {
    match ty {
        TypeRefIr::Native { name, args } => package_runtime_schema_for_builtin(
            contract_projection,
            projection_index,
            source_module,
            name,
            args,
            local_type_names,
            schema_types,
            service_type_names,
            seen,
        ),
        TypeRefIr::LocalType { type_index } => local_type_names
            .get(type_index)
            .and_then(|name| {
                package_runtime_schema_for_named_type(
                    contract_projection,
                    projection_index,
                    source_module,
                    name,
                    schema_types,
                    service_type_names,
                    seen,
                )
            })
            .unwrap_or_else(|| {
                contract_projection.schema_for_source_type_ref(projection_index, source_module, ty)
            }),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            package_runtime_schema_for_named_type(
                contract_projection,
                projection_index,
                source_module,
                &symbol.symbol_path(),
                schema_types,
                service_type_names,
                seen,
            )
            .or_else(|| {
                package_runtime_schema_for_named_type(
                    contract_projection,
                    projection_index,
                    source_module,
                    &symbol.symbol,
                    schema_types,
                    service_type_names,
                    seen,
                )
            })
            .unwrap_or_else(|| {
                contract_projection.schema_for_source_type_ref(projection_index, source_module, ty)
            })
        }
        TypeRefIr::PackageSymbol { symbol } => package_runtime_schema_for_named_type(
            contract_projection,
            projection_index,
            source_module,
            &symbol.symbol_path,
            schema_types,
            service_type_names,
            seen,
        )
        .unwrap_or_else(|| {
            contract_projection.schema_for_source_type_ref(projection_index, source_module, ty)
        }),
        TypeRefIr::Record { fields } => package_runtime_object_schema(
            contract_projection,
            projection_index,
            source_module,
            fields,
            local_type_names,
            schema_types,
            service_type_names,
            seen,
        ),
        TypeRefIr::Union { items } => JsonSchema::one_of(
            items
                .iter()
                .map(|item| {
                    package_runtime_schema_for_type_ref_inner(
                        contract_projection,
                        projection_index,
                        source_module,
                        item,
                        local_type_names,
                        schema_types,
                        service_type_names,
                        seen,
                    )
                })
                .collect(),
        ),
        TypeRefIr::Nullable { inner } => package_runtime_schema_for_type_ref_inner(
            contract_projection,
            projection_index,
            source_module,
            inner,
            local_type_names,
            schema_types,
            service_type_names,
            seen,
        )
        .nullable(),
        TypeRefIr::Literal { value } => package_runtime_literal_schema(value),
        TypeRefIr::AnyInterface { .. } => boundary_rejected_type_schema(),
        TypeRefIr::TypeParam { .. } | TypeRefIr::Function { .. } => JsonSchema::any(),
    }
}

#[allow(clippy::too_many_arguments)]
fn package_runtime_schema_for_builtin(
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    source_module: &str,
    name: &str,
    args: &[TypeRefIr],
    local_type_names: &BTreeMap<u32, String>,
    schema_types: &BTreeMap<String, PackageAbiType>,
    service_type_names: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
) -> JsonSchema {
    let arg_schemas = args
        .iter()
        .map(|arg| {
            package_runtime_schema_for_type_ref_inner(
                contract_projection,
                projection_index,
                source_module,
                arg,
                local_type_names,
                schema_types,
                service_type_names,
                seen,
            )
        })
        .collect::<Vec<_>>();
    match (name, args, arg_schemas.as_slice()) {
        ("string", [], []) => JsonSchema::string(),
        ("integer", [], []) => JsonSchema::integer(),
        ("number", [], []) => JsonSchema::number(),
        ("bool" | "boolean", [], []) => JsonSchema::boolean(),
        ("null" | "void", [], []) => JsonSchema::null(),
        ("Array", [_], [inner]) => JsonSchema::array(inner.clone()),
        ("Map", [_, _], [_, value]) => JsonSchema::map(value.clone()),
        (root, [_], [context]) if is_connect_result_generic_type(root) => {
            gateway_connect_result_schema(context.clone())
        }
        (root, [_], [context]) if is_websocket_connection_generic_type(root) => {
            websocket_connection_schema(context.clone())
        }
        (root, [_], [context]) if is_websocket_receive_event_root(root) => {
            websocket_receive_event_schema(contract_projection.prelude(), context.clone())
        }
        (name, [], []) if is_connection_message_type(name) => {
            connection_message_schema(contract_projection.prelude())
        }
        _ => contract_projection.schema_for_source_type_ref(
            projection_index,
            source_module,
            &TypeRefIr::Native {
                name: name.to_string(),
                args: args.to_vec(),
            },
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn package_runtime_schema_for_named_type(
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    source_module: &str,
    name: &str,
    schema_types: &BTreeMap<String, PackageAbiType>,
    service_type_names: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
) -> Option<JsonSchema> {
    let abi_type = package_abi_type_for_name(schema_types, name)?;
    let symbol = package_service_visible_type_symbol(&abi_type.name, service_type_names);
    if !seen.insert(abi_type.name.clone()) {
        return Some(JsonSchema::reference(&symbol));
    }
    let mut schema = match &abi_type.descriptor {
        PackageAbiTypeDescriptor::Alias { target } => package_runtime_schema_for_type_ref_inner(
            contract_projection,
            projection_index,
            source_module,
            target,
            &abi_type.local_type_names,
            schema_types,
            service_type_names,
            seen,
        )
        .with_x_skiff_alias(package_type_ref_display_text(
            target,
            &abi_type.local_type_names,
            service_type_names,
        )),
        PackageAbiTypeDescriptor::Union { variants } => JsonSchema::one_of(
            variants
                .iter()
                .map(|variant| {
                    package_runtime_schema_for_type_ref_inner(
                        contract_projection,
                        projection_index,
                        source_module,
                        variant,
                        &abi_type.local_type_names,
                        schema_types,
                        service_type_names,
                        seen,
                    )
                })
                .collect(),
        ),
        PackageAbiTypeDescriptor::Record { fields } => package_runtime_object_schema(
            contract_projection,
            projection_index,
            source_module,
            fields,
            &abi_type.local_type_names,
            schema_types,
            service_type_names,
            seen,
        ),
        PackageAbiTypeDescriptor::External => JsonSchema::any(),
    }
    .with_x_skiff_symbol(symbol);
    if let Some(discriminator) = &abi_type.discriminator {
        schema = schema.with_x_skiff_union_discriminator(discriminator.clone());
    }
    seen.remove(&abi_type.name);
    Some(schema)
}

#[allow(clippy::too_many_arguments)]
fn package_runtime_object_schema(
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    source_module: &str,
    fields: &BTreeMap<String, TypeRefIr>,
    local_type_names: &BTreeMap<u32, String>,
    schema_types: &BTreeMap<String, PackageAbiType>,
    service_type_names: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
) -> JsonSchema {
    let mut properties = BTreeMap::new();
    let mut required = Vec::new();
    for (name, ty) in fields {
        let field_schema = package_runtime_schema_for_type_ref_inner(
            contract_projection,
            projection_index,
            source_module,
            ty,
            local_type_names,
            schema_types,
            service_type_names,
            seen,
        );
        if !field_schema.is_nullable() {
            required.push(name.clone());
        }
        properties.insert(name.clone(), field_schema);
    }
    JsonSchema::object(properties, required, false)
}

fn package_runtime_literal_schema(literal: &LiteralIr) -> JsonSchema {
    match literal {
        LiteralIr::Null => JsonSchema::null(),
        LiteralIr::Bool { .. } => JsonSchema::boolean(),
        LiteralIr::Number { .. } => JsonSchema::number(),
        LiteralIr::String { value } => JsonSchema::string_enum(vec![value.clone()]),
    }
}

fn package_abi_type_for_name<'a>(
    schema_types: &'a BTreeMap<String, PackageAbiType>,
    name: &str,
) -> Option<&'a PackageAbiType> {
    schema_types.get(name).or_else(|| {
        name.rsplit_once('.')
            .and_then(|(_, local_name)| schema_types.get(local_name))
    })
}

fn package_service_visible_type_symbol(
    name: &str,
    service_type_names: &BTreeMap<String, String>,
) -> String {
    service_type_names
        .get(name)
        .cloned()
        .or_else(|| {
            name.rsplit_once('.')
                .and_then(|(_, local_name)| service_type_names.get(local_name))
                .cloned()
        })
        .unwrap_or_else(|| name.to_string())
}

fn package_type_ref_display_text(
    ty: &TypeRefIr,
    local_type_names: &BTreeMap<u32, String>,
    service_type_names: &BTreeMap<String, String>,
) -> String {
    let text = type_ref_ir_source_text_with_local_types(ty, &|type_index| {
        local_type_names.get(&type_index).cloned()
    })
    .split_whitespace()
    .collect::<String>();
    service_type_names
        .get(text.as_str())
        .cloned()
        .unwrap_or(text)
}

fn websocket_connection_schema(context: JsonSchema) -> JsonSchema {
    let mut properties = BTreeMap::new();
    properties.insert("id".to_string(), JsonSchema::string());
    properties.insert(
        "businessIdentity".to_string(),
        JsonSchema::string().nullable(),
    );
    properties.insert("context".to_string(), context);
    JsonSchema::object(
        properties,
        vec![
            "id".to_string(),
            "businessIdentity".to_string(),
            "context".to_string(),
        ],
        false,
    )
}

fn websocket_receive_event_schema(prelude: &PreludeProjection, context: JsonSchema) -> JsonSchema {
    let mut properties = BTreeMap::new();
    properties.insert(
        "connection".to_string(),
        websocket_connection_schema(context),
    );
    properties.insert("message".to_string(), connection_message_schema(prelude));
    JsonSchema::object(
        properties,
        vec!["connection".to_string(), "message".to_string()],
        false,
    )
}

fn boundary_rejected_type_schema() -> JsonSchema {
    // AnyInterface must be rejected by boundary validation before package
    // runtime schema emission. This fallback is defensive only and is not a
    // public ABI contract; empty oneOf is intentionally unsatisfiable instead
    // of permissive `any`.
    JsonSchema::one_of(Vec::new())
}

fn is_websocket_connection_generic_type(root: &str) -> bool {
    matches!(
        root,
        "WebSocketConnection" | "std.websocket.WebSocketConnection"
    )
}

fn is_connect_result_generic_type(root: &str) -> bool {
    matches!(
        root,
        "WebSocketConnectResult" | "std.websocket.WebSocketConnectResult"
    )
}

pub fn rewrite_schema_symbols(
    mut schema: JsonSchema,
    symbols: &BTreeMap<String, String>,
) -> JsonSchema {
    if symbols.is_empty() {
        return schema;
    }
    rewrite_schema_tree(&mut schema, symbols);
    schema
}

fn rewrite_symbol(value: &mut Option<String>, symbols: &BTreeMap<String, String>) {
    if let Some(symbol) = value {
        if let Some(rewritten) = symbols.get(symbol.as_str()) {
            *symbol = rewritten.clone();
        }
    }
}

fn rewrite_schema_tree(schema: &mut JsonSchema, symbols: &BTreeMap<String, String>) {
    // Symbol-bearing string keywords ($ref, xSkiffAlias, xSkiffMapKeySymbol,
    // xSkiffSymbol) get renamed in place.
    rewrite_symbol(schema.reference_mut(), symbols);
    rewrite_symbol(schema.x_skiff_alias_mut(), symbols);
    rewrite_symbol(schema.x_skiff_map_key_symbol_mut(), symbols);
    rewrite_symbol(schema.x_skiff_symbol_mut(), symbols);

    // Recurse into every nested schema node.
    for child in schema.child_schemas_mut() {
        rewrite_schema_tree(child, symbols);
    }
}
