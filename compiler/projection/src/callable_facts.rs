use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallableEffectSummary, CanonicalPublicCallableSignature, ExecutableSignatureIr, FileIrUnit,
};
use skiff_compiler_projection_input::ProjectionCallableEffectFacts;

use crate::{
    error::ProjectionError,
    publication_visible_types::{
        projection_visible_executable_signature, publication_type_names_from_file_units,
        PublicationVisibleTypeNames,
    },
    typed_artifacts::public_signature_from_receiver_executable_signature,
};

/// Canonical callable facts owned by one lowered executable.
///
/// Phase 01 keeps the sound effect summary explicitly `Unknown` until the
/// fixed-point analysis lands, while suspension is already known by lowering.
/// Keeping both facts behind the same executable key prevents service
/// operation, entrypoint, and public-instance projection from selecting
/// different owners for the same callable.
#[derive(Debug, Clone)]
pub(crate) struct ProjectedCallableFacts {
    pub(crate) effects: CallableEffectSummary,
    signature: ExecutableSignatureIr,
}

impl ProjectedCallableFacts {
    pub(crate) fn may_suspend(&self) -> bool {
        self.signature.may_suspend
    }

    pub(crate) fn public_signature(&self) -> CanonicalPublicCallableSignature {
        CanonicalPublicCallableSignature::from(self.signature.clone())
    }

    pub(crate) fn receiver_public_signature(&self) -> CanonicalPublicCallableSignature {
        public_signature_from_receiver_executable_signature(self.signature.clone())
    }
}

pub(crate) struct ProjectionCallableFactsIndex<'a> {
    callable_effects: &'a ProjectionCallableEffectFacts,
    file_ir_units: BTreeMap<&'a str, &'a FileIrUnit>,
    publication_type_names: PublicationVisibleTypeNames,
}

impl<'a> ProjectionCallableFactsIndex<'a> {
    pub(crate) fn new(
        file_ir_units: &'a [FileIrUnit],
        callable_effects: &'a ProjectionCallableEffectFacts,
    ) -> Self {
        Self {
            callable_effects,
            file_ir_units: file_ir_units
                .iter()
                .map(|unit| (unit.module_path.as_str(), unit))
                .collect(),
            publication_type_names: publication_type_names_from_file_units(
                file_ir_units
                    .iter()
                    .map(|unit| (unit.module_path.as_str(), unit)),
            ),
        }
    }

    pub(crate) fn for_symbol(
        &self,
        module_path: &str,
        source_symbol: &str,
        context: &str,
    ) -> Result<ProjectedCallableFacts, ProjectionError> {
        let file_ir_unit = self.file_ir_unit(module_path, context)?;
        let declaration = file_ir_unit
            .declarations
            .executables
            .get(source_symbol)
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: format!(
                    "{context} effect source {module_path}.{source_symbol} is missing from File IR declarations"
                ),
            })?;
        self.for_executable(module_path, declaration.executable_index, context)
    }

    pub(crate) fn for_executable(
        &self,
        module_path: &str,
        executable_index: u32,
        context: &str,
    ) -> Result<ProjectedCallableFacts, ProjectionError> {
        let file_ir_unit = self.file_ir_unit(module_path, context)?;
        let executable = file_ir_unit
            .executables
            .get(executable_index as usize)
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: format!(
                    "{context} effect source {module_path}#{executable_index} is missing from File IR executables"
                ),
            })?;
        let effects = self
            .callable_effects
            .operation(module_path, executable_index)
            .cloned()
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: format!(
                    "{context} effect source {module_path}#{executable_index} has no callable effect fact"
                ),
            })?;
        Ok(ProjectedCallableFacts {
            effects,
            signature: projection_visible_executable_signature(
                module_path,
                executable,
                &self.publication_type_names,
            ),
        })
    }

    fn file_ir_unit(
        &self,
        module_path: &str,
        context: &str,
    ) -> Result<&'a FileIrUnit, ProjectionError> {
        self.file_ir_units.get(module_path).copied().ok_or_else(|| {
            ProjectionError::ContractValidation {
                message: format!(
                    "{context} effect source module {module_path} is missing from File IR"
                ),
            }
        })
    }
}
