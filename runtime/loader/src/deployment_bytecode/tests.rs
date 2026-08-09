use super::*;

use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, BytecodeArtifact, BytecodeConstantRef,
    BytecodeFunctionOrigin, BytecodeImage, BytecodePoolEntry, BytecodePools, CallableEffectSummary,
    CallableProvenanceSummary, CallableProvenanceUnknownReason, CallableSemanticFacts,
    ContractDiagnosticText, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentRevision, FileIrRef, FrameLayout, FrozenConstantGraph, FrozenConstantNode, LiteralIr,
    OperationCallableKind, OperationTargetRef, PackageCallableId, PackageCallableLinkFact,
    PackageCallableParameter, PackageCallableSignature, PackageExecutableCoordinate,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef, PackageTypeRef,
    ParameterSlotDecl, RelocatableBytecodeFunction, ServiceProtocolIdentity,
    ServiceSelectorBinding, StatementChargeKind, StatementEntry, TypeRefIr, ValueDropPlan,
    ValueTransferPlan, BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
    PACKAGE_ARTIFACT_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};

fn admitted_bytecode(seed: &str) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_table_fingerprint(),
        native_value_lifecycle_registry:
            skiff_artifact_model::native_value_lifecycle_registry_identity().clone(),
        bytecode_identity: "unassigned".to_string(),
        image: BytecodeImage {
            functions: BTreeMap::new(),
            pools: BytecodePools {
                constants: vec![BytecodePoolEntry::ConstantRef {
                    reference: BytecodeConstantRef::LocalNode { node_index: 0 },
                    type_ref: 0,
                    plan: ValueTransferPlan::SnapshotShare {
                        drop: ValueDropPlan::Trivial,
                    },
                }],
                types: vec![BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::builtin("string"),
                }],
                shapes: Vec::new(),
                effects: Vec::new(),
                resume: Vec::new(),
                callback_capture: Vec::new(),
                writable_paths: Vec::new(),
            },
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph {
                nodes: vec![FrozenConstantNode::Literal {
                    literal: LiteralIr::String {
                        value: seed.to_string(),
                    },
                }],
            },
            debug_table: None,
        },
    };
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

fn package_artifact(
    package_id: &str,
    build_id: &str,
    bytecode: Option<BytecodeArtifactRef>,
) -> Arc<PackageArtifact> {
    Arc::new(PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new(build_id),
        files: Vec::new(),
        static_resources: Vec::new(),
        bytecode,
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new(format!("abi:{package_id}")),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: PackageSchemaIndexIdentity::new(format!(
                "schema:{package_id}"
            )),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks {
            types: BTreeMap::new(),
            constants: BTreeMap::new(),
            functions: BTreeMap::new(),
            impl_methods: BTreeMap::new(),
            operation_targets: BTreeMap::new(),
        },
        callable_links: BTreeMap::new(),
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    })
}

fn package_reference(artifact: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    }
}

fn contract_reference(service_id: &str) -> ServiceContractRef {
    ServiceContractRef {
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new(format!("protocol:{service_id}")),
    }
}

fn contract(reference: &ServiceContractRef) -> Arc<ServiceContract> {
    Arc::new(ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: reference.service_id.clone(),
        contract_version: reference.contract_version.clone(),
        service_protocol_identity: reference.service_protocol_identity.clone(),
        operations: BTreeMap::new(),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: reference.service_id.clone(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    })
}

fn deployment(
    implementation: PackageArtifactRef,
    contract: ServiceContractRef,
    service_selectors: Vec<ServiceSelectorBinding>,
) -> Arc<ServiceDeployment> {
    Arc::new(ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new("revision:consumer"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("deployment:consumer"),
        implementation,
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors,
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "consumer".to_string(),
            notes: BTreeMap::new(),
        },
    })
}

fn deployment_reference(deployment: &ServiceDeployment) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    }
}

fn hydrated_package(
    package_id: &str,
    build_id: &str,
    bytecode: &Arc<ValidatedBytecodeArtifact>,
) -> HydratedBytecodePackage {
    let artifact = package_artifact(package_id, build_id, Some(bytecode.reference().clone()));
    HydratedBytecodePackage::checked(package_reference(&artifact), artifact, Arc::clone(bytecode))
        .unwrap()
}

fn callable_bytecode(
    self_bound: bool,
) -> (
    Arc<ValidatedBytecodeArtifact>,
    PackageExecutableCoordinate,
    PackageCallableId,
) {
    let coordinate = PackageExecutableCoordinate {
        file_ir_identity: "file-ir:manifest".to_string(),
        module_path: "manifest".to_string(),
        executable_index: 0,
    };
    let callable = PackageCallableId::new("callable:manifest:run");
    let mut artifact = admitted_bytecode("manifest").artifact().clone();
    if self_bound {
        artifact.image.pools.types.push(BytecodePoolEntry::TypeRef {
            ty: TypeRefIr::builtin("string"),
        });
    }
    let plan = ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::Trivial,
    };
    artifact.image.functions.insert(
        "manifest::run".to_string(),
        RelocatableBytecodeFunction {
            function_key: "manifest::run".to_string(),
            origin: BytecodeFunctionOrigin::Executable {
                executable: coordinate.clone(),
            },
            type_parameters: Vec::new(),
            self_type_ref: self_bound.then_some(0),
            words: vec![0x14, 0x25],
            relocations: Vec::new(),
            call_loan_layouts: Vec::new(),
            frame_layout: FrameLayout {
                slot_count: if self_bound { 1 } else { 0 },
                slot_type_refs: self_bound.then_some(0).into_iter().collect(),
                parameter_slots: self_bound
                    .then(|| ParameterSlotDecl {
                        slot: 0,
                        mode: skiff_artifact_model::ParamModeIr::Value,
                        plan: plan.clone(),
                    })
                    .into_iter()
                    .collect(),
                writable_local_slots: Vec::new(),
                result_count: 0,
                result_type_refs: Vec::new(),
                result_plans: Vec::new(),
                slot_plans: self_bound.then_some(plan).into_iter().collect(),
            },
            max_operand_depth: 0,
            effect_summary_ref: callable.clone(),
            exception_regions: Vec::new(),
            active_regions: Vec::new(),
            switch_tables: Vec::new(),
            statement_entries: vec![StatementEntry {
                pc: 0,
                statement_id: "manifest:entry".to_string(),
                charge_kind: StatementChargeKind::FunctionEntry,
            }],
            source_map: Vec::new(),
        },
    );
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    (
        Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap()),
        coordinate,
        callable,
    )
}

fn callable_package(
    bytecode: &Arc<ValidatedBytecodeArtifact>,
    coordinate: &PackageExecutableCoordinate,
    callable: &PackageCallableId,
    kind: OperationCallableKind,
) -> Arc<PackageArtifact> {
    let mut artifact = package_artifact(
        "example.manifest",
        "build:manifest",
        Some(bytecode.reference().clone()),
    )
    .as_ref()
    .clone();
    let file = FileIrRef::new(
        coordinate.file_ir_identity.clone(),
        coordinate.module_path.clone(),
    );
    artifact.files = vec![file.clone()];
    artifact.callable_links.insert(
        callable.clone(),
        PackageCallableLinkFact {
            callable_id: callable.clone(),
            target: OperationTargetRef {
                file_ref: file,
                executable_index: coordinate.executable_index,
                callable_abi_id: callable.as_str().to_string(),
                callable_kind: kind,
            },
        },
    );
    artifact.package_local_abi.implementation_symbols.insert(
        "manifest.run".to_string(),
        callable_abi_symbol(callable, kind),
    );
    artifact
        .callable_semantic_facts
        .insert(callable.clone(), callable_semantic_facts());
    Arc::new(artifact)
}

fn add_callable_alias(
    artifact: &mut PackageArtifact,
    canonical: &PackageCallableId,
    alias: &PackageCallableId,
    symbol_path: &str,
    kind: OperationCallableKind,
    implementation_surface: bool,
) {
    let mut link = artifact.callable_links.get(canonical).unwrap().clone();
    link.callable_id = alias.clone();
    link.target.callable_abi_id = alias.as_str().to_string();
    link.target.callable_kind = kind;
    artifact.callable_links.insert(alias.clone(), link);
    let symbols = if implementation_surface {
        &mut artifact.package_local_abi.implementation_symbols
    } else {
        &mut artifact.package_local_abi.public_symbols
    };
    symbols.insert(symbol_path.to_string(), callable_abi_symbol(alias, kind));
    artifact
        .callable_semantic_facts
        .insert(alias.clone(), callable_semantic_facts());
}

fn callable_abi_symbol(
    callable: &PackageCallableId,
    kind: OperationCallableKind,
) -> PackageLocalAbiSymbol {
    PackageLocalAbiSymbol::Callable {
        callable_id: callable.clone(),
        signature: PackageCallableSignature {
            type_params: Vec::new(),
            parameters: self_bound_parameters(kind),
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("void"),
            },
            may_suspend: false,
        },
    }
}

fn callable_semantic_facts() -> CallableSemanticFacts {
    CallableSemanticFacts {
        effects: CallableEffectSummary::analysis_pending(),
        provenance: CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::AnalysisPending,
        },
        resolved_call_targets: BTreeMap::new(),
    }
}

fn with_effect_summary_ref(
    bytecode: &ValidatedBytecodeArtifact,
    callable: &PackageCallableId,
) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = bytecode.artifact().clone();
    artifact
        .image
        .functions
        .get_mut("manifest::run")
        .unwrap()
        .effect_summary_ref = callable.clone();
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

fn with_synthetic_callback(
    bytecode: &ValidatedBytecodeArtifact,
    owner: &PackageExecutableCoordinate,
) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = bytecode.artifact().clone();
    let mut callback = artifact.image.functions["manifest::run"].clone();
    callback.function_key = "manifest::run$callback0".to_string();
    callback.origin = BytecodeFunctionOrigin::SyntheticCallback {
        owner: owner.clone(),
        site_ordinal: 0,
    };
    artifact
        .image
        .functions
        .insert(callback.function_key.clone(), callback);
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

fn self_bound_parameters(kind: OperationCallableKind) -> Vec<PackageCallableParameter> {
    matches!(
        kind,
        OperationCallableKind::ReceiverMethod | OperationCallableKind::ImplMethod
    )
    .then(|| PackageCallableParameter {
        name: "self".to_string(),
        ty: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        },
        mode: skiff_artifact_model::ParamModeIr::Value,
    })
    .into_iter()
    .collect()
}

#[test]
fn package_checked_constructor_admits_only_exact_token_and_exposes_opaque_getters() {
    let bytecode = admitted_bytecode("exact");
    let artifact = package_artifact(
        "example.package",
        "build:package",
        Some(bytecode.reference().clone()),
    );
    let reference = package_reference(&artifact);
    let hydrated = HydratedBytecodePackage::checked(
        reference.clone(),
        Arc::clone(&artifact),
        Arc::clone(&bytecode),
    )
    .unwrap();

    assert_eq!(hydrated.reference(), &reference);
    assert!(Arc::ptr_eq(hydrated.artifact(), &artifact));
    assert!(Arc::ptr_eq(hydrated.bytecode(), &bytecode));
}

#[test]
fn package_checked_constructor_joins_v4_callable_origin_and_self_manifests() {
    let (bytecode, coordinate, callable) = callable_bytecode(true);
    let artifact = callable_package(
        &bytecode,
        &coordinate,
        &callable,
        OperationCallableKind::ImplMethod,
    );
    let hydrated =
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode).unwrap();

    assert_eq!(
        hydrated.function_key_for_executable(&coordinate),
        Some("manifest::run")
    );
    assert_eq!(
        hydrated.function_key_for_callable(&callable),
        Some("manifest::run")
    );
    assert_eq!(
        hydrated.canonical_implementation_callable_for_executable(&coordinate),
        Some(&callable)
    );
    assert_eq!(
        hydrated.canonical_implementation_callable_for_function_key("manifest::run"),
        Some(&callable)
    );
    assert_eq!(
        hydrated.function_key_for_canonical_implementation_callable(&callable),
        Some("manifest::run")
    );
}

#[test]
fn canonical_callable_index_ignores_public_aliases_that_share_one_origin() {
    let (bytecode, coordinate, canonical) = callable_bytecode(true);
    let mut artifact = callable_package(
        &bytecode,
        &coordinate,
        &canonical,
        OperationCallableKind::ImplMethod,
    )
    .as_ref()
    .clone();
    let public_alias = PackageCallableId::new("callable:public:manifest:run");
    add_callable_alias(
        &mut artifact,
        &canonical,
        &public_alias,
        "public.run",
        OperationCallableKind::ImplMethod,
        false,
    );
    let artifact = Arc::new(artifact);

    let hydrated =
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode).unwrap();

    assert_eq!(
        hydrated.function_key_for_callable(&public_alias),
        Some("manifest::run")
    );
    assert_eq!(
        hydrated.canonical_implementation_callable_for_executable(&coordinate),
        Some(&canonical)
    );
    assert_eq!(
        hydrated.canonical_implementation_callable_for_function_key("manifest::run"),
        Some(&canonical)
    );
    assert_eq!(
        hydrated.function_key_for_canonical_implementation_callable(&public_alias),
        None
    );
}

#[test]
fn canonical_callable_index_ignores_public_function_aliases() {
    let (bytecode, coordinate, canonical) = callable_bytecode(false);
    let mut artifact = callable_package(
        &bytecode,
        &coordinate,
        &canonical,
        OperationCallableKind::InternalFunction,
    )
    .as_ref()
    .clone();
    let public_alias = PackageCallableId::new("callable:public:manifest:run");
    add_callable_alias(
        &mut artifact,
        &canonical,
        &public_alias,
        "public.run",
        OperationCallableKind::PublicFunction,
        false,
    );
    let artifact = Arc::new(artifact);

    let hydrated =
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode).unwrap();

    assert_eq!(
        hydrated.function_key_for_callable(&public_alias),
        Some("manifest::run")
    );
    assert_eq!(
        hydrated.canonical_implementation_callable_for_executable(&coordinate),
        Some(&canonical)
    );
    assert_eq!(
        hydrated.function_key_for_canonical_implementation_callable(&public_alias),
        None
    );
}

#[test]
fn canonical_callable_index_rejects_public_effect_summary_drift() {
    let (bytecode, coordinate, canonical) = callable_bytecode(true);
    let public_alias = PackageCallableId::new("callable:public:manifest:run");
    let drifted_bytecode = with_effect_summary_ref(&bytecode, &public_alias);
    let mut artifact = callable_package(
        &drifted_bytecode,
        &coordinate,
        &canonical,
        OperationCallableKind::ImplMethod,
    )
    .as_ref()
    .clone();
    add_callable_alias(
        &mut artifact,
        &canonical,
        &public_alias,
        "public.run",
        OperationCallableKind::ImplMethod,
        false,
    );
    let artifact = Arc::new(artifact);

    assert!(matches!(
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, drifted_bytecode),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::Callable,
            ..
        })
    ));
}

#[test]
fn canonical_callable_index_rejects_ambiguous_implementation_owners() {
    let (bytecode, coordinate, canonical) = callable_bytecode(true);
    let mut artifact = callable_package(
        &bytecode,
        &coordinate,
        &canonical,
        OperationCallableKind::ImplMethod,
    )
    .as_ref()
    .clone();
    let duplicate = PackageCallableId::new("callable:implementation:manifest:duplicate");
    add_callable_alias(
        &mut artifact,
        &canonical,
        &duplicate,
        "manifest.duplicate",
        OperationCallableKind::ImplMethod,
        true,
    );
    let artifact = Arc::new(artifact);

    assert!(matches!(
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::Callable,
            ..
        })
    ));
}

#[test]
fn canonical_callable_index_rejects_missing_implementation_owner() {
    let (bytecode, coordinate, canonical) = callable_bytecode(true);
    let mut artifact = callable_package(
        &bytecode,
        &coordinate,
        &canonical,
        OperationCallableKind::ImplMethod,
    )
    .as_ref()
    .clone();
    let symbol = artifact
        .package_local_abi
        .implementation_symbols
        .remove("manifest.run")
        .unwrap();
    artifact
        .package_local_abi
        .public_symbols
        .insert("public.run".to_string(), symbol);
    let artifact = Arc::new(artifact);

    assert!(matches!(
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::Callable,
            ..
        })
    ));
}

#[test]
fn canonical_callable_index_rejects_wrong_coordinate_owner() {
    let (bytecode, coordinate, canonical) = callable_bytecode(true);
    let mut artifact = callable_package(
        &bytecode,
        &coordinate,
        &canonical,
        OperationCallableKind::ImplMethod,
    )
    .as_ref()
    .clone();
    artifact
        .callable_links
        .get_mut(&canonical)
        .unwrap()
        .target
        .executable_index += 1;
    let artifact = Arc::new(artifact);

    assert!(matches!(
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::Callable,
            ..
        })
    ));
}

#[test]
fn canonical_callable_index_rejects_non_implementation_target_kind() {
    let (bytecode, coordinate, canonical) = callable_bytecode(false);
    let artifact = callable_package(
        &bytecode,
        &coordinate,
        &canonical,
        OperationCallableKind::PublicFunction,
    );

    assert!(matches!(
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::Callable,
            ..
        })
    ));
}

#[test]
fn canonical_callable_index_rejects_synthetic_callbacks_without_owner_manifest() {
    let (bytecode, coordinate, canonical) = callable_bytecode(false);
    let bytecode = with_synthetic_callback(&bytecode, &coordinate);
    let artifact = callable_package(
        &bytecode,
        &coordinate,
        &canonical,
        OperationCallableKind::InternalFunction,
    );

    assert!(matches!(
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::FunctionOrigin,
            detail,
            ..
        }) if detail.contains("no independent package-owned callback manifest")
    ));
}

#[test]
fn package_checked_constructor_rejects_path_free_manifest_gaps_fail_closed() {
    let (bytecode, coordinate, callable) = callable_bytecode(true);
    let mut missing_owner = callable_package(
        &bytecode,
        &coordinate,
        &callable,
        OperationCallableKind::ImplMethod,
    )
    .as_ref()
    .clone();
    missing_owner.files.clear();
    let missing_owner = Arc::new(missing_owner);
    assert!(matches!(
        HydratedBytecodePackage::checked(
            package_reference(&missing_owner),
            missing_owner,
            Arc::clone(&bytecode)
        ),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::FunctionOrigin,
            ..
        })
    ));

    let wrong_self = callable_package(
        &bytecode,
        &coordinate,
        &callable,
        OperationCallableKind::PublicFunction,
    );
    assert!(matches!(
        HydratedBytecodePackage::checked(
            package_reference(&wrong_self),
            wrong_self,
            Arc::clone(&bytecode)
        ),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::SelfType,
            ..
        })
    ));

    let mut missing_facts = callable_package(
        &bytecode,
        &coordinate,
        &callable,
        OperationCallableKind::ImplMethod,
    )
    .as_ref()
    .clone();
    missing_facts.callable_semantic_facts.clear();
    let missing_facts = Arc::new(missing_facts);
    assert!(matches!(
        HydratedBytecodePackage::checked(
            package_reference(&missing_facts),
            missing_facts,
            bytecode
        ),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::Callable,
            ..
        })
    ));
}

#[test]
fn package_checked_constructor_classifies_missing_and_mismatched_bytecode() {
    let bytecode = admitted_bytecode("expected");
    let missing = package_artifact("example.missing", "build:missing", None);
    let missing_reference = package_reference(&missing);
    assert!(matches!(
        HydratedBytecodePackage::checked(
            missing_reference.clone(),
            missing,
            Arc::clone(&bytecode)
        ),
        Err(DeploymentBytecodeHydrationError::MissingBytecode { package })
            if package.as_ref() == &missing_reference
    ));

    let other = admitted_bytecode("other");
    let mismatched = package_artifact(
        "example.mismatch",
        "build:mismatch",
        Some(bytecode.reference().clone()),
    );
    let mismatched_reference = package_reference(&mismatched);
    assert!(matches!(
        HydratedBytecodePackage::checked(mismatched_reference, mismatched, other),
        Err(DeploymentBytecodeHydrationError::ReferenceMismatch {
            expected,
            actual,
        }) if matches!(
            expected.as_ref(),
            DeploymentBytecodeReference::PackageBytecode { .. }
        ) && matches!(
            actual.as_ref(),
            DeploymentBytecodeReference::PackageBytecode { .. }
        )
    ));
}

#[test]
fn deployment_checked_constructor_canonicalizes_consumer_facts() {
    let bytecode = admitted_bytecode("shared");
    let first = hydrated_package("example.a", "build:a", &bytecode);
    let first_reference = first.reference().clone();
    let second = hydrated_package("example.b", "build:b", &bytecode);
    let own_contract = contract_reference("example.consumer");
    let dependency_contract = contract_reference("example.provider");
    let dependency_key = ServiceRequirementKey {
        caller_package_build_id: first_reference.package_build_id.clone(),
        service_requirement_slot: 4,
    };
    let deployment = deployment(
        first_reference,
        own_contract.clone(),
        vec![ServiceSelectorBinding {
            key: dependency_key.clone(),
            contract: dependency_contract.clone(),
        }],
    );
    let reference = deployment_reference(&deployment);
    let contracts = BTreeMap::from([
        (own_contract.clone(), contract(&own_contract)),
        (dependency_contract.clone(), contract(&dependency_contract)),
    ]);
    let dependency = HydratedServiceDependency::new(
        dependency_key.clone(),
        dependency_contract.clone(),
        BTreeSet::from([ContractOperationId::new("operation:provider.call")]),
    );

    let hydrated = HydratedDeploymentBytecode::checked(
        reference.clone(),
        Arc::clone(&deployment),
        contracts,
        vec![dependency],
        vec![second, first],
    )
    .unwrap();

    assert_eq!(hydrated.reference(), &reference);
    assert!(Arc::ptr_eq(hydrated.deployment(), &deployment));
    assert_eq!(hydrated.contract_store().len(), 2);
    let row = hydrated
        .service_dependencies()
        .get(&dependency_key)
        .unwrap();
    assert_eq!(row.key(), &dependency_key);
    assert_eq!(row.contract(), &dependency_contract);
    assert!(row
        .used_operations()
        .contains(&ContractOperationId::new("operation:provider.call")));
    assert_eq!(
        hydrated.packages().keys().cloned().collect::<Vec<_>>(),
        vec![
            PackageBuildId::new("build:a"),
            PackageBuildId::new("build:b")
        ]
    );
}

#[test]
fn deployment_checked_constructor_rejects_duplicate_package_and_service_slot() {
    let bytecode = admitted_bytecode("duplicates");
    let package = hydrated_package("example.package", "build:package", &bytecode);
    let package_again = hydrated_package("example.package", "build:package", &bytecode);
    let package_reference = package.reference().clone();
    let own_contract = contract_reference("example.consumer");
    let deployment_record = deployment(package_reference, own_contract.clone(), Vec::new());
    let reference = deployment_reference(&deployment_record);
    let contracts = BTreeMap::from([(own_contract.clone(), contract(&own_contract))]);
    assert!(matches!(
        HydratedDeploymentBytecode::checked(
            reference,
            deployment_record,
            contracts,
            Vec::new(),
            vec![package, package_again],
        ),
        Err(DeploymentBytecodeHydrationError::DuplicatePackage { .. })
    ));

    let package = hydrated_package("example.package", "build:package", &bytecode);
    let package_reference = package.reference().clone();
    let dependency_contract = contract_reference("example.provider");
    let key = ServiceRequirementKey {
        caller_package_build_id: package_reference.package_build_id.clone(),
        service_requirement_slot: 1,
    };
    let deployment = deployment(
        package_reference,
        own_contract.clone(),
        vec![ServiceSelectorBinding {
            key: key.clone(),
            contract: dependency_contract.clone(),
        }],
    );
    let reference = deployment_reference(&deployment);
    let contracts = BTreeMap::from([
        (own_contract.clone(), contract(&own_contract)),
        (dependency_contract.clone(), contract(&dependency_contract)),
    ]);
    let dependency = || {
        HydratedServiceDependency::new(key.clone(), dependency_contract.clone(), BTreeSet::new())
    };
    assert!(matches!(
        HydratedDeploymentBytecode::checked(
            reference,
            deployment,
            contracts,
            vec![dependency(), dependency()],
            vec![package],
        ),
        Err(DeploymentBytecodeHydrationError::DuplicateServiceSlot { key: duplicate })
            if duplicate == key
    ));
}

#[test]
fn deployment_checked_constructor_rejects_contract_mismatch() {
    let bytecode = admitted_bytecode("contract-mismatch");
    let package = hydrated_package("example.package", "build:package", &bytecode);
    let package_reference = package.reference().clone();
    let own_contract = contract_reference("example.consumer");
    let expected_contract = contract_reference("example.expected");
    let actual_contract = contract_reference("example.actual");
    let key = ServiceRequirementKey {
        caller_package_build_id: package_reference.package_build_id.clone(),
        service_requirement_slot: 7,
    };
    let deployment = deployment(
        package_reference,
        own_contract.clone(),
        vec![ServiceSelectorBinding {
            key: key.clone(),
            contract: expected_contract.clone(),
        }],
    );
    let reference = deployment_reference(&deployment);
    let contracts = BTreeMap::from([
        (own_contract.clone(), contract(&own_contract)),
        (expected_contract.clone(), contract(&expected_contract)),
        (actual_contract.clone(), contract(&actual_contract)),
    ]);
    let dependency =
        HydratedServiceDependency::new(key.clone(), actual_contract.clone(), BTreeSet::new());

    assert!(matches!(
        HydratedDeploymentBytecode::checked(
            reference,
            deployment,
            contracts,
            vec![dependency],
            vec![package],
        ),
        Err(DeploymentBytecodeHydrationError::ContractMismatch {
            key: Some(mismatch_key),
            expected: Some(expected),
            actual: Some(actual),
        }) if mismatch_key == key
            && expected.as_ref() == &expected_contract
            && actual.as_ref() == &actual_contract
    ));
}

mod load;
