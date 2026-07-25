use std::sync::Arc;

use skiff_artifact_model::{
    ContractOperationId, PackageBuildId, PackageCallableId, ServiceProtocolIdentity,
};
use skiff_runtime_capability_context::{
    OutboundResponse, RequestPayloadContext, StreamRuntimeError,
};
use skiff_runtime_linked_program::{LinkedCallTarget, LinkedExprIr};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    service_error::{ExceptionStackFrame, ServiceErrorEnvelope},
};

use super::{
    async_stream_cancel,
    ordinary::tests::{
        service_error_consumer::{
            ConsumerTopology, ProviderFailureKind, ServiceErrorConsumerFixture,
        },
        test_runtime,
    },
    service_error_channel::ServiceErrorExportContext,
    service_error_channel::{CanonicalServiceErrorChannel, ServiceErrorImportContext},
    RuntimeAssemblyExecutionProjection,
};
use crate::{
    env::Env,
    error::RuntimeError,
    eval_context::EvalContext,
    exceptions::user_exception_for_catch,
    test_effect_registry::{
        RegisteredTestEffect, RegisteredTestEffectFailure, RegisteredTestEffectOutcome,
        RegisteredTestEffectThrow, RuntimeTestEffectRegistry, ServiceTestEffectDispatch,
        TestEffectTarget,
    },
    Interpreter,
};

#[tokio::test]
async fn service_error_channel_contract_operation_converges_real_lane_carriers() {
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());

    let (ordinary_result, ordinary_heap, _) = fixture.execute_internal(&interpreter).await;
    let ordinary_error = ordinary_result.expect_err("ordinary provider must throw");
    let ordinary_exception = user_exception_for_catch(&ordinary_error)
        .expect("ordinary central dispatcher must import the fixed failure")
        .request();
    let ordinary_fixed = ordinary_exception
        .fixed_service_error()
        .expect("ordinary import must retain the fixed carrier")
        .clone();
    assert!(ordinary_exception.local_value().is_some());
    assert!(!ordinary_heap.is_empty());

    let caller_target = fixture.caller_eval_target();
    let image = Arc::clone(caller_target.execution_image());
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&image));
    let caller = projection
        .resolve_executable(fixture.caller_addr())
        .expect("linked caller executable");
    let call = caller
        .executable
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            LinkedExprIr::Call { call }
                if matches!(
                    call.target,
                    LinkedCallTarget::ActivationRelativeService { .. }
                ) =>
            {
                Some(call.clone())
            }
            _ => None,
        })
        .expect("linked caller service operation");
    let instruction = match &call.target {
        LinkedCallTarget::ActivationRelativeService { instruction } => instruction.clone(),
        _ => unreachable!("the call was selected by its typed target"),
    };
    let async_target = caller_target
        .resolve_service_call(&instruction)
        .expect("resolved async-unary target");
    let remote_service_id = async_target.contract().service_id.clone();
    let remote_operation_id = async_target.descriptor().operation_id.as_str().to_string();
    let context = fixture.execution_context(&interpreter, caller_target);
    let mut async_heap = RequestHeap::default();
    let mut env = Env::new();
    let mut eval_context = EvalContext::new(
        &interpreter,
        context,
        &mut async_heap,
        &mut env,
        &caller.addr,
        caller.file.as_ref(),
        caller.executable,
    )
    .expect("async-unary caller context");
    let async_result = async_stream_cancel::execute_service_call(
        &mut eval_context,
        &call,
        async_target,
        Vec::new(),
    )
    .await;
    let caller_stack_at_site = eval_context
        .context
        .exception_stack_for_site(call.site.clone());
    drop(eval_context);
    let async_fixed = match async_result {
        Err(RuntimeError::FixedServiceFailure(error)) => error,
        other => panic!("async unary must export the same fixed carrier, got {other:?}"),
    };
    assert_eq!(async_fixed.encoded_bytes(), ordinary_fixed.encoded_bytes());

    let async_import = CanonicalServiceErrorChannel::import_caller_failure(
        async_fixed.clone(),
        ServiceErrorImportContext {
            execution_image: &image,
            type_view: projection.type_view(),
            caller_heap: &mut async_heap,
            caller_package_build_id: fixture.caller_build(),
            caller_executable_addr: fixture.caller_addr(),
            call_site: &call.site,
            caller_stack_at_site: &caller_stack_at_site,
            remote_service_id: &remote_service_id,
            remote_operation_id: &remote_operation_id,
        },
    )
    .expect("async unary fixed carrier must use the common importer");
    assert!(async_import.request().local_value().is_some());
    assert_eq!(
        async_import
            .request()
            .fixed_service_error()
            .expect("async import fixed cause")
            .encoded_bytes(),
        ordinary_fixed.encoded_bytes()
    );

    let ingress_target = fixture.terminal_eval_target();
    let target = fixture.ingress_target(&ingress_target);
    let ingress_context = fixture.execution_context(&interpreter, ingress_target);
    let mut ingress_heap = RequestHeap::default();
    let request = RequestPayloadContext::new("convergence-ingress", &[], None);
    let ingress_error = super::dispatch_ingress_via_in_process_boundary(
        &interpreter,
        ingress_context,
        &mut ingress_heap,
        target,
        &request,
    )
    .await
    .expect_err("ingress must hand the fixed carrier upward");
    let RuntimeError::FixedServiceFailure(ingress_fixed) = ingress_error else {
        panic!("ingress must not import an external caller exception");
    };
    assert_eq!(
        ingress_fixed.encoded_bytes(),
        ordinary_fixed.encoded_bytes()
    );
    assert!(ingress_heap.is_empty());

    let stream_terminal = StreamRuntimeError::fixed_service_failure_with_import(
        async_fixed.clone(),
        fixture.caller_build().clone(),
        fixture.caller_addr().clone(),
        call.site.clone(),
        caller_stack_at_site.clone(),
        remote_service_id.clone(),
        remote_operation_id.clone(),
    );
    let (stream_fixed, stream_import) = stream_terminal
        .fixed_service_failure_parts()
        .expect("typed stream terminal");
    let (
        caller_package_build_id,
        caller_executable_addr,
        call_site,
        stream_stack,
        stream_service_id,
        stream_operation_id,
    ) = stream_import.expect("program-stream import provenance");
    assert_eq!(stream_fixed.encoded_bytes(), ordinary_fixed.encoded_bytes());
    let mut stream_consumer_heap = RequestHeap::default();
    let stream_imported = CanonicalServiceErrorChannel::import_caller_failure(
        stream_fixed.clone(),
        ServiceErrorImportContext {
            execution_image: &image,
            type_view: projection.type_view(),
            caller_heap: &mut stream_consumer_heap,
            caller_package_build_id,
            caller_executable_addr,
            call_site,
            caller_stack_at_site: stream_stack,
            remote_service_id: stream_service_id,
            remote_operation_id: stream_operation_id,
        },
    )
    .expect("program-stream consumer must use the common importer");
    assert!(stream_imported.request().local_value().is_some());
    assert_eq!(
        stream_imported
            .request()
            .fixed_service_error()
            .expect("stream import fixed cause")
            .encoded_bytes(),
        ordinary_fixed.encoded_bytes()
    );

    let response = OutboundResponse::fixed_service_failure(async_fixed);
    let OutboundResponse::FixedServiceFailure(response) = response else {
        unreachable!("typed response constructor returned the wrong branch");
    };
    assert_eq!(
        response.error().encoded_bytes(),
        ordinary_fixed.encoded_bytes()
    );
    assert!(matches!(
        response.error().envelope(),
        ServiceErrorEnvelope::PublicTypedError { .. }
    ));
}

#[tokio::test]
async fn service_error_channel_contract_operation_restores_after_an_unlinked_middle_hop() {
    let unlinked = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        false,
        false,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let (unlinked_result, unlinked_heap, _) = unlinked.execute_internal(&interpreter).await;
    let unlinked_error = unlinked_result.expect_err("unlinked middle hop must stay opaque");
    let unlinked_exception = user_exception_for_catch(&unlinked_error)
        .expect("unlinked fixed failure remains a request exception")
        .request();
    assert!(unlinked_exception.local_value().is_none());
    let original = unlinked_exception
        .fixed_service_error()
        .expect("unlinked middle hop retains raw bytes")
        .clone();

    let unlinked_target = unlinked.caller_eval_target();
    let unlinked_image = Arc::clone(unlinked_target.execution_image());
    let unlinked_projection =
        RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&unlinked_image));
    let forwarded = CanonicalServiceErrorChannel::export_provider_failure(
        &unlinked_error,
        ServiceErrorExportContext {
            execution_image: &unlinked_image,
            type_view: unlinked_projection.type_view(),
            provider_heap: &unlinked_heap,
            provider_package_build_id: unlinked.caller_build(),
            caller_package_build_id: None,
            provider_service_id: "service:opaque-middle",
            operation_id: "operation:forward",
        },
        || panic!("an imported opaque cause cannot allocate a new correlation"),
    )
    .expect("unlinked middle hop forwards the original carrier");
    assert_eq!(forwarded.encoded_bytes(), original.encoded_bytes());

    let linked = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let linked_target = linked.caller_eval_target();
    let linked_image = Arc::clone(linked_target.execution_image());
    let linked_projection =
        RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&linked_image));
    let caller_stack = [ExceptionStackFrame::Local {
        site: linked.caller_site().clone(),
    }];
    let mut linked_heap = RequestHeap::default();
    let restored = CanonicalServiceErrorChannel::import_caller_failure(
        forwarded,
        ServiceErrorImportContext {
            execution_image: &linked_image,
            type_view: linked_projection.type_view(),
            caller_heap: &mut linked_heap,
            caller_package_build_id: linked.caller_build(),
            caller_executable_addr: linked.caller_addr(),
            call_site: linked.caller_site(),
            caller_stack_at_site: &caller_stack,
            remote_service_id: linked.contract().service_id.as_str(),
            remote_operation_id: linked.operation_id().as_str(),
        },
    )
    .expect("the next exact linked caller restores a local value");
    assert!(restored.request().local_value().is_some());
    assert_eq!(
        restored
            .request()
            .fixed_service_error()
            .expect("restored caller retains the original carrier")
            .encoded_bytes(),
        original.encoded_bytes()
    );
}

#[test]
fn service_error_channel_contract_operation_keeps_effect_targets_exact() {
    let fixed = skiff_runtime_model::service_error::OpaqueServiceError::decode(
        skiff_canonical_json::canonical_json_bytes(&ServiceErrorEnvelope::PublicTypedError {
            package_id: "unlinked.example/errors".to_string(),
            stable_schema_key: "Opaque".to_string(),
            package_schema_type_id: "type:opaque".into(),
            encoded_payload: vec![1, 2, 3],
            trace_id: "trace:effect-convergence".to_string(),
            error_id: "trace:effect-convergence:error:1".to_string(),
        })
        .expect("canonical effect envelope"),
    )
    .expect("strict effect carrier");
    let registry = RuntimeTestEffectRegistry::default();
    let service_target = TestEffectTarget::contract_operation(
        ContractOperationId::new("operation:convergence"),
        ServiceProtocolIdentity::new("protocol:convergence"),
    );
    registry.register(
        service_target.clone(),
        RegisteredTestEffect {
            expect: None,
            step_expect: None,
            outcome: RegisteredTestEffectOutcome::Throw(RegisteredTestEffectThrow {
                failure: RegisteredTestEffectFailure::FixedService(fixed.clone()),
                setup_heap: RequestHeap::default(),
                setup_package_build_id: PackageBuildId::new("build:effect-provider"),
            }),
        },
    );
    let mut caller_heap = RequestHeap::default();
    let dispatch = registry
        .dispatch_service(&service_target, &[], None, &mut caller_heap)
        .expect("registered ContractOperation")
        .expect("typed service effect");
    let ServiceTestEffectDispatch::Throw(throw) = dispatch else {
        panic!("ContractOperation fixed failure must remain a throw");
    };
    let RegisteredTestEffectFailure::FixedService(dispatched) = throw.failure else {
        panic!("ContractOperation must retain the fixed service branch");
    };
    assert_eq!(dispatched.encoded_bytes(), fixed.encoded_bytes());
    assert!(throw.setup_heap.is_empty());
    assert!(caller_heap.is_empty());

    let package_target = TestEffectTarget::package_callable(
        PackageBuildId::new("build:package-local"),
        PackageCallableId::new("callable:package-local"),
    );
    let error = match registry
        .dispatch_service(&package_target, &[], None, &mut caller_heap)
        .expect("wrong typed target must fail closed")
    {
        Ok(_) => panic!("PackageCallable cannot enter the service effect channel"),
        Err(error) => error,
    };
    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
}
