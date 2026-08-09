use std::collections::BTreeMap;

use skiff_artifact_model::{
    ContractTypeRef, PackageCallableParameter, PackageCallableSignature, PackageTypeRef,
};

use crate::{
    compile_model::ExportBindingModel, parsed_sources::ParsedCompilerSource, shared::ast::TypeRef,
    SourceDependencyAnalysisInput, SourceSymbolKey, TypeResolutionContext, TypeResolutionModel,
};

mod callables;
mod executables;
mod interfaces;
mod types;
mod validation;

use callables::public_instance_operation_exports;
pub(crate) use interfaces::substitute_package_type;
pub(crate) use types::package_type_contains_contract;
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

pub(crate) fn package_type_ref_from_source_type(
    ty: &TypeRef,
    context: &TypeResolutionContext<'_>,
    type_resolution: &TypeResolutionModel,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> Result<PackageTypeRef, String> {
    ContractAwareTypeResolver::new(type_resolution, dependency_analysis)
        .resolve_source_type_ref(ty, context)
}

/// Exact signature for one source executable.
///
/// Contract nominal types stay explicit here; File IR execution projection
/// consumes this fact instead of reconstructing it from lowered types.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceExecutableSignature {
    pub type_params: Vec<String>,
    pub parameters: Vec<PackageCallableParameter>,
    /// Per-parameter inout mode, aligned with `parameters`.
    pub inout: Vec<bool>,
    pub return_type: PackageTypeRef,
    pub receiver: SourceExecutableReceiver,
    pub may_suspend: bool,
}

impl SourceExecutableSignature {
    fn package_callable_signature(&self) -> Result<PackageCallableSignature, String> {
        let parameters = match self.receiver {
            SourceExecutableReceiver::None | SourceExecutableReceiver::Implicit { .. } => {
                self.parameters.clone()
            }
            SourceExecutableReceiver::ExplicitParameter { parameter_index: 0 } => self
                .parameters
                .get(1..)
                .ok_or_else(|| {
                    "explicit receiver points to a missing source parameter".to_string()
                })?
                .to_vec(),
            SourceExecutableReceiver::ExplicitParameter { parameter_index } => {
                return Err(format!(
                    "explicit receiver must be source parameter 0, found {parameter_index}"
                ));
            }
        };
        Ok(PackageCallableSignature {
            type_params: self.type_params.clone(),
            parameters,
            return_type: self.return_type.clone(),
            may_suspend: self.may_suspend,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SourceExecutableReceiver {
    None,
    Implicit { ty: PackageTypeRef },
    ExplicitParameter { parameter_index: usize },
}

/// Stable source key for one interface operation requirement.
///
/// The key deliberately carries source identity rather than a display-shaped
/// `ServiceSymbol`: dependency aliases are never part of exact conformance.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceInterfaceMethodKey {
    pub interface: SourceSymbolKey,
    pub method: String,
}

/// Exact, pre-erasure interface operation requirement owned by source.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceInterfaceRequirementSignature {
    pub parameters: Vec<PackageCallableParameter>,
    pub return_type: PackageTypeRef,
    pub receiver: SourceExecutableReceiver,
    pub interface_type_params: Vec<String>,
    pub method_type_params: Vec<String>,
    pub is_native: bool,
    pub is_provider: bool,
    pub is_static: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceInterfaceConformanceKey {
    pub receiver: SourceSymbolKey,
    pub interface: SourceSymbolKey,
}

/// One method whose exact requirement has been substituted and checked
/// against its source executable before File IR erasure.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedSourceInterfaceMethod {
    pub key: SourceInterfaceMethodKey,
    pub exact_requirement: SourceInterfaceRequirementSignature,
    pub executable: SourceSymbolKey,
    pub receiver_type: PackageTypeRef,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedSourceInterfaceConformance {
    pub key: SourceInterfaceConformanceKey,
    pub interface_arguments: Vec<PackageTypeRef>,
    pub canonical_substitutions: BTreeMap<String, PackageTypeRef>,
    pub methods: BTreeMap<String, ValidatedSourceInterfaceMethod>,
}

/// Canonical source owner for exact interface requirements and validated
/// implementation conformances.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceInterfaceSignatureFacts {
    requirements: BTreeMap<SourceInterfaceMethodKey, SourceInterfaceRequirementSignature>,
    conformances: BTreeMap<SourceInterfaceConformanceKey, ValidatedSourceInterfaceConformance>,
}

impl SourceInterfaceSignatureFacts {
    pub(crate) fn build(
        parsed_sources: &[ParsedCompilerSource],
        type_resolution: &TypeResolutionModel,
        dependency_analysis: &SourceDependencyAnalysisInput,
        executable_signatures: &SourceExecutableSignatureFacts,
    ) -> Result<Self, String> {
        interfaces::build_interface_signature_facts(
            parsed_sources,
            type_resolution,
            dependency_analysis,
            executable_signatures,
        )
    }

    pub fn requirement(
        &self,
        key: &SourceInterfaceMethodKey,
    ) -> Option<&SourceInterfaceRequirementSignature> {
        self.requirements.get(key)
    }

    pub fn conformance(
        &self,
        key: &SourceInterfaceConformanceKey,
    ) -> Option<&ValidatedSourceInterfaceConformance> {
        self.conformances.get(key)
    }

    pub fn validated_method(
        &self,
        conformance: &SourceInterfaceConformanceKey,
        method: &str,
    ) -> Option<&ValidatedSourceInterfaceMethod> {
        self.conformance(conformance)?.methods.get(method)
    }

    pub fn requirements(
        &self,
    ) -> impl Iterator<
        Item = (
            &SourceInterfaceMethodKey,
            &SourceInterfaceRequirementSignature,
        ),
    > {
        self.requirements.iter()
    }

    pub fn conformances(
        &self,
    ) -> impl Iterator<
        Item = (
            &SourceInterfaceConformanceKey,
            &ValidatedSourceInterfaceConformance,
        ),
    > {
        self.conformances.iter()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceExecutableSignatureFacts {
    by_source_key: BTreeMap<SourceSymbolKey, SourceExecutableSignature>,
}

impl SourceExecutableSignatureFacts {
    pub(crate) fn build(
        parsed_sources: &[ParsedCompilerSource],
        type_resolution: &TypeResolutionModel,
        dependency_analysis: &SourceDependencyAnalysisInput,
        effects: &crate::SourceCallableEffectFacts,
    ) -> Result<Self, String> {
        executables::build_executable_signature_facts(
            parsed_sources,
            type_resolution,
            dependency_analysis,
            effects,
        )
    }

    pub fn signature(&self, source_key: &SourceSymbolKey) -> Option<&SourceExecutableSignature> {
        self.by_source_key.get(source_key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SourceSymbolKey, &SourceExecutableSignature)> {
        self.by_source_key.iter()
    }

    /// Builds the same exact type/receiver facts for standalone File IR
    /// lowering helpers, whose suspension analysis is supplied explicitly by
    /// source owner key. Production package compilation uses [`Self::build`].
    pub fn from_exact_may_suspend(
        parsed_sources: &[ParsedCompilerSource],
        type_resolution: &TypeResolutionModel,
        dependency_analysis: &SourceDependencyAnalysisInput,
        may_suspend: &BTreeMap<SourceSymbolKey, bool>,
    ) -> Result<Self, String> {
        executables::build_executable_signature_facts_from_may_suspend(
            parsed_sources,
            type_resolution,
            dependency_analysis,
            may_suspend,
        )
    }
}

/// Public-path view over [`SourceExecutableSignatureFacts`].
///
/// This table never resolves source types independently. Receiver removal and
/// public binding happen exactly once while creating this view.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceCallableSignatureFacts {
    by_public_path: BTreeMap<String, PackageCallableSignature>,
}

impl SourceCallableSignatureFacts {
    pub(crate) fn build(
        parsed_sources: &[ParsedCompilerSource],
        exports: &ExportBindingModel,
        type_resolution: &TypeResolutionModel,
        executable_signatures: &SourceExecutableSignatureFacts,
    ) -> Result<Self, String> {
        let mut by_public_path = BTreeMap::new();
        let mut callable_exports = exports
            .public_callables()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for instance in exports.public_instances().values() {
            callable_exports.extend(public_instance_operation_exports(
                parsed_sources,
                instance,
                type_resolution,
            )?);
        }
        for export in callable_exports {
            let source_key = SourceSymbolKey::new(&export.source_module, &export.source_symbol);
            let executable_signature = executable_signatures
                .signature(&source_key)
                .ok_or_else(|| {
                    format!(
                        "exported callable `{}` has no exact source executable signature fact for {source_key}",
                        export.public_path
                    )
                })?;
            let signature = executable_signature
                .package_callable_signature()
                .map_err(|error| format!("exported callable `{}`: {error}", export.public_path))?;
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
