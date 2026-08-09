use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_model::*;
use skiff_deployment::{
    assembly::resolve_runtime_assembly, projection::project_service_deployment,
};

use super::resolver::TypedResolver;

const CALLBACK_INTERFACE_SYMBOL: &str = "CallbackProbe";
const CALLBACK_INTERFACE_METHOD: &str = "invoke";
const CALLBACK_RECEIVER_SYMBOL: &str = "CallbackProbeReceiver";
const IMPLEMENTATION_MODULE_PATH: &str = "phase_four.implementation";
const CALLBACK_OWNER_EXECUTABLE_INDEX: u32 = 3;
const CALLBACK_STREAM_OWNER_EXECUTABLE_INDEX: u32 = 2;
const PROVIDER_PACKAGE_ID: &str = "example.phase-four-provider";

fn fixture_instruction_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn package_schema_type(
    stable_schema_key: &str,
    descriptor: ContractTypeDescriptor,
) -> (PackageSchemaTypeRef, PackageSchemaTypeRecord) {
    let canonical_descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor,
    };
    let package_schema_type_id = skiff_artifact_identity::package_schema_type_id(
        PROVIDER_PACKAGE_ID,
        stable_schema_key,
        &canonical_descriptor,
    )
    .expect("fixture Package schema identity should be canonical");
    (
        PackageSchemaTypeRef {
            package_id: PROVIDER_PACKAGE_ID.to_string(),
            stable_schema_key: stable_schema_key.to_string(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
        PackageSchemaTypeRecord {
            package_id: PROVIDER_PACKAGE_ID.to_string(),
            stable_schema_key: stable_schema_key.to_string(),
            package_schema_type_id,
            canonical_descriptor,
        },
    )
}

fn callback_contract_type(interface: &PackageSchemaTypeRef) -> ContractTypeRef {
    ContractTypeRef::AnyInterface {
        interface: Box::new(ContractTypeRef::package_schema(
            interface.package_id.clone(),
            interface.stable_schema_key.clone(),
            interface.package_schema_type_id.clone(),
        )),
        arguments: Vec::new(),
    }
}

#[derive(Clone, Copy)]
enum ProviderBehavior {
    ReturnTrue,
    ReturnNull,
    ThrowTypedError,
    InvokeCallback,
    EmitCallbackStream,
    EmitBooleanSequence,
    EmitBooleanThenError,
}

#[derive(Clone, Copy)]
enum ConsumerBehavior {
    ReturnCall,
    ReturnGenericBooleanStream,
    InvokeCallback,
    ConsumeCallbackStream { break_after_item: bool },
    ConsumeBooleanSequence,
}

enum ImplementationRole {
    Provider(ProviderBehavior),
    Consumer {
        service_call: ServiceCallRef,
        package_call: (String, PackageCallableId),
        behavior: ConsumerBehavior,
    },
}

#[derive(Clone)]
pub(super) struct TypedExecutionContract {
    consumer_operation: BoundaryOperationContract,
    consumer_schema_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    provider_operation: BoundaryOperationContract,
    provider_contract_schema_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    provider_schema_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    consumer_behavior: ConsumerBehavior,
    provider_behavior: ProviderBehavior,
    consumer_may_suspend: bool,
    provider_may_suspend: bool,
    callback_owner_may_suspend: bool,
}

impl TypedExecutionContract {
    fn with_provider_behavior(
        operation: BoundaryOperationContract,
        schema_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
        may_suspend: bool,
        provider_behavior: ProviderBehavior,
    ) -> Self {
        Self {
            consumer_operation: operation.clone(),
            consumer_schema_records: schema_records.clone(),
            provider_operation: operation,
            provider_contract_schema_records: schema_records.clone(),
            provider_schema_records: schema_records,
            consumer_behavior: ConsumerBehavior::ReturnCall,
            provider_behavior,
            consumer_may_suspend: may_suspend,
            provider_may_suspend: may_suspend,
            callback_owner_may_suspend: false,
        }
    }

    pub(super) fn unary() -> Self {
        Self::with_provider_behavior(
            unary_contract(),
            BTreeMap::new(),
            false,
            ProviderBehavior::ReturnTrue,
        )
    }

    pub(super) fn returning_null(
        operation: BoundaryOperationContract,
        schema_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
        may_suspend: bool,
    ) -> Self {
        Self::with_provider_behavior(
            operation,
            schema_records,
            may_suspend,
            ProviderBehavior::ReturnNull,
        )
    }

    pub(super) fn async_typed_error() -> Self {
        let stable_key = "asyncError";
        let payload_fields = BTreeMap::from([(
            "messages".to_string(),
            ContractTypeRef::Builtin {
                name: "Array".to_string(),
                arguments: vec![ContractTypeRef::builtin("string")],
            },
        )]);
        let (_, payload_record) = package_schema_type(
            stable_key,
            ContractTypeDescriptor::Record {
                fields: payload_fields,
            },
        );
        let provider_schema_records = BTreeMap::from([(
            payload_record.package_schema_type_id.clone(),
            payload_record,
        )]);
        Self {
            consumer_operation: unary_contract(),
            consumer_schema_records: BTreeMap::new(),
            provider_operation: unary_contract(),
            provider_contract_schema_records: BTreeMap::new(),
            provider_schema_records,
            consumer_behavior: ConsumerBehavior::ReturnCall,
            provider_behavior: ProviderBehavior::ThrowTypedError,
            consumer_may_suspend: false,
            provider_may_suspend: true,
            callback_owner_may_suspend: false,
        }
    }

    pub(super) fn callback() -> Self {
        Self::callback_with_operation_key(CALLBACK_INTERFACE_METHOD)
    }

    pub(super) fn callback_with_operation_key(contract_operation: &str) -> Self {
        let stable_key = "callbackProbe";
        let (callback_type, callback_record) = package_schema_type(
            stable_key,
            ContractTypeDescriptor::CallbackInterface {
                operations: BTreeMap::from([(
                    contract_operation.to_string(),
                    BoundaryCallbackOperation {
                        parameters: Vec::new(),
                        return_type: ContractTypeRef::builtin("bool"),
                    },
                )]),
            },
        );
        let callback_plan = BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::CallbackCapability,
            encoding: BoundaryValueEncoding::OpaqueCapability,
            owner: BoundaryValueOwner::CapabilityOwner,
            lifetime: BoundaryValueLifetime::Request,
        };
        let mut provider_operation = unary_contract();
        provider_operation.parameters = vec![BoundaryParameter {
            name: "callback".to_string(),
            ty: callback_contract_type(&callback_type),
            value_plan: callback_plan,
        }];
        provider_operation.callbacks = BoundaryCallbackContract::RequestScoped {
            interface_types: vec![callback_type],
            lifetime: BoundaryCallbackLifetime::TopLevelRequest,
            expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
        };
        let provider_schema_records = BTreeMap::from([(
            callback_record.package_schema_type_id.clone(),
            callback_record,
        )]);
        Self {
            consumer_operation: unary_contract(),
            consumer_schema_records: BTreeMap::new(),
            provider_operation,
            provider_contract_schema_records: provider_schema_records.clone(),
            provider_schema_records,
            consumer_behavior: ConsumerBehavior::InvokeCallback,
            provider_behavior: ProviderBehavior::InvokeCallback,
            consumer_may_suspend: false,
            provider_may_suspend: false,
            callback_owner_may_suspend: false,
        }
    }

    pub(super) fn callback_stream() -> Self {
        Self::callback_stream_with_operation_key(CALLBACK_INTERFACE_METHOD)
    }

    pub(super) fn callback_stream_cancel() -> Self {
        let mut fixture = Self::callback_stream();
        fixture.consumer_behavior = ConsumerBehavior::ConsumeCallbackStream {
            break_after_item: true,
        };
        fixture
    }

    pub(super) fn boolean_stream() -> Self {
        let mut fixture = Self::with_provider_behavior(
            boolean_stream_contract(),
            BTreeMap::new(),
            true,
            ProviderBehavior::EmitBooleanSequence,
        );
        fixture.consumer_behavior = ConsumerBehavior::ConsumeBooleanSequence;
        fixture
    }

    pub(super) fn unconsumed_boolean_stream() -> Self {
        let mut fixture = Self::with_provider_behavior(
            boolean_stream_contract(),
            BTreeMap::new(),
            true,
            ProviderBehavior::EmitBooleanSequence,
        );
        fixture.consumer_behavior = ConsumerBehavior::ReturnGenericBooleanStream;
        fixture
    }

    pub(super) fn boolean_stream_error() -> Self {
        let (_, error_record) = package_schema_type(
            "streamError",
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "message".to_string(),
                    ContractTypeRef::builtin("string"),
                )]),
            },
        );
        let operation = boolean_stream_contract();
        Self {
            consumer_operation: operation.clone(),
            consumer_schema_records: BTreeMap::new(),
            provider_operation: operation,
            provider_contract_schema_records: BTreeMap::new(),
            provider_schema_records: BTreeMap::from([(
                error_record.package_schema_type_id.clone(),
                error_record,
            )]),
            consumer_behavior: ConsumerBehavior::ConsumeBooleanSequence,
            provider_behavior: ProviderBehavior::EmitBooleanThenError,
            consumer_may_suspend: true,
            provider_may_suspend: true,
            callback_owner_may_suspend: false,
        }
    }

    pub(super) fn callback_stream_wrong_tuple() -> Self {
        let mut fixture = Self::callback_stream();
        let mut descriptor = fixture
            .provider_schema_records
            .values()
            .next()
            .expect("callback stream schema should contain its interface")
            .canonical_descriptor
            .descriptor
            .clone();
        let ContractTypeDescriptor::CallbackInterface { operations } = &mut descriptor else {
            panic!("callback stream schema should retain its callback descriptor")
        };
        operations
            .get_mut(CALLBACK_INTERFACE_METHOD)
            .expect("callback stream descriptor should contain invoke")
            .return_type = ContractTypeRef::builtin("string");
        let (callback_type, callback_record) = package_schema_type("callbackProbe", descriptor);
        let provider_schema_records = BTreeMap::from([(
            callback_record.package_schema_type_id.clone(),
            callback_record,
        )]);
        fixture.provider_contract_schema_records = provider_schema_records.clone();
        fixture.provider_schema_records = provider_schema_records;
        let BoundaryStreamContract::ServerStream { item_type, .. } =
            &mut fixture.provider_operation.stream
        else {
            panic!("callback stream fixture must remain a server stream")
        };
        *item_type = callback_contract_type(&callback_type);
        let BoundaryCallbackContract::RequestScoped {
            interface_types, ..
        } = &mut fixture.provider_operation.callbacks
        else {
            panic!("callback stream fixture must retain callback declarations")
        };
        *interface_types = vec![callback_type];
        fixture
    }

    pub(super) fn callback_stream_with_operation_key(contract_operation: &str) -> Self {
        let stable_key = "callbackProbe";
        let (callback_type, callback_record) = package_schema_type(
            stable_key,
            ContractTypeDescriptor::CallbackInterface {
                operations: BTreeMap::from([(
                    contract_operation.to_string(),
                    BoundaryCallbackOperation {
                        parameters: Vec::new(),
                        return_type: ContractTypeRef::builtin("bool"),
                    },
                )]),
            },
        );
        let callback_plan = BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::CallbackCapability,
            encoding: BoundaryValueEncoding::OpaqueCapability,
            owner: BoundaryValueOwner::CapabilityOwner,
            lifetime: BoundaryValueLifetime::Stream,
        };
        let mut provider_operation = unary_contract();
        provider_operation.return_value.ty = ContractTypeRef::builtin("void");
        provider_operation.stream = BoundaryStreamContract::ServerStream {
            item_type: callback_contract_type(&callback_type),
            item_value_plan: callback_plan,
        };
        provider_operation.callbacks = BoundaryCallbackContract::RequestScoped {
            interface_types: vec![callback_type],
            lifetime: BoundaryCallbackLifetime::Stream,
            expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
        };
        let provider_schema_records = BTreeMap::from([(
            callback_record.package_schema_type_id.clone(),
            callback_record,
        )]);
        Self {
            consumer_operation: unary_contract(),
            consumer_schema_records: BTreeMap::new(),
            provider_operation,
            provider_contract_schema_records: provider_schema_records.clone(),
            provider_schema_records,
            consumer_behavior: ConsumerBehavior::ConsumeCallbackStream {
                break_after_item: false,
            },
            provider_behavior: ProviderBehavior::EmitCallbackStream,
            consumer_may_suspend: false,
            provider_may_suspend: true,
            callback_owner_may_suspend: false,
        }
    }

    pub(super) fn with_provider_may_suspend(mut self, may_suspend: bool) -> Self {
        self.provider_may_suspend = may_suspend;
        self
    }

    pub(super) fn with_callback_owner_may_suspend(mut self, may_suspend: bool) -> Self {
        self.callback_owner_may_suspend = may_suspend;
        self
    }
}

pub(super) struct ProjectedFixture {
    pub(super) assembly: RuntimeAssembly,
    pub(super) resolver: TypedResolver,
    pub(super) consumer_deployment: ServiceDeploymentRef,
    pub(super) provider_deployment: ServiceDeploymentRef,
    pub(super) provider_operation: ContractOperationId,
    pub(super) provider_callable: PackageCallableId,
    pub(super) consumer_package: PackageArtifactRef,
    pub(super) consumer_file_ir_identity: String,
    pub(super) callback_interface_id: String,
}

impl ProjectedFixture {
    pub(super) fn new(contract_fixture: TypedExecutionContract) -> Self {
        Self::new_with_consumer_service_id(contract_fixture, "example.phase-four.consumer")
    }

    pub(super) fn new_with_consumer_service_id(
        contract_fixture: TypedExecutionContract,
        consumer_service_id: &str,
    ) -> Self {
        let consumer_operation_contract = contract_fixture.consumer_operation;
        let consumer_schema_records = contract_fixture.consumer_schema_records;
        let provider_operation_contract = contract_fixture.provider_operation;
        let provider_contract_schema_records = contract_fixture.provider_contract_schema_records;
        let provider_schema_records = contract_fixture.provider_schema_records;
        let consumer_behavior = contract_fixture.consumer_behavior;
        let provider_behavior = contract_fixture.provider_behavior;
        let consumer_may_suspend = contract_fixture.consumer_may_suspend;
        let provider_may_suspend = contract_fixture.provider_may_suspend;
        let callback_owner_may_suspend = contract_fixture.callback_owner_may_suspend;
        let (provider_contract, provider_operation) = service_contract(
            "example.phase-four.provider",
            "provide",
            provider_operation_contract.clone(),
            &provider_contract_schema_records,
        );
        let provider_contract_ref = contract_ref(&provider_contract);
        let (consumer_contract, consumer_operation) = service_contract(
            consumer_service_id,
            "consume",
            consumer_operation_contract.clone(),
            &consumer_schema_records,
        );
        let consumer_contract_ref = contract_ref(&consumer_contract);

        let provider_callable =
            PackageCallableId::new("pkg-callable:example.phase-four-provider:provide");
        let provider_file = implementation_file(
            IMPLEMENTATION_MODULE_PATH,
            "provide",
            &provider_operation_contract,
            provider_may_suspend,
            callback_owner_may_suspend,
            ImplementationRole::Provider(provider_behavior),
            None,
        );
        let provider_file_ref = file_ref(&provider_file);
        let provider_package = implementation_package(
            "example.phase-four-provider",
            "provide",
            provider_callable.clone(),
            &provider_file,
            provider_operation_contract,
            &provider_schema_records,
            None,
            None,
            None,
        );
        let provider_package_ref = package_ref(&provider_package);
        let provider_abi = provider_package
            .package_local_abi
            .local_abi_identity
            .to_string();

        let service_requirement_slot = 0;
        let service_call = ServiceCallRef {
            service_requirement_slot,
            contract_operation_id: provider_operation.clone(),
            expected_protocol_identity: provider_contract_ref.service_protocol_identity.clone(),
        };
        let provider_requirement = ContractRequirement {
            alias: "provider".to_string(),
            service_id: provider_contract_ref.service_id.clone(),
            contract_version: provider_contract_ref.contract_version.clone(),
            expected_protocol_identity: provider_contract_ref.service_protocol_identity.clone(),
        };
        let consumer_file = implementation_file(
            IMPLEMENTATION_MODULE_PATH,
            "consume",
            &consumer_operation_contract,
            consumer_may_suspend,
            callback_owner_may_suspend,
            ImplementationRole::Consumer {
                service_call: service_call.clone(),
                package_call: ("providerPackage".to_string(), provider_callable.clone()),
                behavior: consumer_behavior.clone(),
            },
            Some(&provider_abi),
        );
        let consumer_file_ref = file_ref(&consumer_file);
        let consumer_file_ir_identity = consumer_file_ref.file_ir_identity.clone();
        let consumer_callable =
            PackageCallableId::new("pkg-callable:example.phase-four-consumer:consume");
        let consumer_package = implementation_package(
            "example.phase-four-consumer",
            "consume",
            consumer_callable.clone(),
            &consumer_file,
            consumer_operation_contract,
            &consumer_schema_records,
            Some((provider_requirement, service_call)),
            Some(("providerPackage".to_string(), provider_package_ref.clone())),
            Some(&provider_abi),
        );
        let consumer_package_ref = package_ref(&consumer_package);
        let consumer_abi = consumer_package
            .package_local_abi
            .local_abi_identity
            .to_string();
        let callback_interface_id = if matches!(
            consumer_behavior,
            ConsumerBehavior::InvokeCallback | ConsumerBehavior::ConsumeCallbackStream { .. }
        ) {
            canonical_callback_interface_ref(&provider_abi).interface_abi_id
        } else {
            canonical_callback_interface_ref_for("example.phase-four-consumer", &consumer_abi)
                .interface_abi_id
        };

        let provider_deployment_artifact = project_service_deployment(
            deployment_input(
                provider_contract_ref.clone(),
                DeploymentRevision::new("phase-four-provider-r1"),
                provider_package_ref.clone(),
                provider_operation.clone(),
                provider_callable.clone(),
                Vec::new(),
                Vec::new(),
                BTreeMap::new(),
                Vec::new(),
            ),
            &provider_contract,
            std::slice::from_ref(&provider_package),
            &provider_contract_schema_records,
        )
        .expect("provider deployment should project from typed contract/package artifacts");
        let provider_deployment =
            skiff_artifact_identity::service_deployment_ref(&provider_deployment_artifact);
        let consumer_deployment_artifact = project_service_deployment(
            deployment_input(
                consumer_contract_ref.clone(),
                DeploymentRevision::new("phase-four-consumer-r1"),
                consumer_package_ref.clone(),
                consumer_operation,
                consumer_callable,
                vec![PackageBinding {
                    key: PackageRequirementKey {
                        caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                        package_requirement_alias: "providerPackage".to_string(),
                    },
                    package: provider_package_ref.clone(),
                }],
                vec![ServiceSelectorBinding {
                    key: ServiceRequirementKey {
                        caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                        service_requirement_slot,
                    },
                    contract: provider_contract_ref.clone(),
                }],
                BTreeMap::new(),
                Vec::new(),
            ),
            &consumer_contract,
            &[consumer_package.clone(), provider_package.clone()],
            &consumer_schema_records,
        )
        .expect("consumer deployment should project from typed contract/package artifacts");
        let consumer_deployment =
            skiff_artifact_identity::service_deployment_ref(&consumer_deployment_artifact);
        let assembly = resolve_runtime_assembly(
            std::slice::from_ref(&consumer_deployment),
            &[
                consumer_deployment_artifact.clone(),
                provider_deployment_artifact.clone(),
            ],
            &[consumer_contract.clone(), provider_contract.clone()],
            &[consumer_package.clone(), provider_package.clone()],
        )
        .expect("typed provider/consumer closure should resolve into a RuntimeAssembly");
        let resolver = TypedResolver {
            deployments: vec![
                (
                    consumer_deployment.clone(),
                    Arc::new(consumer_deployment_artifact),
                ),
                (
                    provider_deployment.clone(),
                    Arc::new(provider_deployment_artifact),
                ),
            ],
            contracts: vec![
                (consumer_contract_ref, Arc::new(consumer_contract)),
                (provider_contract_ref, Arc::new(provider_contract)),
            ],
            packages: vec![
                (consumer_package_ref.clone(), Arc::new(consumer_package)),
                (provider_package_ref.clone(), Arc::new(provider_package)),
            ],
            files: vec![
                (
                    consumer_package_ref.clone(),
                    consumer_file_ref,
                    Arc::new(consumer_file),
                ),
                (
                    provider_package_ref,
                    provider_file_ref,
                    Arc::new(provider_file),
                ),
            ],
            package_schema_records: consumer_schema_records
                .values()
                .chain(provider_schema_records.values())
                .map(|record| {
                    (
                        PackageSchemaTypeRecordRef {
                            package_id: record.package_id.clone(),
                            package_schema_type_id: record.package_schema_type_id.clone(),
                        },
                        Arc::new(record.clone()),
                    )
                })
                .collect(),
        };
        Self {
            assembly,
            resolver,
            consumer_deployment,
            provider_deployment,
            provider_operation,
            provider_callable,
            consumer_package: consumer_package_ref,
            consumer_file_ir_identity,
            callback_interface_id,
        }
    }
}

fn deployment_input(
    contract: ServiceContractRef,
    deployment_revision: DeploymentRevision,
    implementation: PackageArtifactRef,
    operation: ContractOperationId,
    package_callable_id: PackageCallableId,
    package_bindings: Vec<PackageBinding>,
    service_selectors: Vec<ServiceSelectorBinding>,
    gateway_entries: BTreeMap<GatewayEntryKey, DeploymentGatewayEntry>,
    ingress: Vec<DeploymentIngressBinding>,
) -> ServiceDeploymentInput {
    ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision,
        implementation,
        operation_bindings: vec![ServiceDeploymentOperationInput {
            contract_operation_id: operation,
            package_callable_id,
        }],
        package_bindings,
        service_selectors,
        gateway_entries,
        ingress,
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "Phase four typed execution fixture".to_string(),
            notes: BTreeMap::new(),
        },
    }
}

fn service_contract(
    service_id: &str,
    stable_key: &str,
    operation_contract: BoundaryOperationContract,
    schema_records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> (ServiceContract, ContractOperationId) {
    let contract_version = "1.0.0";
    let operation_id =
        skiff_artifact_identity::contract_operation_id(service_id, contract_version, stable_key)
            .expect("fixture operation identity should be canonical");
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: contract_version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation_id.clone(),
                stable_key: stable_key.to_string(),
                contract: operation_contract,
            },
        )]),
        package_type_requirements: if schema_records.is_empty() {
            Vec::new()
        } else {
            vec![PackageTypeRequirement {
                package_id: PROVIDER_PACKAGE_ID.to_string(),
                required_type_ids: schema_records.keys().cloned().collect(),
            }]
        },
        diagnostic_text: ContractDiagnosticText {
            service: "Phase four typed execution fixture".to_string(),
            operations: BTreeMap::new(),
            types: schema_records
                .iter()
                .map(|(id, record)| (id.clone(), record.stable_schema_key.clone()))
                .collect(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract)
        .expect("fixture contract should receive canonical identities");
    (contract, operation_id)
}

fn implementation_file(
    module_path: &str,
    symbol: &str,
    operation_contract: &BoundaryOperationContract,
    may_suspend: bool,
    callback_owner_may_suspend: bool,
    role: ImplementationRole,
    provider_abi: Option<&str>,
) -> FileIrUnit {
    let mut file = FileIrUnit::empty(module_path, format!("source:{module_path}"));
    let callback_ref = match (provider_abi, &role) {
        (Some(provider_abi), ImplementationRole::Consumer { behavior, .. })
            if matches!(
                behavior,
                ConsumerBehavior::InvokeCallback | ConsumerBehavior::ConsumeCallbackStream { .. }
            ) =>
        {
            provider_callback_interface_ref(provider_abi)
        }
        _ => callback_interface_ref(),
    };
    let signature =
        executable_signature_from_operation(operation_contract, may_suspend, &callback_ref);
    let mut entry = ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: signature.params,
        return_type: signature.return_type,
        self_type: None,
        slots: parameter_slots(operation_contract),
        may_suspend,
        body: ExecutableBody {
            blocks: Vec::new(),
            statements: Vec::new(),
            expressions: Vec::new(),
        },
        source_span: None,
    };
    match role {
        ImplementationRole::Provider(behavior) => {
            configure_provider_entry(&mut file, &mut entry, module_path, behavior, &callback_ref);
            file.executables.push(entry);
            install_provider_support(
                &mut file,
                module_path,
                symbol,
                behavior,
                callback_owner_may_suspend,
                &callback_ref,
            );
        }
        ImplementationRole::Consumer {
            service_call,
            package_call,
            behavior,
        } => {
            configure_consumer_entry(&mut file, &mut entry, service_call, behavior, &callback_ref);
            file.executables.push(entry);
            install_consumer_support(
                &mut file,
                module_path,
                symbol,
                package_call,
                behavior,
                callback_owner_may_suspend,
                &callback_ref,
            );
        }
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("fixture File IR should receive a canonical identity");
    file
}

fn executable_signature_from_operation(
    operation: &BoundaryOperationContract,
    may_suspend: bool,
    callback_ref: &InterfaceInstantiationRef,
) -> ExecutableSignatureIr {
    ExecutableSignatureIr {
        params: operation
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| ParamIr {
                name: parameter.name.clone(),
                slot: u32::try_from(index).expect("fixture parameter count must fit u32"),
                ty: file_type_from_contract(&parameter.ty, callback_ref),
            })
            .collect(),
        return_type: operation_return_file_type(operation, callback_ref),
        self_type: None,
        may_suspend,
    }
}

fn parameter_slots(operation: &BoundaryOperationContract) -> SlotLayout {
    SlotLayout {
        slots: operation
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| SlotIr {
                index: u32::try_from(index).expect("fixture parameter count must fit u32"),
                name: parameter.name.clone(),
                kind: SlotKind::Param,
            })
            .collect(),
        frame_size: u32::try_from(operation.parameters.len())
            .expect("fixture parameter count must fit u32"),
    }
}

fn operation_return_file_type(
    operation: &BoundaryOperationContract,
    callback_ref: &InterfaceInstantiationRef,
) -> TypeRefIr {
    match &operation.stream {
        BoundaryStreamContract::Unary => {
            file_type_from_contract(&operation.return_value.ty, callback_ref)
        }
        BoundaryStreamContract::ServerStream { item_type, .. } => TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![file_type_from_contract(item_type, callback_ref)],
        },
        BoundaryStreamContract::Unsupported { .. } => {
            panic!("available fixture operation cannot contain an unsupported stream")
        }
    }
}

fn file_type_from_contract(
    ty: &ContractTypeRef,
    callback_ref: &InterfaceInstantiationRef,
) -> TypeRefIr {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments
                .iter()
                .map(|ty| file_type_from_contract(ty, callback_ref))
                .collect(),
        },
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => TypeRefIr::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            assert!(
                arguments.is_empty()
                    && matches!(interface.as_ref(), ContractTypeRef::PackageSchema { .. }),
                "callback fixture requires an exact non-generic any-interface contract"
            );
            TypeRefIr::AnyInterface {
                interface: callback_ref.clone(),
            }
        }
        ContractTypeRef::TypeParam { name } => TypeRefIr::TypeParam { name: name.clone() },
        ContractTypeRef::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| (name.clone(), file_type_from_contract(ty, callback_ref)))
                .collect(),
        },
        ContractTypeRef::StructuralUnion { variants } => TypeRefIr::Union {
            items: variants
                .iter()
                .map(|ty| file_type_from_contract(ty, callback_ref))
                .collect(),
        },
        ContractTypeRef::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(file_type_from_contract(inner, callback_ref)),
        },
        ContractTypeRef::Literal { value } => TypeRefIr::Literal {
            value: match value {
                ContractLiteral::String { value } => LiteralIr::String {
                    value: value.clone(),
                },
            },
        },
    }
}

fn configure_provider_entry(
    file: &mut FileIrUnit,
    entry: &mut ExecutableIr,
    module_path: &str,
    behavior: ProviderBehavior,
    callback_ref: &InterfaceInstantiationRef,
) {
    match behavior {
        ProviderBehavior::ReturnTrue => configure_return_true_entry(entry),
        ProviderBehavior::ReturnNull => configure_return_null_provider_entry(entry),
        ProviderBehavior::ThrowTypedError => {
            configure_async_typed_error_provider_entry(file, entry, module_path);
        }
        ProviderBehavior::InvokeCallback => configure_callback_provider_entry(entry, callback_ref),
        ProviderBehavior::EmitCallbackStream => configure_callback_stream_provider_entry(
            entry,
            CALLBACK_STREAM_OWNER_EXECUTABLE_INDEX,
            callback_ref,
        ),
        ProviderBehavior::EmitBooleanSequence => {
            configure_boolean_stream_provider_entry(file, entry, module_path, false);
        }
        ProviderBehavior::EmitBooleanThenError => {
            configure_boolean_stream_provider_entry(file, entry, module_path, true);
        }
    }
}

fn configure_consumer_entry(
    file: &mut FileIrUnit,
    entry: &mut ExecutableIr,
    service_call: ServiceCallRef,
    behavior: ConsumerBehavior,
    callback_ref: &InterfaceInstantiationRef,
) {
    file.external_refs.service_call_refs.push(service_call);
    let call_args = match behavior {
        ConsumerBehavior::InvokeCallback => append_callback_preimage(entry, callback_ref),
        ConsumerBehavior::ReturnCall
        | ConsumerBehavior::ReturnGenericBooleanStream
        | ConsumerBehavior::ConsumeCallbackStream { .. }
        | ConsumerBehavior::ConsumeBooleanSequence => Vec::new(),
    };
    let call_expression = u32::try_from(entry.body.expressions.len())
        .expect("fixture expression count should fit u32");
    entry.body.expressions.push(ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::ServiceCall {
                service_call_ref_index: ServiceCallRefIndex::new(0),
            },
            site: fixture_instruction_site(),
            args: call_args,
            type_args: if matches!(
                behavior,
                ConsumerBehavior::ReturnGenericBooleanStream
                    | ConsumerBehavior::ConsumeBooleanSequence
            ) {
                BTreeMap::from([("T".to_string(), TypeRefIr::builtin("bool"))])
            } else {
                BTreeMap::new()
            },
            metadata: BTreeMap::new(),
        },
    });
    match behavior {
        ConsumerBehavior::ReturnCall
        | ConsumerBehavior::ReturnGenericBooleanStream
        | ConsumerBehavior::InvokeCallback => {
            entry.body.statements.push(StmtIr::Return {
                value: Some(ExprRefIr {
                    expression: call_expression,
                }),
            });
            entry.body.blocks.push(BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            });
        }
        ConsumerBehavior::ConsumeCallbackStream { break_after_item } => {
            configure_callback_stream_consumer_entry(
                entry,
                call_expression,
                break_after_item,
                callback_ref,
            );
        }
        ConsumerBehavior::ConsumeBooleanSequence => {
            configure_boolean_stream_consumer_entry(entry, call_expression);
        }
    }
}

fn install_provider_support(
    file: &mut FileIrUnit,
    module_path: &str,
    symbol: &str,
    behavior: ProviderBehavior,
    callback_owner_may_suspend: bool,
    callback_ref: &InterfaceInstantiationRef,
) {
    match behavior {
        ProviderBehavior::InvokeCallback => {
            install_callback_interface_fixture(
                file,
                module_path,
                symbol,
                false,
                CALLBACK_OWNER_EXECUTABLE_INDEX,
                callback_owner_may_suspend,
                callback_ref,
            );
        }
        ProviderBehavior::EmitCallbackStream => {
            install_callback_interface_fixture(
                file,
                module_path,
                symbol,
                true,
                CALLBACK_STREAM_OWNER_EXECUTABLE_INDEX,
                callback_owner_may_suspend,
                callback_ref,
            );
            configure_return_true_entry(
                file.executables
                    .last_mut()
                    .expect("callback stream owner executable should be installed"),
            );
        }
        _ => {}
    }
}

fn install_consumer_support(
    file: &mut FileIrUnit,
    module_path: &str,
    symbol: &str,
    package_call: (String, PackageCallableId),
    behavior: ConsumerBehavior,
    callback_owner_may_suspend: bool,
    callback_ref: &InterfaceInstantiationRef,
) {
    let (dependency_ref, package_callable_id) = package_call;
    let package_ref = PackageRefIr::Dependency { dependency_ref };
    file.external_refs
        .package_callables
        .push(PackageCallableRef {
            package_ref: package_ref.clone(),
            package_callable_id: package_callable_id.clone(),
        });
    let package_target = CallTargetIr::PackageCallable {
        package_ref,
        package_callable_id,
    };
    if matches!(behavior, ConsumerBehavior::ConsumeBooleanSequence) {
        file.executables.push(package_stream_consumer_executable(
            format!("{symbol}_package_direct"),
            package_target,
        ));
    } else {
        let type_args = if matches!(behavior, ConsumerBehavior::ReturnGenericBooleanStream) {
            BTreeMap::from([("T".to_string(), TypeRefIr::builtin("bool"))])
        } else {
            BTreeMap::new()
        };
        file.executables.push(checkpoint_call_executable(
            format!("{symbol}_package_direct"),
            package_target,
            Vec::new(),
            type_args,
        ));
    }
    install_callback_interface_fixture(
        file,
        module_path,
        symbol,
        matches!(behavior, ConsumerBehavior::InvokeCallback),
        CALLBACK_OWNER_EXECUTABLE_INDEX,
        callback_owner_may_suspend,
        callback_ref,
    );
}

fn configure_boolean_stream_provider_entry(
    file: &mut FileIrUnit,
    entry: &mut ExecutableIr,
    module_path: &str,
    fail_after_first: bool,
) {
    entry.type_params = vec!["T".to_string()];
    entry.return_type = TypeRefIr::Builtin {
        name: "Stream".to_string(),
        args: vec![TypeRefIr::TypeParam {
            name: "T".to_string(),
        }],
    };
    entry.body.expressions = vec![
        ExprIr::Literal {
            value: LiteralIr::Bool { value: true },
        },
        ExprIr::Literal {
            value: LiteralIr::Bool { value: false },
        },
    ];
    entry.body.statements.push(StmtIr::Emit {
        operation: "provide".to_string(),
        value: ExprRefIr { expression: 0 },
    });
    if fail_after_first {
        file.declarations.types.insert(
            "StreamError".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: format!("{module_path}.StreamError"),
                source_span: None,
            },
        );
        file.type_table.push(TypeDeclIr {
            name: "StreamError".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("message".to_string(), TypeRefIr::builtin("string"))]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        entry.body.expressions.extend([
            ExprIr::Literal {
                value: LiteralIr::String {
                    value: "provider stream typed error".to_string(),
                },
            },
            ExprIr::Construct {
                type_ref: TypeRefIr::LocalType { type_index: 0 },
                fields: BTreeMap::from([("message".to_string(), ExprRefIr { expression: 2 })]),
            },
        ]);
        entry.body.statements.push(StmtIr::Throw {
            value: ExprRefIr { expression: 3 },
            payload_type: TypeRefIr::LocalType { type_index: 0 },
            site: fixture_instruction_site(),
        });
    } else {
        entry.body.statements.push(StmtIr::Emit {
            operation: "provide".to_string(),
            value: ExprRefIr { expression: 1 },
        });
    }
    entry.body.blocks.push(BlockIr {
        label: "entry".to_string(),
        statements: (0..entry.body.statements.len())
            .map(|statement| StmtRefIr {
                statement: u32::try_from(statement)
                    .expect("fixture statement index should fit u32"),
            })
            .collect(),
    });
}

fn configure_boolean_stream_consumer_entry(entry: &mut ExecutableIr, stream_expression: u32) {
    let item = u32::try_from(entry.body.expressions.len()).expect("fixture index should fit u32");
    entry.body.expressions.push(ExprIr::LoadSlot { slot: 0 });
    let seen = u32::try_from(entry.body.expressions.len()).expect("fixture index should fit u32");
    entry.body.expressions.push(ExprIr::LoadSlot { slot: 1 });
    let first = u32::try_from(entry.body.expressions.len()).expect("fixture index should fit u32");
    entry.body.expressions.push(ExprIr::Literal {
        value: LiteralIr::Bool { value: true },
    });
    let second = u32::try_from(entry.body.expressions.len()).expect("fixture index should fit u32");
    entry.body.expressions.push(ExprIr::Literal {
        value: LiteralIr::Bool { value: false },
    });
    let ordered =
        u32::try_from(entry.body.expressions.len()).expect("fixture index should fit u32");
    entry.body.expressions.push(ExprIr::Binary {
        op: BinaryOpIr::Equal,
        left: ExprRefIr { expression: item },
        right: ExprRefIr { expression: seen },
    });
    entry.slots = SlotLayout {
        slots: vec![
            SlotIr {
                index: 0,
                name: "item".to_string(),
                kind: SlotKind::Pattern,
            },
            SlotIr {
                index: 1,
                name: "seenFirst".to_string(),
                kind: SlotKind::Local,
            },
        ],
        frame_size: 2,
    };
    entry.body.statements.extend([
        StmtIr::Let {
            slot: 1,
            value: ExprRefIr { expression: first },
        },
        StmtIr::ForIn {
            item_slot: 0,
            item_type: Some(TypeRefIr::builtin("bool")),
            value_slot: None,
            iterable: ExprRefIr {
                expression: stream_expression,
            },
            body: "consume_boolean".to_string(),
        },
        StmtIr::Assert {
            condition: ExprRefIr {
                expression: ordered,
            },
            message: None,
        },
        StmtIr::Assign {
            target: AssignTargetIr::Slot { slot: 1 },
            value: ExprRefIr { expression: second },
        },
        StmtIr::Return { value: None },
    ]);
    entry.body.blocks.extend([
        BlockIr {
            label: "entry".to_string(),
            statements: vec![
                StmtRefIr { statement: 0 },
                StmtRefIr { statement: 1 },
                StmtRefIr { statement: 4 },
            ],
        },
        BlockIr {
            label: "consume_boolean".to_string(),
            statements: vec![StmtRefIr { statement: 2 }, StmtRefIr { statement: 3 }],
        },
    ]);
}

fn configure_return_true_entry(entry: &mut ExecutableIr) {
    entry.body = ExecutableBody {
        blocks: vec![BlockIr {
            label: "entry".to_string(),
            statements: vec![StmtRefIr { statement: 0 }],
        }],
        statements: vec![StmtIr::Return {
            value: Some(ExprRefIr { expression: 0 }),
        }],
        expressions: vec![ExprIr::Literal {
            value: LiteralIr::Bool { value: true },
        }],
    };
}

fn configure_return_null_provider_entry(entry: &mut ExecutableIr) {
    entry.body = ExecutableBody {
        blocks: vec![BlockIr {
            label: "entry".to_string(),
            statements: vec![StmtRefIr { statement: 0 }],
        }],
        statements: vec![StmtIr::Return { value: None }],
        expressions: Vec::new(),
    };
}

fn configure_async_typed_error_provider_entry(
    file: &mut FileIrUnit,
    entry: &mut ExecutableIr,
    module_path: &str,
) {
    let fields = BTreeMap::from([(
        "messages".to_string(),
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        },
    )]);
    file.declarations.types.insert(
        "AsyncError".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: format!("{module_path}.AsyncError"),
            source_span: None,
        },
    );
    file.type_table.push(TypeDeclIr {
        name: "AsyncError".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: fields.clone(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    let payload_type = TypeRefIr::LocalType { type_index: 0 };
    entry.body = ExecutableBody {
        blocks: vec![BlockIr {
            label: "entry".to_string(),
            statements: vec![StmtRefIr { statement: 0 }],
        }],
        statements: vec![StmtIr::Throw {
            value: ExprRefIr { expression: 2 },
            payload_type: payload_type.clone(),
            site: fixture_instruction_site(),
        }],
        expressions: vec![
            ExprIr::Literal {
                value: LiteralIr::String {
                    value: "provider async typed error".to_string(),
                },
            },
            ExprIr::ArrayLiteral {
                items: vec![ExprRefIr { expression: 0 }],
            },
            ExprIr::Construct {
                type_ref: payload_type,
                fields: BTreeMap::from([("messages".to_string(), ExprRefIr { expression: 1 })]),
            },
        ],
    };
}

fn configure_callback_stream_provider_entry(
    entry: &mut ExecutableIr,
    owner_executable_index: u32,
    callback_ref: &InterfaceInstantiationRef,
) {
    let callback_interface = callback_ref.clone();
    entry.return_type = TypeRefIr::Builtin {
        name: "Stream".to_string(),
        args: vec![TypeRefIr::AnyInterface {
            interface: callback_interface,
        }],
    };
    let callback = append_callback_preimage_at(entry, owner_executable_index, callback_ref);
    entry.body.statements.push(StmtIr::Emit {
        operation: "provide".to_string(),
        value: callback[0],
    });
    entry.body.blocks.push(BlockIr {
        label: "entry".to_string(),
        statements: vec![StmtRefIr { statement: 0 }],
    });
}

fn configure_callback_stream_consumer_entry(
    entry: &mut ExecutableIr,
    stream_expression: u32,
    break_after_item: bool,
    callback_ref: &InterfaceInstantiationRef,
) {
    let callback_interface = callback_ref.clone();
    let callback_method_abi_id = skiff_artifact_identity::canonical_interface_method_abi_id(
        &callback_interface,
        CALLBACK_INTERFACE_METHOD,
    );
    let callback_expression = u32::try_from(entry.body.expressions.len())
        .expect("fixture expression count should fit u32");
    entry.body.expressions.push(ExprIr::LoadSlot { slot: 0 });
    let invoke_expression = u32::try_from(entry.body.expressions.len())
        .expect("fixture expression count should fit u32");
    entry.body.expressions.push(ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::InterfaceMethod {
                interface: callback_interface.clone(),
                method_abi_id: callback_method_abi_id,
                slot: 0,
            },
            site: fixture_instruction_site(),
            args: vec![ExprRefIr {
                expression: callback_expression,
            }],
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        },
    });
    entry.slots = SlotLayout {
        slots: vec![SlotIr {
            index: 0,
            name: "callback".to_string(),
            kind: SlotKind::Pattern,
        }],
        frame_size: 1,
    };
    entry.body.statements.extend([
        StmtIr::ForIn {
            item_slot: 0,
            item_type: Some(TypeRefIr::AnyInterface {
                interface: callback_interface,
            }),
            value_slot: None,
            iterable: ExprRefIr {
                expression: stream_expression,
            },
            body: "consume_callback".to_string(),
        },
        StmtIr::Assert {
            condition: ExprRefIr {
                expression: invoke_expression,
            },
            message: None,
        },
    ]);
    let mut body_statements = vec![StmtRefIr { statement: 1 }];
    if break_after_item {
        let break_statement = u32::try_from(entry.body.statements.len())
            .expect("fixture statement count should fit u32");
        entry.body.statements.push(StmtIr::Break);
        body_statements.push(StmtRefIr {
            statement: break_statement,
        });
    }
    entry.body.blocks.extend([
        BlockIr {
            label: "entry".to_string(),
            statements: vec![StmtRefIr { statement: 0 }],
        },
        BlockIr {
            label: "consume_callback".to_string(),
            statements: body_statements,
        },
    ]);
}

fn configure_callback_provider_entry(
    entry: &mut ExecutableIr,
    callback_ref: &InterfaceInstantiationRef,
) {
    let callback_interface = callback_ref.clone();
    let callback_method_abi_id = skiff_artifact_identity::canonical_interface_method_abi_id(
        &callback_interface,
        CALLBACK_INTERFACE_METHOD,
    );
    assert_eq!(
        entry.params,
        vec![ParamIr {
            name: "callback".to_string(),
            slot: 0,
            ty: TypeRefIr::AnyInterface {
                interface: callback_interface.clone(),
            },
        }],
        "callback provider File IR signature must be derived from its operation"
    );
    entry.body = ExecutableBody {
        blocks: vec![BlockIr {
            label: "entry".to_string(),
            statements: vec![StmtRefIr { statement: 0 }],
        }],
        statements: vec![StmtIr::Expr {
            value: ExprRefIr { expression: 1 },
        }],
        expressions: vec![
            ExprIr::LoadSlot { slot: 0 },
            ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::InterfaceMethod {
                        interface: callback_interface,
                        method_abi_id: callback_method_abi_id,
                        slot: 0,
                    },
                    site: fixture_instruction_site(),
                    args: vec![ExprRefIr { expression: 0 }],
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            },
        ],
    };
}

fn append_callback_preimage(
    entry: &mut ExecutableIr,
    callback_ref: &InterfaceInstantiationRef,
) -> Vec<ExprRefIr> {
    append_callback_preimage_at(entry, CALLBACK_OWNER_EXECUTABLE_INDEX, callback_ref)
}

fn append_callback_preimage_at(
    entry: &mut ExecutableIr,
    owner_executable_index: u32,
    callback_ref: &InterfaceInstantiationRef,
) -> Vec<ExprRefIr> {
    let callback_interface = callback_ref.clone();
    let callback_method_abi_id = skiff_artifact_identity::canonical_interface_method_abi_id(
        &callback_interface,
        CALLBACK_INTERFACE_METHOD,
    );
    let concrete_type = TypeRefIr::LocalType { type_index: 0 };
    entry.body.expressions.extend([
        ExprIr::Construct {
            type_ref: concrete_type.clone(),
            fields: BTreeMap::new(),
        },
        ExprIr::InterfaceBox {
            value: ExprRefIr { expression: 0 },
            interface: callback_interface.clone(),
            source: BoxSourceIr::Local {
                concrete_type: concrete_type.clone(),
                method_table: InterfaceMethodTablePlanIr {
                    interface: callback_interface,
                    concrete_type: concrete_type.clone(),
                    slots: vec![InterfaceMethodSlotPlanIr {
                        slot: 0,
                        method_name: CALLBACK_INTERFACE_METHOD.to_string(),
                        method_abi_id: callback_method_abi_id,
                        signature: InterfaceMethodSlotSignatureIr {
                            params: vec![FunctionTypeParamIr {
                                name: "self".to_string(),
                                ty: concrete_type,
                            }],
                            return_type: TypeRefIr::builtin("bool"),
                        },
                        target: InterfaceMethodSlotTargetIr {
                            executable_index: owner_executable_index,
                            receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
                        },
                    }],
                },
            },
        },
    ]);
    vec![ExprRefIr { expression: 1 }]
}

fn install_callback_interface_fixture(
    file: &mut FileIrUnit,
    module_path: &str,
    symbol: &str,
    include_owner_method: bool,
    owner_executable_index: u32,
    owner_may_suspend: bool,
    callback_ref: &InterfaceInstantiationRef,
) {
    let callback_interface = callback_ref.clone();
    let callback_method_abi_id = skiff_artifact_identity::canonical_interface_method_abi_id(
        &callback_interface,
        CALLBACK_INTERFACE_METHOD,
    );
    file.declarations.types.insert(
        CALLBACK_RECEIVER_SYMBOL.to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: format!("{module_path}.{CALLBACK_RECEIVER_SYMBOL}"),
            source_span: None,
        },
    );
    file.declarations.types.insert(
        CALLBACK_INTERFACE_SYMBOL.to_string(),
        TypeDeclarationIr {
            type_index: 1,
            symbol: format!("{module_path}.{CALLBACK_INTERFACE_SYMBOL}"),
            source_span: None,
        },
    );
    file.declarations.interfaces.insert(
        CALLBACK_INTERFACE_SYMBOL.to_string(),
        InterfaceDeclIr {
            name: CALLBACK_INTERFACE_SYMBOL.to_string(),
            type_params: Vec::new(),
            operations: vec![InterfaceOperationIr {
                name: CALLBACK_INTERFACE_METHOD.to_string(),
                type_params: Vec::new(),
                params: vec![FunctionTypeParamIr {
                    name: "self".to_string(),
                    ty: TypeRefIr::builtin("Self"),
                }],
                return_type: TypeRefIr::builtin("bool"),
                is_native: false,
                is_provider: false,
                is_static: false,
                implicit_self: None,
            }],
            source_span: None,
        },
    );
    file.type_table.push(TypeDeclIr {
        name: CALLBACK_RECEIVER_SYMBOL.to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.type_table.push(TypeDeclIr {
        name: CALLBACK_INTERFACE_SYMBOL.to_string(),
        descriptor: TypeDescriptorIr::Interface,
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.executables.push(callback_checkpoint_executable(
        format!("{symbol}_callback"),
        callback_interface,
        callback_method_abi_id,
    ));
    if include_owner_method {
        assert_eq!(
            file.executables.len(),
            owner_executable_index as usize,
            "callback fixture owner executable index must match its admitted method table"
        );
        file.executables.push(ExecutableIr {
            kind: ExecutableKind::ImplMethod,
            symbol: format!("{CALLBACK_INTERFACE_SYMBOL}.{CALLBACK_INTERFACE_METHOD}"),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("bool"),
            self_type: Some(TypeRefIr::LocalType { type_index: 0 }),
            slots: SlotLayout::default(),
            may_suspend: owner_may_suspend,
            // Intentional callback-owner missing-entry probe; validation/execution must fail
            // closed instead of inventing an owner-method body.
            body: ExecutableBody::default(),
            source_span: None,
        });
    }
}

fn checkpoint_call_executable(
    symbol: String,
    target: CallTargetIr,
    args: Vec<ExprRefIr>,
    type_args: BTreeMap<String, TypeRefIr>,
) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol,
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("bool"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr { expression: 0 }),
            }],
            expressions: vec![ExprIr::Call {
                call: CallIr {
                    target,
                    site: fixture_instruction_site(),
                    args,
                    type_args,
                    metadata: BTreeMap::new(),
                },
            }],
        },
        source_span: None,
    }
}

fn package_stream_consumer_executable(symbol: String, target: CallTargetIr) -> ExecutableIr {
    let mut executable = ExecutableIr {
        kind: ExecutableKind::Function,
        symbol,
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("void"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: Vec::new(),
            statements: Vec::new(),
            expressions: vec![ExprIr::Call {
                call: CallIr {
                    target,
                    site: fixture_instruction_site(),
                    args: Vec::new(),
                    type_args: BTreeMap::from([("T".to_string(), TypeRefIr::builtin("bool"))]),
                    metadata: BTreeMap::new(),
                },
            }],
        },
        source_span: None,
    };
    configure_boolean_stream_consumer_entry(&mut executable, 0);
    executable
}

fn callback_checkpoint_executable(
    symbol: String,
    interface: InterfaceInstantiationRef,
    method_abi_id: String,
) -> ExecutableIr {
    let mut executable = checkpoint_call_executable(
        symbol,
        CallTargetIr::InterfaceMethod {
            interface: interface.clone(),
            method_abi_id,
            slot: 0,
        },
        vec![ExprRefIr { expression: 0 }],
        BTreeMap::new(),
    );
    executable.params.push(ParamIr {
        name: "callback".to_string(),
        slot: 0,
        ty: TypeRefIr::AnyInterface { interface },
    });
    executable.slots = SlotLayout {
        slots: vec![SlotIr {
            index: 0,
            name: "callback".to_string(),
            kind: SlotKind::Param,
        }],
        frame_size: 1,
    };
    executable
        .body
        .expressions
        .insert(0, ExprIr::LoadSlot { slot: 0 });
    executable.body.statements[0] = StmtIr::Expr {
        value: ExprRefIr { expression: 1 },
    };
    executable
}

pub(super) fn callback_interface_ref() -> InterfaceInstantiationRef {
    skiff_artifact_identity::interface_instantiation_ref(
        TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: IMPLEMENTATION_MODULE_PATH.to_string(),
                symbol: CALLBACK_INTERFACE_SYMBOL.to_string(),
            },
        },
        Vec::new(),
    )
}

fn provider_callback_interface_ref(provider_abi: &str) -> InterfaceInstantiationRef {
    skiff_artifact_identity::interface_instantiation_ref(
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "providerPackage".to_string(),
                },
                symbol_path: format!("{IMPLEMENTATION_MODULE_PATH}.{CALLBACK_INTERFACE_SYMBOL}"),
                abi_expectation: Some(provider_abi.to_string()),
            },
        },
        Vec::new(),
    )
}

fn canonical_callback_interface_ref_for(package_id: &str, abi: &str) -> InterfaceInstantiationRef {
    skiff_artifact_identity::interface_instantiation_ref(
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.to_string(),
                },
                symbol_path: format!("{IMPLEMENTATION_MODULE_PATH}.{CALLBACK_INTERFACE_SYMBOL}"),
                abi_expectation: Some(abi.to_string()),
            },
        },
        Vec::new(),
    )
}

pub(super) fn canonical_callback_interface_ref(provider_abi: &str) -> InterfaceInstantiationRef {
    canonical_callback_interface_ref_for(PROVIDER_PACKAGE_ID, provider_abi)
}

fn implementation_package(
    package_id: &str,
    public_path: &str,
    callable_id: PackageCallableId,
    file: &FileIrUnit,
    operation_contract: BoundaryOperationContract,
    schema_records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    service_dependency: Option<(ContractRequirement, ServiceCallRef)>,
    package_dependency: Option<(String, PackageArtifactRef)>,
    provider_abi: Option<&str>,
) -> PackageArtifact {
    let file_ref = file_ref(file);
    let entry = file
        .executables
        .first()
        .expect("fixture implementation must expose its entry executable");
    let may_suspend = entry.may_suspend;
    let callback_ref = match (provider_abi, &operation_contract.callbacks) {
        (Some(provider_abi), BoundaryCallbackContract::RequestScoped { .. }) => {
            provider_callback_interface_ref(provider_abi)
        }
        _ => callback_interface_ref(),
    };
    let package_signature = package_signature_from_operation(
        &operation_contract,
        &entry.type_params,
        may_suspend,
        &callback_ref,
    );
    let effects = no_effects(may_suspend);
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: Vec::new(),
        direct_return_origins: Vec::new(),
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    let mut contract_requirements = Vec::new();
    let mut service_requirements = Vec::new();
    let mut service_call_refs = Vec::new();
    if let Some((contract_requirement, service_call)) = service_dependency {
        contract_requirements.push(contract_requirement.clone());
        service_requirements.push(ServiceRequirement {
            contract_requirement,
            service_binding_slot: service_call.service_requirement_slot,
            used_operations: BTreeSet::from([service_call.contract_operation_id.clone()]),
        });
        service_call_refs.push(service_call);
    }
    let package_requirements = package_dependency
        .into_iter()
        .map(|(alias, package)| PackageRequirement {
            alias,
            package_id: package.package_id,
            exact_version: package.package_version,
            expected_local_abi: package.package_local_abi_identity,
            expected_package_build: None,
        })
        .collect();
    let schema_types = schema_records
        .values()
        .map(|record| {
            (
                record.stable_schema_key.clone(),
                PackageSchemaIndexEntry {
                    package_schema_type_id: record.package_schema_type_id.clone(),
                    public_path: Some(record.stable_schema_key.clone()),
                    nameability: ContractTypeNameability::PublicNameable,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let schema_type_links = schema_records
        .values()
        .map(|record| {
            let (type_index, declaration) = file
                .type_table
                .iter()
                .enumerate()
                .find(|(_, declaration)| {
                    matches!(
                        (
                            &declaration.descriptor,
                            &record.canonical_descriptor.descriptor
                        ),
                        (
                            TypeDescriptorIr::Record { .. },
                            ContractTypeDescriptor::Record { .. }
                        ) | (
                            TypeDescriptorIr::Interface,
                            ContractTypeDescriptor::CallbackInterface { .. }
                        )
                    )
                })
                .unwrap_or_else(|| {
                    panic!(
                        "fixture public schema {} must have an exact implementation type",
                        record.stable_schema_key
                    )
                });
            (
                record.stable_schema_key.clone(),
                TypeExport {
                    file: file_ref.clone(),
                    type_index: u32::try_from(type_index)
                        .expect("fixture type index should fit u32"),
                    symbol: declaration.name.clone(),
                    is_interface: matches!(declaration.descriptor, TypeDescriptorIr::Interface),
                    descriptor: Some(declaration.descriptor.clone()),
                    type_params: declaration.type_params.clone(),
                    interface_methods: Vec::new(),
                    actor: None,
                },
            )
        })
        .collect();
    let mut implementation_symbols = BTreeMap::new();
    let mut implementation_type_links: BTreeMap<String, TypeExport> = schema_type_links;
    for (name, declaration) in &file.declarations.types {
        let ty = file
            .type_table
            .get(declaration.type_index as usize)
            .unwrap_or_else(|| {
                panic!(
                    "fixture implementation type {}.{} must target an exact type table entry",
                    file.module_path, name
                )
            });
        let source_path = format!("{}.{}", file.module_path, name);
        let interface = file.declarations.interfaces.get(name);
        let interface_methods: Vec<InterfaceMethodSignature> = interface
            .map(|interface| {
                interface
                    .operations
                    .iter()
                    .map(|operation| InterfaceMethodSignature {
                        name: operation.name.clone(),
                        type_params: operation.type_params.clone(),
                        params: operation.params.clone(),
                        return_type: operation.return_type.clone(),
                        is_native: operation.is_native,
                        is_provider: operation.is_provider,
                        is_static: operation.is_static,
                        implicit_self: operation.implicit_self.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if implementation_symbols
            .insert(
                source_path.clone(),
                PackageLocalAbiSymbol::Type {
                    local_type_id: format!("type:{package_id}:top-level:{source_path}"),
                    descriptor: ty.descriptor.clone(),
                    is_alias: false,
                    is_interface: interface.is_some(),
                    type_params: ty.type_params.clone(),
                    interface_methods: interface_methods.clone(),
                    actor: None,
                },
            )
            .is_some()
        {
            panic!("fixture implementation package has duplicate type source path {source_path}");
        }
        implementation_type_links.insert(
            source_path.clone(),
            TypeExport {
                file: file_ref.clone(),
                type_index: declaration.type_index,
                symbol: declaration.symbol.clone(),
                is_interface: interface.is_some(),
                descriptor: Some(ty.descriptor.clone()),
                type_params: ty.type_params.clone(),
                interface_methods,
                actor: None,
            },
        );
    }
    let mut package = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file_ref.clone()],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::from([(
                public_path.to_string(),
                PackageLocalAbiSymbol::Callable {
                    callable_id: callable_id.clone(),
                    signature: package_signature,
                },
            )]),
            implementation_symbols,
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &schema_types,
            )
            .expect("fixture Package schema index should be canonical"),
        },
        package_schema_type_records: schema_records
            .values()
            .map(|record| {
                (
                    record.package_schema_type_id.clone(),
                    PackageSchemaTypeRecordRef {
                        package_id: record.package_id.clone(),
                        package_schema_type_id: record.package_schema_type_id.clone(),
                    },
                )
            })
            .collect(),
        implementation_links: PackageImplementationLinks {
            types: implementation_type_links,
            functions: BTreeMap::from([(
                public_path.to_string(),
                ExecutableExport {
                    file: file_ref.clone(),
                    executable_index: 0,
                    symbol: entry.symbol.clone(),
                    signature: ExecutableSignatureIr {
                        params: entry.params.clone(),
                        return_type: entry.return_type.clone(),
                        self_type: entry.self_type.clone(),
                        may_suspend: entry.may_suspend,
                    },
                },
            )]),
            ..PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::from([(
            callable_id.clone(),
            PackageCallableLinkFact {
                callable_id: callable_id.clone(),
                target: OperationTargetRef {
                    file_ref,
                    executable_index: 0,
                    callable_abi_id: callable_id.to_string(),
                    callable_kind: OperationCallableKind::PublicFunction,
                },
            },
        )]),
        package_requirements,
        contract_requirements,
        service_requirements,
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::from([(
            callable_id.clone(),
            CallableSemanticFacts {
                effects: CallableEffectSummary::Analyzed {
                    effects: effects.clone(),
                },
                provenance: provenance.clone(),
                resolved_call_targets: BTreeMap::new(),
            },
        )]),
        boundary_projections: BTreeMap::from([(
            callable_id,
            BoundaryCallableProjection::Available {
                operation_contract,
                implementation_requirements: BoundaryImplementationRequirements {
                    config: Vec::new(),
                    state: Vec::new(),
                    native_capabilities: Vec::new(),
                    complete_may_effects: effects,
                    provenance,
                },
            },
        )]),
        service_call_refs,
        bytecode: None,
    };
    skiff_artifact_identity::assign_package_artifact_identities(&mut package)
        .expect("fixture package should receive canonical identities");
    package
}

fn package_signature_from_operation(
    operation: &BoundaryOperationContract,
    type_params: &[String],
    may_suspend: bool,
    callback_ref: &InterfaceInstantiationRef,
) -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: type_params.to_vec(),
        parameters: operation
            .parameters
            .iter()
            .map(|parameter| PackageCallableParameter {
                name: parameter.name.clone(),
                ty: package_type_from_contract(&parameter.ty, callback_ref),
            })
            .collect(),
        return_type: match &operation.stream {
            BoundaryStreamContract::Unary => {
                package_type_from_contract(&operation.return_value.ty, callback_ref)
            }
            BoundaryStreamContract::ServerStream { item_type, .. } => PackageTypeRef::Container {
                name: "Stream".to_string(),
                arguments: vec![package_type_from_contract(item_type, callback_ref)],
            },
            BoundaryStreamContract::Unsupported { .. } => {
                panic!("available fixture operation cannot contain an unsupported stream")
            }
        },
        may_suspend,
    }
}

fn package_type_from_contract(
    ty: &ContractTypeRef,
    callback_ref: &InterfaceInstantiationRef,
) -> PackageTypeRef {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => PackageTypeRef::Container {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(|ty| package_type_from_contract(ty, callback_ref))
                .collect(),
        },
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => PackageTypeRef::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => PackageTypeRef::AnyInterface {
            interface: Box::new(package_type_from_contract(interface, callback_ref)),
            arguments: arguments
                .iter()
                .map(|ty| package_type_from_contract(ty, callback_ref))
                .collect(),
        },
        ContractTypeRef::Nullable { inner } => PackageTypeRef::Nullable {
            inner: Box::new(package_type_from_contract(inner, callback_ref)),
        },
        ContractTypeRef::TypeParam { .. }
        | ContractTypeRef::Record { .. }
        | ContractTypeRef::StructuralUnion { .. }
        | ContractTypeRef::Literal { .. } => PackageTypeRef::Local {
            local_type: file_type_from_contract(ty, callback_ref),
        },
    }
}

fn unary_contract() -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("bool"),
            value_plan: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Provider,
                lifetime: BoundaryValueLifetime::Call,
            },
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
    }
}

fn boolean_stream_contract() -> BoundaryOperationContract {
    let mut contract = unary_contract();
    contract.return_value.ty = ContractTypeRef::builtin("void");
    contract.return_value.value_plan = BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Provider,
        lifetime: BoundaryValueLifetime::Call,
    };
    contract.stream = BoundaryStreamContract::ServerStream {
        item_type: ContractTypeRef::builtin("bool"),
        item_value_plan: BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            owner: BoundaryValueOwner::Provider,
            lifetime: BoundaryValueLifetime::Stream,
        },
    };
    contract
}

fn no_effects(may_suspend: bool) -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: may_suspend,
    
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}

        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}

fn file_ref(file: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    }
}

fn contract_ref(contract: &ServiceContract) -> ServiceContractRef {
    ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn package_ref(package: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}
