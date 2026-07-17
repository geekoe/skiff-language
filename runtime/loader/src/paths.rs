use std::path::{Path, PathBuf};

pub use skiff_artifact_identity::ArtifactRelativePath as ArtifactRootRelativePath;

pub fn resolve_index_artifact_path(
    artifact_root: &Path,
    artifact_path: &ArtifactRootRelativePath,
    label: &str,
) -> anyhow::Result<PathBuf> {
    artifact_path
        .resolve_existing(artifact_root, label)
        .map_err(anyhow::Error::from)
}

pub fn service_id_artifact_path(service_id: &str) -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from(publication_storage_segment(
        service_id,
        "serviceId",
    )?))
}

pub(crate) fn service_id_artifact_json_path(service_id: &str) -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from(format!(
        "{}.json",
        publication_storage_segment(service_id, "serviceId")?
    )))
}

pub(crate) fn publication_storage_segment(value: &str, label: &str) -> anyhow::Result<String> {
    skiff_artifact_identity::publication_storage_segment(value, label).map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests {
    use super::publication_storage_segment;

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PublicationIdFixture {
        schema_version: u32,
        encoding: String,
        max_bytes: usize,
        valid: Vec<PublicationIdCase>,
        invalid: Vec<InvalidPublicationIdCase>,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PublicationIdCase {
        canonical_id: String,
        runtime_target_component: String,
        applies_to: Vec<String>,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct InvalidPublicationIdCase {
        id: String,
        applies_to: Vec<String>,
    }

    fn publication_id_fixture() -> PublicationIdFixture {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("runtime crate should live under the skiff repository root")
            .join("cross-system-fixtures/publication-id-cases.json");
        let text =
            std::fs::read_to_string(&path).expect("publication id fixture should be readable");
        let fixture: PublicationIdFixture =
            serde_json::from_str(&text).expect("publication id fixture should parse");
        assert_eq!(fixture.schema_version, 1);
        assert_eq!(fixture.encoding, "url-like-with-storage-safe-projection");
        assert_eq!(fixture.max_bytes, 63);
        fixture
    }

    #[test]
    fn publication_id_fixture_projects_runtime_artifact_paths() {
        let fixture = publication_id_fixture();
        for case in fixture
            .valid
            .iter()
            .filter(|case| case.applies_to.iter().any(|system| system == "runtime"))
        {
            let segment = publication_storage_segment(&case.canonical_id, "publication id")
                .expect("runtime publication id should project to storage segment");

            assert_eq!(segment, case.runtime_target_component);
        }
    }

    #[test]
    fn publication_id_fixture_rejects_invalid_runtime_artifact_paths() {
        let fixture = publication_id_fixture();
        for case in fixture
            .invalid
            .iter()
            .filter(|case| case.applies_to.iter().any(|system| system == "runtime"))
        {
            assert!(
                publication_storage_segment(&case.id, "publication id").is_err(),
                "{:?} should be rejected for runtime artifact paths",
                case.id
            );
        }
    }

    #[test]
    fn publication_id_requires_authority_path_for_runtime_artifact_paths() {
        for value in ["billing", "skiff.run"] {
            assert!(
                publication_storage_segment(value, "publication id").is_err(),
                "{value:?} should not be a runtime artifact publication id"
            );
        }
    }
}
