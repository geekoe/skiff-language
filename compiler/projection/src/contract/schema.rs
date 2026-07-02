use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use crate::typed_artifacts::PublicInstanceExport;

use super::{
    ContractAliasProjection, ContractInterfaceOperationProjection, ContractInterfaceProjection,
    ContractLiteralKey, ContractPackageRefKey, ContractProjection,
    ContractTypeDescriptorProjection, ContractTypeKey, ContractTypeProjection,
};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(transparent)]
pub struct CanonicalContractProjectionSchema {
    value: Value,
}

impl CanonicalContractProjectionSchema {
    pub fn canonical_json(&self) -> String {
        serde_json::to_string(self).expect("canonical contract projection schema must serialize")
    }
}

pub fn canonical_contract_projection_schema(
    projection: &ContractProjection,
) -> CanonicalContractProjectionSchema {
    canonical_contract_projection_schema_with_public_instances(projection, &[])
}

pub fn canonical_contract_projection_schema_with_public_instances(
    projection: &ContractProjection,
    public_instances: &[PublicInstanceExport],
) -> CanonicalContractProjectionSchema {
    CanonicalContractProjectionSchema {
        value: canonical_contract_projection_schema_value(projection, public_instances),
    }
}

fn canonical_contract_projection_schema_value(
    projection: &ContractProjection,
    public_instances: &[PublicInstanceExport],
) -> Value {
    json!({
        "schemaVersion": "skiff-contract-canonical-v1",
        "standard_librarySchemaIdentity": projection.prelude().schema_identity(),
        "types": canonical_projection_types(projection),
        "interfaces": canonical_projection_interfaces(projection),
        "publicInstances": canonical_public_instances(public_instances),
    })
}

fn canonical_public_instances(public_instances: &[PublicInstanceExport]) -> Value {
    serde_json::to_value(public_instances)
        .expect("typed public instance metadata must serialize into canonical schema")
}

fn canonical_projection_types(projection: &ContractProjection) -> BTreeMap<String, Value> {
    projection
        .types
        .iter()
        .map(|(public_name, ty)| {
            (
                public_name.clone(),
                canonical_type_projection_decl_value(ty),
            )
        })
        .collect::<BTreeMap<_, _>>()
}

fn canonical_type_projection_decl_value(ty: &ContractTypeProjection) -> Value {
    match &ty.descriptor {
        ContractTypeDescriptorProjection::Record { fields } => json!({
            "alias": Value::Null,
            "discriminator": ty.discriminator,
            "fields": canonical_type_key_fields_value(fields),
        }),
        ContractTypeDescriptorProjection::Union { variants } => json!({
            "alias": canonical_contract_type_key_value(&ContractTypeKey::Union {
                items: variants.clone(),
            }),
            "discriminator": ty.discriminator,
            "fields": BTreeMap::<String, Value>::new(),
        }),
        ContractTypeDescriptorProjection::Native { symbol } => json!({
            "alias": {
                "name": symbol,
            },
            "discriminator": ty.discriminator,
            "fields": BTreeMap::<String, Value>::new(),
        }),
    }
}

fn canonical_alias_projection_decl_value(alias: &ContractAliasProjection) -> Value {
    json!({
        "alias": canonical_contract_type_key_value(&alias.target),
        "discriminator": Value::Null,
        "fields": BTreeMap::<String, Value>::new(),
    })
}

fn canonical_projection_interfaces(projection: &ContractProjection) -> BTreeMap<String, Value> {
    projection
        .interfaces
        .iter()
        .map(|(public_name, interface)| {
            (
                public_name.clone(),
                canonical_interface_projection_value(interface),
            )
        })
        .collect()
}

fn canonical_interface_projection_value(interface: &ContractInterfaceProjection) -> Value {
    let operations = interface
        .operations
        .iter()
        .map(|operation| {
            (
                operation.name.clone(),
                canonical_interface_operation_value(operation),
            )
        })
        .collect::<BTreeMap<_, _>>();

    json!({
        "operations": operations,
    })
}

fn canonical_interface_operation_value(operation: &ContractInterfaceOperationProjection) -> Value {
    let params = operation
        .params
        .iter()
        .map(|param| {
            json!({
                "name": param.name,
                "type": canonical_contract_type_key_value(&param.ty),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "params": params,
        "returnType": canonical_contract_type_key_value(&operation.return_type),
    })
}

fn canonical_contract_type_key_value(key: &ContractTypeKey) -> Value {
    match key {
        ContractTypeKey::Builtin { name, args } if args.is_empty() => json!({
            "name": name,
        }),
        ContractTypeKey::Builtin { name, args } => canonical_type_key_as_generic(name, args),
        ContractTypeKey::Named(name) => json!({
            "name": name.canonical_symbol_ref(),
        }),
        ContractTypeKey::PackageSymbol {
            package,
            symbol_path,
            abi_expectation,
        } => json!({
            "packageSymbol": {
                "package": canonical_package_ref_value(package),
                "symbolPath": symbol_path,
                "abiExpectation": abi_expectation,
            },
        }),
        ContractTypeKey::AnyInterface {
            interface,
            canonical_type_args,
        } => json!({
            "anyInterface": {
                "interface": canonical_contract_type_key_value(interface),
                "canonicalTypeArgs": canonical_type_args
                    .iter()
                    .map(canonical_contract_type_key_value)
                    .collect::<Vec<_>>(),
            },
        }),
        ContractTypeKey::DbObjectSymbol {
            module_path,
            symbol,
        } => json!({
            "dbObjectSymbol": {
                "modulePath": module_path,
                "symbol": symbol,
            },
        }),
        ContractTypeKey::Record { fields } => json!({
            "record": canonical_type_key_fields_value(fields),
        }),
        ContractTypeKey::Union { items } => canonical_union_value(items),
        ContractTypeKey::Nullable { inner } => json!({
            "nullable": canonical_contract_type_key_value(inner),
        }),
        ContractTypeKey::Literal(literal) => canonical_literal_value(literal),
        ContractTypeKey::TypeParam { name } => json!({
            "typeParam": name,
        }),
        ContractTypeKey::Function {
            params,
            return_type,
        } => {
            let params = params
                .iter()
                .map(|param| {
                    json!({
                        "name": param.name,
                        "type": canonical_contract_type_key_value(&param.ty),
                    })
                })
                .collect::<Vec<_>>();

            json!({
                "function": {
                    "params": params,
                    "returnType": canonical_contract_type_key_value(return_type),
                },
            })
        }
    }
}

fn canonical_type_key_fields_value(
    fields: &BTreeMap<String, ContractTypeKey>,
) -> BTreeMap<String, Value> {
    fields
        .iter()
        .map(|(name, ty)| (name.clone(), canonical_contract_type_key_value(ty)))
        .collect()
}

fn canonical_type_key_as_generic(root: &str, args: &[ContractTypeKey]) -> Value {
    json!({
        "generic": {
            "root": root,
            "args": args
                .iter()
                .map(canonical_contract_type_key_value)
                .collect::<Vec<_>>(),
        },
    })
}

fn canonical_union_value(items: &[ContractTypeKey]) -> Value {
    let mut variants = items
        .iter()
        .map(canonical_contract_type_key_value)
        .collect::<Vec<_>>();
    variants.sort_by_key(canonical_type_sort_key);

    let mut seen = BTreeSet::new();
    variants.retain(|variant| seen.insert(canonical_type_sort_key(variant)));

    match variants.as_slice() {
        [variant] => variant.clone(),
        _ => json!({
            "union": variants,
        }),
    }
}

fn canonical_literal_value(literal: &ContractLiteralKey) -> Value {
    match literal {
        ContractLiteralKey::Null => json!({
            "literal": Value::Null,
        }),
        ContractLiteralKey::Bool(value) => json!({
            "literal": value,
        }),
        ContractLiteralKey::Number(value) => json!({
            "literal": {
                "number": value,
            },
        }),
        ContractLiteralKey::String(value) => json!({
            "literal": value,
        }),
    }
}

fn canonical_package_ref_value(package: &ContractPackageRefKey) -> Value {
    match package {
        ContractPackageRefKey::PackageId { package_id } => json!({
            "packageId": package_id,
        }),
        ContractPackageRefKey::Dependency { dependency_ref } => json!({
            "dependency": dependency_ref,
        }),
    }
}

fn canonical_type_sort_key(value: &Value) -> String {
    serde_json::to_string(value).expect("canonical type value must be serializable")
}
