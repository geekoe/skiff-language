use std::collections::HashSet;

use super::{
    admit_projection, ProjectionAdmissionRow, ProjectionCandidate, ProjectionEffect,
    ProjectionOperation, ProjectionPhase, ProjectionSemanticClass, PROJECTION_ADMISSION_ROWS,
};
use crate::platform_error_projection::{
    PlatformErrorProjectionPayload, StdActorActivationTimeoutErrorPayload,
    StdActorMethodInvocationTimeoutErrorPayload, StdCollectionArrayIndexOutOfBoundsErrorPayload,
    StdCollectionJsonObjectPropertyNotFoundErrorPayload, StdCollectionMapKeyNotFoundErrorPayload,
    StdErrorInstructionLimitExceededErrorPayload, StdErrorTimeoutErrorPayload,
    StdHttpRequestTimeoutErrorPayload,
};

type PayloadFactory = fn() -> PlatformErrorProjectionPayload;

#[derive(Clone, Copy)]
struct AdmissionCase {
    operation: ProjectionOperation,
    payload: PayloadFactory,
    semantic_class: ProjectionSemanticClass,
    phase: ProjectionPhase,
    effect: ProjectionEffect,
}

const ADMISSION_CASES: [AdmissionCase; 9] = [
    AdmissionCase {
        operation: ProjectionOperation::ActorActivation,
        payload: actor_activation_timeout_payload,
        semantic_class: ProjectionSemanticClass::ActorActivationDeadlineExceeded,
        phase: ProjectionPhase::AwaitPrimitiveOutcome,
        effect: ProjectionEffect::OutcomeUnknown,
    },
    AdmissionCase {
        operation: ProjectionOperation::ActorMethodInvocation,
        payload: actor_method_timeout_payload,
        semantic_class: ProjectionSemanticClass::ActorMethodInvocationDeadlineExceeded,
        phase: ProjectionPhase::AwaitPrimitiveOutcome,
        effect: ProjectionEffect::OutcomeUnknown,
    },
    AdmissionCase {
        operation: ProjectionOperation::BytecodeArrayGet,
        payload: array_out_of_bounds_payload,
        semantic_class: ProjectionSemanticClass::ArrayIndexOutOfBounds,
        phase: ProjectionPhase::ExecuteInstruction,
        effect: ProjectionEffect::NoEffect,
    },
    AdmissionCase {
        operation: ProjectionOperation::BytecodeSetWritablePathArraySegment,
        payload: array_out_of_bounds_payload,
        semantic_class: ProjectionSemanticClass::ArrayIndexOutOfBounds,
        phase: ProjectionPhase::TraverseWritablePath,
        effect: ProjectionEffect::NoEffect,
    },
    AdmissionCase {
        operation: ProjectionOperation::BytecodeMapGet,
        payload: map_key_not_found_payload,
        semantic_class: ProjectionSemanticClass::MapKeyNotFound,
        phase: ProjectionPhase::ExecuteInstruction,
        effect: ProjectionEffect::NoEffect,
    },
    AdmissionCase {
        operation: ProjectionOperation::BytecodeSetWritablePathMapSegment,
        payload: map_key_not_found_payload,
        semantic_class: ProjectionSemanticClass::MapKeyNotFound,
        phase: ProjectionPhase::TraverseWritablePath,
        effect: ProjectionEffect::NoEffect,
    },
    AdmissionCase {
        operation: ProjectionOperation::ServiceCall,
        payload: instruction_limit_payload,
        semantic_class: ProjectionSemanticClass::ImportedInstructionLimitExceeded,
        phase: ProjectionPhase::ReceiveServiceOutcome,
        effect: ProjectionEffect::OutcomeUnknown,
    },
    AdmissionCase {
        operation: ProjectionOperation::LexicalTimeoutScope,
        payload: lexical_timeout_payload,
        semantic_class: ProjectionSemanticClass::LexicalScopeDeadlineExceeded,
        phase: ProjectionPhase::ScopeDeadlineWinner,
        effect: ProjectionEffect::OutcomeUnknown,
    },
    AdmissionCase {
        operation: ProjectionOperation::HttpRequest,
        payload: http_request_timeout_payload,
        semantic_class: ProjectionSemanticClass::HttpRequestDeadlineExceeded,
        phase: ProjectionPhase::AwaitPrimitiveOutcome,
        effect: ProjectionEffect::OutcomeUnknown,
    },
];

fn array_out_of_bounds_payload() -> PlatformErrorProjectionPayload {
    PlatformErrorProjectionPayload::StdCollectionArrayIndexOutOfBoundsError(
        StdCollectionArrayIndexOutOfBoundsErrorPayload {
            index: 7,
            length: 3,
        },
    )
}

fn map_key_not_found_payload() -> PlatformErrorProjectionPayload {
    PlatformErrorProjectionPayload::StdCollectionMapKeyNotFoundError(
        StdCollectionMapKeyNotFoundErrorPayload {},
    )
}

fn json_object_property_not_found_payload() -> PlatformErrorProjectionPayload {
    PlatformErrorProjectionPayload::StdCollectionJsonObjectPropertyNotFoundError(
        StdCollectionJsonObjectPropertyNotFoundErrorPayload {},
    )
}

fn lexical_timeout_payload() -> PlatformErrorProjectionPayload {
    PlatformErrorProjectionPayload::StdErrorTimeoutError(StdErrorTimeoutErrorPayload {
        timeout_ms: 1_000,
    })
}

fn http_request_timeout_payload() -> PlatformErrorProjectionPayload {
    PlatformErrorProjectionPayload::StdHttpRequestTimeoutError(StdHttpRequestTimeoutErrorPayload {
        timeout_ms: 2_000,
    })
}

fn actor_method_timeout_payload() -> PlatformErrorProjectionPayload {
    PlatformErrorProjectionPayload::StdActorMethodInvocationTimeoutError(
        StdActorMethodInvocationTimeoutErrorPayload { timeout_ms: 3_000 },
    )
}

fn actor_activation_timeout_payload() -> PlatformErrorProjectionPayload {
    PlatformErrorProjectionPayload::StdActorActivationTimeoutError(
        StdActorActivationTimeoutErrorPayload { timeout_ms: 4_000 },
    )
}

fn instruction_limit_payload() -> PlatformErrorProjectionPayload {
    PlatformErrorProjectionPayload::StdErrorInstructionLimitExceededError(
        StdErrorInstructionLimitExceededErrorPayload {
            instruction_count: 101,
            limit: 100,
        },
    )
}

fn candidate(case: AdmissionCase) -> ProjectionCandidate {
    ProjectionCandidate::new((case.payload)(), case.semantic_class, case.phase)
}

fn assert_denied(operation: ProjectionOperation, candidate: ProjectionCandidate) {
    let error = match admit_projection(operation, candidate) {
        Ok(_) => panic!("unlisted projection tuple must be denied"),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "platform error projection is not admitted"
    );
    assert_eq!(format!("{error:?}"), "ProjectionDenied");
}

#[test]
fn every_exact_policy_row_is_admitted_with_its_effect_and_payload() {
    for case in ADMISSION_CASES {
        let expected_payload = (case.payload)();
        let expected_key = expected_payload.key();
        let admitted = admit_projection(case.operation, candidate(case))
            .expect("the exact closed policy row must be admitted");

        assert_eq!(admitted.operation(), case.operation);
        assert_eq!(admitted.projection_key(), expected_key);
        assert_eq!(admitted.payload(), &expected_payload);
        assert_eq!(admitted.semantic_class(), case.semantic_class);
        assert_eq!(admitted.phase(), case.phase);
        assert_eq!(admitted.effect(), case.effect);
        assert_eq!(admitted.into_payload(), expected_payload);
    }
}

#[test]
fn every_policy_row_denies_a_wrong_operation_key_class_or_phase() {
    for case in ADMISSION_CASES {
        assert_denied(ProjectionOperation::TaskSubmit, candidate(case));

        let wrong_payload = json_object_property_not_found_payload();
        assert_ne!(wrong_payload.key(), (case.payload)().key());
        assert_denied(
            case.operation,
            ProjectionCandidate::new(wrong_payload, case.semantic_class, case.phase),
        );

        assert_denied(
            case.operation,
            ProjectionCandidate::new(
                (case.payload)(),
                ProjectionSemanticClass::ImportedFixedServiceFailure,
                case.phase,
            ),
        );

        let wrong_phase = match case.phase {
            ProjectionPhase::ExecuteInstruction => ProjectionPhase::TraverseWritablePath,
            ProjectionPhase::TraverseWritablePath => ProjectionPhase::ExecuteInstruction,
            ProjectionPhase::ScopeDeadlineWinner
            | ProjectionPhase::AwaitPrimitiveOutcome
            | ProjectionPhase::ReceiveServiceOutcome
            | ProjectionPhase::BeforeDispatch => ProjectionPhase::ExecuteInstruction,
        };
        assert_denied(
            case.operation,
            ProjectionCandidate::new((case.payload)(), case.semantic_class, wrong_phase),
        );
    }
}

#[test]
fn json_object_and_task_operations_have_no_admission_rows() {
    let json_get = ProjectionCandidate::new(
        json_object_property_not_found_payload(),
        ProjectionSemanticClass::JsonObjectPropertyNotFound,
        ProjectionPhase::ExecuteInstruction,
    );
    assert_denied(ProjectionOperation::BytecodeJsonObjectGet, json_get);

    let json_writable_segment = ProjectionCandidate::new(
        json_object_property_not_found_payload(),
        ProjectionSemanticClass::JsonObjectPropertyNotFound,
        ProjectionPhase::TraverseWritablePath,
    );
    assert_denied(
        ProjectionOperation::BytecodeSetWritablePathJsonObjectSegment,
        json_writable_segment,
    );

    for semantic_class in [
        ProjectionSemanticClass::TaskSubmitDefiniteRejection,
        ProjectionSemanticClass::TaskSubmitOutcomeUnknown,
    ] {
        assert_denied(
            ProjectionOperation::TaskSubmit,
            ProjectionCandidate::new(
                lexical_timeout_payload(),
                semantic_class,
                ProjectionPhase::BeforeDispatch,
            ),
        );
    }
}

#[test]
fn imported_fixed_service_failure_is_default_denied() {
    assert_denied(
        ProjectionOperation::ServiceCall,
        ProjectionCandidate::new(
            instruction_limit_payload(),
            ProjectionSemanticClass::ImportedFixedServiceFailure,
            ProjectionPhase::ReceiveServiceOutcome,
        ),
    );
}

fn row_sort_key(
    row: &ProjectionAdmissionRow,
) -> (
    &'static str,
    ProjectionOperation,
    ProjectionSemanticClass,
    ProjectionPhase,
) {
    (
        row.projection_key.as_str(),
        row.operation,
        row.semantic_class,
        row.phase,
    )
}

#[test]
fn admission_table_is_ascii_ordered_duplicate_free_and_complete() {
    assert_eq!(PROJECTION_ADMISSION_ROWS.len(), ADMISSION_CASES.len());
    assert!(PROJECTION_ADMISSION_ROWS
        .iter()
        .all(|row| row.projection_key.as_str().is_ascii()));
    assert!(PROJECTION_ADMISSION_ROWS
        .windows(2)
        .all(|rows| row_sort_key(&rows[0]) < row_sort_key(&rows[1])));

    let mut exact_tuples = HashSet::new();
    for row in PROJECTION_ADMISSION_ROWS {
        assert!(exact_tuples.insert((
            row.operation,
            row.projection_key,
            row.semantic_class,
            row.phase,
        )));
    }

    for (row, case) in PROJECTION_ADMISSION_ROWS.iter().zip(ADMISSION_CASES) {
        assert_eq!(row.operation, case.operation);
        assert_eq!(row.projection_key, (case.payload)().key());
        assert_eq!(row.semantic_class, case.semantic_class);
        assert_eq!(row.phase, case.phase);
        assert_eq!(row.effect, case.effect);
    }
}
