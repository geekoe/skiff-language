use std::{
    fmt, fs,
    path::{Component, Display, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtifactRootRelativePath {
    path: PathBuf,
}

impl ArtifactRootRelativePath {
    pub fn new(path: impl AsRef<Path>, label: &str) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !is_safe_artifact_root_relative_path(path) {
            anyhow::bail!(
                "{} path {} must be relative and stay inside artifacts root",
                label,
                path.display()
            );
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub fn parse(path: &str, label: &str) -> anyhow::Result<Self> {
        Self::new(Path::new(path), label)
    }

    pub fn as_path(&self) -> &Path {
        &self.path
    }

    pub fn display(&self) -> Display<'_> {
        self.path.display()
    }
}

impl fmt::Display for ArtifactRootRelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.path.display())
    }
}

pub fn resolve_index_artifact_path(
    artifact_root: &Path,
    artifact_path: &ArtifactRootRelativePath,
    label: &str,
) -> anyhow::Result<PathBuf> {
    let root = fs::canonicalize(artifact_root).map_err(|error| {
        anyhow::anyhow!(
            "failed to resolve artifacts root {}: {error}",
            artifact_root.display()
        )
    })?;
    let path = root.join(artifact_path.as_path());
    let canonical_path = fs::canonicalize(&path)
        .map_err(|error| anyhow::anyhow!("failed to resolve {}: {error}", path.display()))?;
    if !canonical_path.starts_with(&root) {
        anyhow::bail!(
            "{} path {} escapes artifacts root {}",
            label,
            artifact_path,
            root.display()
        );
    }

    Ok(canonical_path)
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
    validate_publication_id(value, label)?;
    Ok(value.replace('.', "~").replace('/', "~~"))
}

fn validate_publication_id(value: &str, label: &str) -> anyhow::Result<()> {
    if value.is_empty() || value.len() > 63 || value == "std" {
        anyhow::bail!("{label} {value} must be a publication id");
    }
    if value != value.trim()
        || value.bytes().any(|byte| byte.is_ascii_control())
        || value.contains("://")
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
        || value.contains('~')
        || value
            .bytes()
            .any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
    {
        anyhow::bail!("{label} {value} must be a publication id");
    }

    let Some((authority, local)) = value.split_once('/') else {
        anyhow::bail!("{label} {value} must be a publication id");
    };
    validate_authority(authority, label, value)?;
    if local.is_empty()
        || local
            .split('/')
            .any(|segment| !is_valid_local_segment(segment))
    {
        anyhow::bail!("{label} {value} must be a publication id");
    }
    Ok(())
}

fn validate_authority(authority: &str, label: &str, value: &str) -> anyhow::Result<()> {
    let labels = authority.split('.').collect::<Vec<_>>();
    if labels.len() < 2 || labels.iter().any(|item| !is_valid_authority_label(item)) {
        anyhow::bail!("{label} {value} must be a publication id");
    }
    Ok(())
}

fn is_valid_authority_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes[0] != b'-'
        && bytes.last() != Some(&b'-')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_valid_local_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes.last() != Some(&b'-')
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
        })
}

fn is_safe_artifact_root_relative_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return false;
    }
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
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
