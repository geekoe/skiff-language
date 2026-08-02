//! Strict reader/loader for one immutable actor routing projection record.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use skiff_artifact_identity::{ArtifactIdentityError, ArtifactRelativePath};
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};

use super::{catalog::ActorRoutingCatalog, strict_json::strict_value};

/// Per-record byte budget for the actor routing projection.
///
/// Aligned with the C-model-artifact snapshot budget (16 MiB); the projection
/// is a small canonical record, so this is a fail-closed safety ceiling.
pub const MAX_ACTOR_ROUTING_PROJECTION_RECORD_BYTES: u64 = 16 * 1024 * 1024;

/// Consumer-side exact reference to one actor routing projection record.
///
/// A0 freezes the projection data contract but not the bootstrap/artifact refs
/// (A0 §1); the canonical record identity / path derivation belongs to the A1
/// producer output surface. This reference carries only an escape-proof
/// relative record path; the reader never guesses or derives paths itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorRoutingProjectionRef {
    pub record_path: ArtifactRelativePath,
}

impl ActorRoutingProjectionRef {
    pub fn new(record_path: ArtifactRelativePath) -> Self {
        Self { record_path }
    }
}

/// Filesystem store for immutable actor routing projection records.
///
/// The store is `Send + Sync`; blocking reads are intended to run through the
/// bounded blocking pool owned by W-bootstrap (C-bootstrap §2.3).
#[derive(Debug, Clone)]
pub struct ActorRoutingProjectionStore {
    root: PathBuf,
}

impl ActorRoutingProjectionStore {
    /// Opens a canonical artifact root directory.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ActorRoutingProjectionError> {
        let root = root.as_ref();
        let canonical =
            fs::canonicalize(root).map_err(|source| ActorRoutingProjectionError::InvalidRoot {
                path: root.to_path_buf(),
                source,
            })?;
        if !canonical.is_dir() {
            return Err(ActorRoutingProjectionError::RootNotDirectory { path: canonical });
        }
        Ok(Self { root: canonical })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Strictly loads and validates one actor routing projection record.
    ///
    /// The full fail-closed chain is: escape-proof path resolution → bounded
    /// read → duplicate-key-free strict JSON → exact schema version → typed
    /// deserialization (`deny_unknown_fields` + construction invariants) →
    /// canonical bytes equality. File IR / source / executable payload fields
    /// are rejected by the frozen projection shape at the typed boundary.
    pub fn load(
        &self,
        reference: &ActorRoutingProjectionRef,
    ) -> Result<Arc<ActorRoutingProjection>, ActorRoutingProjectionError> {
        let resolved = reference
            .record_path
            .resolve_existing(&self.root, "actor routing projection record")
            .map_err(ActorRoutingProjectionError::from_path_error)?;
        let bytes = fs::read(&resolved).map_err(|source| ActorRoutingProjectionError::Io {
            path: resolved.clone(),
            source,
        })?;
        if bytes.len() as u64 > MAX_ACTOR_ROUTING_PROJECTION_RECORD_BYTES {
            return Err(ActorRoutingProjectionError::RecordTooLarge {
                path: resolved,
                limit: MAX_ACTOR_ROUTING_PROJECTION_RECORD_BYTES,
            });
        }
        let value =
            strict_value(&bytes).map_err(|source| ActorRoutingProjectionError::Malformed {
                path: resolved.clone(),
                source,
            })?;
        let actual_schema = value
            .get("schemaVersion")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        if actual_schema != ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION {
            return Err(ActorRoutingProjectionError::SchemaVersionMismatch {
                path: resolved.clone(),
                actual: actual_schema,
            });
        }
        let projection: ActorRoutingProjection =
            serde_json::from_value(value).map_err(|source| {
                ActorRoutingProjectionError::InvalidProjection {
                    path: resolved.clone(),
                    source,
                }
            })?;
        let canonical = canonical_json_bytes(&projection).map_err(|source| {
            ActorRoutingProjectionError::InvalidProjection {
                path: resolved.clone(),
                source,
            }
        })?;
        if canonical != bytes {
            return Err(ActorRoutingProjectionError::NonCanonical { path: resolved });
        }
        Ok(Arc::new(projection))
    }

    /// Loads the projection and builds the immutable actor routing catalog.
    ///
    /// This is the artifact loader seam used when constructing a candidate
    /// `RoutingEpoch`: the catalog is built once from the validated projection
    /// and belongs to the immutable epoch (plan §3.3).
    pub fn load_catalog(
        &self,
        reference: &ActorRoutingProjectionRef,
    ) -> Result<Arc<ActorRoutingCatalog>, ActorRoutingProjectionError> {
        let projection = self.load(reference)?;
        Ok(Arc::new(ActorRoutingCatalog::from_projection(projection)))
    }
}

/// Fail-closed errors of the actor routing projection strict reader.
#[derive(Debug, thiserror::Error)]
pub enum ActorRoutingProjectionError {
    #[error("failed to resolve actor routing projection artifact root {path}: {source}")]
    InvalidRoot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("actor routing projection artifact root {path} is not a directory")]
    RootNotDirectory { path: PathBuf },
    #[error("actor routing projection record path is invalid: {message}")]
    InvalidRecordPath { message: String },
    #[error("actor routing projection record {path} is missing")]
    MissingRecord { path: PathBuf },
    #[error("actor routing projection record {path} exceeds the {limit} byte budget")]
    RecordTooLarge { path: PathBuf, limit: u64 },
    #[error("actor routing projection record {path} is not strict JSON: {source}")]
    Malformed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("actor routing projection record {path} has unsupported schemaVersion {actual:?}")]
    SchemaVersionMismatch { path: PathBuf, actual: String },
    #[error("actor routing projection record {path} failed typed validation: {source}")]
    InvalidProjection {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("actor routing projection record {path} is not canonical JSON")]
    NonCanonical { path: PathBuf },
    #[error("failed to read actor routing projection record {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl ActorRoutingProjectionError {
    fn from_path_error(error: ArtifactIdentityError) -> Self {
        match error {
            ArtifactIdentityError::ResolveArtifactPath { path, source }
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Self::MissingRecord {
                    path: PathBuf::from(path),
                }
            }
            other => Self::InvalidRecordPath {
                message: other.to_string(),
            },
        }
    }
}
