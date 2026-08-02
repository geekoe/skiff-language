use std::time::{Duration, Instant};

use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_capability_context::{
    CancellationSource, ExecutionScope, ExecutionScopeTerminal,
};
use skiff_runtime_model::request_heap::RequestHeap;

use super::{
    eval_error_to_native, rematerialize_runtime_error_between_heaps, OrdinaryRuntimeError,
    RequestHeapOwnedStreamError, RuntimeError, ScopeTerminalCarrier,
};

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

#[test]
fn scope_terminal_normal_cancel_local_inherited_matrix_keeps_internal_boundary() {
    let base = Instant::now();
    let request_cancel = CancellationSource::new();
    let root = ExecutionScope::request(
        request_cancel.token(),
        Some(base + Duration::from_millis(5)),
    );
    assert!(root.terminal_at(base).is_none());

    request_cancel.cancel();
    assert!(matches!(
        ScopeTerminalCarrier::runtime_error(ExecutionScopeTerminal::AncestorCancelled),
        RuntimeError::Cancelled
    ));

    let local_root = ExecutionScope::request(CancellationSource::new().token(), None);
    let local_scope = local_root
        .derive(base + Duration::from_millis(5), site())
        .expect("local scope");
    let local_terminal = local_scope
        .terminal_at(base + Duration::from_millis(5))
        .expect("local terminal");
    let local = ScopeTerminalCarrier::new(local_terminal);
    assert!(local.is_owned_by(&local_scope));

    let inherited_root = ExecutionScope::request(
        CancellationSource::new().token(),
        Some(base + Duration::from_millis(5)),
    );
    let inherited_scope = inherited_root
        .derive(base + Duration::from_millis(10), site())
        .expect("inherited scope");
    let inherited_terminal = inherited_scope
        .terminal_at(base + Duration::from_millis(5))
        .expect("inherited terminal");
    let inherited = ScopeTerminalCarrier::new(inherited_terminal);
    assert!(!inherited.is_owned_by(&inherited_scope));
}

#[test]
fn scope_terminal_is_rejected_by_payload_catch_wire_and_stream_wrappers() {
    let base = Instant::now();
    let root = ExecutionScope::request(CancellationSource::new().token(), None);
    let scope = root
        .derive(base + Duration::from_millis(1), site())
        .expect("scope should derive");
    let terminal = scope
        .terminal_at(base + Duration::from_secs(1))
        .expect("deadline should be terminal");
    let error = RuntimeError::ScopeTerminal(ScopeTerminalCarrier::new(terminal));

    assert!(error.ordinary_payload().is_none());
    assert!(error.ordinary_catch_projection().is_none());
    assert!(matches!(
        OrdinaryRuntimeError::try_new(error),
        Err(RuntimeError::ScopeTerminal(_))
    ));

    let terminal = scope
        .terminal_at(base + Duration::from_secs(1))
        .expect("terminal remains observable");
    let error = RuntimeError::ScopeTerminal(ScopeTerminalCarrier::new(terminal));
    assert!(matches!(
        RequestHeapOwnedStreamError::try_new(error, RequestHeap::default()),
        Err(RuntimeError::ScopeTerminal(_))
    ));
}

#[test]
fn scope_terminal_survives_diagnostic_wrappers_without_becoming_catchable() {
    let base = Instant::now();
    let root = ExecutionScope::request(CancellationSource::new().token(), None);
    let scope = root
        .derive(base + Duration::from_millis(1), site())
        .expect("scope");
    let terminal = scope
        .terminal_at(base + Duration::from_millis(1))
        .expect("terminal");
    let error = RuntimeError::ScopeTerminal(ScopeTerminalCarrier::new(terminal))
        .with_source(7, serde_json::json!({ "sourceId": 7 }))
        .with_diagnostic_frame(serde_json::json!({ "kind": "test" }));

    let carrier = error.scope_terminal().expect("wrapped terminal");
    assert!(carrier.is_owned_by(&scope));
    assert!(error.ordinary_payload().is_none());
    assert!(error.ordinary_catch_projection().is_none());
    assert!(matches!(
        OrdinaryRuntimeError::try_new(error),
        Err(RuntimeError::WithDiagnosticFrame { .. })
    ));
}

#[test]
fn scope_terminal_cannot_be_erased_into_native_ordinary_error() {
    let base = Instant::now();
    let root = ExecutionScope::request(CancellationSource::new().token(), None);
    let scope = root
        .derive(base + Duration::from_millis(1), site())
        .expect("scope");
    let terminal = scope
        .terminal_at(base + Duration::from_millis(1))
        .expect("terminal");
    let native = eval_error_to_native(RuntimeError::ScopeTerminal(ScopeTerminalCarrier::new(
        terminal,
    )));
    assert!(matches!(
        native,
        skiff_runtime_native::error::RuntimeError::Cancelled
    ));
}

#[test]
fn scope_terminal_cross_heap_rematerialization_is_an_exact_heap_free_passthrough() {
    let base = Instant::now();
    let root = ExecutionScope::request(CancellationSource::new().token(), None);
    let scope = root
        .derive(base + Duration::from_millis(1), site())
        .expect("scope");
    let terminal = scope
        .terminal_at(base + Duration::from_millis(1))
        .expect("terminal");
    let error = RuntimeError::ScopeTerminal(ScopeTerminalCarrier::new(terminal));
    let source = RequestHeap::default();
    let mut destination = RequestHeap::default();

    let materialized = rematerialize_runtime_error_between_heaps(error, &source, &mut destination)
        .expect("scope terminal passthrough cannot materialize a value");
    assert!(matches!(materialized, RuntimeError::ScopeTerminal(_)));
    assert!(destination.is_empty());
}
