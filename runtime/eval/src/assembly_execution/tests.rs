use super::*;

#[test]
fn assembly_execution_handoff_lanes_are_distinct_and_fail_closed() {
    for (lane, expected) in [
        (AssemblyExecutionLaneKind::OrdinaryError, "ordinary/error"),
        (
            AssemblyExecutionLaneKind::AsyncStreamCancel,
            "async/stream/cancel",
        ),
        (AssemblyExecutionLaneKind::CallbackNative, "callback/native"),
    ] {
        let error = AssemblyExecutionHandoffError::unavailable(lane);
        assert!(matches!(error, RuntimeError::ProviderUnavailable { .. }));
        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn assembly_execution_handoff_missing_target_is_structured() {
    let error = RuntimeError::from(RuntimeAssemblyEvalSeamError::MissingExecutionTarget);
    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
    assert!(error.to_string().contains("no runtime assembly target"));
}

#[test]
fn unsupported_stream_contract_remains_a_typed_runtime_error() {
    let error = unsupported_stream_error(
        &ContractOperationId::new("operation:unsupported-stream"),
        &BoundaryFeatureUnavailableReason::UnknownSemantics,
    );
    assert!(matches!(error, RuntimeError::Unsupported(_)));
}
