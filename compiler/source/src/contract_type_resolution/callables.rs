use crate::{
    compile_model::{ExportCallableBinding, ExportPublicInstanceBinding},
    parsed_sources::ParsedCompilerSource,
    SourceInterfaceConformanceKey, SourceInterfaceSignatureFacts, SourceSymbolKey,
    TypeResolutionContext, TypeResolutionModel,
};

pub(super) fn public_instance_operation_exports(
    parsed_sources: &[ParsedCompilerSource],
    instance: &ExportPublicInstanceBinding,
    type_resolution: &TypeResolutionModel,
    interface_signatures: &SourceInterfaceSignatureFacts,
) -> Result<Vec<ExportCallableBinding>, String> {
    let instance_source = unique_source_module(
        parsed_sources,
        &instance.source_module,
        &instance.public_path,
    )?;
    let constants = instance_source
        .ast()
        .consts
        .iter()
        .filter(|constant| constant.name == instance.source_symbol)
        .collect::<Vec<_>>();
    if constants.len() != 1 {
        return Err(format!(
            "public instance `{}` resolves to {} source constants named `{}`",
            instance.public_path,
            constants.len(),
            instance.source_symbol
        ));
    }
    let declared_type = constants[0].ty.as_ref().ok_or_else(|| {
        format!(
            "public instance `{}` source constant `{}` must have an explicit source type",
            instance.public_path, instance.source_symbol
        )
    })?;
    let context = TypeResolutionContext::source(&instance.source_module);
    let resolved_type = type_resolution
        .resolve_type_ref(declared_type, &context)
        .map_err(|error| {
            format!(
                "public instance `{}` type resolution failed: {error}",
                instance.public_path
            )
        })?;
    let receiver = type_resolution
        .concrete_nominal_record_symbol(&resolved_type, &context)
        .ok_or_else(|| {
            format!(
                "public instance `{}` source constant type must resolve to a local nominal record",
                instance.public_path
            )
        })?;

    let mut exports = Vec::new();
    for interface in &instance.interfaces {
        let interface_source = unique_source_module(
            parsed_sources,
            &interface.source_module,
            &instance.public_path,
        )?;
        let declarations = interface_source
            .ast()
            .interfaces
            .iter()
            .filter(|declaration| declaration.name == interface.source_symbol)
            .collect::<Vec<_>>();
        if declarations.len() != 1 {
            return Err(format!(
                "public instance `{}` resolves to {} source interfaces named `{}`",
                instance.public_path,
                declarations.len(),
                interface.source_symbol
            ));
        }
        let conformance_key = SourceInterfaceConformanceKey {
            receiver: receiver.clone(),
            interface: SourceSymbolKey::new(&interface.source_module, &interface.source_symbol),
        };
        let conformance = interface_signatures
            .conformance(&conformance_key)
            .ok_or_else(|| {
                format!(
                    "public instance `{}` receiver `{}` has no exact conformance to `{}`",
                    instance.public_path, conformance_key.receiver, conformance_key.interface
                )
            })?;
        exports.extend(
            declarations[0]
                .operations
                .iter()
                .map(|operation| {
                    let validated = conformance.methods.get(&operation.name).ok_or_else(|| {
                        format!(
                            "public instance `{}` interface `{}.{}` method `{}` has no exact validated implementation",
                            instance.public_path,
                            interface.source_module,
                            interface.source_symbol,
                            operation.name
                        )
                    })?;
                    Ok(ExportCallableBinding {
                    public_path: format!("{}.{}", instance.public_path, operation.name),
                    source_module: validated.executable.module_path().to_string(),
                    source_symbol: validated.executable.symbol().to_string(),
                    kind: crate::api::PublicCallableKind::Method,
                    service_call: false,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        );
    }
    Ok(exports)
}

fn unique_source_module<'a>(
    parsed_sources: &'a [ParsedCompilerSource],
    module_path: &str,
    public_path: &str,
) -> Result<&'a ParsedCompilerSource, String> {
    let matches = parsed_sources
        .iter()
        .filter(|parsed| parsed.module_path() == module_path)
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        return Ok(matches[0]);
    }
    Err(format!(
        "public instance `{public_path}` resolves to {} source modules named `{module_path}`",
        matches.len()
    ))
}
