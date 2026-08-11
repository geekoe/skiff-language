use super::*;

use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, derive_bytecode_statement_manifest_identity,
    BytecodeArtifact, BytecodeConstantRef, BytecodeFunctionOrigin,
    BytecodeFunctionStatementManifest, BytecodeImage, BytecodePoolEntry, BytecodePools,
    BytecodeStatementManifestIdentity, CallableEffectSummary, CallableProvenanceSummary,
    CallableProvenanceUnknownReason, CallableSemanticFacts, ContractDiagnosticText,
    ContractTypeDescriptor, ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentRevision, FileIrRef, FrameLayout, FrozenConstantGraph, FrozenConstantNode,
    InstructionSourceSite, LiteralIr, OperationCallableKind, OperationTargetRef, PackageCallableId,
    PackageCallableLinkFact, PackageCallableParameter, PackageCallableSignature,
    PackageExecutableCoordinate, PackageImplementationLinks, PackageLocalAbi,
    PackageLocalAbiIdentity, PackageLocalAbiSymbol, PackageRuntimeRequirements,
    PackageSchemaCanonicalDescriptor, PackageSchemaIndexIdentity, PackageSchemaIndexRef,
    PackageSchemaTypeRecord, PackageSyntheticCallbackOwner, PackageTypeRef, ParameterSlotDecl,
    RelocatableBytecodeFunction, ServiceProtocolIdentity, ServiceSelectorBinding, SourceMapEntry,
    StatementAttributionId, StatementEntry, SyntheticInstructionSiteReason, TypeRefIr,
    ValueDropPlan, ValueTransferPlan, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION, PACKAGE_ARTIFACT_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
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
        value_lifecycle_policy: skiff_artifact_model::value_lifecycle_policy_identity().clone(),
        host_effect_registry: skiff_artifact_model::host_effect_registry_identity().clone(),
        intrinsic_registry: skiff_artifact_model::intrinsic_registry_identity().clone(),
        platform_error_projection_registry:
            skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
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

fn historical_platform_error_projection_registry_ref(
    fingerprint_character: char,
) -> skiff_artifact_model::PlatformErrorProjectionRegistryRef {
    let current = skiff_artifact_model::current_platform_error_projection_registry_ref();
    serde_json::from_value(serde_json::json!({
        "registryId": current.registry_id(),
        "registryVersion": current.registry_version(),
        "fingerprint": format!(
            "sha256:{}",
            fingerprint_character.to_string().repeat(64)
        ),
    }))
    .unwrap()
}

fn assert_header_pin_drift_rejected(seed: &str, field: &str, mutate: fn(&mut BytecodeArtifact)) {
    let mut artifact = admitted_bytecode(seed).artifact().clone();
    mutate(&mut artifact);

    let error = ValidatedBytecodeArtifact::admit(artifact)
        .expect_err("semantic authority drift must fail before loader admission");
    assert!(matches!(
        &error,
        skiff_artifact_identity::ArtifactIdentityError::InvalidBytecodeStructural(
            skiff_artifact_model::bytecode::validate::StructuralValidationError::Header { .. }
        )
    ));
    assert!(error.to_string().contains(field), "{field}: {error}");
}

fn bytecode_with_type_root(seed: &str, ty: TypeRefIr) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = admitted_bytecode(seed).artifact().clone();
    artifact
        .image
        .pools
        .types
        .push(BytecodePoolEntry::TypeRef { ty });
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

fn schema_record(
    package_id: &str,
    stable_schema_key: &str,
    descriptor: ContractTypeDescriptor,
) -> PackageSchemaTypeRecord {
    let canonical_descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor,
    };
    let package_schema_type_id = skiff_artifact_model::derive_package_schema_type_id(
        package_id,
        stable_schema_key,
        &canonical_descriptor,
    )
    .unwrap();
    PackageSchemaTypeRecord {
        package_id: package_id.to_string(),
        stable_schema_key: stable_schema_key.to_string(),
        package_schema_type_id,
        canonical_descriptor,
    }
}

fn schema_type_ref(record: &PackageSchemaTypeRecord) -> TypeRefIr {
    TypeRefIr::PackageSchema {
        package_id: record.package_id.clone(),
        stable_schema_key: record.stable_schema_key.clone(),
        package_schema_type_id: record.package_schema_type_id.clone(),
    }
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
        platform_error_projection_registry:
            skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
        files: Vec::new(),
        static_resources: Vec::new(),
        bytecode,
        bytecode_statement_manifest_identity: derive_bytecode_statement_manifest_identity(
            package_id,
            &[],
        )
        .unwrap(),
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

fn statement_manifest_identity(
    package_id: &str,
    bytecode: &ValidatedBytecodeArtifact,
) -> BytecodeStatementManifestIdentity {
    let mut functions = bytecode
        .view()
        .functions()
        .iter()
        .map(|function| {
            BytecodeFunctionStatementManifest::new(
                function.origin.clone(),
                function.statement_entries.clone(),
            )
        })
        .collect::<Vec<_>>();
    functions.sort_by(|left, right| left.origin.cmp(&right.origin));
    derive_bytecode_statement_manifest_identity(package_id, &functions).unwrap()
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

fn hydrated_deployment(
    implementation: PackageArtifactRef,
    packages: Vec<HydratedBytecodePackage>,
) -> Result<HydratedDeploymentBytecode, DeploymentBytecodeHydrationError> {
    let own_contract = contract_reference("example.consumer");
    let deployment = deployment(implementation, own_contract.clone(), Vec::new());
    HydratedDeploymentBytecode::checked(
        deployment_reference(&deployment),
        deployment,
        BTreeMap::from([(own_contract.clone(), contract(&own_contract))]),
        Vec::new(),
        packages,
    )
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
                sequence_ordinal: 0,
                attribution_id: StatementAttributionId::Generated { ordinal: 0 },
                site: InstructionSourceSite::Synthetic {
                    reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
                },
            }],
            source_map: vec![SourceMapEntry {
                start_pc: 0,
                end_pc: 1,
                site: InstructionSourceSite::Synthetic {
                    reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
                },
            }],
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
    artifact.bytecode_statement_manifest_identity =
        statement_manifest_identity(&artifact.package_id, bytecode);
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
    package_id: &str,
    ordinary_callable: &PackageCallableId,
) -> (Arc<ValidatedBytecodeArtifact>, PackageCallableId) {
    let callback_callable = skiff_artifact_model::derive_synthetic_callback_callable_id(
        package_id,
        ordinary_callable,
        0,
    )
    .unwrap();
    let mut artifact = bytecode.artifact().clone();
    let mut callback = artifact.image.functions["manifest::run"].clone();
    callback.function_key = "manifest::run$callback0".to_string();
    callback.origin = BytecodeFunctionOrigin::SyntheticCallback {
        owner: owner.clone(),
        site_ordinal: 0,
    };
    callback.effect_summary_ref = callback_callable.clone();
    artifact
        .image
        .functions
        .insert(callback.function_key.clone(), callback);
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    (
        Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap()),
        callback_callable,
    )
}

fn add_synthetic_callback_owner(
    artifact: &mut PackageArtifact,
    owner: &PackageExecutableCoordinate,
    callback_callable: &PackageCallableId,
) {
    artifact
        .synthetic_callback_owners
        .push(PackageSyntheticCallbackOwner {
            owner: owner.clone(),
            site_ordinal: 0,
            package_callable_id: callback_callable.clone(),
        });
    artifact
        .callable_semantic_facts
        .insert(callback_callable.clone(), callable_semantic_facts());
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
fn package_checked_constructor_pins_exact_v7_header_view_reference_and_registry() {
    let bytecode = admitted_bytecode("v7-header");
    let artifact = package_artifact(
        "example.package",
        "build:package",
        Some(bytecode.reference().clone()),
    );
    let hydrated = HydratedBytecodePackage::checked(
        package_reference(&artifact),
        artifact,
        Arc::clone(&bytecode),
    )
    .unwrap();
    let admitted = hydrated.bytecode();
    let artifact = admitted.artifact();
    let view = admitted.view();
    let opcode_fingerprint = opcode_table_fingerprint();

    assert_eq!(artifact.magic.as_str(), BYTECODE_MAGIC);
    assert_eq!(artifact.schema_version.as_str(), BYTECODE_SCHEMA_VERSION);
    assert_eq!(view.schema_version(), BYTECODE_SCHEMA_VERSION);
    assert_eq!(view.schema_version(), artifact.schema_version.as_str());
    assert_eq!(artifact.isa_version.as_str(), BYTECODE_ISA_VERSION);
    assert_eq!(view.isa_version(), BYTECODE_ISA_VERSION);
    assert_eq!(view.isa_version(), artifact.isa_version.as_str());
    assert_eq!(
        artifact.opcode_table_fingerprint.as_str(),
        opcode_fingerprint.as_str()
    );
    assert_eq!(view.opcode_table_fingerprint(), opcode_fingerprint.as_str());
    assert_eq!(
        view.opcode_table_fingerprint(),
        artifact.opcode_table_fingerprint.as_str()
    );
    assert_eq!(
        &artifact.native_value_lifecycle_registry,
        skiff_artifact_model::native_value_lifecycle_registry_identity()
    );
    assert_eq!(
        view.native_value_lifecycle_registry(),
        &artifact.native_value_lifecycle_registry
    );
    assert_eq!(
        &artifact.value_lifecycle_policy,
        skiff_artifact_model::value_lifecycle_policy_identity()
    );
    assert_eq!(
        view.value_lifecycle_policy(),
        &artifact.value_lifecycle_policy
    );
    assert_eq!(
        &artifact.host_effect_registry,
        skiff_artifact_model::host_effect_registry_identity()
    );
    assert_eq!(view.host_effect_registry(), &artifact.host_effect_registry);
    assert_eq!(
        &artifact.intrinsic_registry,
        skiff_artifact_model::intrinsic_registry_identity()
    );
    assert_eq!(view.intrinsic_registry(), &artifact.intrinsic_registry);
    assert_eq!(
        admitted.reference().bytecode_identity.as_str(),
        artifact.bytecode_identity.as_str()
    );
    assert_eq!(
        hydrated.artifact().bytecode.as_ref(),
        Some(admitted.reference())
    );
    assert_eq!(
        view.bytecode_identity(),
        admitted.reference().bytecode_identity.as_str()
    );
    let registry = hydrated.platform_error_projection_registry();
    assert_eq!(
        registry,
        &hydrated.artifact().platform_error_projection_registry
    );
    assert_eq!(registry, &artifact.platform_error_projection_registry);
    assert_eq!(registry, view.platform_error_projection_registry());
    assert_eq!(
        registry,
        skiff_artifact_model::current_platform_error_projection_registry_ref()
    );
}

#[test]
fn bytecode_admission_rejects_native_lifecycle_registry_pin_drift_before_loader() {
    assert_header_pin_drift_rejected(
        "native-lifecycle-drift",
        "nativeValueLifecycleRegistry",
        |artifact| {
            artifact
                .native_value_lifecycle_registry
                .fingerprint
                .push_str(":drift");
        },
    );
}

#[test]
fn bytecode_admission_rejects_value_lifecycle_policy_pin_drift_before_loader() {
    assert_header_pin_drift_rejected(
        "value-lifecycle-drift",
        "valueLifecyclePolicy",
        |artifact| {
            artifact
                .value_lifecycle_policy
                .fingerprint
                .push_str(":drift")
        },
    );
}

#[test]
fn bytecode_admission_rejects_host_effect_registry_pin_drift_before_loader() {
    assert_header_pin_drift_rejected("host-effect-drift", "hostEffectRegistry", |artifact| {
        artifact.host_effect_registry.fingerprint.push_str(":drift")
    });
}

#[test]
fn bytecode_admission_rejects_intrinsic_registry_pin_drift_before_loader() {
    assert_header_pin_drift_rejected("intrinsic-drift", "intrinsicRegistry", |artifact| {
        artifact.intrinsic_registry.fingerprint.push_str(":drift")
    });
}

#[test]
fn bytecode_admission_rejects_historical_platform_error_registry_before_loader() {
    assert_header_pin_drift_rejected(
        "platform-error-registry-drift",
        "platformErrorProjectionRegistry",
        |artifact| {
            artifact.platform_error_projection_registry =
                historical_platform_error_projection_registry_ref('0');
        },
    );
}

#[test]
fn package_hydration_rejects_legal_historical_platform_error_registry() {
    let bytecode = admitted_bytecode("historical-package-registry");
    let mut artifact = package_artifact(
        "example.historical",
        "unassigned",
        Some(bytecode.reference().clone()),
    )
    .as_ref()
    .clone();
    let historical = historical_platform_error_projection_registry_ref('1');
    artifact.platform_error_projection_registry = historical.clone();
    skiff_artifact_identity::assign_package_artifact_identities(&mut artifact).unwrap();
    skiff_artifact_identity::validate_package_artifact_identities(&artifact).unwrap();
    let artifact = Arc::new(artifact);
    let reference = package_reference(&artifact);

    let error =
        HydratedBytecodePackage::checked(reference.clone(), artifact, Arc::clone(&bytecode))
            .expect_err("a legal historical PackageArtifact must not join the current runtime");
    assert!(matches!(
        error,
        DeploymentBytecodeHydrationError::PlatformErrorProjectionRegistryMismatch {
            package,
            package_artifact,
            bytecode_header,
            structurally_validated_view,
            runtime,
        } if package.as_ref() == &reference
            && package_artifact.as_ref() == &historical
            && bytecode_header.as_ref()
                == &bytecode.artifact().platform_error_projection_registry
            && structurally_validated_view.as_ref()
                == bytecode.view().platform_error_projection_registry()
            && runtime.as_ref()
                == skiff_artifact_model::current_platform_error_projection_registry_ref()
    ));
}

#[test]
fn platform_error_registry_mismatch_precedes_reference_and_manifest_mismatches() {
    let bytecode = admitted_bytecode("registry-precedence");
    let mut artifact = package_artifact(
        "example.precedence",
        "unassigned",
        Some(bytecode.reference().clone()),
    )
    .as_ref()
    .clone();
    let stale_reference = package_reference(&artifact);
    artifact.platform_error_projection_registry =
        historical_platform_error_projection_registry_ref('2');
    artifact.bytecode_statement_manifest_identity =
        derive_bytecode_statement_manifest_identity("example.other", &[]).unwrap();
    skiff_artifact_identity::assign_package_artifact_identities(&mut artifact).unwrap();
    skiff_artifact_identity::validate_package_artifact_identities(&artifact).unwrap();
    let artifact = Arc::new(artifact);
    assert_ne!(stale_reference, package_reference(&artifact));

    assert!(matches!(
        HydratedBytecodePackage::checked(stale_reference, artifact, bytecode),
        Err(DeploymentBytecodeHydrationError::PlatformErrorProjectionRegistryMismatch { .. })
    ));
}

#[test]
fn package_checked_constructor_joins_v6_callable_origin_and_self_manifests() {
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
fn synthetic_callback_index_rejects_missing_owner_manifest() {
    let (bytecode, coordinate, canonical) = callable_bytecode(false);
    let (bytecode, _) =
        with_synthetic_callback(&bytecode, &coordinate, "example.manifest", &canonical);
    let artifact = callable_package(
        &bytecode,
        &coordinate,
        &canonical,
        OperationCallableKind::InternalFunction,
    );

    assert!(matches!(
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::SyntheticCallback,
            ..
        })
    ));
}

#[test]
fn synthetic_callback_index_binds_exact_owner_effect_and_function() {
    let (bytecode, coordinate, canonical) = callable_bytecode(false);
    let (bytecode, callback_callable) =
        with_synthetic_callback(&bytecode, &coordinate, "example.manifest", &canonical);
    let mut artifact = callable_package(
        &bytecode,
        &coordinate,
        &canonical,
        OperationCallableKind::InternalFunction,
    )
    .as_ref()
    .clone();
    add_synthetic_callback_owner(&mut artifact, &coordinate, &callback_callable);
    let artifact = Arc::new(artifact);

    let hydrated =
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode).unwrap();

    assert_eq!(
        hydrated.function_key_for_synthetic_callback(&coordinate, 0),
        Some("manifest::run$callback0")
    );
    assert_eq!(
        hydrated.synthetic_callback_callable(&coordinate, 0),
        Some(&callback_callable)
    );
    assert_eq!(
        hydrated.function_key_for_synthetic_callback_callable(&callback_callable),
        Some("manifest::run$callback0")
    );
    assert_eq!(
        hydrated.canonical_effect_callable_for_function_key("manifest::run$callback0"),
        Some(&callback_callable)
    );
    assert_eq!(
        hydrated.canonical_effect_callable_for_function_key("manifest::run"),
        Some(&canonical)
    );
}

#[test]
fn synthetic_callback_index_rejects_effect_owner_drift() {
    let (bytecode, coordinate, canonical) = callable_bytecode(false);
    let (bytecode, callback_callable) =
        with_synthetic_callback(&bytecode, &coordinate, "example.manifest", &canonical);
    let mut drifted = bytecode.artifact().clone();
    drifted
        .image
        .functions
        .get_mut("manifest::run$callback0")
        .unwrap()
        .effect_summary_ref = canonical.clone();
    skiff_artifact_identity::assign_bytecode_identity(&mut drifted).unwrap();
    let drifted = Arc::new(ValidatedBytecodeArtifact::admit(drifted).unwrap());
    let mut artifact = callable_package(
        &drifted,
        &coordinate,
        &canonical,
        OperationCallableKind::InternalFunction,
    )
    .as_ref()
    .clone();
    add_synthetic_callback_owner(&mut artifact, &coordinate, &callback_callable);
    let artifact = Arc::new(artifact);

    assert!(matches!(
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, drifted),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::SyntheticCallback,
            ..
        })
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
    assert_eq!(
        hydrated.platform_error_projection_registry(),
        skiff_artifact_model::current_platform_error_projection_registry_ref()
    );
    assert!(hydrated.packages().values().all(|package| {
        let descriptor = hydrated.platform_error_projection_registry();
        package.platform_error_projection_registry() == descriptor
            && &package.artifact().platform_error_projection_registry == descriptor
            && &package
                .bytecode()
                .artifact()
                .platform_error_projection_registry
                == descriptor
            && package
                .bytecode()
                .view()
                .platform_error_projection_registry()
                == descriptor
    }));
}

#[test]
fn deployment_registry_join_rejects_mixed_closure_before_manifest_validation() {
    let bytecode = admitted_bytecode("mixed-registry-closure");
    let implementation = hydrated_package("example.a", "build:a", &bytecode);
    let implementation_reference = implementation.reference().clone();
    let mut dependency = hydrated_package("example.b", "build:b", &bytecode);
    let historical = historical_platform_error_projection_registry_ref('3');
    dependency.platform_error_projection_registry = historical.clone();

    let own_contract = contract_reference("example.consumer");
    let mut deployment_record = deployment(
        implementation_reference.clone(),
        own_contract.clone(),
        Vec::new(),
    );
    Arc::make_mut(&mut deployment_record)
        .operation_bindings
        .push(skiff_artifact_model::DeploymentOperationBinding {
            contract_operation_id: ContractOperationId::new("operation:missing"),
            package_callable_id: PackageCallableId::new("callable:missing"),
        });
    let reference = deployment_reference(&deployment_record);
    let contracts = BTreeMap::from([(own_contract.clone(), contract(&own_contract))]);

    let error = HydratedDeploymentBytecode::checked(
        reference,
        deployment_record,
        contracts,
        Vec::new(),
        vec![implementation, dependency],
    )
    .expect_err("mixed registry closure must fail before the missing callable manifest");
    assert!(matches!(
        error,
        DeploymentBytecodeHydrationError::MixedPlatformErrorProjectionRegistry {
            implementation,
            implementation_registry,
            package,
            package_registry,
        } if implementation.as_ref() == &implementation_reference
            && implementation_registry.as_ref()
                == skiff_artifact_model::current_platform_error_projection_registry_ref()
            && package.package_id == "example.b"
            && package_registry.as_ref() == &historical
    ));
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

#[test]
fn deployment_schema_closure_hydrates_cross_package_descriptors_without_schema_resolver() {
    let child = schema_record(
        "example.schema",
        "model.MessageBody",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("value".to_string(), ContractTypeRef::builtin("string"))]),
        },
    );
    let schema = schema_record(
        "example.schema",
        "model.Message",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::from([(
                "body".to_string(),
                ContractTypeRef::package_schema(
                    child.package_id.clone(),
                    child.stable_schema_key.clone(),
                    child.package_schema_type_id.clone(),
                ),
            )]),
        },
    );
    let consumer_bytecode = bytecode_with_type_root("schema-consumer", schema_type_ref(&schema));
    let schema_bytecode = admitted_bytecode("schema-owner");

    let consumer_artifact = package_artifact(
        "example.consumer-package",
        "build:schema-consumer",
        Some(consumer_bytecode.reference().clone()),
    );
    let consumer = HydratedBytecodePackage::checked(
        package_reference(&consumer_artifact),
        consumer_artifact,
        consumer_bytecode,
    )
    .unwrap();
    let consumer_reference = consumer.reference().clone();
    let mut schema_artifact = package_artifact(
        "example.schema",
        "build:schema-owner",
        Some(schema_bytecode.reference().clone()),
    )
    .as_ref()
    .clone();
    schema_artifact
        .bytecode_schema_records
        .insert(schema.package_schema_type_id.clone(), schema);
    schema_artifact
        .bytecode_schema_records
        .insert(child.package_schema_type_id.clone(), child);
    let schema_artifact = Arc::new(schema_artifact);
    let schema_owner = HydratedBytecodePackage::checked(
        package_reference(&schema_artifact),
        schema_artifact,
        schema_bytecode,
    )
    .unwrap();

    hydrated_deployment(consumer_reference, vec![schema_owner, consumer]).unwrap();
}

#[test]
fn deployment_schema_closure_rejects_missing_and_extra_descriptor_rows() {
    let schema = schema_record(
        "example.schema",
        "model.Message",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    );
    let consumer_bytecode = bytecode_with_type_root("schema-missing", schema_type_ref(&schema));
    let consumer_artifact = package_artifact(
        "example.consumer-package",
        "build:schema-missing",
        Some(consumer_bytecode.reference().clone()),
    );
    let consumer = HydratedBytecodePackage::checked(
        package_reference(&consumer_artifact),
        consumer_artifact,
        consumer_bytecode,
    )
    .unwrap();
    let consumer_reference = consumer.reference().clone();
    assert!(matches!(
        hydrated_deployment(consumer_reference, vec![consumer]),
        Err(DeploymentBytecodeHydrationError::MissingSchemaPackageOwner { .. })
    ));

    let extra = schema_record(
        "example.extra",
        "model.Extra",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    );
    let bytecode = admitted_bytecode("schema-extra");
    let mut artifact = package_artifact(
        "example.extra",
        "build:schema-extra",
        Some(bytecode.reference().clone()),
    )
    .as_ref()
    .clone();
    artifact
        .bytecode_schema_records
        .insert(extra.package_schema_type_id.clone(), extra);
    let artifact = Arc::new(artifact);
    let package =
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode).unwrap();
    let package_reference = package.reference().clone();
    assert!(matches!(
        hydrated_deployment(package_reference, vec![package]),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::SchemaDescriptor,
            ..
        })
    ));
}

mod load;
mod package_types;
mod statement_attribution;
