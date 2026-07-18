use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallableEffectSummary, ContractTypeRef, PackageCallableParameter, PackageCallableSignature,
    PackageTypeRef,
};

use crate::{
    callable_effects::SourceCallableEffectFacts, compile_model::ExportBindingModel,
    parsed_sources::ParsedCompilerSource, SourceDependencyAnalysisInput, TypeResolutionContext,
    TypeResolutionModel,
};

mod callables;
mod types;
mod validation;

use callables::exported_callable_source;
use types::ContractAwareTypeResolver;
pub(crate) use validation::validate_contract_type_uses;

#[cfg(test)]
mod tests;

/// Converts only refs sourced from T03A-validated contract facts. Inline
/// shapes intentionally have no fallback in this phase.
#[allow(dead_code)]
pub(crate) fn package_type_ref_from_validated_contract_ref(
    ty: &ContractTypeRef,
) -> Result<PackageTypeRef, String> {
    types::package_type_ref_from_validated_contract_ref(ty)
}

/// Exact callable signatures owned by source analysis.
///
/// Contract nominal types stay explicit in `PackageCallableSignature`; they
/// are never encoded through File IR or reconstructed by projection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceCallableSignatureFacts {
    by_public_path: BTreeMap<String, PackageCallableSignature>,
}

impl SourceCallableSignatureFacts {
    pub(crate) fn build(
        parsed_sources: &[ParsedCompilerSource],
        exports: &ExportBindingModel,
        type_resolution: &TypeResolutionModel,
        dependency_analysis: &SourceDependencyAnalysisInput,
        effects: &SourceCallableEffectFacts,
    ) -> Result<Self, String> {
        let resolver = ContractAwareTypeResolver::new(type_resolution, dependency_analysis);
        let mut by_public_path = BTreeMap::new();
        for export in exports.public_callables().values() {
            let source = exported_callable_source(parsed_sources, export)?;
            let context =
                TypeResolutionContext::with_type_params(&export.source_module, source.type_params);
            let parameters = source
                .params
                .iter()
                .skip(source.receiver_parameter_offset)
                .map(|parameter| {
                    Ok(PackageCallableParameter {
                        name: parameter.name.clone(),
                        ty: resolver.resolve_source_type_ref(&parameter.ty, &context)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let return_type = resolver.resolve_source_type_ref(source.return_type, &context)?;
            let effect = effects
                .operations()
                .get(&source.effect_key)
                .ok_or_else(|| {
                    format!(
                        "exported callable `{}` has no source-owned effect facts for {}",
                        export.public_path, source.effect_key
                    )
                })?;
            let may_suspend = match effect {
                CallableEffectSummary::Analyzed { effects } => effects.may_suspend,
                CallableEffectSummary::Unknown { reason } => {
                    return Err(format!(
                        "exported callable `{}` has unknown source effects: {reason:?}",
                        export.public_path
                    ));
                }
            };
            let signature = PackageCallableSignature {
                parameters,
                return_type,
                throw_types: Vec::new(),
                may_suspend,
            };
            if by_public_path
                .insert(export.public_path.clone(), signature)
                .is_some()
            {
                return Err(format!(
                    "exported callable path `{}` is declared more than once",
                    export.public_path
                ));
            }
        }
        Ok(Self { by_public_path })
    }

    pub fn signature(&self, public_path: &str) -> Option<&PackageCallableSignature> {
        self.by_public_path.get(public_path)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &PackageCallableSignature)> {
        self.by_public_path
            .iter()
            .map(|(path, signature)| (path.as_str(), signature))
    }
}
