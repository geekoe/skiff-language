use std::{
    num::NonZeroU64,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, ResumeSiteIndex};
use skiff_runtime_request_contract::{
    PlatformErrorProjectionPayload, ProjectionCandidate, ProjectionEffect, ProjectionOperation,
    ProjectionPhase, ProjectionSemanticClass, StdCollectionArrayIndexOutOfBoundsErrorPayload,
    StdCollectionMapKeyNotFoundErrorPayload, StdErrorTimeoutErrorPayload,
};
use skiff_runtime_vm::VmResumeToken;

use super::{
    mint_active_request_call_site, promote_call_site_error, sealed, ActiveCallSiteKey,
    CurrentContinuationFacts, FailureProjectionError, FailureSite, GuardedProjectionOperation,
    RequestGeneration, ResumeOwnerKey, ResumeOwnerKind,
};
use crate::ContinuationLaneId;

const IMAGE_KEY: usize = 41;
const AUTHORITY_ID: u64 = 73;

struct TestAuthority {
    id: u64,
    execution_image: usize,
    call_site: ActiveCallSiteKey,
    resume_owner: ResumeOwnerKey,
    drop_count: Option<Arc<AtomicUsize>>,
}

impl TestAuthority {
    fn exact() -> Self {
        Self {
            id: AUTHORITY_ID,
            execution_image: IMAGE_KEY,
            call_site: exact_call_site(),
            resume_owner: exact_resume_owner(),
            drop_count: None,
        }
    }

    fn tracked(drop_count: Arc<AtomicUsize>) -> Self {
        Self {
            id: AUTHORITY_ID,
            execution_image: IMAGE_KEY,
            call_site: exact_call_site(),
            resume_owner: exact_resume_owner(),
            drop_count: Some(drop_count),
        }
    }
}

impl Drop for TestAuthority {
    fn drop(&mut self) {
        if let Some(drop_count) = &self.drop_count {
            drop_count.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl sealed::Sealed for TestAuthority {}

impl sealed::ContinuationAuthority for TestAuthority {
    fn execution_image_key(&self) -> super::ExecutionImageKey {
        super::ExecutionImageKey::for_test(self.execution_image)
    }

    fn active_call_site_key(&self) -> ActiveCallSiteKey {
        self.call_site
    }

    fn resume_owner_key(&self) -> ResumeOwnerKey {
        self.resume_owner
    }
}

fn nonzero(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).expect("test identity must be nonzero")
}

fn generation(value: u64) -> RequestGeneration {
    RequestGeneration::from_nonzero(nonzero(value))
}

fn lane(value: u64) -> ContinuationLaneId {
    ContinuationLaneId::from_nonzero(nonzero(value))
}

fn exact_call_site() -> ActiveCallSiteKey {
    ActiveCallSiteKey::Resume {
        function: FunctionIndex::new(3),
        instruction: InstructionIndex::new(11),
        resume_site: ResumeSiteIndex::new(5),
    }
}

fn exact_resume_owner() -> ResumeOwnerKey {
    ResumeOwnerKey::new(ResumeOwnerKind::Child, 17)
}

fn source(reason: SyntheticInstructionSiteReason) -> InstructionSourceSite {
    InstructionSourceSite::Synthetic { reason }
}

fn exact_current() -> CurrentContinuationFacts {
    CurrentContinuationFacts::for_test(
        generation(1),
        lane(1),
        exact_call_site(),
        exact_resume_owner(),
        IMAGE_KEY,
    )
}

fn array_candidate() -> ProjectionCandidate {
    ProjectionCandidate::new(
        PlatformErrorProjectionPayload::StdCollectionArrayIndexOutOfBoundsError(
            StdCollectionArrayIndexOutOfBoundsErrorPayload {
                index: 8,
                length: 3,
            },
        ),
        ProjectionSemanticClass::ArrayIndexOutOfBounds,
        ProjectionPhase::ExecuteInstruction,
    )
}

fn map_candidate() -> ProjectionCandidate {
    ProjectionCandidate::new(
        PlatformErrorProjectionPayload::StdCollectionMapKeyNotFoundError(
            StdCollectionMapKeyNotFoundErrorPayload {},
        ),
        ProjectionSemanticClass::MapKeyNotFound,
        ProjectionPhase::ExecuteInstruction,
    )
}

fn task_submit_candidate() -> ProjectionCandidate {
    ProjectionCandidate::new(
        PlatformErrorProjectionPayload::StdErrorTimeoutError(StdErrorTimeoutErrorPayload {
            timeout_ms: 1_000,
        }),
        ProjectionSemanticClass::TaskSubmitDefiniteRejection,
        ProjectionPhase::BeforeDispatch,
    )
}

fn guarded_site(
    operation: ProjectionOperation,
    call_source: InstructionSourceSite,
) -> FailureSite<TestAuthority> {
    mint_active_request_call_site(
        GuardedProjectionOperation::try_new(operation)
            .expect("test operation must be eligible to receive a guard"),
        call_source,
        generation(1),
        lane(1),
        TestAuthority::exact(),
    )
}

fn assert_error<A>(
    result: Result<super::AdmittedCallSiteProjection<A>, FailureProjectionError>,
    expected: FailureProjectionError,
) {
    match result {
        Ok(_) => panic!("failure projection unexpectedly passed"),
        Err(actual) => assert_eq!(actual, expected),
    }
}

#[test]
fn exact_active_call_site_admits_and_preserves_only_the_guarded_source() {
    let guarded_source = source(SyntheticInstructionSiteReason::CompilerGeneratedWrapper);
    let site = guarded_site(
        ProjectionOperation::BytecodeArrayGet,
        guarded_source.clone(),
    );

    let proof = match promote_call_site_error(site, exact_current(), array_candidate()) {
        Ok(proof) => proof,
        Err(error) => panic!("exact active projection was rejected: {error}"),
    };

    assert_eq!(proof.source(), &guarded_source);
    assert_eq!(
        proof.admitted().operation(),
        ProjectionOperation::BytecodeArrayGet
    );
    assert_eq!(proof.admitted().effect(), ProjectionEffect::NoEffect);
    assert_eq!(proof.guard().request_generation(), generation(1));
    assert_eq!(proof.guard().lane(), lane(1));
    assert_eq!(proof.guard().call_site(), exact_call_site());
    assert_eq!(proof.guard().resume_owner(), exact_resume_owner());
}

#[test]
fn identical_candidate_is_rejected_at_each_of_the_other_five_sites() {
    let sites: [FailureSite<TestAuthority>; 5] = [
        FailureSite::ControlLoad,
        FailureSite::IngressBeforeHandler,
        FailureSite::ProviderEgressAfterHandler,
        FailureSite::DurableBackgroundPlatformWork,
        FailureSite::InternalStop,
    ];

    for site in sites {
        assert_error(
            promote_call_site_error(site, exact_current(), array_candidate()),
            FailureProjectionError::SiteNotActive,
        );
    }
}

fn assert_guard_mismatch_precedes_admission(
    current: CurrentContinuationFacts,
    expected: FailureProjectionError,
) {
    let site = guarded_site(
        ProjectionOperation::BytecodeArrayGet,
        source(SyntheticInstructionSiteReason::RuntimeBoundaryDispatch),
    );
    assert_error(
        promote_call_site_error(site, current, map_candidate()),
        expected,
    );
}

#[test]
fn every_exact_guard_fact_is_checked_before_closed_admission() {
    assert_guard_mismatch_precedes_admission(
        CurrentContinuationFacts::for_test(
            generation(2),
            lane(1),
            exact_call_site(),
            exact_resume_owner(),
            IMAGE_KEY,
        ),
        FailureProjectionError::RequestGenerationMismatch,
    );
    assert_guard_mismatch_precedes_admission(
        CurrentContinuationFacts::for_test(
            generation(1),
            lane(2),
            exact_call_site(),
            exact_resume_owner(),
            IMAGE_KEY,
        ),
        FailureProjectionError::ContinuationLaneMismatch,
    );
    assert_guard_mismatch_precedes_admission(
        CurrentContinuationFacts::for_test(
            generation(1),
            lane(1),
            ActiveCallSiteKey::Inline {
                function: FunctionIndex::new(3),
                instruction: InstructionIndex::new(11),
            },
            exact_resume_owner(),
            IMAGE_KEY,
        ),
        FailureProjectionError::ActiveCallSiteMismatch,
    );
    assert_guard_mismatch_precedes_admission(
        CurrentContinuationFacts::for_test(
            generation(1),
            lane(1),
            exact_call_site(),
            ResumeOwnerKey::new(ResumeOwnerKind::Adapter, 17),
            IMAGE_KEY,
        ),
        FailureProjectionError::ResumeOwnerMismatch,
    );
    assert_guard_mismatch_precedes_admission(
        CurrentContinuationFacts::for_test(
            generation(1),
            lane(1),
            exact_call_site(),
            ResumeOwnerKey::new(ResumeOwnerKind::Child, 18),
            IMAGE_KEY,
        ),
        FailureProjectionError::ResumeOwnerMismatch,
    );
    assert_guard_mismatch_precedes_admission(
        CurrentContinuationFacts::for_test(
            generation(1),
            lane(1),
            exact_call_site(),
            exact_resume_owner(),
            IMAGE_KEY + 1,
        ),
        FailureProjectionError::ExecutionImageMismatch,
    );
}

#[test]
fn exact_guard_still_rejects_a_wrong_operation_tuple() {
    let site = guarded_site(
        ProjectionOperation::BytecodeMapGet,
        source(SyntheticInstructionSiteReason::RuntimeBoundaryDispatch),
    );
    assert_error(
        promote_call_site_error(site, exact_current(), array_candidate()),
        FailureProjectionError::ProjectionNotAdmitted,
    );
}

#[test]
fn task_submit_and_internal_stop_have_no_projection_path() {
    assert!(
        GuardedProjectionOperation::try_new(ProjectionOperation::TaskSubmit).is_none(),
        "task.submit must be rejected before a guard can be minted"
    );

    // Durable task settlement and cancellation/internal-stop paths carry no
    // authority at all. `()` makes that structural fact explicit here.
    let background: FailureSite<()> = FailureSite::DurableBackgroundPlatformWork;
    assert_error(
        promote_call_site_error(background, exact_current(), task_submit_candidate()),
        FailureProjectionError::SiteNotActive,
    );
    let cancellation: FailureSite<()> = FailureSite::InternalStop;
    assert_error(
        promote_call_site_error(cancellation, exact_current(), task_submit_candidate()),
        FailureProjectionError::SiteNotActive,
    );
}

#[test]
fn one_move_only_authority_forms_exactly_one_admitted_result() {
    let drop_count = Arc::new(AtomicUsize::new(0));
    let site = mint_active_request_call_site(
        GuardedProjectionOperation::try_new(ProjectionOperation::BytecodeArrayGet)
            .expect("array get is guard eligible"),
        source(SyntheticInstructionSiteReason::CompilerDesugaring),
        generation(1),
        lane(1),
        TestAuthority::tracked(Arc::clone(&drop_count)),
    );

    let proof = match promote_call_site_error(site, exact_current(), array_candidate()) {
        Ok(proof) => proof,
        Err(error) => panic!("move-only authority was rejected: {error}"),
    };
    assert_eq!(proof.guard._authority.id, AUTHORITY_ID);
    assert_eq!(drop_count.load(Ordering::SeqCst), 0);

    drop(proof);
    assert_eq!(drop_count.load(Ordering::SeqCst), 1);
}

#[test]
fn vm_resume_token_implements_the_production_authority_seam() {
    fn assert_authority<A: sealed::ContinuationAuthority>() {}

    assert_authority::<VmResumeToken>();
}
