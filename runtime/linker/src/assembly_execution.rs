use std::sync::Arc;

use anyhow::Context;
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, AssemblyPackageExecutionCode, LinkedCallTarget, LinkedFileUnit,
};

use crate::linker::linked_file_unit_from_assembly_artifact;

mod address_resolver;
mod code_linker;
mod indexes;

pub(super) fn link_assembly_execution_image(
    shared: Arc<skiff_runtime_linked_program::SharedPackageLinkedImage>,
) -> anyhow::Result<Arc<AssemblyExecutionImage>> {
    let converted = convert_canonical_files(shared.as_ref())?;
    let linked_files = code_linker::link_execution_files(shared.as_ref(), &converted)?;
    let types = indexes::build_execution_type_index(shared.as_ref(), &linked_files)?;
    let code_slots = shared
        .code_slots()
        .iter()
        .zip(linked_files)
        .map(|(code, files)| {
            AssemblyPackageExecutionCode::try_new(code, files)
                .map(Arc::new)
                .map_err(anyhow::Error::new)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    AssemblyExecutionImage::try_new(shared, code_slots, types)
        .map(Arc::new)
        .map_err(anyhow::Error::new)
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
                    let linked =
                        linked_file_unit_from_assembly_artifact(file, &|target| match target {
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
                        })
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
