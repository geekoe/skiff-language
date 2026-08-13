use std::{num::NonZeroU64, sync::Arc};

use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, ResumeSiteIndex};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_request_contract::{
    admit_projection, AdmittedProjection, ProjectionCandidate, ProjectionOperation,
};
use skiff_runtime_vm::{VmResumeKind, VmResumeToken};

/// A nonzero generation assigned monotonically by the request owner.
///
/// Construction remains inside `skiff-runtime-request`; an error producer or
/// downstream host cannot invent a generation and use it to mint a guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestGeneration(NonZeroU64);

impl RequestGeneration {
    #[allow(dead_code)]
    pub(crate) const fn from_nonzero(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// A nonzero scheduler lane owned by one active request continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContinuationLaneId(NonZeroU64);

impl ContinuationLaneId {
    #[allow(dead_code)]
    pub(crate) const fn from_nonzero(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Closed VM call-site identity bound into a continuation guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActiveCallSiteKey {
    Inline {
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
    Resume {
        function: FunctionIndex,
        instruction: InstructionIndex,
        resume_site: ResumeSiteIndex,
    },
}

/// Closed owner families represented by the current VM resume authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResumeOwnerKind {
    Child,
    Adapter,
    StreamChild,
    StreamItem,
}

/// Exact resume owner and its monotonic VM-issued sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResumeOwnerKey {
    kind: ResumeOwnerKind,
    sequence: u64,
}

impl ResumeOwnerKey {
    #[allow(dead_code)]
    pub(crate) const fn new(kind: ResumeOwnerKind, sequence: u64) -> Self {
        Self { kind, sequence }
    }

    pub const fn kind(self) -> ResumeOwnerKind {
        self.kind
    }

    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExecutionImageKey(usize);

impl ExecutionImageKey {
    fn for_vm(image: &Arc<DeploymentExecutionImage>) -> Self {
        Self(Arc::as_ptr(image) as usize)
    }

    #[cfg(test)]
    const fn for_test(value: usize) -> Self {
        Self(value)
    }
}

mod sealed {
    use super::{ActiveCallSiteKey, ExecutionImageKey, ResumeOwnerKey};

    pub trait Sealed {}

    pub(crate) trait ContinuationAuthority: Sealed {
        fn execution_image_key(&self) -> ExecutionImageKey;
        fn active_call_site_key(&self) -> ActiveCallSiteKey;
        fn resume_owner_key(&self) -> ResumeOwnerKey;
    }
}

impl sealed::Sealed for VmResumeToken {}

impl sealed::ContinuationAuthority for VmResumeToken {
    fn execution_image_key(&self) -> ExecutionImageKey {
        ExecutionImageKey::for_vm(self.image())
    }

    fn active_call_site_key(&self) -> ActiveCallSiteKey {
        ActiveCallSiteKey::Resume {
            function: self.function(),
            instruction: self.instruction(),
            resume_site: self.resume_site(),
        }
    }

    fn resume_owner_key(&self) -> ResumeOwnerKey {
        let kind = match self.kind() {
            VmResumeKind::Child => ResumeOwnerKind::Child,
            VmResumeKind::Adapter => ResumeOwnerKind::Adapter,
            VmResumeKind::StreamChild => ResumeOwnerKind::StreamChild,
            VmResumeKind::StreamItem => ResumeOwnerKind::StreamItem,
        };
        ResumeOwnerKey::new(kind, self.sequence())
    }
}

/// Runtime-minted, one-shot authority for projection at one continuation.
///
/// The fields and minting seam are private, and the authority itself must
/// implement a request-local sealed trait. This type intentionally implements
/// neither `Clone` nor `Copy` and carries no settlement boolean: moving the
/// authority is the once-only proof.
///
/// ```compile_fail
/// use skiff_runtime_request::ContinuationProjectionGuard;
///
/// let _forged = ContinuationProjectionGuard::<()> {};
/// ```
///
/// ```compile_fail
/// use skiff_runtime_request::ContinuationProjectionGuard;
///
/// fn duplicate<A>(guard: ContinuationProjectionGuard<A>) {
///     let _second = guard.clone();
/// }
/// ```
#[must_use = "a continuation projection guard is unique, move-only authority"]
pub struct ContinuationProjectionGuard<A> {
    request_generation: RequestGeneration,
    lane: ContinuationLaneId,
    call_site: ActiveCallSiteKey,
    resume_owner: ResumeOwnerKey,
    execution_image: ExecutionImageKey,
    _authority: A,
}

impl<A> ContinuationProjectionGuard<A> {
    pub const fn request_generation(&self) -> RequestGeneration {
        self.request_generation
    }

    pub const fn lane(&self) -> ContinuationLaneId {
        self.lane
    }

    pub const fn call_site(&self) -> ActiveCallSiteKey {
        self.call_site
    }

    pub const fn resume_owner(&self) -> ResumeOwnerKey {
        self.resume_owner
    }

    fn validate(&self, current: &CurrentContinuationFacts) -> Result<(), FailureProjectionError> {
        if self.request_generation != current.request_generation {
            return Err(FailureProjectionError::RequestGenerationMismatch);
        }
        if self.lane != current.lane {
            return Err(FailureProjectionError::ContinuationLaneMismatch);
        }
        if self.call_site != current.call_site {
            return Err(FailureProjectionError::ActiveCallSiteMismatch);
        }
        if self.resume_owner != current.resume_owner {
            return Err(FailureProjectionError::ResumeOwnerMismatch);
        }
        if self.execution_image != current.execution_image {
            return Err(FailureProjectionError::ExecutionImageMismatch);
        }
        Ok(())
    }
}

/// An active request call site coupled to its unique continuation guard.
///
/// Neither a source site nor a projection DTO can construct this value. The
/// only constructor is the crate-private runtime minting seam below.
///
/// ```compile_fail
/// use skiff_runtime_request::ActiveRequestCallSite;
///
/// let _forged = ActiveRequestCallSite::<()> {};
/// ```
///
/// ```compile_fail
/// use skiff_runtime_request::ActiveRequestCallSite;
///
/// fn duplicate<A>(site: ActiveRequestCallSite<A>) {
///     let _second = site.clone();
/// }
/// ```
pub struct ActiveRequestCallSite<A> {
    operation: ProjectionOperation,
    source: InstructionSourceSite,
    guard: ContinuationProjectionGuard<A>,
}

impl<A> ActiveRequestCallSite<A> {
    pub const fn operation(&self) -> ProjectionOperation {
        self.operation
    }

    pub const fn source(&self) -> &InstructionSourceSite {
        &self.source
    }

    pub const fn guard(&self) -> &ContinuationProjectionGuard<A> {
        &self.guard
    }
}

/// Closed locations at which a runtime failure may be observed.
///
/// Only `ActiveRequestCall` carries a guard. In particular, control, ingress,
/// provider egress, durable/background work, cancellation, and internal stop
/// cannot borrow a stale request heap to project an exception.
pub enum FailureSite<A> {
    ControlLoad,
    IngressBeforeHandler,
    ActiveRequestCall(ActiveRequestCallSite<A>),
    ProviderEgressAfterHandler,
    DurableBackgroundPlatformWork,
    InternalStop,
}

/// Closed request-owned gate for operations allowed to receive a projection
/// guard at all.
///
/// `task.submit` is excluded before guard minting because neither definite
/// rejection nor ambiguous acceptance may re-enter the submitting Skiff
/// continuation as a catchable failure.
pub(crate) struct GuardedProjectionOperation(ProjectionOperation);

impl GuardedProjectionOperation {
    #[allow(dead_code)]
    pub(crate) const fn try_new(operation: ProjectionOperation) -> Option<Self> {
        match operation {
            ProjectionOperation::TaskSubmit => None,
            ProjectionOperation::BytecodeArrayGet
            | ProjectionOperation::BytecodeMapGet
            | ProjectionOperation::BytecodeSetWritablePathArraySegment
            | ProjectionOperation::BytecodeSetWritablePathMapSegment
            | ProjectionOperation::BytecodeJsonObjectGet
            | ProjectionOperation::BytecodeSetWritablePathJsonObjectSegment
            | ProjectionOperation::LexicalTimeoutScope
            | ProjectionOperation::HttpRequest
            | ProjectionOperation::ActorMethodInvocation
            | ProjectionOperation::ActorActivation
            | ProjectionOperation::ServiceCall => Some(Self(operation)),
        }
    }

    const fn get(&self) -> ProjectionOperation {
        self.0
    }
}

/// Sanitized failure classes from the dormant routing stage.
///
/// No variant retains a candidate, source site, generation, lane, image,
/// sequence, or authority value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FailureProjectionError {
    #[error("failure site is not an active request call")]
    SiteNotActive,
    #[error("continuation projection guard does not match the active request generation")]
    RequestGenerationMismatch,
    #[error("continuation projection guard does not match the active lane")]
    ContinuationLaneMismatch,
    #[error("continuation projection guard does not match the active call site")]
    ActiveCallSiteMismatch,
    #[error("continuation projection guard does not match the active resume owner")]
    ResumeOwnerMismatch,
    #[error("continuation projection guard does not match the active execution image")]
    ExecutionImageMismatch,
    #[error("platform error projection is not admitted")]
    ProjectionNotAdmitted,
}

/// Move-only proof that both the continuation and closed admission row match.
///
/// It does not materialize a generated payload, create a request exception, or
/// resume the VM. A later production stage must consume this proof to do any
/// of those things.
///
/// ```compile_fail
/// use skiff_runtime_request::AdmittedCallSiteProjection;
///
/// let _forged = AdmittedCallSiteProjection::<()> {};
/// ```
///
/// ```compile_fail
/// use skiff_runtime_request::AdmittedCallSiteProjection;
///
/// fn duplicate<A>(proof: AdmittedCallSiteProjection<A>) {
///     let _second = proof.clone();
/// }
/// ```
#[must_use = "an admitted call-site projection must be consumed by the next projection stage"]
pub struct AdmittedCallSiteProjection<A> {
    source: InstructionSourceSite,
    guard: ContinuationProjectionGuard<A>,
    admitted: AdmittedProjection,
}

impl<A> AdmittedCallSiteProjection<A> {
    pub const fn source(&self) -> &InstructionSourceSite {
        &self.source
    }

    pub const fn guard(&self) -> &ContinuationProjectionGuard<A> {
        &self.guard
    }

    pub const fn admitted(&self) -> &AdmittedProjection {
        &self.admitted
    }
}

/// Trusted scheduler facts sampled at the routing point.
///
/// This type and both constructors are request-crate-only. It is deliberately
/// not a DTO and never crosses the host/capability boundary.
pub(crate) struct CurrentContinuationFacts {
    request_generation: RequestGeneration,
    lane: ContinuationLaneId,
    call_site: ActiveCallSiteKey,
    resume_owner: ResumeOwnerKey,
    execution_image: ExecutionImageKey,
}

impl CurrentContinuationFacts {
    #[allow(dead_code)]
    pub(crate) fn for_vm(
        request_generation: RequestGeneration,
        lane: ContinuationLaneId,
        call_site: ActiveCallSiteKey,
        resume_owner: ResumeOwnerKey,
        image: &Arc<DeploymentExecutionImage>,
    ) -> Self {
        Self {
            request_generation,
            lane,
            call_site,
            resume_owner,
            execution_image: ExecutionImageKey::for_vm(image),
        }
    }

    #[cfg(test)]
    const fn for_test(
        request_generation: RequestGeneration,
        lane: ContinuationLaneId,
        call_site: ActiveCallSiteKey,
        resume_owner: ResumeOwnerKey,
        execution_image: usize,
    ) -> Self {
        Self {
            request_generation,
            lane,
            call_site,
            resume_owner,
            execution_image: ExecutionImageKey::for_test(execution_image),
        }
    }
}

/// The sole request-crate mint seam for a guarded active call site.
///
/// All guard facts except request generation and lane are derived from the
/// sealed move-only authority. No error producer, source site, generated DTO,
/// host, or capability context can call this function across the crate
/// boundary.
#[allow(dead_code)]
pub(crate) fn mint_active_request_call_site<A>(
    operation: GuardedProjectionOperation,
    source: InstructionSourceSite,
    request_generation: RequestGeneration,
    lane: ContinuationLaneId,
    authority: A,
) -> FailureSite<A>
where
    A: sealed::ContinuationAuthority,
{
    let execution_image = authority.execution_image_key();
    let call_site = authority.active_call_site_key();
    let resume_owner = authority.resume_owner_key();
    FailureSite::ActiveRequestCall(ActiveRequestCallSite {
        operation: operation.get(),
        source,
        guard: ContinuationProjectionGuard {
            request_generation,
            lane,
            call_site,
            resume_owner,
            execution_image,
            _authority: authority,
        },
    })
}

/// Dormant stage that routes, validates, and admits without materializing.
///
/// Validation order is intentionally fixed: site, then every guard fact, then
/// the closed request-contract admission table. Any failure consumes and drops
/// the one-shot authority and returns only a sanitized typed error.
#[allow(dead_code)]
pub(crate) fn promote_call_site_error<A>(
    site: FailureSite<A>,
    current: CurrentContinuationFacts,
    candidate: ProjectionCandidate,
) -> Result<AdmittedCallSiteProjection<A>, FailureProjectionError> {
    let FailureSite::ActiveRequestCall(active) = site else {
        return Err(FailureProjectionError::SiteNotActive);
    };

    active.guard.validate(&current)?;
    let admitted = admit_projection(active.operation, candidate)
        .map_err(|_| FailureProjectionError::ProjectionNotAdmitted)?;

    Ok(AdmittedCallSiteProjection {
        source: active.source,
        guard: active.guard,
        admitted,
    })
}

#[cfg(test)]
mod tests;
