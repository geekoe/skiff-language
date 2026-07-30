use std::{collections::BTreeMap, path::Path, path::PathBuf};

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractRequirement, ContractTypeDescriptor,
    ContractTypeNameability, ContractTypeRef, PackageBuildId, PackageLocalAbiIdentity,
    PackageLocalAbiSymbol, PackageRefIr, PackageRequirement, PackageSchemaCanonicalDescriptor,
    PackageSchemaIndex, PackageSchemaIndexEntry, PackageSchemaTypeRecord, PackageTypeRef,
    PackageTypeRequirement, TypeRefIr,
};
use skiff_compiler_compiled::{projection_input::build_projection_input, CompiledPackage};
use skiff_compiler_contract::{
    compile_service_contract_definition, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};
use skiff_compiler_input::{
    CompilerPlatformSources, PublicationApiPublicInstanceEntry, ResolvedContractDependency,
};
use skiff_compiler_projection::package_artifact::{
    project_compiled_package_artifact, PackageArtifactProjectionInput,
};
use skiff_compiler_projection_input::ResolvedPackageSchema;
use skiff_compiler_source::{
    build_package_from_parsed_sources_with_dependency_analysis,
    parsed_sources::parse_publication_sources, prelude_registry::initialize_prelude_registry,
    source_graph::CompilerSourceFile, CompileParsedPackageSourcesInput, PackageCompilePolicy,
    PublicationApiEntry, PublicationApiSpec, SourceDependencyAnalysisInput,
};

#[test]
fn public_instance_exact_signature_reaches_package_local_abi() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = crate_root.parent().unwrap().parent().unwrap();
    initialize_prelude_registry(&CompilerPlatformSources::new(workspace_root).unwrap()).unwrap();
    let (dependency, package_schema) = contract_dependency();
    let requirement = dependency.requirement().clone();
    let dependency_analysis = SourceDependencyAnalysisInput::new(Vec::new(), [dependency]).unwrap();
    let source = CompilerSourceFile::parse(
        PathBuf::from("api.skiff"),
        "api".to_string(),
        true,
        false,
        r#"
            type Local { value: string }
            function localHelper(value: Local) -> Local { return value }
            interface PublicApi {
              function submit(
                self: Self,
                input: payments.User,
                nested: Array<payments.User?>?
              ) -> payments.User
            }
            type Handler implements PublicApi {}
            impl Handler {
              function submit(
                input: payments.User,
                nested: Array<payments.User?>?
              ) -> payments.User {
                return input
              }
            }
            const handler: Handler = Handler {}
        "#
        .to_string(),
        "api.skiff",
    )
    .unwrap();
    let production_sources = vec![source];
    let diagnostic_root = Path::new("/tmp/compiled-public-instance-signature-handoff");
    let parsed_sources = parse_publication_sources(diagnostic_root, &production_sources).unwrap();
    let publication_api = PublicationApiSpec::new(
        vec![
            PublicationApiEntry::for_source("Local", "api", "Local"),
            PublicationApiEntry::for_source("localHelper", "api", "localHelper"),
        ],
        vec![PublicationApiPublicInstanceEntry::for_source(
            "handler",
            "root.api.handler",
            ["root.api.PublicApi"],
        )
        .unwrap()],
        None,
    );
    let package_aliases = BTreeMap::new();
    let model = build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root,
            publication_api: Some(&publication_api),
            package_aliases: &package_aliases,
            package_dependencies: &[],
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new("example.com/public-instance"),
        },
        &dependency_analysis,
    )
    .unwrap();
    let lowered = skiff_compiler_lowering::lower(&model).unwrap();
    let compiled = CompiledPackage::new(model, lowered);
    let execution = compiled
        .file_ir_units()
        .iter()
        .flat_map(|unit| &unit.executables)
        .find(|executable| executable.symbol.ends_with("Handler.submit"))
        .expect("public-instance implementation executable");
    let input_param = execution
        .params
        .iter()
        .find(|param| param.name == "input")
        .expect("implementation input parameter");
    assert_payments_user_file_ir_type(&input_param.ty);
    assert_payments_user_file_ir_type(&execution.return_type);
    let projection = build_projection_input(&compiled).unwrap();
    let projected = project_compiled_package_artifact(PackageArtifactProjectionInput {
        package_id: "example.com/public-instance",
        package_version: "1.0.0",
        projection: projection.view(),
        package_requirements: vec![PackageRequirement {
            alias: "paymentsSchema".to_string(),
            package_id: "example.com/payments-schema".to_string(),
            exact_version: "1.0.0".to_string(),
            expected_local_abi: PackageLocalAbiIdentity::new("payments-schema-abi"),
            expected_package_build: Some(PackageBuildId::new("payments-schema-build")),
        }],
        resolved_package_schemas: std::slice::from_ref(&package_schema),
        contract_requirements: vec![requirement],
        service_requirements: Vec::new(),
        service_call_refs: Vec::new(),
    })
    .unwrap();
    let PackageLocalAbiSymbol::Callable { signature, .. } =
        &projected.artifact.package_local_abi.public_symbols["handler.submit"]
    else {
        panic!("public-instance operation must be projected as a Local ABI callable");
    };

    assert_eq!(
        signature.parameters.len(),
        2,
        "receiver must be trimmed once"
    );
    assert!(matches!(
        &signature.parameters[0].ty,
        PackageTypeRef::PackageSchema { .. }
    ));
    assert!(matches!(
        &signature.parameters[1].ty,
        PackageTypeRef::Nullable { inner }
            if matches!(inner.as_ref(), PackageTypeRef::Container { arguments, .. }
                if matches!(arguments.as_slice(), [PackageTypeRef::Nullable { inner }]
                    if matches!(inner.as_ref(), PackageTypeRef::PackageSchema { .. })))
    ));
    assert!(matches!(
        &signature.return_type,
        PackageTypeRef::PackageSchema { .. }
    ));
    assert_eq!(signature.may_suspend, execution.may_suspend);
    assert!(!signature.may_suspend);

    let PackageLocalAbiSymbol::Callable {
        signature: local_signature,
        ..
    } = &projected.artifact.package_local_abi.public_symbols["localHelper"]
    else {
        panic!("local helper must be projected as a Local ABI callable");
    };
    for ty in [
        &local_signature.parameters[0].ty,
        &local_signature.return_type,
    ] {
        assert!(matches!(
            ty,
            PackageTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                ..
            } if package_id == "example.com/public-instance" && stable_schema_key == "Local"
        ));
    }
}

fn assert_payments_user_file_ir_type(ty: &TypeRefIr) {
    assert!(matches!(
        ty,
        TypeRefIr::PackageSymbol { symbol }
            if matches!(
                &symbol.package,
                PackageRefIr::PackageId { package_id }
                    if package_id == "example.com/payments-schema"
            ) && symbol.symbol_path == "User"
    ));
}

fn contract_dependency() -> (ResolvedContractDependency, ResolvedPackageSchema) {
    let service_id = "example.payments";
    let version = "1.0.0";
    let package_id = "example.com/payments-schema";
    let descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("value".to_string(), ContractTypeRef::builtin("string"))]),
        },
    };
    let user_id =
        skiff_artifact_identity::package_schema_type_id(package_id, "User", &descriptor).unwrap();
    let user = ContractTypeRef::package_schema(package_id, "User", user_id.clone());
    let schema_types = BTreeMap::from([(
        "User".to_string(),
        PackageSchemaIndexEntry {
            package_schema_type_id: user_id.clone(),
            public_path: Some("User".to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        },
    )]);
    let package_schema = ResolvedPackageSchema::new(
        "paymentsSchema".to_string(),
        package_id.to_string(),
        version.to_string(),
        PackageBuildId::new("payments-schema-build"),
        PackageLocalAbiIdentity::new("payments-schema-abi"),
        PackageSchemaIndex {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &schema_types,
            )
            .unwrap(),
            types: schema_types,
        },
        BTreeMap::from([(
            user_id.clone(),
            PackageSchemaTypeRecord {
                package_id: package_id.to_string(),
                stable_schema_key: "User".to_string(),
                package_schema_type_id: user_id.clone(),
                canonical_descriptor: descriptor,
            },
        )]),
    )
    .unwrap();
    let contract = compile_service_contract_definition(ServiceContractDefinition {
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        operations: BTreeMap::from([(
            "ping".to_string(),
            BoundaryOperationContract {
                parameters: vec![skiff_artifact_model::BoundaryParameter {
                    name: "user".to_string(),
                    ty: user.clone(),
                    value_plan: linkable(BoundaryValueOwner::Caller),
                }],
                return_value: BoundaryReturn {
                    ty: user,
                    value_plan: linkable(BoundaryValueOwner::Provider),
                },
                stream: BoundaryStreamContract::Unary,
                callbacks: BoundaryCallbackContract::None,
                effect_guarantee: BoundaryEffectGuarantee {
                    detached_parameters: true,
                    detached_return: true,
                    detached_error: true,
                    no_caller_reachable_mutation: true,
                    no_caller_value_escape: true,
                    no_same_heap_identity: true,
                },
            },
        )]),
        package_type_requirements: vec![PackageTypeRequirement {
            package_id: package_id.to_string(),
            required_type_ids: vec![user_id.clone()],
        }],
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "payments".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::from([(user_id, "User".to_string())]),
        },
    })
    .unwrap();
    let dependency = ResolvedContractDependency::validated(
        ContractRequirement {
            alias: "payments".to_string(),
            service_id: service_id.to_string(),
            contract_version: version.to_string(),
            expected_protocol_identity: contract.service_protocol_identity.clone(),
        },
        contract,
        std::slice::from_ref(&package_schema),
    )
    .unwrap();
    (dependency, package_schema)
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}
