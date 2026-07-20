use std::cell::Cell;

use skiff_runtime_model::{
    recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    },
    request_heap::RequestHeap,
    runtime_value::{CallbackCapabilityCarrier, InterfaceCarrier, InterfaceValue, RuntimeValue},
};

use crate::{
    error::{RecoverableBoundaryErrorCode, RuntimeError},
    recoverable::{
        RecoverableBehaviorHooks, RecoverableBoundaryCodec, RecoverableEncodedLocalInterfaceSelf,
        RecoverableInterfaceConformanceRequest, RecoverableInterfaceMethodTableRequest,
        RecoverableLocalInterfaceEncodeRequest, RecoverableLocalInterfaceRestoreRequest,
        RecoverableRemoteInterfaceCarrierRequest, RecoverableRestoredLocalInterfaceSelf,
    },
};

#[derive(Default)]
struct CountingRecoverableHooks {
    calls: Cell<usize>,
}

impl RecoverableBehaviorHooks for CountingRecoverableHooks {
    fn encode_local_interface_self(
        &self,
        _request: RecoverableLocalInterfaceEncodeRequest<'_>,
        _heap: &RequestHeap,
    ) -> crate::Result<Option<RecoverableEncodedLocalInterfaceSelf>> {
        self.calls.set(self.calls.get() + 1);
        Ok(None)
    }

    fn restore_local_interface_self(
        &self,
        _request: RecoverableLocalInterfaceRestoreRequest<'_>,
        _heap: &mut RequestHeap,
    ) -> crate::Result<Option<RecoverableRestoredLocalInterfaceSelf>> {
        self.calls.set(self.calls.get() + 1);
        Ok(None)
    }

    fn concrete_type_conforms_to_interface(
        &self,
        _request: RecoverableInterfaceConformanceRequest<'_>,
    ) -> crate::Result<bool> {
        self.calls.set(self.calls.get() + 1);
        Ok(false)
    }

    fn rebuild_local_interface_method_table(
        &self,
        _request: RecoverableInterfaceMethodTableRequest<'_>,
    ) -> crate::Result<Option<skiff_runtime_model::runtime_value::InterfaceMethodTable>> {
        self.calls.set(self.calls.get() + 1);
        Ok(None)
    }

    fn rebuild_remote_interface_operation_table(
        &self,
        _request: RecoverableRemoteInterfaceCarrierRequest<'_>,
    ) -> crate::Result<Option<skiff_runtime_model::runtime_value::RemoteOperationTable>> {
        self.calls.set(self.calls.get() + 1);
        Ok(None)
    }
}

#[test]
fn callback_materialization_is_terminal_at_the_persistent_boundary() {
    let carrier = CallbackCapabilityCarrier::new(
        "runtime-a",
        "activation-a",
        41,
        "contract:observer",
        "callback:41:1",
    );
    let mut heap = RequestHeap::default();
    let value = RuntimeValue::Heap(
        heap.alloc_interface(InterfaceValue::new(
            "contract:observer".to_string(),
            InterfaceCarrier::CallbackCapability(carrier),
        ))
        .expect("opaque callback wrapper should allocate"),
    );
    let context = RuntimeRecoverableBoundaryContext::new(
        RuntimeRecoverableBoundaryKind::QueueWorkItemPayload,
        RuntimeRecoverableTrustBoundary::OwnerInternal,
        RuntimeRecoverableStorageLane::RecoverableEnvelope,
    )
    .with_explicit_recoverable_slot();
    let expected = RuntimeRecoverableExpectedTypePlan::unresolved("callback");

    let hooks = CountingRecoverableHooks::default();
    let error =
        RecoverableBoundaryCodec::encode_with_behavior(&value, &expected, &context, &heap, &hooks)
            .expect_err("request callback must not enter a persistent lane");
    let RuntimeError::Recoverable(error) = error else {
        panic!("callback persistence rejection must stay structured");
    };
    assert_eq!(
        error.code(),
        RecoverableBoundaryErrorCode::CallbackCapabilityNotRecoverable
    );
    assert_eq!(
        error
            .detail()
            .and_then(|detail| detail.get("rebuildAttempted"))
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        error
            .detail()
            .and_then(|detail| detail.get("fallbackAttempted"))
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        hooks.calls.get(),
        0,
        "persistent rejection must precede encode, conformance, rebuild, and fallback hooks"
    );
}
