use skiff_artifact_model::{ContractTypeRef, PackageRefIr, PackageTypeRef, TypeRefIr};

use super::type_projection::{package_type_assignable, resolved_contract_type};

#[test]
fn package_schema_assignability_requires_exact_owner_key_and_identity() {
    let exact = package_type("example.types", "Payload", "type:payload");
    assert!(package_type_assignable(&exact, &exact));
    assert!(!package_type_assignable(
        &exact,
        &package_type("other.types", "Payload", "type:payload")
    ));
    assert!(!package_type_assignable(
        &exact,
        &package_type("example.types", "Renamed", "type:payload")
    ));
    assert!(!package_type_assignable(
        &exact,
        &package_type("example.types", "Payload", "type:other")
    ));
}

#[test]
fn contract_ref_materializes_as_owner_package_symbol_not_service_symbol() {
    let resolved = resolved_contract_type(
        &ContractTypeRef::package_schema("example.types", "Payload", "type:payload".into()),
        "renamedService",
    )
    .unwrap();
    assert_eq!(resolved.source_text, "renamedService.Payload");
    assert!(matches!(
        resolved.ir,
        TypeRefIr::PackageSymbol { symbol }
            if symbol.package == PackageRefIr::PackageId {
                package_id: "example.types".to_string()
            } && symbol.symbol_path == "Payload"
    ));
}

#[test]
fn stream_and_container_refs_preserve_package_schema_identity() {
    let nested = PackageTypeRef::Container {
        name: "Stream".to_string(),
        arguments: vec![package_type("example.types", "Event", "type:event")],
    };
    assert!(package_type_assignable(&nested, &nested));
}

fn package_type(package_id: &str, stable_key: &str, type_id: &str) -> PackageTypeRef {
    PackageTypeRef::PackageSchema {
        package_id: package_id.to_string(),
        stable_schema_key: stable_key.to_string(),
        package_schema_type_id: type_id.into(),
    }
}
