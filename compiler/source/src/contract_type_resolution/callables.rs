use crate::{
    compile_model::{ExportCallableBinding, ExportPublicInstanceBinding},
    parsed_sources::ParsedCompilerSource,
    TypeResolutionModel,
};

pub(super) fn public_instance_operation_exports(
    parsed_sources: &[ParsedCompilerSource],
    instance: &ExportPublicInstanceBinding,
    type_resolution: &TypeResolutionModel,
) -> Result<Vec<ExportCallableBinding>, String> {
    let resolved = crate::public_instance_operations::resolve_public_instance(
        parsed_sources,
        instance,
        type_resolution,
    )
    .map_err(|error| error.to_string())?;
    Ok(resolved
        .interfaces
        .into_iter()
        .flat_map(|interface| interface.slots)
        .map(|slot| ExportCallableBinding {
            public_path: slot.operation_stable_key,
            source_module: slot.implementation.module_path().to_string(),
            source_symbol: slot.implementation.symbol().to_string(),
            kind: crate::api::PublicCallableKind::Method,
        })
        .collect())
}
