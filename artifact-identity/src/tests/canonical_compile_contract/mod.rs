use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryCallbackContract, BoundaryCallbackExpirationError,
    BoundaryCallbackLifetime, BoundaryCallbackOperation, BoundaryCancellationContract,
    BoundaryConfigRequirement, BoundaryEffectGuarantee, BoundaryErrorContract,
    BoundaryImplementationRequirements, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryParameter, BoundaryReturn, BoundaryStateKind, BoundaryStateRequirement,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, CallableEffectSummary, CallableMayEffects,
    CallableProvenanceSummary, CallableSemanticFacts, CallableTargetFact, ContractDiagnosticText,
    ContractRequirement, ContractSchemaType, ContractTypeDescriptor, ContractTypeNameability,
    ContractTypeRef, ContractTypeShape, ExecutableExport, ExecutableSignatureIr, FileIrRef,
    OperationCallableKind, OperationTargetRef, PackageArtifact, PackageBuildId, PackageCallableId,
    PackageCallableLinkFact, PackageCallableParameter, PackageCallableSignature,
    PackageConfigRequirement, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageLocalAbiSymbol, PackageRequirement, PackageResourceRequirement,
    PackageRuntimeCapabilityRequirement, PackageRuntimeRequirements, PackageTypeRef,
    PublicationResourceRef, ServiceCallRef, ServiceContract, ServiceProtocolIdentity,
    ServiceRequirement, TypeRefIr, ValueProvenance, PACKAGE_ARTIFACT_SCHEMA_VERSION,
    SERVICE_CONTRACT_SCHEMA_VERSION,
};

use super::*;

mod contract_identity;
mod package_artifact_identity;
mod schema_fidelity;

pub(super) fn contract_fixture() -> ServiceContract {
    let service_id = "example.echo";
    let version = "1.0.0";
    let payload_id = contract_type_id(service_id, version, "payload").unwrap();
    let callback_id = contract_type_id(service_id, version, "observer").unwrap();
    let echo_id = contract_operation_id(service_id, version, "echo").unwrap();
    let health_id = contract_operation_id(service_id, version, "health").unwrap();

    let payload_shape = ContractTypeShape {
        nameability: ContractTypeNameability::PublicNameable,
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("message".to_string(), ContractTypeRef::builtin("string"))]),
        },
    };
    let callback_shape = ContractTypeShape {
        nameability: ContractTypeNameability::PublicNameable,
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::CallbackInterface {
            operations: BTreeMap::from([(
                "observe".to_string(),
                BoundaryCallbackOperation {
                    parameters: vec![ContractTypeRef::contract(payload_id.clone())],
                    return_type: ContractTypeRef::builtin("void"),
                    may_suspend: false,
                },
            )]),
        },
    };
    let echo = BoundaryOperationDescriptor {
        operation_id: echo_id.clone(),
        stable_key: "echo".to_string(),
        contract: BoundaryOperationContract {
            parameters: vec![BoundaryParameter {
                name: "input".to_string(),
                ty: ContractTypeRef::contract(payload_id.clone()),
                value_plan: linkable_plan(BoundaryValueOwner::Caller),
            }],
            return_value: BoundaryReturn {
                ty: ContractTypeRef::contract(payload_id.clone()),
                value_plan: linkable_plan(BoundaryValueOwner::Provider),
            },
            errors: BoundaryErrorContract::Typed {
                payload_type: ContractTypeRef::builtin("string"),
                value_plan: linkable_plan(BoundaryValueOwner::Provider),
            },
            stream: BoundaryStreamContract::Unary,
            cancellation: BoundaryCancellationContract::Cooperative,
            callbacks: BoundaryCallbackContract::RequestScoped {
                interface_type_ids: vec![callback_id.clone()],
                lifetime: BoundaryCallbackLifetime::TopLevelRequest,
                expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
            },
            may_suspend: true,
            effect_guarantee: detached_effect_guarantee(),
        },
    };
    let health = BoundaryOperationDescriptor {
        operation_id: health_id.clone(),
        stable_key: "health".to_string(),
        contract: BoundaryOperationContract {
            parameters: Vec::new(),
            return_value: BoundaryReturn {
                ty: ContractTypeRef::builtin("bool"),
                value_plan: linkable_plan(BoundaryValueOwner::Provider),
            },
            errors: BoundaryErrorContract::None,
            stream: BoundaryStreamContract::Unary,
            cancellation: BoundaryCancellationContract::NotCancellable,
            callbacks: BoundaryCallbackContract::None,
            may_suspend: false,
            effect_guarantee: detached_effect_guarantee(),
        },
    };

    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(echo_id.clone(), echo), (health_id.clone(), health)]),
        boundary_schema: BTreeMap::from([
            (
                payload_id.clone(),
                ContractSchemaType {
                    contract_type_id: payload_id.clone(),
                    stable_key: "payload".to_string(),
                    shape: payload_shape,
                },
            ),
            (
                callback_id.clone(),
                ContractSchemaType {
                    contract_type_id: callback_id.clone(),
                    stable_key: "observer".to_string(),
                    shape: callback_shape,
                },
            ),
        ]),
        diagnostic_text: ContractDiagnosticText {
            service: "Echo service".to_string(),
            operations: BTreeMap::from([
                (echo_id, "Echo a payload".to_string()),
                (health_id, "Health probe".to_string()),
            ]),
            types: BTreeMap::from([
                (payload_id, "Echo payload".to_string()),
                (callback_id, "Observer callback".to_string()),
            ]),
        },
    };
    assign_service_contract_identities(&mut contract).unwrap();
    contract
}

pub(super) fn package_artifact_fixture() -> PackageArtifact {
    let contract = contract_fixture();
    let echo = contract
        .operations
        .values()
        .find(|descriptor| descriptor.stable_key == "echo")
        .unwrap()
        .clone();
    let payload_id = contract
        .boundary_schema
        .values()
        .find(|schema| schema.stable_key == "payload")
        .unwrap()
        .contract_type_id
        .clone();
    let callable_id = PackageCallableId::new("pkg-callable:handle");
    let file = FileIrRef {
        file_ir_identity: "skiff-file-ir-v5:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        module_path: "pkg.main".to_string(),
        artifact_path: Some("units/files/pkg.json".to_string()),
        source_ast_hash: Some("source-only".to_string()),
    };
    let target = OperationTargetRef {
        file_ref: file.clone(),
        executable_index: 0,
        callable_abi_id: callable_id.to_string(),
        callable_kind: OperationCallableKind::PublicFunction,
    };
    let contract_requirement = ContractRequirement {
        alias: "echo".to_string(),
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    };
    let semantic_facts = CallableSemanticFacts {
        effects: CallableEffectSummary::Analyzed {
            effects: no_effects(),
        },
        provenance: CallableProvenanceSummary::Analyzed {
            return_origins: vec![ValueProvenance::Fresh],
            throw_origins: vec![ValueProvenance::Fresh],
            escape_lanes: Vec::new(),
        },
        resolved_call_targets: BTreeMap::from([(
            4,
            CallableTargetFact::ContractOperation {
                operation_id: echo.operation_id.clone(),
            },
        )]),
    };
    let implementation_requirements = BoundaryImplementationRequirements {
        config: vec![BoundaryConfigRequirement {
            path: "echo.prefix".to_string(),
            value_type: "string".to_string(),
            required: true,
        }],
        state: vec![BoundaryStateRequirement {
            key: "echo-state".to_string(),
            kind: BoundaryStateKind::Database,
        }],
        native_capabilities: Vec::new(),
        runtime_capabilities: vec!["async".to_string()],
        complete_may_effects: no_effects(),
        provenance: semantic_facts.provenance.clone(),
    };
    let mut used_operations = BTreeSet::new();
    used_operations.insert(echo.operation_id.clone());

    let mut implementation_links = PackageImplementationLinks::default();
    implementation_links.functions.insert(
        "handle".to_string(),
        ExecutableExport {
            file: file.clone(),
            executable_index: 0,
            symbol: "handle".to_string(),
            signature: ExecutableSignatureIr {
                params: Vec::new(),
                return_type: TypeRefIr::native("Json"),
                self_type: None,
                may_suspend: true,
            },
        },
    );

    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "example.echo-provider".to_string(),
        package_version: "2.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file],
        static_resources: vec![PublicationResourceRef {
            path: "templates/echo.txt".to_string(),
            sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            byte_len: 12,
            content_type: Some("text/plain".to_string()),
            artifact_path: Some("resources/sha256/bb".to_string()),
        }],
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::from([(
                "handle".to_string(),
                PackageLocalAbiSymbol::Callable {
                    callable_id: callable_id.clone(),
                    signature: PackageCallableSignature {
                        parameters: vec![PackageCallableParameter {
                            name: "input".to_string(),
                            ty: PackageTypeRef::Contract {
                                contract_type_id: payload_id.clone(),
                            },
                        }],
                        return_type: PackageTypeRef::Contract {
                            contract_type_id: payload_id,
                        },
                        throw_types: vec![PackageTypeRef::Local {
                            local_type: TypeRefIr::native("string"),
                        }],
                        may_suspend: true,
                    },
                },
            )]),
        },
        implementation_links,
        callable_links: BTreeMap::from([(
            callable_id.clone(),
            PackageCallableLinkFact {
                callable_id: callable_id.clone(),
                target,
            },
        )]),
        package_requirements: vec![PackageRequirement {
            alias: "util".to_string(),
            package_id: "example.util".to_string(),
            exact_version: "1.4.0".to_string(),
            expected_local_abi: PackageLocalAbiIdentity::new("util-abi"),
        }],
        contract_requirements: vec![contract_requirement.clone()],
        service_requirements: vec![ServiceRequirement {
            contract_requirement: contract_requirement.clone(),
            service_binding_slot: 0,
            used_operations,
        }],
        runtime_requirements: PackageRuntimeRequirements {
            config: vec![PackageConfigRequirement {
                path: "echo.prefix".to_string(),
                value_type: "string".to_string(),
                required: true,
            }],
            resources: vec![PackageResourceRequirement {
                key: "echo-db".to_string(),
                capability: "mongodb".to_string(),
            }],
            runtime_capabilities: vec![PackageRuntimeCapabilityRequirement {
                capability: "async".to_string(),
                required_version: "1".to_string(),
            }],
        },
        callable_semantic_facts: BTreeMap::from([(callable_id.clone(), semantic_facts)]),
        boundary_projections: BTreeMap::from([(
            callable_id,
            BoundaryCallableProjection::Available {
                operation_contract: echo.contract.clone(),
                implementation_requirements,
            },
        )]),
        service_call_refs: vec![ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: echo.operation_id,
            expected_protocol_identity: contract_requirement.expected_protocol_identity,
        }],
    };
    assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

pub(super) fn linkable_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn detached_effect_guarantee() -> BoundaryEffectGuarantee {
    BoundaryEffectGuarantee {
        detached_parameters: true,
        detached_return: true,
        detached_error: true,
        no_caller_reachable_mutation: true,
        no_caller_value_escape: true,
        no_same_heap_identity: true,
    }
}

pub(super) fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend: true,
    }
}
