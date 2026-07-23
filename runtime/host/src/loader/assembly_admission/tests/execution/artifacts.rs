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
const CALLBACK_OWNER_EXECUTABLE_INDEX: u32 = 3;
const CALLBACK_STREAM_OWNER_EXECUTABLE_INDEX: u32 = 2;

#[derive(Clone, Copy)]
enum ProviderBehavior {
    ReturnTrue,
    ReturnNull,
    ThrowTypedError,
    InvokeCallback,
    EmitCallbackStream,
}

#[derive(Clone, Copy)]
enum ConsumerBehavior {
    ReturnCall,
    InvokeCallback,
    ConsumeCallbackStream { break_after_item: bool },
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
    consumer_boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
    provider_operation: BoundaryOperationContract,
    provider_boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
    consumer_behavior: ConsumerBehavior,
    provider_behavior: ProviderBehavior,
}

impl TypedExecutionContract {
    fn with_provider_behavior(
        operation: BoundaryOperationContract,
        boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
        provider_behavior: ProviderBehavior,
    ) -> Self {
        Self {
            consumer_operation: operation.clone(),
            consumer_boundary_schema: boundary_schema.clone(),
            provider_operation: operation,
            provider_boundary_schema: boundary_schema,
            consumer_behavior: ConsumerBehavior::ReturnCall,
            provider_behavior,
        }
    }

    pub(super) fn unary() -> Self {
        Self::with_provider_behavior(
            unary_contract(),
            BTreeMap::new(),
            ProviderBehavior::ReturnTrue,
        )
    }

    pub(super) fn returning_null(
        operation: BoundaryOperationContract,
        boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
    ) -> Self {
        Self::with_provider_behavior(operation, boundary_schema, ProviderBehavior::ReturnNull)
    }

    pub(super) fn async_typed_error() -> Self {
        let service_id = "example.phase-four.provider";
        let contract_version = "1.0.0";
        let stable_key = "asyncError";
        let payload_type_id =
            skiff_artifact_identity::contract_type_id(service_id, contract_version, stable_key)
                .expect("async error fixture ContractTypeId should be canonical");
        let payload_fields = BTreeMap::from([(
            "messages".to_string(),
            ContractTypeRef::Builtin {
                name: "Array".to_string(),
                arguments: vec![ContractTypeRef::builtin("string")],
            },
        )]);
        let mut operation = unary_contract();
        operation.errors = BoundaryErrorContract::Typed {
            payload_type: ContractTypeRef::contract(payload_type_id.clone()),
            value_plan: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Provider,
                lifetime: BoundaryValueLifetime::Call,
            },
        };
        operation.cancellation = BoundaryCancellationContract::Cooperative;
        operation.may_suspend = true;
        let provider_boundary_schema = BTreeMap::from([(
            payload_type_id.clone(),
            ContractSchemaType {
                contract_type_id: payload_type_id,
                stable_key: stable_key.to_string(),
                shape: ContractTypeShape {
                    nameability: ContractTypeNameability::PublicNameable,
                    descriptor: ContractTypeDescriptor::Record {
                        fields: payload_fields,
                    },
                },
            },
        )]);
        Self {
            consumer_operation: unary_contract(),
            consumer_boundary_schema: BTreeMap::new(),
            provider_operation: operation,
            provider_boundary_schema,
            consumer_behavior: ConsumerBehavior::ReturnCall,
            provider_behavior: ProviderBehavior::ThrowTypedError,
        }
    }

    pub(super) fn callback() -> Self {
        Self::callback_with_operation_key(CALLBACK_INTERFACE_METHOD)
    }

    pub(super) fn callback_with_operation_key(contract_operation: &str) -> Self {
        let service_id = "example.phase-four.provider";
        let contract_version = "1.0.0";
        let stable_key = "callbackProbe";
        let callback_type_id =
            skiff_artifact_identity::contract_type_id(service_id, contract_version, stable_key)
                .expect("callback fixture ContractTypeId should be canonical");
        let callback_type = ContractTypeRef::contract(callback_type_id.clone());
        let callback_plan = BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::CallbackCapability,
            encoding: BoundaryValueEncoding::OpaqueCapability,
            owner: BoundaryValueOwner::CapabilityOwner,
            lifetime: BoundaryValueLifetime::Request,
        };
        let mut provider_operation = unary_contract();
        provider_operation.parameters = vec![BoundaryParameter {
            name: "callback".to_string(),
            ty: callback_type,
            value_plan: callback_plan,
        }];
        provider_operation.callbacks = BoundaryCallbackContract::RequestScoped {
            interface_type_ids: vec![callback_type_id.clone()],
            lifetime: BoundaryCallbackLifetime::TopLevelRequest,
            expiration_error: BoundaryCallbackExpirationError::CapabilityUnavailable,
        };
        let provider_boundary_schema = BTreeMap::from([(
            callback_type_id.clone(),
            ContractSchemaType {
                contract_type_id: callback_type_id,
                stable_key: stable_key.to_string(),
                shape: ContractTypeShape {
                    nameability: ContractTypeNameability::PublicNameable,
                    descriptor: ContractTypeDescriptor::CallbackInterface {
                        operations: BTreeMap::from([(
                            contract_operation.to_string(),
                            BoundaryCallbackOperation {
                                parameters: Vec::new(),
                                return_type: ContractTypeRef::builtin("bool"),
                                may_suspend: false,
                            },
                        )]),
                    },
                },
            },
        )]);
        Self {
            consumer_operation: unary_contract(),
            consumer_boundary_schema: BTreeMap::new(),
            provider_operation,
            provider_boundary_schema,
            consumer_behavior: ConsumerBehavior::InvokeCallback,
            provider_behavior: ProviderBehavior::InvokeCallback,
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

    pub(super) fn callback_stream_wrong_tuple() -> Self {
        let mut fixture = Self::callback_stream();
        let ContractTypeDescriptor::CallbackInterface { operations } = &mut fixture
            .provider_boundary_schema
            .values_mut()
            .next()
            .expect("callback stream schema should contain its interface")
            .shape
            .descriptor
        else {
            panic!("callback stream schema should retain its callback descriptor")
        };
        operations
            .get_mut(CALLBACK_INTERFACE_METHOD)
            .expect("callback stream descriptor should contain invoke")
            .return_type = ContractTypeRef::builtin("string");
        fixture
    }

    pub(super) fn callback_stream_with_operation_key(contract_operation: &str) -> Self {
        let service_id = "example.phase-four.provider";
        let contract_version = "1.0.0";
        let stable_key = "callbackProbe";
        let callback_type_id =
            skiff_artifact_identity::contract_type_id(service_id, contract_version, stable_key)
                .expect("callback stream fixture ContractTypeId should be canonical");
        let callback_type = ContractTypeRef::contract(callback_type_id.clone());
        let callback_plan = BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::CallbackCapability,
            encoding: BoundaryValueEncoding::OpaqueCapability,
            owner: BoundaryValueOwner::CapabilityOwner,
            lifetime: BoundaryValueLifetime::Stream,
        };
        let mut provider_operation = unary_contract();
        provider_operation.stream = BoundaryStreamContract::ServerStream {
            item_type: callback_type,
            item_value_plan: callback_plan,
        };
        provider_operation.callbacks = BoundaryCallbackContract::RequestScoped {
            interface_type_ids: vec![callback_type_id.clone()],
            lifetime: BoundaryCallbackLifetime::Stream,
            expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
        };
        provider_operation.cancellation = BoundaryCancellationContract::Cooperative;
        provider_operation.may_suspend = true;
        let provider_boundary_schema = BTreeMap::from([(
            callback_type_id.clone(),
            ContractSchemaType {
                contract_type_id: callback_type_id,
                stable_key: stable_key.to_string(),
                shape: ContractTypeShape {
                    nameability: ContractTypeNameability::PublicNameable,
                    descriptor: ContractTypeDescriptor::CallbackInterface {
                        operations: BTreeMap::from([(
                            contract_operation.to_string(),
                            BoundaryCallbackOperation {
                                parameters: Vec::new(),
                                return_type: ContractTypeRef::builtin("bool"),
                                may_suspend: false,
                            },
                        )]),
                    },
                },
            },
        )]);
        Self {
            consumer_operation: unary_contract(),
            consumer_boundary_schema: BTreeMap::new(),
            provider_operation,
            provider_boundary_schema,
            consumer_behavior: ConsumerBehavior::ConsumeCallbackStream {
                break_after_item: false,
            },
            provider_behavior: ProviderBehavior::EmitCallbackStream,
        }
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
        let consumer_boundary_schema = contract_fixture.consumer_boundary_schema;
        let provider_operation_contract = contract_fixture.provider_operation;
        let provider_boundary_schema = contract_fixture.provider_boundary_schema;
        let consumer_behavior = contract_fixture.consumer_behavior;
        let provider_behavior = contract_fixture.provider_behavior;
        let (provider_contract, provider_operation) = service_contract(
            "example.phase-four.provider",
            "provide",
            provider_operation_contract.clone(),
            provider_boundary_schema,
        );
        let provider_contract_ref = contract_ref(&provider_contract);
        let (consumer_contract, consumer_operation) = service_contract(
            consumer_service_id,
            "consume",
            consumer_operation_contract.clone(),
            consumer_boundary_schema,
        );
        let consumer_contract_ref = contract_ref(&consumer_contract);

        let provider_callable = PackageCallableId::new("callable:phase-four-provider");
        let provider_file = implementation_file(
            "phase_four.provider",
            "provide",
            provider_operation_contract.may_suspend,
            ImplementationRole::Provider(provider_behavior),
        );
        let provider_file_ref = file_ref(&provider_file);
        let provider_package = implementation_package(
            "example.phase-four-provider",
            "provide",
            provider_callable.clone(),
            &provider_file,
            provider_operation_contract,
            None,
            None,
        );
        let provider_package_ref = package_ref(&provider_package);

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
            "phase_four.consumer",
            "consume",
            consumer_operation_contract.may_suspend,
            ImplementationRole::Consumer {
                service_call: service_call.clone(),
                package_call: ("providerPackage".to_string(), provider_callable.clone()),
                behavior: consumer_behavior,
            },
        );
        let consumer_file_ref = file_ref(&consumer_file);
        let consumer_file_ir_identity = consumer_file_ref.file_ir_identity.clone();
        let consumer_package = implementation_package(
            "example.phase-four-consumer",
            "consume",
            PackageCallableId::new("callable:phase-four-consumer"),
            &consumer_file,
            consumer_operation_contract,
            Some((provider_requirement, service_call)),
            Some(("providerPackage".to_string(), provider_package_ref.clone())),
        );
        let consumer_package_ref = package_ref(&consumer_package);

        let provider_deployment_artifact = project_service_deployment(
            deployment_input(
                provider_contract_ref.clone(),
                DeploymentRevision::new("phase-four-provider-r1"),
                provider_package_ref.clone(),
                provider_operation.clone(),
                "provide",
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            &provider_contract,
            std::slice::from_ref(&provider_package),
        )
        .expect("provider deployment should project from typed contract/package artifacts");
        let provider_deployment =
            skiff_artifact_identity::service_deployment_ref(&provider_deployment_artifact);
        let ingress = DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                host: "phase-four.test".to_string(),
                method: Some("POST".to_string()),
                path: "/consume".to_string(),
            },
            contract_operation_id: consumer_operation.clone(),
        };
        let consumer_deployment_artifact = project_service_deployment(
            deployment_input(
                consumer_contract_ref.clone(),
                DeploymentRevision::new("phase-four-consumer-r1"),
                consumer_package_ref.clone(),
                consumer_operation,
                "consume",
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
                vec![ingress],
            ),
            &consumer_contract,
            &[consumer_package.clone(), provider_package.clone()],
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
        }
    }
}

fn deployment_input(
    contract: ServiceContractRef,
    deployment_revision: DeploymentRevision,
    implementation: PackageArtifactRef,
    operation: ContractOperationId,
    public_path: &str,
    package_bindings: Vec<PackageBinding>,
    service_selectors: Vec<ServiceSelectorBinding>,
    ingress: Vec<DeploymentIngressBinding>,
) -> ServiceDeploymentInput {
    ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision,
        implementation,
        operation_bindings: vec![ServiceDeploymentOperationInput {
            contract_operation_id: operation,
            package_public_path: public_path.to_string(),
        }],
        package_bindings,
        service_selectors,
        ingress,
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        state_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        runtime_capability_bindings: Vec::new(),
        policy: policy(),
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
    boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
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
        boundary_schema,
        diagnostic_text: ContractDiagnosticText {
            service: "Phase four typed execution fixture".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract)
        .expect("fixture contract should receive canonical identities");
    (contract, operation_id)
}

fn implementation_file(
    module_path: &str,
    symbol: &str,
    may_suspend: bool,
    role: ImplementationRole,
) -> FileIrUnit {
    let mut file = FileIrUnit::empty(module_path, format!("source:{module_path}"));
    let mut entry = ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::native("bool"),
        self_type: None,
        slots: SlotLayout::default(),
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
            configure_provider_entry(&mut file, &mut entry, module_path, behavior);
            file.executables.push(entry);
            install_provider_support(&mut file, module_path, symbol, behavior);
        }
        ImplementationRole::Consumer {
            service_call,
            package_call,
            behavior,
        } => {
            configure_consumer_entry(&mut file, &mut entry, module_path, service_call, behavior);
            file.executables.push(entry);
            install_consumer_support(&mut file, module_path, symbol, package_call, behavior);
        }
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("fixture File IR should receive a canonical identity");
    file
}

fn configure_provider_entry(
    file: &mut FileIrUnit,
    entry: &mut ExecutableIr,
    module_path: &str,
    behavior: ProviderBehavior,
) {
    match behavior {
        ProviderBehavior::ReturnTrue => configure_return_true_entry(entry),
        ProviderBehavior::ReturnNull => configure_return_null_provider_entry(entry),
        ProviderBehavior::ThrowTypedError => {
            configure_async_typed_error_provider_entry(file, entry, module_path);
        }
        ProviderBehavior::InvokeCallback => configure_callback_provider_entry(entry),
        ProviderBehavior::EmitCallbackStream => configure_callback_stream_provider_entry(
            entry,
            module_path,
            CALLBACK_STREAM_OWNER_EXECUTABLE_INDEX,
        ),
    }
}

fn configure_consumer_entry(
    file: &mut FileIrUnit,
    entry: &mut ExecutableIr,
    module_path: &str,
    service_call: ServiceCallRef,
    behavior: ConsumerBehavior,
) {
    file.external_refs.service_call_refs.push(service_call);
    let call_args = match behavior {
        ConsumerBehavior::InvokeCallback => append_callback_preimage(entry, module_path),
        ConsumerBehavior::ReturnCall | ConsumerBehavior::ConsumeCallbackStream { .. } => Vec::new(),
    };
    let call_expression = u32::try_from(entry.body.expressions.len())
        .expect("fixture expression count should fit u32");
    entry.body.expressions.push(ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::ServiceCall {
                service_call_ref_index: ServiceCallRefIndex::new(0),
            },
            args: call_args,
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        },
    });
    match behavior {
        ConsumerBehavior::ReturnCall | ConsumerBehavior::InvokeCallback => {
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
            configure_callback_stream_consumer_entry(entry, call_expression, break_after_item);
        }
    }
}

fn install_provider_support(
    file: &mut FileIrUnit,
    module_path: &str,
    symbol: &str,
    behavior: ProviderBehavior,
) {
    if matches!(behavior, ProviderBehavior::EmitCallbackStream) {
        install_callback_interface_fixture(
            file,
            module_path,
            symbol,
            true,
            CALLBACK_STREAM_OWNER_EXECUTABLE_INDEX,
        );
        configure_return_true_entry(
            file.executables
                .last_mut()
                .expect("callback stream owner executable should be installed"),
        );
    }
}

fn install_consumer_support(
    file: &mut FileIrUnit,
    module_path: &str,
    symbol: &str,
    package_call: (String, PackageCallableId),
    behavior: ConsumerBehavior,
) {
    let (dependency_ref, package_callable_id) = package_call;
    let package_ref = PackageRefIr::Dependency { dependency_ref };
    file.external_refs
        .package_callables
        .push(PackageCallableRef {
            package_ref: package_ref.clone(),
            package_callable_id: package_callable_id.clone(),
        });
    file.executables.push(checkpoint_call_executable(
        format!("{symbol}_package_direct"),
        CallTargetIr::PackageCallable {
            package_ref,
            package_callable_id,
        },
        Vec::new(),
    ));
    install_callback_interface_fixture(
        file,
        module_path,
        symbol,
        matches!(behavior, ConsumerBehavior::InvokeCallback),
        CALLBACK_OWNER_EXECUTABLE_INDEX,
    );
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
        TypeRefIr::Native {
            name: "Array".to_string(),
            args: vec![TypeRefIr::native("string")],
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
        discriminator: None,
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
    module_path: &str,
    owner_executable_index: u32,
) {
    let callback_interface = callback_interface_ref(module_path);
    entry.return_type = TypeRefIr::Native {
        name: "Stream".to_string(),
        args: vec![TypeRefIr::AnyInterface {
            interface: callback_interface,
        }],
    };
    let callback = append_callback_preimage_at(entry, module_path, owner_executable_index);
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
) {
    let callback_interface = callback_interface_ref("phase_four.provider");
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

fn configure_callback_provider_entry(entry: &mut ExecutableIr) {
    let callback_interface = callback_interface_ref("phase_four.consumer");
    let callback_method_abi_id = skiff_artifact_identity::canonical_interface_method_abi_id(
        &callback_interface,
        CALLBACK_INTERFACE_METHOD,
    );
    entry.params.push(ParamIr {
        name: "callback".to_string(),
        slot: 0,
        ty: TypeRefIr::AnyInterface {
            interface: callback_interface.clone(),
        },
    });
    entry.slots = SlotLayout {
        slots: vec![SlotIr {
            index: 0,
            name: "callback".to_string(),
            kind: SlotKind::Param,
        }],
        frame_size: 1,
    };
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
                    args: vec![ExprRefIr { expression: 0 }],
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            },
        ],
    };
}

fn append_callback_preimage(entry: &mut ExecutableIr, module_path: &str) -> Vec<ExprRefIr> {
    append_callback_preimage_at(entry, module_path, CALLBACK_OWNER_EXECUTABLE_INDEX)
}

fn append_callback_preimage_at(
    entry: &mut ExecutableIr,
    module_path: &str,
    owner_executable_index: u32,
) -> Vec<ExprRefIr> {
    let callback_interface = callback_interface_ref(module_path);
    let callback_method_abi_id = skiff_artifact_identity::canonical_interface_method_abi_id(
        &callback_interface,
        CALLBACK_INTERFACE_METHOD,
    );
    let concrete_type = TypeRefIr::LocalType { type_index: 0 };
    entry.body.expressions.extend([
        ExprIr::Literal {
            value: LiteralIr::Bool { value: true },
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
                            return_type: TypeRefIr::native("bool"),
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
) {
    let callback_interface = callback_interface_ref(module_path);
    let callback_method_abi_id = skiff_artifact_identity::canonical_interface_method_abi_id(
        &callback_interface,
        CALLBACK_INTERFACE_METHOD,
    );
    file.declarations.types.insert(
        CALLBACK_INTERFACE_SYMBOL.to_string(),
        TypeDeclarationIr {
            type_index: 0,
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
                    ty: TypeRefIr::native("Self"),
                }],
                return_type: TypeRefIr::native("bool"),
                is_native: false,
                is_provider: false,
                is_static: false,
                implicit_self: None,
            }],
            source_span: None,
        },
    );
    file.type_table.push(TypeDeclIr {
        name: CALLBACK_INTERFACE_SYMBOL.to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        discriminator: None,
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
            return_type: TypeRefIr::native("bool"),
            self_type: Some(TypeRefIr::LocalType { type_index: 0 }),
            slots: SlotLayout::default(),
            may_suspend: false,
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
) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol,
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::native("bool"),
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
                    args,
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            }],
        },
        source_span: None,
    }
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

pub(super) fn callback_interface_ref(module_path: &str) -> InterfaceInstantiationRef {
    skiff_artifact_identity::interface_instantiation_ref(
        TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: module_path.to_string(),
                symbol: CALLBACK_INTERFACE_SYMBOL.to_string(),
            },
        },
        Vec::new(),
    )
}

fn implementation_package(
    package_id: &str,
    public_path: &str,
    callable_id: PackageCallableId,
    file: &FileIrUnit,
    operation_contract: BoundaryOperationContract,
    service_dependency: Option<(ContractRequirement, ServiceCallRef)>,
    package_dependency: Option<(String, PackageArtifactRef)>,
) -> PackageArtifact {
    let file_ref = file_ref(file);
    let may_suspend = operation_contract.may_suspend;
    let effects = no_effects(may_suspend);
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: Vec::new(),
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
        })
        .collect();
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
                    signature: PackageCallableSignature {
                        parameters: Vec::new(),
                        return_type: PackageTypeRef::Local {
                            local_type: TypeRefIr::native("bool"),
                        },
                        throw_types: Vec::new(),
                        may_suspend,
                    },
                },
            )]),
        },
        implementation_links: PackageImplementationLinks::default(),
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
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
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
                    runtime_capabilities: Vec::new(),
                    complete_may_effects: effects,
                    provenance,
                },
            },
        )]),
        service_call_refs,
    };
    skiff_artifact_identity::assign_package_artifact_identities(&mut package)
        .expect("fixture package should receive canonical identities");
    package
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
    }
}

fn no_effects(may_suspend: bool) -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend,
    }
}

fn policy() -> DeploymentPolicy {
    DeploymentPolicy {
        timeout_ms: 1_000,
        resources: ResourcePolicy {
            cpu_millis: 100,
            memory_bytes: 1_048_576,
        },
        activation: ActivationPolicy {
            max_concurrency: 4,
            idle_timeout_ms: None,
        },
        principal: "service:phase-four-fixture".to_string(),
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
