use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
    task::{Wake, Waker},
};

use bytes::Bytes;
use serde_json::{json, Value};
use skiff_artifact_model::{
    CanonicalPublicCallableSignature, InstructionSourceSite, InterfaceInstantiationRef, LiteralIr,
    OperationAbiRef, PublicationAbiUnit, PublicationOperationAbi, PublicationOperationKind,
    PublicationPublicInstanceExport, SourceCallMethodIndexEntry, SyntheticInstructionSiteReason,
    TypeRefIr,
};
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_boundary::{
    binary::encode_payload_plan,
    file::{immutable_file_wire, FileCreateOptions, ImmutableFileRef},
    json::RuntimeBoundaryCodec,
    payload::{PayloadBoundary, PayloadBoundaryKind},
    plan::BoundaryUse,
};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorCapabilityApi, ActorCapabilityContext, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorInvocationOutcome, ActorInvocationRequest,
    ActorRemoveControlRequest, ActorReplaceControlRequest, CancellationToken, CapabilityError,
    CapabilityFuture, CapabilityResult, DbCapabilityContext, FileCapabilityApi,
    FileCapabilityContext, FileCapabilityFuture, FileCapabilitySource, FileCapabilitySourceApi,
    FileChunkSource, FileSourceStreamApi, FileSourceStreamContext, OutboundRequestCancelSendError,
    OutboundRequestCancelSender, OutboundRequestRegistry, OutboundResponse,
    OutboundResponseReceiver, OutboundStartedRequest, OwnedActorCapabilityContext,
    RequestEffectDoubleControl, SpawnSubmitControlRequest, StreamCancelSignal, StreamInternalItem,
    StreamLifetimeGuard, StreamPoll, StreamPullSource, StreamRuntime, StreamRuntimeApi,
    StreamRuntimeError, StreamRuntimeResult, StreamSink, StreamSinkApi,
};
use skiff_runtime_linked_program::{
    anonymous_type_decl, CallIr, DbQueryIr, DbTargetIr, LinkedCallTarget, LinkedExecutableBody,
    LinkedInterfaceInstantiationRef, LinkedRemoteOperationSlotPlanIr,
    LinkedRemoteOperationTablePlanIr, LinkedTypeDescriptor, NativeTarget, PackageSymbolKey,
    PackageUnit, ServiceDependencyConstraint, ServiceDependencySymbolRef, ServiceMeta, SlotIr,
    SlotLayoutIr, TypeAddr,
};
use skiff_runtime_model::{
    request_heap::RequestHeapLimits,
    runtime_value::ActorRef,
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};

use super::*;
use crate::{
    capabilities::{EvalCapabilityFuture, OutboundServiceApi, OutboundServiceContext},
    env::Env,
    eval_context::EvalContext,
    program_execution::ProgramExecutionInput,
};
#[path = "evaluator_actual_pending/actor_dispatch.rs"]
mod actor_dispatch;
#[path = "evaluator_actual_pending/callback_matrix.rs"]
mod callback_matrix;
#[path = "evaluator_actual_pending/canonical_emit_matrix.rs"]
mod canonical_emit_matrix;
#[path = "evaluator_actual_pending/emit.rs"]
mod emit;
#[path = "evaluator_actual_pending/file_create_from_stream.rs"]
mod file_create_from_stream;
#[path = "evaluator_actual_pending/native_websocket_db_query.rs"]
mod native_websocket_db_query;
#[path = "evaluator_actual_pending/outbound.rs"]
mod outbound;
#[path = "evaluator_actual_pending/support.rs"]
mod support;

use support::*;
