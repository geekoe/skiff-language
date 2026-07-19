use std::{collections::BTreeMap, path::Path, path::PathBuf};

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryReturn, BoundaryStreamContract,
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
    BoundaryValuePlan, ContractRequirement, ContractTypeDescriptor, ContractTypeNameability,
    ContractTypeRef, ContractTypeShape, PackageLocalAbiSymbol, PackageTypeRef,
};
use skiff_compiler_compiled::{projection_input::build_projection_input, CompiledPackage};
use skiff_compiler_contract::{
    compile_service_contract_definition, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};
use skiff_compiler_input::{PublicationApiPublicInstanceEntry, ResolvedContractDependency};
use skiff_compiler_projection::package_artifact::{
    project_compiled_package_artifact, PackageArtifactProjectionInput,
};
use skiff_compiler_source::{
    build_package_from_parsed_sources_with_dependency_analysis,
    parsed_sources::parse_publication_sources, source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageCompilePolicy, PublicationApiEntry,
    PublicationApiSpec, SourceDependencyAnalysisInput,
};

#[test]
fn public_instance_exact_signature_reaches_package_local_abi() {
    let dependency = contract_dependency();
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
            policy: PackageCompilePolicy::new("example.com/public-instance"),
        },
        &dependency_analysis,
    )
    .unwrap();
    let lowered = skiff_compiler_lowering::lower(&model).unwrap();
    let compiled = CompiledPackage::new(model, lowered);
    let projection = build_projection_input(&compiled).unwrap();
    let projected = project_compiled_package_artifact(PackageArtifactProjectionInput {
        package_id: "example.com/public-instance",
        package_version: "1.0.0",
        projection: projection.view(),
        package_requirements: Vec::new(),
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
        PackageTypeRef::Contract { .. }
    ));
    assert!(matches!(
        &signature.parameters[1].ty,
        PackageTypeRef::Nullable { inner }
            if matches!(inner.as_ref(), PackageTypeRef::Container { arguments, .. }
                if matches!(arguments.as_slice(), [PackageTypeRef::Nullable { inner }]
                    if matches!(inner.as_ref(), PackageTypeRef::Contract { .. })))
    ));
    assert!(matches!(
        &signature.return_type,
        PackageTypeRef::Contract { .. }
    ));
    assert!(!signature.may_suspend);

    let PackageLocalAbiSymbol::Callable {
        signature: local_signature,
        ..
    } = &projected.artifact.package_local_abi.public_symbols["localHelper"]
    else {
        panic!("local helper must be projected as a Local ABI callable");
    };
    assert!(matches!(
        &local_signature.parameters[0].ty,
        PackageTypeRef::Local { .. }
    ));
    assert!(matches!(
        &local_signature.return_type,
        PackageTypeRef::Local { .. }
    ));
}

fn contract_dependency() -> ResolvedContractDependency {
    let service_id = "example.payments";
    let version = "1.0.0";
    let contract = compile_service_contract_definition(ServiceContractDefinition {
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        operations: BTreeMap::from([(
            "ping".to_string(),
            BoundaryOperationContract {
                parameters: Vec::new(),
                return_value: BoundaryReturn {
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: linkable(BoundaryValueOwner::Provider),
                },
                errors: BoundaryErrorContract::None,
                stream: BoundaryStreamContract::Unary,
                cancellation: BoundaryCancellationContract::NotCancellable,
                callbacks: BoundaryCallbackContract::None,
                may_suspend: false,
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
        boundary_schema: BTreeMap::from([(
            "User".to_string(),
            ContractTypeShape {
                nameability: ContractTypeNameability::PublicNameable,
                descriptor: ContractTypeDescriptor::Record {
                    fields: BTreeMap::from([(
                        "value".to_string(),
                        ContractTypeRef::builtin("string"),
                    )]),
                },
            },
        )]),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "payments".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    })
    .unwrap();
    ResolvedContractDependency::validated(
        ContractRequirement {
            alias: "payments".to_string(),
            service_id: service_id.to_string(),
            contract_version: version.to_string(),
            expected_protocol_identity: contract.service_protocol_identity.clone(),
        },
        contract,
    )
    .unwrap()
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}
