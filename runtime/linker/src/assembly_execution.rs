use std::{collections::BTreeMap, sync::Arc};

use anyhow::Context;
use skiff_artifact_model::MetadataValue;
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, ExecutableAddr, ExecutableKind, LinkedCallTarget, LinkedExprIr,
    LinkedFileUnit, RuntimeExecutionPackage,
};

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
    let spawn_routes = build_spawn_routes(&image)?;
    image
        .with_spawn_routes(spawn_routes)
        .map(Arc::new)
        .map_err(anyhow::Error::new)
}

fn build_spawn_routes(
    image: &AssemblyExecutionImage,
) -> anyhow::Result<BTreeMap<String, ExecutableAddr>> {
    const SPAWN_SUBMIT_METADATA_KEY: &str = "spawnSubmit";

    let mut routes = BTreeMap::<String, ExecutableAddr>::new();
    for package in image.execution_packages() {
        for file in package.files() {
            for owner in &file.executables {
                for expression in &owner.body.expressions {
                    let LinkedExprIr::Call { call } = expression else {
                        continue;
                    };
                    let Some(metadata) = call.metadata.get(SPAWN_SUBMIT_METADATA_KEY) else {
                        continue;
                    };
                    let MetadataValue::Object(metadata) = metadata else {
                        anyhow::bail!(
                            "spawnSubmit metadata must be an object with targetKind and target"
                        );
                    };
                    let Some(MetadataValue::String(target_kind)) = metadata.get("targetKind")
                    else {
                        anyhow::bail!("spawnSubmit metadata targetKind must be a string");
                    };
                    if target_kind != "function" {
                        anyhow::bail!(
                            "spawnSubmit metadata targetKind {target_kind} is unsupported"
                        );
                    }
                    let Some(MetadataValue::String(metadata_target)) = metadata.get("target")
                    else {
                        anyhow::bail!("spawnSubmit metadata target must be a string");
                    };
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
                            "canonical spawn function target is not an exact linked executable"
                        ),
                    };
                    let executable = image.executable_at(&addr).map_err(anyhow::Error::new)?;
                    if executable.executable().kind != ExecutableKind::Function {
                        anyhow::bail!(
                            "canonical spawn target {} is not a function",
                            executable.executable().symbol
                        );
                    }
                    if metadata_target != &expected_metadata_target {
                        anyhow::bail!(
                            "spawnSubmit metadata target {metadata_target} does not match linked executable {expected_metadata_target}"
                        );
                    }
                    let route_target = format!("function:{}", executable.executable().symbol);
                    let canonical_addr = executable.addr().clone();
                    if let Some(existing) =
                        routes.insert(route_target.clone(), canonical_addr.clone())
                    {
                        if existing != canonical_addr {
                            anyhow::bail!(
                                "canonical spawn route {route_target} resolves to more than one executable"
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(routes)
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
