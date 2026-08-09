use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    ActivationTemplate, ActorAbiInput, ActorDeclarationIr, ActorFieldEncodingIr, ActorFieldIr,
    ActorImplementationIdentity, ActorMethodIdentity, ActorPublicMethodIr,
    BoundaryCallableProjection, BoundaryCallbackContract, BoundaryEffectGuarantee,
    BoundaryImplementationRequirements, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, CallIr, CallTargetIr,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts,
    CanonicalPackageLinkPlan, ContractDiagnosticText, ContractOperationId, ContractRequirement,
    DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentGatewayEntry,
    DeploymentIngressBinding, DeploymentOperationBinding, DeploymentRevision, ExecutableBody,
    ExecutableExport, ExecutableIr, ExecutableKind, ExecutableSignatureIr, ExprIr, ExprRefIr,
    FileIrRef, FileIrUnit, FunctionTypeParamIr, GatewayAdapterArg, GatewayAdapterKind,
    GatewayAdapterPlan, GatewayAdapterSource, GatewayDispatchMode, GatewayEntryIdentity,
    GatewayEntryKey, GatewayEntryProtocolSurface, GatewayExternalErrorProjection,
    GatewayExternalSchema, GatewayHttpProtocolSurface, GatewayIngressBinding,
    GatewayProtocolSurface, IngressProtocol, IngressSelector, InstructionSourceSite,
    InterfaceDeclIr, InterfaceMethodSignature, InterfaceOperationIr, OperationCallableKind,
    OperationTargetRef, PackageArtifact, PackageArtifactRef, PackageBinding, PackageBuildId,
    PackageCallableId, PackageCallableLinkFact, PackageCallableParameter, PackageCallableRef,
    PackageCallableSignature, PackageCodeSlot, PackageImplementationLinks, PackageLocalAbi,
    PackageLocalAbiIdentity, PackageLocalAbiSymbol, PackageRefIr, PackageRequirement,
    PackageRequirementKey, PackageRuntimeRequirements, PackageSchemaIndex, PackageSchemaIndexRef,
    PackageTypeRef, PublicationResourceRef, ResolvedServiceBinding, RuntimeAssembly,
    ServiceBindingTemplate, ServiceCallRef, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentRef, ServiceProtocolIdentity, ServiceRequirement, ServiceRequirementKey,
    ServiceSelectorBinding, SlotLayout, SyntheticInstructionSiteReason, TypeDeclIr,
    TypeDeclarationIr, TypeDescriptorIr, TypeExport, TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_runtime_loader::RuntimeAssemblyContentResolver;

fn test_instruction_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

pub(super) struct CycleFixture {
    pub assembly: RuntimeAssembly,
    pub resolver: FixtureResolver,
    pub operation_id: ContractOperationId,
    pub contract_ref: ServiceContractRef,
    pub service_callable: PackageCallableId,
    pub helper_callable: PackageCallableId,
    pub shared_build: PackageBuildId,
    pub helper_build: PackageBuildId,
    pub helper_abi: PackageLocalAbiIdentity,
    pub shared_file_identity: String,
    pub activation_a: ServiceDeploymentRef,
    pub activation_b: ServiceDeploymentRef,
    pub ingress_selector: IngressSelector,
    pub ingress_alias_selector: IngressSelector,
    pub gateway_handler: PackageCallableId,
    pub gateway_pre: PackageCallableId,
    pub gateway_guard: PackageCallableId,
    pub gateway_entry_key: GatewayEntryKey,
    pub gateway_entry_identity: GatewayEntryIdentity,
}

impl CycleFixture {
    pub fn new() -> Self {
        let service_id = "example.cycle";
        let contract_version = "1.0.0";
        let operation_id =
            skiff_artifact_identity::contract_operation_id(service_id, contract_version, "call")
                .unwrap();
        let operation_contract = operation_contract();
        let mut contract = ServiceContract {
            schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: service_id.to_string(),
            contract_version: contract_version.to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
            operations: BTreeMap::from([(
                operation_id.clone(),
                BoundaryOperationDescriptor {
                    operation_id: operation_id.clone(),
                    stable_key: "call".to_string(),
                    contract: operation_contract.clone(),
                },
            )]),
            package_type_requirements: Vec::new(),
            diagnostic_text: ContractDiagnosticText {
                service: "Cycle".to_string(),
                operations: BTreeMap::from([(operation_id.clone(), "Call".to_string())]),
                types: BTreeMap::new(),
            },
        };
        skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
        let second_operation_id = skiff_artifact_identity::contract_operation_id(
            "example.cycle-secondary",
            contract_version,
            "call",
        )
        .unwrap();
        let mut second_contract = contract.clone();
        second_contract.service_id = "example.cycle-secondary".to_string();
        let mut second_descriptor = second_contract.operations.pop_first().unwrap().1;
        second_descriptor.operation_id = second_operation_id.clone();
        second_contract.operations =
            BTreeMap::from([(second_operation_id.clone(), second_descriptor)]);
        second_contract.diagnostic_text.service = "Cycle secondary".to_string();
        second_contract.diagnostic_text.operations =
            BTreeMap::from([(second_operation_id.clone(), "Call".to_string())]);
        skiff_artifact_identity::assign_service_contract_identities(&mut second_contract).unwrap();
        let second_contract_ref = contract_ref(&second_contract);
        let contract_ref = contract_ref(&contract);

        let helper_callable = PackageCallableId::new("pkg-callable:example.helper:entry");
        let mut helper_file = file("helper.main");
        helper_file.declarations.types.insert(
            "LocalRecord".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "helper.main.LocalRecord".to_string(),
                source_span: None,
            },
        );
        helper_file.declarations.db.insert(
            "LocalRecord".to_string(),
            skiff_artifact_model::DbDeclarationIr {
                type_ref: TypeRefIr::LocalType { type_index: 0 },
                type_name: "LocalRecord".to_string(),
                collection_name: Some("helper_local_record".to_string()),
                implements: None,
                identity_fields: std::collections::BTreeMap::new(),
                kind: skiff_artifact_model::DbObjectKindIr::Object,
                key: skiff_artifact_model::DbObjectKeyIr {
                    name: "id".to_string(),
                    ty: TypeRefIr::builtin("string"),
                },
                fields: Vec::new(),
                retention: None,
                leases: Vec::new(),
                indexes: Vec::new(),
                source_span: None,
            },
        );
        let helper_interface_index = helper_file.type_table.len() as u32;
        let helper_interface_operation = InterfaceOperationIr {
            name: "read".to_string(),
            type_params: Vec::new(),
            params: vec![FunctionTypeParamIr {
                name: "self".to_string(),
                ty: TypeRefIr::builtin("Self"),
            }],
            return_type: TypeRefIr::builtin("string"),
            is_native: false,
            is_provider: false,
            is_static: false,
            implicit_self: None,
        };
        helper_file.declarations.types.insert(
            "Reader".to_string(),
            TypeDeclarationIr {
                type_index: helper_interface_index,
                symbol: "helper.main.Reader".to_string(),
                source_span: None,
            },
        );
        helper_file.declarations.interfaces.insert(
            "Reader".to_string(),
            InterfaceDeclIr {
                name: "Reader".to_string(),
                type_params: Vec::new(),
                operations: vec![helper_interface_operation.clone()],
                source_span: None,
            },
        );
        helper_file.type_table.push(TypeDeclIr {
            name: "Reader".to_string(),
            descriptor: TypeDescriptorIr::Interface,
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        skiff_artifact_identity::assign_file_ir_identity(&mut helper_file).unwrap();
        let mut helper = package(
            "example.helper",
            &helper_file,
            helper_callable.clone(),
            operation_contract.clone(),
        );
        let helper_interface_methods = vec![InterfaceMethodSignature {
            name: helper_interface_operation.name.clone(),
            type_params: helper_interface_operation.type_params.clone(),
            params: helper_interface_operation.params.clone(),
            return_type: helper_interface_operation.return_type.clone(),
            is_native: helper_interface_operation.is_native,
            is_provider: helper_interface_operation.is_provider,
            is_static: helper_interface_operation.is_static,
            implicit_self: helper_interface_operation.implicit_self.clone(),
        }];
        helper.package_local_abi.public_symbols.insert(
            "Reader".to_string(),
            PackageLocalAbiSymbol::Type {
                local_type_id: "type:Reader".to_string(),
                descriptor: TypeDescriptorIr::Interface,
                is_alias: false,
                is_interface: true,
                type_params: Vec::new(),
                interface_methods: helper_interface_methods.clone(),
                actor: None,
            },
        );
        helper.package_local_abi.implementation_symbols.insert(
            "helper.main.Reader".to_string(),
            PackageLocalAbiSymbol::Type {
                local_type_id: "type:example.helper:top-level:helper.main.Reader".to_string(),
                descriptor: TypeDescriptorIr::Interface,
                is_alias: false,
                is_interface: true,
                type_params: Vec::new(),
                interface_methods: helper_interface_methods.clone(),
                actor: None,
            },
        );
        helper.implementation_links.types.insert(
            "Reader".to_string(),
            TypeExport {
                file: file_ref(&helper_file),
                type_index: helper_interface_index,
                symbol: "Reader".to_string(),
                is_interface: true,
                descriptor: Some(TypeDescriptorIr::Interface),
                type_params: Vec::new(),
                interface_methods: helper_interface_methods.clone(),
                actor: None,
            },
        );
        helper.implementation_links.types.insert(
            "helper.main.Reader".to_string(),
            TypeExport {
                file: file_ref(&helper_file),
                type_index: helper_interface_index,
                symbol: "helper.main.Reader".to_string(),
                is_interface: true,
                descriptor: Some(TypeDescriptorIr::Interface),
                type_params: Vec::new(),
                interface_methods: helper_interface_methods.clone(),
                actor: None,
            },
        );
        helper.implementation_links.types.insert(
            "helper.main.LocalRecord".to_string(),
            TypeExport {
                file: file_ref(&helper_file),
                type_index: 0,
                symbol: "helper.main.LocalRecord".to_string(),
                is_interface: false,
                descriptor: Some(TypeDescriptorIr::Record {
                    fields: BTreeMap::new(),
                }),
                type_params: Vec::new(),
                interface_methods: Vec::new(),
                actor: None,
            },
        );
        let helper_resource: Arc<[u8]> = Arc::from(b"shared helper resource".as_slice());
        helper.static_resources.push(PublicationResourceRef {
            path: "assets/helper.txt".to_string(),
            sha256: hex::encode(Sha256::digest(helper_resource.as_ref())),
            byte_len: helper_resource.len() as u64,
            content_type: Some("text/plain".to_string()),
            artifact_path: None,
        });
        skiff_artifact_identity::assign_package_artifact_identities(&mut helper).unwrap();
        let helper_ref = package_ref(&helper);

        let service_call = ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: second_operation_id.clone(),
            expected_protocol_identity: second_contract_ref.service_protocol_identity.clone(),
        };
        let service_callable = PackageCallableId::new("pkg-callable:example.shared:entry");
        let mut shared_file = file("shared.main");
        shared_file
            .external_refs
            .service_call_refs
            .push(service_call.clone());
        shared_file
            .external_refs
            .package_callables
            .push(PackageCallableRef {
                package_ref: PackageRefIr::Dependency {
                    dependency_ref: "helper".to_string(),
                },
                package_callable_id: helper_callable.clone(),
            });
        shared_file.executables[0]
            .body
            .expressions
            .push(ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::PackageCallable {
                        package_ref: PackageRefIr::Dependency {
                            dependency_ref: "helper".to_string(),
                        },
                        package_callable_id: helper_callable.clone(),
                    },
                    site: test_instruction_site(),
                    args: Vec::new(),
                    inout_args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            });
        shared_file.executables.push(ExecutableIr {
            kind: ExecutableKind::Function,
            symbol: "localHelper".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("bool"),
            self_type: None,
            slots: SlotLayout::default(),
            may_suspend: false,
            body: ExecutableBody::default(),
            expression_types: Vec::new(),
            statement_spans: Vec::new(),
            source_span: None,
        });
        shared_file.executables[0]
            .body
            .expressions
            .push(ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::LocalExecutable {
                        executable_index: 1,
                    },
                    site: test_instruction_site(),
                    args: Vec::new(),
                    inout_args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            });
        shared_file.executables[0]
            .body
            .expressions
            .push(ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::ServiceCall {
                        service_call_ref_index: skiff_artifact_model::ServiceCallRefIndex::new(0),
                    },
                    site: test_instruction_site(),
                    args: Vec::new(),
                    inout_args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            });
        let method_identity = ActorMethodIdentity::new("actor-method:submit");
        let actor_abi = ActorAbiInput {
            actor_name: "DocHub".to_string(),
            actor_id_type: TypeRefIr::builtin("string"),
            key_field: "id".to_string(),
            fields: vec![
                ActorFieldIr {
                    name: "id".to_string(),
                    ty: TypeRefIr::builtin("string"),
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                },
                ActorFieldIr {
                    name: "nextSeq".to_string(),
                    ty: TypeRefIr::builtin("number"),
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                },
            ],
            create: None,
            public_methods: vec![ActorPublicMethodIr {
                method_identity: method_identity.clone(),
                name: "submit".to_string(),
                parameters: Vec::new(),
                return_type: TypeRefIr::builtin("bool"),
                may_suspend: false,
            }],
            actor_runtime_abi_version: skiff_artifact_model::ACTOR_RUNTIME_ABI_VERSION_V1
                .to_string(),
        };
        let actor_abi_identity = skiff_artifact_identity::actor_abi_identity(&actor_abi).unwrap();
        let actor_implementation_identity = ActorImplementationIdentity::new("actor-impl:doc-hub");
        shared_file.actor_declarations.push(ActorDeclarationIr {
            actor_abi_identity: actor_abi_identity.clone(),
            actor_implementation_identity: actor_implementation_identity.clone(),
            abi: actor_abi,
            method_implementations: BTreeMap::from([(method_identity.clone(), 1)]),
            create_implementation: None,
        });
        shared_file.executables[0]
            .body
            .expressions
            .push(ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::ActorMethod {
                        actor: skiff_artifact_model::ServiceSymbolRef {
                            module_path: shared_file.module_path.clone(),
                            symbol: "DocHub".to_string(),
                        },
                        actor_abi_identity,
                        actor_implementation_identity,
                        method_identity,
                    },
                    site: test_instruction_site(),
                    args: vec![ExprRefIr { expression: 1 }],
                    inout_args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            });
        shared_file.executables[0].expression_types = vec![TypeRefIr::builtin("bool"); 4];
        skiff_artifact_identity::assign_file_ir_identity(&mut shared_file).unwrap();
        let contract_requirement = ContractRequirement {
            alias: "cycle".to_string(),
            service_id: second_contract_ref.service_id.clone(),
            contract_version: second_contract_ref.contract_version.clone(),
            expected_protocol_identity: second_contract_ref.service_protocol_identity.clone(),
        };
        let mut shared = package(
            "example.shared",
            &shared_file,
            service_callable.clone(),
            operation_contract,
        );
        shared.package_requirements.push(PackageRequirement {
            alias: "helper".to_string(),
            package_id: helper_ref.package_id.clone(),
            exact_version: helper_ref.package_version.clone(),
            expected_local_abi: helper_ref.package_local_abi_identity.clone(),
            expected_package_build: Some(helper_ref.package_build_id.clone()),
        });
        shared
            .contract_requirements
            .push(contract_requirement.clone());
        shared.service_requirements.push(ServiceRequirement {
            contract_requirement,
            service_binding_slot: 0,
            used_operations: BTreeSet::from([second_operation_id.clone()]),
        });
        shared.service_call_refs.push(service_call);
        let gateway_handler = PackageCallableId::new(
            "pkg-callable:example.shared:top-level:shared.main.gateway_handler",
        );
        let gateway_pre =
            PackageCallableId::new("pkg-callable:example.shared:top-level:shared.main.gateway_pre");
        let gateway_guard = PackageCallableId::new(
            "pkg-callable:example.shared:top-level:shared.main.gateway_guard",
        );
        for (path, callable) in [
            ("shared.main.gateway_handler", &gateway_handler),
            ("shared.main.gateway_pre", &gateway_pre),
            ("shared.main.gateway_guard", &gateway_guard),
        ] {
            add_private_gateway_callable(&mut shared, path, callable);
        }
        skiff_artifact_identity::assign_package_artifact_identities(&mut shared).unwrap();
        let shared_ref = package_ref(&shared);

        let ingress_selector = IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: "/call".to_string(),
        };
        let ingress_alias_selector = IngressSelector {
            path: "/call-alias".to_string(),
            ..ingress_selector.clone()
        };
        let mut deployment_a = deployment(
            "revision-a",
            "a",
            &contract_ref,
            &shared_ref,
            &helper_ref,
            &service_callable,
            &operation_id,
            Some((
                ingress_selector.clone(),
                gateway_handler.clone(),
                gateway_pre.clone(),
                gateway_guard.clone(),
            )),
        );
        let mut deployment_b = deployment(
            "revision-b",
            "b",
            &second_contract_ref,
            &shared_ref,
            &helper_ref,
            &service_callable,
            &second_operation_id,
            None,
        );
        deployment_a.service_selectors[0].contract = second_contract_ref.clone();
        deployment_a.ingress.push(DeploymentIngressBinding {
            selector: ingress_alias_selector.clone(),
            gateway_entry_key: deployment_a.ingress[0].gateway_entry_key.clone(),
        });
        skiff_artifact_identity::assign_service_deployment_identity(&mut deployment_a).unwrap();
        skiff_artifact_identity::assign_service_deployment_identity(&mut deployment_b).unwrap();
        let activation_a = skiff_artifact_identity::service_deployment_ref(&deployment_a);
        let activation_b = skiff_artifact_identity::service_deployment_ref(&deployment_b);
        let service_key = ServiceRequirementKey {
            caller_package_build_id: shared.package_build_id.clone(),
            service_requirement_slot: 0,
        };

        let mut assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new("unassigned"),
            roots: vec![activation_a.clone()],
            resolved_deployments: vec![activation_a.clone(), activation_b.clone()],
            resolved_contracts: vec![contract_ref.clone(), second_contract_ref.clone()],
            resolved_packages: vec![shared_ref.clone(), helper_ref.clone()],
            package_link_plan: CanonicalPackageLinkPlan {
                code_slots: vec![
                    PackageCodeSlot {
                        package: shared_ref.clone(),
                    },
                    PackageCodeSlot {
                        package: helper_ref.clone(),
                    },
                ],
                package_links: vec![PackageBinding {
                    key: PackageRequirementKey {
                        caller_package_build_id: shared.package_build_id.clone(),
                        package_requirement_alias: "helper".to_string(),
                    },
                    package: helper_ref.clone(),
                }],
            },
            service_binding_templates: vec![
                ServiceBindingTemplate {
                    activation: activation_a.clone(),
                    bindings: vec![ResolvedServiceBinding {
                        key: service_key.clone(),
                        contract: second_contract_ref.clone(),
                        provider: activation_b.clone(),
                        used_operations: vec![second_operation_id.clone()],
                    }],
                },
                ServiceBindingTemplate {
                    activation: activation_b.clone(),
                    bindings: vec![ResolvedServiceBinding {
                        key: service_key,
                        contract: second_contract_ref.clone(),
                        provider: activation_b.clone(),
                        used_operations: vec![second_operation_id.clone()],
                    }],
                },
            ],
            activation_templates: vec![
                activation_template(&activation_a, &deployment_a),
                activation_template(&activation_b, &deployment_b),
            ],
            gateway_ingress: deployment_a
                .ingress
                .iter()
                .map(|binding| {
                    let entry = deployment_a
                        .gateway_entries
                        .get(&binding.gateway_entry_key)
                        .unwrap();
                    GatewayIngressBinding {
                        selector: binding.selector.clone(),
                        deployment: activation_a.clone(),
                        gateway_entry_key: binding.gateway_entry_key.clone(),
                        gateway_entry_identity: entry.gateway_entry_identity.clone(),
                    }
                })
                .collect(),
        };
        skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();

        let shared_build = shared.package_build_id.clone();
        let helper_build = helper.package_build_id.clone();
        let helper_abi = helper.package_local_abi.local_abi_identity.clone();
        let shared_file_identity = shared_file.file_ir_identity.clone();
        let schema_indexes = [&shared, &helper]
            .into_iter()
            .map(|package| {
                (
                    package.package_schema_index.clone(),
                    Arc::new(PackageSchemaIndex {
                        package_id: package.package_id.clone(),
                        package_schema_index_identity: package
                            .package_schema_index
                            .package_schema_index_identity
                            .clone(),
                        types: BTreeMap::new(),
                    }),
                )
            })
            .collect();
        let resolver = FixtureResolver {
            deployments: BTreeMap::from([
                (activation_a.clone(), Arc::new(deployment_a.clone())),
                (activation_b.clone(), Arc::new(deployment_b)),
            ]),
            contracts: BTreeMap::from([
                (contract_ref.clone(), Arc::new(contract)),
                (second_contract_ref, Arc::new(second_contract)),
            ]),
            packages: BTreeMap::from([
                (shared_ref, Arc::new(shared)),
                (helper_ref, Arc::new(helper)),
            ]),
            schema_indexes,
            files: BTreeMap::from([
                (
                    (shared_build.clone(), shared_file_identity.clone()),
                    Arc::new(shared_file),
                ),
                (
                    (helper_build.clone(), helper_file.file_ir_identity.clone()),
                    Arc::new(helper_file),
                ),
            ]),
            resources: BTreeMap::from([(
                (helper_build.clone(), "assets/helper.txt".to_string()),
                helper_resource,
            )]),
        };

        Self {
            assembly,
            resolver,
            operation_id,
            contract_ref,
            service_callable,
            helper_callable,
            shared_build,
            helper_build,
            helper_abi,
            shared_file_identity,
            activation_a,
            activation_b,
            ingress_selector,
            ingress_alias_selector,
            gateway_handler,
            gateway_pre,
            gateway_guard,
            gateway_entry_key: deployment_a.ingress[0].gateway_entry_key.clone(),
            gateway_entry_identity: deployment_a
                .gateway_entries
                .values()
                .next()
                .unwrap()
                .gateway_entry_identity
                .clone(),
        }
    }

    pub fn tamper_deployment_callable(&mut self) {
        self.mutate_activation_a_deployment(|deployment| {
            deployment.operation_bindings[0].package_callable_id =
                PackageCallableId::new("callable:missing");
        });
    }

    pub fn tamper_gateway_to_dependency_callable(&mut self) {
        let dependency_callable = self.helper_callable.clone();
        self.mutate_activation_a_deployment(|deployment| {
            deployment
                .gateway_entries
                .values_mut()
                .next()
                .unwrap()
                .handler = Some(dependency_callable);
        });
    }

    fn mutate_activation_a_deployment(&mut self, mutate: impl FnOnce(&mut ServiceDeployment)) {
        let old_reference = self.activation_a.clone();
        let mut deployment = self
            .resolver
            .deployments
            .remove(&old_reference)
            .unwrap()
            .as_ref()
            .clone();
        mutate(&mut deployment);
        skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
        let new_reference = skiff_artifact_identity::service_deployment_ref(&deployment);
        for root in &mut self.assembly.roots {
            if root == &old_reference {
                *root = new_reference.clone();
            }
        }
        for reference in &mut self.assembly.resolved_deployments {
            if reference == &old_reference {
                *reference = new_reference.clone();
            }
        }
        for template in &mut self.assembly.service_binding_templates {
            if template.activation == old_reference {
                template.activation = new_reference.clone();
            }
            for binding in &mut template.bindings {
                if binding.provider == old_reference {
                    binding.provider = new_reference.clone();
                }
            }
        }
        for template in &mut self.assembly.activation_templates {
            if template.deployment == old_reference {
                template.deployment = new_reference.clone();
            }
        }
        for ingress in &mut self.assembly.gateway_ingress {
            if ingress.deployment == old_reference {
                ingress.deployment = new_reference.clone();
            }
        }
        self.resolver
            .deployments
            .insert(new_reference.clone(), Arc::new(deployment));
        self.activation_a = new_reference;
        skiff_artifact_identity::assign_runtime_assembly_identity(&mut self.assembly).unwrap();
    }

    pub fn mutate_shared_file(&mut self, mutate: impl FnOnce(&mut FileIrUnit)) {
        let old_build = self.shared_build.clone();
        let old_file_identity = self.shared_file_identity.clone();
        let old_package_ref = self
            .resolver
            .packages
            .keys()
            .find(|reference| reference.package_build_id == old_build)
            .cloned()
            .unwrap();
        let mut file = self
            .resolver
            .files
            .remove(&(old_build.clone(), old_file_identity.clone()))
            .unwrap()
            .as_ref()
            .clone();
        mutate(&mut file);
        skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
        let new_file_ref = file_ref(&file);

        let mut package = self
            .resolver
            .packages
            .remove(&old_package_ref)
            .unwrap()
            .as_ref()
            .clone();
        for reference in &mut package.files {
            if reference.file_ir_identity == old_file_identity {
                *reference = new_file_ref.clone();
            }
        }
        for callable in package.callable_links.values_mut() {
            if callable.target.file_ref.file_ir_identity == old_file_identity {
                callable.target.file_ref = new_file_ref.clone();
            }
        }
        for executable in package
            .implementation_links
            .functions
            .values_mut()
            .chain(package.implementation_links.impl_methods.values_mut())
        {
            if executable.file.file_ir_identity == old_file_identity {
                executable.file = new_file_ref.clone();
            }
        }
        for export in package.implementation_links.types.values_mut() {
            if export.file.file_ir_identity == old_file_identity {
                export.file = new_file_ref.clone();
            }
        }
        for (name, interface) in &file.declarations.interfaces {
            let declaration = file
                .declarations
                .types
                .get(name)
                .expect("fixture interface must have an exact type declaration");
            let source_path = format!("{}.{}", file.module_path, name);
            assert_eq!(declaration.symbol, source_path);
            let interface_methods = interface
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
                .collect::<Vec<_>>();
            package.package_local_abi.implementation_symbols.insert(
                source_path.clone(),
                PackageLocalAbiSymbol::Type {
                    local_type_id: format!("type:{}:top-level:{source_path}", package.package_id),
                    descriptor: TypeDescriptorIr::Interface,
                    is_alias: false,
                    is_interface: true,
                    type_params: interface.type_params.clone(),
                    interface_methods: interface_methods.clone(),
                    actor: None,
                },
            );
            package.implementation_links.types.insert(
                source_path.clone(),
                TypeExport {
                    file: new_file_ref.clone(),
                    type_index: declaration.type_index,
                    symbol: source_path,
                    is_interface: true,
                    descriptor: Some(TypeDescriptorIr::Interface),
                    type_params: interface.type_params.clone(),
                    interface_methods,
                    actor: None,
                },
            );
        }
        let new_file_identity = file.file_ir_identity.clone();
        self.replace_shared_package(
            old_build,
            old_package_ref,
            package,
            vec![file],
            new_file_identity,
        );
    }

    pub fn make_local_db_target_ambiguous(&mut self) {
        let old_build = self.shared_build.clone();
        let old_file_identity = self.shared_file_identity.clone();
        let old_package_ref = self
            .resolver
            .packages
            .keys()
            .find(|reference| reference.package_build_id == old_build)
            .cloned()
            .unwrap();
        let mut primary = self
            .resolver
            .files
            .remove(&(old_build.clone(), old_file_identity.clone()))
            .unwrap()
            .as_ref()
            .clone();
        attach_local_db_declaration(&mut primary, true);
        skiff_artifact_identity::assign_file_ir_identity(&mut primary).unwrap();
        let primary_ref = file_ref(&primary);

        let mut duplicate = file("shared.main");
        duplicate.source_ast_hash = "source:shared.main:duplicate-db-owner".to_string();
        attach_local_db_declaration(&mut duplicate, false);
        skiff_artifact_identity::assign_file_ir_identity(&mut duplicate).unwrap();
        let duplicate_ref = file_ref(&duplicate);

        let mut package = self
            .resolver
            .packages
            .remove(&old_package_ref)
            .unwrap()
            .as_ref()
            .clone();
        for reference in &mut package.files {
            if reference.file_ir_identity == old_file_identity {
                *reference = primary_ref.clone();
            }
        }
        package.files.push(duplicate_ref);
        for callable in package.callable_links.values_mut() {
            if callable.target.file_ref.file_ir_identity == old_file_identity {
                callable.target.file_ref = primary_ref.clone();
            }
        }
        for executable in package
            .implementation_links
            .functions
            .values_mut()
            .chain(package.implementation_links.impl_methods.values_mut())
        {
            if executable.file.file_ir_identity == old_file_identity {
                executable.file = primary_ref.clone();
            }
        }
        let primary_identity = primary.file_ir_identity.clone();
        self.replace_shared_package(
            old_build,
            old_package_ref,
            package,
            vec![primary, duplicate],
            primary_identity,
        );
    }

    fn replace_shared_package(
        &mut self,
        old_build: PackageBuildId,
        old_package_ref: PackageArtifactRef,
        mut package: PackageArtifact,
        files: Vec<FileIrUnit>,
        primary_file_identity: String,
    ) {
        skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
        let new_package_ref = package_ref(&package);
        let new_build = package.package_build_id.clone();
        for file in files {
            self.resolver.files.insert(
                (new_build.clone(), file.file_ir_identity.clone()),
                Arc::new(file),
            );
        }
        self.resolver
            .packages
            .insert(new_package_ref.clone(), Arc::new(package));

        let old_deployments = std::mem::take(&mut self.resolver.deployments);
        let mut deployment_refs = BTreeMap::new();
        for (old_reference, deployment) in old_deployments {
            let mut deployment = deployment.as_ref().clone();
            if deployment.implementation == old_package_ref {
                deployment.implementation = new_package_ref.clone();
            }
            for binding in &mut deployment.package_bindings {
                if binding.key.caller_package_build_id == old_build {
                    binding.key.caller_package_build_id = new_build.clone();
                }
            }
            for selector in &mut deployment.service_selectors {
                if selector.key.caller_package_build_id == old_build {
                    selector.key.caller_package_build_id = new_build.clone();
                }
            }
            skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
            let new_reference = skiff_artifact_identity::service_deployment_ref(&deployment);
            deployment_refs.insert(old_reference, new_reference.clone());
            self.resolver
                .deployments
                .insert(new_reference, Arc::new(deployment));
        }

        for reference in &mut self.assembly.resolved_packages {
            if reference == &old_package_ref {
                *reference = new_package_ref.clone();
            }
        }
        for slot in &mut self.assembly.package_link_plan.code_slots {
            if slot.package == old_package_ref {
                slot.package = new_package_ref.clone();
            }
        }
        for binding in &mut self.assembly.package_link_plan.package_links {
            if binding.key.caller_package_build_id == old_build {
                binding.key.caller_package_build_id = new_build.clone();
            }
            if binding.package == old_package_ref {
                binding.package = new_package_ref.clone();
            }
        }
        for root in &mut self.assembly.roots {
            if let Some(new_reference) = deployment_refs.get(root) {
                *root = new_reference.clone();
            }
        }
        for reference in &mut self.assembly.resolved_deployments {
            if let Some(new_reference) = deployment_refs.get(reference) {
                *reference = new_reference.clone();
            }
        }
        for template in &mut self.assembly.service_binding_templates {
            if let Some(new_reference) = deployment_refs.get(&template.activation) {
                template.activation = new_reference.clone();
            }
            for binding in &mut template.bindings {
                if binding.key.caller_package_build_id == old_build {
                    binding.key.caller_package_build_id = new_build.clone();
                }
                if let Some(new_reference) = deployment_refs.get(&binding.provider) {
                    binding.provider = new_reference.clone();
                }
            }
        }
        for template in &mut self.assembly.activation_templates {
            if let Some(new_reference) = deployment_refs.get(&template.deployment) {
                template.deployment = new_reference.clone();
            }
            if template.implementation_package_build_id == old_build {
                template.implementation_package_build_id = new_build.clone();
            }
        }
        for ingress in &mut self.assembly.gateway_ingress {
            if let Some(new_reference) = deployment_refs.get(&ingress.deployment) {
                ingress.deployment = new_reference.clone();
            }
        }
        self.activation_a = deployment_refs[&self.activation_a].clone();
        self.activation_b = deployment_refs[&self.activation_b].clone();
        self.shared_build = new_build;
        self.shared_file_identity = primary_file_identity;
        skiff_artifact_identity::assign_runtime_assembly_identity(&mut self.assembly).unwrap();
    }
}

pub(super) struct FixtureResolver {
    deployments: BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    packages: BTreeMap<PackageArtifactRef, Arc<PackageArtifact>>,
    schema_indexes: Vec<(PackageSchemaIndexRef, Arc<PackageSchemaIndex>)>,
    files: BTreeMap<(PackageBuildId, String), Arc<FileIrUnit>>,
    resources: BTreeMap<(PackageBuildId, String), Arc<[u8]>>,
}

impl FixtureResolver {
    pub fn file(&self, build_id: &PackageBuildId, identity: &str) -> &FileIrUnit {
        self.files
            .get(&(build_id.clone(), identity.to_string()))
            .unwrap()
    }
}

impl RuntimeAssemblyContentResolver for FixtureResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.deployments
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing deployment"))
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.contracts
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing contract"))
    }

    fn resolve_package_schema_type(
        &self,
        _reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        anyhow::bail!("fixture has no package schema records")
    }

    fn resolve_package_schema_index(
        &self,
        reference: &PackageSchemaIndexRef,
    ) -> anyhow::Result<Arc<PackageSchemaIndex>> {
        self.schema_indexes
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, index)| index)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing package schema index"))
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.packages
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing package"))
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        let artifact = self
            .packages
            .get(package)
            .ok_or_else(|| anyhow::anyhow!("missing package"))?;
        if !artifact.files.contains(reference) {
            anyhow::bail!("File IR ref is outside package")
        }
        self.files
            .get(&(
                package.package_build_id.clone(),
                reference.file_ir_identity.clone(),
            ))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing File IR"))
    }

    fn resolve_static_resource(
        &self,
        package: &PackageArtifactRef,
        reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        let artifact = self
            .packages
            .get(package)
            .ok_or_else(|| anyhow::anyhow!("missing package"))?;
        if !artifact.static_resources.contains(reference) {
            anyhow::bail!("resource ref is outside package")
        }
        self.resources
            .get(&(package.package_build_id.clone(), reference.path.clone()))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing static resource"))
    }
}

fn package(
    package_id: &str,
    file: &FileIrUnit,
    callable_id: PackageCallableId,
    operation_contract: BoundaryOperationContract,
) -> PackageArtifact {
    let reference = file_ref(file);
    let effects = no_effects();
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: Vec::new(),
        direct_return_origins: Vec::new(),
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![reference.clone()],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::from([(
                "entry".to_string(),
                PackageLocalAbiSymbol::Callable {
                    callable_id: callable_id.clone(),
                    signature: PackageCallableSignature {
                        type_params: Vec::new(),
                        parameters: Vec::new(),
                        return_type: PackageTypeRef::Local {
                            local_type: TypeRefIr::builtin("bool"),
                        },
                        may_suspend: false,
                    },
                },
            )]),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &BTreeMap::new(),
            )
            .expect("empty Package schema index is canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks {
            functions: BTreeMap::from([(
                "entry".to_string(),
                ExecutableExport {
                    file: reference.clone(),
                    executable_index: 0,
                    symbol: "entry".to_string(),
                    signature: ExecutableSignatureIr {
                        params: Vec::new(),
                        return_type: TypeRefIr::builtin("bool"),
                        self_type: None,
                        may_suspend: false,
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
                    file_ref: reference,
                    executable_index: 0,
                    callable_abi_id: callable_id.to_string(),
                    callable_kind: OperationCallableKind::PublicFunction,
                },
            },
        )]),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
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
            callable_id.clone(),
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
        service_call_refs: Vec::new(),
        bytecode: None,
    }
}

fn add_private_gateway_callable(
    package: &mut PackageArtifact,
    source_path: &str,
    callable_id: &PackageCallableId,
) {
    let signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: "body".to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        },
        may_suspend: false,
    };
    package.package_local_abi.implementation_symbols.insert(
        source_path.to_string(),
        PackageLocalAbiSymbol::Callable {
            callable_id: callable_id.clone(),
            signature,
        },
    );
    let public_callable = package
        .package_local_abi
        .public_symbols
        .values()
        .find_map(|symbol| match symbol {
            PackageLocalAbiSymbol::Callable { callable_id, .. } => Some(callable_id.clone()),
            _ => None,
        })
        .unwrap();
    let mut target = package.callable_links[&public_callable].target.clone();
    target.callable_abi_id = callable_id.to_string();
    target.callable_kind = OperationCallableKind::InternalFunction;
    package.callable_links.insert(
        callable_id.clone(),
        PackageCallableLinkFact {
            callable_id: callable_id.clone(),
            target,
        },
    );
    package.callable_semantic_facts.insert(
        callable_id.clone(),
        package.callable_semantic_facts[&public_callable].clone(),
    );
}

#[allow(clippy::too_many_arguments)]
fn deployment(
    revision: &str,
    owner: &str,
    contract: &ServiceContractRef,
    implementation: &PackageArtifactRef,
    helper: &PackageArtifactRef,
    callable: &PackageCallableId,
    operation: &ContractOperationId,
    gateway: Option<(
        IngressSelector,
        PackageCallableId,
        PackageCallableId,
        PackageCallableId,
    )>,
) -> ServiceDeployment {
    let package_key = PackageRequirementKey {
        caller_package_build_id: implementation.package_build_id.clone(),
        package_requirement_alias: "helper".to_string(),
    };
    let service_key = ServiceRequirementKey {
        caller_package_build_id: implementation.package_build_id.clone(),
        service_requirement_slot: 0,
    };
    let (gateway_entries, ingress) = match gateway {
        Some((selector, handler, pre, guard)) => {
            let gateway_entry_key =
                GatewayEntryKey::parse("fixture-http").expect("fixture gateway entry key");
            let mut entry = gateway_entry(handler);
            entry.pre = Some(pre);
            entry.guard = Some(guard);
            (
                BTreeMap::from([(gateway_entry_key.clone(), entry)]),
                vec![DeploymentIngressBinding {
                    selector,
                    gateway_entry_key,
                }],
            )
        }
        None => (BTreeMap::new(), Vec::new()),
    };
    ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: contract.clone(),
        deployment_revision: DeploymentRevision::new(revision),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: implementation.clone(),
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: operation.clone(),
            package_callable_id: callable.clone(),
        }],
        package_bindings: vec![PackageBinding {
            key: package_key,
            package: helper.clone(),
        }],
        service_selectors: vec![ServiceSelectorBinding {
            key: service_key,
            contract: contract.clone(),
        }],
        gateway_entries,
        ingress,
        diagnostic_text: DeploymentDiagnosticText {
            display_name: format!("Cycle {owner}"),
            notes: BTreeMap::new(),
        },
    }
}

fn gateway_entry(handler: PackageCallableId) -> DeploymentGatewayEntry {
    let protocol_surface = GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::TypedJson,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![GatewayAdapterSource::HttpBody],
            request_body_schema: Some(GatewayExternalSchema::String),
            response_schema: Some(GatewayExternalSchema::String),
            stream_item_schema: None,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    DeploymentGatewayEntry {
        gateway_entry_identity: skiff_artifact_identity::gateway_entry_identity(&protocol_surface)
            .expect("fixture gateway surface identity"),
        protocol_surface,
        handler: Some(handler),
        close_handler: None,
        close_adapter_plan: None,
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::TypedJson,
            args: vec![GatewayAdapterArg {
                param: "body".to_string(),
                source: GatewayAdapterSource::HttpBody,
            }],
        },
    }
}

fn activation_template(
    reference: &ServiceDeploymentRef,
    deployment: &ServiceDeployment,
) -> ActivationTemplate {
    ActivationTemplate {
        deployment: reference.clone(),
        implementation_package_build_id: deployment.implementation.package_build_id.clone(),
    }
}

fn file(module_path: &str) -> FileIrUnit {
    let mut file = FileIrUnit::empty(module_path, format!("source:{module_path}"));
    file.type_table.push(TypeDeclIr {
        name: "LocalRecord".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "entry".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("bool"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        expression_types: Vec::new(),
        statement_spans: Vec::new(),
        source_span: None,
    });
    file
}

fn attach_local_db_declaration(file: &mut FileIrUnit, include_target: bool) {
    file.declarations.types.insert(
        "LocalRecord".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "shared.main.LocalRecord".to_string(),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        "LocalRecord".to_string(),
        skiff_artifact_model::DbDeclarationIr {
            type_ref: TypeRefIr::LocalType { type_index: 0 },
            type_name: "LocalRecord".to_string(),
            collection_name: Some("ambiguous_local_record".to_string()),
            implements: None,
            identity_fields: std::collections::BTreeMap::new(),
            kind: skiff_artifact_model::DbObjectKindIr::Object,
            key: skiff_artifact_model::DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: Vec::new(),
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    if include_target {
        file.executables[0]
            .body
            .expressions
            .push(ExprIr::DbOperation {
                operation: skiff_artifact_model::DbOperationIr {
                    op: skiff_artifact_model::DbOpKindIr::Count,
                    many: false,
                    target: skiff_artifact_model::DbTargetIr {
                        type_ref: TypeRefIr::DbObjectSymbol {
                            symbol: skiff_artifact_model::ServiceSymbolRef {
                                module_path: "shared.main".to_string(),
                                symbol: "LocalRecord".to_string(),
                            },
                        },
                        type_name: "LocalRecord".to_string(),
                    },
                    selector: None,
                    query: None,
                    projection: None,
                    body: None,
                    insert_body: None,
                    change: None,
                    result_type: TypeRefIr::builtin("number"),
                    source_span: None,
                },
            });
        file.executables[0]
            .expression_types
            .push(TypeRefIr::builtin("number"));
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

fn package_ref(package: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}

fn contract_ref(contract: &ServiceContract) -> ServiceContractRef {
    ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn operation_contract() -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: skiff_artifact_model::ContractTypeRef::builtin("bool"),
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

fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}
