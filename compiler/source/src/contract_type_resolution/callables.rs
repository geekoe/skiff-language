use std::collections::BTreeSet;

use crate::{
    compile_model::{ExportCallableBinding, ExportPublicInstanceBinding},
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::{
        ast::{Param, TypeRef},
        type_syntax::generic_parts,
    },
    SourceSymbolKey, TypeResolutionContext, TypeResolutionModel,
};

pub(super) struct ExportedCallableSource<'a> {
    pub(super) params: &'a [Param],
    pub(super) return_type: &'a TypeRef,
    pub(super) type_params: BTreeSet<String>,
    pub(super) receiver_parameter_offset: usize,
    pub(super) effect_key: SourceSymbolKey,
}

pub(super) fn exported_callable_source<'a>(
    parsed_sources: &'a [ParsedCompilerSource],
    export: &ExportCallableBinding,
) -> Result<ExportedCallableSource<'a>, String> {
    let parsed = parsed_sources
        .iter()
        .filter(|parsed| parsed.module_path() == export.source_module)
        .collect::<Vec<_>>();
    if parsed.len() != 1 {
        return Err(format!(
            "exported callable `{}` resolves to {} source modules named `{}`",
            export.public_path,
            parsed.len(),
            export.source_module
        ));
    }
    let parsed = parsed[0];

    match export.kind {
        crate::api::PublicCallableKind::Function => {
            let matches = parsed
                .ast()
                .functions
                .iter()
                .filter(|function| function.name == export.source_symbol)
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(format!(
                    "exported callable `{}` resolves to {} source functions named `{}`",
                    export.public_path,
                    matches.len(),
                    export.source_symbol
                ));
            }
            let function = matches[0];
            Ok(ExportedCallableSource {
                params: &function.params,
                return_type: &function.return_type,
                type_params: function.type_params.iter().cloned().collect(),
                receiver_parameter_offset: 0,
                effect_key: SourceSymbolKey::new(&export.source_module, &function.name),
            })
        }
        crate::api::PublicCallableKind::Method => {
            let (target, method_name) = export.source_symbol.rsplit_once('.').ok_or_else(|| {
                format!(
                    "exported method `{}` has invalid source symbol `{}`",
                    export.public_path, export.source_symbol
                )
            })?;
            let mut matches = Vec::new();
            for implementation in &parsed.ast().impls {
                if local_implementation_target(&implementation.target, &export.source_module)
                    != Some(target)
                {
                    continue;
                }
                for method in &implementation.methods {
                    if method.name == method_name {
                        matches.push((implementation, method));
                    }
                }
            }
            if matches.len() != 1 {
                return Err(format!(
                    "exported callable `{}` resolves to {} source methods named `{}`",
                    export.public_path,
                    matches.len(),
                    export.source_symbol
                ));
            }
            let (implementation, method) = matches[0];
            let inherited = generic_parts(&implementation.target)
                .map(|parts| parts.args.into_iter().map(str::to_string))
                .into_iter()
                .flatten();
            Ok(ExportedCallableSource {
                params: &method.params,
                return_type: &method.return_type,
                type_params: inherited
                    .chain(method.type_params.iter().cloned())
                    .collect(),
                // `implicit_self` is a separate source fact; it is not stored
                // in the declared parameter list. Only an explicit legacy
                // `self` parameter consumes a parameter slot here.
                receiver_parameter_offset: usize::from(
                    method
                        .params
                        .first()
                        .is_some_and(|param| param.name == "self"),
                ),
                effect_key: SourceSymbolKey::new(
                    &export.source_module,
                    impl_method_declaration_name(&implementation.target, &method.name),
                ),
            })
        }
    }
}

pub(super) fn public_instance_operation_exports(
    parsed_sources: &[ParsedCompilerSource],
    instance: &ExportPublicInstanceBinding,
    type_resolution: &TypeResolutionModel,
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
        exports.extend(
            declarations[0]
                .operations
                .iter()
                .map(|operation| ExportCallableBinding {
                    public_path: format!("{}.{}", instance.public_path, operation.name),
                    source_module: receiver.module_path().to_string(),
                    source_symbol: format!("{}.{}", receiver.symbol(), operation.name),
                    kind: crate::api::PublicCallableKind::Method,
                }),
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

fn local_implementation_target<'a>(target: &'a str, module_path: &str) -> Option<&'a str> {
    let target = target.strip_prefix("root.").unwrap_or(target);
    if let Some(local) = target.strip_prefix(&format!("{module_path}.")) {
        return Some(local);
    }
    (!target.contains('.')).then_some(target)
}
