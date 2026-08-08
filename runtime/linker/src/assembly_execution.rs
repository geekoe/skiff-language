use std::{collections::BTreeMap, sync::Arc};

use anyhow::Context;
use skiff_artifact_model::MetadataValue;
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, ExecutableAddr, ExecutableKind, LinkedCallTarget, LinkedExprIr,
    LinkedFileUnit, RuntimeExecutionPackage,
};
use skiff_runtime_linked_type_plan::build_recoverable_behavior_index;

use crate::linker::linked_file_unit_from_assembly_artifact;

mod address_resolver;
mod call_semantics;
mod code_linker;
mod indexes;
mod service_error_index;

pub(super) fn link_assembly_execution_image(
    shared: Arc<skiff_runtime_linked_program::SharedPackageLinkedImage>,
) -> anyhow::Result<Arc<AssemblyExecutionImage>> {
    let converted = convert_canonical_files(shared.as_ref())?;
    let linked_files = code_linker::link_execution_files(shared.as_ref(), &converted)?;
    let types = indexes::build_execution_type_index(shared.as_ref(), &linked_files)?;
    let service_error_types =
        service_error_index::build_service_error_type_index(shared.as_ref(), &types)?;
    let code_slots = shared
        .code_slots()
        .iter()
        .zip(linked_files)
        .map(|(code, files)| {
            RuntimeExecutionPackage::try_from_shared(Arc::clone(code), files)
                .map(Arc::new)
                .map_err(anyhow::Error::new)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let image =
        AssemblyExecutionImage::try_new(shared, code_slots, types, Arc::new(service_error_types))
            .map_err(anyhow::Error::new)?;
    let task_routes = build_task_routes(&image)?;
    let recoverable_behavior_index = build_recoverable_behavior_index(
        None,
        &[],
        image.execution_packages(),
        image.link_overlay(),
        image.types(),
    )
    .map_err(|message| {
        anyhow::anyhow!("recoverable behavior index materialization failed: {message}")
    })?;
    image
        .with_recoverable_behavior_index(recoverable_behavior_index)
        .with_task_routes(task_routes)
        .map(Arc::new)
        .map_err(anyhow::Error::new)
}

fn build_task_routes(
    image: &AssemblyExecutionImage,
) -> anyhow::Result<BTreeMap<String, ExecutableAddr>> {
    const TASK_SUBMIT_METADATA_KEY: &str = "dispatchSubmit";

    let mut routes = BTreeMap::<String, ExecutableAddr>::new();
    for package in image.execution_packages() {
        for file in package.files() {
            for owner in &file.executables {
                for expression in &owner.body.expressions {
                    let LinkedExprIr::Call { call } = expression else {
                        continue;
                    };
                    let Some(metadata) = call.metadata.get(TASK_SUBMIT_METADATA_KEY) else {
                        continue;
                    };
                    let MetadataValue::Object(metadata) = metadata else {
                        anyhow::bail!(
                            "dispatchSubmit metadata must be an object with targetKind and target"
                        );
                    };
                    let Some(MetadataValue::String(target_kind)) = metadata.get("targetKind")
                    else {
                        anyhow::bail!("dispatchSubmit metadata targetKind must be a string");
                    };
                    let Some(MetadataValue::String(metadata_target)) = metadata.get("target")
                    else {
                        anyhow::bail!("dispatchSubmit metadata target must be a string");
                    };
                    if target_kind == "actorMethod" {
                        let LinkedCallTarget::ActorDispatch { plan } = &call.target else {
                            anyhow::bail!(
                                "canonical task actor method target is not a linked actor dispatch"
                            );
                        };
                        let expected_metadata_target = format!(
                            "actorMethod:{}:{}",
                            plan.declaration_owner.actor_symbol,
                            plan.method_identity.as_str()
                        );
                        if metadata_target != &expected_metadata_target {
                            anyhow::bail!(
                                "dispatchSubmit metadata target {metadata_target} does not match linked actor method {expected_metadata_target}"
                            );
                        }
                        continue;
                    }
                    if target_kind != "function" {
                        anyhow::bail!(
                            "dispatchSubmit metadata targetKind {target_kind} is unsupported"
                        );
                    }
                    let (addr, expected_metadata_target) = match &call.target {
                        LinkedCallTarget::Executable { addr } => {
                            let executable =
                                image.executable_at(addr).map_err(anyhow::Error::new)?;
                            (
                                executable.addr().clone(),
                                format!("function:{}", executable.executable().symbol),
                            )
                        }
                        LinkedCallTarget::PackageDirect { call } => (
                            call.executable_addr().clone(),
                            format!("package:{}", call.package_callable_id().as_str()),
                        ),
                        _ => anyhow::bail!(
                            "canonical task function target is not an exact linked executable"
                        ),
                    };
                    let executable = image.executable_at(&addr).map_err(anyhow::Error::new)?;
                    if executable.executable().kind != ExecutableKind::Function {
                        anyhow::bail!(
                            "canonical dispatch target {} is not a function",
                            executable.executable().symbol
                        );
                    }
                    if metadata_target != &expected_metadata_target {
                        anyhow::bail!(
                            "dispatchSubmit metadata target {metadata_target} does not match linked executable {expected_metadata_target}"
                        );
                    }
                    let route_target = format!("function:{}", executable.executable().symbol);
                    let canonical_addr = executable.addr().clone();
                    insert_task_route(&mut routes, route_target, canonical_addr)?;
                }
            }
        }
    }
    Ok(routes)
}

fn insert_task_route(
    routes: &mut BTreeMap<String, ExecutableAddr>,
    target: String,
    addr: ExecutableAddr,
) -> anyhow::Result<()> {
    if let Some(existing) = routes.get(&target) {
        if existing != &addr {
            anyhow::bail!("canonical task route {target} resolves to more than one executable");
        }
        return Ok(());
    }
    routes.insert(target, addr);
    Ok(())
}

fn convert_canonical_files(
    shared: &skiff_runtime_linked_program::SharedPackageLinkedImage,
) -> anyhow::Result<Vec<Vec<Arc<LinkedFileUnit>>>> {
    shared
        .code_slots()
        .iter()
        .map(|code| {
            code.files()
                .iter()
                .map(|file| {
                    let linked = linked_file_unit_from_assembly_artifact(
                        file,
                        &|target| match target {
                            skiff_artifact_model::CallTargetIr::PackageCallable {
                                package_ref,
                                package_callable_id,
                            } => Ok(LinkedCallTarget::PackageDirect {
                                call: shared
                                    .resolve_package_direct_call(
                                        code.package_build_id(),
                                        package_ref,
                                        package_callable_id,
                                    )
                                    .map_err(anyhow::Error::new)?,
                            }),
                            skiff_artifact_model::CallTargetIr::ServiceCall {
                                service_call_ref_index,
                            } => Ok(LinkedCallTarget::ActivationRelativeService {
                                instruction: shared
                                    .resolve_activation_relative_service_call(
                                        code.package_build_id(),
                                        &file.file_ir_identity,
                                        *service_call_ref_index,
                                    )
                                    .map_err(anyhow::Error::new)?,
                            }),
                            _ => anyhow::bail!(
                                "non-canonical call target reached canonical resolver"
                            ),
                        },
                        &|target| {
                            shared
                                .resolve_db_object_target(code.package_build_id(), &target.type_ref)
                                .map_err(anyhow::Error::new)
                        },
                    )
                    .with_context(|| {
                        format!(
                            "failed to convert assembly File IR {} from package {}",
                            file.file_ir_identity,
                            code.package_build_id()
                        )
                    })?;
                    Ok(Arc::new(linked))
                })
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn relink_execution_files_for_test(
    shared: &skiff_runtime_linked_program::SharedPackageLinkedImage,
    files: &[Vec<Arc<LinkedFileUnit>>],
) -> anyhow::Result<Vec<Vec<Arc<LinkedFileUnit>>>> {
    code_linker::link_execution_files(shared, files)
}

#[cfg(test)]
mod task_route_tests {
    use super::*;
    use skiff_artifact_model::{
        AssemblyIdentity, BlockIr, CanonicalPackageLinkPlan, ExecutableBody, ExecutableIr,
        FileIrRef, FileIrUnit, MetadataValue, PackageArtifact, PackageArtifactRef, PackageBuildId,
        PackageCodeSlot, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
        PackageRuntimeRequirements, PackageSchemaIndex, PackageSchemaIndexRef, RuntimeAssembly,
        SlotLayout, StmtIr, StmtRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION,
        RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    };
    use skiff_runtime_linked_program::{
        FileAddr, HydratedPackageCode, PublicationResourceTable, UnitAddr,
    };

    const PACKAGE_ID: &str = "example.canonical-task";
    const TARGET_SYMBOL: &str = "task.fixture.run";

    fn addr(executable: usize) -> ExecutableAddr {
        ExecutableAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            executable,
        }
    }

    #[test]
    fn repeated_exact_task_route_is_idempotent() {
        let mut routes = BTreeMap::new();
        insert_task_route(&mut routes, "function:run".to_string(), addr(1)).unwrap();
        insert_task_route(&mut routes, "function:run".to_string(), addr(1)).unwrap();
        assert_eq!(routes.get("function:run"), Some(&addr(1)));
    }

    #[test]
    fn duplicate_task_target_with_different_address_fails_linking() {
        let mut routes = BTreeMap::new();
        insert_task_route(&mut routes, "function:run".to_string(), addr(1)).unwrap();
        let error =
            insert_task_route(&mut routes, "function:run".to_string(), addr(2)).unwrap_err();
        assert!(error.to_string().contains("more than one executable"));
        assert_eq!(routes.get("function:run"), Some(&addr(1)));
    }

    #[test]
    fn metadata_target_not_matching_linked_symbol_fails_linking() {
        let mut file = FileIrUnit::empty("task.fixture", "source:canonical-task");
        file.executables = vec![caller_executable("task.fixture.other"), target_executable()];
        skiff_artifact_identity::assign_file_ir_identity(&mut file)
            .expect("canonical task File IR should receive an identity");
        let mut package = private_package(&file);
        skiff_artifact_identity::assign_package_artifact_identities(&mut package)
            .expect("canonical task package should receive identities");
        let package_ref = package_ref(&package);
        let assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: AssemblyIdentity::new("assembly:canonical-task-linker"),
            roots: Vec::new(),
            resolved_deployments: Vec::new(),
            resolved_contracts: Vec::new(),
            resolved_packages: vec![package_ref.clone()],
            package_link_plan: CanonicalPackageLinkPlan {
                code_slots: vec![PackageCodeSlot {
                    package: package_ref,
                }],
                package_links: Vec::new(),
            },
            service_binding_templates: Vec::new(),
            activation_templates: Vec::new(),
            gateway_ingress: Vec::new(),
        };
        let schema_index = PackageSchemaIndex {
            package_id: package.package_schema_index.package_id.clone(),
            package_schema_index_identity: package
                .package_schema_index
                .package_schema_index_identity
                .clone(),
            types: BTreeMap::new(),
        };
        let hydrated = HydratedPackageCode::new(
            Arc::new(package),
            vec![Arc::new(file)],
            PublicationResourceTable::default(),
        )
        .with_schema_index(Arc::new(schema_index));

        let error = crate::link_package_fixture_from_runtime_assembly(&assembly, [hydrated])
            .expect_err("mismatched task metadata must fail while linking");

        assert_eq!(
            error.to_string(),
            "dispatchSubmit metadata target function:task.fixture.other does not match linked executable function:task.fixture.run"
        );
    }

    fn caller_executable(metadata_symbol: &str) -> ExecutableIr {
        ExecutableIr {
            kind: skiff_artifact_model::ExecutableKind::Function,
            symbol: "task.fixture.submit".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: skiff_artifact_model::TypeRefIr::builtin("null"),
            self_type: None,
            slots: SlotLayout::default(),
            may_suspend: true,
            body: ExecutableBody {
                blocks: vec![BlockIr {
                    label: "entry".to_string(),
                    statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
                }],
                statements: vec![
                    StmtIr::Dispatch {
                        call: skiff_artifact_model::ExprRefIr { expression: 0 },
                    },
                    StmtIr::Return { value: None },
                ],
                expressions: vec![skiff_artifact_model::ExprIr::Call {
                    call: skiff_artifact_model::CallIr {
                        target: skiff_artifact_model::CallTargetIr::LocalExecutable {
                            executable_index: 1,
                        },
                        site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                            reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                        },
                        args: Vec::new(),
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::from([(
                            "dispatchSubmit".to_string(),
                            MetadataValue::Object(BTreeMap::from([
                                (
                                    "targetKind".to_string(),
                                    MetadataValue::String("function".to_string()),
                                ),
                                (
                                    "target".to_string(),
                                    MetadataValue::String(format!("function:{metadata_symbol}")),
                                ),
                            ])),
                        )]),
                    },
                }],
            },
            source_span: None,
        }
    }

    fn target_executable() -> ExecutableIr {
        ExecutableIr {
            kind: skiff_artifact_model::ExecutableKind::Function,
            symbol: TARGET_SYMBOL.to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: skiff_artifact_model::TypeRefIr::builtin("null"),
            self_type: None,
            slots: SlotLayout::default(),
            may_suspend: false,
            body: ExecutableBody::default(),
            source_span: None,
        }
    }

    fn private_package(file: &FileIrUnit) -> PackageArtifact {
        PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: PACKAGE_ID.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("unassigned"),
            files: vec![FileIrRef {
                file_ir_identity: file.file_ir_identity.clone(),
                module_path: file.module_path.clone(),
                artifact_path: None,
                source_ast_hash: Some(file.source_ast_hash.clone()),
            }],
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
                public_symbols: BTreeMap::new(),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: PACKAGE_ID.to_string(),
                package_schema_index_identity:
                    skiff_artifact_identity::package_schema_index_identity(
                        PACKAGE_ID,
                        &BTreeMap::new(),
                    )
                    .expect("empty Package schema index is canonical"),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: PackageImplementationLinks::default(),
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::new(),
            service_call_refs: Vec::new(),
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
}
