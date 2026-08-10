//! Phase 2 bytecode compilation handoff.
//!
//! This is an in-memory compiler boundary, not an emitter and not an artifact
//! writer. It accepts an emission-produced [`BytecodeArtifact`], its exact,
//! path-free [`BytecodeArtifactRef`], and the package-owned statement manifest
//! produced independently of the bytecode rows. Construction runs C1-C9
//! admission, validates the package-bound manifest identity, and exact-joins
//! both inputs by function origin. It never opens File IR or reconstructs
//! source, type, liveness, effect, or relocation facts.
//!
//! The path-free reference is deliberate. The upper publication owner can
//! first use it in the package identity projection, derive the exact package
//! record family, write the bytecode record, attach the canonical returned
//! path, and only then write the PackageArtifact record. No store I/O or
//! PackageArtifact mutation occurs here, so a failed enabled emission cannot
//! be hidden by a partial write or an implicit disabled/legacy fallback.

use std::collections::BTreeMap;

use skiff_artifact_identity::{validate_bytecode_identity, ArtifactIdentityError};
use skiff_artifact_model::{
    validate_bytecode_statement_manifest_identity, BytecodeArtifact, BytecodeArtifactRef,
    BytecodeFunctionOrigin, BytecodeFunctionStatementManifest, BytecodeStatementManifestIdentity,
    HostEffectRegistryIdentity, IntrinsicRegistryIdentity, NativeValueLifecycleRegistryIdentity,
    StatementManifestIdentityError, ValueLifecyclePolicyIdentity,
};
use thiserror::Error;

/// An admitted emission result ready for fail-closed publication planning.
///
/// Fields are private so the artifact, reference, and receipt cannot drift
/// apart after [`Self::try_new`] validates them.
#[derive(Debug, Clone, PartialEq)]
pub struct BytecodeCompilationHandoff {
    artifact: BytecodeArtifact,
    reference: BytecodeArtifactRef,
    receipt: BytecodeCompilationReceipt,
    statement_manifest: BytecodeStatementManifestHandoff,
}

/// Package-owned statement authority retained inside the compilation handoff.
///
/// This helper is deliberately private: callers cannot detach it and pair it
/// with another admitted bytecode image. Its rows remain available read-only
/// through [`BytecodeCompilationHandoff::statement_manifest`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct BytecodeStatementManifestHandoff {
    functions: Vec<BytecodeFunctionStatementManifest>,
    receipt: BytecodeStatementManifestReceipt,
}

/// Canonical evidence derived from the admitted bytecode artifact.
///
/// This is an emission receipt, not proof that any artifact-store write has
/// happened. In particular it contains no record path. The publication owner
/// must obtain that path from its successful bytecode write before publishing
/// a PackageArtifact that references the bytecode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytecodeCompilationReceipt {
    bytecode_identity: String,
    schema_version: String,
    isa_version: String,
    opcode_table_fingerprint: String,
    authorities: BytecodeCompilationAuthorityPins,
    function_count: u64,
    word_count: u64,
    relocation_count: u64,
}

/// Read-only evidence derived from the independent statement manifest input.
///
/// Construction is private and never inspects statement rows embedded in a
/// bytecode image. Those rows are read only by the exact-join comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytecodeStatementManifestReceipt {
    package_id: String,
    identity: BytecodeStatementManifestIdentity,
    function_count: u64,
    event_count: u64,
}

/// Exact semantic authorities retained from an admitted bytecode image.
///
/// The four pins stay grouped so a receipt consumer cannot select the native
/// lifecycle registry while overlooking the classifier, host-effect, or
/// intrinsic authority introduced by bytecode schema v6. Construction is
/// private: production values can only be derived from the artifact admitted
/// by [`BytecodeCompilationHandoff::try_new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytecodeCompilationAuthorityPins {
    native_value_lifecycle_registry: NativeValueLifecycleRegistryIdentity,
    value_lifecycle_policy: ValueLifecyclePolicyIdentity,
    host_effect_registry: HostEffectRegistryIdentity,
    intrinsic_registry: IntrinsicRegistryIdentity,
}

/// Failure to admit the exact emission output at the compiled-package seam.
#[derive(Debug, Error)]
pub enum BytecodeCompilationHandoffError {
    #[error("emitted bytecode artifact did not pass canonical C1-C9 admission: {source}")]
    InvalidCanonicalArtifact {
        #[source]
        source: ArtifactIdentityError,
    },
    #[error(
        "emitted bytecode reference identity {reference_identity} does not match artifact identity {artifact_identity}"
    )]
    ReferenceIdentityMismatch {
        artifact_identity: String,
        reference_identity: String,
    },
    #[error(
        "emitted bytecode reference must be path-free before the canonical store write, got artifactPath {artifact_path}"
    )]
    PrematureArtifactPath { artifact_path: String },
    #[error("independent bytecode statement manifest is invalid: {source}")]
    InvalidStatementManifest {
        #[source]
        source: StatementManifestIdentityError,
    },
    #[error("statement manifest contains origin absent from admitted bytecode: {origin:?}")]
    ExtraStatementManifestOrigin { origin: BytecodeFunctionOrigin },
    #[error("statement manifest omits admitted bytecode origin: {origin:?}")]
    MissingStatementManifestOrigin { origin: BytecodeFunctionOrigin },
    #[error("statement manifest entries differ from admitted bytecode at origin {origin:?}")]
    StatementManifestEntriesMismatch { origin: BytecodeFunctionOrigin },
}

/// Explicit bytecode-lane result.
///
/// `Disabled` is valid only when the compile request explicitly disabled
/// bytecode emission. `Failed` represents an enabled request that did not
/// produce a complete handoff; it is intentionally distinct from `Disabled`.
/// [`Self::into_result`] preserves that distinction by mapping only
/// `Disabled` to `Ok(None)` and every `Failed` value to `Err`.
#[must_use = "an enabled bytecode failure must be propagated; it must not become a disabled lane"]
#[derive(Debug, Clone, PartialEq)]
pub enum BytecodeCompilationOutcome<E> {
    Disabled,
    Enabled(Box<BytecodeCompilationHandoff>),
    Failed(E),
}

impl BytecodeCompilationHandoff {
    /// Admits one exact artifact/reference pair produced by the emitter.
    ///
    /// Admission validates the complete artifact (C1-C9), checks that the
    /// supplied reference names the admitted content, and rejects a record
    /// path because no canonical store write can have occurred at this seam.
    /// It then validates the independently supplied, package-bound statement
    /// manifest and exact-joins its origins and rows to that same artifact.
    pub fn try_new(
        package_id: String,
        statement_manifest: Vec<BytecodeFunctionStatementManifest>,
        statement_manifest_identity: BytecodeStatementManifestIdentity,
        artifact: BytecodeArtifact,
        reference: BytecodeArtifactRef,
    ) -> Result<Self, BytecodeCompilationHandoffError> {
        validate_bytecode_identity(&artifact).map_err(|source| {
            BytecodeCompilationHandoffError::InvalidCanonicalArtifact { source }
        })?;
        if reference.bytecode_identity != artifact.bytecode_identity {
            return Err(BytecodeCompilationHandoffError::ReferenceIdentityMismatch {
                artifact_identity: artifact.bytecode_identity.clone(),
                reference_identity: reference.bytecode_identity,
            });
        }
        if let Some(artifact_path) = reference.artifact_path {
            return Err(BytecodeCompilationHandoffError::PrematureArtifactPath { artifact_path });
        }
        validate_bytecode_statement_manifest_identity(
            &package_id,
            &statement_manifest,
            &statement_manifest_identity,
        )
        .map_err(|source| BytecodeCompilationHandoffError::InvalidStatementManifest { source })?;
        validate_statement_manifest_join(&statement_manifest, &artifact)?;

        let reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());
        let receipt = BytecodeCompilationReceipt::from_artifact(&artifact);
        let statement_manifest = BytecodeStatementManifestHandoff::new(
            package_id,
            statement_manifest,
            statement_manifest_identity,
        );
        Ok(Self {
            artifact,
            reference,
            receipt,
            statement_manifest,
        })
    }

    pub fn artifact(&self) -> &BytecodeArtifact {
        &self.artifact
    }

    /// Exact path-free reference for PackageArtifact identity projection.
    pub fn reference(&self) -> &BytecodeArtifactRef {
        &self.reference
    }

    pub fn receipt(&self) -> &BytecodeCompilationReceipt {
        &self.receipt
    }

    /// Independently produced statement rows, retained with their admitted
    /// bytecode counterpart so they cannot be re-paired through this API.
    pub fn statement_manifest(&self) -> &[BytecodeFunctionStatementManifest] {
        &self.statement_manifest.functions
    }

    pub const fn statement_manifest_receipt(&self) -> &BytecodeStatementManifestReceipt {
        &self.statement_manifest.receipt
    }
}

impl BytecodeStatementManifestHandoff {
    fn new(
        package_id: String,
        functions: Vec<BytecodeFunctionStatementManifest>,
        identity: BytecodeStatementManifestIdentity,
    ) -> Self {
        let receipt =
            BytecodeStatementManifestReceipt::from_manifest(package_id, identity, &functions);
        Self { functions, receipt }
    }
}

impl BytecodeStatementManifestReceipt {
    fn from_manifest(
        package_id: String,
        identity: BytecodeStatementManifestIdentity,
        functions: &[BytecodeFunctionStatementManifest],
    ) -> Self {
        let event_count = functions
            .iter()
            .map(|function| function.statement_entries.len() as u64)
            .sum();
        Self {
            package_id,
            identity,
            function_count: functions.len() as u64,
            event_count,
        }
    }

    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub const fn identity(&self) -> &BytecodeStatementManifestIdentity {
        &self.identity
    }

    pub const fn function_count(&self) -> u64 {
        self.function_count
    }

    pub const fn event_count(&self) -> u64 {
        self.event_count
    }
}

fn validate_statement_manifest_join(
    manifest: &[BytecodeFunctionStatementManifest],
    artifact: &BytecodeArtifact,
) -> Result<(), BytecodeCompilationHandoffError> {
    let mut admitted_by_origin = artifact
        .image
        .functions
        .values()
        .map(|function| {
            (
                function.origin.clone(),
                function.statement_entries.as_slice(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    for function in manifest {
        let Some(admitted_entries) = admitted_by_origin.remove(&function.origin) else {
            return Err(
                BytecodeCompilationHandoffError::ExtraStatementManifestOrigin {
                    origin: function.origin.clone(),
                },
            );
        };
        if admitted_entries != function.statement_entries.as_slice() {
            return Err(
                BytecodeCompilationHandoffError::StatementManifestEntriesMismatch {
                    origin: function.origin.clone(),
                },
            );
        }
    }

    if let Some((origin, _)) = admitted_by_origin.into_iter().next() {
        return Err(BytecodeCompilationHandoffError::MissingStatementManifestOrigin { origin });
    }
    Ok(())
}

impl BytecodeCompilationReceipt {
    fn from_artifact(artifact: &BytecodeArtifact) -> Self {
        let (word_count, relocation_count) = artifact.image.functions.values().fold(
            (0_u64, 0_u64),
            |(words, relocations), function| {
                (
                    words + function.words.len() as u64,
                    relocations + function.relocations.len() as u64,
                )
            },
        );
        Self {
            bytecode_identity: artifact.bytecode_identity.clone(),
            schema_version: artifact.schema_version.clone(),
            isa_version: artifact.isa_version.clone(),
            opcode_table_fingerprint: artifact.opcode_table_fingerprint.clone(),
            authorities: BytecodeCompilationAuthorityPins::from_artifact(artifact),
            function_count: artifact.image.functions.len() as u64,
            word_count,
            relocation_count,
        }
    }

    pub fn bytecode_identity(&self) -> &str {
        &self.bytecode_identity
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub fn isa_version(&self) -> &str {
        &self.isa_version
    }

    pub fn opcode_table_fingerprint(&self) -> &str {
        &self.opcode_table_fingerprint
    }

    /// All exact semantic authorities admitted with this v6 image.
    pub const fn authorities(&self) -> &BytecodeCompilationAuthorityPins {
        &self.authorities
    }

    pub fn function_count(&self) -> u64 {
        self.function_count
    }

    pub fn word_count(&self) -> u64 {
        self.word_count
    }

    pub fn relocation_count(&self) -> u64 {
        self.relocation_count
    }
}

impl BytecodeCompilationAuthorityPins {
    fn from_artifact(artifact: &BytecodeArtifact) -> Self {
        Self {
            native_value_lifecycle_registry: artifact.native_value_lifecycle_registry.clone(),
            value_lifecycle_policy: artifact.value_lifecycle_policy.clone(),
            host_effect_registry: artifact.host_effect_registry.clone(),
            intrinsic_registry: artifact.intrinsic_registry.clone(),
        }
    }

    pub const fn native_value_lifecycle_registry(&self) -> &NativeValueLifecycleRegistryIdentity {
        &self.native_value_lifecycle_registry
    }

    pub const fn value_lifecycle_policy(&self) -> &ValueLifecyclePolicyIdentity {
        &self.value_lifecycle_policy
    }

    pub const fn host_effect_registry(&self) -> &HostEffectRegistryIdentity {
        &self.host_effect_registry
    }

    pub const fn intrinsic_registry(&self) -> &IntrinsicRegistryIdentity {
        &self.intrinsic_registry
    }
}

impl<E> BytecodeCompilationOutcome<E> {
    /// Records that the caller explicitly selected the disabled lane.
    pub fn disabled() -> Self {
        Self::Disabled
    }

    /// Records a complete, admitted result for an enabled request.
    pub fn enabled(handoff: BytecodeCompilationHandoff) -> Self {
        Self::Enabled(Box::new(handoff))
    }

    /// Records failure of an enabled request. This never degrades to
    /// [`Self::Disabled`].
    pub fn failed(error: E) -> Self {
        Self::Failed(error)
    }

    /// Adapts an enabled emitter/admission result without depending on the
    /// emitter crate's concrete error type.
    pub fn from_enabled_result(
        result: Result<BytecodeCompilationHandoff, E>,
    ) -> BytecodeCompilationOutcome<E> {
        match result {
            Ok(handoff) => Self::enabled(handoff),
            Err(error) => Self::failed(error),
        }
    }

    /// Converts the lane into the fail-closed orchestration shape.
    ///
    /// Only an explicitly disabled request produces `Ok(None)`. An enabled
    /// failure is always returned as `Err`.
    pub fn into_result(self) -> Result<Option<BytecodeCompilationHandoff>, E> {
        match self {
            Self::Disabled => Ok(None),
            Self::Enabled(handoff) => Ok(Some(*handoff)),
            Self::Failed(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests;
