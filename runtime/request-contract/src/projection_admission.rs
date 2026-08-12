use std::fmt;

use skiff_artifact_model::platform_error_projection::PlatformErrorProjectionKey;

use crate::platform_error_projection::PlatformErrorProjectionPayload;

/// Runtime-owned operation families that may ask the projection policy for
/// admission.
///
/// These variants are closed metadata, not names parsed from an error or wire
/// payload. JsonObject and task operations are present so the policy can deny
/// their current semantic classes explicitly; neither has an admission row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectionOperation {
    BytecodeArrayGet,
    BytecodeMapGet,
    BytecodeSetWritablePathArraySegment,
    BytecodeSetWritablePathMapSegment,
    BytecodeJsonObjectGet,
    BytecodeSetWritablePathJsonObjectSegment,
    LexicalTimeoutScope,
    HttpRequest,
    ActorMethodInvocation,
    ActorActivation,
    ServiceCall,
    TaskSubmit,
}

/// Source-owner semantic classification of one projection candidate.
///
/// Internal cancellation and request-root/inherited deadlines intentionally
/// have no variants here: they are execution terminals and cannot become
/// projection candidates.
///
/// ```compile_fail
/// use skiff_runtime_request_contract::ProjectionSemanticClass;
///
/// let _ = ProjectionSemanticClass::InternalCancellation;
/// ```
///
/// ```compile_fail
/// use skiff_runtime_request_contract::ProjectionSemanticClass;
///
/// let _ = ProjectionSemanticClass::RequestRootDeadlineExceeded;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectionSemanticClass {
    ArrayIndexOutOfBounds,
    MapKeyNotFound,
    JsonObjectPropertyNotFound,
    LexicalScopeDeadlineExceeded,
    HttpRequestDeadlineExceeded,
    ActorMethodInvocationDeadlineExceeded,
    ActorActivationDeadlineExceeded,
    ImportedInstructionLimitExceeded,
    ImportedFixedServiceFailure,
    TaskSubmitDefiniteRejection,
    TaskSubmitOutcomeUnknown,
}

/// Closed execution phases at which the first admission rows may be checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectionPhase {
    BeforeDispatch,
    ExecuteInstruction,
    TraverseWritablePath,
    ScopeDeadlineWinner,
    AwaitPrimitiveOutcome,
    ReceiveServiceOutcome,
}

/// Effect certainty promised by an admitted operation contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectionEffect {
    NoEffect,
    EffectAlreadyVisible,
    OutcomeUnknown,
}

/// A typed payload classified by its source semantic owner.
///
/// The projection key is deliberately absent: it is always derived from the
/// exact generated payload with [`PlatformErrorProjectionPayload::key`]. The
/// constructor consumes the payload so callers cannot retain an alias to the
/// value that enters admission.
///
/// External callers may use [`ProjectionCandidate::new`], but cannot forge a
/// candidate with a struct literal:
///
/// ```compile_fail
/// use skiff_runtime_request_contract::{
///     PlatformErrorProjectionPayload, ProjectionCandidate, ProjectionPhase,
///     ProjectionSemanticClass, StdCollectionArrayIndexOutOfBoundsErrorPayload,
/// };
///
/// let payload = PlatformErrorProjectionPayload::StdCollectionArrayIndexOutOfBoundsError(
///     StdCollectionArrayIndexOutOfBoundsErrorPayload { index: 4, length: 2 },
/// );
/// let _forged = ProjectionCandidate {
///     payload,
///     semantic_class: ProjectionSemanticClass::ArrayIndexOutOfBounds,
///     phase: ProjectionPhase::ExecuteInstruction,
/// };
/// ```
///
/// Its classification and payload also cannot be replaced after construction:
///
/// ```compile_fail
/// use skiff_runtime_request_contract::{
///     PlatformErrorProjectionPayload, ProjectionCandidate, ProjectionPhase,
///     ProjectionSemanticClass, StdCollectionArrayIndexOutOfBoundsErrorPayload,
///     StdCollectionMapKeyNotFoundErrorPayload,
/// };
///
/// let payload = PlatformErrorProjectionPayload::StdCollectionArrayIndexOutOfBoundsError(
///     StdCollectionArrayIndexOutOfBoundsErrorPayload { index: 4, length: 2 },
/// );
/// let mut candidate = ProjectionCandidate::new(
///     payload,
///     ProjectionSemanticClass::ArrayIndexOutOfBounds,
///     ProjectionPhase::ExecuteInstruction,
/// );
/// candidate.payload = PlatformErrorProjectionPayload::StdCollectionMapKeyNotFoundError(
///     StdCollectionMapKeyNotFoundErrorPayload {},
/// );
/// ```
pub struct ProjectionCandidate {
    payload: PlatformErrorProjectionPayload,
    semantic_class: ProjectionSemanticClass,
    phase: ProjectionPhase,
}

impl ProjectionCandidate {
    pub fn new(
        payload: PlatformErrorProjectionPayload,
        semantic_class: ProjectionSemanticClass,
        phase: ProjectionPhase,
    ) -> Self {
        Self {
            payload,
            semantic_class,
            phase,
        }
    }

    pub fn payload(&self) -> &PlatformErrorProjectionPayload {
        &self.payload
    }

    pub fn projection_key(&self) -> PlatformErrorProjectionKey {
        self.payload.key()
    }

    pub const fn semantic_class(&self) -> ProjectionSemanticClass {
        self.semantic_class
    }

    pub const fn phase(&self) -> ProjectionPhase {
        self.phase
    }
}

/// Proof that one exact operation/key/class/phase row was admitted.
///
/// This value is move-only, has private fields, and can only be created by
/// [`admit_projection`]. In particular, external code cannot construct it:
///
/// ```compile_fail
/// use skiff_runtime_request_contract::{
///     AdmittedProjection, PlatformErrorProjectionPayload, ProjectionEffect,
///     ProjectionOperation, ProjectionPhase, ProjectionSemanticClass,
///     StdCollectionArrayIndexOutOfBoundsErrorPayload,
/// };
///
/// let _forged = AdmittedProjection {
///     operation: ProjectionOperation::BytecodeArrayGet,
///     payload: PlatformErrorProjectionPayload::StdCollectionArrayIndexOutOfBoundsError(
///         StdCollectionArrayIndexOutOfBoundsErrorPayload { index: 4, length: 2 },
///     ),
///     semantic_class: ProjectionSemanticClass::ArrayIndexOutOfBounds,
///     phase: ProjectionPhase::ExecuteInstruction,
///     effect: ProjectionEffect::NoEffect,
/// };
/// ```
///
/// Admission proof cannot be cloned:
///
/// ```compile_fail
/// use skiff_runtime_request_contract::{
///     admit_projection, PlatformErrorProjectionPayload, ProjectionCandidate,
///     ProjectionOperation, ProjectionPhase, ProjectionSemanticClass,
///     StdCollectionArrayIndexOutOfBoundsErrorPayload,
/// };
///
/// let candidate = ProjectionCandidate::new(
///     PlatformErrorProjectionPayload::StdCollectionArrayIndexOutOfBoundsError(
///         StdCollectionArrayIndexOutOfBoundsErrorPayload { index: 4, length: 2 },
///     ),
///     ProjectionSemanticClass::ArrayIndexOutOfBounds,
///     ProjectionPhase::ExecuteInstruction,
/// );
/// let admitted = admit_projection(ProjectionOperation::BytecodeArrayGet, candidate)
///     .expect("the exact row is admitted");
/// let _duplicate = admitted.clone();
/// ```
///
/// The payload cannot be replaced after admission:
///
/// ```compile_fail
/// use skiff_runtime_request_contract::{
///     admit_projection, PlatformErrorProjectionPayload, ProjectionCandidate,
///     ProjectionOperation, ProjectionPhase, ProjectionSemanticClass,
///     StdCollectionArrayIndexOutOfBoundsErrorPayload,
///     StdCollectionMapKeyNotFoundErrorPayload,
/// };
///
/// let candidate = ProjectionCandidate::new(
///     PlatformErrorProjectionPayload::StdCollectionArrayIndexOutOfBoundsError(
///         StdCollectionArrayIndexOutOfBoundsErrorPayload { index: 4, length: 2 },
///     ),
///     ProjectionSemanticClass::ArrayIndexOutOfBounds,
///     ProjectionPhase::ExecuteInstruction,
/// );
/// let mut admitted = admit_projection(ProjectionOperation::BytecodeArrayGet, candidate)
///     .expect("the exact row is admitted");
/// admitted.payload = PlatformErrorProjectionPayload::StdCollectionMapKeyNotFoundError(
///     StdCollectionMapKeyNotFoundErrorPayload {},
/// );
/// ```
pub struct AdmittedProjection {
    operation: ProjectionOperation,
    payload: PlatformErrorProjectionPayload,
    semantic_class: ProjectionSemanticClass,
    phase: ProjectionPhase,
    effect: ProjectionEffect,
}

impl AdmittedProjection {
    pub const fn operation(&self) -> ProjectionOperation {
        self.operation
    }

    pub fn payload(&self) -> &PlatformErrorProjectionPayload {
        &self.payload
    }

    pub fn projection_key(&self) -> PlatformErrorProjectionKey {
        self.payload.key()
    }

    pub const fn semantic_class(&self) -> ProjectionSemanticClass {
        self.semantic_class
    }

    pub const fn phase(&self) -> ProjectionPhase {
        self.phase
    }

    pub const fn effect(&self) -> ProjectionEffect {
        self.effect
    }

    pub fn into_payload(self) -> PlatformErrorProjectionPayload {
        self.payload
    }
}

/// Sanitized default-deny result from [`admit_projection`].
///
/// It intentionally retains neither the generated payload nor caller-provided
/// diagnostic text.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProjectionDenied {
    _private: (),
}

impl ProjectionDenied {
    const fn new() -> Self {
        Self { _private: () }
    }
}

impl fmt::Debug for ProjectionDenied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProjectionDenied")
    }
}

impl fmt::Display for ProjectionDenied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("platform error projection is not admitted")
    }
}

impl std::error::Error for ProjectionDenied {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProjectionAdmissionRow {
    operation: ProjectionOperation,
    projection_key: PlatformErrorProjectionKey,
    semantic_class: ProjectionSemanticClass,
    phase: ProjectionPhase,
    effect: ProjectionEffect,
}

// Rows are grouped in strictly ascending ASCII projection-key order. Equal
// keys are ordered by the closed operation enum. JsonObject and task variants
// deliberately have no rows.
const PROJECTION_ADMISSION_ROWS: [ProjectionAdmissionRow; 9] = [
    ProjectionAdmissionRow {
        operation: ProjectionOperation::ActorActivation,
        projection_key: PlatformErrorProjectionKey::StdActorActivationTimeoutError,
        semantic_class: ProjectionSemanticClass::ActorActivationDeadlineExceeded,
        phase: ProjectionPhase::AwaitPrimitiveOutcome,
        effect: ProjectionEffect::OutcomeUnknown,
    },
    ProjectionAdmissionRow {
        operation: ProjectionOperation::ActorMethodInvocation,
        projection_key: PlatformErrorProjectionKey::StdActorMethodInvocationTimeoutError,
        semantic_class: ProjectionSemanticClass::ActorMethodInvocationDeadlineExceeded,
        phase: ProjectionPhase::AwaitPrimitiveOutcome,
        effect: ProjectionEffect::OutcomeUnknown,
    },
    ProjectionAdmissionRow {
        operation: ProjectionOperation::BytecodeArrayGet,
        projection_key: PlatformErrorProjectionKey::StdCollectionArrayIndexOutOfBoundsError,
        semantic_class: ProjectionSemanticClass::ArrayIndexOutOfBounds,
        phase: ProjectionPhase::ExecuteInstruction,
        effect: ProjectionEffect::NoEffect,
    },
    ProjectionAdmissionRow {
        operation: ProjectionOperation::BytecodeSetWritablePathArraySegment,
        projection_key: PlatformErrorProjectionKey::StdCollectionArrayIndexOutOfBoundsError,
        semantic_class: ProjectionSemanticClass::ArrayIndexOutOfBounds,
        phase: ProjectionPhase::TraverseWritablePath,
        effect: ProjectionEffect::NoEffect,
    },
    ProjectionAdmissionRow {
        operation: ProjectionOperation::BytecodeMapGet,
        projection_key: PlatformErrorProjectionKey::StdCollectionMapKeyNotFoundError,
        semantic_class: ProjectionSemanticClass::MapKeyNotFound,
        phase: ProjectionPhase::ExecuteInstruction,
        effect: ProjectionEffect::NoEffect,
    },
    ProjectionAdmissionRow {
        operation: ProjectionOperation::BytecodeSetWritablePathMapSegment,
        projection_key: PlatformErrorProjectionKey::StdCollectionMapKeyNotFoundError,
        semantic_class: ProjectionSemanticClass::MapKeyNotFound,
        phase: ProjectionPhase::TraverseWritablePath,
        effect: ProjectionEffect::NoEffect,
    },
    ProjectionAdmissionRow {
        operation: ProjectionOperation::ServiceCall,
        projection_key: PlatformErrorProjectionKey::StdErrorInstructionLimitExceededError,
        semantic_class: ProjectionSemanticClass::ImportedInstructionLimitExceeded,
        phase: ProjectionPhase::ReceiveServiceOutcome,
        effect: ProjectionEffect::OutcomeUnknown,
    },
    ProjectionAdmissionRow {
        operation: ProjectionOperation::LexicalTimeoutScope,
        projection_key: PlatformErrorProjectionKey::StdErrorTimeoutError,
        semantic_class: ProjectionSemanticClass::LexicalScopeDeadlineExceeded,
        phase: ProjectionPhase::ScopeDeadlineWinner,
        effect: ProjectionEffect::OutcomeUnknown,
    },
    ProjectionAdmissionRow {
        operation: ProjectionOperation::HttpRequest,
        projection_key: PlatformErrorProjectionKey::StdHttpRequestTimeoutError,
        semantic_class: ProjectionSemanticClass::HttpRequestDeadlineExceeded,
        phase: ProjectionPhase::AwaitPrimitiveOutcome,
        effect: ProjectionEffect::OutcomeUnknown,
    },
];

/// Consumes one classified payload and admits only an exact closed policy row.
///
/// There is no open key, string, boolean, or wire-payload input. Every key used
/// for the match comes from the exact generated payload, and every unlisted
/// tuple is denied.
pub fn admit_projection(
    operation: ProjectionOperation,
    candidate: ProjectionCandidate,
) -> Result<AdmittedProjection, ProjectionDenied> {
    let projection_key = candidate.payload.key();
    let effect = PROJECTION_ADMISSION_ROWS
        .iter()
        .find(|row| {
            row.operation == operation
                && row.projection_key == projection_key
                && row.semantic_class == candidate.semantic_class
                && row.phase == candidate.phase
        })
        .map(|row| row.effect)
        .ok_or_else(ProjectionDenied::new)?;

    Ok(AdmittedProjection {
        operation,
        payload: candidate.payload,
        semantic_class: candidate.semantic_class,
        phase: candidate.phase,
        effect,
    })
}

#[cfg(test)]
mod tests;
