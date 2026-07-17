use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

use crate::{ArtifactIdentityError, Result};

/// A canonical `/`-separated path below one artifact root.
///
/// This type is the trust boundary for every path carried by an artifact
/// pointer. Construction rejects host-dependent spellings before any join or
/// filesystem read is attempted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtifactRelativePath(String);

impl ArtifactRelativePath {
    pub fn new(path: impl AsRef<Path>, label: impl Into<String>) -> Result<Self> {
        let label = label.into();
        let path = path.as_ref();
        let Some(path) = path.to_str() else {
            return Err(ArtifactIdentityError::PathEscape {
                label,
                path: path.to_string_lossy().into_owned(),
            });
        };
        Self::parse(path, label)
    }

    pub fn parse(path: &str, label: impl Into<String>) -> Result<Self> {
        let label = label.into();
        let windows_drive = path.as_bytes().get(1) == Some(&b':')
            && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
        if path.is_empty()
            || path.starts_with('/')
            || windows_drive
            || path.contains('\\')
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(ArtifactIdentityError::PathEscape {
                label,
                path: path.to_string(),
            });
        }
        Ok(Self(path.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    pub fn display(&self) -> &str {
        &self.0
    }

    pub fn resolve_existing(&self, artifact_root: &Path, label: &str) -> Result<PathBuf> {
        let root = fs::canonicalize(artifact_root).map_err(|source| {
            ArtifactIdentityError::ResolveArtifactRoot {
                path: artifact_root.display().to_string(),
                source,
            }
        })?;
        let joined = root.join(self.as_path());
        let resolved = fs::canonicalize(&joined).map_err(|source| {
            ArtifactIdentityError::ResolveArtifactPath {
                path: joined.display().to_string(),
                source,
            }
        })?;
        if !resolved.starts_with(&root) {
            return Err(ArtifactIdentityError::ArtifactPathEscapesRoot {
                label: label.to_string(),
                path: self.0.clone(),
                root: root.display().to_string(),
            });
        }
        Ok(resolved)
    }
}

impl fmt::Display for ArtifactRelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
