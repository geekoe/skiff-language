//! Phase 2 bytecode compilation handoff.
//!
//! This is an in-memory compiler boundary, not an emitter and not an artifact
//! writer. It accepts only an emission-produced [`BytecodeArtifact`] plus its
//! exact, path-free [`BytecodeArtifactRef`]. Construction runs C1-C9 admission
//! and derives the evidence receipt from the admitted artifact; it never
//! opens File IR or reconstructs source, type, liveness, effect, or relocation
//! facts.
//!
//! The path-free reference is deliberate. The upper publication owner can
//! first use it in the package identity projection, derive the exact package
//! record family, write the bytecode record, attach the canonical returned
//! path, and only then write the PackageArtifact record. No store I/O or
//! PackageArtifact mutation occurs here, so a failed enabled emission cannot
//! be hidden by a partial write or an implicit disabled/legacy fallback.

use skiff_artifact_identity::{validate_bytecode_identity, ArtifactIdentityError};
use skiff_artifact_model::{BytecodeArtifact, BytecodeArtifactRef};
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
    function_count: u64,
    word_count: u64,
    relocation_count: u64,
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
    pub fn try_new(
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

        let reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());
        let receipt = BytecodeCompilationReceipt::from_artifact(&artifact);
        Ok(Self {
            artifact,
            reference,
            receipt,
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

    /// Returns the three exact values needed by upper-level publication
    /// planning without performing or claiming any store write.
    pub fn into_parts(
        self,
    ) -> (
        BytecodeArtifact,
        BytecodeArtifactRef,
        BytecodeCompilationReceipt,
    ) {
        (self.artifact, self.reference, self.receipt)
    }
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
