use skiff_artifact_model::{
    PackageCallableParameter, PackageCallableSignature, PackageSchemaTypeId, PackageTypeRef,
    ParamModeIr, TypeRefIr,
};

use super::*;

#[test]
fn package_public_path_normalization_has_one_std_scope() {
    assert_eq!(
        canonical_package_public_path(skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID, "io"),
        "std.io"
    );
    assert_eq!(
        canonical_package_public_path(skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID, "std.io"),
        "std.io"
    );
    assert_eq!(canonical_package_public_path("example.com/pkg", "io"), "io");
}

#[test]
fn duplicate_callable_signature_key_is_rejected() {
    let key = ProjectionPackageCallableKey::new("run", "api", 0);
    let signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: Vec::new(),
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        },
        may_suspend: false,
    };
    let error = ProjectionPackageCallableSignatureFacts::try_from_entries([
        (key.clone(), signature.clone()),
        (key.clone(), signature),
    ])
    .expect_err("duplicate key must fail closed");
    assert_eq!(error.key(), &key);
}

#[test]
fn callable_signature_wire_contains_only_open_error_surface() {
    let signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: Vec::new(),
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("void"),
        },
        may_suspend: false,
    };
    let wire = serde_json::to_value(&signature).unwrap();

    assert_eq!(
        wire.as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["typeParams", "parameters", "returnType", "maySuspend"]
    );
    assert!(wire.get("throwTypes").is_none());

    let mut legacy = wire;
    legacy["throwTypes"] = serde_json::json!([]);
    assert!(serde_json::from_value::<PackageCallableSignature>(legacy).is_err());
}

#[test]
fn projection_input_preserves_exact_nested_package_schema_signature() {
    let key = ProjectionPackageCallableKey::new("submit", "api", 4);
    let contract = PackageTypeRef::PackageSchema {
        package_id: "example.com/models".to_string(),
        stable_schema_key: "api.User".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("package-type:models:User"),
    };
    let signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: "users".to_string(),
            ty: PackageTypeRef::Nullable {
                inner: Box::new(PackageTypeRef::Container {
                    name: "Array".to_string(),
                    arguments: vec![contract.clone()],
                }),
            },
            mode: ParamModeIr::Value,
        }],
        return_type: contract,
        may_suspend: true,
    };
    let facts = ProjectionPackageCallableSignatureFacts::try_from_entries([(
        key.clone(),
        signature.clone(),
    )])
    .unwrap();
    let input = crate::ProjectionInput::new(
        Vec::new(),
        Vec::new(),
        crate::ProjectionSourceFacts::default(),
        crate::ProjectionLoweringFacts::default(),
        facts,
    );

    assert_eq!(
        input.view().callable_signatures().signature(&key),
        Some(&signature)
    );
}
