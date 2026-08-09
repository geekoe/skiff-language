use std::collections::BTreeMap;

use skiff_artifact_model::{
    ContractTypeDescriptor, ContractTypeRef, PackageSchemaCanonicalDescriptor,
    PackageSchemaTypeRecord,
};

use super::*;

const SERVICE_ALIAS: &str = "payments";
const PACKAGE_ID: &str = "example.com/payments-schema";
const STABLE_KEY: &str = "User";

fn model_with_selected_schema() -> TypeResolutionModel {
    let mut model = TypeResolutionModel::build(
        &[],
        &BTreeMap::new(),
        &[],
        None,
        None,
        &crate::PublicationTypeSymbolIndex::default(),
    )
    .expect("empty type resolution model should build");
    let schema = PackageSchemaTypeRecord {
        package_id: PACKAGE_ID.to_string(),
        stable_schema_key: STABLE_KEY.to_string(),
        package_schema_type_id: "schema:user".into(),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Alias {
                target: ContractTypeRef::builtin("string"),
            },
        },
    };
    model.service_api_schemas.insert(
        SERVICE_ALIAS.to_string(),
        BTreeMap::from([(STABLE_KEY.to_string(), schema)]),
    );
    model
}

fn service_symbol(module_path: &str, symbol: &str) -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
        },
    }
}

fn exact_schema_type() -> TypeRefIr {
    TypeRefIr::PackageSchema {
        package_id: PACKAGE_ID.to_string(),
        stable_schema_key: STABLE_KEY.to_string(),
        package_schema_type_id: "schema:user".into(),
    }
}

#[test]
fn known_service_schema_symbol_normalizes_to_exact_package_owner() {
    let model = model_with_selected_schema();

    assert_eq!(
        model
            .owner_stable_conformance_type_ref(
                "local.owner",
                &service_symbol("root.payments", STABLE_KEY),
                "interface type argument 0",
            )
            .expect("selected schema should normalize"),
        exact_schema_type()
    );
}

#[test]
fn unknown_and_local_service_symbols_remain_service_owned() {
    let model = model_with_selected_schema();

    for (input, expected) in [
        (
            service_symbol("root.other", "User"),
            service_symbol("other", "User"),
        ),
        (
            service_symbol("root.local.types", "User"),
            service_symbol("local.types", "User"),
        ),
    ] {
        assert_eq!(
            model
                .owner_stable_conformance_type_ref(
                    "local.owner",
                    &input,
                    "interface type argument 0",
                )
                .expect("non-schema service symbol should remain owner-stable"),
            expected
        );
    }
}

#[test]
fn selected_service_alias_with_unknown_schema_fails_closed() {
    let model = model_with_selected_schema();

    assert_eq!(
        model
            .owner_stable_conformance_type_ref(
                "local.owner",
                &service_symbol("root.payments", "Missing"),
                "interface type argument 0",
            )
            .expect_err("invalid selected schema authority must not degrade to a local symbol"),
        Error::ServiceSchemaAuthorityLookup {
            location: "interface type argument 0".to_string(),
            module_path: "payments".to_string(),
            symbol: "Missing".to_string(),
            message: "service dependency `payments` has no public API type `Missing`".to_string(),
        }
    );
}

#[test]
fn service_schema_normalization_recurses_through_container_and_any_interface() {
    let model = model_with_selected_schema();
    let input = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::AnyInterface {
            interface: interface_instantiation_ref(
                service_symbol("local.api", "Reader"),
                vec![TypeRefIr::Nullable {
                    inner: Box::new(service_symbol(SERVICE_ALIAS, STABLE_KEY)),
                }],
            ),
        }],
    };
    let expected = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::AnyInterface {
            interface: interface_instantiation_ref(
                service_symbol("local.api", "Reader"),
                vec![TypeRefIr::Nullable {
                    inner: Box::new(exact_schema_type()),
                }],
            ),
        }],
    };

    let normalized = model
        .owner_stable_conformance_type_ref("local.owner", &input, "interface type argument 0")
        .expect("nested selected schema should normalize");
    assert_eq!(normalized, expected);
}
