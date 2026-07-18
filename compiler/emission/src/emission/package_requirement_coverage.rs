use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{PackageArtifact, PackageRefIr};

use crate::{
    emission::artifact::PublishedFileIrArtifact,
    error::{EmissionError, Result},
};

pub(super) fn validate_file_ir_package_requirement_coverage(
    artifact: &PackageArtifact,
    files: &[PublishedFileIrArtifact],
) -> Result<()> {
    let mut package_ids = BTreeSet::new();
    let mut aliases = BTreeMap::new();
    for requirement in &artifact.package_requirements {
        package_ids.insert(requirement.package_id.as_str());
        if aliases
            .insert(requirement.alias.as_str(), requirement.package_id.as_str())
            .is_some()
        {
            return Err(validation_error(format!(
                "package {}@{} contains duplicate package requirement alias {}",
                artifact.package_id, artifact.package_version, requirement.alias
            )));
        }
    }

    for file in files {
        for symbol in &file.unit.external_refs.package_symbols {
            validate_package_ref(
                artifact,
                &file.unit.module_path,
                "packageSymbols",
                &symbol.package,
                &aliases,
                &package_ids,
            )?;
        }
        for callable in &file.unit.external_refs.package_callables {
            validate_package_ref(
                artifact,
                &file.unit.module_path,
                "packageCallables",
                &callable.package_ref,
                &aliases,
                &package_ids,
            )?;
        }
    }
    Ok(())
}

fn validate_package_ref(
    artifact: &PackageArtifact,
    module_path: &str,
    table: &str,
    package_ref: &PackageRefIr,
    aliases: &BTreeMap<&str, &str>,
    package_ids: &BTreeSet<&str>,
) -> Result<()> {
    match package_ref {
        PackageRefIr::Dependency { dependency_ref } => {
            let package_id = aliases.get(dependency_ref.as_str()).ok_or_else(|| {
                validation_error(format!(
                    "package {}@{} File IR module {module_path} {table} references unknown package dependency alias {dependency_ref}",
                    artifact.package_id, artifact.package_version
                ))
            })?;
            if *package_id == artifact.package_id {
                return Err(external_self_ref_error(
                    artifact,
                    module_path,
                    table,
                    format!("dependency alias {dependency_ref}"),
                ));
            }
        }
        PackageRefIr::PackageId { package_id } => {
            if package_id == &artifact.package_id {
                return Err(external_self_ref_error(
                    artifact,
                    module_path,
                    table,
                    format!("package id {package_id}"),
                ));
            }
            if !package_ids.contains(package_id.as_str()) {
                return Err(validation_error(format!(
                    "package {}@{} File IR module {module_path} {table} references unknown package id {package_id}",
                    artifact.package_id, artifact.package_version
                )));
            }
        }
    }
    Ok(())
}

fn external_self_ref_error(
    artifact: &PackageArtifact,
    module_path: &str,
    table: &str,
    coordinate: String,
) -> EmissionError {
    validation_error(format!(
        "package {}@{} File IR module {module_path} {table} contains unrewritten external self reference through {coordinate}",
        artifact.package_id, artifact.package_version
    ))
}

fn validation_error(message: String) -> EmissionError {
    EmissionError::ContractValidation { message }
}
