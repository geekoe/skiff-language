use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, FileIrRef, FileIrUnit, PackageArtifact,
    PackageArtifactRef, PublicationResourceRef, RuntimeAssembly, ServiceContract,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentRef, ServiceIngressKey,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_loader::{RuntimeAssemblyContentResolver, RuntimeAssemblyLoader};

use super::*;

mod fixtures;
mod representation_wrap;
mod tail_call_structure;

use fixtures::CycleFixture;

fn test_instruction_site() -> skiff_artifact_model::InstructionSourceSite {
    skiff_artifact_model::InstructionSourceSite::Synthetic {
        reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

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

    fn resolve_package_schema_type(
        &self,
        _reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        panic!("empty assembly must not resolve package schema")
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

    fn resolve_package_schema_type(
        &self,
        reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        self.inner.resolve_package_schema_type(reference)
    }

    fn resolve_package_schema_index(
        &self,
        reference: &skiff_artifact_model::PackageSchemaIndexRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaIndex>> {
        self.inner.resolve_package_schema_index(reference)
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
        gateway_ingress: Vec::new(),
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
    assert_eq!(binding_b.provider(), &fixture.activation_b);
    assert_eq!(binding_a.provider(), binding_b.provider());

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
    assert_eq!(candidate.execution_image().execution_packages().len(), 2);
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
            .execution_packages()
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
    assert_eq!(binding_a.provider(), binding_b.provider());

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

    let LinkedExprIr::Call { call: actor } = &expressions[3] else {
        panic!("fourth expression must be the Actor method call")
    };
    let LinkedCallTarget::ActorDispatch { plan } = &actor.target else {
        panic!("Actor method call was not linked to routed Actor dispatch")
    };
    assert_eq!(plan.declaration_owner.actor_symbol, "DocHub");
    assert_eq!(plan.method_identity.as_str(), "actor-method:submit");
    let actor_declaration = &file.actor_declarations[0];
    assert_eq!(
        plan.actor_implementation_identity,
        actor_declaration.actor_implementation_identity
    );
    let skiff_runtime_linked_program::LinkedActorMethodImplementation::Executable {
        addr: actor_method_addr,
    } = &actor_declaration.public_methods[0].implementation
    else {
        panic!("Actor method declaration entry was not linked")
    };
    assert_eq!(actor_method_addr.executable, 1);

    let type_addr = image
        .type_addr(&fixture.shared_build, &fixture.shared_file_identity, 0)
        .unwrap();
    assert_eq!(
        image.types().declaration(&type_addr).unwrap().name,
        "LocalRecord"
    );
}

#[test]
fn assembly_candidate_retains_internal_operation_and_exact_linked_gateway_entry() {
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
    assert!(candidate
        .activation(&fixture.activation_a)
        .unwrap()
        .operation(&fixture.operation_id)
        .is_some());

    let ingress = candidate
        .ingress(&ServiceIngressKey {
            deployment: fixture.activation_a.clone(),
            selector: fixture.ingress_selector.clone(),
        })
        .unwrap();
    let alias = candidate
        .ingress(&ServiceIngressKey {
            deployment: fixture.activation_a.clone(),
            selector: fixture.ingress_alias_selector.clone(),
        })
        .unwrap();
    assert!(Arc::ptr_eq(ingress, alias));
    assert_eq!(ingress.owner(), &fixture.activation_a);
    assert_eq!(ingress.gateway_entry_key(), &fixture.gateway_entry_key);
    assert_eq!(
        ingress.gateway_entry_identity(),
        &fixture.gateway_entry_identity
    );
    assert!(matches!(
        &ingress.protocol_surface().protocol,
        skiff_artifact_model::GatewayProtocolSurface::Http(_)
    ));
    assert_eq!(ingress.adapter_plan().args[0].param, "body");
    assert_eq!(ingress.handler().callable_id(), &fixture.gateway_handler);
    assert_eq!(
        ingress.handler().target().callable_kind,
        skiff_artifact_model::OperationCallableKind::InternalFunction
    );
    assert_eq!(ingress.handler().signature().parameters[0].name, "body");
    assert_eq!(ingress.pre().unwrap().callable_id(), &fixture.gateway_pre);
    assert_eq!(
        ingress.guard().unwrap().callable_id(),
        &fixture.gateway_guard
    );
    assert!(candidate
        .gateway_entry(&fixture.activation_a, &fixture.gateway_entry_key)
        .is_some_and(|entry| Arc::ptr_eq(entry, ingress)));
}

#[test]
fn tampered_activation_template_fails_before_a_partial_candidate_exists() {
    let mut fixture = CycleFixture::new();
    fixture.assembly.activation_templates[0].implementation_package_build_id =
        PackageBuildId::new("tampered");
    let error = skiff_artifact_identity::assign_runtime_assembly_identity(&mut fixture.assembly)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("activation implementation package"),
        "unexpected error: {error:#}"
    );
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
fn assembly_gateway_cannot_borrow_a_dependency_package_callable() {
    let mut fixture = CycleFixture::new();
    fixture.tamper_gateway_to_dependency_callable();

    let error = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly)
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("is missing from implementation package"),
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
        .gateway_ingress
        .push(ingress_collision.gateway_ingress[0].clone());
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
        .execution_packages()
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

#[test]
fn assembly_linker_attaches_exact_identity_to_local_db_target() {
    let image = link_identity_valid_execution_image(|file| {
        attach_local_db_target(file, true);
    })
    .unwrap();
    let code = image
        .execution_packages()
        .iter()
        .find(|code| {
            code.files()
                .iter()
                .any(|file| file.module_path == "shared.main")
        })
        .unwrap();
    let file = code
        .files()
        .iter()
        .find(|file| file.module_path == "shared.main")
        .unwrap();
    let skiff_runtime_linked_program::LinkedExprIr::DbOperation { operation } =
        file.executables[0].body.expressions.last().unwrap()
    else {
        panic!("fixture must end in a linked DB operation")
    };

    assert_eq!(
        operation
            .target
            .target_id
            .package_artifact_ref
            .package_build_id,
        *code.package_build_id()
    );
    assert_eq!(
        operation.target.target_id.file_ir_ref.file_ir_identity,
        file.file_ir_identity
    );
    assert_eq!(operation.target.target_id.type_index, 0);
    assert!(matches!(
        operation.target.type_ref,
        skiff_runtime_linked_program::LinkedTypeRef::Address { .. }
    ));
}

#[test]
fn assembly_linker_rejects_db_target_without_provider_attachment() {
    let error = link_identity_valid_execution_image(|file| {
        attach_local_db_target(file, false);
    })
    .unwrap_err();

    assert!(
        format!("{error:#}").contains("MissingDbTargetAttachment"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn assembly_linker_projects_exact_package_db_target_into_every_execution_carrier() {
    use skiff_runtime_linked_program::{FileAddr, LinkedExprIr, LinkedTypeRef, UnitAddr};

    let mut fixture = CycleFixture::new();
    let helper_abi = fixture.helper_abi.to_string();
    fixture.mutate_shared_file(|file| {
        attach_local_db_target(file, true);
        attach_package_db_target_carriers(file, &helper_abi);
    });
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly)
        .unwrap();
    let candidate = link_runtime_assembly(hydrated).unwrap();
    let image = candidate.execution_image();
    let (helper_slot, helper_code) = image
        .execution_packages()
        .iter()
        .enumerate()
        .find(|(_, code)| code.package_build_id() == &fixture.helper_build)
        .unwrap();
    let (helper_file_index, helper_file) = helper_code
        .files()
        .iter()
        .enumerate()
        .find(|(_, file)| file.module_path == "helper.main")
        .unwrap();
    let shared_file = image
        .execution_packages()
        .iter()
        .flat_map(|code| code.files())
        .find(|file| file.module_path == "shared.main")
        .unwrap();
    let helper_artifact_ref = candidate
        .shared_image()
        .code_by_build(&fixture.helper_build)
        .unwrap()
        .artifact_ref()
        .clone();
    let expected_target = skiff_runtime_linked_program::DbObjectTargetId {
        package_artifact_ref: helper_artifact_ref,
        file_ir_ref: skiff_artifact_model::FileIrRef {
            file_ir_identity: helper_file.file_ir_identity.clone(),
            module_path: helper_file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(helper_file.source_ast_hash.clone()),
        },
        type_index: 0,
    };
    let expected_addr = skiff_runtime_linked_program::TypeAddr {
        unit: UnitAddr::Package(helper_slot),
        file: FileAddr::LoadedFileIndex(helper_file_index),
        type_index: 0,
    };
    let targets = shared_file.executables[0]
        .body
        .expressions
        .iter()
        .rev()
        .take(4)
        .map(|expression| match expression {
            LinkedExprIr::DbOperation { operation } => &operation.target,
            LinkedExprIr::DbQuery { target, .. } => &target,
            LinkedExprIr::DbLeaseClaim { claim } => &claim.target,
            LinkedExprIr::DbLeaseRead { read } => &read.target,
            other => panic!("unexpected DB carrier: {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(targets.len(), 4);
    for target in targets {
        assert_eq!(target.target_id, expected_target);
        assert_eq!(
            target.type_ref,
            LinkedTypeRef::Address {
                addr: expected_addr.clone(),
            }
        );
    }
}

#[test]
fn assembly_linker_rejects_package_db_target_with_wrong_local_abi() {
    let mut fixture = CycleFixture::new();
    fixture.mutate_shared_file(|file| {
        attach_package_db_target_carriers(file, "wrong-helper-local-abi");
    });
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly)
        .unwrap();
    let error = link_runtime_assembly(hydrated).unwrap_err();

    assert!(
        format!("{error:#}").contains("DbTargetAbiExpectationMismatch"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn assembly_pipeline_rejects_ambiguous_same_module_local_db_target_before_linking() {
    let mut fixture = CycleFixture::new();
    fixture.make_local_db_target_ambiguous();
    let error = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly)
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("repeats File IR module path shared.main"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn assembly_code_linker_rejects_db_target_id_address_mismatch() {
    let mut fixture = CycleFixture::new();
    fixture.mutate_shared_file(|file| {
        attach_local_db_target(file, true);
        attach_second_local_db_object(file);
    });
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly)
        .unwrap();
    let candidate = link_runtime_assembly(hydrated).unwrap();
    let mut files = candidate
        .execution_image()
        .execution_packages()
        .iter()
        .map(|code| code.files().to_vec())
        .collect::<Vec<_>>();
    let shared_file = files
        .iter_mut()
        .flatten()
        .find(|file| file.module_path == "shared.main")
        .unwrap();
    let skiff_runtime_linked_program::LinkedExprIr::DbOperation { operation } =
        Arc::make_mut(shared_file).executables[0]
            .body
            .expressions
            .last_mut()
            .unwrap()
    else {
        panic!("fixture must end in a linked DB operation")
    };
    operation.target.target_id.type_index = 1;

    let error = crate::assembly_execution::relink_execution_files_for_test(
        candidate.shared_image().as_ref(),
        &files,
    )
    .unwrap_err();
    assert!(
        format!("{error:#}")
            .contains("type reference does not match its exact artifact/file/type identity"),
        "unexpected error: {error:#}"
    );
}

fn attach_local_db_target(file: &mut FileIrUnit, include_attachment: bool) {
    file.declarations.types.insert(
        "LocalRecord".to_string(),
        skiff_artifact_model::TypeDeclarationIr {
            type_index: 0,
            symbol: "LocalRecord".to_string(),
            source_span: None,
        },
    );
    if include_attachment {
        file.declarations.db.insert(
            "LocalRecord".to_string(),
            skiff_artifact_model::DbDeclarationIr {
                type_ref: skiff_artifact_model::TypeRefIr::LocalType { type_index: 0 },
                type_name: "LocalRecord".to_string(),
                collection_name: "local_record".to_string(),
                kind: skiff_artifact_model::DbObjectKindIr::Object,
                key: skiff_artifact_model::DbObjectKeyIr {
                    name: "id".to_string(),
                    ty: skiff_artifact_model::TypeRefIr::builtin("string"),
                },
                fields: Vec::new(),
                retention: None,
                leases: Vec::new(),
                indexes: Vec::new(),
                source_span: None,
            },
        );
    }
    file.executables[0]
        .body
        .expressions
        .push(skiff_artifact_model::ExprIr::DbOperation {
            operation: skiff_artifact_model::DbOperationIr {
                op: skiff_artifact_model::DbOpKindIr::Count,
                many: false,
                target: skiff_artifact_model::DbTargetIr {
                    type_ref: skiff_artifact_model::TypeRefIr::DbObjectSymbol {
                        symbol: skiff_artifact_model::ServiceSymbolRef {
                            module_path: file.module_path.clone(),
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
                result_type: skiff_artifact_model::TypeRefIr::builtin("number"),
                source_span: None,
            },
        });
}

fn attach_second_local_db_object(file: &mut FileIrUnit) {
    let type_index = u32::try_from(file.type_table.len()).unwrap();
    file.type_table.push(skiff_artifact_model::TypeDeclIr {
        name: "SecondRecord".to_string(),
        descriptor: skiff_artifact_model::TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        "SecondRecord".to_string(),
        skiff_artifact_model::TypeDeclarationIr {
            type_index,
            symbol: "SecondRecord".to_string(),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        "SecondRecord".to_string(),
        skiff_artifact_model::DbDeclarationIr {
            type_ref: skiff_artifact_model::TypeRefIr::LocalType { type_index },
            type_name: "SecondRecord".to_string(),
            collection_name: "second_record".to_string(),
            kind: skiff_artifact_model::DbObjectKindIr::Object,
            key: skiff_artifact_model::DbObjectKeyIr {
                name: "id".to_string(),
                ty: skiff_artifact_model::TypeRefIr::builtin("string"),
            },
            fields: Vec::new(),
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
}

fn attach_package_db_target_carriers(file: &mut FileIrUnit, abi_expectation: &str) {
    let target = skiff_artifact_model::DbTargetIr {
        type_ref: skiff_artifact_model::TypeRefIr::PackageSymbol {
            symbol: skiff_artifact_model::PackageSymbolRef {
                package: skiff_artifact_model::PackageRefIr::Dependency {
                    dependency_ref: "helper".to_string(),
                },
                symbol_path: "helper.main.LocalRecord".to_string(),
                abi_expectation: Some(abi_expectation.to_string()),
            },
        },
        type_name: "LocalRecord".to_string(),
    };
    let query = skiff_artifact_model::DbQueryIr {
        where_clauses: Vec::new(),
        order: Vec::new(),
        limit: None,
        offset: None,
        after: None,
    };
    file.executables[0].body.expressions.extend([
        skiff_artifact_model::ExprIr::DbOperation {
            operation: skiff_artifact_model::DbOperationIr {
                op: skiff_artifact_model::DbOpKindIr::Count,
                many: false,
                target: target.clone(),
                selector: None,
                query: None,
                projection: None,
                body: None,
                insert_body: None,
                change: None,
                result_type: skiff_artifact_model::TypeRefIr::builtin("number"),
                source_span: None,
            },
        },
        skiff_artifact_model::ExprIr::DbQuery {
            query: skiff_artifact_model::DbQueryValueIr {
                target: target.clone(),
                query,
                result_type: skiff_artifact_model::TypeRefIr::builtin("number"),
                source_span: None,
            },
        },
        skiff_artifact_model::ExprIr::DbLeaseClaim {
            claim: skiff_artifact_model::DbLeaseClaimIr {
                target: target.clone(),
                key: skiff_artifact_model::ExprRefIr { expression: 0 },
                slot: "lease".to_string(),
                binding_slot: Some(0),
                body: "claimBody".to_string(),
                result_type: skiff_artifact_model::TypeRefIr::builtin("bool"),
                source_span: None,
            },
        },
        skiff_artifact_model::ExprIr::DbLeaseRead {
            read: skiff_artifact_model::DbLeaseReadIr {
                target,
                key: skiff_artifact_model::ExprRefIr { expression: 0 },
                slot: "lease".to_string(),
                result_type: skiff_artifact_model::TypeRefIr::builtin("bool"),
                source_span: None,
            },
        },
    ]);
}

fn linked_call(
    target: skiff_runtime_linked_program::LinkedCallTarget,
    arg_count: usize,
) -> skiff_runtime_linked_program::CallIr {
    use skiff_runtime_linked_program::ExprRefIr;

    skiff_runtime_linked_program::CallIr {
        target,
        site: test_instruction_site(),
        args: (0..arg_count)
            .map(|expression| ExprRefIr {
                expression: expression as u32,
            })
            .collect(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
        actor_metadata: None,
    }
}

#[test]
fn assembly_code_linker_links_required_catch_applied_nominal_exactly() {
    use skiff_artifact_model::{
        ExprIr, ExprRefIr, NominalTypeRefBaseIr, TypeDeclIr, TypeDescriptorIr, TypeRefIr,
    };
    use skiff_runtime_linked_program::{
        FileAddr, LinkedExprIr, LinkedNominalTypeRefBase, LinkedTypeRef, TypeAddr, UnitAddr,
    };

    let image = link_identity_valid_execution_image(|file| {
        let generic_type_index = u32::try_from(file.type_table.len()).unwrap();
        file.type_table.push(TypeDeclIr {
            name: "Box".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "value".to_string(),
                    TypeRefIr::TypeParam {
                        name: "T".to_string(),
                    },
                )]),
            },
            type_params: vec!["T".to_string()],
            implements: Vec::new(),
            source_span: None,
        });
        file.executables[0].body.expressions.push(ExprIr::Catch {
            try_expression: ExprRefIr { expression: 0 },
            catch_slot: 0,
            catch_type: TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType {
                    type_index: generic_type_index,
                },
                arguments: vec![TypeRefIr::LocalType { type_index: 0 }],
            },
            body: ExprRefIr { expression: 0 },
        });
    })
    .expect("required catch type should link");

    let (code_slot, file_index, file) = image
        .execution_packages()
        .iter()
        .enumerate()
        .flat_map(|(code_slot, code)| {
            code.files()
                .iter()
                .enumerate()
                .map(move |(file_index, file)| (code_slot, file_index, file))
        })
        .find(|(_, _, file)| file.module_path == "shared.main")
        .expect("mutated shared file should be in the execution image");
    let LinkedExprIr::Catch { catch_type, .. } =
        file.executables[0].body.expressions.last().unwrap()
    else {
        panic!("last expression should remain a catch")
    };
    let generic_type_index = file.types.len() - 1;
    let generic_addr = TypeAddr {
        unit: UnitAddr::Package(code_slot),
        file: FileAddr::LoadedFileIndex(file_index),
        type_index: generic_type_index,
    };
    let argument_addr = TypeAddr {
        unit: UnitAddr::Package(code_slot),
        file: FileAddr::LoadedFileIndex(file_index),
        type_index: 0,
    };
    assert_eq!(
        catch_type,
        &LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::Address { addr: generic_addr },
            arguments: vec![LinkedTypeRef::Address {
                addr: argument_addr,
            }],
        }
    );
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
                site: test_instruction_site(),
                args: vec![ExprRefIr { expression: 0 }],
                type_args: BTreeMap::from([("T0".to_string(), TypeRefIr::builtin("Json"))]),
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

fn append_actor_registry_call(
    file: &mut FileIrUnit,
    binding_key: &str,
    actor_id_type: skiff_artifact_model::TypeRefIr,
) {
    use skiff_artifact_model::{
        ActorAbiInput, ActorDeclarationIr, ActorFieldEncodingIr, ActorFieldIr, CallIr,
        CallTargetIr, ExprIr, ExprRefIr, NativeTarget, ServiceSymbolRef, TypeRefIr,
        ACTOR_RUNTIME_ABI_VERSION_V1,
    };

    if file.actor_declarations.is_empty() {
        let abi = ActorAbiInput {
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
            public_methods: Vec::new(),
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        };
        file.actor_declarations.push(ActorDeclarationIr {
            actor_abi_identity: skiff_artifact_identity::actor_abi_identity(&abi).unwrap(),
            actor_implementation_identity: skiff_artifact_model::ActorImplementationIdentity::new(
                "actor-impl:test",
            ),
            abi,
            method_implementations: BTreeMap::new(),
            create_implementation: None,
        });
    }
    let mut type_args = BTreeMap::from([
        (
            "T0".to_string(),
            TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: file.module_path.clone(),
                    symbol: "DocHub".to_string(),
                },
            },
        ),
        ("T1".to_string(), actor_id_type),
    ]);
    file.executables[0].body.expressions.push(ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::Native {
                target: NativeTarget {
                    namespace: "std.actor".to_string(),
                    symbol: binding_key
                        .strip_prefix("std.actor.")
                        .unwrap_or(binding_key)
                        .to_string(),
                    binding_key: Some(binding_key.to_string()),
                    metadata: BTreeMap::new(),
                },
            },
            site: test_instruction_site(),
            args: vec![ExprRefIr { expression: 0 }],
            type_args,
            metadata: BTreeMap::new(),
        },
    });
}

#[test]
fn assembly_execution_links_actor_registry_call_to_declaration_owner() {
    use skiff_artifact_model::TypeRefIr;
    use skiff_runtime_linked_program::{LinkedExprIr, UnitAddr};

    let image = link_identity_valid_execution_image(|file| {
        append_actor_registry_call(file, "std.actor.get", TypeRefIr::builtin("string"));
    })
    .expect("canonical Actor registry call should link");
    let code = image.execution_packages()[0].as_ref();
    let file = code.files()[0].as_ref();
    let LinkedExprIr::Call { call } = file.executables[0]
        .body
        .expressions
        .last()
        .expect("Actor call")
    else {
        panic!("last expression must be Actor call")
    };
    let metadata = call
        .actor_metadata
        .as_ref()
        .expect("linker-proven Actor metadata");
    assert_eq!(metadata.declaration_owner.unit, UnitAddr::Package(0));
    assert_eq!(metadata.declaration_owner.actor_symbol, "DocHub");
    assert_eq!(
        file.actor_declarations[0].implementation_owner.as_ref(),
        Some(&metadata.declaration_owner)
    );
}

#[test]
fn assembly_execution_defers_actor_metadata_for_generic_native_declaration() {
    use skiff_artifact_model::{ExprIr, TypeRefIr};
    use skiff_runtime_linked_program::LinkedExprIr;

    let image = link_identity_valid_execution_image(|file| {
        append_actor_registry_call(file, "std.actor.get", TypeRefIr::builtin("string"));
        file.executables[0]
            .type_params
            .push("ActorType".to_string());
        let ExprIr::Call { call } = file.executables[0]
            .body
            .expressions
            .last_mut()
            .expect("Actor call")
        else {
            panic!("last expression must be Actor call")
        };
        call.type_args.insert(
            "T0".to_string(),
            TypeRefIr::TypeParam {
                name: "ActorType".to_string(),
            },
        );
    })
    .expect("generic Actor native declaration should defer concrete owner resolution");
    let LinkedExprIr::Call { call } = image.execution_packages()[0].files()[0].executables[0]
        .body
        .expressions
        .last()
        .expect("Actor call")
    else {
        panic!("last expression must be Actor call")
    };
    assert!(call.actor_metadata.is_none());
}

#[test]
fn assembly_execution_rejects_actor_registry_id_and_create_argument_count_mismatch() {
    use skiff_artifact_model::TypeRefIr;

    let id_error = link_identity_valid_execution_image(|file| {
        append_actor_registry_call(file, "std.actor.get", TypeRefIr::builtin("integer"));
    })
    .expect_err("Actor id mismatch must fail");
    assert!(format!("{id_error:#}").contains("T1 does not match"));

    let count_error = link_identity_valid_execution_image(|file| {
        append_actor_registry_call(file, "std.actor.get", TypeRefIr::builtin("string"));
        file.executables[0]
            .body
            .expressions
            .last_mut()
            .and_then(|expression| match expression {
                skiff_artifact_model::ExprIr::Call { call } => Some(call),
                _ => None,
            })
            .unwrap()
            .args
            .push(skiff_artifact_model::ExprRefIr { expression: 0 });
    })
    .expect_err("Actor create argument count mismatch must fail");
    assert!(format!("{count_error:#}").contains("expects id and create argument(s)"));
}

#[test]
fn assembly_execution_rejects_missing_actor_declaration() {
    use skiff_artifact_model::TypeRefIr;

    let error = link_identity_valid_execution_image(|file| {
        append_actor_registry_call(file, "std.actor.get", TypeRefIr::builtin("string"));
        file.actor_declarations.clear();
    })
    .expect_err("missing Actor declaration must fail");
    assert!(format!("{error:#}").contains("without an Actor declaration"));
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
                        ty: TypeRefIr::builtin("Self"),
                    }],
                    return_type: TypeRefIr::builtin("string"),
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
            descriptor: TypeDescriptorIr::Interface,
            type_params: Vec::new(),
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
                site: test_instruction_site(),
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
            descriptor: LinkedTypeDescriptor::Interface,
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        let interface = LinkedInterfaceInstantiationRef {
            interface_abi_id,
            canonical_type_args: Vec::new(),
        };
        let local_interface = LinkedInterfaceInstantiationRef {
            interface_abi_id: skiff_artifact_identity::type_ref_abi_key(&TypeRefIr::LocalType {
                type_index: (file.types.len() - 1) as u32,
            }),
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
            LinkedExprIr::Call {
                call: linked_call(
                    LinkedCallTarget::InterfaceMethod {
                        method_abi_id: format!("method:{}:read", local_interface.interface_abi_id),
                        interface: local_interface,
                        slot: 0,
                    },
                    0,
                ),
            },
        ]);
    })
    .expect("valid builtin, native, and interface calls must keep linking");
}

#[test]
fn assembly_execution_interface_lookup_uses_exact_package_owner_and_abi() {
    use skiff_artifact_model::PackageRefIr;

    relink_cycle_package_interface_call(
        PackageRefIr::PackageId {
            package_id: "example.helper".to_string(),
        },
        "Reader",
        FixtureInterfaceAbi::Helper,
        FixtureInterfaceCollision::CallerDecoy,
    )
    .expect("direct package interface owner with exact ABI must link");

    relink_cycle_package_interface_call(
        PackageRefIr::Dependency {
            dependency_ref: "helper".to_string(),
        },
        "Reader",
        FixtureInterfaceAbi::Helper,
        FixtureInterfaceCollision::None,
    )
    .expect("dependency package interface owner with exact ABI must link");

    for (label, package, symbol_path, abi, collision, expected) in [
        (
            "wrong ABI",
            PackageRefIr::PackageId {
                package_id: "example.helper".to_string(),
            },
            "Reader",
            FixtureInterfaceAbi::Wrong,
            FixtureInterfaceCollision::None,
            "local ABI expectation mismatches",
        ),
        (
            "missing ABI",
            PackageRefIr::PackageId {
                package_id: "example.helper".to_string(),
            },
            "Reader",
            FixtureInterfaceAbi::Missing,
            FixtureInterfaceCollision::None,
            "exact local ABI expectation",
        ),
        (
            "wrong package",
            PackageRefIr::PackageId {
                package_id: "example.shared".to_string(),
            },
            "Reader",
            FixtureInterfaceAbi::Shared,
            FixtureInterfaceCollision::None,
            "is not exported",
        ),
        (
            "wrong symbol path",
            PackageRefIr::PackageId {
                package_id: "example.helper".to_string(),
            },
            "MissingReader",
            FixtureInterfaceAbi::Helper,
            FixtureInterfaceCollision::None,
            "is not exported",
        ),
        (
            "ambiguous exact declaration coordinate",
            PackageRefIr::PackageId {
                package_id: "example.helper".to_string(),
            },
            "Reader",
            FixtureInterfaceAbi::Helper,
            FixtureInterfaceCollision::OwnerAmbiguous,
            "unique interface declaration",
        ),
    ] {
        let error = relink_cycle_package_interface_call(package, symbol_path, abi, collision)
            .expect_err(label);
        let detail = format!("{error:#}");
        assert!(
            detail.contains(expected),
            "unexpected {label} error: {detail}"
        );
    }
}

#[test]
fn assembly_execution_normalizes_recoverable_interface_owner_spellings() {
    use skiff_artifact_model::{PackageRefIr, PackageSymbolRef, TypeRefIr};
    use skiff_runtime_linked_program::{
        ExprRefIr, LinkedBoxSourceIr, LinkedExprIr, LinkedInterfaceInstantiationRef,
        LinkedInterfaceMethodTablePlanIr, LinkedTypeRef,
    };

    let fixture = CycleFixture::new();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver)
        .load(fixture.assembly)
        .expect("cycle fixture should hydrate");
    let candidate = link_runtime_assembly(hydrated).expect("cycle fixture should link");
    let helper_abi = candidate
        .shared_image()
        .code_slots()
        .iter()
        .find(|code| code.artifact().package_id == "example.helper")
        .expect("helper package code")
        .local_abi_identity()
        .as_str()
        .to_string();
    let canonical_interface_abi_id =
        skiff_artifact_identity::type_ref_abi_key(&TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "example.helper".to_string(),
                },
                symbol_path: "Reader".to_string(),
                abi_expectation: Some(helper_abi.clone()),
            },
        });
    let consumer_interface_abi_id =
        skiff_artifact_identity::type_ref_abi_key(&TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "helper".to_string(),
                },
                symbol_path: "Reader".to_string(),
                abi_expectation: Some(helper_abi),
            },
        });
    let local_interface_abi_id =
        skiff_artifact_identity::type_ref_abi_key(&TypeRefIr::LocalType { type_index: 1 });
    let publication_interface_abi_id =
        skiff_artifact_identity::type_ref_abi_key(&TypeRefIr::PublicationType {
            module_path: "helper.main".to_string(),
            type_index: 1,
        });
    let interface_box = |interface_abi_id: String| {
        let interface = LinkedInterfaceInstantiationRef {
            interface_abi_id,
            canonical_type_args: Vec::new(),
        };
        LinkedExprIr::InterfaceBox {
            value: ExprRefIr { expression: 0 },
            interface: interface.clone(),
            source: LinkedBoxSourceIr::Local {
                concrete_type: LinkedTypeRef::LocalType { type_index: 0 },
                method_table: LinkedInterfaceMethodTablePlanIr {
                    interface,
                    concrete_type: LinkedTypeRef::LocalType { type_index: 0 },
                    slots: Vec::new(),
                },
            },
        }
    };

    let mut files = candidate
        .execution_image()
        .execution_packages()
        .iter()
        .map(|code| code.files().to_vec())
        .collect::<Vec<_>>();
    Arc::make_mut(&mut files[1][0]).executables[0]
        .body
        .expressions
        .push(interface_box(local_interface_abi_id));

    let mut sibling = files[1][0].as_ref().clone();
    sibling.file_ir_identity = "fixture:helper-sibling".to_string();
    sibling.source_ast_hash = "source:helper-sibling".to_string();
    sibling.module_path = "helper.sibling".to_string();
    sibling.declarations.types.clear();
    sibling.declarations.interfaces.clear();
    sibling.declarations.db.clear();
    sibling.declarations.executables.clear();
    sibling.declarations.constants.clear();
    sibling.declarations.symbols.clear();
    sibling.link_targets.types.clear();
    sibling.link_targets.executables.clear();
    sibling.link_targets.constants.clear();
    sibling.actor_declarations.clear();
    sibling.constants.clear();
    sibling.executables.truncate(1);
    sibling.executables[0].body = Default::default();
    sibling.executables[0]
        .body
        .expressions
        .push(interface_box(publication_interface_abi_id));
    files[1].push(Arc::new(sibling));

    Arc::make_mut(&mut files[0][0]).executables[0].return_type =
        Some(LinkedTypeRef::AnyInterface {
            interface: LinkedInterfaceInstantiationRef {
                interface_abi_id: consumer_interface_abi_id,
                canonical_type_args: Vec::new(),
            },
        });

    let linked = crate::assembly_execution::relink_execution_files_for_test(
        candidate.shared_image().as_ref(),
        &files,
    )
    .expect("equivalent interface owner spellings should link");
    let linked_interface = |code_slot: usize, file_index: usize| {
        let expression = linked[code_slot][file_index].executables[0]
            .body
            .expressions
            .last()
            .expect("fixture interface box");
        let LinkedExprIr::InterfaceBox {
            interface,
            source: LinkedBoxSourceIr::Local { method_table, .. },
            ..
        } = expression
        else {
            panic!("fixture expression should remain an interface box")
        };
        assert_eq!(
            method_table.interface.interface_abi_id, interface.interface_abi_id,
            "the box and its recoverable method table must share one interface identity"
        );
        interface.interface_abi_id.clone()
    };
    let owner_identity = linked_interface(1, 0);
    let sibling_identity = linked_interface(1, 1);
    let Some(LinkedTypeRef::AnyInterface {
        interface: consumer_interface,
    }) = &linked[0][0].executables[0].return_type
    else {
        panic!("consumer fixture should retain its expected any-interface type")
    };
    let consumer_identity = consumer_interface.interface_abi_id.clone();

    assert_eq!(owner_identity, canonical_interface_abi_id);
    assert_eq!(sibling_identity, canonical_interface_abi_id);
    assert_eq!(consumer_identity, canonical_interface_abi_id);
    let recoverable_index = BTreeMap::from([(
        (
            owner_identity.clone(),
            format!("interface:{owner_identity}"),
        ),
        "owner method table",
    )]);
    assert_eq!(
        recoverable_index.get(&(
            consumer_identity.clone(),
            format!("interface:{consumer_identity}")
        )),
        Some(&"owner method table"),
        "recoverable restore must find the producer table by the consumer interface/projection composite key"
    );
}

fn relink_helper_local_interface_with_artifact_mutation(
    canonical_symbol_path: &str,
    mutate: impl FnOnce(&mut PackageArtifact),
) -> anyhow::Result<(String, String, String)> {
    use skiff_artifact_model::{PackageRefIr, PackageSymbolRef, TypeRefIr};
    use skiff_runtime_linked_program::{
        ExprRefIr, HydratedPackageCode, LinkedBoxSourceIr, LinkedExprIr,
        LinkedInterfaceInstantiationRef, LinkedInterfaceMethodTablePlanIr,
        LoadedPublicationResource, PublicationResourceTable, SharedPackageLinkedImage,
    };

    let fixture = CycleFixture::new();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver).load(fixture.assembly)?;
    let assembly = Arc::clone(hydrated.assembly());
    let mut mutate = Some(mutate);
    let mut packages = Vec::with_capacity(hydrated.code_slots().len());
    for slot in hydrated.code_slots() {
        let mut artifact = slot.artifact().as_ref().clone();
        if artifact.package_id == "example.helper" {
            mutate.take().expect("helper package mutation")(&mut artifact);
        }
        let mut resources = PublicationResourceTable::default();
        for resource in slot.resources() {
            resources.insert(
                resource.reference().path.clone(),
                LoadedPublicationResource {
                    meta: resource.reference().clone(),
                    bytes: Arc::clone(resource.bytes()),
                },
            );
        }
        packages.push(
            HydratedPackageCode::new(Arc::new(artifact), slot.files().to_vec(), resources)
                .with_schema_index(Arc::clone(slot.schema_index()))
                .with_schema_records(slot.schema_records().clone()),
        );
    }
    assert!(mutate.is_none(), "fixture must contain the helper package");
    let shared = Arc::new(SharedPackageLinkedImage::from_runtime_assembly(
        &assembly, packages,
    )?);
    let execution = crate::assembly_execution::link_assembly_execution_image(Arc::clone(&shared))?;
    let helper_slot = shared
        .code_slots()
        .iter()
        .position(|code| code.artifact().package_id == "example.helper")
        .expect("helper package code slot");
    let helper_abi = shared.code_slots()[helper_slot]
        .local_abi_identity()
        .as_str()
        .to_string();
    let canonical_interface_abi_id =
        skiff_artifact_identity::type_ref_abi_key(&TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "example.helper".to_string(),
                },
                symbol_path: canonical_symbol_path.to_string(),
                abi_expectation: Some(helper_abi),
            },
        });
    let local_interface_abi_id =
        skiff_artifact_identity::type_ref_abi_key(&TypeRefIr::LocalType { type_index: 1 });
    let interface = LinkedInterfaceInstantiationRef {
        interface_abi_id: local_interface_abi_id.clone(),
        canonical_type_args: Vec::new(),
    };
    let mut files = execution
        .execution_packages()
        .iter()
        .map(|code| code.files().to_vec())
        .collect::<Vec<_>>();
    Arc::make_mut(&mut files[helper_slot][0]).executables[0]
        .body
        .expressions
        .push(LinkedExprIr::InterfaceBox {
            value: ExprRefIr { expression: 0 },
            interface: interface.clone(),
            source: LinkedBoxSourceIr::Local {
                concrete_type: skiff_runtime_linked_program::LinkedTypeRef::LocalType {
                    type_index: 0,
                },
                method_table: LinkedInterfaceMethodTablePlanIr {
                    interface,
                    concrete_type: skiff_runtime_linked_program::LinkedTypeRef::LocalType {
                        type_index: 0,
                    },
                    slots: Vec::new(),
                },
            },
        });
    let linked = crate::assembly_execution::relink_execution_files_for_test(&shared, &files)?;
    let LinkedExprIr::InterfaceBox { interface, .. } = linked[helper_slot][0].executables[0]
        .body
        .expressions
        .last()
        .expect("fixture interface box")
    else {
        panic!("fixture expression should remain an interface box")
    };
    Ok((
        interface.interface_abi_id.clone(),
        local_interface_abi_id,
        canonical_interface_abi_id,
    ))
}

#[test]
fn assembly_execution_uses_public_interface_name_when_implementation_alias_shares_coordinate() {
    let (actual, _, canonical) =
        relink_helper_local_interface_with_artifact_mutation("LlmClient", |artifact| {
            let symbol = artifact
                .package_local_abi
                .public_symbols
                .remove("Reader")
                .expect("fixture public interface");
            let export = artifact
                .implementation_links
                .types
                .remove("Reader")
                .expect("fixture interface link");
            artifact
                .package_local_abi
                .public_symbols
                .insert("LlmClient".to_string(), symbol.clone());
            artifact
                .package_local_abi
                .implementation_symbols
                .insert("types.LlmClient".to_string(), symbol);
            artifact
                .implementation_links
                .types
                .insert("LlmClient".to_string(), export.clone());
            artifact
                .implementation_links
                .types
                .insert("types.LlmClient".to_string(), export);
        })
        .expect("an implementation alias must not compete with its public interface name");

    assert_eq!(actual, canonical);
}

#[test]
fn assembly_execution_rejects_two_public_interface_aliases_at_one_coordinate() {
    let error = relink_helper_local_interface_with_artifact_mutation("Reader", |artifact| {
        let symbol = artifact.package_local_abi.public_symbols["Reader"].clone();
        let export = artifact.implementation_links.types["Reader"].clone();
        artifact
            .package_local_abi
            .public_symbols
            .insert("ReaderAlias".to_string(), symbol);
        artifact
            .implementation_links
            .types
            .insert("ReaderAlias".to_string(), export);
    })
    .expect_err("two public names for one exact interface coordinate must fail closed");

    assert!(
        format!("{error:#}").contains("unique public package interface export"),
        "unexpected duplicate public interface error: {error:#}"
    );
}

#[test]
fn assembly_execution_preserves_private_interface_owner_spelling() {
    let (actual, local, _) =
        relink_helper_local_interface_with_artifact_mutation("Reader", |artifact| {
            let symbol = artifact
                .package_local_abi
                .public_symbols
                .remove("Reader")
                .expect("fixture public interface");
            let export = artifact
                .implementation_links
                .types
                .remove("Reader")
                .expect("fixture interface link");
            artifact
                .package_local_abi
                .implementation_symbols
                .insert("types.Reader".to_string(), symbol);
            artifact
                .implementation_links
                .types
                .insert("types.Reader".to_string(), export);
        })
        .expect("a private interface keeps its exact local owner spelling");

    assert_eq!(actual, local);
}

#[test]
fn assembly_execution_rejects_public_interface_link_with_non_interface_export() {
    let error = relink_helper_local_interface_with_artifact_mutation("Reader", |artifact| {
        artifact
            .implementation_links
            .types
            .get_mut("Reader")
            .expect("fixture interface link")
            .is_interface = false;
    })
    .expect_err("a public interface link must remain marked as an interface export");

    assert!(
        format!("{error:#}").contains("package interface export at exact owner coordinate"),
        "unexpected non-interface export error: {error:#}"
    );
}

#[derive(Clone, Copy)]
enum FixtureInterfaceAbi {
    Helper,
    Shared,
    Wrong,
    Missing,
}

#[derive(Clone, Copy)]
enum FixtureInterfaceCollision {
    None,
    CallerDecoy,
    OwnerAmbiguous,
}

fn relink_cycle_package_interface_call(
    package: skiff_artifact_model::PackageRefIr,
    symbol_path: &str,
    abi: FixtureInterfaceAbi,
    collision: FixtureInterfaceCollision,
) -> anyhow::Result<()> {
    use crate::program::linked::TypeDeclarationIr;
    use skiff_artifact_model::{PackageSymbolRef, TypeRefIr};
    use skiff_runtime_linked_program::{
        FunctionTypeParamIr, InterfaceDeclIr, InterfaceOperationIr, LinkedCallTarget, LinkedExprIr,
        LinkedInterfaceInstantiationRef, LinkedTypeDescriptor, LinkedTypeRef, TypeDeclIr,
    };

    let fixture = CycleFixture::new();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver).load(fixture.assembly)?;
    let candidate = link_runtime_assembly(hydrated)?;
    let package_abi = |package_id: &str| {
        candidate
            .shared_image()
            .code_slots()
            .iter()
            .find(|code| code.artifact().package_id == package_id)
            .expect("fixture package code slot")
            .local_abi_identity()
            .as_str()
            .to_string()
    };
    let abi_expectation = match abi {
        FixtureInterfaceAbi::Helper => Some(package_abi("example.helper")),
        FixtureInterfaceAbi::Shared => Some(package_abi("example.shared")),
        FixtureInterfaceAbi::Wrong => Some("wrong-local-abi".to_string()),
        FixtureInterfaceAbi::Missing => None,
    };
    let interface_abi_id = skiff_artifact_identity::type_ref_abi_key(&TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package,
            symbol_path: symbol_path.to_string(),
            abi_expectation,
        },
    });
    let interface = LinkedInterfaceInstantiationRef {
        interface_abi_id,
        canonical_type_args: Vec::new(),
    };
    let mut files = candidate
        .execution_image()
        .execution_packages()
        .iter()
        .map(|code| code.files().to_vec())
        .collect::<Vec<_>>();
    if matches!(collision, FixtureInterfaceCollision::CallerDecoy) {
        let shared_file = Arc::make_mut(&mut files[0][0]);
        let type_index = shared_file.types.len();
        shared_file.declarations.types.insert(
            "Reader".to_string(),
            TypeDeclarationIr {
                type_index,
                symbol: "shared.main.Reader".to_string(),
                source_span: None,
            },
        );
        shared_file.declarations.interfaces.insert(
            "Reader".to_string(),
            InterfaceDeclIr {
                name: "Reader".to_string(),
                type_params: Vec::new(),
                operations: vec![InterfaceOperationIr {
                    name: "decoy".to_string(),
                    type_params: Vec::new(),
                    params: vec![FunctionTypeParamIr {
                        name: "self".to_string(),
                        ty: LinkedTypeRef::TypeParam {
                            name: "Self".to_string(),
                        },
                    }],
                    return_type: LinkedTypeRef::Native {
                        name: "bool".to_string(),
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
        shared_file.types.push(TypeDeclIr {
            name: "Reader".to_string(),
            descriptor: LinkedTypeDescriptor::Interface,
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
    }
    if matches!(collision, FixtureInterfaceCollision::OwnerAmbiguous) {
        let helper_file = files
            .iter_mut()
            .flat_map(|package_files| package_files.iter_mut())
            .find(|file| file.module_path == "helper.main")
            .expect("fixture helper file");
        let helper_file = Arc::make_mut(helper_file);
        let declaration = helper_file.declarations.types["Reader"].clone();
        let interface = helper_file.declarations.interfaces["Reader"].clone();
        helper_file
            .declarations
            .types
            .insert("ReaderAlias".to_string(), declaration);
        helper_file
            .declarations
            .interfaces
            .insert("ReaderAlias".to_string(), interface);
    }
    let shared_file = Arc::make_mut(&mut files[0][0]);
    shared_file.executables[0]
        .body
        .expressions
        .push(LinkedExprIr::Call {
            call: linked_call(
                LinkedCallTarget::InterfaceMethod {
                    method_abi_id: format!("method:{}:read", interface.interface_abi_id),
                    interface,
                    slot: 0,
                },
                0,
            ),
        });
    crate::assembly_execution::relink_execution_files_for_test(
        candidate.shared_image().as_ref(),
        &files,
    )?;
    Ok(())
}
