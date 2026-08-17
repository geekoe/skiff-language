//! Router-owned Actor owner invoke/control execution.
//!
//! The RuntimeHost runs the exact linked Actor method after the Router
//! commits an owner fence. The concrete executor stays small and synchronous
//! for the Phase 6 durable task surface; methods that would park or enter a
//! child fail closed instead of leaking a half-finished incarnation.

use std::{
    num::{NonZeroU32, NonZeroUsize},
    sync::Arc,
};

use base64::Engine as _;
use skiff_runtime_model::{
    bytecode_execution_observation::{BytecodeExecutionCorrelation, BytecodeExecutionObserver},
    request_heap::RequestHeapLimits,
    runtime_value::ActorRef,
    vm_heap::VmHeap,
};
use skiff_runtime_request::RequestVmHeap;
use skiff_runtime_transport::actor_method::{
    encode_actor_method_frame, ActorMethodFrame, ActorMethodReturnFrameHeader,
    ACTOR_RETURN_ENCODING_V1,
};
use skiff_runtime_transport::actor_owner::{
    decode_actor_owner_control_frame, decode_actor_owner_invoke_frame,
    encode_actor_owner_control_ack_frame, ActorOwnerControlAckFrameHeader,
    ActorOwnerControlOperation, ACTOR_OWNER_CONTROL_ACK_FRAME_TYPE,
};
use skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION;
use skiff_runtime_vm::{Vm, VmBudget, VmCompletion, VmControl, VmLimits};
use tokio::sync::mpsc;

use crate::{
    error::{Result, RuntimeError},
    host::RouterWriterMessage,
    host::{bytecode_actor_executor::allocate_actor_state, RuntimeHost},
};

pub(super) async fn handle_actor_owner_frame(
    host: &RuntimeHost,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<RouterWriterMessage>,
) -> Result<()> {
    let frame_type = skiff_runtime_transport::protocol::decode_binary_frame(bytes)
        .map_err(super::transport_error_into_runtime_error)?
        .header
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    match frame_type.as_str() {
        "actor.owner.control" => handle_actor_owner_control(host, bytes, sender).await,
        "actor.owner.invoke" => handle_actor_owner_invoke(host, bytes, sender).await,
        other => Err(RuntimeError::Decode(format!(
            "unexpected actor owner frame type {other}"
        ))),
    }
}

async fn handle_actor_owner_control(
    host: &RuntimeHost,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<RouterWriterMessage>,
) -> Result<()> {
    let control = decode_actor_owner_control_frame(bytes)
        .map_err(super::transport_error_into_runtime_error)?;
    if control.target_runtime_id != host.base_runtime_id {
        return Err(RuntimeError::Decode(
            "actor.owner.control targets a different Runtime".to_string(),
        ));
    }
    let accepted = match control.operation {
        ActorOwnerControlOperation::ActivateInitial => true,
        ActorOwnerControlOperation::Activate
        | ActorOwnerControlOperation::Discard
        | ActorOwnerControlOperation::IdleEvict
        | ActorOwnerControlOperation::MarkUpgrading => true,
    };
    let ack = ActorOwnerControlAckFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: ACTOR_OWNER_CONTROL_ACK_FRAME_TYPE.to_string(),
        runtime_id: host.base_runtime_id.clone(),
        request_id: control.request_id,
        operation: control.operation,
        accepted,
        reason: None,
    };
    let bytes = encode_actor_owner_control_ack_frame(&ack).map_err(|error| {
        RuntimeError::Decode(format!("actor owner control ack encode failed: {error}"))
    })?;
    sender
        .send(RouterWriterMessage::Binary(bytes))
        .map_err(|_| RuntimeError::Decode("actor owner control writer closed".to_string()))
}

async fn handle_actor_owner_invoke(
    host: &RuntimeHost,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<RouterWriterMessage>,
) -> Result<()> {
    let (header, _arguments_payload) = decode_actor_owner_invoke_frame(bytes)
        .map_err(super::transport_error_into_runtime_error)?;
    if header.target_runtime_id != host.base_runtime_id {
        return Err(RuntimeError::Decode(
            "actor.owner.invoke targets a different Runtime".to_string(),
        ));
    }
    let image = host
        .bytecode_deployments
        .loaded_sync_by_build_id(&header.route_authority.build_id)
        .ok_or_else(|| {
            RuntimeError::Decode(format!(
                "actor owner invoke build {} is not loaded",
                header.route_authority.build_id
            ))
        })?;
    let method = image
        .actor_methods()
        .iter()
        .find(|method| {
            method.actor_abi_identity() == &header.invoke.actor_abi_identity
                && method.actor_implementation_identity()
                    == &header.invoke.actor_implementation_identity
                && method.method_identity() == &header.invoke.method_identity
        })
        .ok_or_else(|| {
            RuntimeError::Decode(
                "actor owner invoke method is absent from the exact execution image".to_string(),
            )
        })?;
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(&header.invoke.actor_ref.canonical_actor_id_key_bytes_base64)
        .map_err(|error| RuntimeError::Decode(format!("actor owner key decode failed: {error}")))?;
    let actor_ref = ActorRef::new(
        header.invoke.actor_ref.service_id.clone(),
        method
            .actor_implementation()
            .actor_type_identity()
            .to_string(),
        method
            .actor_implementation()
            .actor_id_type_identity()
            .to_string(),
        header.invoke.actor_ref.actor_id_encoding_version.clone(),
        key_bytes,
        header.invoke.actor_ref.actor_id_hash.clone(),
        Some(header.owner_fence.epoch),
    );
    if method.signature().parameter_types().len() > 1 {
        return Err(RuntimeError::Decode(
            "actor owner invoke method arguments are not supported by the Phase 6 task control surface"
                .to_string(),
        ));
    }

    let limits = RequestHeapLimits {
        max_estimated_bytes: host.memory_budgets.request_heap_bytes,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestVmHeap::new(limits);
    let observer = BytecodeExecutionObserver::new(
        Arc::clone(&host.bytecode_execution_event_sink),
        BytecodeExecutionCorrelation {
            router_session_id: "actor-owner".to_string(),
            request_id: header.invoke.invocation_id.clone(),
        },
    );
    let vm_limits = VmLimits::new(
        NonZeroUsize::new(128).expect("VM frame limit is non-zero"),
        NonZeroUsize::new(4096).expect("VM value slot limit is non-zero"),
        NonZeroU32::new(1024).expect("VM segment instruction limit is non-zero"),
    );
    let actor_type = *method
        .signature()
        .parameter_types()
        .first()
        .ok_or_else(|| RuntimeError::Decode("actor method signature has no self".to_string()))?;
    let key_field = method.actor_implementation().key_field().to_string();
    let state = allocate_actor_state(
        &mut heap,
        &image,
        actor_type,
        &key_field,
        &actor_ref,
        method.actor_implementation().state_fields(),
    )
    .map_err(|error| RuntimeError::Decode(error.to_string()))?;

    let create = image
        .actor_creates()
        .iter()
        .find(|create| {
            create
                .actor_implementation()
                .actor_implementation_identity()
                == method
                    .actor_implementation()
                    .actor_implementation_identity()
        })
        .ok_or_else(|| RuntimeError::Decode("actor create row is absent".to_string()))?;
    let create_entry = image
        .function_entry(create.function())
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
    let mut budget = NoopVmBudget;
    let mut create_fiber = Vm::start_with_retained_parameter(
        create_entry,
        vec![state].into_boxed_slice(),
        vm_limits,
        observer.clone(),
    )
    .map_err(|error| RuntimeError::Decode(error.to_string()))?;
    let _ = run_to_completion(&mut create_fiber, &mut heap, &mut budget)
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
    let state = create_fiber
        .take_terminal_retained_parameter()
        .ok_or_else(|| RuntimeError::Decode("actor create lost its self root".to_string()))?;

    let method_entry = image
        .function_entry(method.function())
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
    let mut method_fiber = Vm::start_with_retained_parameter(
        method_entry,
        vec![state].into_boxed_slice(),
        vm_limits,
        observer.clone(),
    )
    .map_err(|error| RuntimeError::Decode(error.to_string()))?;
    let _completion = run_to_completion(&mut method_fiber, &mut heap, &mut budget)
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;

    let frame = ActorMethodFrame::Return(
        ActorMethodReturnFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.method.return".to_string(),
            invocation_id: header.invoke.invocation_id,
            return_encoding_version: ACTOR_RETURN_ENCODING_V1.to_string(),
        },
        b"null".to_vec(),
    );
    let bytes = encode_actor_method_frame(&frame).map_err(|error| {
        RuntimeError::Decode(format!("actor method return encode failed: {error}"))
    })?;
    sender
        .send(RouterWriterMessage::Binary(bytes))
        .map_err(|_| RuntimeError::Decode("actor method return writer closed".to_string()))
}

fn run_to_completion(
    fiber: &mut skiff_runtime_vm::VmFiber,
    heap: &mut dyn VmHeap,
    budget: &mut dyn VmBudget,
) -> std::result::Result<VmCompletion, String> {
    loop {
        match fiber.run_segment(heap, budget) {
            VmControl::Complete(completion) => return Ok(completion),
            VmControl::Continue => {}
            VmControl::EnterChild(_)
            | VmControl::EnterAdapter(_)
            | VmControl::EmitStream(_)
            | VmControl::Park(_) => {
                return Err(format!("actor method reached unsupported VM control"));
            }
        }
    }
}

struct NoopVmBudget;

impl VmBudget for NoopVmBudget {
    fn before_dispatch(&mut self) -> std::result::Result<(), skiff_runtime_vm::VmBudgetClosed> {
        Ok(())
    }

    fn poll_interrupt(&mut self) -> std::result::Result<(), skiff_runtime_vm::VmBudgetClosed> {
        Ok(())
    }

    fn charge_semantic(
        &mut self,
        _charge: skiff_runtime_vm::VmSemanticCharge<'_>,
    ) -> std::result::Result<(), skiff_runtime_vm::VmBudgetClosed> {
        Ok(())
    }
}
