use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, FileIrRef, FileIrUnit, PackageArtifact,
    PackageArtifactRef, PublicationResourceRef, RuntimeAssembly, ServiceContract,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentRef, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_loader::{RuntimeAssemblyContentResolver, RuntimeAssemblyLoader};

use super::*;

mod fixtures;

use fixtures::CycleFixture;

struct NoContent;

impl RuntimeAssemblyContentResolver for NoContent {
    fn resolve_deployment(
        &self,
        _reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        panic!("empty assembly must not resolve deployments")
    }

    fn resolve_contract(
        &self,
        _reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        panic!("empty assembly must not resolve contracts")
    }

    fn resolve_package(
        &self,
        _reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        panic!("empty assembly must not resolve packages")
    }

    fn resolve_file_ir(
        &self,
        _package: &PackageArtifactRef,
        _reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        panic!("empty assembly must not resolve File IR")
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        panic!("empty assembly must not resolve resources")
    }
}

struct StoragefulFileRefResolver<'a, R: ?Sized> {
    inner: &'a R,
}

impl<R> RuntimeAssemblyContentResolver for StoragefulFileRefResolver<'_, R>
where
    R: RuntimeAssemblyContentResolver + ?Sized,
{
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.inner.resolve_deployment(reference)
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.inner.resolve_contract(reference)
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        let mut artifact = self.inner.resolve_package(reference)?.as_ref().clone();
        for file in &mut artifact.files {
            file.artifact_path = Some("records/package-artifacts/test/file-ir.json".to_string());
        }
        Ok(Arc::new(artifact))
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        let mut source_reference = reference.clone();
        source_reference.artifact_path = None;
        self.inner.resolve_file_ir(package, &source_reference)
    }

    fn resolve_static_resource(
        &self,
        package: &PackageArtifactRef,
        reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        self.inner.resolve_static_resource(package, reference)
    }
}

#[test]
fn empty_assembly_links_and_all_candidate_lookups_fail_closed() {
    let mut assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("unassigned"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        global_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();
    let hydrated = RuntimeAssemblyLoader::new(&NoContent)
        .load(assembly)
        .unwrap();

    let candidate = link_runtime_assembly(hydrated).unwrap();

    assert!(candidate.is_empty());
    assert_eq!(candidate.activations().len(), 0);
    assert_eq!(candidate.ingress_bindings().len(), 0);
    assert!(candidate
        .activation(&ServiceDeploymentRef {
            service_id: "missing".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: skiff_artifact_model::DeploymentRevision::new("missing"),
            deployment_artifact_identity: skiff_artifact_model::DeploymentArtifactIdentity::new(
                "missing"
            ),
        })
        .is_none());
}

#[test]
fn candidate_keeps_code_shared_and_service_bindings_activation_relative() {
    let fixture = CycleFixture::new();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly.clone())
        .unwrap();

    let candidate = link_runtime_assembly(hydrated).unwrap();

    assert_eq!(candidate.shared_image().code_slots().len(), 2);
    let activation_a = candidate.activation(&fixture.activation_a).unwrap();
    let activation_b = candidate.activation(&fixture.activation_b).unwrap();
    assert_eq!(
        activation_a.implementation_code_slot(),
        activation_b.implementation_code_slot()
    );
    assert_eq!(
        activation_a.implementation_package_build_id(),
        &fixture.shared_build
    );
    assert_ne!(
        activation_a.source().config_literals,
        activation_b.source().config_literals
    );
    assert_ne!(
        activation_a.source().state_bindings,
        activation_b.source().state_bindings
    );
    assert_ne!(
        activation_a.source().resource_bindings,
        activation_b.source().resource_bindings
    );

    let direct_call = candidate
        .shared_image()
        .resolve_package_direct_call_by_alias(
            &fixture.shared_build,
            "helper",
            &fixture.helper_callable,
        )
        .unwrap();
    assert_eq!(
        direct_call.dependency_package_build_id(),
        &fixture.helper_build
    );
    assert_eq!(direct_call.package_callable_id(), &fixture.helper_callable);
    assert_eq!(
        candidate
            .shared_image()
            .code_by_build(&fixture.helper_build)
            .unwrap()
            .static_resources()
            .get("assets/helper.txt")
            .unwrap()
            .bytes
            .as_ref(),
        b"shared helper resource"
    );

    let service_call = candidate
        .shared_image()
        .resolve_activation_relative_service_call(
            &fixture.shared_build,
            &fixture.shared_file_identity,
            skiff_artifact_model::ServiceCallRefIndex::new(0),
        )
        .unwrap();
    let binding_a = candidate
        .resolve_activation_relative_service_call(&fixture.activation_a, &service_call)
        .unwrap();
    let binding_b = candidate
        .resolve_activation_relative_service_call(&fixture.activation_b, &service_call)
        .unwrap();
    assert_eq!(binding_a.provider(), &fixture.activation_b);
    assert_eq!(binding_b.provider(), &fixture.activation_a);
    assert_ne!(binding_a.provider(), binding_b.provider());

    let provider_operation = candidate
        .activation(binding_a.provider())
        .unwrap()
        .operation(service_call.operation_id())
        .unwrap();
    assert_eq!(
        provider_operation.package_callable_id(),
        &fixture.service_callable
    );
    assert_eq!(
        provider_operation.target().callable_abi_id,
        fixture.service_callable.as_str()
    );
}

#[test]
fn production_assembly_linker_accepts_storageful_files_with_pathless_nested_targets() {
    let fixture = CycleFixture::new();
    let resolver = StoragefulFileRefResolver {
        inner: &fixture.resolver,
    };
    let hydrated = RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap();

    let candidate = link_runtime_assembly(hydrated).unwrap();
    let helper = candidate
        .shared_image()
        .code_by_build(&fixture.helper_build)
        .unwrap();

    assert!(helper.artifact().files[0].artifact_path.is_some());
    assert!(helper
        .callable_target(&fixture.helper_callable)
        .unwrap()
        .file_ref
        .artifact_path
        .is_none());
    assert_eq!(candidate.shared_image().code_slots().len(), 2);
    assert_eq!(candidate.execution_image().code_slots().len(), 2);
}

#[test]
fn assembly_execution_image_keeps_code_shared_and_call_kinds_distinct() {
    use skiff_runtime_linked_program::{LinkedCallTarget, LinkedExprIr};

    let fixture = CycleFixture::new();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly.clone())
        .unwrap();
    let candidate = link_runtime_assembly(hydrated).unwrap();
    let image = candidate.execution_image();

    let shared_by_build = Arc::clone(image.code_by_build(&fixture.shared_build).unwrap());
    let shared_by_slot = Arc::clone(
        image
            .code_slots()
            .get(shared_by_build.code_slot().index())
            .unwrap(),
    );
    assert!(Arc::ptr_eq(&shared_by_build, &shared_by_slot));
    assert_eq!(
        candidate
            .activation(&fixture.activation_a)
            .unwrap()
            .implementation_code_slot(),
        candidate
            .activation(&fixture.activation_b)
            .unwrap()
            .implementation_code_slot()
    );

    let file = shared_by_build.file(&fixture.shared_file_identity).unwrap();
    let expressions = &file.executables[0].body.expressions;
    let LinkedExprIr::Call { call: direct } = &expressions[0] else {
        panic!("first expression must be the canonical package call")
    };
    let LinkedCallTarget::PackageDirect { call: direct } = &direct.target else {
        panic!("package call did not retain its distinct linked kind")
    };
    assert_eq!(direct.caller_package_build_id(), &fixture.shared_build);
    assert_eq!(direct.dependency_package_build_id(), &fixture.helper_build);
    assert_eq!(
        image
            .executable_at(direct.executable_addr())
            .unwrap()
            .addr(),
        direct.executable_addr()
    );

    let LinkedExprIr::Call { call: service } = &expressions[2] else {
        panic!("third expression must be the canonical service call")
    };
    let LinkedCallTarget::ActivationRelativeService { instruction } = &service.target else {
        panic!("service call did not retain its activation-relative linked kind")
    };
    assert_eq!(instruction.caller_package_build_id(), &fixture.shared_build);
    assert_eq!(instruction.service_requirement_slot(), 0);
    let binding_a = candidate
        .resolve_activation_relative_service_call(&fixture.activation_a, instruction)
        .unwrap();
    let binding_b = candidate
        .resolve_activation_relative_service_call(&fixture.activation_b, instruction)
        .unwrap();
    assert_ne!(binding_a.provider(), binding_b.provider());

    let LinkedExprIr::Call { call: local } = &expressions[1] else {
        panic!("second expression must be the local executable call")
    };
    let LinkedCallTarget::Executable { addr: local_addr } = &local.target else {
        panic!("local executable was not resolved to an assembly address")
    };
    assert_eq!(
        local_addr.unit,
        skiff_runtime_linked_program::UnitAddr::Package(0)
    );
    assert_eq!(local_addr.executable, 1);
    assert_eq!(
        image.executable_at(local_addr).unwrap().executable().symbol,
        "localHelper"
    );

    let type_addr = image
        .type_addr(&fixture.shared_build, &fixture.shared_file_identity, 0)
        .unwrap();
    assert_eq!(
        image.types().declaration(&type_addr).unwrap().name,
        "LocalRecord"
    );
}

#[test]
fn candidate_retains_canonical_contract_descriptor_and_typed_ingress() {
    let fixture = CycleFixture::new();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly.clone())
        .unwrap();
    let candidate = link_runtime_assembly(hydrated).unwrap();

    let through_candidate = candidate
        .operation_descriptor(&fixture.contract_ref, &fixture.operation_id)
        .unwrap();
    let through_store = candidate
        .contract_store()
        .operation_descriptor(&fixture.contract_ref, &fixture.operation_id)
        .unwrap();
    assert!(std::ptr::eq(through_candidate, through_store));
    assert_eq!(through_candidate.operation_id, fixture.operation_id);

    let ingress = candidate.ingress(&fixture.ingress_selector).unwrap();
    assert_eq!(ingress.deployment, fixture.activation_a);
    assert_eq!(ingress.contract, fixture.contract_ref);
    assert_eq!(ingress.contract_operation_id, fixture.operation_id);
}

#[test]
fn tampered_activation_template_fails_before_a_partial_candidate_exists() {
    let mut fixture = CycleFixture::new();
    fixture.assembly.activation_templates[0].config_literals[0].value =
        skiff_artifact_model::MetadataValue::String("tampered".to_string());
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut fixture.assembly).unwrap();

    let error = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly)
        .unwrap_err();

    assert!(
        error.to_string().contains("activation template"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn canonical_calls_cannot_fall_back_to_the_service_specific_converter() {
    let fixture = CycleFixture::new();
    let shared_file = fixture
        .resolver
        .file(&fixture.shared_build, &fixture.shared_file_identity);

    let error = crate::linked_file_unit_from_artifact(shared_file)
        .expect_err("canonical calls require the assembly linker");

    assert!(error.to_string().contains("RuntimeAssembly"));
}

#[test]
fn missing_provider_callable_is_rejected_before_linking_a_candidate() {
    let mut fixture = CycleFixture::new();
    fixture.tamper_deployment_callable();

    let error = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly)
        .unwrap_err();

    assert!(
        error.to_string().contains("missing callable"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn link_plan_abi_protocol_and_ingress_tamper_fail_closed() {
    let fixture = CycleFixture::new();

    let mut wrong_abi = fixture.assembly.clone();
    wrong_abi.package_link_plan.package_links[0]
        .package
        .package_local_abi_identity =
        skiff_artifact_model::PackageLocalAbiIdentity::new("tampered-abi");
    assert!(skiff_artifact_identity::assign_runtime_assembly_identity(&mut wrong_abi).is_err());

    let mut wrong_protocol = fixture.assembly.clone();
    wrong_protocol.service_binding_templates[0].bindings[0]
        .contract
        .service_protocol_identity =
        skiff_artifact_model::ServiceProtocolIdentity::new("tampered-protocol");
    assert!(
        skiff_artifact_identity::assign_runtime_assembly_identity(&mut wrong_protocol).is_err()
    );

    let mut ingress_collision = fixture.assembly;
    ingress_collision
        .global_ingress
        .push(ingress_collision.global_ingress[0].clone());
    assert!(
        skiff_artifact_identity::assign_runtime_assembly_identity(&mut ingress_collision).is_err()
    );
}

fn relink_cycle_execution_files(
    mutate: impl FnOnce(&mut skiff_runtime_linked_program::LinkedFileUnit),
) -> anyhow::Result<()> {
    let fixture = CycleFixture::new();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver).load(fixture.assembly)?;
    let candidate = link_runtime_assembly(hydrated)?;
    let mut files = candidate
        .execution_image()
        .code_slots()
        .iter()
        .map(|code| code.files().to_vec())
        .collect::<Vec<_>>();
    mutate(Arc::make_mut(&mut files[0][0]));
    crate::assembly_execution::relink_execution_files_for_test(
        candidate.shared_image().as_ref(),
        &files,
    )?;
    Ok(())
}

fn link_identity_valid_execution_image(
    mutate: impl FnOnce(&mut FileIrUnit),
) -> anyhow::Result<Arc<skiff_runtime_linked_program::AssemblyExecutionImage>> {
    let mut fixture = CycleFixture::new();
    fixture.mutate_shared_file(mutate);
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver).load(fixture.assembly)?;
    let candidate = link_runtime_assembly(hydrated)?;
    Ok(Arc::clone(candidate.execution_image()))
}

fn linked_call(
    target: skiff_runtime_linked_program::LinkedCallTarget,
    arg_count: usize,
) -> skiff_runtime_linked_program::CallIr {
    use skiff_runtime_linked_program::ExprRefIr;

    skiff_runtime_linked_program::CallIr {
        target,
        args: (0..arg_count)
            .map(|expression| ExprRefIr {
                expression: expression as u32,
            })
            .collect(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn append_linked_receiver_call(
    file: &mut skiff_runtime_linked_program::LinkedFileUnit,
    executable: usize,
    method_abi_id: String,
) {
    use skiff_artifact_model::ReceiverCallAbi;
    use skiff_runtime_linked_program::{
        ConstAddr, ConstIr, ExecutableAddr, FileAddr, LinkedCallTarget, LinkedExecutableBody,
        LinkedExprIr, LinkedTypeRef, UnitAddr,
    };

    file.constants.push(ConstIr {
        name: "receiver".to_string(),
        ty: LinkedTypeRef::Native {
            name: "bool".to_string(),
            args: Vec::new(),
        },
        body: LinkedExecutableBody::default(),
        source_span: None,
    });
    file.executables[0]
        .body
        .expressions
        .push(LinkedExprIr::Call {
            call: linked_call(
                LinkedCallTarget::LocalConstReceiverExecutable {
                    const_addr: ConstAddr {
                        unit: UnitAddr::Package(0),
                        file: FileAddr::LoadedFileIndex(0),
                        const_index: 0,
                    },
                    executable_addr: ExecutableAddr::package(0, 0, executable),
                    method_abi_id,
                    receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
                },
                0,
            ),
        });
}

#[test]
fn assembly_execution_call_validation_rejects_identity_valid_native_tamper() {
    use skiff_artifact_model::{CallIr, CallTargetIr, ExprIr, ExprRefIr, NativeTarget, TypeRefIr};

    let error = link_identity_valid_execution_image(|file| {
        file.executables[0].body.expressions.push(ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::Native {
                    target: NativeTarget {
                        namespace: "std.http".to_string(),
                        symbol: "json".to_string(),
                        binding_key: Some("std.http.response.json".to_string()),
                        metadata: BTreeMap::new(),
                    },
                },
                args: vec![ExprRefIr { expression: 0 }],
                type_args: BTreeMap::from([("T0".to_string(), TypeRefIr::native("Json"))]),
                metadata: BTreeMap::new(),
            },
        });
    })
    .expect_err("identity-valid malformed native call must fail before image creation");

    assert!(
        format!("{error:#}").contains("expected 2 args, got 1"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn assembly_execution_call_validation_rejects_identity_valid_interface_tamper() {
    use skiff_artifact_model::{
        CallIr, CallTargetIr, ExprIr, FunctionTypeParamIr, InterfaceDeclIr,
        InterfaceInstantiationRef, InterfaceOperationIr, ServiceSymbolRef, TypeDeclIr,
        TypeDeclarationIr, TypeDescriptorIr, TypeRefIr,
    };

    let error = link_identity_valid_execution_image(|file| {
        let interface_name = "Reader";
        let interface_abi_id =
            skiff_artifact_identity::type_ref_abi_key(&TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: file.module_path.clone(),
                    symbol: interface_name.to_string(),
                },
            });
        file.declarations.types.insert(
            interface_name.to_string(),
            TypeDeclarationIr {
                type_index: file.type_table.len() as u32,
                symbol: format!("{}.{}", file.module_path, interface_name),
                source_span: None,
            },
        );
        file.declarations.interfaces.insert(
            interface_name.to_string(),
            InterfaceDeclIr {
                name: interface_name.to_string(),
                type_params: Vec::new(),
                operations: vec![InterfaceOperationIr {
                    name: "read".to_string(),
                    type_params: Vec::new(),
                    params: vec![FunctionTypeParamIr {
                        name: "self".to_string(),
                        ty: TypeRefIr::TypeParam {
                            name: "Self".to_string(),
                        },
                    }],
                    return_type: TypeRefIr::native("string"),
                    is_native: false,
                    is_provider: false,
                    is_static: false,
                    implicit_self: None,
                }],
                source_span: None,
            },
        );
        file.type_table.push(TypeDeclIr {
            name: interface_name.to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.executables[0].body.expressions.push(ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::InterfaceMethod {
                    interface: InterfaceInstantiationRef {
                        interface_abi_id,
                        canonical_type_args: Vec::new(),
                    },
                    method_abi_id: "method:tampered".to_string(),
                    slot: 1,
                },
                args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        });
    })
    .expect_err("identity-valid malformed interface call must fail before image creation");

    assert!(
        format!("{error:#}").contains("interface method call target slot"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn assembly_execution_call_validation_rejects_receiver_target_and_abi_tamper() {
    let error = relink_cycle_execution_files(|file| {
        append_linked_receiver_call(file, usize::MAX, "method:reader".to_string());
    })
    .expect_err("assembly execution image must reject malformed receiver calls");

    assert!(
        format!("{error:#}").contains("receiver executable target"),
        "unexpected error: {error:#}"
    );

    let error = relink_cycle_execution_files(|file| {
        append_linked_receiver_call(file, 0, String::new());
    })
    .expect_err("assembly execution image must reject empty receiver method ABI");

    assert!(
        format!("{error:#}").contains("non-empty local receiver executable methodAbiId"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn assembly_execution_call_validation_accepts_builtin_native_and_interface_calls() {
    use crate::program::linked::TypeDeclarationIr;
    use skiff_artifact_model::{NativeTarget, ServiceSymbolRef, TypeRefIr};
    use skiff_runtime_linked_program::{
        FunctionTypeParamIr, InterfaceDeclIr, InterfaceOperationIr, LinkedCallTarget, LinkedExprIr,
        LinkedInterfaceInstantiationRef, LinkedTypeDescriptor, LinkedTypeRef, TypeDeclIr,
    };

    relink_cycle_execution_files(|file| {
        let interface_name = "Reader";
        let interface_abi_id =
            skiff_artifact_identity::type_ref_abi_key(&TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: file.module_path.clone(),
                    symbol: interface_name.to_string(),
                },
            });
        file.declarations.types.insert(
            interface_name.to_string(),
            TypeDeclarationIr {
                type_index: file.types.len(),
                symbol: format!("{}.{}", file.module_path, interface_name),
                source_span: None,
            },
        );
        file.declarations.interfaces.insert(
            interface_name.to_string(),
            InterfaceDeclIr {
                name: interface_name.to_string(),
                type_params: Vec::new(),
                operations: vec![InterfaceOperationIr {
                    name: "read".to_string(),
                    type_params: Vec::new(),
                    params: vec![FunctionTypeParamIr {
                        name: "self".to_string(),
                        ty: LinkedTypeRef::TypeParam {
                            name: "Self".to_string(),
                        },
                    }],
                    return_type: LinkedTypeRef::Native {
                        name: "string".to_string(),
                        args: Vec::new(),
                    },
                    is_native: false,
                    is_provider: false,
                    is_static: false,
                    implicit_self: None,
                }],
                source_span: None,
            },
        );
        file.types.push(TypeDeclIr {
            name: interface_name.to_string(),
            descriptor: LinkedTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        let interface = LinkedInterfaceInstantiationRef {
            interface_abi_id,
            canonical_type_args: Vec::new(),
        };
        let mut native = linked_call(
            LinkedCallTarget::Native {
                target: NativeTarget {
                    namespace: "std.http".to_string(),
                    symbol: "json".to_string(),
                    binding_key: Some("std.http.response.json".to_string()),
                    metadata: BTreeMap::new(),
                },
            },
            2,
        );
        native.type_args.insert(
            "T0".to_string(),
            LinkedTypeRef::Native {
                name: "Json".to_string(),
                args: Vec::new(),
            },
        );
        file.executables[0].body.expressions.extend([
            LinkedExprIr::Call {
                call: linked_call(
                    LinkedCallTarget::Builtin {
                        op: "test.builtin".to_string(),
                    },
                    0,
                ),
            },
            LinkedExprIr::Call { call: native },
            LinkedExprIr::Call {
                call: linked_call(
                    LinkedCallTarget::InterfaceMethod {
                        method_abi_id: format!("method:{}:read", interface.interface_abi_id),
                        interface,
                        slot: 0,
                    },
                    0,
                ),
            },
        ]);
    })
    .expect("valid builtin, native, and interface calls must keep linking");
}
