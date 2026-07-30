use std::collections::BTreeMap;

use skiff_artifact_model::{CallableEffectSummary, PackageCallableParameter};

use crate::{
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::{ast::FunctionDecl, type_syntax::generic_type_parameter_names},
    SourceCallableEffectFacts, SourceDependencyAnalysisInput, SourceSymbolKey,
    TypeResolutionContext, TypeResolutionModel,
};

use super::{
    types::ContractAwareTypeResolver, SourceExecutableReceiver, SourceExecutableSignature,
    SourceExecutableSignatureFacts,
};

pub(super) fn build_executable_signature_facts(
    parsed_sources: &[ParsedCompilerSource],
    type_resolution: &TypeResolutionModel,
    dependency_analysis: &SourceDependencyAnalysisInput,
    effects: &SourceCallableEffectFacts,
) -> Result<SourceExecutableSignatureFacts, String> {
    let may_suspend = MaySuspendFacts::Effects(effects);
    build_executable_signature_facts_with_may_suspend(
        parsed_sources,
        type_resolution,
        dependency_analysis,
        &may_suspend,
    )
}

pub(super) fn build_executable_signature_facts_from_may_suspend(
    parsed_sources: &[ParsedCompilerSource],
    type_resolution: &TypeResolutionModel,
    dependency_analysis: &SourceDependencyAnalysisInput,
    may_suspend: &BTreeMap<SourceSymbolKey, bool>,
) -> Result<SourceExecutableSignatureFacts, String> {
    let may_suspend = MaySuspendFacts::Exact(may_suspend);
    build_executable_signature_facts_with_may_suspend(
        parsed_sources,
        type_resolution,
        dependency_analysis,
        &may_suspend,
    )
}

fn build_executable_signature_facts_with_may_suspend(
    parsed_sources: &[ParsedCompilerSource],
    type_resolution: &TypeResolutionModel,
    dependency_analysis: &SourceDependencyAnalysisInput,
    may_suspend: &MaySuspendFacts<'_>,
) -> Result<SourceExecutableSignatureFacts, String> {
    let mut builder = ExecutableSignatureBuilder {
        resolver: ContractAwareTypeResolver::new(type_resolution, dependency_analysis),
        may_suspend,
        by_source_key: BTreeMap::new(),
    };
    for parsed in parsed_sources {
        let module_path = parsed.module_path();
        for function in &parsed.ast().functions {
            builder.insert(
                SourceSymbolKey::new(module_path, &function.name),
                function,
                Vec::new(),
                false,
                module_path,
            )?;
        }
        for implementation in &parsed.ast().impls {
            let inherited_type_params = generic_type_parameter_names(&implementation.target);
            for method in &implementation.method_bodies {
                builder.insert(
                    SourceSymbolKey::new(
                        module_path,
                        impl_method_declaration_name(&implementation.target, &method.name),
                    ),
                    method,
                    inherited_type_params.clone(),
                    true,
                    module_path,
                )?;
            }
        }
    }
    Ok(SourceExecutableSignatureFacts {
        by_source_key: builder.by_source_key,
    })
}

struct ExecutableSignatureBuilder<'a> {
    resolver: ContractAwareTypeResolver<'a>,
    may_suspend: &'a MaySuspendFacts<'a>,
    by_source_key: BTreeMap<SourceSymbolKey, SourceExecutableSignature>,
}

enum MaySuspendFacts<'a> {
    Effects(&'a SourceCallableEffectFacts),
    Exact(&'a BTreeMap<SourceSymbolKey, bool>),
}

impl MaySuspendFacts<'_> {
    fn get(&self, source_key: &SourceSymbolKey) -> Result<bool, String> {
        match self {
            Self::Effects(effects) => exact_may_suspend(effects, source_key),
            Self::Exact(values) => values.get(source_key).copied().ok_or_else(|| {
                format!("source executable `{source_key}` has no exact suspension fact")
            }),
        }
    }
}

impl ExecutableSignatureBuilder<'_> {
    fn insert(
        &mut self,
        source_key: SourceSymbolKey,
        function: &FunctionDecl,
        mut type_params: Vec<String>,
        is_impl_method: bool,
        module_path: &str,
    ) -> Result<(), String> {
        type_params.extend(function.type_params.iter().cloned());
        let context = TypeResolutionContext::with_type_params(
            module_path,
            type_params.iter().cloned().collect(),
        );
        let parameters = function
            .params
            .iter()
            .map(|parameter| {
                Ok(PackageCallableParameter {
                    name: parameter.name.clone(),
                    ty: self
                        .resolver
                        .resolve_source_type_ref(&parameter.ty, &context)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let return_type = self
            .resolver
            .resolve_source_type_ref(&function.return_type, &context)?;
        let receiver = executable_receiver(function, is_impl_method, &self.resolver, &context)?;
        let may_suspend = self.may_suspend.get(&source_key)?;
        let signature = SourceExecutableSignature {
            type_params,
            parameters,
            return_type,
            receiver,
            may_suspend,
        };
        if self
            .by_source_key
            .insert(source_key.clone(), signature)
            .is_some()
        {
            return Err(format!(
                "source executable `{source_key}` has more than one exact signature fact"
            ));
        }
        Ok(())
    }
}

fn executable_receiver(
    function: &FunctionDecl,
    is_impl_method: bool,
    resolver: &ContractAwareTypeResolver<'_>,
    context: &TypeResolutionContext<'_>,
) -> Result<SourceExecutableReceiver, String> {
    if !is_impl_method || function.is_static {
        if function.implicit_self.is_some() {
            return Err("non-method executable cannot declare an implicit receiver".to_string());
        }
        return Ok(SourceExecutableReceiver::None);
    }

    match (
        function.implicit_self.as_ref(),
        function
            .params
            .first()
            .filter(|parameter| parameter.name == "self"),
    ) {
        (Some(_), Some(_)) => {
            Err("method executable cannot declare both implicit and explicit receivers".to_string())
        }
        (Some(receiver), None) => Ok(SourceExecutableReceiver::Implicit {
            ty: resolver.resolve_source_type_ref(receiver, context)?,
        }),
        (None, Some(_)) => Ok(SourceExecutableReceiver::ExplicitParameter { parameter_index: 0 }),
        (None, None) => Err(
            "non-static method executable has neither an implicit nor explicit receiver"
                .to_string(),
        ),
    }
}

fn exact_may_suspend(
    effects: &SourceCallableEffectFacts,
    source_key: &SourceSymbolKey,
) -> Result<bool, String> {
    match effects.operations().get(source_key) {
        Some(CallableEffectSummary::Analyzed { effects }) => Ok(effects.may_suspend),
        Some(CallableEffectSummary::Unknown { reason }) => Err(format!(
            "source executable `{source_key}` has unknown source effects: {reason:?}"
        )),
        None => Err(format!(
            "source executable `{source_key}` has no source-owned effect fact"
        )),
    }
}
