use std::collections::BTreeSet;

use crate::{
    compile_model::ExportCallableBinding,
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::{
        ast::{Param, TypeRef},
        type_syntax::generic_parts,
    },
    SourceSymbolKey,
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
                receiver_parameter_offset: usize::from(
                    method.implicit_self.is_some()
                        || method
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

fn local_implementation_target<'a>(target: &'a str, module_path: &str) -> Option<&'a str> {
    let target = target.strip_prefix("root.").unwrap_or(target);
    if let Some(local) = target.strip_prefix(&format!("{module_path}.")) {
        return Some(local);
    }
    (!target.contains('.')).then_some(target)
}
