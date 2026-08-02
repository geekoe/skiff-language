//! Committed-activation bootstrap reader (C-bootstrap §2.1, plan §7
//! E-bootstrap).
//!
//! The reader is the read-only repository port: it consumes the
//! W-activation-state `ActivationStateRepository` read side, projects a
//! durable `CommittedActivation` into shared refs, and fail-closes on
//! missing/malformed/pending/identity-mismatch/repository failures. It never
//! writes, CASes or stages; complete recovery belongs to E-activation.

use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use skiff_artifact_model::{RuntimeAssemblyRef, RuntimeConfigSnapshotRef};
use skiff_deployment::storage::{CanonicalArtifactStore, CommittedActivation};

use crate::activation::{ActivationStateRepository, RepositoryError};

use super::loader::{BlockingLoader, BlockingLoaderError};

/// Shared refs projected from a durable committed activation (C-bootstrap
/// §2.2): the durable→shared projection is total for a committed record and
/// never projects a pending candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedBootstrapRefs {
    pub generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub config_snapshot: RuntimeConfigSnapshotRef,
}

impl CommittedBootstrapRefs {
    pub fn project_committed(committed: &CommittedActivation) -> Self {
        Self {
            generation: committed.generation,
            assembly: committed.assembly.clone(),
            config_snapshot: committed.config_snapshot.clone(),
        }
    }
}

/// Closed read outcomes of the committed bootstrap reader.
///
/// The five durable-state outcomes follow C-bootstrap §2.1;
/// `FailClosedRepository` is the W-bootstrap alignment for the repository
/// port's transient/closed infrastructure failures (documented in the leaf).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapReadOutcome {
    StableCommitted {
        generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
    },
    FailClosedPending {
        activation_id: String,
    },
    FailClosedMissing,
    FailClosedMalformed {
        message: String,
    },
    FailClosedIdentityMismatch {
        message: String,
    },
    FailClosedRepository {
        message: String,
    },
}

impl BootstrapReadOutcome {
    pub fn is_stable(&self) -> bool {
        matches!(self, Self::StableCommitted { .. })
    }

    pub fn refs(&self) -> Option<CommittedBootstrapRefs> {
        match self {
            Self::StableCommitted {
                generation,
                assembly,
                config_snapshot,
            } => Some(CommittedBootstrapRefs {
                generation: *generation,
                assembly: assembly.clone(),
                config_snapshot: config_snapshot.clone(),
            }),
            _ => None,
        }
    }
}

impl fmt::Display for BootstrapReadOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StableCommitted { generation, .. } => {
                write!(formatter, "stable committed generation {generation}")
            }
            Self::FailClosedPending { activation_id } => {
                write!(formatter, "fail closed: pending activation {activation_id}")
            }
            Self::FailClosedMissing => write!(formatter, "fail closed: activation state missing"),
            Self::FailClosedMalformed { message } => {
                write!(
                    formatter,
                    "fail closed: malformed activation state: {message}"
                )
            }
            Self::FailClosedIdentityMismatch { message } => {
                write!(
                    formatter,
                    "fail closed: committed reference mismatch: {message}"
                )
            }
            Self::FailClosedRepository { message } => {
                write!(
                    formatter,
                    "fail closed: activation repository failure: {message}"
                )
            }
        }
    }
}

/// Reference-validation seam: identity checks are performed by the owner
/// store (`CanonicalArtifactStore::read_runtime_assembly`), never re-implemented
/// by the port.
pub trait CommittedRefValidator: Send + Sync {
    fn validate_committed(&self, refs: &CommittedBootstrapRefs) -> Result<(), String>;
}

/// Validator over the canonical artifact store's strict assembly reader.
#[derive(Debug, Clone)]
pub struct CanonicalCommittedRefValidator {
    store: CanonicalArtifactStore,
}

impl CanonicalCommittedRefValidator {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        CanonicalArtifactStore::open(root)
            .map(|store| Self { store })
            .map_err(|error| format!("open canonical artifact store: {error}"))
    }
}

impl CommittedRefValidator for CanonicalCommittedRefValidator {
    fn validate_committed(&self, refs: &CommittedBootstrapRefs) -> Result<(), String> {
        self.store
            .read_runtime_assembly(&refs.assembly)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// Fail-closed counters (`bootstrapReader.failClosed.*`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReaderFailClosedCounters {
    pub missing: u64,
    pub malformed: u64,
    pub identity_mismatch: u64,
    pub pending: u64,
    pub repository: u64,
}

/// Read-only committed bootstrap reader.
pub struct CommittedActivationBootstrapReader {
    repository: Arc<dyn ActivationStateRepository>,
    validator: Arc<dyn CommittedRefValidator>,
    loader: Arc<BlockingLoader>,
    missing: AtomicU64,
    malformed: AtomicU64,
    identity_mismatch: AtomicU64,
    pending: AtomicU64,
    repository_failures: AtomicU64,
}

impl CommittedActivationBootstrapReader {
    pub fn new(
        repository: Arc<dyn ActivationStateRepository>,
        validator: Arc<dyn CommittedRefValidator>,
        loader: Arc<BlockingLoader>,
    ) -> Self {
        Self {
            repository,
            validator,
            loader,
            missing: AtomicU64::new(0),
            malformed: AtomicU64::new(0),
            identity_mismatch: AtomicU64::new(0),
            pending: AtomicU64::new(0),
            repository_failures: AtomicU64::new(0),
        }
    }

    /// Reads one committed environment and fail-closes on every non-stable
    /// outcome. Each call independently validates the repository read and the
    /// committed refs (no cache).
    pub async fn read_committed(&self, environment: &str) -> BootstrapReadOutcome {
        let state = match self.repository.read(environment).await {
            Ok(state) => state,
            Err(RepositoryError::CasMismatch { .. }) => {
                self.missing.fetch_add(1, Ordering::Relaxed);
                return BootstrapReadOutcome::FailClosedMissing;
            }
            Err(RepositoryError::InvalidRecord { message, .. }) => {
                self.malformed.fetch_add(1, Ordering::Relaxed);
                return BootstrapReadOutcome::FailClosedMalformed { message };
            }
            Err(RepositoryError::Transient { message }) => {
                self.repository_failures.fetch_add(1, Ordering::Relaxed);
                return BootstrapReadOutcome::FailClosedRepository { message };
            }
            Err(RepositoryError::Closed) => {
                self.repository_failures.fetch_add(1, Ordering::Relaxed);
                return BootstrapReadOutcome::FailClosedRepository {
                    message: "activation state repository is closed".to_string(),
                };
            }
        };
        if let Some(pending) = &state.pending {
            self.pending.fetch_add(1, Ordering::Relaxed);
            return BootstrapReadOutcome::FailClosedPending {
                activation_id: pending.activation_id.clone(),
            };
        }
        let refs = CommittedBootstrapRefs::project_committed(&state.committed);
        let validator = Arc::clone(&self.validator);
        let validator_refs = refs.clone();
        let validation = self
            .loader
            .run(move || validator.validate_committed(&validator_refs))
            .await;
        match validation {
            Ok(()) => BootstrapReadOutcome::StableCommitted {
                generation: refs.generation,
                assembly: refs.assembly,
                config_snapshot: refs.config_snapshot,
            },
            Err(BlockingLoaderError::Operation(message)) => {
                self.identity_mismatch.fetch_add(1, Ordering::Relaxed);
                BootstrapReadOutcome::FailClosedIdentityMismatch { message }
            }
            Err(error) => {
                self.repository_failures.fetch_add(1, Ordering::Relaxed);
                BootstrapReadOutcome::FailClosedRepository {
                    message: error.to_string(),
                }
            }
        }
    }

    pub fn fail_closed(&self) -> ReaderFailClosedCounters {
        ReaderFailClosedCounters {
            missing: self.missing.load(Ordering::Relaxed),
            malformed: self.malformed.load(Ordering::Relaxed),
            identity_mismatch: self.identity_mismatch.load(Ordering::Relaxed),
            pending: self.pending.load(Ordering::Relaxed),
            repository: self.repository_failures.load(Ordering::Relaxed),
        }
    }
}

impl fmt::Debug for CommittedActivationBootstrapReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommittedActivationBootstrapReader")
            .field("fail_closed", &self.fail_closed())
            .finish()
    }
}
