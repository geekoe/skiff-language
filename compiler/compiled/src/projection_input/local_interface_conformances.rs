use std::collections::BTreeMap;

use skiff_artifact_model::{ExecutableKind, FileIrUnit};
use skiff_compiler_projection_input::{
    ProjectionExecutableKey, ProjectionLocalInterfaceConformance,
    ProjectionLocalInterfaceConformanceFacts, ProjectionSourceSymbolKey,
};
use skiff_compiler_source::{PackageSourceModel, SourceSymbolKey};

use crate::ProjectionInputBuildError;

pub(super) fn build(
    model: &PackageSourceModel,
    file_ir_units: &[FileIrUnit],
) -> Result<ProjectionLocalInterfaceConformanceFacts, ProjectionInputBuildError> {
    // Source owns the conformance set, binders, identities, and slot order.
    // File IR is used only to address each exact implementation source key.
    let source_facts = model.local_interface_conformance_facts()?;
    let mut units_by_module = BTreeMap::new();
    for unit in file_ir_units {
        if units_by_module
            .insert(unit.module_path.as_str(), unit)
            .is_some()
        {
            return Err(
                ProjectionInputBuildError::DuplicateLocalInterfaceFileIrModule {
                    module_path: unit.module_path.clone(),
                },
            );
        }
    }
    let entries = source_facts
        .iter()
        .map(|conformance| {
            let implementation_executables = conformance
                .implementation_methods()
                .iter()
                .enumerate()
                .map(|(slot, implementation)| {
                    implementation_executable(&units_by_module, slot, implementation)
                })
                .collect::<Result<Vec<_>, ProjectionInputBuildError>>()?;
            Ok(ProjectionLocalInterfaceConformance::try_new(
                conformance.receiver_type_parameters().to_vec(),
                ProjectionSourceSymbolKey::new(
                    conformance.receiver().module_path(),
                    conformance.receiver().symbol(),
                ),
                conformance.interface().clone(),
                implementation_executables,
            )?)
        })
        .collect::<Result<Vec<_>, ProjectionInputBuildError>>()?;
    Ok(ProjectionLocalInterfaceConformanceFacts::try_from_entries(
        entries,
    )?)
}

fn implementation_executable(
    units_by_module: &BTreeMap<&str, &FileIrUnit>,
    slot: usize,
    implementation: &SourceSymbolKey,
) -> Result<ProjectionExecutableKey, ProjectionInputBuildError> {
    let unit = units_by_module
        .get(implementation.module_path())
        .copied()
        .ok_or_else(
            || ProjectionInputBuildError::MissingLocalInterfaceImplementationModule {
                slot,
                implementation_module: implementation.module_path().to_string(),
                implementation_symbol: implementation.symbol().to_string(),
            },
        )?;
    let declaration = unit
        .declarations
        .executables
        .get(implementation.symbol())
        .ok_or_else(|| {
            ProjectionInputBuildError::MissingLocalInterfaceImplementationDeclaration {
                slot,
                implementation_module: implementation.module_path().to_string(),
                implementation_symbol: implementation.symbol().to_string(),
            }
        })?;
    let expected_symbol = format!(
        "{}.{}",
        implementation.module_path(),
        implementation.symbol()
    );
    if declaration.symbol != expected_symbol {
        return Err(
            ProjectionInputBuildError::LocalInterfaceImplementationDeclarationSymbolMismatch {
                module_path: implementation.module_path().to_string(),
                source_symbol: implementation.symbol().to_string(),
                expected_symbol,
                actual_symbol: declaration.symbol.clone(),
            },
        );
    }
    let executable = unit
        .executables
        .get(declaration.executable_index as usize)
        .ok_or_else(|| {
            ProjectionInputBuildError::MissingLocalInterfaceImplementationExecutable {
                module_path: implementation.module_path().to_string(),
                source_symbol: implementation.symbol().to_string(),
                executable_index: declaration.executable_index,
            }
        })?;
    if executable.symbol != declaration.symbol {
        return Err(
            ProjectionInputBuildError::LocalInterfaceImplementationExecutableSymbolMismatch {
                module_path: implementation.module_path().to_string(),
                source_symbol: implementation.symbol().to_string(),
                declaration_symbol: declaration.symbol.clone(),
                executable_symbol: executable.symbol.clone(),
            },
        );
    }
    if executable.kind != ExecutableKind::ImplMethod {
        return Err(
            ProjectionInputBuildError::LocalInterfaceImplementationKindMismatch {
                module_path: implementation.module_path().to_string(),
                source_symbol: implementation.symbol().to_string(),
                actual_kind: executable.kind,
            },
        );
    }
    Ok(ProjectionExecutableKey::new(
        implementation.module_path(),
        declaration.executable_index,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn implementation() -> SourceSymbolKey {
        SourceSymbolKey::new("models", "Box<T>.zeta")
    }

    #[test]
    fn missing_implementation_module_is_structured_and_never_defaulted() {
        let error = implementation_executable(&BTreeMap::new(), 0, &implementation()).unwrap_err();

        assert!(matches!(
            error,
            ProjectionInputBuildError::MissingLocalInterfaceImplementationModule {
                slot: 0,
                implementation_module,
                implementation_symbol,
            } if implementation_module == "models"
                && implementation_symbol == "Box<T>.zeta"
        ));
    }

    #[test]
    fn missing_implementation_declaration_is_structured_and_never_guessed() {
        let unit = FileIrUnit::empty("models", "source-hash");
        let units = BTreeMap::from([("models", &unit)]);
        let error = implementation_executable(&units, 1, &implementation()).unwrap_err();

        assert!(matches!(
            error,
            ProjectionInputBuildError::MissingLocalInterfaceImplementationDeclaration {
                slot: 1,
                implementation_module,
                implementation_symbol,
            } if implementation_module == "models"
                && implementation_symbol == "Box<T>.zeta"
        ));
    }

    #[test]
    fn missing_indexed_executable_is_structured_and_never_projected() {
        let mut unit = FileIrUnit::empty("models", "source-hash");
        unit.declarations.executables.insert(
            "Box<T>.zeta".to_string(),
            skiff_artifact_model::ExecutableDeclarationIr {
                executable_index: 7,
                symbol: "models.Box<T>.zeta".to_string(),
                source_span: None,
            },
        );
        let units = BTreeMap::from([("models", &unit)]);
        let error = implementation_executable(&units, 0, &implementation()).unwrap_err();

        assert!(matches!(
            error,
            ProjectionInputBuildError::MissingLocalInterfaceImplementationExecutable {
                module_path,
                source_symbol,
                executable_index: 7,
            } if module_path == "models" && source_symbol == "Box<T>.zeta"
        ));
    }
}
