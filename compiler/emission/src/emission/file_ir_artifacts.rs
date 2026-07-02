use crate::projection::{ProjectionSourceMetadata, ProjectionView};
use crate::{
    emission::artifact::PublishedFileIrArtifact,
    error::{EmissionError, Result},
};
use skiff_artifact_model::FileIrUnit;
use skiff_compiler_core::{
    json_utils::{canonical_json_value, sha256_hex},
    source_role::PublicationSourceRole,
};

pub fn published_file_ir_artifacts_from_projection_input(
    input: ProjectionView<'_>,
) -> Result<Vec<PublishedFileIrArtifact>> {
    projection_input_units_with_sources(input)?
        .into_iter()
        .map(|(unit, source)| published_compiled_file_ir_artifact(unit, source))
        .collect()
}

pub fn published_file_ir_artifacts_from_units_with_projection_sources(
    units: &[FileIrUnit],
    input: ProjectionView<'_>,
) -> Result<Vec<PublishedFileIrArtifact>> {
    let sources = input.source_metadata();
    if units.len() != sources.len() {
        return Err(EmissionError::ContractValidation {
            message: format!(
                "publication has {} File IR units but {} source metadata entries",
                units.len(),
                sources.len()
            ),
        });
    }
    units
        .iter()
        .zip(sources.iter())
        .map(|(unit, source)| published_compiled_file_ir_artifact(unit, source))
        .collect()
}

fn published_compiled_file_ir_artifact(
    unit: &FileIrUnit,
    source: &ProjectionSourceMetadata,
) -> Result<PublishedFileIrArtifact> {
    validate_compiled_source_matches_file_ir(unit, source)?;
    Ok(published_file_ir_artifact_from_unit(
        unit,
        source.source_path.clone(),
        source.module_path.clone(),
        file_ir_role_for_source_role(source.role).to_string(),
    ))
}

fn file_ir_role_for_source_role(role: PublicationSourceRole) -> &'static str {
    match role {
        PublicationSourceRole::Contract => "contract",
        PublicationSourceRole::Implementation => "implementation",
        PublicationSourceRole::Package => "package",
    }
}

pub fn published_file_ir_artifact_from_unit(
    unit: &FileIrUnit,
    source_path: String,
    module_path: String,
    role: String,
) -> PublishedFileIrArtifact {
    let hash = file_ir_artifact_hash(unit);
    PublishedFileIrArtifact {
        unit: unit.clone(),
        identity: unit.file_ir_identity.clone(),
        hash: hash.clone(),
        path: format!("units/files/{hash}.json"),
        source_path,
        module_path,
        role,
    }
}

pub fn file_ir_artifact_hash(unit: &FileIrUnit) -> String {
    let value = serde_json::to_value(unit).expect("FileIrUnit artifact payload must serialize");
    let canonical = canonical_json_value(&value);
    let bytes =
        serde_json::to_vec(&canonical).expect("canonical FileIrUnit artifact must serialize");
    sha256_hex(&bytes)
}

fn projection_input_units_with_sources(
    input: ProjectionView<'_>,
) -> Result<Vec<(&FileIrUnit, &ProjectionSourceMetadata)>> {
    let sources = input.source_metadata();
    if input.file_ir_units().len() != sources.len() {
        return Err(EmissionError::ContractValidation {
            message: format!(
                "compiled publication has {} File IR units but {} source metadata entries",
                input.file_ir_units().len(),
                sources.len()
            ),
        });
    }
    Ok(input.file_ir_units().iter().zip(sources.iter()).collect())
}

fn validate_compiled_source_matches_file_ir(
    unit: &FileIrUnit,
    source: &ProjectionSourceMetadata,
) -> Result<()> {
    let unit_source = file_ir_unit_source(unit)?;
    if unit_source.path != source.source_path || unit_source.module_path != source.module_path {
        return Err(EmissionError::ContractValidation {
            message: format!(
                "compiled source metadata {} ({}) does not match File IR source {} ({})",
                source.source_path, source.module_path, unit_source.path, unit_source.module_path
            ),
        });
    }
    Ok(())
}

struct FileIrSourceMetadata {
    path: String,
    module_path: String,
}

fn file_ir_unit_source(unit: &FileIrUnit) -> Result<FileIrSourceMetadata> {
    unit.source_map.sources.first().map_or_else(
        || {
            Err(EmissionError::ContractValidation {
                message: format!(
                    "compiled File IR unit {} has no source map source",
                    unit.file_ir_identity
                ),
            })
        },
        |source| {
            Ok(FileIrSourceMetadata {
                path: source.path.clone(),
                module_path: source.module_path.clone(),
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::file_ir_artifact_hash;
    use crate::emission::identity::assign_file_ir_identity;
    use skiff_artifact_model::FileIrUnit;

    #[test]
    fn artifact_hash_includes_source_ast_hash_fields() {
        let mut left = FileIrUnit::empty("surface", "source-ast-hash-a");
        assign_file_ir_identity(&mut left).expect("left identity");
        let mut right = FileIrUnit::empty("surface", "source-ast-hash-b");
        assign_file_ir_identity(&mut right).expect("right identity");

        assert_eq!(left.file_ir_identity, right.file_ir_identity);
        assert_ne!(file_ir_artifact_hash(&left), file_ir_artifact_hash(&right));
    }
}
